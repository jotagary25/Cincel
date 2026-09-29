//! Demo of the editor: opens a real file (or a synthetic buffer) with syntax
//! highlighting and the review UI of `02-visual.md` §6 fed with a
//! [`ReviewView`], the way the workspace feeds it.
//!
//! The sample is `sample.rs` against a "base" version that differs in four
//! places: a pure addition, a modified block (2 lines out, 3 in, with word
//! diffs), a one-word change left over from a previous turn (clock on the
//! pill) and a pure deletion.
//!
//! Usage:
//! ```text
//! cargo run -p cincel-editor --example phantom
//! cargo run -p cincel-editor --example phantom -- --simulate-actions
//! cargo run -p cincel-editor --example phantom -- --turn-active
//! cargo run -p cincel-editor --example phantom -- --file src/main.rs
//! cargo run -p cincel-editor --example phantom -- --smoke-test
//! cargo run -p cincel-editor --example phantom -- --rows 50000 --soft-wrap --smoke-test
//! cargo run -p cincel-editor --example phantom -- --type-test
//! ```
//!
//! The editor never applies a review decision: it emits
//! `EditorEvent::Review(action)` and waits for a new `ReviewView`. Without a
//! flag this demo only prints the actions. With `--simulate-actions` it plays
//! the host: it keeps the base text, applies every action to the base or to
//! the buffer, recomputes the hunks with a line diff and calls `set_review`
//! again — so clicking ✓/✗, the `+`/`−` icons or `Ctrl+Enter` makes the hunk
//! go away, `Alt+Shift+U` undoes the last rejection, and typing refreshes the
//! hunks like the real store does. `--turn-active` shows the "El agente está
//! editando…" state, with every review control disabled.
//!
//! `--type-test` types into the real window for ~3 s with the render probe on
//! and reports what the frames actually painted: how many rows were shaped
//! again, and how many lost highlights they had on the previous frame. The
//! second number must be 0 — a row that goes from coloured to uncoloured and
//! back is exactly the flicker of every keystroke.
//!
//! `Ctrl+S` prints "save requested" on stdout: the real save belongs to the
//! workspace, the editor only emits the event.

use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use cincel_editor::{
    EditorEvent, EditorSettings, EditorTheme, EditorView, PhantomHunk, ReviewAction,
    ReviewHunkKind, ReviewHunkView, ReviewLineView, ReviewView, ReviewWordDiffs, SharedBuffer,
    bind_default_keys, shared,
};
use cincel_syntax::LanguageRegistry;
use cincel_text::{Buffer, EditSource};
use gpui::{
    App, AppContext, Bounds, Focusable, KeyBinding, WindowBounds, WindowOptions, actions, px, size,
};
use gpui_platform::application;

actions!(
    phantom_example,
    [
        /// Closes the demo.
        Quit
    ]
);

/// The file shown by default.
const SAMPLE: &str = include_str!("sample.rs");

/// What `--type-test` types, one character per frame. Plain letters at the end
/// of the first comment: an edit whose parse stays valid, so any row that loses
/// its colours lost them to the flicker and not to a real re-highlight.
const TYPED: &[&str] = &["a", "b", "c", "d", " "];

/// Hunks this large or smaller get word diffs (Zed's `MAX_WORD_DIFF_LINE_COUNT`).
const MAX_WORD_DIFF_LINES: usize = 5;

/// Pending hunks the demo pretends the other files of the review have.
const OTHER_FILES_PENDING: usize = 4;

/// A deleted line containing this comes from a "previous turn".
const PREVIOUS_TURN_MARK: &str = "self.rate";

struct Args {
    smoke_test: bool,
    type_test: bool,
    simulate_actions: bool,
    turn_active: bool,
    rows: Option<usize>,
    file: Option<String>,
    soft_wrap: bool,
    show_whitespace: bool,
}

