//! The active color theme, as GPUI sees it.
//!
//! `asteroid-settings` owns the theme *files* and hands us plain sRGB
//! [`asteroid_settings::Rgba`] values (`docs/specs/03-arquitectura.md` §2 keeps
//! that crate free of GPUI). [`ThemeColors`] is the same table converted to
//! `Hsla` once, installed as a GPUI global so every UI crate can read it
//! without re-parsing anything, and projected onto gpui-kit's own theme (see
//! [`apply`]) so the components we do not draw ourselves (dock, title bar,
//! status bar, buttons, menus) follow the same palette.
//!
//! gpui-kit's palette is component-shaped rather than role-shaped, so the
//! mapping is one token to several of its colors, and a few of its colors
//! (charts, sliders, calendars) have no counterpart here and keep whatever the
//! built-in theme gave them.

use asteroid_settings::{Rgba, Settings, SyntaxTheme};
use gpui_kit::component::ThemeMode;
use gpui_kit::{App, Global, Hsla, SharedString, px, rgba};

/// Converts a settings color into the GPUI one.
fn hsla(color: Rgba) -> Hsla {
    rgba(color.rgba_u32()).into()
}

/// The twelve syntax captures of `docs/specs/02-visual.md` §2, in GPUI colors.
///
/// The code editor (`asteroid-editor`) is the only consumer; it lives here so
/// the whole theme is converted in one place.
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(missing_docs)]
pub struct SyntaxColors {
    pub keyword: Hsla,
    pub function: Hsla,
    pub type_: Hsla,
    pub string: Hsla,
    pub number: Hsla,
    pub comment: Hsla,
    pub variable: Hsla,
    pub property: Hsla,
    pub operator: Hsla,
    pub punctuation: Hsla,
    pub constant: Hsla,
    pub attribute: Hsla,
}

impl From<&SyntaxTheme> for SyntaxColors {
    fn from(syntax: &SyntaxTheme) -> Self {
        Self {
            keyword: hsla(syntax.keyword),
            function: hsla(syntax.function),
            type_: hsla(syntax.type_),
            string: hsla(syntax.string),
            number: hsla(syntax.number),
            comment: hsla(syntax.comment),
            variable: hsla(syntax.variable),
            property: hsla(syntax.property),
            operator: hsla(syntax.operator),
            punctuation: hsla(syntax.punctuation),
            constant: hsla(syntax.constant),
            attribute: hsla(syntax.attribute),
        }
    }
}

/// Color tokens of `docs/specs/02-visual.md` §2, ready for GPUI.
///
/// Field names follow the spec's dotted token names with `.` replaced by `_`
/// (`bg.app` is [`bg_app`](Self::bg_app), `diff.deleted.word` is
/// [`diff_deleted_word`](Self::diff_deleted_word)).
#[derive(Debug, Clone, PartialEq)]
#[allow(missing_docs)]
pub struct ThemeColors {
    /// Name of the theme, as it appears in the settings.
    pub name: SharedString,
    /// Whether this is a dark theme.
    pub dark: bool,
    pub bg_app: Hsla,
    pub bg_editor: Hsla,
    pub bg_surface: Hsla,
    pub bg_elevated: Hsla,
    pub border: Hsla,
    pub border_focus: Hsla,
    pub text: Hsla,
    pub text_muted: Hsla,
    pub text_accent: Hsla,
    pub cursor: Hsla,
    pub selection: Hsla,
    pub diff_deleted_bg: Hsla,
    pub diff_deleted_word: Hsla,
    pub diff_added_bg: Hsla,
    pub diff_added_word: Hsla,
    pub diff_gutter_deleted: Hsla,
    pub diff_gutter_added: Hsla,
    pub diff_gutter_modified: Hsla,
    pub status_error: Hsla,
    pub status_warning: Hsla,
    pub status_ok: Hsla,
    /// Syntax colors, for the code editor.
    pub syntax: SyntaxColors,
}

impl Global for ThemeColors {}

