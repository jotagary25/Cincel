//! The "Archivos" panel: the project tree on the right dock.
//!
//! The rows come from gpui-kit's virtualized tree, fed from
//! [`cincel_project::Worktree`]. Only the expanded folders are materialized:
//! a collapsed one carries a single placeholder child, which is what makes
//! gpui-kit draw it as a folder without walking a project of 50 000 files on
//! every rebuild.
//!
//! Interaction follows `docs/specs/modulos/workspace.md`: single click
//! previews, double click pins, `Enter` opens the selection, the arrows move
//! and fold (gpui-kit's own bindings), and the context menu offers "Revelar en
//! carpeta", "Copiar ruta" and "Copiar ruta relativa". Colors follow
//! `docs/specs/02-visual.md` §6.4: files with pending agent changes in
//! `#e5c07b` with a `+N −M` suffix, files the agent created in `#98c379`
//! with `+N`, files it deleted struck through in `status.error` with `−N`
//! (kept in the tree until the deletion is decided, even once the worktree
//! dropped them), folders with pending children carry a 6 px dot, and the
//! header over the tree shows the project total; git colors apply to
//! everything else.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use cincel_project::{EntryKind, GitFileStatus, Worktree};
use gpui::{
    App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString,
    Subscription, WeakEntity, Window,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::tree::{TreeEntry, TreeEvent, TreeItem, TreeState, tree};
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{div, px};

use crate::WorkspaceEvent;
use crate::project::{Project, ProjectEvent};
use crate::review::{
    DELETED_TOOLTIP, FileState, FileSummary, Review, SharedSummary, outside_label, tree_stats_label,
};
use crate::theme::ThemeColors;

/// The GPUI key context the tree declares, and the one the `Enter` binding
/// uses. gpui-kit's own tree element sets it.
pub const KEY_CONTEXT: &str = "Tree";

/// Height of a tree row (`docs/specs/02-visual.md` §4).
const ROW_HEIGHT: f32 = 24.;
/// Indent per level.
const INDENT: f32 = 12.;
/// Separates a folder's path from the marker of its placeholder child, so the
/// two can never collide with a real path.
const PLACEHOLDER_SUFFIX: &str = "\u{0}placeholder";

/// The file tree panel.
pub struct FilesPanel {
    project: Option<Entity<Project>>,
    tree: Entity<TreeState>,
    /// Relative paths of the folders the user has opened.
    expanded: HashSet<PathBuf>,
    /// Relative path of the entry the context menu was opened on.
    context_target: Option<PathBuf>,
    /// What the review says about each file.
    review: SharedSummary,
    /// Whether [`FilesPanel::set_review`] added its observation.
    review_wired: bool,
    /// Relative paths of the files the agent deleted, still pending, that
    /// the worktree no longer lists: the tree keeps showing them, struck
    /// through, until the deletion is decided.
    ghosts: BTreeSet<PathBuf>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

/// How the tree paints a file with pending agent changes
/// (`docs/specs/02-visual.md` §6.4).
#[derive(Clone, Debug, PartialEq)]
pub struct ReviewRowStyle {
    /// Color of the name and the icon: `status.ok` for a file the agent
    /// created, `status.error` for one it deleted, `status.warning` for one
    /// it changed.
    pub color: gpui::Hsla,
    /// The counter after the name: `+N`, `−N`, `+N −M` or the label of a
    /// file reviewed outside the store.
    pub suffix: SharedString,
    /// A deleted file's name is struck through.
    pub strikethrough: bool,
    /// What hovering the row says (a deleted file only).
    pub tooltip: Option<&'static str>,
}

/// The [`ReviewRowStyle`] of `file`.
pub fn review_row_style(file: &FileSummary, theme: &ThemeColors) -> ReviewRowStyle {
    let deleted = file.state == FileState::Deleted;
    ReviewRowStyle {
        color: review_color(file.state, theme),
        suffix: SharedString::from(
            outside_label(file)
                .map(str::to_owned)
                .unwrap_or_else(|| tree_stats_label(file)),
        ),
        strikethrough: deleted,
        tooltip: deleted.then_some(DELETED_TOOLTIP),
    }
}

impl FilesPanel {
    /// Builds the panel entity, with no project yet.
    pub fn new(cx: &mut App) -> Entity<Self> {
        cx.new(|cx| {
            let tree = cx.new(|cx| TreeState::new(cx));
            let subscription = cx.subscribe(&tree, Self::on_tree_event);
            Self {
                project: None,
                tree,
                expanded: HashSet::new(),
                context_target: None,
                review: SharedSummary::default(),
                review_wired: false,
                ghosts: BTreeSet::new(),
                focus_handle: cx.focus_handle(),
                _subscriptions: vec![subscription],
            }
        })
    }

    /// Shows `project`, or nothing when it is `None`.
    pub fn set_project(&mut self, project: Option<Entity<Project>>, cx: &mut Context<Self>) {
        // The tree events and, once wired, the review observation stay.
        let keep = if self.has_review_observation() { 2 } else { 1 };
        self._subscriptions.truncate(keep);
        self.expanded.clear();
        self.context_target = None;
        if let Some(project) = &project {
            let subscription = cx.subscribe(project, |this, _, event, cx| match event {
                ProjectEvent::TreeChanged => this.rebuild(cx),
                ProjectEvent::GitChanged | ProjectEvent::BuffersChanged(_) => cx.notify(),
            });
            self._subscriptions.push(subscription);
        }
        self.project = project;
        self.rebuild(cx);
    }

    /// Paints the review state of `review` from now on, repainting whenever
    /// it changes.
    pub fn set_review(&mut self, review: &Entity<Review>, cx: &mut Context<Self>) {
        self.review = review.read(cx).summary();
        // A deletion that appears or gets decided adds or drops a row of
        // its own; anything else only repaints.
        let subscription = cx.observe(review, |this, _, cx| {
            if this.ghost_paths(cx) != this.ghosts {
                this.rebuild(cx);
            } else {
                cx.notify();
            }
        });
        // Kept before the project subscription, which `set_project` replaces.
        if self.review_wired {
            self._subscriptions[1] = subscription;
        } else {
            self._subscriptions.insert(1, subscription);
            self.review_wired = true;
        }
        cx.notify();
    }

    /// The project on screen.
    pub fn project(&self) -> Option<&Entity<Project>> {
        self.project.as_ref()
    }

    /// The root line's counters ("1 archivo · 3 cambios · +0 −36",
    /// `crate::review::totals_label`), or `None` with nothing pending.
    pub fn root_review_label(&self) -> Option<String> {
        let review = self.review.borrow();
        (review.pending > 0)
            .then(|| crate::review::totals_label(review.files.len(), review.pending, review.total))
    }

    fn has_review_observation(&self) -> bool {
        self.review_wired
    }

    /// The review label of `relative` as the tree paints it (`+N −M`), for
    /// the tests.
    pub fn review_label(&self, relative: &Path, cx: &App) -> Option<String> {
        self.review_style(relative, cx)
            .map(|style| style.suffix.to_string())
    }

    /// How the row of `relative` paints its review state, for the tests.
    pub fn review_style(&self, relative: &Path, cx: &App) -> Option<ReviewRowStyle> {
        let project = self.project.as_ref()?;
        let absolute = project.read(cx).absolute(relative);
        let theme = ThemeColors::global(cx);
        self.review
            .borrow()
            .file(&absolute)
            .map(|file| review_row_style(file, theme))
    }

    /// The pending deletions the worktree no longer lists, relative to the
    /// root.
    fn ghost_paths(&self, cx: &App) -> BTreeSet<PathBuf> {
        let Some(project) = &self.project else {
            return BTreeSet::new();
        };
        let project = project.read(cx);
        let review = self.review.borrow();
        review
            .files
            .iter()
            .filter(|(_, file)| file.state == FileState::Deleted)
            .map(|(path, _)| project.relative(path))
            .filter(|relative| {
                !relative.as_os_str().is_empty() && !project.worktree().contains(relative)
            })
            .collect()
    }

    /// The tree state, for tests and for the status bar.
    pub fn tree_state(&self) -> &Entity<TreeState> {
        &self.tree
    }

    /// How many rows the tree shows right now (expanded folders included).
    ///
    /// gpui-kit's `TreeState` exposes entries one by one rather than as a
    /// slice, so this walks them; it is meant for the tests and for the odd
    /// diagnostic, not for rendering.
    pub fn visible_rows(&self, cx: &App) -> usize {
        let state = self.tree.read(cx);
        let mut count = 0;
        while state.entry(count).is_some() {
            count += 1;
        }
        count
    }

    /// The labels of the visible rows, for the tests.
    pub fn visible_labels(&self, cx: &App) -> Vec<String> {
        let state = self.tree.read(cx);
        let mut labels = Vec::new();
        let mut index = 0;
        while let Some(entry) = state.entry(index) {
            labels.push(entry.item().label.to_string());
            index += 1;
        }
        labels
    }

    /// Whether the keyboard focus is anywhere inside the panel: the tree, its
    /// context menu or the panel itself
    /// (`docs/specs/07-etapa5-productividad.md` §7.1).
    ///
    /// Answers from the last rendered frame, like every GPUI focus query.
    pub fn contains_focus(&self, window: &Window, cx: &App) -> bool {
        self.focus_handle.contains_focused(window, cx)
    }

    /// "Enfocar" the panel (§7.1): the selected row of the tree, or the first
    /// one when nothing is selected, so the arrows work right away. With an
    /// empty tree or no project, the panel itself.
    pub fn focus_selected_or_first(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.project.is_none() || self.tree.read(cx).entry(0).is_none() {
            window.focus(&self.focus_handle, cx);
            return;
        }
        self.tree.update(cx, |tree, cx| {
            if tree.selected_index().is_none() {
                tree.set_selected_index(Some(0), cx);
            }
            tree.focus(window, cx);
        });
    }

    /// Opens the file at `relative` as if the tree had been clicked. Used by
    /// the tests, which cannot synthesize a double click.
    pub fn open_for_test(&mut self, relative: &Path, pin: bool, cx: &mut Context<Self>) {
        self.open(relative, pin, cx);
    }

    /// Rebuilds the tree items from the worktree.
    pub fn rebuild(&mut self, cx: &mut Context<Self>) {
        // `CINCEL_TRACE_TIMINGS=1` (`crate::bench::TIMING_TARGET`).
        let _tree_rebuild = tracing::info_span!(target: "cincel::timing", "tree_rebuild").entered();
        self.ghosts = self.ghost_paths(cx);
        let items = match &self.project {
            Some(project) => {
                let project = project.read(cx);
                build_items(
                    project.worktree(),
                    Path::new(""),
                    &self.expanded,
                    &self.ghosts,
                )
            }
            None => Vec::new(),
        };
        self.tree.update(cx, |tree, cx| {
            let selected = tree.selected_item().map(|item| item.id.clone());
            tree.set_items(items, cx);
            if let Some(id) = selected
                && let Some(index) = tree.index_of(&id)
            {
                tree.set_selected_index(Some(index), cx);
            }
        });
        cx.notify();
    }

    fn on_tree_event(&mut self, _: Entity<TreeState>, event: &TreeEvent, cx: &mut Context<Self>) {
        match event {
            TreeEvent::Expanded(id) => {
                self.expanded.insert(path_of(id));
            }
            TreeEvent::Collapsed(id) => {
                self.expanded.remove(&path_of(id));
            }
        }
        self.rebuild(cx);
    }

    /// Asks the workspace to open `relative`, previewing or pinning it.
    fn open(&mut self, relative: &Path, pin: bool, cx: &mut Context<Self>) {
        let Some(project) = &self.project else {
            return;
        };
        let path = project.read(cx).absolute(relative);
        if path.is_dir() {
            return;
        }
        tracing::debug!(path = %path.display(), pin, "abriendo desde el árbol");
        cx.emit(WorkspaceEvent::OpenFile { path, pin });
    }

    /// `Enter`: opens the selected file, pinned.
    fn on_open_selected(
        &mut self,
        _: &crate::actions::OpenSelected,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(selected) = self
            .tree
            .read(cx)
            .selected_item()
            .map(|item| path_of(&item.id))
        else {
            return;
        };
        self.open(&selected, true, cx);
    }

    /// The path the context menu (or, failing that, the selection) points at.
    fn target(&self, cx: &App) -> Option<PathBuf> {
        self.context_target.clone().or_else(|| {
            self.tree
                .read(cx)
                .selected_item()
                .map(|item| path_of(&item.id))
        })
    }

    /// The base folder for "Nuevo archivo" (`docs/specs/07-etapa5-
    /// productividad.md` §8.2, D6, E5-I): the folder selected in the tree,
    /// the folder containing the selected file, or `None` with nothing
    /// selected (the caller falls back to the project root).
    pub fn new_file_base_folder(&self, cx: &App) -> Option<PathBuf> {
        let project = self.project.as_ref()?;
        let relative = self.target(cx)?;
        let absolute = project.read(cx).absolute(&relative);
        if absolute.is_dir() {
            Some(absolute)
        } else {
            absolute.parent().map(Path::to_path_buf)
        }
    }

    fn on_reveal_in_folder(
        &mut self,
        _: &crate::actions::RevealInFolder,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((project, relative)) = self.project.clone().zip(self.target(cx)) else {
            return;
        };
        let absolute = project.read(cx).absolute(&relative);
        let folder = if absolute.is_dir() {
            absolute
        } else {
            absolute
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| project.read(cx).root().to_path_buf())
        };
        match std::process::Command::new("xdg-open").arg(&folder).spawn() {
            Ok(_) => tracing::debug!(path = %folder.display(), "carpeta revelada"),
            Err(error) => {
                tracing::warn!(%error, "no se pudo abrir el explorador de archivos");
                crate::toast::error("No se pudo abrir el explorador de archivos", cx);
            }
        }
    }

    fn on_copy_path(
        &mut self,
        _: &crate::actions::CopyPath,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((project, relative)) = self.project.clone().zip(self.target(cx)) else {
            return;
        };
        let absolute = project.read(cx).absolute(&relative);
        copy(&absolute.to_string_lossy(), cx);
    }

    fn on_copy_relative_path(
        &mut self,
        _: &crate::actions::CopyRelativePath,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(relative) = self.target(cx) else {
            return;
        };
        copy(&relative.to_string_lossy(), cx);
    }

    /// "Mencionar en el chat": the context-menu stand-in for dragging a file
    /// from the tree onto the chat composer (`docs/etapas/etapa-2.md`,
    /// `gpui-kit`'s tree has no drag source to hook a cross-panel drop onto).
    fn on_mention_in_chat(
        &mut self,
        _: &crate::actions::MentionInChat,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((project, relative)) = self.project.clone().zip(self.target(cx)) else {
            return;
        };
        let absolute = project.read(cx).absolute(&relative);
        if absolute.is_dir() {
            return;
        }
        cx.emit(WorkspaceEvent::MentionFile { path: absolute });
    }
}

/// Puts `text` on the clipboard and says so.
fn copy(text: &str, cx: &mut App) {
    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.to_owned()));
    crate::toast::info_keyed("copy-path", format!("Ruta copiada: {text}"), cx);
}