fn parse_args() -> Args {
    let mut args = Args {
        smoke_test: false,
        type_test: false,
        simulate_actions: false,
        turn_active: false,
        rows: None,
        file: None,
        soft_wrap: false,
        show_whitespace: false,
    };
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--smoke-test" => args.smoke_test = true,
            "--type-test" => args.type_test = true,
            "--simulate-actions" => args.simulate_actions = true,
            "--turn-active" => args.turn_active = true,
            "--soft-wrap" => args.soft_wrap = true,
            "--show-whitespace" => args.show_whitespace = true,
            "--rows" => args.rows = iter.next().and_then(|value| value.parse().ok()),
            "--file" => args.file = iter.next(),
            other => eprintln!("argumento desconocido: {other}"),
        }
    }
    args
}

/// Builds a synthetic buffer of `rows` lines for the scroll benchmark.
fn synthetic_text(rows: usize) -> String {
    let mut text = String::with_capacity(rows * 48);
    for row in 0..rows {
        text.push_str(&format!(
            "fn generated_{row}(value: u32) -> u32 {{ value * {} + {row} }}\n",
            row % 7 + 1
        ));
    }
    text
}

/// The text the agent started from: `sample.rs` with four changes undone.
fn sample_base() -> String {
    let replacements = [
        // A pure addition: the agent added the `tax_rate` field.
        (
            "    pub unit_price: f64,\n    pub tax_rate: f64,\n",
            "    pub unit_price: f64,\n",
        ),
        // A modified block: 2 lines out, 3 in.
        (
            "        let gross = quantity * self.unit_price;\n        let discount = if quantity >= 10.0 { 0.05 } else { 0.0 };\n        gross * (1.0 - discount)\n",
            "        let gross = quantity * self.unit_price * 1.0;\n        gross\n",
        ),
        // A one-word change, still pending from a previous turn.
        (
            "        self.subtotal() * self.tax_rate\n",
            "        self.subtotal() * self.rate\n",
        ),
        // A pure deletion.
        (
            "    /// Total of the line, taxes included.\n",
            "    // FIXME: round to cents\n    /// Total of the line, taxes included.\n",
        ),
    ];
    let mut base = SAMPLE.to_string();
    for (current, before) in replacements {
        assert!(base.contains(current), "sample.rs changed: {current:?}");
        base = base.replacen(current, before, 1);
    }
    base
}

fn lines_of(text: &str) -> Vec<String> {
    text.split('\n').map(str::to_string).collect()
}

/// The regions where two line lists differ, as `(base rows, current rows)`,
/// from a longest-common-subsequence alignment.
fn diff_lines(base: &[String], current: &[String]) -> Vec<(Range<usize>, Range<usize>)> {
    let prefix = base.iter().zip(current).take_while(|(a, b)| a == b).count();
    let suffix = base[prefix..]
        .iter()
        .rev()
        .zip(current[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let a = &base[prefix..base.len() - suffix];
    let b = &current[prefix..current.len() - suffix];
    // lcs[i][j] = LCS length of a[i..] and b[j..].
    let width = b.len() + 1;
    let mut lcs = vec![0u32; (a.len() + 1) * width];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i * width + j] = if a[i] == b[j] {
                lcs[(i + 1) * width + j + 1] + 1
            } else {
                lcs[(i + 1) * width + j].max(lcs[i * width + j + 1])
            };
        }
    }
    let mut regions = Vec::new();
    let (mut i, mut j) = (0, 0);
    let mut open: Option<(usize, usize)> = None;
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            if let Some((si, sj)) = open.take() {
                regions.push((prefix + si..prefix + i, prefix + sj..prefix + j));
            }
            i += 1;
            j += 1;
            continue;
        }
        open.get_or_insert((i, j));
        if j >= b.len() || (i < a.len() && lcs[(i + 1) * width + j] >= lcs[i * width + j + 1]) {
            i += 1;
        } else {
            j += 1;
        }
    }
    if let Some((si, sj)) = open {
        regions.push((prefix + si..prefix + i, prefix + sj..prefix + j));
    }
    regions
}