impl From<&asteroid_settings::Theme> for ThemeColors {
    fn from(theme: &asteroid_settings::Theme) -> Self {
        Self {
            name: SharedString::from(theme.name.clone()),
            dark: theme.dark,
            bg_app: hsla(theme.bg_app),
            bg_editor: hsla(theme.bg_editor),
            bg_surface: hsla(theme.bg_surface),
            bg_elevated: hsla(theme.bg_elevated),
            border: hsla(theme.border),
            border_focus: hsla(theme.border_focus),
            text: hsla(theme.text),
            text_muted: hsla(theme.text_muted),
            text_accent: hsla(theme.text_accent),
            cursor: hsla(theme.cursor),
            selection: hsla(theme.selection),
            diff_deleted_bg: hsla(theme.diff_deleted_bg),
            diff_deleted_word: hsla(theme.diff_deleted_word),
            diff_added_bg: hsla(theme.diff_added_bg),
            diff_added_word: hsla(theme.diff_added_word),
            diff_gutter_deleted: hsla(theme.diff_gutter_deleted),
            diff_gutter_added: hsla(theme.diff_gutter_added),
            diff_gutter_modified: hsla(theme.diff_gutter_modified),
            status_error: hsla(theme.status_error),
            status_warning: hsla(theme.status_warning),
            status_ok: hsla(theme.status_ok),
            syntax: SyntaxColors::from(&theme.syntax),
        }
    }
}

impl Default for ThemeColors {
    fn default() -> Self {
        ThemeColors::from(&asteroid_settings::Theme::asteroid_dark())
    }
}

impl ThemeColors {
    /// The theme in force. Installed by [`apply`]; falls back to the built-in
    /// dark theme when nothing has been installed yet (tests).
    pub fn global(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    /// Whether a global theme has been installed.
    pub fn is_installed(cx: &App) -> bool {
        cx.has_global::<Self>()
    }
}

/// Typography from `docs/specs/02-visual.md` §3 and the settings.
///
/// GPUI takes a single family name, so the fallback chains of the spec are
/// resolved here against the families the platform actually has.
pub struct Typography;

impl Typography {
    /// Fallbacks tried after the configured UI family.
    pub const UI_FALLBACKS: &'static [&'static str] = &[
        "Inter",
        "system-ui",
        "Inter Display",
        "Cantarell",
        "DejaVu Sans",
    ];
    /// Fallbacks tried after the configured code family.
    pub const CODE_FALLBACKS: &'static [&'static str] = &[
        "JetBrains Mono",
        "JetBrainsMono Nerd Font Mono",
        "Zed Mono",
        "DejaVu Sans Mono",
        "monospace",
    ];

    /// The first of `families` the platform text system actually has.
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

    /// The UI family to use on this machine: the configured one first, then
    /// the spec's chain.
    pub fn ui_font_family(settings: &Settings, cx: &App) -> Option<SharedString> {
        Self::resolve(&settings.ui_font_family, Self::UI_FALLBACKS, cx)
    }

    /// The code family to use on this machine.
    pub fn buffer_font_family(settings: &Settings, cx: &App) -> Option<SharedString> {
        Self::resolve(&settings.buffer_font_family, Self::CODE_FALLBACKS, cx)
    }

    fn resolve(wanted: &str, fallbacks: &[&str], cx: &App) -> Option<SharedString> {
        let mut chain = vec![wanted];
        chain.extend_from_slice(fallbacks);
        Self::first_installed(cx, &chain)
    }
}

/// Projects the Asteroid theme onto the code editor's own theme type.
///
/// Every token of `02-visual.md` §2 the element paints has a counterpart here;
/// the twelve syntax captures keep the order of
/// [`asteroid_syntax::HighlightId`], which is how the element indexes them.
/// `search_match` has no token of its own in the spec table, so it takes
/// `status.warning`, the colour the editor's own default uses.
impl From<&ThemeColors> for asteroid_editor::EditorTheme {
    fn from(theme: &ThemeColors) -> Self {
        let syntax_by_id = [
            theme.syntax.keyword,
            theme.syntax.function,
            theme.syntax.type_,
            theme.syntax.string,
            theme.syntax.number,
            theme.syntax.comment,
            theme.syntax.variable,
            theme.syntax.property,
            theme.syntax.operator,
            theme.syntax.punctuation,
            theme.syntax.constant,
            theme.syntax.attribute,
        ];
        asteroid_editor::EditorTheme {
            background: theme.bg_editor.into(),
            surface: theme.bg_surface.into(),
            elevated: theme.bg_elevated.into(),
            border: theme.border.into(),
            border_focus: theme.border_focus.into(),
            text: theme.text.into(),
            text_muted: theme.text_muted.into(),
            text_accent: theme.text_accent.into(),
            cursor: theme.cursor.into(),
            selection: theme.selection.into(),
            diff_deleted: theme.diff_deleted_bg.into(),
            diff_added: theme.diff_added_bg.into(),
            diff_modified: theme.diff_gutter_modified.into(),
            search_match: theme.status_warning.into(),
            status_error: theme.status_error.into(),
            status_warning: theme.status_warning.into(),
            status_ok: theme.status_ok.into(),
            syntax: syntax_by_id.map(Into::into),
        }
    }
}

