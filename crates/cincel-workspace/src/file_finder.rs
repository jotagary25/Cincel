//! The quick file finder (`Ctrl+P`, `docs/specs/07-etapa5-productividad.md`
//! §3): a floating palette, anchored top-center over the editor area, that
//! fuzzy-matches the project's relative paths with `frizbee` (D1) and opens
//! the pick as a preview tab.
//!
//! [`rank`] is the pure, GPUI-free half: scoring, tie-breaking and the
//! empty-query fallback, all testable without an `App` (see
//! `crate::file_finder_tests`). [`FileFinder`] is the entity that owns the
//! floating panel, the candidate list (refreshed from
//! [`cincel_project::Worktree`] whenever it opens or the tree changes) and a
//! best-effort "most recently used" order built by watching which tab the
//! center panel activates — `CenterPanel` keeps no such history of its own,
//! so this is the smallest way to get one without touching that file (only
//! new files are safe to touch while other sub-stages work on
//! `cincel-workspace` in parallel).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use cincel_project::EntryKind;
use gpui::{
    App, Context, Entity, FocusHandle, Focusable, HighlightStyle, Hsla, Subscription, Task,
    TextOverflow, WhiteSpace, Window,
};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{FontWeight, SharedString, StyledText, div, px};

use crate::center::CenterPanel;
use crate::project::{Project, ProjectEvent};
use crate::theme::ThemeColors;
use crate::tree_panel::file_icon;
use crate::workspace::Workspace;

/// The GPUI key context of the floating panel; `keymap.rs` binds the
/// navigation keys against it (`built_in_bindings`, like the modals).
pub const KEY_CONTEXT: &str = "FileFinder";

/// §3.1: "tope 200 filas" / "Tope de 200 resultados" for both the
/// empty-query and the fuzzy-match paths.
const MAX_RESULTS: usize = 200;
/// §3.1: above this many candidates, filtering moves to the background
/// executor so a keystroke never blocks the frame.
const BACKGROUND_THRESHOLD: usize = 5_000;

/// §3.2 geometry, all multiplied by `ui_scale`.
const TOP_OFFSET: f32 = 72.;
const MAX_WIDTH: f32 = 560.;
const WINDOW_MARGIN: f32 = 48.;
const MAX_HEIGHT: f32 = 420.;
const INPUT_HEIGHT: f32 = 32.;
const ROW_HEIGHT: f32 = 28.;
const ICON_SIZE: f32 = 14.;

/// `file_finder::select_next`.
#[derive(Clone, Copy, Debug, Default, PartialEq, gpui::Action)]
#[action(namespace = file_finder, name = "select_next")]
pub struct SelectNext;
/// `file_finder::select_prev`.
#[derive(Clone, Copy, Debug, Default, PartialEq, gpui::Action)]
#[action(namespace = file_finder, name = "select_prev")]
pub struct SelectPrev;
/// `file_finder::page_down`.
#[derive(Clone, Copy, Debug, Default, PartialEq, gpui::Action)]
#[action(namespace = file_finder, name = "page_down")]
pub struct PageDown;
/// `file_finder::page_up`.
#[derive(Clone, Copy, Debug, Default, PartialEq, gpui::Action)]
#[action(namespace = file_finder, name = "page_up")]
pub struct PageUp;
/// `file_finder::confirm`: opens the selected row as a preview tab.
#[derive(Clone, Copy, Debug, Default, PartialEq, gpui::Action)]
#[action(namespace = file_finder, name = "confirm")]
pub struct Confirm;
/// `file_finder::dismiss`: closes without opening anything.
#[derive(Clone, Copy, Debug, Default, PartialEq, gpui::Action)]
#[action(namespace = file_finder, name = "dismiss")]
pub struct Dismiss;

/// One suggestion: the project-relative path (`/`-separated) and the byte
/// offsets of the characters that matched the query, ascending and already
/// snapped to `char` boundaries, ready to highlight.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinderMatch {
    /// Path relative to the project root.
    pub relative: Arc<str>,
    /// Byte offsets into `relative` of the matched characters, one per
    /// character (never a whole multi-byte character split in two).
    pub positions: Vec<usize>,
}

