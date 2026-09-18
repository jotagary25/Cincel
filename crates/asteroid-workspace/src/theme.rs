//! The Asteroid color theme.
//!
//! [`Theme`] holds exactly the tokens listed in `docs/specs/02-visual.md` §2,
//! so the rest of the application never spells a raw color. `Theme::Asteroid
//! Dark` is the default; the light theme arrives in a later stage.
//!
//! The tokens are also projected onto gpui-kit's own `Theme` global (see
//! [`apply_to_kit`]) so the components we do not draw ourselves (dock, tab
//! bar, title bar, status bar, buttons) follow the same palette. gpui-kit's
//! palette is component-shaped rather than role-shaped, so the mapping is
//! one token to several of its colors, and a few of its colors (charts,
//! sliders, calendars) have no counterpart here and keep the values the
//! built-in dark theme gave them.

use gpui_kit::component::ThemeMode;
use gpui_kit::{App, Global, Hsla, SharedString, px, rgb};

/// Typography defaults from `docs/specs/02-visual.md` §3.
pub struct Typography;

impl Typography {
    /// UI font family. `system-ui` is the documented fallback; GPUI takes a
    /// single family name, so [`ui_font_family`](Self::ui_font_family)
    /// resolves the fallback itself against the installed fonts.
    pub const UI_FAMILY: &'static str = "Inter";
    /// Fallback chain used when `Inter` is not installed.
    pub const UI_FAMILY_FALLBACKS: &'static [&'static str] =
        &["system-ui", "Inter Display", "Cantarell", "DejaVu Sans"];
    /// UI font size.
    pub const UI_SIZE: f32 = 13.;
    /// Code font family (used from stage 1 on, by `asteroid-editor`).
    pub const CODE_FAMILY: &'static str = "JetBrains Mono";
    /// Fallback chain for the code font.
    pub const CODE_FAMILY_FALLBACKS: &'static [&'static str] =
        &["Zed Mono", "DejaVu Sans Mono", "monospace"];
    /// Code font size.
    pub const CODE_SIZE: f32 = 14.;

    /// The first family of `families` that the platform text system actually
    /// has, or `None` when it has none of them.
    ///
    /// GPUI's `Theme::font_family` is a single name and an unknown family
    /// makes every text lookup miss, so the fallback chain from the spec has
    /// to be resolved before the name is handed over.
    pub fn first_installed(cx: &App, families: &[&str]) -> Option<SharedString> {
        let installed = cx.text_system().all_font_names();
        families
            .iter()
            .find(|wanted| {
                installed
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(wanted))
            })
            .map(|name| SharedString::from(name.to_string()))
    }

    /// The UI family to use on this machine, spec family first.
    pub fn ui_font_family(cx: &App) -> Option<SharedString> {
        let mut chain = vec![Self::UI_FAMILY];
        chain.extend_from_slice(Self::UI_FAMILY_FALLBACKS);
        Self::first_installed(cx, &chain)
    }
}

/// Color tokens of `docs/specs/02-visual.md` §2.
///
/// Field names follow the spec's dotted token names with `.` replaced by `_`
/// (`bg.app` is [`bg_app`](Self::bg_app), `diff.deleted.word` is
/// [`diff_deleted_word`](Self::diff_deleted_word)).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    /// Name of the theme, as it appears in the settings.
    pub name: &'static str,
    /// Whether this is a dark theme.
    pub dark: bool,
    /// Panel background.
    pub bg_app: Hsla,
    /// Editor background.
    pub bg_editor: Hsla,
    /// Cards and inputs.
    pub bg_surface: Hsla,
    /// Menus, popovers, floating bar.
    pub bg_elevated: Hsla,
    /// Panel borders.
    pub border: Hsla,
    /// Focus ring.
    pub border_focus: Hsla,
    /// Main text.
    pub text: Hsla,
    /// Secondary text and line numbers.
    pub text_muted: Hsla,
    /// Links and active items.
    pub text_accent: Hsla,
    /// Caret.
    pub cursor: Hsla,
    /// Selection.
    pub selection: Hsla,
    /// Background of a deleted line.
    pub diff_deleted_bg: Hsla,
    /// A deleted word inside a deleted line.
    pub diff_deleted_word: Hsla,
    /// Background of an added line.
    pub diff_added_bg: Hsla,
    /// An added word inside an added line.
    pub diff_added_word: Hsla,
    /// Gutter bar of a deleted line.
    pub diff_gutter_deleted: Hsla,
    /// Gutter bar of an added line.
    pub diff_gutter_added: Hsla,
    /// Gutter bar of a modified line.
    pub diff_gutter_modified: Hsla,
    /// Error state.
    pub status_error: Hsla,
    /// Warning state.
    pub status_warning: Hsla,
    /// Success state.
    pub status_ok: Hsla,
}

impl Global for Theme {}

/// Hex literal to `Hsla`, the way every token below is written.
const fn c(hex: u32) -> Rgb {
    Rgb(hex)
}