/// The theme every open editor should be using right now.
pub fn editor_theme(cx: &App) -> asteroid_editor::EditorTheme {
    asteroid_editor::EditorTheme::from(ThemeColors::global(cx))
}

/// Projects the user's settings onto the editor element's own settings.
///
/// The font chain is resolved here rather than inside the element: the first
/// family the platform actually has goes first and the spec's fallbacks follow
/// (`docs/etapas/etapa-0.md`, finding 9). `editor.ruler = 0` means "no guide",
/// which the element spells `None`. The zoom of `workspace::zoom_*` scales the
/// code font too, so the editor grows with the rest of the interface.
pub fn editor_settings(cx: &App) -> asteroid_editor::EditorSettings {
    let settings = crate::settings::settings(cx);
    let settings = &settings;
    let ui_scale = crate::settings::ui_scale(cx);
    let mut font_family = Vec::new();
    if let Some(resolved) = Typography::buffer_font_family(settings, cx) {
        font_family.push(resolved.to_string());
    }
    font_family.extend(
        Typography::CODE_FALLBACKS
            .iter()
            .map(|family| (*family).to_string()),
    );
    font_family.dedup();

    asteroid_editor::EditorSettings {
        soft_wrap: settings.editor.soft_wrap,
        tab_size: settings.editor.tab_size,
        insert_spaces: settings.editor.insert_spaces,
        show_whitespace: settings.editor.show_whitespace,
        ruler: (settings.editor.ruler > 0).then_some(settings.editor.ruler),
        cursor_blink: settings.editor.cursor_blink,
        font_family,
        font_size: settings.buffer_font_size * ui_scale,
        line_height: settings.buffer_line_height,
    }
}

/// The WCAG AA contrast floor for normal text.
pub const MIN_CONTRAST: f32 = 4.5;

/// Relative luminance of a colour, as WCAG 2.1 defines it.
fn luminance(color: Hsla) -> f32 {
    let rgba = gpui::Rgba::from(color);
    let channel = |value: f32| {
        if value <= 0.03928 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(rgba.r) + 0.7152 * channel(rgba.g) + 0.0722 * channel(rgba.b)
}

/// WCAG contrast ratio between two colours, from 1 (identical) to 21.
pub fn contrast(a: Hsla, b: Hsla) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    let (light, dark) = if a > b { (a, b) } else { (b, a) };
    (light + 0.05) / (dark + 0.05)
}

