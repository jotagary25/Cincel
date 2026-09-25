//! The center of the window: the tab bar, the breadcrumb and the tab body.
//!
//! Every tab holds an [`asteroid_editor::EditorView`] over the
//! [`asteroid_project::BufferHandle`] the store opened, which is the same
//! `Arc<Mutex<Buffer>>` the agent writes through, so edits, saves and reloads
//! all meet in one place. The tab bar, the dirty dot, the preview italics, the
//! close dialog, the breadcrumb, the status bar and the layout file work off
//! [`Tab`], never off the editor.
//!
//! A file that is not valid UTF-8 still opens, in an editor over a read-only
//! buffer built with [`asteroid_text::Buffer::from_bytes_lossy`]
//! (`docs/specs/modulos/workspace.md`: "solo lectura").

use std::path::{Path, PathBuf};

use asteroid_editor::{EditorEvent, EditorView, shared};
use asteroid_project::{BufferChange, BufferHandle, OpenError, ReloadOutcome};
use asteroid_syntax::Language;
use asteroid_text::{Buffer, LineEnding, Point};
use gpui::{
    App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, MouseButton,
    MouseDownEvent, SharedString, Subscription, Window,
};
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{FontWeight, div, px};
use std::sync::Arc;

use crate::layout::TabLayout;
use crate::project::{Project, ProjectEvent};
use crate::theme::ThemeColors;

/// The GPUI key context the tab area declares. The code editor will declare
/// `Editor` inside it in stage 2.
pub const KEY_CONTEXT: &str = "Center";

/// The extra context the tab area declares while the "¿Guardar cambios?"
/// dialog is up, so `Enter` and `Esc` belong to the dialog.
pub const MODAL_CONTEXT: &str = "asking_to_save";

/// [`KEY_CONTEXT`] plus [`MODAL_CONTEXT`], as GPUI's node syntax spells it.
const ASKING_CONTEXT: &str = "Center asking_to_save";

/// Height of a tab (`docs/specs/02-visual.md` §4).
const TAB_HEIGHT: f32 = 32.;
/// Width of the "¿Guardar cambios?" dialog (`docs/specs/02-visual.md` §6.5).
const DIALOG_WIDTH: f32 = 420.;
/// Height of each of its buttons.
const DIALOG_BUTTON_HEIGHT: f32 = 28.;
/// Height of the breadcrumb line under the tabs.
const BREADCRUMB_HEIGHT: f32 = 22.;
/// Where a reopened file was left (`layout.json`).
#[derive(Clone, Copy, Debug)]
pub struct SavedPosition {
    /// First visible wrap row.
    pub scroll: f32,
    /// Cursor, in buffer coordinates.
    pub cursor: Point,
}

/// What a tab draws: always the code editor, over the shared buffer.
pub enum TabContent {
    /// The code editor of `asteroid-editor`.
    Editor(Entity<EditorView>),
}

impl TabContent {
    /// Builds the body of a tab.
    ///
    /// `read_only` files (non-UTF-8) get exactly the same element: the buffer
    /// itself refuses edits, so there is no second kind of view to maintain.
    fn build(
        buffer: &BufferHandle,
        language: Option<Arc<Language>>,
        project: &Entity<Project>,
        position: Option<SavedPosition>,
        window: &mut Window,
        cx: &mut App,
    ) -> Self {
        let registry = project.read(cx).languages().clone();
        let settings = crate::theme::editor_settings(cx);
        let theme = crate::theme::editor_theme(cx);
        let buffer = buffer.clone();
        TabContent::Editor(cx.new(|cx| {
            let mut editor =
                EditorView::new(buffer, language, registry, settings, theme, window, cx);
            // Scroll first, cursor second: the editor autoscrolls only when the
            // cursor lands off-screen, so a cursor inside the restored viewport
            // leaves the view exactly where it was saved.
            if let Some(position) = position {
                editor.set_scroll_row(position.scroll, cx);
                editor.set_cursor(position.cursor, cx);
            }
            editor
        }))
    }

    /// The editor of this tab.
    pub fn editor(&self) -> &Entity<EditorView> {
        match self {
            TabContent::Editor(editor) => editor,
        }
    }

    /// The element the center area shows for this tab.
    fn render(&self) -> gpui::AnyElement {
        match self {
            TabContent::Editor(editor) => editor.clone().into_any_element(),
        }
    }
}