/// A hex color that has not been converted yet.
///
/// `rgb()` is not a `const fn` and `Hsla`'s conversion is not either, so the
/// table of tokens is written as hex and converted in [`Theme::asteroid_dark`].
#[derive(Clone, Copy)]
struct Rgb(u32);

impl Rgb {
    fn hsla(self) -> Hsla {
        rgb(self.0).into()
    }

    /// The same color at `alpha` opacity, for the diff tints the spec gives
    /// as "`#e06c75` al 18%".
    fn alpha(self, alpha: f32) -> Hsla {
        self.hsla().alpha(alpha)
    }
}

impl Theme {
    /// The default dark theme, "Asteroid Dark".
    pub fn asteroid_dark() -> Self {
        // One Dark base hues, shared by several tokens.
        let red = c(0xe06c75);
        let green = c(0x98c379);
        let yellow = c(0xe5c07b);

        Self {
            name: "Asteroid Dark",
            dark: true,
            bg_app: c(0x1e2127).hsla(),
            bg_editor: c(0x282c34).hsla(),
            bg_surface: c(0x21252b).hsla(),
            bg_elevated: c(0x2c313a).hsla(),
            border: c(0x181a1f).hsla(),
            border_focus: c(0x528bff).hsla(),
            text: c(0xabb2bf).hsla(),
            text_muted: c(0x5c6370).hsla(),
            text_accent: c(0x61afef).hsla(),
            cursor: c(0x528bff).hsla(),
            selection: c(0x3e4451).hsla(),
            diff_deleted_bg: red.alpha(0.18),
            diff_deleted_word: red.alpha(0.38),
            diff_added_bg: green.alpha(0.18),
            diff_added_word: green.alpha(0.38),
            diff_gutter_deleted: red.hsla(),
            diff_gutter_added: green.hsla(),
            diff_gutter_modified: yellow.hsla(),
            status_error: red.hsla(),
            status_warning: yellow.hsla(),
            status_ok: green.hsla(),
        }
    }

    /// The active theme.
    pub fn global(cx: &App) -> &Self {
        cx.global::<Self>()
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::asteroid_dark()
    }
}

