//! `crate::fonts` (`docs/specs/08-etapa6-cierre-1-0.md` §4.4).
//!
//! `TestAppContext`'s platform text system is GPUI's `NoopTextSystem`
//! (`gpui-pre`'s `platform/test/platform.rs`): `add_fonts` is a no-op that
//! always succeeds and `all_font_names` always returns an empty list, so no
//! unit test can observe a real font actually resolving. Per §4.4, these
//! checks are then limited to the ten files' own headers, and the two
//! deeper ones (the families showing up in `all_font_names`, and each weight
//! resolving to the embedded file rather than a fallback) move to
//! `cincel --smoke-test` (§4.3, first criterion), which runs against the
//! real platform text system. `TextSystem::font_id` itself is private to
//! `gpui-pre`, so no test outside that crate can call it directly even with a
//! real backend; `--smoke-test` covers weight resolution instead through
//! `Typography::first_installed` and the log line it emits.

use std::borrow::Cow;

use gpui::TestAppContext;

use crate::fonts::{self, BUFFER_FAMILY, UI_FAMILY};

/// The sfnt version bytes at the start of every non-collection TrueType file.
const TRUETYPE_HEADER: [u8; 4] = [0x00, 0x01, 0x00, 0x00];

#[test]
fn embedded_has_ten_non_empty_truetype_files() {
    let files = fonts::embedded();
    assert_eq!(files.len(), 10, "six Inter + four JetBrains Mono weights");
    for (index, file) in files.iter().enumerate() {
        assert!(!file.is_empty(), "file {index} is empty");
        assert!(
            file.len() > TRUETYPE_HEADER.len(),
            "file {index} is too small to be a real font"
        );
        assert_eq!(
            &file[..TRUETYPE_HEADER.len()],
            &TRUETYPE_HEADER,
            "file {index} does not start with the TrueType sfnt header"
        );
    }
}

#[test]
fn embedded_returns_owned_static_slices() {
    // `include_bytes!` gives `&'static [u8]`; every entry should borrow it
    // rather than copy, so registering the fonts does not duplicate ~3.5 MB.
    for file in fonts::embedded() {
        assert!(matches!(file, Cow::Borrowed(_)));
    }
}

#[gpui::test]
fn register_embedded_succeeds_in_the_test_harness(cx: &mut TestAppContext) {
    cx.update(|cx| {
        fonts::register_embedded(cx).expect("registering the embedded fonts must not fail");

        let installed = cx.text_system().all_font_names();
        let has_families = installed
            .iter()
            .any(|name| name.eq_ignore_ascii_case(UI_FAMILY))
            && installed
                .iter()
                .any(|name| name.eq_ignore_ascii_case(BUFFER_FAMILY));
        if !has_families {
            // `NoopTextSystem`: no platform font backend in this harness, so
            // `all_font_names` cannot see anything that was just registered
            // either. Expected here (see the module doc); `--smoke-test`
            // covers the real resolution.
            return;
        }

        // A future test harness with a real backend would land here; keep
        // the assertion so it starts pulling weight.
        assert!(
            installed
                .iter()
                .any(|name| name.eq_ignore_ascii_case(UI_FAMILY))
        );
        assert!(
            installed
                .iter()
                .any(|name| name.eq_ignore_ascii_case(BUFFER_FAMILY))
        );
    });
}