/// One open tab.
pub struct Tab {
    /// Absolute path of the file.
    pub path: PathBuf,
    /// Path relative to the project root, for the tab title and breadcrumb.
    pub relative: PathBuf,
    /// The file name.
    pub title: SharedString,
    /// Preview tabs are drawn in italics and are replaced by the next preview
    /// (`docs/specs/modulos/workspace.md`).
    pub preview: bool,
    /// First visible wrap row, from [`EditorEvent::ScrollChanged`].
    pub scroll: f32,
    /// Where the cursor is, for the status bar and for `layout.json`.
    pub cursor: Point,
    /// The definition the cursor is inside, for the breadcrumb.
    pub symbol: Option<SharedString>,
    /// Whether the file was deleted behind our back.
    pub deleted: bool,
    /// Whether the buffer refuses edits (a file that is not UTF-8).
    pub read_only: bool,
    /// Whether an agent wrote to this file during the open session. Shown as
    /// a "◆" suffix on the tab title; a real per-hunk indicator is Etapa 3
    /// (`docs/etapas/etapa-2.md`).
    pub agent_touched: bool,
    /// The buffer behind the tab, shared with the store.
    pub buffer: BufferHandle,
    /// The language the registry detected, or `None` for plain text.
    pub language: Option<Arc<Language>>,
    /// What the tab draws.
    pub content: TabContent,
    /// Keeps the subscription to the editor's events alive.
    _subscription: Subscription,
}

impl Tab {
    /// The end-of-line convention of the file, for the status bar.
    pub fn line_ending(&self) -> LineEnding {
        self.buffer.lock().line_ending()
    }

    /// The editor of this tab.
    pub fn editor(&self) -> &Entity<EditorView> {
        self.content.editor()
    }

    /// The title as the tab bar shows it, marked when the file is gone or an
    /// agent touched it this session.
    pub fn display_title(&self) -> SharedString {
        if self.deleted {
            SharedString::from(format!("{} (eliminado)", self.title))
        } else if self.agent_touched {
            SharedString::from(format!("{} ◆", self.title))
        } else {
            self.title.clone()
        }
    }

    /// The cursor as a human reads it: line and column from 1, the column in
    /// characters rather than bytes (`docs/specs/02-visual.md` §1).
    pub fn cursor_line_column(&self) -> (u32, u32) {
        let line = self.buffer.lock().line_text(self.cursor.row);
        let byte = (self.cursor.column as usize).min(line.len());
        let column = line
            .get(..byte)
            .map(|prefix| prefix.chars().count())
            .unwrap_or(0);
        (self.cursor.row + 1, column as u32 + 1)
    }

    /// Its layout entry.
    fn to_layout(&self) -> TabLayout {
        TabLayout {
            path: self.relative.clone(),
            preview: self.preview,
            scroll: self.scroll,
            cursor: Some([self.cursor.row, self.cursor.column]),
        }
    }
}

/// The payload of a tab being dragged, for reordering the tab bar.
#[derive(Clone, Debug)]
struct TabDrag {
    /// Index the drag started from.
    index: usize,
    /// Title, so the thing under the cursor says what it is.
    title: SharedString,
}

/// What the cursor carries while a tab is being dragged.
struct TabDragPreview {
    title: SharedString,
}

impl Render for TabDragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = ThemeColors::global(cx).clone();
        div()
            .px_3()
            .py_1()
            .rounded(px(4.))
            .bg(theme.bg_elevated)
            .text_color(theme.text)
            .text_sm()
            .child(self.title.clone())
    }
}

/// What a pending `Ctrl+W` is waiting for.
struct PendingClose {
    index: usize,
    title: SharedString,
}

/// The center panel: tabs, breadcrumb and the active tab's body.
pub struct CenterPanel {
    project: Option<Entity<Project>>,
    tabs: Vec<Tab>,
    active: Option<usize>,
    pending_close: Option<PendingClose>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl CenterPanel {
    /// Builds the panel entity, with no project and no tabs.
    pub fn new(cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self {
            project: None,
            tabs: Vec::new(),
            active: None,
            pending_close: None,
            focus_handle: cx.focus_handle(),
            _subscriptions: Vec::new(),
        })
    }

    /// Shows `project`, closing every tab of the previous one.
    pub fn set_project(&mut self, project: Option<Entity<Project>>, cx: &mut Context<Self>) {
        self.tabs.clear();
        self.active = None;
        self.pending_close = None;
        self._subscriptions.clear();
        if let Some(project) = &project {
            let subscription = cx.subscribe(project, |this, _, event, cx| {
                if let ProjectEvent::BuffersChanged(changes) = event {
                    this.apply_buffer_changes(changes, cx);
                }
            });
            self._subscriptions.push(subscription);
        }
        self.project = project;
        cx.notify();
    }

    /// The open tabs, in tab bar order.
    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    /// The active tab, if any.
    pub fn active_tab(&self) -> Option<&Tab> {
        self.active.and_then(|index| self.tabs.get(index))
    }