/// Installs `theme` as the global Asteroid theme and projects it onto
/// gpui-kit's theme, so its components share the palette.
///
/// Call after `gpui_kit::init`, which installs gpui-kit's own theme global.
pub fn apply_to_kit(theme: Theme, cx: &mut App) {
    use gpui_kit::component::{Theme as KitTheme, ThemeTokens};

    cx.set_global(theme);

    // Start from gpui-kit's built-in theme of the right mode so every color
    // we do not map keeps a sane value, then overwrite the mapped ones.
    KitTheme::change(
        if theme.dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        },
        None,
        cx,
    );

    let ui_font_family = Typography::ui_font_family(cx);
    let kit = KitTheme::global_mut(cx);

    // §3 Typography. GPUI takes one family name, so an unresolvable chain
    // leaves gpui-kit's own resolved system font in place.
    if let Some(family) = ui_font_family {
        kit.font_family = family;
    }
    kit.font_size = px(Typography::UI_SIZE);
    kit.mono_font_size = px(Typography::CODE_SIZE);

    // §4 Shape: 4px on buttons and cards, 6px on popovers, no shadows on
    // panels.
    kit.radius = px(4.);
    kit.radius_lg = px(6.);

    // §2 Colors.
    let colors = &mut kit.colors;

    // bg.app: every panel surface, including the chrome around them.
    colors.background = theme.bg_app;
    colors.sidebar = theme.bg_app;
    colors.title_bar = theme.bg_app;
    colors.status_bar = theme.bg_app;
    colors.tab_bar = theme.bg_app;
    colors.list = theme.bg_app;
    colors.table = theme.bg_app;

    // bg.surface: cards, inputs, inactive tabs.
    colors.secondary = theme.bg_surface;
    colors.accordion = theme.bg_surface;
    colors.group_box = theme.bg_surface;
    colors.tab = theme.bg_surface;
    colors.tab_bar_segmented = theme.bg_surface;
    colors.list_head = theme.bg_surface;
    colors.table_head = theme.bg_surface;
    colors.table_foot = theme.bg_surface;
    colors.muted = theme.bg_surface;
    colors.skeleton = theme.bg_surface;
    colors.progress_bar = theme.bg_surface;
    colors.switch = theme.bg_surface;
    colors.button = theme.bg_surface;
    colors.button_secondary = theme.bg_surface;
    colors.input = theme.bg_surface;

    // bg.elevated: menus, popovers, hover states, the active tab.
    colors.popover = theme.bg_elevated;
    colors.accent = theme.bg_elevated;
    colors.tab_active = theme.bg_elevated;
    colors.list_hover = theme.bg_elevated;
    colors.list_even = theme.bg_elevated;
    colors.table_hover = theme.bg_elevated;
    colors.table_even = theme.bg_elevated;
    colors.sidebar_accent = theme.bg_elevated;
    colors.button_hover = theme.bg_elevated;
    colors.button_secondary_hover = theme.bg_elevated;
    colors.secondary_hover = theme.bg_elevated;
    colors.drop_target = theme.bg_elevated;
    colors.overlay = theme.bg_elevated.alpha(0.6);

    // border.
    colors.border = theme.border;
    colors.sidebar_border = theme.border;
    colors.title_bar_border = theme.border;
    colors.status_bar_border = theme.border;
    colors.table_row_border = theme.border;
    colors.window_border = theme.border;

    // border.focus.
    colors.ring = theme.border_focus;
    colors.drag_border = theme.border_focus;
    colors.list_active_border = theme.border_focus;
    colors.table_active_border = theme.border_focus;

    // text.
    colors.foreground = theme.text;
    colors.sidebar_foreground = theme.text;
    colors.popover_foreground = theme.text;
    colors.button_foreground = theme.text;
    colors.button_secondary_foreground = theme.text;
    colors.secondary_foreground = theme.text;
    colors.group_box_foreground = theme.text;
    colors.tab_active_foreground = theme.text;

    // text.muted.
    colors.muted_foreground = theme.text_muted;
    colors.tab_foreground = theme.text_muted;
    colors.table_head_foreground = theme.text_muted;
    colors.table_foot_foreground = theme.text_muted;
    colors.description_list_label_foreground = theme.text_muted;
    colors.scrollbar_thumb = theme.text_muted.alpha(0.5);
    colors.scrollbar_thumb_hover = theme.text_muted;
    colors.scrollbar = theme.bg_app.alpha(0.);

    // text.accent.
    colors.primary = theme.text_accent;
    colors.primary_hover = theme.text_accent;
    colors.primary_active = theme.text_accent;
    colors.accent_foreground = theme.text_accent;
    colors.link = theme.text_accent;
    colors.link_hover = theme.text_accent;
    colors.link_active = theme.text_accent;
    colors.sidebar_primary = theme.text_accent;
    colors.button_primary = theme.text_accent;
    colors.button_primary_hover = theme.text_accent;
    colors.button_primary_active = theme.text_accent;
    colors.button_primary_foreground = theme.bg_app;
    colors.primary_foreground = theme.bg_app;
    colors.sidebar_primary_foreground = theme.bg_app;
    colors.sidebar_accent_foreground = theme.text_accent;

    // cursor and selection.
    colors.caret = theme.cursor;
    colors.selection = theme.selection;
    colors.list_active = theme.selection;
    colors.table_active = theme.selection;
    colors.secondary_active = theme.selection;
    colors.button_active = theme.selection;
    colors.slider_bar = theme.text_accent;
    colors.slider_thumb = theme.text;

    // status.*.
    colors.danger = theme.status_error;
    colors.danger_hover = theme.status_error;
    colors.danger_active = theme.status_error;
    colors.button_danger = theme.status_error;
    colors.button_danger_hover = theme.status_error;
    colors.button_danger_active = theme.status_error;
    colors.warning = theme.status_warning;
    colors.warning_hover = theme.status_warning;
    colors.warning_active = theme.status_warning;
    colors.button_warning = theme.status_warning;
    colors.button_warning_hover = theme.status_warning;
    colors.button_warning_active = theme.status_warning;
    colors.success = theme.status_ok;
    colors.success_hover = theme.status_ok;
    colors.success_active = theme.status_ok;
    colors.button_success = theme.status_ok;
    colors.button_success_hover = theme.status_ok;
    colors.button_success_active = theme.status_ok;
    colors.info = theme.text_accent;
    colors.info_hover = theme.text_accent;
    colors.info_active = theme.text_accent;
    colors.button_info = theme.text_accent;
    colors.button_info_hover = theme.text_accent;
    colors.button_info_active = theme.text_accent;

    // The `tokens` are gpui-kit's resolved view of `colors`; components read
    // both, so they have to be regenerated after the overwrite.
    kit.tokens = ThemeTokens::from(&kit.colors);

    // Push the result down to gpui-base, which owns scrollbars and resize
    // handles and would otherwise keep the built-in palette.
    KitTheme::sync_base(cx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_theme_matches_the_spec_table() {
        let theme = Theme::asteroid_dark();
        assert_eq!(theme.bg_app, rgb(0x1e2127).into());
        assert_eq!(theme.bg_editor, rgb(0x282c34).into());
        assert_eq!(theme.border_focus, rgb(0x528bff).into());
        assert_eq!(theme.text_muted, rgb(0x5c6370).into());
        // Diff tints keep the hue and take the opacity from the spec.
        assert_eq!(theme.diff_deleted_bg.a, 0.18);
        assert_eq!(theme.diff_added_word.a, 0.38);
        assert_eq!(theme.diff_gutter_deleted, rgb(0xe06c75).into());
    }
}