/// Fuzzy-ranks `candidates` against `query`, or (when `query` is empty)
/// returns `recent` first — deduplicated against `candidates` — followed by
/// the rest of `candidates` in their given (tree) order (§3.1).
///
/// Pure and GPUI-free: the caller decides what counts as "recent" (the
/// workspace's tab-activation history) and what order `candidates` arrive in
/// (the worktree's display order).
pub fn rank(
    candidates: &[Arc<str>],
    query: &str,
    recent: &[Arc<str>],
    limit: usize,
) -> Vec<FinderMatch> {
    if query.trim().is_empty() {
        empty_query_matches(candidates, recent, limit)
    } else {
        fuzzy_matches(candidates, query, limit)
    }
}

fn empty_query_matches(
    candidates: &[Arc<str>],
    recent: &[Arc<str>],
    limit: usize,
) -> Vec<FinderMatch> {
    let known: HashSet<&str> = candidates.iter().map(Arc::as_ref).collect();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut out = Vec::new();
    for path in recent {
        if out.len() >= limit {
            break;
        }
        if known.contains(path.as_ref()) && seen.insert(path.as_ref()) {
            out.push(FinderMatch {
                relative: path.clone(),
                positions: Vec::new(),
            });
        }
    }
    for path in candidates {
        if out.len() >= limit {
            break;
        }
        if seen.insert(path.as_ref()) {
            out.push(FinderMatch {
                relative: path.clone(),
                positions: Vec::new(),
            });
        }
    }
    out
}

fn fuzzy_matches(candidates: &[Arc<str>], query: &str, limit: usize) -> Vec<FinderMatch> {
    if candidates.is_empty() {
        return Vec::new();
    }
    let haystacks: Vec<&str> = candidates.iter().map(Arc::as_ref).collect();
    let mut matcher = frizbee::Matcher::new(query, &frizbee::Config::default());
    let mut matches = matcher.match_list_indices(&haystacks);

    // Score descending; ties broken by path length (chars) then plain
    // alphabetical order (§3.1). `match_list_indices` does not sort by
    // `radix_sort_matches` (that helper only takes `Vec<Match>`, without
    // indices), so the tie-break is applied here instead.
    //
    // Sorted and truncated to `limit` *before* `char_indices_to_byte_offsets`
    // runs on anything: a short query against a huge corpus can match nearly
    // every candidate (§5.5's own perf test caught this), and converting
    // every one of those matches' positions only to throw all but `limit` of
    // them away is most of that cost for nothing (`docs/specs/08-etapa6-
    // cierre-1-0.md` §5.5).
    matches.sort_by(|a, b| {
        let path_a = &candidates[a.index as usize];
        let path_b = &candidates[b.index as usize];
        b.score
            .cmp(&a.score)
            .then_with(|| path_a.chars().count().cmp(&path_b.chars().count()))
            .then_with(|| path_a.cmp(path_b))
    });

    matches
        .into_iter()
        .take(limit)
        .map(|found| {
            let candidate = candidates[found.index as usize].clone();
            let positions = char_indices_to_byte_offsets(&candidate, &found.indices);
            FinderMatch {
                relative: candidate,
                positions,
            }
        })
        .collect()
}

/// frizbee gives the matched positions in reverse order, and a multi-byte
/// character can appear more than once (once per matched byte of it, see the
/// crate's own `unicode_indices_expand_multibyte_scalars` test). This snaps
/// every raw offset down to the start of its UTF-8 character and removes the
/// duplicates that snapping produces, so the result is exactly one ascending
/// byte offset per matched `char` — safe to slice `haystack` at (D1: "se
/// convierten a offsets de byte en límite de char").
fn char_indices_to_byte_offsets(haystack: &str, indices_desc: &[u32]) -> Vec<usize> {
    let mut ascending: Vec<usize> = indices_desc.iter().rev().map(|&i| i as usize).collect();
    for offset in &mut ascending {
        while *offset > 0 && !haystack.is_char_boundary(*offset) {
            *offset -= 1;
        }
    }
    ascending.dedup();
    ascending
}