    /// The index of the active tab.
    pub fn active_index(&self) -> Option<usize> {
        self.active
    }

    /// Opens `path` (absolute), previewing it or pinning it.
    ///
    /// A file that is already open is simply activated (and pinned when the
    /// click asked for it). A preview replaces the previous preview tab, the
    /// way every editor does it.
    pub fn open_file(
        &mut self,
        path: &Path,
        pin: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_file_at(path, pin, None, window, cx);
    }

    /// Same, reopening the file where it was left.
    pub fn open_file_at(
        &mut self,
        path: &Path,
        pin: bool,
        position: Option<SavedPosition>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.project.clone() else {
            return;
        };

        if let Some(index) = self.tabs.iter().position(|tab| tab.path == path) {
            if pin {
                self.tabs[index].preview = false;
            }
            self.activate(index, window, cx);
            return;
        }

        let opened = project.update(cx, |project, cx| {
            project
                .open_buffer(path, cx)
                .map(|buffer| (buffer, project.language_for(path), project.relative(path)))
        });
        let (buffer, language, relative, read_only) = match opened {
            Ok((buffer, language, relative)) => (buffer, language, relative, false),
            // A file that is not UTF-8 still opens, read only, out of the
            // store (`docs/specs/modulos/workspace.md`).
            Err(OpenError::NotUtf8(_)) => {
                let Some(buffer) = read_only_buffer(path, cx) else {
                    return;
                };
                crate::toast::warn("Archivo abierto en solo lectura: contenido no UTF-8", cx);
                let (language, relative) = project.read_with(cx, |project, _| {
                    (project.language_for(path), project.relative(path))
                });
                (buffer, language, relative, true)
            }
            Err(error) => {
                report_open_error(&error, cx);
                return;
            }
        };

        let content = TabContent::build(&buffer, language.clone(), &project, position, window, cx);
        let editor = content.editor().clone();
        let subscription = cx.subscribe_in(&editor, window, {
            let path = path.to_path_buf();
            move |this, editor, event: &EditorEvent, window, cx| {
                this.on_editor_event(&path, editor, event, window, cx);
            }
        });
        let tab = Tab {
            path: path.to_path_buf(),
            title: SharedString::from(
                relative
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| relative.display().to_string()),
            ),
            relative,
            preview: !pin,
            scroll: position.map(|position| position.scroll).unwrap_or(0.),
            cursor: position.map(|position| position.cursor).unwrap_or_default(),
            symbol: None,
            deleted: false,
            read_only,
            agent_touched: false,
            buffer,
            language,
            content,
            _subscription: subscription,
        };

