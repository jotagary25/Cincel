//! "Nuevo archivo…" (`Ctrl+N`, `docs/specs/07-etapa5-productividad.md` §8.2,
//! D6, E5-I): a floating field, anchored top-center over the editor area —
//! same style and position as the quick file finder (`crate::file_finder`,
//! D16: it must never push the editor) — that creates an empty file on disk
//! under a base folder and opens it pinned, with the focus in its editor.
//!
//! [`validate_new_file_name`] is the pure, GPUI-free half: it decides whether
//! a typed name is acceptable and what relative path it spells (subfolders
//! included), without touching a file system or a project. [`NewFilePrompt`]
//! is the entity that owns the floating panel, built once per open project
//! (like [`crate::file_finder::FileFinder`]) so `Workspace::open_project`
//! only has to build it alongside the finder.

use std::path::{Path, PathBuf};

use gpui::{App, Context, Entity, FocusHandle, Focusable, Subscription, Window};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::v_flex;
use gpui_kit::prelude::*;
use gpui_kit::{FontWeight, SharedString, div, px};

use crate::center::CenterPanel;
use crate::project::Project;
use crate::theme::ThemeColors;
use crate::workspace::Workspace;

/// The GPUI key context of the floating field; `keymap.rs` binds `Enter` and
/// `Esc` against it (`built_in_bindings`, like the file finder and the
/// shortcuts modal).
pub const KEY_CONTEXT: &str = "NewFilePrompt";

/// §3.2 geometry, reused verbatim (D16: same floating style and position as
/// the file finder, so this never competes with it for screen space).
const TOP_OFFSET: f32 = 72.;
const MAX_WIDTH: f32 = 560.;
const WINDOW_MARGIN: f32 = 48.;
const INPUT_HEIGHT: f32 = 32.;

/// `new_file::confirm`: validates the typed name, creates the file and
/// opens it pinned.
#[derive(Clone, Copy, Debug, Default, PartialEq, gpui::Action)]
#[action(namespace = new_file, name = "confirm")]
pub struct Confirm;
/// `new_file::dismiss`: closes without creating anything.
#[derive(Clone, Copy, Debug, Default, PartialEq, gpui::Action)]
#[action(namespace = new_file, name = "dismiss")]
pub struct Dismiss;

/// Validates and normalizes what the user typed into "Nuevo archivo" (§8.2):
/// rejects an empty name and any path that would leave the project through
/// an absolute path or a `..` component, and accepts subfolders (`a/b.rs`,
/// which the caller creates). Pure: it knows nothing about a real file
/// system or a project root, so it is testable on its own.
pub fn validate_new_file_name(name: &str) -> Result<PathBuf, SharedString> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(SharedString::from("El nombre no puede estar vacío"));
    }
    let candidate = Path::new(trimmed);
    let leaves_the_project = candidate.is_absolute()
        || candidate
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir));
    if leaves_the_project {
        return Err(SharedString::from(
            "El archivo tiene que quedar dentro del proyecto",
        ));
    }
    Ok(candidate.to_path_buf())
}