/// Splits a relative path into its directory (may be empty) and file name,
/// for the two-column row of §3.2.
fn split_relative(relative: &str) -> (&str, &str) {
    match relative.rfind('/') {
        Some(index) => (&relative[..index], &relative[index + 1..]),
        None => ("", relative),
    }
}

/// The floating panel over the center of the window (§3.2).
///
/// Kept alive for the whole life of the open project (built once in
/// `Workspace::open_project`, like the review panel), so the typed query and
/// the tab-activation history survive between `Ctrl+P` presses within the
/// same project — opening a different project starts a fresh one.
pub struct FileFinder {
    project: Entity<Project>,
    center: Entity<CenterPanel>,
    query: Entity<InputState>,
    open: bool,
    /// Every file the worktree knows about, relative paths, refreshed on
    /// open and on `ProjectEvent::TreeChanged` while open (§3.1).
    candidates: Arc<Vec<Arc<str>>>,
    results: Vec<FinderMatch>,
    selected: usize,
    previous_focus: Option<FocusHandle>,
    filter_task: Option<Task<()>>,
    /// Absolute paths, oldest activation first; the tab the center panel
    /// shows right now is always last (see the module doc).
    activation_order: Vec<PathBuf>,
    /// Bumped on every re-filter; a background result whose generation is
    /// stale (a newer keystroke landed first) is discarded (§3.1).
    generation: u64,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl FileFinder {
    /// Builds a closed finder over `project`, watching `center` for its
    /// tab-activation history.
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
        let query =
            cx.new(|cx| InputState::new(window, cx).placeholder("Buscar archivos por nombre…"));
        let subscriptions = vec![
            cx.subscribe_in(&query, window, |this, _, event, window, cx| match event {
                InputEvent::PressEnter { .. } => this.confirm(window, cx),
                InputEvent::Change => this.recompute(cx),
                _ => {}
            }),
            cx.subscribe(&project, |this, _, event, cx| {
                if this.open && matches!(event, ProjectEvent::TreeChanged) {
                    this.refresh_candidates(cx);
                    this.recompute(cx);
                }
            }),
            cx.observe(&center, |this, _, cx| this.track_activation(cx)),
        ];

        Self {
            project,
            center,
            query,
            open: false,
            candidates: Arc::new(Vec::new()),
            results: Vec::new(),
            selected: 0,
            previous_focus: None,
            filter_task: None,
            activation_order: Vec::new(),
            generation: 0,
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// Whether the panel is on screen.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The current results, for the tests.
    pub fn results(&self) -> &[FinderMatch] {
        &self.results
    }

    /// Whether a background filter is still running, so [`Self::results`]
    /// may not match the query yet (`cincel --bench finder`).
    pub fn is_filtering(&self) -> bool {
        self.filter_task.is_some()
    }

    /// The selected row, for the tests.
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// The query field, for the tests.
    pub fn query(&self) -> &Entity<InputState> {
        &self.query
    }

    /// Seeds [`Self::candidates`] directly and re-filters, without a real
    /// worktree behind it. Used by `file_finder_perf_tests.rs`
    /// (`docs/specs/08-etapa6-cierre-1-0.md` §5.5) to exercise the
    /// background-filtering path (`BACKGROUND_THRESHOLD`) at a corpus size
    /// (50 000 paths) that would be needlessly slow and disk-heavy to build
    /// through an actual project directory.
    pub fn set_candidates_for_test(&mut self, candidates: Vec<Arc<str>>, cx: &mut Context<Self>) {
        self.candidates = Arc::new(candidates);
        self.recompute(cx);
    }

    /// Opens the panel: fresh candidates, the previous query preselected
    /// (§3.1: "conservado... preseleccionado para reemplazarlo al
    /// escribir"), and the field focused. `previous_focus` is what the
    /// caller had before the shortcut fired, restored by [`Self::dismiss`].
    pub fn open(
        &mut self,
        previous_focus: Option<FocusHandle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open = true;
        self.previous_focus = previous_focus;
        self.refresh_candidates(cx);
        self.recompute(cx);
        self.query
            .update(cx, |query, cx| query.select_all(window, cx));
        let handle = self.query.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    /// Closes without opening anything, giving the keyboard back to
    /// whatever had it before (§3.1: "Al cerrar sin abrir, el foco vuelve a
    /// donde estaba").
    pub fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        self.open = false;
        self.filter_task = None;
        if let Some(previous) = self.previous_focus.take() {
            window.focus(&previous, cx);
        }
        cx.notify();
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(found) = self.results.get(self.selected) else {
            return;
        };
        let relative = found.relative.clone();
        let absolute = self.project.read(cx).absolute(Path::new(relative.as_ref()));
        self.open = false;
        self.filter_task = None;
        // The focus goes to the opened editor, not back to `previous_focus`
        // (§3.1: "Enter abre el seleccionado... y el foco queda en su
        // editor"); `CenterPanel::activate` (through `open_file`) does that.
        self.previous_focus = None;
        self.center.update(cx, |center, cx| {
            center.open_file(&absolute, false, window, cx)
        });
        cx.notify();
    }

    fn move_selection(&mut self, delta: i64, cx: &mut Context<Self>) {
        if self.results.is_empty() {
            return;
        }
        let len = self.results.len() as i64;
        let next = (self.selected as i64 + delta).rem_euclid(len);
        self.selected = next as usize;
        cx.notify();
    }

    /// Rebuilds [`Self::candidates`] from the worktree's files (folders,
    /// `.git`, `.gitignore` and `files.exclude` are already out: the
    /// worktree is the same one the tree panel paints, so the finder
    /// inherits its filtering exactly, §3.1).
    fn refresh_candidates(&mut self, cx: &mut Context<Self>) {
        let candidates: Vec<Arc<str>> = self
            .project
            .read(cx)
            .worktree()
            .entries()
            .filter(|entry| entry.kind == EntryKind::File)
            .map(|entry| Arc::<str>::from(entry.path.to_string_lossy().as_ref()))
            .collect();
        self.candidates = Arc::new(candidates);
    }

    /// Re-filters [`Self::candidates`] against the query field, in the
    /// background above [`BACKGROUND_THRESHOLD`] candidates (§3.1).
    fn recompute(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        let query = self.query.read(cx).value().to_string();
        let candidates = self.candidates.clone();
        let recent = self.recent_relative_paths(cx);

        if candidates.len() > BACKGROUND_THRESHOLD {
            let compute = cx.background_executor().spawn(async move {
                let start = Instant::now();
                let results = rank(&candidates, &query, &recent, MAX_RESULTS);
                (results, start.elapsed())
            });
            self.filter_task = Some(cx.spawn(async move |this, cx| {
                let (results, elapsed) = compute.await;
                let _ = this.update(cx, |this, cx| {
                    // A newer keystroke already started another filter.
                    if this.generation != generation {
                        return;
                    }
                    tracing::debug!(
                        ?elapsed,
                        count = results.len(),
                        "buscador de archivos: filtrado en segundo plano"
                    );
                    this.results = results;
                    this.selected = 0;
                    this.filter_task = None;
                    cx.notify();
                });
            }));
        } else {
            self.filter_task = None;
            let start = Instant::now();
            self.results = rank(&candidates, &query, &recent, MAX_RESULTS);
            tracing::debug!(
                elapsed = ?start.elapsed(),
                count = self.results.len(),
                "buscador de archivos: filtrado"
            );
            self.selected = 0;
            cx.notify();
        }
    }

    /// The "most recently used" list of §3.1, built from
    /// [`Self::activation_order`] restricted to tabs still open: most recent
    /// first, the active tab last (so `Ctrl+P Enter` lands on the *previous*
    /// tab, like Zed).
    fn recent_relative_paths(&self, cx: &App) -> Vec<Arc<str>> {
        let project = self.project.read(cx);
        let open: HashSet<&PathBuf> = self
            .center
            .read(cx)
            .tabs()
            .iter()
            .map(|tab| &tab.path)
            .collect();
        let mut recent: Vec<Arc<str>> = self
            .activation_order
            .iter()
            .rev()
            .skip(1)
            .filter(|path| open.contains(path))
            .map(|path| relative_arc(project, path))
            .collect();
        if let Some(active) = self
            .activation_order
            .last()
            .filter(|path| open.contains(path))
        {
            recent.push(relative_arc(project, active));
        }
        recent
    }

    /// Watches which tab the center panel shows, keeping
    /// [`Self::activation_order`] up to date (`CenterPanel` has no
    /// activation history of its own to read instead, see the module doc).
    fn track_activation(&mut self, cx: &mut Context<Self>) {
        let Some(active_path) = self
            .center
            .read(cx)
            .active_tab()
            .map(|tab| tab.path.clone())
        else {
            return;
        };
        if self.activation_order.last() == Some(&active_path) {
            return;
        }
        self.activation_order.retain(|path| *path != active_path);
        self.activation_order.push(active_path);
    }

    fn on_select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(1, cx);
    }

    fn on_select_prev(&mut self, _: &SelectPrev, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(-1, cx);
    }

    fn on_page_down(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(10, cx);
    }

    fn on_page_up(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(-10, cx);
    }

    fn on_confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm(window, cx);
    }