        // A preview takes the place of the previous preview.
        let index = match self.tabs.iter().position(|tab| tab.preview) {
            Some(index) if !pin => {
                self.close_without_asking(index, window, cx);
                index.min(self.tabs.len())
            }
            _ => self.tabs.len(),
        };
        self.tabs.insert(index, tab);
        self.activate(index, window, cx);
    }

    /// What the editor of `path` has to say.
    fn on_editor_event(
        &mut self,
        path: &Path,
        editor: &Entity<EditorView>,
        event: &EditorEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            // The dot in the tab follows the editor, not a poll of the store.
            // An edit can also change the enclosing definition, and the symbol
            // may only resolve once the background parse lands.
            EditorEvent::DirtyChanged(_) => {
                self.refresh_symbol(path, editor, cx);
                cx.notify();
            }
            EditorEvent::CursorMoved { point } => {
                if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.path == path) {
                    tab.cursor = *point;
                }
                self.refresh_symbol(path, editor, cx);
                cx.notify();
            }
            EditorEvent::ScrollChanged { row } => {
                if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.path == path) {
                    tab.scroll = *row;
                }
                // Nothing to repaint: the layout file is written when the
                // window or the project closes.
            }
            EditorEvent::SaveRequested => {
                self.save_path(path, Some(editor), window, cx);
            }
        }
    }

    /// Re-reads the definition the cursor is inside, for the breadcrumb.
    fn refresh_symbol(&mut self, path: &Path, editor: &Entity<EditorView>, cx: &mut Context<Self>) {
        let symbol = editor
            .read(cx)
            .symbol_at_cursor()
            .map(|(name, _)| SharedString::from(name));
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.path == path)
            && tab.symbol != symbol
        {
            tab.symbol = symbol;
            cx.notify();
        }
    }

    /// Writes `path` to disk and tells its editor the file is clean again.
    fn save_path(
        &mut self,
        path: &Path,
        editor: Option<&Entity<EditorView>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(project) = self.project.clone() else {
            return false;
        };
        if self
            .tabs
            .iter()
            .any(|tab| tab.path == path && tab.read_only)
        {
            crate::toast::warn("El archivo está abierto en solo lectura", cx);
            return false;
        }
        match project.update(cx, |project, cx| project.save(path, cx)) {
            Ok(()) => {
                let editor = editor
                    .cloned()
                    .or_else(|| self.tab_for(path).map(|tab| tab.editor().clone()));
                if let Some(editor) = editor {
                    editor.update(cx, |editor, cx| editor.mark_saved(cx));
                }
                project.read(cx).refresh_git();
                tracing::info!(path = %path.display(), "archivo guardado");
                cx.notify();
                true
            }
            Err(error) => {
                crate::toast::error(error, cx);
                false
            }
        }
    }

    /// Saves the active tab (`editor::save` arrives as an event, this is for
    /// the dialog and for autosave).
    pub fn save_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.active_tab().map(|tab| tab.path.clone()) else {
            return;
        };
        self.save_path(&path, None, window, cx);
    }

    /// `editor::save_all`: every tab with unsaved changes.
    pub fn save_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dirty = self.dirty_paths(cx);
        let count = dirty.len();
        for path in dirty {
            self.save_path(&path, None, window, cx);
        }
        if count == 0 {
            crate::toast::info_keyed("save-all", "No hay cambios sin guardar", cx);
        }
    }

    /// Saves everything that is dirty, for `files.autosave = "on_focus_change"`.
    pub fn autosave(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if crate::settings::settings(cx).files.autosave
            != asteroid_settings::Autosave::OnFocusChange
        {
            return;
        }
        for path in self.dirty_paths(cx) {
            self.save_path(&path, None, window, cx);
        }
    }

    /// The open files with unsaved changes.
    fn dirty_paths(&self, cx: &App) -> Vec<PathBuf> {
        self.tabs
            .iter()
            .filter(|tab| !tab.read_only && tab.editor().read(cx).is_dirty())
            .map(|tab| tab.path.clone())
            .collect()
    }

    /// The tab showing `path`, if any.
    fn tab_for(&self, path: &Path) -> Option<&Tab> {
        self.tabs.iter().find(|tab| tab.path == path)
    }

    /// Whether the tab at `index` has unsaved changes.
    pub fn is_tab_dirty(&self, index: usize, cx: &App) -> bool {
        self.tabs
            .get(index)
            .is_some_and(|tab| tab.editor().read(cx).is_dirty())
    }

    /// Hands every open editor the settings, theme and grammar in force, after
    /// a hot reload or a zoom change.
    pub fn refresh_editor_style(&mut self, cx: &mut Context<Self>) {
        let settings = crate::theme::editor_settings(cx);
        let theme = crate::theme::editor_theme(cx);
        for tab in &self.tabs {
            tab.editor().update(cx, |editor, cx| {
                editor.set_settings(settings.clone(), cx);
                editor.set_theme(theme, cx);
            });
        }
        self.refresh_languages(cx);
        cx.notify();
    }

    /// Re-resolves the grammar of every tab against the registry.
    ///
    /// The path of a tab can start meaning a different language — the file was
    /// renamed on disk, or the registry learned about a new extension — and
    /// the editor takes the new grammar without being rebuilt.
    fn refresh_languages(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.project.clone() else {
            return;
        };
        for index in 0..self.tabs.len() {
            let path = self.tabs[index].path.clone();
            let language = project.read(cx).language_for(&path);
            let same = match (&language, &self.tabs[index].language) {
                (Some(new), Some(old)) => new.name() == old.name(),
                (None, None) => true,
                _ => false,
            };
            if same {
                continue;
            }
            self.tabs[index].language = language.clone();
            let editor = self.tabs[index].editor().clone();
            editor.update(cx, |editor, cx| editor.set_language(language, cx));
        }
    }

    /// Reacts to what the disk did to the open buffers
    /// (`docs/specs/modulos/workspace.md`, "cambios externos").
    pub fn apply_buffer_changes(&mut self, changes: &[BufferChange], cx: &mut Context<Self>) {
        for change in changes {
            let Some(index) = self.tabs.iter().position(|tab| tab.path == change.path) else {
                continue;
            };
            match change.outcome {
                ReloadOutcome::Reloaded => {
                    self.tabs[index].deleted = false;
                    let editor = self.tabs[index].editor().clone();
                    editor.update(cx, |editor, cx| editor.buffer_changed(cx));
                    self.refresh_languages(cx);
                }
                ReloadOutcome::Conflict => {
                    self.tabs[index].deleted = false;
                    self.ask_about_conflict(&change.path, cx);
                }
                ReloadOutcome::Deleted => {
                    self.tabs[index].deleted = true;
                }
                ReloadOutcome::NotOpen | ReloadOutcome::Unchanged | ReloadOutcome::SelfWrite => {}
            }
        }
        cx.notify();
    }

    /// Whether `path` is open and clean (no unsaved user changes), which is
    /// the "reload from disk if clean" gate `asteroid-workspace/agents.rs`
    /// needs before touching a buffer on the agent's behalf.
    pub fn is_open_and_clean(&self, path: &Path, cx: &App) -> bool {
        self.tab_for(path)
            .is_some_and(|tab| !tab.read_only && !tab.editor().read(cx).is_dirty())
    }

    /// Marks `path`'s tab as agent-written without touching its buffer: the
    /// content already landed through `BufferStore::apply_agent_write`, which
    /// mutates the shared buffer in place, so the editor only needs the same
    /// "something external changed" nudge the disk-reload path gives it.
    ///
    /// Does nothing when `path` has no open tab (E2 opens no new tabs on the
    /// agent's behalf, `docs/etapas/etapa-2.md`).
    pub fn note_agent_write(&mut self, path: &Path, cx: &mut Context<Self>) {
        let Some(index) = self.tabs.iter().position(|tab| tab.path == path) else {
            return;
        };
        self.tabs[index].deleted = false;
        self.tabs[index].agent_touched = true;
        let editor = self.tabs[index].editor().clone();
        editor.update(cx, |editor, cx| editor.buffer_changed(cx));
        cx.notify();
    }

    /// Reloads `path`'s open buffer from disk after an agent tool call
    /// finished editing it outside `fs/write_text_file` (a shell command, its
    /// own file API, …), and marks the tab. Skips dirty buffers: E3 owns the
    /// "buffer sucio" dialog this needs (`docs/specs/modulos/workspace.md`).
    ///
    /// Returns whether the buffer was reloaded (or reported deleted/in
    /// conflict): callers use it to decide whether a git refresh is worth it.
    pub fn reload_after_agent_edit(&mut self, path: &Path, cx: &mut Context<Self>) -> bool {
        let Some(project) = self.project.clone() else {
            return false;
        };
        if !self.is_open_and_clean(path, cx) {
            return false;
        }
        let outcome = project.update(cx, |project, _| {
            project.buffers_mut().reload_from_disk(path)
        });
        let handled = matches!(
            outcome,
            ReloadOutcome::Reloaded | ReloadOutcome::Conflict | ReloadOutcome::Deleted
        );
        if handled {
            self.apply_buffer_changes(
                &[BufferChange {
                    path: path.to_path_buf(),
                    outcome,
                }],
                cx,
            );
        }
        if outcome == ReloadOutcome::Reloaded
            && let Some(tab) = self.tabs.iter_mut().find(|tab| tab.path == path)
        {
            tab.agent_touched = true;
        }
        cx.notify();
        handled
    }

    /// The toast that lets the user decide who wins a conflict.
    fn ask_about_conflict(&mut self, path: &Path, cx: &mut Context<Self>) {
        let Some(project) = self.project.clone() else {
            return;
        };
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());
        let center = cx.entity().downgrade();

        let reload = {
            let project = project.clone();
            let path = path.to_path_buf();
            let center = center.clone();
            move |cx: &mut App| {
                project.update(cx, |project, cx| {
                    project.buffers_mut().discard_changes(&path);
                    cx.notify();
                });
                let _ = center.update(cx, |center, cx| {
                    if let Some(tab) = center.tab_for(&path) {
                        tab.editor().update(cx, |editor, cx| {
                            editor.buffer_changed(cx);
                            editor.mark_saved(cx);
                        });
                    }
                    cx.notify();
                });
            }
        };
        let keep = {
            let path = path.to_path_buf();
            move |cx: &mut App| {
                project.update(cx, |project, cx| {
                    project.buffers_mut().keep_my_version(&path);
                    cx.notify();
                });
            }
        };

        crate::toast::ask(
            format!("«{name}» cambió en el disco y tenés cambios sin guardar"),
            [
                (
                    "Recargar del disco",
                    Box::new(reload) as crate::toast::ToastAction,
                ),
                (
                    "Mantener mi versión",
                    Box::new(keep) as crate::toast::ToastAction,
                ),
            ],
            cx,
        );
    }

    /// Moves the tab at `from` to `to`, which is what dropping a dragged tab
    /// does (`docs/specs/modulos/workspace.md`, "reordenar arrastrando").
    pub fn move_tab(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        if from == to || from >= self.tabs.len() || to >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        self.active = match self.active {
            Some(active) if active == from => Some(to),
            Some(active) => {
                // Everything between the two ends shifts by one.
                let mut active = active;
                if from < active && active <= to {
                    active -= 1;
                } else if to <= active && active < from {
                    active += 1;
                }
                Some(active)
            }
            None => None,
        };
        cx.notify();
    }

    /// Makes the tab at `index` the active one and puts the cursor in it.
    ///
    /// Leaving a tab is a focus change, so `files.autosave` fires here too.
    pub fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        if self.active != Some(index) {
            self.autosave(window, cx);
        }
        self.active = Some(index);
        self.focus_active(window, cx);
        cx.notify();
    }

    /// Gives the keyboard to the editor of the active tab.
    pub fn focus_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.active_tab() else {
            window.focus(&self.focus_handle, cx);
            return;
        };
        let handle = tab.editor().read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    }

    /// Closes the tab at `index`, asking first when it has unsaved changes.
    pub fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        // The editor is the authority on "unsaved changes": it is what the
        // user typed into.
        let dirty = !tab.read_only && tab.editor().read(cx).is_dirty();
        if dirty {
            self.pending_close = Some(PendingClose {
                index,
                title: tab.title.clone(),
            });
            // The dialog takes the keyboard: GPUI matches a binding at the
            // innermost context node first, so while the editor holds the
            // focus its own `Esc` and `Enter` would win over the dialog's.
            window.focus(&self.focus_handle, cx);
            cx.notify();
            return;
        }
        self.close_without_asking(index, window, cx);
    }

    /// Closes the active tab (`workspace::close_tab`).
    pub fn close_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.active {
            self.close_tab(index, window, cx);
        }
    }

    fn close_without_asking(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(index);
        if let Some(project) = &self.project {
            project.update(cx, |project, cx| project.close_buffer(&tab.path, cx));
        }
        self.active = match self.active {
            Some(active) if self.tabs.is_empty() => {
                let _ = active;
                None
            }
            Some(active) if active > index => Some(active - 1),
            Some(active) if active == index => Some(active.min(self.tabs.len() - 1)),
            other => other,
        };
        // The closed view had the keyboard; without this the focus stays on a
        // view that is gone, the `Editor`/`Center` contexts stop matching and
        // the next `Ctrl+W` does nothing. With no tabs left the panel itself
        // takes it, so the global commands keep working.
        self.focus_active(window, cx);
        cx.notify();
    }

    /// "Guardar" in the unsaved changes dialog.
    fn save_and_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_close.take() else {
            return;
        };
        let Some(path) = self.tabs.get(pending.index).map(|tab| tab.path.clone()) else {
            return;
        };
        if !self.save_path(&path, None, window, cx) {
            cx.notify();
            return;
        }
        self.close_without_asking(pending.index, window, cx);
    }

    /// "Descartar" in the unsaved changes dialog.
    fn discard_and_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_close.take() else {
            return;
        };
        self.close_without_asking(pending.index, window, cx);
    }

    /// "Cancelar" in the unsaved changes dialog.
    fn cancel_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pending_close = None;
        self.focus_active(window, cx);
        cx.notify();
    }

    /// Whether the unsaved changes dialog is up (for the tests).
    pub fn is_asking_about_unsaved_changes(&self) -> bool {
        self.pending_close.is_some()
    }

    /// "Cancelar" in the dialog, for the tests (which cannot click).
    pub fn cancel_close_for_test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_close(window, cx);
    }

    /// "Descartar" in the dialog, for the tests.
    pub fn discard_and_close_for_test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.discard_and_close(window, cx);
    }

    /// "Guardar" in the dialog, for the tests.
    pub fn save_and_close_for_test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_and_close(window, cx);
    }

    /// The tabs as the layout file stores them.
    pub fn to_layout(&self) -> (Vec<TabLayout>, Option<usize>) {
        (self.tabs.iter().map(Tab::to_layout).collect(), self.active)
    }

    /// Reopens the tabs of a restored layout. Paths that no longer exist are
    /// skipped without a word: a project changes between runs.
    pub fn restore(
        &mut self,
        tabs: &[TabLayout],
        active: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.project.clone() else {
            return;
        };
        let root = project.read(cx).root().to_path_buf();
        for tab in tabs {
            let path = root.join(&tab.path);
            if !path.is_file() {
                continue;
            }
            let position = SavedPosition {
                scroll: tab.scroll,
                cursor: tab
                    .cursor
                    .map(|[row, column]| Point { row, column })
                    .unwrap_or_default(),
            };
            self.open_file_at(&path, !tab.preview, Some(position), window, cx);
        }
        if let Some(active) = active.filter(|index| *index < self.tabs.len()) {
            self.activate(active, window, cx);
        }
    }

    fn on_close_tab(
        &mut self,
        _: &crate::actions::CloseTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_active(window, cx);
    }

    /// The tab bar (`docs/specs/02-visual.md` §1).
    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = ThemeColors::global(cx).clone();
        let dirty: Vec<bool> = self
            .tabs
            .iter()
            .map(|tab| tab.editor().read(cx).is_dirty())
            .collect();

        h_flex()
            .id("tab-bar")
            .h(px(TAB_HEIGHT))
            .flex_none()
            .w_full()
            .overflow_hidden()
            .bg(theme.bg_app)
            .border_b_1()
            .border_color(theme.border)
            .children(self.tabs.iter().enumerate().map(|(index, tab)| {
                let active = self.active == Some(index);
                let is_dirty = dirty[index];
                h_flex()
                    .id(("tab", index))
                    .h_full()
                    .px_3()
                    .gap_2()
                    .items_center()
                    .cursor_pointer()
                    .border_r_1()
                    .border_color(theme.border)
                    .bg(if active {
                        theme.bg_elevated
                    } else {
                        theme.bg_surface
                    })
                    .text_color(if active { theme.text } else { theme.text_muted })
                    .when(tab.preview, |this| this.italic())
                    .when(active, |this| this.font_weight(FontWeight::MEDIUM))
                    .child(tab.display_title())
                    .child(
                        div()
                            .id(("tab-close", index))
                            .w(px(14.))
                            .text_color(theme.text_muted)
                            .cursor_pointer()
                            .hover(|style| style.text_color(theme.text))
                            .child(if is_dirty {
                                SharedString::from("●")
                            } else {
                                SharedString::from("×")
                            })
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.close_tab(index, window, cx);
                            })),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.activate(index, window, cx);
                    }))
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                            this.close_tab(index, window, cx);
                        }),
                    )
                    .on_drag(
                        TabDrag {
                            index,
                            title: tab.title.clone(),
                        },
                        |drag, _, _, cx| {
                            let title = drag.title.clone();
                            cx.new(|_| TabDragPreview { title })
                        },
                    )
                    .drag_over::<TabDrag>(move |style, _, _, _| style.bg(theme.bg_elevated))
                    .on_drop(cx.listener(move |this, drag: &TabDrag, _, cx| {
                        this.move_tab(drag.index, index, cx);
                    }))
            }))
    }

    /// The breadcrumb under the tabs: the path of the active tab relative to
    /// the project root, and the definition the cursor is inside
    /// (`docs/specs/02-visual.md` §1: "src › main.rs › fn main").
    fn render_breadcrumb(&self, cx: &App) -> impl IntoElement {
        let theme = ThemeColors::global(cx).clone();
        let crumbs = self
            .active_tab()
            .map(|tab| {
                let mut crumbs = tab
                    .relative
                    .components()
                    .map(|component| component.as_os_str().to_string_lossy().to_string())
                    .collect::<Vec<_>>();
                if let Some(symbol) = &tab.symbol {
                    crumbs.push(symbol.to_string());
                }
                crumbs.join(" › ")
            })
            .unwrap_or_default();

        div()
            .h(px(BREADCRUMB_HEIGHT))
            .flex_none()
            .px_3()
            .flex()
            .items_center()
            .text_xs()
            .text_color(theme.text_muted)
            .bg(theme.bg_editor)
            .child(SharedString::from(crumbs))
    }

    /// The "¿Guardar cambios?" dialog of `Ctrl+W`
    /// (`docs/specs/02-visual.md` §4 and §6.5).
    ///
    /// Three neutral buttons of the same size: no colour carries meaning here,
    /// the words do. `Enter` guarda y `Esc` cancela, and the hints live under
    /// the row instead of inside the labels.
    fn render_unsaved_dialog(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let pending = self.pending_close.as_ref()?;
        let theme = ThemeColors::global(cx).clone();
        let title = pending.title.clone();

        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                // Scrim: the editor behind stays visible but out of the way.
                .bg(theme.bg_app.alpha(0.4))
                .child(
                    v_flex()
                        .w(px(DIALOG_WIDTH))
                        .p_4()
                        .gap_3()
                        .rounded(px(6.))
                        .bg(theme.bg_elevated)
                        .border_1()
                        .border_color(theme.border)
                        .shadow_lg()
                        .child(
                            div()
                                .text_size(px(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.text)
                                .child("¿Guardar cambios?"),
                        )
                        .child(div().text_sm().text_color(theme.text_muted).child(
                            SharedString::from(format!("«{title}» tiene cambios sin guardar.")),
                        ))
                        .child(
                            h_flex()
                                .gap_2()
                                .justify_end()
                                .child(
                                    dialog_button("dialog-cancel", "Cancelar", false, &theme)
                                        .on_click(cx.listener(
                                            |this, _: &ClickEvent, window, cx| {
                                                this.cancel_close(window, cx)
                                            },
                                        )),
                                )
                                .child(
                                    dialog_button("dialog-discard", "Descartar", false, &theme)
                                        .on_click(cx.listener(
                                            |this, _: &ClickEvent, window, cx| {
                                                this.discard_and_close(window, cx)
                                            },
                                        )),
                                )
                                .child(
                                    dialog_button("dialog-save", "Guardar", true, &theme).on_click(
                                        cx.listener(|this, _: &ClickEvent, window, cx| {
                                            this.save_and_close(window, cx)
                                        }),
                                    ),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.text_muted)
                                .child("Enter guarda · Esc cancela"),
                        ),
                ),
        )
    }

    /// `Enter` in the dialog: save and close.
    fn on_confirm_close(
        &mut self,
        _: &crate::actions::ConfirmClose,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.save_and_close(window, cx);
    }

    /// `Esc` in the dialog: leave everything as it was.
    fn on_cancel_close(
        &mut self,
        _: &crate::actions::CancelClose,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_close(window, cx);
    }
}

