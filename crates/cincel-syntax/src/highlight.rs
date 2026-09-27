//! The 12 standard highlight captures and the theme that colours them.

use std::fmt;

/// An RGB colour, without any GPUI type: `cincel-syntax` has no graphics
/// dependency. The UI converts it to whatever the renderer wants.
pub type Rgb = [u8; 3];

/// One of the 12 standard highlight captures of `docs/specs/02-visual.md §2`.
///
/// Capture names coming out of a tree-sitter query (`variable.parameter`,
/// `function.method.builtin`, …) are mapped onto these with the same rule
/// `tree_sitter_highlight::HighlightConfiguration::configure` uses: the
/// recognized name whose dot-separated parts are all present in the capture
/// name wins, longest match first, ties broken by the order below.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum HighlightId {
    /// `keyword`
    Keyword = 0,
    /// `function`
    Function = 1,
    /// `type`
    Type = 2,
    /// `string`
    String = 3,
    /// `number`
    Number = 4,
    /// `comment`
    Comment = 5,
    /// `variable`
    Variable = 6,
    /// `property`
    Property = 7,
    /// `operator`
    Operator = 8,
    /// `punctuation`
    Punctuation = 9,
    /// `constant`
    Constant = 10,
    /// `attribute`
    Attribute = 11,
}

impl HighlightId {
    /// Every capture, in precedence order.
    pub const ALL: [HighlightId; 12] = [
        HighlightId::Keyword,
        HighlightId::Function,
        HighlightId::Type,
        HighlightId::String,
        HighlightId::Number,
        HighlightId::Comment,
        HighlightId::Variable,
        HighlightId::Property,
        HighlightId::Operator,
        HighlightId::Punctuation,
        HighlightId::Constant,
        HighlightId::Attribute,
    ];

    /// The canonical name of this capture.
    pub fn name(self) -> &'static str {
        match self {
            HighlightId::Keyword => "keyword",
            HighlightId::Function => "function",
            HighlightId::Type => "type",
            HighlightId::String => "string",
            HighlightId::Number => "number",
            HighlightId::Comment => "comment",
            HighlightId::Variable => "variable",
            HighlightId::Property => "property",
            HighlightId::Operator => "operator",
            HighlightId::Punctuation => "punctuation",
            HighlightId::Constant => "constant",
            HighlightId::Attribute => "attribute",
        }
    }

    /// Index into a [`HighlightTheme`]'s colour table.
    pub fn index(self) -> usize {
        self as usize
    }

    /// Maps a tree-sitter query capture name onto a capture, or `None` when the
    /// query captures something this theme does not colour (`text.title`,
    /// `label`, `@local.scope`, …).
    pub fn from_capture_name(capture_name: &str) -> Option<HighlightId> {
        let parts: Vec<&str> = capture_name.split('.').collect();
        let mut best: Option<(usize, HighlightId)> = None;
        for id in HighlightId::ALL {
            let recognized: Vec<&str> = id.name().split('.').collect();
            if recognized.iter().all(|part| parts.contains(part)) {
                let len = recognized.len();
                if best.is_none_or(|(best_len, _)| len > best_len) {
                    best = Some((len, id));
                }
            }
        }
        best.map(|(_, id)| id)
    }
}

impl fmt::Display for HighlightId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Colour per [`HighlightId`]. Lives here, with no GPUI types, so the same
/// values can be loaded from a theme JSON and handed to the renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HighlightTheme {
    /// Colours indexed by [`HighlightId::index`].
    pub colors: [Rgb; 12],
}

impl HighlightTheme {
    /// The One Dark palette of `docs/specs/02-visual.md §2`.
    pub const fn one_dark() -> Self {
        Self {
            colors: [
                [0xc6, 0x78, 0xdd], // keyword
                [0x61, 0xaf, 0xef], // function
                [0xe5, 0xc0, 0x7b], // type
                [0x98, 0xc3, 0x79], // string
                [0xd1, 0x9a, 0x66], // number
                [0x5c, 0x63, 0x70], // comment
                [0xab, 0xb2, 0xbf], // variable
                [0xd1, 0x9a, 0x66], // property
                [0x56, 0xb6, 0xc2], // operator
                [0xab, 0xb2, 0xbf], // punctuation
                [0xd1, 0x9a, 0x66], // constant
                [0x61, 0xaf, 0xef], // attribute
            ],
        }
    }

    /// The colour of a capture.
    pub fn color(&self, id: HighlightId) -> Rgb {
        self.colors[id.index()]
    }

    /// The colour of a capture as `0x00RRGGBB`.
    pub fn color_u32(&self, id: HighlightId) -> u32 {
        let [r, g, b] = self.color(id);
        (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
    }

    /// Overrides one colour.
    pub fn set_color(&mut self, id: HighlightId, color: Rgb) {
        self.colors[id.index()] = color;
    }
}

impl Default for HighlightTheme {
    fn default() -> Self {
        Self::one_dark()
    }
}