fn is_word(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// The changed byte ranges of a pair of lines: common prefix and suffix
/// trimmed, then widened to whole words.
fn word_change(old: &str, new: &str) -> Option<(Range<usize>, Range<usize>)> {
    let a: Vec<(usize, char)> = old.char_indices().collect();
    let b: Vec<(usize, char)> = new.char_indices().collect();
    let mut prefix = a
        .iter()
        .zip(&b)
        .take_while(|((_, x), (_, y))| x == y)
        .count();
    let mut suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|((_, x), (_, y))| x == y)
        .count();
    // Nothing in common but indentation: the whole line is the change, and
    // the row background already says so.
    let shared = a[..prefix]
        .iter()
        .chain(&a[a.len() - suffix..])
        .any(|(_, ch)| !ch.is_whitespace());
    if !shared {
        return None;
    }
    let word_at =
        |chars: &[(usize, char)], ix: usize| chars.get(ix).is_some_and(|(_, ch)| is_word(*ch));
    while prefix > 0 && word_at(&a, prefix - 1) && (word_at(&a, prefix) || word_at(&b, prefix)) {
        prefix -= 1;
    }
    while suffix > 0 {
        let (ea, eb) = (a.len() - suffix, b.len() - suffix);
        let inside = word_at(&a, ea)
            && (ea > prefix && word_at(&a, ea - 1) || eb > prefix && word_at(&b, eb - 1));
        if !inside {
            break;
        }
        suffix -= 1;
    }
    let byte = |chars: &[(usize, char)], ix: usize, text: &str| {
        chars.get(ix).map(|(byte, _)| *byte).unwrap_or(text.len())
    };
    let old_range = byte(&a, prefix, old)..byte(&a, a.len() - suffix, old);
    let new_range = byte(&b, prefix, new)..byte(&b, b.len() - suffix, new);
    (!old_range.is_empty() || !new_range.is_empty()).then_some((old_range, new_range))
}

/// The host side of the demo: the base text and what the review looks like
/// against the buffer.
struct DemoReview {
    base: Vec<String>,
    turn_active: bool,
    /// `(base, buffer text)` before each rejection, for `UndoLastReject`.
    undo: Vec<(Vec<String>, String)>,
}

impl DemoReview {
    fn view(&self, text: &str) -> ReviewView {
        let current = lines_of(text);
        let mut hunks = Vec::new();
        for (base_rows, rows) in diff_lines(&self.base, &current) {
            let deleted_lines: Vec<String> = self.base[base_rows.clone()].to_vec();
            let added: Vec<String> = current[rows.clone()].to_vec();
            let kind = match (deleted_lines.is_empty(), added.is_empty()) {
                (false, false) => ReviewHunkKind::Modified,
                (false, true) => ReviewHunkKind::Deleted,
                _ => ReviewHunkKind::Added,
            };
            let paired = deleted_lines.len().min(added.len());
            let mut lines: Vec<ReviewLineView> = (0..paired)
                .map(|ix| ReviewLineView {
                    base_line: Some(ix as u32),
                    buffer_row: Some((rows.start + ix) as u32),
                })
                .collect();
            lines.extend((paired..deleted_lines.len()).map(|ix| ReviewLineView {
                base_line: Some(ix as u32),
                buffer_row: None,
            }));
            lines.extend((paired..added.len()).map(|ix| ReviewLineView {
                base_line: None,
                buffer_row: Some((rows.start + ix) as u32),
            }));
            let word_diffs = (kind == ReviewHunkKind::Modified
                && deleted_lines.len() <= MAX_WORD_DIFF_LINES
                && added.len() <= MAX_WORD_DIFF_LINES)
                .then(|| {
                    let mut words = ReviewWordDiffs::default();
                    let (mut old_offset, mut new_offset) = (0, 0);
                    for ix in 0..paired {
                        if let Some((old, new)) = word_change(&deleted_lines[ix], &added[ix]) {
                            if !old.is_empty() {
                                words
                                    .deleted
                                    .push(old.start + old_offset..old.end + old_offset);
                            }
                            if !new.is_empty() {
                                words
                                    .added
                                    .push(new.start + new_offset..new.end + new_offset);
                            }
                        }
                        old_offset += deleted_lines[ix].len() + 1;
                        new_offset += added[ix].len() + 1;
                    }
                    words
                });
            let mut hasher = DefaultHasher::new();
            deleted_lines.hash(&mut hasher);
            added.hash(&mut hasher);
            hunks.push(ReviewHunkView {
                id: hasher.finish(),
                base_rows: base_rows.start as u32..base_rows.end as u32,
                from_previous_turn: deleted_lines
                    .iter()
                    .any(|line| line.contains(PREVIOUS_TURN_MARK)),
                deleted_lines,
                buffer_rows: rows.start as u32..rows.end as u32,
                kind,
                word_diffs,
                lines,
            });
        }
        ReviewView {
            pending_in_file: hunks.len(),
            hunks,
            turn_active: self.turn_active,
            pending_in_other_files: OTHER_FILES_PENDING,
            current_index: None,
            file_actions: false,
        }
    }