/// The relative path an item id encodes.
fn path_of(id: &SharedString) -> PathBuf {
    let id = id.as_ref();
    PathBuf::from(id.strip_suffix(PLACEHOLDER_SUFFIX).unwrap_or(id))
}

/// Whether an item id is the placeholder child of a folder.
fn is_placeholder(id: &SharedString) -> bool {
    id.as_ref().ends_with(PLACEHOLDER_SUFFIX)
}

/// Builds the items of one directory, recursing into the expanded ones.
///
/// `ghosts` are pending deletions the worktree no longer lists (relative
/// paths): they are merged in among the files of their folder, and a folder
/// the agent removed with them is shown too, so every deletion stays
/// visible until it is decided.
fn build_items(
    worktree: &Worktree,
    dir: &Path,
    expanded: &HashSet<PathBuf>,
    ghosts: &BTreeSet<PathBuf>,
) -> Vec<TreeItem> {
    // `(is_dir, name, relative path)` in the worktree's order: folders
    // first, then files.
    let mut children: Vec<(bool, String, PathBuf)> = worktree
        .children(dir)
        .map(|entry| {
            (
                entry.kind == EntryKind::Dir,
                entry.name().to_string(),
                entry.path.clone(),
            )
        })
        .collect();
    let mut missing: BTreeMap<PathBuf, bool> = BTreeMap::new();
    for ghost in ghosts {
        let Ok(rest) = ghost.strip_prefix(dir) else {
            continue;
        };
        let Some(first) = rest.components().next() else {
            continue;
        };
        let child = dir.join(first);
        if worktree.contains(&child) {
            continue;
        }
        *missing.entry(child.clone()).or_default() |= &child != ghost;
    }
    for (path, is_dir) in missing {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        let key = name.to_lowercase();
        // Among its kind, alphabetically (case-insensitive).
        let at = children
            .iter()
            .position(|(other_dir, other, _)| {
                (*other_dir == is_dir && other.to_lowercase() > key) || (is_dir && !*other_dir)
            })
            .unwrap_or(children.len());
        children.insert(at, (is_dir, name, path));
    }
    children
        .into_iter()
        .map(|(is_dir, name, path)| {
            let id = SharedString::from(path.to_string_lossy().to_string());
            let item = TreeItem::new(id.clone(), SharedString::from(name));
            if !is_dir {
                return item;
            }
            let is_expanded = expanded.contains(&path);
            let children = if is_expanded {
                build_items(worktree, &path, expanded, ghosts)
            } else {
                Vec::new()
            };
            let item = if children.is_empty() {
                // A folder needs at least one child to be drawn as a folder.
                // A collapsed one never shows it; an empty one says so.
                item.child(
                    TreeItem::new(
                        SharedString::from(format!("{id}{PLACEHOLDER_SUFFIX}")),
                        SharedString::from("(vacía)"),
                    )
                    .disabled(true),
                )
            } else {
                item.children(children)
            };
            item.expanded(is_expanded)
        })
        .collect()
}