/// One button of the unsaved changes dialog.
///
/// All three look the same — surface, one pixel of border, radius 4, 28 px
/// tall — because none of them is more dangerous than the others; `default`
/// only adds the focus ring to the one `Enter` triggers.
fn dialog_button(
    id: &'static str,
    label: &'static str,
    default: bool,
    theme: &ThemeColors,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h(px(DIALOG_BUTTON_HEIGHT))
        .px(px(12.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .bg(theme.bg_surface)
        .border_1()
        .border_color(if default {
            theme.border_focus
        } else {
            theme.border
        })
        .text_color(theme.text)
        .cursor_pointer()
        .hover(|style| style.bg(theme.bg_elevated))
        .child(label)
}

/// Opens a file that is not valid UTF-8 as a read-only buffer.
///
/// It is deliberately *not* registered in the store: nothing may ever write
/// those bytes back, and the store's job is the files Asteroid can save.
fn read_only_buffer(path: &Path, cx: &mut App) -> Option<BufferHandle> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "no se pudo leer el archivo");
            crate::toast::error(format!("No se pudo leer «{}»", path.display()), cx);
            return None;
        }
    };
    let (mut buffer, _had_bom) = Buffer::from_bytes_lossy(&bytes);
    buffer.set_read_only(true);
    Some(shared(buffer))
}