    /// Applies an action; returns the new buffer text when it changed.
    fn apply(&mut self, action: ReviewAction, text: &str) -> Option<String> {
        let review = self.view(text);
        let mut current = lines_of(text);
        let find = |id: u64| review.hunks.iter().find(|hunk| hunk.id == id);
        let before = (self.base.clone(), text.to_string());
        let mut rejected = false;
        match action {
            ReviewAction::AcceptHunk(id) => {
                let hunk = find(id)?;
                let rows = hunk.buffer_rows.start as usize..hunk.buffer_rows.end as usize;
                let base_rows = hunk.base_rows.start as usize..hunk.base_rows.end as usize;
                self.base.splice(base_rows, current[rows].to_vec());
            }
            ReviewAction::RejectHunk(id) => {
                let hunk = find(id)?;
                let rows = hunk.buffer_rows.start as usize..hunk.buffer_rows.end as usize;
                current.splice(rows, hunk.deleted_lines.clone());
                rejected = true;
            }
            ReviewAction::AcceptLine { hunk, line } => {
                let hunk = find(hunk)?;
                let base_start = hunk.base_rows.start as usize;
                match *hunk.lines.get(line)? {
                    ReviewLineView {
                        base_line: Some(b),
                        buffer_row: Some(r),
                    } => self.base[base_start + b as usize] = current[r as usize].clone(),
                    ReviewLineView {
                        base_line: Some(b),
                        buffer_row: None,
                    } => {
                        self.base.remove(base_start + b as usize);
                    }
                    ReviewLineView {
                        base_line: None,
                        buffer_row: Some(r),
                    } => {
                        let k = (r - hunk.buffer_rows.start) as usize;
                        let at = base_start + k.min(hunk.deleted_lines.len());
                        self.base.insert(at, current[r as usize].clone());
                    }
                    _ => return None,
                }
            }
            ReviewAction::RejectLine { hunk, line } => {
                let hunk = find(hunk)?;
                let base_start = hunk.base_rows.start as usize;
                match *hunk.lines.get(line)? {
                    ReviewLineView {
                        base_line: Some(b),
                        buffer_row: Some(r),
                    } => current[r as usize] = self.base[base_start + b as usize].clone(),
                    ReviewLineView {
                        base_line: Some(b),
                        buffer_row: None,
                    } => {
                        let at = hunk.buffer_rows.start as usize
                            + (b as usize).min(hunk.buffer_rows.len());
                        current.insert(at, hunk.deleted_lines[b as usize].clone());
                    }
                    ReviewLineView {
                        base_line: None,
                        buffer_row: Some(r),
                    } => {
                        current.remove(r as usize);
                    }
                    _ => return None,
                }
                rejected = true;
            }
            // The demo has one file, so its turn is the file.
            ReviewAction::AcceptFile | ReviewAction::AcceptTurn => self.base = current.clone(),
            ReviewAction::RejectFile | ReviewAction::RejectTurn => {
                current = self.base.clone();
                rejected = true;
            }
            ReviewAction::UndoLastReject => {
                let (base, text) = self.undo.pop()?;
                self.base = base;
                println!("rechazo deshecho");
                return Some(text);
            }
            ReviewAction::NextHunk | ReviewAction::PrevHunk => return None,
            ReviewAction::NextFile => {
                println!("siguiente archivo (lo resuelve el workspace)");
                return None;
            }
            ReviewAction::OpenReviewPanel => {
                println!("abrir el panel de revisión (lo resuelve el workspace)");
                return None;
            }
        }
        if rejected {
            self.undo.push(before);
            println!("Segmento rechazado · Deshacer (Alt+Shift+U)");
        }
        let new_text = current.join("\n");
        (new_text != text).then_some(new_text)
    }
}