/// The icon of a file, by extension (`docs/specs/02-visual.md` §4: Lucide).
///
/// `pub(crate)` so the file finder (`crate::file_finder`, E5-F) can reuse the
/// exact same icon the tree shows (§3.2).
pub(crate) fn file_icon(path: &Path) -> IconName {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "rs" | "c" | "h" | "cpp" | "go" | "py" | "java" | "js" | "jsx" | "ts" | "tsx" | "rb"
        | "php" | "swift" | "kt" | "lua" | "zig" => IconName::FileCode,
        "json" | "toml" | "yaml" | "yml" | "ini" | "conf" | "lock" => IconName::FileCog,
        "md" | "markdown" | "txt" | "rst" | "adoc" => IconName::FileText,
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "ico" | "bmp" => IconName::FileImage,
        "sh" | "bash" | "zsh" | "fish" => IconName::FileTerminal,
        _ => IconName::File,
    }
}

/// The color a row's name takes, from the git status
/// (`docs/specs/02-visual.md` §6.4; the agent colors arrive in stage 3).
fn status_color(status: Option<GitFileStatus>, theme: &ThemeColors) -> Option<gpui::Hsla> {
    match status? {
        GitFileStatus::Untracked | GitFileStatus::Added => Some(theme.status_ok),
        GitFileStatus::Modified => Some(theme.status_warning),
        GitFileStatus::Deleted | GitFileStatus::Conflicted => Some(theme.status_error),
        GitFileStatus::Ignored => Some(theme.text_muted),
    }
}