    fn on_dismiss(&mut self, _: &Dismiss, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss(window, cx);
    }

    fn render_row(
        &self,
        index: usize,
        item: &FinderMatch,
        theme: &ThemeColors,
        scale: f32,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let relative = item.relative.clone();
        let (dir, name) = split_relative(&relative);
        let name_start = relative.len() - name.len();
        let name_positions: Vec<usize> = item
            .positions
            .iter()
            .filter(|&&position| position >= name_start)
            .map(|&position| position - name_start)
            .collect();
        let dir_positions: Vec<usize> = item
            .positions
            .iter()
            .filter(|&&position| position < dir.len())
            .copied()
            .collect();
        let icon = file_icon(Path::new(name));
        let selected = index == self.selected;
        let dir = dir.to_string();
        let name = name.to_string();

        h_flex()
            .id(("file-finder-row", index))
            .debug_selector(move || format!("file-finder-row-{index}"))
            .h(px(ROW_HEIGHT * scale))
            .items_center()
            .gap_2()
            .px_2()
            .cursor_pointer()
            .when(selected, |row| row.bg(theme.bg_surface))
            .hover(|style| style.bg(theme.bg_surface))
            .child(
                div()
                    .w(px(ICON_SIZE * scale))
                    .flex_none()
                    .text_color(theme.text_muted)
                    .child(icon),
            )
            .child(div().flex_none().child(highlighted_text(
                SharedString::from(name),
                &name_positions,
                theme.text,
                theme.text_accent,
                13.,
                false,
                window,
            )))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .child(highlighted_text(
                        SharedString::from(dir),
                        &dir_positions,
                        theme.text_muted,
                        theme.text_accent,
                        12.,
                        true,
                        window,
                    )),
            )
            .on_click(cx.listener(move |this, _: &gpui::ClickEvent, window, cx| {
                this.selected = index;
                this.confirm(window, cx);
            }))
    }
}

