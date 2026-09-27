//! The toast queue (`docs/specs/02-visual.md` §6.5).
//!
//! Short messages that appear at the bottom center of the editor area on
//! `bg.elevated` and disappear after four seconds. Anything in the application
//! can post one through [`show`], which goes through a weak global handle so
//! the queue is not kept alive by the global itself.

use std::time::Duration;

use gpui::{
    App, ClickEvent, Context, Entity, Global, InteractiveElement as _, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement as _, Styled, WeakEntity, Window, div, px,
};

use crate::theme::ThemeColors;

/// How long a toast stays on screen.
pub const TOAST_DURATION: Duration = Duration::from_secs(4);

/// How long an undo offer stays on screen (`docs/etapas/etapa-2.md`).
pub const UNDO_DURATION: Duration = Duration::from_secs(5);

/// The tone of a toast, which picks its accent color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    /// Neutral information.
    Info,
    /// Something did not work, but the application carries on.
    Warning,
    /// Something failed.
    Error,
}

/// What a button on a toast does when it is clicked.
pub type ToastAction = Box<dyn Fn(&mut App) + 'static>;

/// One message on screen.
pub struct Toast {
    /// Identity, so the timer dismisses the right one.
    pub id: u64,
    /// Messages that replace each other instead of piling up share a key
    /// (the zoom level, the configuration reload).
    pub key: Option<&'static str>,
    /// The text, in Spanish.
    pub message: SharedString,
    /// Its tone.
    pub kind: ToastKind,
    /// Buttons, for the messages that ask something (a conflict with the
    /// disk). A toast with buttons waits for an answer instead of expiring.
    pub actions: Vec<(SharedString, ToastAction)>,
}

impl std::fmt::Debug for Toast {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Toast")
            .field("id", &self.id)
            .field("message", &self.message)
            .field("kind", &self.kind)
            .field("key", &self.key)
            .field("actions", &self.actions.len())
            .finish()
    }
}

/// The queue of visible toasts.
pub struct Toasts {
    items: Vec<Toast>,
    next_id: u64,
}

impl Default for Toasts {
    fn default() -> Self {
        Self::new()
    }
}

