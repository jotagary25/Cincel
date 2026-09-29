//! The git gutter end to end (`docs/specs/07-etapa5-productividad.md` §6.5):
//! a file of a temporary repository opened in a tab gets its diff against
//! `HEAD` in the editor, and saving recomputes it.
//!
//! Every test skips itself when this machine has no usable `git`, like the
//! git tests of `cincel-project`.

use std::path::{Path, PathBuf};
use std::process::Command;

use cincel_editor::{EditorView, GitGutterColors, GitGutterHunk, GitGutterKind};
use cincel_settings::Config;
use gpui::{Entity, TestAppContext, VisualTestContext};

use crate::center::TabContent;
use crate::project::ProjectOptions;
use crate::test_support::isolate_state;
use crate::workspace::{Workspace, WorkspaceOptions};

fn init_test(cx: &mut TestAppContext) {
    isolate_state();
    cx.update(|cx| crate::init(Config::default(), cx));
}

fn git(dir: &Path, args: &[&str]) -> Option<()> {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|_| ())
}

/// Twelve numbered lines, committed as `f.txt`.
fn original() -> String {
    (1..=12).map(|n| format!("línea {n}\n")).collect()
}

/// A repository with `f.txt` committed, or `None` without a usable `git`.
fn repo() -> Option<tempfile::TempDir> {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"])?;
    git(dir.path(), &["config", "user.email", "test@cincel"])?;
    git(dir.path(), &["config", "user.name", "Test"])?;
    std::fs::write(dir.path().join("f.txt"), original()).unwrap();
    git(dir.path(), &["add", "."])?;
    git(dir.path(), &["commit", "-qm", "inicial"])?;
    Some(dir)
}

/// A workspace window on `root` with no background threads but the git
/// gutter on (it runs on GPUI's own executor, which the test drives).
fn workspace_window<'a>(
    root: &Path,
    cx: &'a mut TestAppContext,
) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    let options = WorkspaceOptions {
        project: Some(root.to_path_buf()),
        project_options: ProjectOptions {
            git_gutter: true,
            ..ProjectOptions::inert()
        },
        ..WorkspaceOptions::default()
    };
    let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(options, window, cx));
    cx.run_until_parked();
    (workspace, cx)
}

/// Opens `relative` in a pinned tab and returns its editor.
fn open_tab(
    workspace: &Entity<Workspace>,
    relative: &str,
    cx: &mut VisualTestContext,
) -> Entity<EditorView> {
    let files = workspace.read_with(cx, |workspace, _| workspace.files().clone());
    files.update(cx, |files, cx| {
        files.open_for_test(Path::new(relative), true, cx)
    });
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, cx| {
        let center = workspace.center().read(cx);
        let tab = center.active_tab().expect("hay pestaña");
        match &tab.content {
            TabContent::Editor(editor) | TabContent::MarkdownPreview(editor, _) => editor.clone(),
        }
    })
}

fn git_diff(editor: &Entity<EditorView>, cx: &mut VisualTestContext) -> Option<Vec<GitGutterHunk>> {
    editor.read_with(cx, |editor, _| editor.git_diff())
}

fn hunk(kind: GitGutterKind, rows: std::ops::Range<u32>) -> GitGutterHunk {
    GitGutterHunk { kind, rows }
}