/// The color of a file with pending agent changes (`02-visual.md` §6.4).
fn review_color(state: FileState, theme: &ThemeColors) -> gpui::Hsla {
    match state {
        FileState::Created => theme.status_ok,
        FileState::Modified => theme.status_warning,
        FileState::Deleted => theme.status_error,
    }
}

impl EventEmitter<PanelEvent> for FilesPanel {}
impl EventEmitter<WorkspaceEvent> for FilesPanel {}

impl Focusable for FilesPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for FilesPanel {
    fn panel_name(&self) -> &'static str {
        "FilesPanel"
    }

    fn closable(&self, _: &App) -> bool {
        // Closing it would leave no way to bring it back; `Ctrl+Shift+E`
        // collapses the whole dock instead.
        false
    }
}

impl Panel for FilesPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        SharedString::from("Archivos")
    }
}

impl Render for FilesPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = cx.entity().downgrade();
        let project = self.project.clone();
        let theme = ThemeColors::global(cx).clone();
        let tree_state = self.tree.clone();

        let body = match project.clone() {
            None => div()
                .p_3()
                .text_color(cx.theme().muted_foreground)
                .child("Abrí una carpeta para ver el árbol.")
                .into_any_element(),
            Some(project) => {
                // gpui-kit keeps this closure inside the `TreeState` entity
                // between frames, so every handle it captures has to be weak:
                // a strong `Entity<TreeState>` in here would be a cycle and
                // the tree would never be released.
                let render_panel = panel.clone();
                let render_project = project.downgrade();
                let render_theme = theme.clone();
                let click_state = tree_state.downgrade();
                let render_review = self.review.clone();
                tree(&self.tree, move |_, entry, _, _, cx| {
                    render_row(
                        entry,
                        &render_project,
                        &render_theme,
                        &render_panel,
                        &click_state,
                        &render_review,
                        cx,
                    )
                })
                .context_menu({
                    let panel = panel.clone();
                    move |_, entry, menu, _, cx| {
                        let relative = path_of(&entry.item().id);
                        let _ = panel.update(cx, |panel, _| {
                            panel.context_target = Some(relative);
                        });
                        menu.menu(
                            "Revelar en carpeta",
                            Box::new(crate::actions::RevealInFolder),
                        )
                        .menu("Copiar ruta", Box::new(crate::actions::CopyPath))
                        .menu(
                            "Copiar ruta relativa",
                            Box::new(crate::actions::CopyRelativePath),
                        )
                        .menu(
                            "Mencionar en el chat",
                            Box::new(crate::actions::MentionInChat),
                        )
                    }
                })
                .size_full()
                .into_any_element()
            }
        };

        // The root line: the project and its total of pending changes
        // (`02-visual.md` §6.4, "Raíz: contador total").
        let header = project.as_ref().map(|project| {
            let root = project.read(cx).root();
            let name = root
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| root.display().to_string());
            let pending = self.review.borrow().pending;
            let totals = self.root_review_label();
            let scale = crate::settings::ui_scale(cx);
            div()
                .id("files-root")
                .h(px(ROW_HEIGHT * scale))
                .flex_none()
                .px_2()
                .flex()
                .items_center()
                .gap_2()
                .text_color(theme.text_muted)
                .text_size(px(12. * scale))
                .child(SharedString::from(name))
                .when(pending > 0, |this| {
                    this.children(totals.map(|totals| {
                        div()
                            .text_size(px(11. * scale))
                            .text_color(theme.status_warning)
                            .child(SharedString::from(totals))
                    }))
                })
        });

        v_flex()
            .id("files-panel")
            .key_context("FilesPanel")
            .track_focus(&self.focus_handle)
            // Clicking anywhere in the panel (a folder row included) gives the
            // tree the focus, which is what makes the arrow keys work.
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _: &gpui::MouseDownEvent, window, cx| {
                    this.tree.update(cx, |tree, cx| tree.focus(window, cx));
                }),
            )
            .on_action(cx.listener(Self::on_open_selected))
            .on_action(cx.listener(Self::on_reveal_in_folder))
            .on_action(cx.listener(Self::on_copy_path))
            .on_action(cx.listener(Self::on_copy_relative_path))
            .on_action(cx.listener(Self::on_mention_in_chat))
            .size_full()
            .bg(theme.bg_app)
            .children(header)
            .child(div().flex_1().min_h_0().child(body))
    }
}