/// Turns an [`OpenError`] into a toast.
fn report_open_error(error: &OpenError, cx: &mut App) {
    tracing::warn!(%error, "no se pudo abrir el archivo");
    crate::toast::error(error.to_string(), cx);
}

impl EventEmitter<PanelEvent> for CenterPanel {}

impl Focusable for CenterPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for CenterPanel {
    fn panel_name(&self) -> &'static str {
        "CenterPanel"
    }

    fn closable(&self, _: &App) -> bool {
        false
    }

    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl Panel for CenterPanel {
    /// Deliberately empty. gpui-kit 0.6.1 always draws a title strip above a
    /// group holding a single panel and offers no way to suppress it (see
    /// `docs/etapas/etapa-0.md`, finding 8), so the tab bar is drawn inside
    /// the panel and the strip is left as a thin separator.
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl Render for CenterPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = ThemeColors::global(cx).clone();
        let body = match self.active_tab() {
            Some(tab) => tab.content.render(),
            None => div()
                .flex_1()
                .min_h_0()
                .flex()
                .items_center()
                .justify_center()
                .text_color(cx.theme().muted_foreground)
                .child("Abrí un archivo o hablale al agente")
                .into_any_element(),
        };

        v_flex()
            .id("center-panel")
            // Node syntax: with the dialog up both names are on the stack, so
            // the modal bindings of `crate::keymap` match.
            .key_context(if self.pending_close.is_some() {
                ASKING_CONTEXT
            } else {
                KEY_CONTEXT
            })
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_close_tab))
            .on_action(cx.listener(Self::on_confirm_close))
            .on_action(cx.listener(Self::on_cancel_close))
            .relative()
            .size_full()
            .min_h(px(TAB_HEIGHT))
            .bg(theme.bg_editor)
            .border_t_1()
            .border_color(theme.border)
            .when(!self.tabs.is_empty(), |this| {
                this.child(self.render_tabs(cx)).child(
                    // The breadcrumb only makes sense with a file open.
                    self.render_breadcrumb(cx),
                )
            })
            .child(div().flex_1().min_h_0().child(body))
            .children(self.render_unsaved_dialog(cx))
    }
}