#[gpui::test]
fn opening_a_changed_file_shows_its_diff_and_saving_recomputes_it(cx: &mut TestAppContext) {
    init_test(cx);
    let Some(dir) = repo() else {
        eprintln!("sin git utilizable; se omite");
        return;
    };
    // Line 3 changed on disk before the tab opens.
    let mut lines: Vec<String> = original().lines().map(str::to_owned).collect();
    lines[2] = "línea 3 cambiada".to_owned();
    std::fs::write(dir.path().join("f.txt"), lines.join("\n") + "\n").unwrap();

    let (workspace, cx) = workspace_window(dir.path(), cx);
    let editor = open_tab(&workspace, "f.txt", cx);
    assert_eq!(
        git_diff(&editor, cx),
        Some(vec![hunk(GitGutterKind::Modified, 2..3)])
    );
    // The project keeps the diff too, under the absolute path.
    let project = workspace.read_with(cx, |workspace, _| workspace.project().cloned().unwrap());
    let path: PathBuf = project.read_with(cx, |project, _| project.absolute(Path::new("f.txt")));
    let stored = project.read_with(cx, |project, _| project.git_diff(&path).cloned());
    assert_eq!(stored.map(|diff| diff.hunks.len()), Some(1));

    // Two lines typed after line 10, then saved.
    editor.update_in(cx, |editor, _window, cx| {
        editor.set_cursor(cincel_text::Point::new(9, 100), cx);
        editor.insert_text("\nnueva a\nnueva b", cx);
    });
    cx.run_until_parked();
    // Not saved yet: the bars only follow the text.
    assert_eq!(
        git_diff(&editor, cx),
        Some(vec![hunk(GitGutterKind::Modified, 2..3)])
    );
    let center = workspace.read_with(cx, |workspace, _| workspace.center().clone());
    center.update_in(cx, |center, window, cx| center.save_active(window, cx));
    cx.run_until_parked();
    assert_eq!(
        git_diff(&editor, cx),
        Some(vec![
            hunk(GitGutterKind::Modified, 2..3),
            hunk(GitGutterKind::Added, 10..12),
        ])
    );

    // The colours are the `git.*` of the theme in force.
    let colors = editor.read_with(cx, |editor, _| editor.git_colors());
    let theme = cx.update(|_, cx| cx.global::<crate::settings::AppSettings>().theme().clone());
    assert_eq!(
        colors,
        GitGutterColors {
            added: gpui::rgba(theme.git_added.rgba_u32()),
            modified: gpui::rgba(theme.git_modified.rgba_u32()),
            deleted: gpui::rgba(theme.git_deleted.rgba_u32()),
        }
    );

    // Back to the committed text and saved: no bars.
    editor.update_in(cx, |editor, _window, cx| {
        editor.set_text(&original(), 0, cx);
    });
    center.update_in(cx, |center, window, cx| center.save_active(window, cx));
    cx.run_until_parked();
    assert_eq!(git_diff(&editor, cx), Some(Vec::new()));
}

/// §6.4: an untracked file and a folder without git show no bars and no
/// warnings.
#[gpui::test]
fn untracked_files_and_folders_without_git_have_no_bars(cx: &mut TestAppContext) {
    init_test(cx);
    let Some(dir) = repo() else {
        eprintln!("sin git utilizable; se omite");
        return;
    };
    std::fs::write(dir.path().join("nuevo.txt"), "hola\n").unwrap();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    let editor = open_tab(&workspace, "nuevo.txt", cx);
    assert_eq!(git_diff(&editor, cx), None);
    // A clean tracked file has a diff, and it is empty.
    let clean = open_tab(&workspace, "f.txt", cx);
    assert_eq!(git_diff(&clean, cx), Some(Vec::new()));
}

#[gpui::test]
fn a_folder_without_git_has_no_bars(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = tempfile::tempdir().unwrap();
    // A temp dir is normally outside any repository; skip otherwise.
    if cincel_project::git_dir(dir.path()).is_some() {
        eprintln!("el directorio temporal está dentro de un repositorio; se omite");
        return;
    }
    std::fs::write(dir.path().join("a.txt"), "hola\n").unwrap();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    let editor = open_tab(&workspace, "a.txt", cx);
    assert_eq!(git_diff(&editor, cx), None);
}

/// With the gutter off (the inert options every other test uses) nothing
/// runs git and nothing reaches the editor.
#[gpui::test]
fn the_inert_project_computes_no_diff(cx: &mut TestAppContext) {
    init_test(cx);
    let Some(dir) = repo() else {
        eprintln!("sin git utilizable; se omite");
        return;
    };
    std::fs::write(dir.path().join("f.txt"), "cambiado\n").unwrap();
    let options = WorkspaceOptions {
        project: Some(dir.path().to_path_buf()),
        project_options: ProjectOptions::inert(),
        ..WorkspaceOptions::default()
    };
    let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(options, window, cx));
    cx.run_until_parked();
    let editor = open_tab(&workspace, "f.txt", cx);
    assert_eq!(git_diff(&editor, cx), None);
}
