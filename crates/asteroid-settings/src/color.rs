//! The color type theme files are written in.
//!
//! GPUI works in `Hsla`, but this crate must not depend on it
//! (`docs/specs/03-arquitectura.md` §2), so themes carry plain sRGB bytes and
//! the GPUI crates convert with `gpui::rgba(color.rgba_u32())`.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

/// An sRGB color with straight (non-premultiplied) alpha.
///
/// Written in JSON as `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`; serialized
/// back as `#rrggbb` when opaque and `#rrggbbaa` otherwise. The spec writes
/// tints as "`#e06c75` at 18%", which is `#e06c752e` here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rgba {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
    /// Alpha channel, `255` meaning opaque.
    pub a: u8,
}

impl Rgba {
    /// An opaque color from a `0xrrggbb` literal.
    pub const fn hex(rgb: u32) -> Self {
        Self {
            r: ((rgb >> 16) & 0xff) as u8,
            g: ((rgb >> 8) & 0xff) as u8,
            b: (rgb & 0xff) as u8,
            a: 0xff,
        }
    }

    /// The same color at `alpha` (0.0 – 1.0) opacity.
    pub fn alpha(self, alpha: f32) -> Self {
        Self {
            a: (alpha.clamp(0., 1.) * 255.).round() as u8,
            ..self
        }
    }

    /// The color packed as `0xrrggbbaa`, the layout `gpui::rgba` expects.
    pub fn rgba_u32(self) -> u32 {
        u32::from_be_bytes([self.r, self.g, self.b, self.a])
    }

    /// The four channels as 0.0 – 1.0 floats.
    pub fn to_f32(self) -> [f32; 4] {
        [
            self.r as f32 / 255.,
            self.g as f32 / 255.,
            self.b as f32 / 255.,
            self.a as f32 / 255.,
        ]
    }

    /// The alpha channel as a 0.0 – 1.0 float.
    pub fn alpha_f32(self) -> f32 {
        self.a as f32 / 255.
    }

    /// Parses `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`. The leading `#` is
    /// optional.
    pub fn parse(text: &str) -> Result<Self, String> {
        let hex = text.strip_prefix('#').unwrap_or(text);
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("«{text}» no es un color hexadecimal"));
        }
        let nibble = |index: usize| -> u8 {
            u8::from_str_radix(&hex[index..index + 1], 16).expect("verified above")
        };
        let byte =
            |index: usize| -> u8 { u8::from_str_radix(&hex[index..index + 2], 16).expect("idem") };
        match hex.len() {
            3 | 4 => Ok(Self {
                r: nibble(0) * 17,
                g: nibble(1) * 17,
                b: nibble(2) * 17,
                a: if hex.len() == 4 { nibble(3) * 17 } else { 0xff },
            }),
            6 | 8 => Ok(Self {
                r: byte(0),
                g: byte(2),
                b: byte(4),
                a: if hex.len() == 8 { byte(6) } else { 0xff },
            }),
            _ => Err(format!(
                "«{text}» debe tener 3, 4, 6 u 8 dígitos hexadecimales"
            )),
        }
    }
}

impl fmt::Display for Rgba {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.a == 0xff {
            write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            write!(
                f,
                "#{:02x}{:02x}{:02x}{:02x}",
                self.r, self.g, self.b, self.a
            )
        }
    }
}

impl Serialize for Rgba {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Rgba {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Rgba::parse(&text).map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_length() {
        assert_eq!(Rgba::parse("#1e2127").unwrap(), Rgba::hex(0x1e2127));
        assert_eq!(Rgba::parse("1e2127").unwrap(), Rgba::hex(0x1e2127));
        assert_eq!(Rgba::parse("#fff").unwrap(), Rgba::hex(0xffffff));
        assert_eq!(Rgba::parse("#0008").unwrap().a, 0x88);
        assert_eq!(Rgba::parse("#e06c752e").unwrap().a, 0x2e);
        assert!(Rgba::parse("#12345").is_err());
        assert!(Rgba::parse("rojo").is_err());
    }

    #[test]
    fn alpha_matches_the_spec_percentages() {
        // "#e06c75 al 18%" and "al 38%".
        assert_eq!(Rgba::hex(0xe06c75).alpha(0.18).to_string(), "#e06c752e");
        assert_eq!(Rgba::hex(0x98c379).alpha(0.38).to_string(), "#98c37961");
        assert!((Rgba::hex(0xe06c75).alpha(0.18).alpha_f32() - 0.18).abs() < 0.005);
    }

    #[test]
    fn round_trips_through_json() {
        for color in [
            Rgba::hex(0x1e2127),
            Rgba::hex(0xe06c75).alpha(0.18),
            Rgba::hex(0x000000).alpha(0.),
        ] {
            let json = serde_json::to_string(&color).unwrap();
            assert_eq!(serde_json::from_str::<Rgba>(&json).unwrap(), color);
        }
    }

    #[test]
    fn packs_the_way_gpui_expects() {
        assert_eq!(Rgba::hex(0x1e2127).rgba_u32(), 0x1e2127ff);
        assert_eq!(Rgba::hex(0xe06c75).alpha(0.18).rgba_u32(), 0xe06c752e);
    }
}