/// The document to show: `(text, review host, static review, path)`.
fn document(args: &Args) -> (String, Option<DemoReview>, ReviewView, String) {
    if let Some(path) = &args.file {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("no se pudo leer {path}: {error}"));
        return (text, None, ReviewView::default(), path.clone());
    }

    if let Some(rows) = args.rows {
        let text = synthetic_text(rows);
        let hunks = [
            PhantomHunk {
                insert_before_buffer_row: 20,
                deleted_text: vec![
                    "fn generated_20(value: u32) -> u32 { value * 6 }".to_string(),
                    "// old helper".to_string(),
                ],
                added_rows: 20..23,
            },
            PhantomHunk {
                insert_before_buffer_row: 60,
                deleted_text: vec!["// removed line".to_string()],
                added_rows: 60..60,
            },
        ];
        let mut review = ReviewView::from_phantoms(&hunks);
        review.turn_active = args.turn_active;
        return (text, None, review, "generado.rs".to_string());
    }

    let host = DemoReview {
        base: lines_of(&sample_base()),
        turn_active: args.turn_active,
        undo: Vec::new(),
    };
    let mut review = host.view(SAMPLE);
    // Open on the first modified hunk, the way the review panel opens a file
    // on its first pending change: the host sets `current_index`.
    review.current_index = review
        .hunks
        .iter()
        .position(|hunk| hunk.kind == ReviewHunkKind::Modified);
    (
        SAMPLE.to_string(),
        Some(host),
        review,
        "sample.rs".to_string(),
    )
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = parse_args();
    let (text, host, review, path) = document(&args);
    let smoke_test = args.smoke_test;
    let type_test = args.type_test;
    let simulate = args.simulate_actions;
    let mut settings = EditorSettings {
        soft_wrap: args.soft_wrap,
        show_whitespace: args.show_whitespace,
        ..Default::default()
    };
    settings.ruler = Some(100);
    let host = host.map(|host| Rc::new(RefCell::new(host)));

    application().run(move |cx: &mut App| {
        bind_default_keys(cx);
        cx.bind_keys([KeyBinding::new("ctrl-q", Quit, None)]);
        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());

        let registry = Arc::new(LanguageRegistry::new());
        let language = registry.language_for_path(&path);
        println!(
            "archivo: {path} · lenguaje: {}",
            language
                .as_ref()
                .map(|language| language.name())
                .unwrap_or("texto plano")
        );
        let buffer: SharedBuffer = shared(Buffer::new(&text));

        let bounds = Bounds::centered(None, size(px(1100.), px(760.)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |window, cx| {
                    cx.new(|cx| {
                        EditorView::new(
                            buffer,
                            language,
                            registry,
                            settings,
                            EditorTheme::default(),
                            window,
                            cx,
                        )
                    })
                },
            )
            .expect("no se pudo abrir la ventana");

        let host = host.clone();
        window
            .update(cx, |view, window, cx| {
                window.focus(&view.focus_handle(cx), cx);
                cx.activate(true);
                let entity = cx.entity();
                let mut seen_version = view.text_version();
                cx.subscribe(
                    &entity,
                    move |view, _entity, event: &EditorEvent, cx| match event {
                        EditorEvent::SaveRequested => println!("save requested"),
                        EditorEvent::DirtyChanged(dirty) => println!("dirty: {dirty}"),
                        EditorEvent::Review(action) => {
                            println!("review: {action:?}");
                            let Some(host) = host.as_ref().filter(|_| simulate) else {
                                return;
                            };
                            let text = view.text();
                            let new_text = host.borrow_mut().apply(*action, &text);
                            if let Some(new_text) = new_text {
                                view.buffer()
                                    .lock()
                                    .set_text_minimal(&new_text, EditSource::Review);
                                view.buffer_changed(cx);
                            }
                            seen_version = view.text_version();
                            let review = host.borrow().view(&view.text());
                            view.set_review(review, cx);
                        }
                        // Typing changes the hunks: refresh them like the
                        // store does on every edit.
                        EditorEvent::CursorMoved { .. } => {
                            if let Some(host) = host.as_ref().filter(|_| simulate)
                                && view.text_version() != seen_version
                            {
                                seen_version = view.text_version();
                                let review = host.borrow().view(&view.text());
                                view.set_review(review, cx);
                            }
                        }
                        EditorEvent::ScrollChanged { .. } => {}
                    },
                )
                .detach();
                view.set_review(review, cx);
            })
            .expect("no se pudo enfocar el editor");

        if type_test {
            // Type into the real window and let the render probe say what the
            // frames painted.
            window
                .update(cx, |view, _window, cx| {
                    view.set_render_probe(true);
                    // End of the first line, inside the comment.
                    view.set_cursor(cincel_text::Point::new(0, u32::MAX), cx);
                })
                .ok();
            cx.spawn(async move |cx| {
                for step in 0..180 {
                    cx.background_executor()
                        .timer(Duration::from_millis(16))
                        .await;
                    let typed = window.update(cx, |view, _window, cx| {
                        view.insert_text(TYPED[step % TYPED.len()], cx);
                    });
                    if typed.is_err() {
                        break;
                    }
                }
                window
                    .update(cx, |view, _window, _cx| {
                        let stats = view.frame_stats();
                        let frames = view.render_frames();
                        let painted: usize =
                            frames.iter().map(|frame| frame.rows.len()).sum();
                        let reshaped: usize = frames
                            .iter()
                            .map(|frame| {
                                frame.rows.iter().filter(|row| row.reshaped).count()
                            })
                            .sum();
                        // A row that had colours on one frame and none on the
                        // next: the flash, counted.
                        let mut lost = 0usize;
                        for pair in frames.windows(2) {
                            for row in &pair[1].rows {
                                let had = pair[0]
                                    .row(row.display_row)
                                    .is_some_and(|before| before.highlight_spans > 0);
                                if had && row.highlight_spans == 0 {
                                    lost += 1;
                                }
                            }
                        }
                        println!(
                            "frames: {} · p50 {:.2} ms · p95 {:.2} ms · max {:.2} ms",
                            stats.count,
                            stats.p50_us as f64 / 1000.,
                            stats.p95_us as f64 / 1000.,
                            stats.max_us as f64 / 1000.,
                        );
                        println!(
                            "probe: {} frames · {painted} filas pintadas · {reshaped} re-shaped · {lost} filas perdieron sus colores",
                            frames.len(),
                        );
                    })
                    .ok();
                window
                    .update(cx, |_view, window, _cx| window.remove_window())
                    .ok();
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
                cx.update(|cx| cx.quit());
            })
            .detach();
        }

        if smoke_test {
            // Scroll for ~3 s so the frame statistics have something to say,
            // then exit.
            cx.spawn(async move |cx| {
                for step in 0..180 {
                    cx.background_executor()
                        .timer(Duration::from_millis(16))
                        .await;
                    let scrolled = window.update(cx, |view, _window, cx| {
                        view.scroll_rows(if step % 40 == 39 { -39. } else { 1. }, cx);
                    });
                    if scrolled.is_err() {
                        break;
                    }
                }
                window
                    .update(cx, |view, _window, _cx| {
                        let stats = view.frame_stats();
                        println!(
                            "frames: {} · p50 {:.2} ms · p95 {:.2} ms · max {:.2} ms",
                            stats.count,
                            stats.p50_us as f64 / 1000.,
                            stats.p95_us as f64 / 1000.,
                            stats.max_us as f64 / 1000.,
                        );
                    })
                    .ok();
                // Close the window before quitting so no entity handle
                // outlives the app (gpui's leak detection is strict).
                window
                    .update(cx, |_view, window, _cx| window.remove_window())
                    .ok();
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
                cx.update(|cx| cx.quit());
            })
            .detach();
        }
    });
}