impl Toasts {
    /// An empty queue.
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            next_id: 0,
        }
    }

    /// The toasts currently on screen, oldest first.
    pub fn items(&self) -> &[Toast] {
        &self.items
    }

    /// Whether anything is on screen.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Queues `message` and starts its timer.
    pub fn push(
        &mut self,
        message: impl Into<SharedString>,
        kind: ToastKind,
        cx: &mut Context<Self>,
    ) {
        self.push_with_actions(message, kind, Vec::new(), None, cx);
    }

    /// Queues `message` under `key`: a message already on screen with the same
    /// key is replaced and its timer starts again, instead of a second toast
    /// piling on. This is what keeps `Ctrl+=` from leaving a stack of "Zoom
    /// N %" behind.
    pub fn push_keyed(
        &mut self,
        key: &'static str,
        message: impl Into<SharedString>,
        kind: ToastKind,
        cx: &mut Context<Self>,
    ) {
        self.push_with_actions(message, kind, Vec::new(), Some(key), cx);
    }

    /// Queues `message` with buttons. A toast with buttons has no timer: the
    /// question stays until the user answers it.
    pub fn push_with_actions(
        &mut self,
        message: impl Into<SharedString>,
        kind: ToastKind,
        actions: Vec<(SharedString, ToastAction)>,
        key: Option<&'static str>,
        cx: &mut Context<Self>,
    ) {
        self.queue(message, kind, actions, key, None, cx);
    }

    /// Queues `message` with buttons **and** a timer: the buttons are an offer
    /// (undo), not a question, so the toast still goes away by itself.
    pub fn push_timed(
        &mut self,
        message: impl Into<SharedString>,
        kind: ToastKind,
        actions: Vec<(SharedString, ToastAction)>,
        timeout: Duration,
        cx: &mut Context<Self>,
    ) {
        self.queue(message, kind, actions, None, Some(timeout), cx);
    }

    /// [`Toasts::push_timed`] under `key`: a new offer replaces the previous
    /// one instead of piling up (several rejects in a row leave one toast).
    pub fn push_timed_keyed(
        &mut self,
        key: &'static str,
        message: impl Into<SharedString>,
        kind: ToastKind,
        actions: Vec<(SharedString, ToastAction)>,
        timeout: Duration,
        cx: &mut Context<Self>,
    ) {
        self.queue(message, kind, actions, Some(key), Some(timeout), cx);
    }

    fn queue(
        &mut self,
        message: impl Into<SharedString>,
        kind: ToastKind,
        actions: Vec<(SharedString, ToastAction)>,
        key: Option<&'static str>,
        timeout: Option<Duration>,
        cx: &mut Context<Self>,
    ) {
        let id = self.next_id;
        self.next_id += 1;
        let message = message.into();
        let waits_for_an_answer = !actions.is_empty() && timeout.is_none();
        let timeout = timeout.unwrap_or(TOAST_DURATION);
        tracing::info!(%message, ?kind, "aviso");
        // A keyed message takes the place of the one already on screen, and
        // the new timer below replaces its timer.
        if let Some(key) = key {
            self.items.retain(|toast| toast.key != Some(key));
        }
        self.items.push(Toast {
            id,
            key,
            message,
            kind,
            actions,
        });
        cx.notify();

        if waits_for_an_answer {
            return;
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(timeout).await;
            let _ = this.update(cx, |this, cx| this.dismiss(id, cx));
        })
        .detach();
    }

    /// Removes the toast with `id`, if it is still there.
    pub fn dismiss(&mut self, id: u64, cx: &mut Context<Self>) {
        let before = self.items.len();
        self.items.retain(|toast| toast.id != id);
        if self.items.len() != before {
            cx.notify();
        }
    }

    /// Runs the button `index` of the toast `id` and dismisses it.
    pub fn run_action(&mut self, id: u64, index: usize, cx: &mut Context<Self>) {
        let Some(toast) = self.items.iter().position(|toast| toast.id == id) else {
            return;
        };
        let mut toast = self.items.remove(toast);
        cx.notify();
        if index < toast.actions.len() {
            let (_, action) = toast.actions.remove(index);
            action(cx);
        }
    }

    /// Removes everything.
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        if !self.items.is_empty() {
            self.items.clear();
            cx.notify();
        }
    }
}

impl Render for Toasts {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = ThemeColors::global(cx).clone();
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .children(self.items.iter().map(|toast| {
                let accent = match toast.kind {
                    ToastKind::Info => theme.text_accent,
                    ToastKind::Warning => theme.status_warning,
                    ToastKind::Error => theme.status_error,
                };
                let id = toast.id;
                let buttons = (!toast.actions.is_empty()).then(|| {
                    div()
                        .flex()
                        .gap_2()
                        .children(toast.actions.iter().enumerate().map(|(index, (label, _))| {
                            div()
                                .id(("toast-action", (id as usize) * 8 + index))
                                .px_2()
                                .py_0p5()
                                .rounded(px(4.))
                                .bg(theme.bg_surface)
                                .text_color(theme.text_accent)
                                .child(label.clone())
                                .on_click(cx.listener(
                                    move |this: &mut Toasts, _: &ClickEvent, _, cx| {
                                        this.run_action(id, index, cx);
                                    },
                                ))
                        }))
                });
                div()
                    .px_3()
                    .py_1p5()
                    .rounded(px(6.))
                    .bg(theme.bg_elevated)
                    .border_l_2()
                    .border_color(accent)
                    .text_color(theme.text)
                    .text_sm()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(toast.message.clone())
                    .children(buttons)
            }))
    }
}

/// A weak handle to the window's toast queue, so any code with an `App` can
/// post a message. Weak on purpose: a strong handle in a global would keep the
/// entity alive for the whole process (and show up in GPUI's leak detector).
struct ToastHandle(WeakEntity<Toasts>);

impl Global for ToastHandle {}

/// Makes `toasts` the queue [`show`] posts to.
pub fn set_global(toasts: &Entity<Toasts>, cx: &mut App) {
    cx.set_global(ToastHandle(toasts.downgrade()));
}

/// Forgets the global queue. Called when the window goes away.
pub fn clear_global(cx: &mut App) {
    if cx.has_global::<ToastHandle>() {
        cx.remove_global::<ToastHandle>();
    }
}