/// The floating "Nuevo archivo" field, kept alive for the life of the open
/// project like [`crate::file_finder::FileFinder`].
pub struct NewFilePrompt {
    project: Entity<Project>,
    center: Entity<CenterPanel>,
    input: Entity<InputState>,
    open: bool,
    /// Absolute path of the folder new files are created under, fixed when
    /// the prompt opens (§8.2: the tree's selected folder, the folder of the
    /// selected file, or the project root).
    base_folder: PathBuf,
    /// What "Nuevo archivo en …/" shows for [`Self::base_folder`].
    base_label: SharedString,
    error: Option<SharedString>,
    previous_focus: Option<FocusHandle>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl NewFilePrompt {
    /// Builds a closed prompt over `project`.
    pub fn new(
        project: Entity<Project>,
        center: Entity<CenterPanel>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        cx.new(|cx| Self::build(project, center, window, cx))
    }

    fn build(
        project: Entity<Project>,
        center: Entity<CenterPanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("nombre.rs"));
        let subscriptions = vec![
            cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
                InputEvent::PressEnter { .. } => this.confirm(window, cx),
                InputEvent::Change if this.error.is_some() => {
                    this.error = None;
                    cx.notify();
                }
                _ => {}
            }),
        ];

        Self {
            project,
            center,
            input,
            open: false,
            base_folder: PathBuf::new(),
            base_label: SharedString::default(),
            error: None,
            previous_focus: None,
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// Whether the panel is on screen.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The typed name, for the tests.
    pub fn input(&self) -> &Entity<InputState> {
        &self.input
    }

    /// The banner shown under the field, for the tests.
    pub fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    /// The folder new files land in, for the tests.
    pub fn base_folder(&self) -> &Path {
        &self.base_folder
    }

    /// Opens the field over `base_folder` (an absolute path), with the
    /// previous input preserved from the last time it was open in this
    /// project (like the file finder's query).
    pub fn open(
        &mut self,
        base_folder: PathBuf,
        previous_focus: Option<FocusHandle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open = true;
        self.error = None;
        self.previous_focus = previous_focus;
        self.base_label = SharedString::from(format!("{}/", base_folder.display()));
        self.base_folder = base_folder;
        self.input
            .update(cx, |input, cx| input.select_all(window, cx));
        let handle = self.input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    /// Closes without creating anything, giving the keyboard back to
    /// whatever had it before.
    pub fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        self.open = false;
        self.error = None;
        if let Some(previous) = self.previous_focus.take() {
            window.focus(&previous, cx);
        }
        cx.notify();
    }

    /// `Enter`: validates, creates the file and opens it pinned with the
    /// focus in its editor (`CenterPanel::open_file`, `pin: true`). Stays
    /// open with the exact error of §8.2 on any failure.
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.input.read(cx).value().to_string();
        let relative = match validate_new_file_name(&typed) {
            Ok(relative) => relative,
            Err(message) => {
                self.error = Some(message);
                cx.notify();
                return;
            }
        };
        let absolute = self.base_folder.join(&relative);
        if absolute.exists() {
            let shown = self.project.read(cx).relative(&absolute);
            self.error = Some(SharedString::from(format!(
                "Ya existe «{}»",
                shown.display()
            )));
            cx.notify();
            return;
        }
        if let Some(parent) = absolute.parent()
            && let Err(error) = std::fs::create_dir_all(parent)
        {
            self.error = Some(SharedString::from(format!("No se pudo crear: {error}")));
            cx.notify();
            return;
        }
        if let Err(error) = std::fs::write(&absolute, b"") {
            self.error = Some(SharedString::from(format!("No se pudo crear: {error}")));
            cx.notify();
            return;
        }
        // The user's new file, never the agent's (`crate::review`).
        self.project
            .update(cx, |project, cx| project.note_host_write(&absolute, cx));
        self.open = false;
        self.error = None;
        // The focus goes to the opened editor, not back to `previous_focus`
        // (like the file finder's own confirm).
        self.previous_focus = None;
        self.center.update(cx, |center, cx| {
            center.open_file(&absolute, true, window, cx)
        });
        cx.notify();
    }

    fn on_confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm(window, cx);
    }

    fn on_dismiss(&mut self, _: &Dismiss, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss(window, cx);
    }
}

impl Focusable for NewFilePrompt {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for NewFilePrompt {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().into_any_element();
        }
        let theme = ThemeColors::global(cx).clone();
        let scale = crate::settings::ui_scale(cx);
        let window_width: f32 = window.viewport_size().width.into();
        let width = (MAX_WIDTH * scale).min((window_width - WINDOW_MARGIN * scale).max(0.));
        let error = self.error.clone();

        div()
            .absolute()
            .inset_0()
            // Keeps mouse and scroll-wheel events from reaching the editor
            // behind the prompt: without this, the wheel scrolled the file
            // open underneath it.
            .occlude()
            // A transparent layer catches the outside click and closes the
            // panel, same as the file finder (§3.2: "sin velo oscuro").
            .child(
                div()
                    .id("new-file-scrim")
                    .absolute()
                    .inset_0()
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, _: &gpui::MouseDownEvent, window, cx| {
                            this.dismiss(window, cx);
                        }),
                    ),
            )
            .child(
                v_flex()
                    .id("new-file-prompt")
                    .debug_selector(|| "new-file-prompt".to_string())
                    .key_context(KEY_CONTEXT)
                    .track_focus(&self.focus_handle)
                    .on_action(cx.listener(Self::on_confirm))
                    .on_action(cx.listener(Self::on_dismiss))
                    .absolute()
                    .top(px(TOP_OFFSET * scale))
                    .left_0()
                    .right_0()
                    .mx_auto()
                    .w(px(width))
                    .rounded(px(6. * scale))
                    .bg(theme.bg_elevated)
                    .border_1()
                    .border_color(theme.border)
                    .shadow_lg()
                    .p_2()
                    .gap_1()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .text_size(px(12. * scale))
                            .text_color(theme.text_muted)
                            .child(SharedString::from(format!(
                                "Nuevo archivo en {}",
                                self.base_label
                            ))),
                    )
                    .child(Input::new(&self.input).h(px(INPUT_HEIGHT * scale)))
                    .when_some(error, |panel, error| {
                        panel.child(
                            div()
                                .text_size(px(12. * scale))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme.status_error)
                                .child(error),
                        )
                    }),
            )
            .into_any_element()
    }
}

impl Workspace {
    /// `Ctrl+N` (`workspace::new_file`, §8.2, D6, E5-I): opens the field over
    /// the tree's selected folder, the selected file's folder, or the
    /// project root, or toasts without a project.
    pub fn open_new_file_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.project().cloned() else {
            crate::toast::info("Abrí una carpeta para crear un archivo", cx);
            return;
        };
        let Some(prompt) = self.new_file_prompt().cloned() else {
            return;
        };
        let root = project.read(cx).root().to_path_buf();
        let base = self
            .files()
            .read(cx)
            .new_file_base_folder(cx)
            .unwrap_or(root);
        let previous = window.focused(cx);
        prompt.update(cx, |prompt, cx| prompt.open(base, previous, window, cx));
    }
}