/// `path` (absolute) as a `/`-separated string relative to `project`'s root.
fn relative_arc(project: &Project, path: &Path) -> Arc<str> {
    Arc::<str>::from(project.relative(path).to_string_lossy().as_ref())
}

/// Builds one row's text with the matched characters highlighted
/// (`text.accent`, weight 600, §3.2). `truncate_start` gives the directory
/// column its "elipsis al inicio" when it does not fit.
fn highlighted_text(
    text: SharedString,
    positions: &[usize],
    color: Hsla,
    accent: Hsla,
    font_size: f32,
    truncate_start: bool,
    window: &Window,
) -> StyledText {
    let mut style = window.text_style();
    style.color = color;
    style.font_size = px(font_size).into();
    if truncate_start {
        style.white_space = WhiteSpace::Nowrap;
        style.text_overflow = Some(TextOverflow::TruncateStart(SharedString::from("…")));
    }
    // Collected eagerly (not left as a lazy iterator) so the borrow of
    // `text` it needs ends here, before `text` moves into `StyledText::new`.
    let highlights: Vec<_> = positions
        .iter()
        .filter_map(|&start| {
            let ch = text.get(start..)?.chars().next()?;
            let end = start + ch.len_utf8();
            Some((
                start..end,
                HighlightStyle {
                    color: Some(accent),
                    font_weight: Some(FontWeight::SEMIBOLD),
                    ..Default::default()
                },
            ))
        })
        .collect();
    StyledText::new(text).with_default_highlights(&style, highlights)
}