/// Posts a message to the window's toast queue.
///
/// Silently does nothing when there is no window yet, which is the case for
/// problems found while the application is still booting; those are logged by
/// the caller as well.
pub fn show(message: impl Into<SharedString>, kind: ToastKind, cx: &mut App) {
    let Some(handle) = cx
        .try_global::<ToastHandle>()
        .map(|handle| handle.0.clone())
    else {
        tracing::debug!("todavía no hay cola de avisos");
        return;
    };
    let Some(toasts) = handle.upgrade() else {
        return;
    };
    toasts.update(cx, |toasts, cx| toasts.push(message, kind, cx));
}

/// Posts a message that replaces the previous one with the same key.
pub fn info_keyed(key: &'static str, message: impl Into<SharedString>, cx: &mut App) {
    let Some(toasts) = cx
        .try_global::<ToastHandle>()
        .and_then(|handle| handle.0.upgrade())
    else {
        tracing::debug!("todavía no hay cola de avisos");
        return;
    };
    toasts.update(cx, |toasts, cx| {
        toasts.push_keyed(key, message, ToastKind::Info, cx)
    });
}

/// Posts an informational message.
pub fn info(message: impl Into<SharedString>, cx: &mut App) {
    show(message, ToastKind::Info, cx);
}

/// Posts a warning.
pub fn warn(message: impl Into<SharedString>, cx: &mut App) {
    show(message, ToastKind::Warning, cx);
}

/// Posts a question with buttons, which stays until the user answers.
pub fn ask<const N: usize>(
    message: impl Into<SharedString>,
    actions: [(&'static str, ToastAction); N],
    cx: &mut App,
) {
    let Some(toasts) = cx
        .try_global::<ToastHandle>()
        .and_then(|handle| handle.0.upgrade())
    else {
        tracing::debug!("todavía no hay cola de avisos");
        return;
    };
    let actions = actions
        .into_iter()
        .map(|(label, action)| (SharedString::from(label), action))
        .collect();
    toasts.update(cx, |toasts, cx| {
        toasts.push_with_actions(message, ToastKind::Warning, actions, None, cx)
    });
}

/// Posts a "hecho, ¿lo deshago?" with a single "Deshacer" button and a timer.
///
/// Unlike [`ask`] this one is not a question: the action already happened, so
/// the toast expires by itself after [`UNDO_DURATION`] and the button is the
/// way back (`docs/etapas/etapa-2.md` § correcciones, borrar conversaciones).
pub fn undo(message: impl Into<SharedString>, action: impl Fn(&mut App) + 'static, cx: &mut App) {
    let Some(toasts) = cx
        .try_global::<ToastHandle>()
        .and_then(|handle| handle.0.upgrade())
    else {
        tracing::debug!("todavía no hay cola de avisos");
        return;
    };
    let actions = vec![(
        SharedString::from("Deshacer"),
        Box::new(action) as ToastAction,
    )];
    toasts.update(cx, |toasts, cx| {
        toasts.push_timed(message, ToastKind::Info, actions, UNDO_DURATION, cx);
    });
}

/// Posts an offer with one button (`label`) that expires by itself after
/// [`TOAST_DURATION`] and replaces the previous offer with the same `key`:
/// "Segmento rechazado · Deshacer (Alt+Shift+U)" (`02-visual.md` §6.5).
pub fn offer(
    key: &'static str,
    message: impl Into<SharedString>,
    label: &'static str,
    action: impl Fn(&mut App) + 'static,
    cx: &mut App,
) {
    let Some(toasts) = cx
        .try_global::<ToastHandle>()
        .and_then(|handle| handle.0.upgrade())
    else {
        tracing::debug!("todavía no hay cola de avisos");
        return;
    };
    let actions = vec![(SharedString::from(label), Box::new(action) as ToastAction)];
    toasts.update(cx, |toasts, cx| {
        toasts.push_timed_keyed(key, message, ToastKind::Info, actions, TOAST_DURATION, cx);
    });
}

/// Posts an error.
pub fn error(message: impl Into<SharedString>, cx: &mut App) {
    show(message, ToastKind::Error, cx);
}
