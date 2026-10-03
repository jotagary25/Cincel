//! `Ctrl+Alt+W` (`editor::toggle_whitespace`) and the default bindings of the
//! one-row scroll keys (`docs/specs/10-etapa7-ronda2.md` §7.7, §12.1).

use gpui::{Keystroke, TestAppContext};

use crate::actions::{
    CONTEXT_EDITOR, ScrollLineDown, ScrollLineUp, ToggleWhitespace, bind_default_keys,
    default_key_bindings,
};
use crate::comment_box_tests::open;
use crate::review::ReviewView;
use crate::settings::EditorSettings;

#[gpui::test]
fn ctrl_alt_w_toggles_the_whitespace_of_that_view_only(cx: &mut TestAppContext) {
    let (first, first_handle, mut first_visual, _host) = open(
        cx,
        "uno  dos\n\tvarios\n",
        ReviewView::default(),
        EditorSettings::default(),
    );
    let (second, _second_handle, mut second_visual, _host) = open(
        cx,
        "tres\n",
        ReviewView::default(),
        EditorSettings::default(),
    );
    let shown = |view: &gpui::Entity<crate::view::EditorView>, cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, cx| view.read(cx).settings().show_whitespace)
    };
    assert!(!shown(&first, &mut first_visual));
    assert!(!shown(&second, &mut second_visual));

    // Keys go to the focused window: the second one was opened last.
    second_visual.simulate_keystrokes("ctrl-alt-w");
    assert!(shown(&second, &mut second_visual));
    assert!(
        !shown(&first, &mut first_visual),
        "the other tab keeps its own value"
    );

    // Back to the first window: it flips on its own, and flips back.
    first_visual.update(|window, cx| {
        window.activate_window();
        first.update(cx, |view, cx| window.focus(&view.focus_handle, cx));
    });
    first_visual.run_until_parked();
    let _ = first_handle;
    first_visual.simulate_keystrokes("ctrl-alt-w");
    assert!(shown(&first, &mut first_visual));
    assert!(shown(&second, &mut second_visual));
    first_visual.simulate_keystrokes("ctrl-alt-w");
    assert!(!shown(&first, &mut first_visual));
    assert!(shown(&second, &mut second_visual));
}

#[gpui::test]
fn the_default_keymap_resolves_the_three_keys_in_the_editor(cx: &mut TestAppContext) {
    // Straight from the list the workspace installs…
    let bindings = default_key_bindings();
    let find = |keys: &str| {
        let wanted = Keystroke::parse(keys).unwrap();
        bindings
            .iter()
            .filter(|binding| {
                binding.keystrokes().len() == 1
                    && binding.keystrokes()[0].inner().unparse() == wanted.unparse()
            })
            .collect::<Vec<_>>()
    };
    for (keys, name) in [
        ("ctrl-up", "editor::scroll_line_up"),
        ("ctrl-down", "editor::scroll_line_down"),
        ("ctrl-alt-w", "editor::toggle_whitespace"),
    ] {
        let found = find(keys);
        assert_eq!(found.len(), 1, "{keys} is bound exactly once");
        assert_eq!(found[0].action().name(), name);
        assert!(
            found[0].predicate().is_some(),
            "{keys} is scoped to a context"
        );
    }
    assert!(find("ctrl-up")[0].action().partial_eq(&ScrollLineUp));
    assert!(find("ctrl-down")[0].action().partial_eq(&ScrollLineDown));
    assert!(find("ctrl-alt-w")[0].action().partial_eq(&ToggleWhitespace));
    assert_eq!(CONTEXT_EDITOR, "Editor");

    // …and as the app resolves them once installed.
    cx.update(|cx| {
        bind_default_keys(cx);
        for (keys, name) in [
            ("ctrl-up", "editor::scroll_line_up"),
            ("ctrl-down", "editor::scroll_line_down"),
            ("ctrl-alt-w", "editor::toggle_whitespace"),
        ] {
            let resolved = cx.all_bindings_for_input(&[Keystroke::parse(keys).unwrap()]);
            assert_eq!(resolved.len(), 1, "{keys}");
            assert_eq!(resolved[0].action().name(), name);
        }
    });
}