impl Focusable for FileFinder {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for FileFinder {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().into_any_element();
        }
        let theme = ThemeColors::global(cx).clone();
        let scale = crate::settings::ui_scale(cx);
        let window_width: f32 = window.viewport_size().width.into();
        let width = (MAX_WIDTH * scale).min((window_width - WINDOW_MARGIN * scale).max(0.));

        let has_files = !self.candidates.is_empty();
        let query_text = self.query.read(cx).value().to_string();
        let scanning = !self.project.read(cx).worktree().is_scan_complete();

        let body: gpui::AnyElement = if self.results.is_empty() {
            let message = if !has_files {
                "Este proyecto no tiene archivos visibles (revisá files.exclude y .gitignore)"
                    .to_string()
            } else {
                format!("Ningún archivo coincide con «{query_text}»")
            };
            div()
                .p_3()
                .text_sm()
                .text_color(theme.text_muted)
                .child(message)
                .into_any_element()
        } else {
            let results = self.results.clone();
            let rows: Vec<_> = results
                .iter()
                .enumerate()
                .map(|(index, item)| self.render_row(index, item, &theme, scale, window, cx))
                .collect();
            v_flex()
                .id("file-finder-results")
                .overflow_y_scroll()
                .children(rows)
                .into_any_element()
        };

        div()
            .absolute()
            .inset_0()
            // Keeps mouse and scroll-wheel events from reaching the editor
            // behind the finder: without this, the wheel scrolled both the
            // results list and the file open underneath it.
            .occlude()
            // A transparent layer catches the outside click and closes the
            // panel; no scrim, like Zed's palette (§3.2).
            .child(
                div()
                    .id("file-finder-scrim")
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
                    .id("file-finder")
                    .debug_selector(|| "file-finder".to_string())
                    .key_context(KEY_CONTEXT)
                    .track_focus(&self.focus_handle)
                    .on_action(cx.listener(Self::on_select_next))
                    .on_action(cx.listener(Self::on_select_prev))
                    .on_action(cx.listener(Self::on_page_down))
                    .on_action(cx.listener(Self::on_page_up))
                    .on_action(cx.listener(Self::on_confirm))
                    .on_action(cx.listener(Self::on_dismiss))
                    .absolute()
                    .top(px(TOP_OFFSET * scale))
                    .left_0()
                    .right_0()
                    .mx_auto()
                    .w(px(width))
                    .max_h(px(MAX_HEIGHT * scale))
                    .rounded(px(6. * scale))
                    .bg(theme.bg_elevated)
                    .border_1()
                    .border_color(theme.border)
                    .shadow_lg()
                    .overflow_hidden()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .flex_none()
                            .p_2()
                            .border_b_1()
                            .border_color(theme.border)
                            .child(Input::new(&self.query).h(px(INPUT_HEIGHT * scale))),
                    )
                    .child(div().flex_1().min_h_0().child(body))
                    .when(scanning, |panel| {
                        panel.child(
                            div()
                                .flex_none()
                                .px_2()
                                .py_1()
                                .border_t_1()
                                .border_color(theme.border)
                                .text_size(px(11. * scale))
                                .text_color(theme.text_muted)
                                .child("Buscando archivos…"),
                        )
                    }),
            )
            .into_any_element()
    }
}

impl Workspace {
    /// `Ctrl+P` (`workspace::toggle_file_finder`, §3.1): opens the finder
    /// with a project, closes it when it is already open, or toasts when
    /// there is none.
    pub fn toggle_file_finder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(finder) = self.file_finder().cloned() else {
            crate::toast::info("Abrí una carpeta para buscar archivos", cx);
            return;
        };
        if finder.read(cx).is_open() {
            finder.update(cx, |finder, cx| finder.dismiss(window, cx));
        } else {
            let previous = window.focused(cx);
            finder.update(cx, |finder, cx| finder.open(previous, window, cx));
        }
    }
}