/// The most readable of `candidates` over every background a button state can
/// have (base, hover, pressed): the winner is the one whose *worst* state
/// still reads best.
///
/// Used for every filled button: a red "Descartar" is unreadable if the label
/// is red too, and which of our tokens reads best depends on the theme.
fn readable_on(backgrounds: [Hsla; 3], candidates: [Hsla; 4]) -> Hsla {
    let worst = |candidate: Hsla| {
        backgrounds
            .iter()
            .map(|background| contrast(candidate, *background))
            .fold(f32::MAX, f32::min)
    };
    candidates
        .into_iter()
        .max_by(|a, b| {
            worst(*a)
                .partial_cmp(&worst(*b))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(candidates[0])
}

/// Deepens `background` just enough that a label can reach the WCAG AA floor
/// of 4.5 on it, keeping the hue.
///
/// A mid-lightness accent (the blue of the light theme, for one) reads badly
/// under both white and black text; one step at a time away from the middle
/// fixes that without turning the colour into another one.
fn readable_fill(background: Hsla, candidates: [Hsla; 4], dark: bool) -> Hsla {
    let mut background = background;
    for _ in 0..45 {
        let states = [
            background,
            shade(background, 0.06, dark),
            shade(background, 0.12, dark),
        ];
        let ink = readable_on(states, candidates);
        let worst = states
            .iter()
            .map(|state| contrast(ink, *state))
            .fold(f32::MAX, f32::min);
        if worst >= MIN_CONTRAST {
            break;
        }
        background.l = if dark {
            (background.l + 0.01).min(1.)
        } else {
            (background.l - 0.01).max(0.)
        };
    }
    background
}

/// Deepens a *text* colour until it reads on every surface it can land on.
///
/// The accent of a light theme is a mid blue: fine as a fill, thin at 13 px on
/// `bg.surface`. This keeps the hue and moves the lightness away from the
/// background until it clears [`MIN_CONTRAST`].
fn readable_ink(color: Hsla, backgrounds: [Hsla; 3], dark: bool) -> Hsla {
    let mut color = color;
    for _ in 0..45 {
        let worst = backgrounds
            .iter()
            .map(|background| contrast(color, *background))
            .fold(f32::MAX, f32::min);
        if worst >= MIN_CONTRAST {
            break;
        }
        color.l = if dark {
            (color.l + 0.01).min(1.)
        } else {
            (color.l - 0.01).max(0.)
        };
    }
    color
}

/// A slightly stronger version of `color`, for a hover or pressed state:
/// lighter on a dark theme, darker on a light one.
fn shade(color: Hsla, amount: f32, dark: bool) -> Hsla {
    let mut shaded = color;
    shaded.l = if dark {
        (color.l + amount).min(1.)
    } else {
        (color.l - amount).max(0.)
    };
    shaded
}

/// Installs `theme` as the global Asteroid theme and projects it, together
/// with the typography of `settings`, onto gpui-kit's theme.
///
/// `ui_scale` is the zoom factor of `workspace::zoom_*` (1.0 is 100 %).
/// Call after `gpui_kit::init`, which installs gpui-kit's own theme global.
pub fn apply(theme: &asteroid_settings::Theme, settings: &Settings, ui_scale: f32, cx: &mut App) {
    use gpui_kit::component::{Theme as KitTheme, ThemeTokens};

    let theme = ThemeColors::from(theme);
    let dark = theme.dark;
    cx.set_global(theme.clone());

    // Start from gpui-kit's built-in theme of the right mode so every color we
    // do not map keeps a sane value, then overwrite the mapped ones.
    KitTheme::change(
        if dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        },
        None,
        cx,
    );

    let ui_font_family = Typography::ui_font_family(settings, cx);
    let kit = KitTheme::global_mut(cx);

    // §3 Typography. An unresolvable chain leaves gpui-kit's own resolved
    // system font in place.
    if let Some(family) = ui_font_family {
        kit.font_family = family;
    }
    kit.font_size = px(settings.ui_font_size * ui_scale);
    kit.mono_font_size = px(settings.buffer_font_size * ui_scale);

    // §4 Shape: 4px on buttons and cards, 6px on popovers, no shadows on panels.
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

    // text.accent. As *text* the accent has to read on the three surfaces it
    // can land on, which a mid-lightness accent does not do on its own.
    let surfaces = [theme.bg_app, theme.bg_surface, theme.bg_elevated];
    let accent_text = readable_ink(theme.text_accent, surfaces, theme.dark);
    colors.accent_foreground = accent_text;
    colors.link = accent_text;
    colors.link_hover = accent_text;
    colors.link_active = accent_text;
    colors.sidebar_primary = theme.text_accent;
    colors.sidebar_accent_foreground = accent_text;
    colors.slider_bar = theme.text_accent;
    colors.slider_thumb = theme.text;

    // cursor and selection.
    colors.caret = theme.cursor;
    colors.selection = theme.selection;
    colors.list_active = theme.selection;
    colors.table_active = theme.selection;

    // Every button variant, as a readable background/foreground pair.
    //
    // gpui-kit ships its own defaults for the `*_foreground` fields, and
    // leaving them alone is how "Descartar" ended up red on red: the label
    // kept the palette's red while the background became ours. Each variant
    // below therefore sets background, hover, active *and* foreground, and the
    // foreground is picked for contrast (see `readable_on`).
    let dark = theme.dark;
    let ink = |backgrounds: [Hsla; 3]| {
        readable_on(
            backgrounds,
            [theme.bg_app, theme.text, gpui::white(), gpui::black()],
        )
    };

    // Filled: primary, danger, warning, success, info. `states` is the
    // (base, hover, active, foreground) tuple of one variant.
    let candidates = [theme.bg_app, theme.text, gpui::white(), gpui::black()];
    let states = |background: Hsla| {
        let background = readable_fill(background, candidates, dark);
        let hover = shade(background, 0.06, dark);
        let active = shade(background, 0.12, dark);
        (background, hover, active, ink([background, hover, active]))
    };

    {
        let (base, hover, active, ink) = states(theme.text_accent);
        colors.primary = base;
        colors.primary_hover = hover;
        colors.primary_active = active;
        colors.primary_foreground = ink;
        colors.button_primary = base;
        colors.button_primary_hover = hover;
        colors.button_primary_active = active;
        colors.button_primary_foreground = ink;
        colors.sidebar_primary_foreground = ink;
    }
    {
        let (base, hover, active, ink) = states(theme.status_error);
        colors.danger = base;
        colors.danger_hover = hover;
        colors.danger_active = active;
        colors.danger_foreground = ink;
        colors.button_danger = base;
        colors.button_danger_hover = hover;
        colors.button_danger_active = active;
        colors.button_danger_foreground = ink;
    }
    {
        let (base, hover, active, ink) = states(theme.status_warning);
        colors.warning = base;
        colors.warning_hover = hover;
        colors.warning_active = active;
        colors.warning_foreground = ink;
        colors.button_warning = base;
        colors.button_warning_hover = hover;
        colors.button_warning_active = active;
        colors.button_warning_foreground = ink;
    }
    {
        let (base, hover, active, ink) = states(theme.status_ok);
        colors.success = base;
        colors.success_hover = hover;
        colors.success_active = active;
        colors.success_foreground = ink;
        colors.button_success = base;
        colors.button_success_hover = hover;
        colors.button_success_active = active;
        colors.button_success_foreground = ink;
    }
    {
        let (base, hover, active, ink) = states(theme.text_accent);
        colors.info = base;
        colors.info_hover = hover;
        colors.info_active = active;
        colors.info_foreground = ink;
        colors.button_info = base;
        colors.button_info_hover = hover;
        colors.button_info_active = active;
        colors.button_info_foreground = ink;
    }

    // Neutral: the default button and the secondary one, which is what the
    // dialogs use. Surface with a border, never a coloured fill.
    colors.button = theme.bg_elevated;
    colors.button_hover = theme.bg_surface;
    colors.button_active = theme.bg_surface;
    colors.button_foreground = theme.text;
    colors.button_secondary = theme.bg_elevated;
    colors.button_secondary_hover = theme.bg_surface;
    colors.button_secondary_active = theme.bg_surface;
    colors.button_secondary_foreground = theme.text;
    colors.secondary = theme.bg_elevated;
    colors.secondary_hover = theme.bg_surface;
    colors.secondary_active = theme.bg_surface;
    // Ghost buttons paint no background of their own and take this colour, so
    // it has to read on the surface behind them, not on a fill.
    colors.secondary_foreground = theme.text;

    // The `tokens` are gpui-kit's resolved view of `colors`; components read
    // both, so they have to be regenerated after the overwrite.
    kit.tokens = ThemeTokens::from(&kit.colors);

    // Push the result down to gpui-base, which owns scrollbars and resize
    // handles and would otherwise keep the built-in palette.
    KitTheme::sync_base(cx);

    tracing::debug!(
        theme = %ThemeColors::global(cx).name,
        rendering = ?settings.text_rendering,
        "tema aplicado"
    );
    // `text_rendering` is a renderer-level choice in GPUI and gpui-pre 0.3.5
    // exposes no switch for it, so the setting is only logged for now.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_the_spec_table_from_the_settings_theme() {
        let theme = ThemeColors::from(&asteroid_settings::Theme::asteroid_dark());
        assert_eq!(theme.name, "Asteroid Dark");
        assert!(theme.dark);
        assert_eq!(theme.bg_app, rgba(0x1e2127ff).into());
        assert_eq!(theme.bg_editor, rgba(0x282c34ff).into());
        assert_eq!(theme.border_focus, rgba(0x528bffff).into());
        assert_eq!(theme.text_muted, rgba(0x5c6370ff).into());
        assert_eq!(theme.diff_gutter_deleted, rgba(0xe06c75ff).into());
        // Diff tints keep the hue and take the opacity from the spec (18 %
        // and 38 % rounded to a byte).
        assert!((theme.diff_deleted_bg.a - 0.18).abs() < 0.01);
        assert!((theme.diff_added_word.a - 0.38).abs() < 0.01);
    }

    #[test]
    fn light_theme_is_not_dark() {
        let theme = ThemeColors::from(&asteroid_settings::Theme::asteroid_light());
        assert!(!theme.dark);
        assert_eq!(theme.name, "Asteroid Light");
    }

    #[test]
    fn syntax_colors_come_across() {
        let settings_theme = asteroid_settings::Theme::asteroid_dark();
        let theme = ThemeColors::from(&settings_theme);
        assert_eq!(theme.syntax.keyword, hsla(settings_theme.syntax.keyword));
        assert_eq!(theme.syntax.comment, hsla(settings_theme.syntax.comment));
    }

    /// Every filled button variant has to be readable in both themes, which is
    /// what went wrong when "Descartar" came out red on red: the AA floor of
    /// WCAG for normal text is 4.5.
    #[gpui::test]
    fn every_button_variant_is_readable(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            for asteroid in [
                asteroid_settings::Theme::asteroid_dark(),
                asteroid_settings::Theme::asteroid_light(),
            ] {
                let name = asteroid.name.clone();
                apply(&asteroid, &Settings::default(), 1.0, cx);
                let colors = &gpui_kit::component::Theme::global(cx).colors;
                let pairs: [(&str, Hsla, [Hsla; 3]); 7] = [
                    (
                        "primary",
                        colors.button_primary_foreground,
                        [
                            colors.button_primary,
                            colors.button_primary_hover,
                            colors.button_primary_active,
                        ],
                    ),
                    (
                        "danger",
                        colors.button_danger_foreground,
                        [
                            colors.button_danger,
                            colors.button_danger_hover,
                            colors.button_danger_active,
                        ],
                    ),
                    (
                        "warning",
                        colors.button_warning_foreground,
                        [
                            colors.button_warning,
                            colors.button_warning_hover,
                            colors.button_warning_active,
                        ],
                    ),
                    (
                        "success",
                        colors.button_success_foreground,
                        [
                            colors.button_success,
                            colors.button_success_hover,
                            colors.button_success_active,
                        ],
                    ),
                    (
                        "info",
                        colors.button_info_foreground,
                        [
                            colors.button_info,
                            colors.button_info_hover,
                            colors.button_info_active,
                        ],
                    ),
                    (
                        "secondary",
                        colors.button_secondary_foreground,
                        [
                            colors.button_secondary,
                            colors.button_secondary_hover,
                            colors.button_secondary_active,
                        ],
                    ),
                    (
                        "button",
                        colors.button_foreground,
                        [colors.button, colors.button_hover, colors.button_active],
                    ),
                ];
                for (variant, foreground, backgrounds) in pairs {
                    for (state, background) in ["base", "hover", "active"].iter().zip(backgrounds) {
                        let ratio = contrast(foreground, background);
                        assert!(
                            ratio >= 4.5,
                            "{name}: {variant} ({state}) contrasta {ratio:.2}"
                        );
                    }
                }
                // Ghost and link paint no fill: they read on the surfaces.
                for background in [colors.background, colors.popover, colors.secondary] {
                    assert!(
                        contrast(colors.secondary_foreground, background) >= 4.5,
                        "{name}: ghost sobre una superficie"
                    );
                    assert!(
                        contrast(colors.link, background) >= 4.5,
                        "{name}: link sobre una superficie"
                    );
                }
            }
        });
    }
}
