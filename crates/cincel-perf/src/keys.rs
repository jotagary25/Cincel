//! Key strokes for the virtual keyboard: evdev key codes of a US layout.

use anyhow::{Result, bail};

/// Keymap uploaded with the virtual keyboard. The compositor compiles it
/// with its own XKB data, so the includes resolve to the standard US layout.
pub const KEYMAP: &str = "xkb_keymap {\n\
    xkb_keycodes { include \"evdev+aliases(qwerty)\" };\n\
    xkb_types { include \"complete\" };\n\
    xkb_compat { include \"complete\" };\n\
    xkb_symbols { include \"pc+us+inet(evdev)\" };\n\
    xkb_geometry { include \"pc(pc105)\" };\n\
};\n";

pub const KEY_LEFTCTRL: u32 = 29;
pub const KEY_LEFTSHIFT: u32 = 42;
pub const KEY_LEFTALT: u32 = 56;
pub const MASK_SHIFT: u32 = 1;
pub const MASK_CTRL: u32 = 4;
pub const MASK_ALT: u32 = 8;

/// One key press with its modifiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stroke {
    pub key: u32,
    pub modifier_keys: Vec<u32>,
    pub modifier_mask: u32,
}

impl Stroke {
    fn plain(key: u32) -> Self {
        Stroke {
            key,
            modifier_keys: Vec::new(),
            modifier_mask: 0,
        }
    }

    fn shifted(key: u32) -> Self {
        Stroke {
            key,
            modifier_keys: vec![KEY_LEFTSHIFT],
            modifier_mask: MASK_SHIFT,
        }
    }
}

const LETTERS: [u32; 26] = [
    30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20, 22, 47, 17, 45,
    21, 44,
];

/// The stroke that types `c` on a US layout.
pub fn char_stroke(c: char) -> Result<Stroke> {
    Ok(match c {
        'a'..='z' => Stroke::plain(LETTERS[(c as u8 - b'a') as usize]),
        'A'..='Z' => Stroke::shifted(LETTERS[(c as u8 - b'A') as usize]),
        '1'..='9' => Stroke::plain(2 + u32::from(c as u8 - b'1')),
        '0' => Stroke::plain(11),
        '-' => Stroke::plain(12),
        '_' => Stroke::shifted(12),
        '=' => Stroke::plain(13),
        '.' => Stroke::plain(52),
        ',' => Stroke::plain(51),
        '/' => Stroke::plain(53),
        ' ' => Stroke::plain(57),
        other => bail!("no sé escribir el carácter {other:?}"),
    })
}

/// Named keys: `enter`, `backspace`, `escape`, `tab`, `space`, `shift` or a
/// single character.
fn named_key(name: &str) -> Result<Stroke> {
    Ok(match name {
        "enter" | "return" => Stroke::plain(28),
        "backspace" => Stroke::plain(14),
        "escape" | "esc" => Stroke::plain(1),
        "tab" => Stroke::plain(15),
        "space" => Stroke::plain(57),
        "shift" => Stroke::plain(KEY_LEFTSHIFT),
        "down" => Stroke::plain(108),
        "up" => Stroke::plain(103),
        "pagedown" => Stroke::plain(109),
        "pageup" => Stroke::plain(104),
        single if single.chars().count() == 1 => char_stroke(single.chars().next().unwrap_or(' '))?,
        other => bail!("tecla desconocida: {other}"),
    })
}

/// Parses a combination such as `ctrl+p`, `ctrl+shift+p`, `enter` or `a`.
pub fn parse_combo(combo: &str) -> Result<Stroke> {
    let parts: Vec<&str> = combo.split('+').collect();
    let Some((key, modifiers)) = parts.split_last() else {
        bail!("combinación vacía");
    };
    let mut stroke = named_key(&key.to_ascii_lowercase())?;
    for modifier in modifiers {
        let (key, mask) = match modifier.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => (KEY_LEFTCTRL, MASK_CTRL),
            "shift" => (KEY_LEFTSHIFT, MASK_SHIFT),
            "alt" => (KEY_LEFTALT, MASK_ALT),
            other => bail!("modificador desconocido: {other}"),
        };
        if stroke.modifier_mask & mask == 0 {
            stroke.modifier_keys.push(key);
            stroke.modifier_mask |= mask;
        }
    }
    Ok(stroke)
}

/// Strokes that type `text`.
pub fn text_strokes(text: &str) -> Result<Vec<Stroke>> {
    text.chars().map(char_stroke).collect()
}