/// One row of the tree.
fn render_row(
    entry: &TreeEntry,
    project: &WeakEntity<Project>,
    theme: &ThemeColors,
    panel: &WeakEntity<FilesPanel>,
    tree_state: &WeakEntity<TreeState>,
    review: &SharedSummary,
    cx: &mut App,
) -> ListItem {
    let item = entry.item();
    let Some(project) = project.upgrade() else {
        return ListItem::new(item.id.clone()).child(item.label.clone());
    };
    let relative = path_of(&item.id);
    let placeholder = is_placeholder(&item.id);
    let is_folder = entry.is_folder() && !placeholder;
    let expanded = entry.is_expanded();

    let (status, has_changes_under, absolute) = {
        let project = project.read(cx);
        let absolute = project.absolute(&relative);
        (
            project.git_status(&absolute),
            is_folder && project.has_changes_under(&absolute),
            absolute,
        )
    };
    let (agent, pending_under) = {
        let review = review.borrow();
        (
            review.file(&absolute).cloned(),
            is_folder && review.has_pending_under(&absolute),
        )
    };
    let style = agent
        .as_ref()
        .filter(|_| !placeholder)
        .map(|file| review_row_style(file, theme));
    let color = if placeholder {
        Some(theme.text_muted)
    } else if let Some(style) = &style {
        Some(style.color)
    } else {
        status_color(status, theme)
    };
    let strikethrough = style.as_ref().is_some_and(|style| style.strikethrough);
    let tooltip = style.as_ref().and_then(|style| style.tooltip);
    let suffix = style.map(|style| style.suffix);
    let has_changes_under = has_changes_under || pending_under;

    let icon = if placeholder {
        None
    } else if is_folder {
        Some(if expanded {
            IconName::FolderOpen
        } else {
            IconName::Folder
        })
    } else {
        Some(file_icon(&relative))
    };

    let chevron = if is_folder {
        Some(if expanded {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        })
    } else {
        None
    };

    // Every pixel size follows the zoom (`workspace::zoom_*`).
    let scale = crate::settings::ui_scale(cx);
    let mut row = ListItem::new(item.id.clone())
        .h(px(ROW_HEIGHT * scale))
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .w_full()
                .pl(px(entry.depth() as f32 * INDENT * scale))
                .child(
                    div()
                        .w(px(14. * scale))
                        .flex_none()
                        .text_color(theme.text_muted)
                        .children(chevron),
                )
                .child(
                    div()
                        .w(px(16. * scale))
                        .flex_none()
                        .text_color(color.unwrap_or(theme.text_muted))
                        .children(icon),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .when_some(color, |this, color| this.text_color(color))
                        // A file the agent deleted, still pending.
                        .when(strikethrough, |this| this.line_through())
                        .child(item.label.clone()),
                )
                .children(suffix.map(|suffix| {
                    div()
                        .flex_none()
                        .mr_1()
                        .text_size(px(11. * scale))
                        .text_color(theme.text_muted)
                        .child(suffix)
                }))
                .when(has_changes_under, |this| {
                    // A folder whose children changed gets a 6 px dot
                    // (`02-visual.md` §6.4).
                    this.child(
                        div()
                            .flex_none()
                            .mr_1()
                            .size(px(6. * scale))
                            .rounded_full()
                            .bg(theme.status_warning),
                    )
                }),
        );

    if !placeholder && !is_folder {
        let panel = panel.clone();
        let tree_state = tree_state.clone();
        row = row.on_click(move |event: &ClickEvent, window, cx| {
            // A double click arrives as two events (count 1, then 2): the
            // first previews, the second pins, which is exactly the behavior
            // `modulos/workspace.md` asks for.
            let pin = event.click_count() >= 2;
            let _ = tree_state.update(cx, |tree, cx| tree.focus(window, cx));
            let _ = panel.update(cx, |panel, cx| panel.open(&relative, pin, cx));
        });
    }
    if let Some(tooltip) = tooltip {
        row = row.tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx));
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;
    use cincel_project::WorktreeConfig;

    fn tree_of(dir: &Path) -> Worktree {
        Worktree::scan_blocking(dir, WorktreeConfig::default()).unwrap()
    }

    #[test]
    fn builds_the_root_with_collapsed_folders() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();

        let worktree = tree_of(dir.path());
        let items = build_items(&worktree, Path::new(""), &HashSet::new(), &BTreeSet::new());
        assert_eq!(items.len(), 2);
        // Folders first.
        assert_eq!(items[0].label, "src");
        assert!(items[0].is_folder());
        assert!(!items[0].is_expanded());
        // The collapsed folder carries only its placeholder.
        assert_eq!(items[0].children.len(), 1);
        assert!(is_placeholder(&items[0].children[0].id));
        assert_eq!(items[1].label, "Cargo.toml");
        assert!(!items[1].is_folder());
    }

    #[test]
    fn an_expanded_folder_shows_its_children() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();

        let worktree = tree_of(dir.path());
        let expanded = HashSet::from([PathBuf::from("src")]);
        let items = build_items(&worktree, Path::new(""), &expanded, &BTreeSet::new());
        assert!(items[0].is_expanded());
        assert_eq!(items[0].children.len(), 1);
        assert_eq!(items[0].children[0].label, "main.rs");
        assert_eq!(
            path_of(&items[0].children[0].id),
            PathBuf::from("src/main.rs")
        );
    }

    /// Pending deletions the worktree no longer lists keep their row, in
    /// order among the files, and a folder removed with them shows too.
    #[test]
    fn pending_deletions_stay_in_the_tree() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("a.py"), "").unwrap();
        std::fs::write(dir.path().join("z.py"), "").unwrap();

        let worktree = tree_of(dir.path());
        let ghosts = BTreeSet::from([PathBuf::from("m.py"), PathBuf::from("viejo/uno.py")]);
        let expanded = HashSet::from([PathBuf::from("viejo")]);
        let items = build_items(&worktree, Path::new(""), &expanded, &ghosts);
        let labels: Vec<&str> = items.iter().map(|item| item.label.as_ref()).collect();
        assert_eq!(labels, ["src", "viejo", "a.py", "m.py", "z.py"]);
        assert!(items[1].is_folder());
        assert_eq!(items[1].children[0].label, "uno.py");
        assert_eq!(
            path_of(&items[1].children[0].id),
            PathBuf::from("viejo/uno.py")
        );
    }

    #[test]
    fn icons_follow_the_extension() {
        assert!(matches!(file_icon(Path::new("a/b.rs")), IconName::FileCode));
        assert!(matches!(
            file_icon(Path::new("Cargo.toml")),
            IconName::FileCog
        ));
        assert!(matches!(
            file_icon(Path::new("LEEME.md")),
            IconName::FileText
        ));
        assert!(matches!(
            file_icon(Path::new("sin-extension")),
            IconName::File
        ));
    }

    #[test]
    fn git_colors_follow_the_spec() {
        let theme = ThemeColors::default();
        assert_eq!(
            status_color(Some(GitFileStatus::Untracked), &theme),
            Some(theme.status_ok)
        );
        assert_eq!(
            status_color(Some(GitFileStatus::Modified), &theme),
            Some(theme.status_warning)
        );
        assert_eq!(
            status_color(Some(GitFileStatus::Conflicted), &theme),
            Some(theme.status_error)
        );
        assert_eq!(status_color(None, &theme), None);
    }
}
