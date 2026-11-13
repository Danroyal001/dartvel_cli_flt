//! Terminal keys to the identifiers Flutter's keyboard expects.
//!
//! Flutter takes each key twice: as `KeyData` (physical and logical key ids,
//! which `HardwareKeyboard` and shortcuts read) followed by a raw event on
//! `flutter/keyevent` in the GTK shape, which is what dispatches the queued
//! `KeyData`. Physical ids are USB HID usages; a terminal never says which
//! physical key was pressed, so they assume a US layout, which is what
//! shortcuts are written against. Logical ids follow Flutter's
//! `LogicalKeyboardKey` planes.

use crate::input::{Key, Modifiers, Side};

/// Flutter's ids for one key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlutterKey {
    /// `PhysicalKeyboardKey.usbHidUsage`.
    pub physical: u64,
    /// `LogicalKeyboardKey.keyId`.
    pub logical: u64,
    /// GDK keyval, for the raw event's `keyCode`.
    pub gtk_key_code: u32,
    /// X11 keycode (evdev + 8), for the raw event's `scanCode`.
    pub gtk_scan_code: u32,
}

const UNPRINTABLE_PLANE: u64 = 0x0100000000;
const SYNONYM_PLANE: u64 = 0x0200000000;

/// The ids for `key`. `text` is what it typed, used for keys a US layout
/// does not have.
pub fn flutter_key(key: Key) -> FlutterKey {
    let usage = |code: u64| 0x00070000 | code;
    let named = |hid: u64, logical: u64, gdk: u32, evdev: u32| FlutterKey {
        physical: usage(hid),
        logical,
        gtk_key_code: gdk,
        gtk_scan_code: evdev + 8,
    };
    match key {
        Key::Char(character) => {
            let (hid, evdev) = us_layout(character).unwrap_or((0, 0));
            let code = character as u64;
            let logical = if character.is_ascii_uppercase() { code + 0x20 } else { code };
            FlutterKey {
                // Off the US layout there is no physical key to name; a
                // stable id in an unused plane keeps press and release paired.
                physical: if hid != 0 { usage(hid) } else { 0x1100000000 | code },
                logical,
                gtk_key_code: if code < 0x100 { logical as u32 } else { 0x01000000 | code as u32 },
                gtk_scan_code: if evdev != 0 { evdev + 8 } else { 0 },
            }
        }
        Key::Enter => named(0x28, UNPRINTABLE_PLANE | 0x0d, 0xff0d, 28),
        Key::Escape => named(0x29, UNPRINTABLE_PLANE | 0x1b, 0xff1b, 1),
        Key::Backspace => named(0x2a, UNPRINTABLE_PLANE | 0x08, 0xff08, 14),
        Key::Tab => named(0x2b, UNPRINTABLE_PLANE | 0x09, 0xff09, 15),
        Key::Delete => named(0x4c, UNPRINTABLE_PLANE | 0x7f, 0xffff, 111),
        Key::Insert => named(0x49, UNPRINTABLE_PLANE | 0x407, 0xff63, 110),
        Key::Down => named(0x51, UNPRINTABLE_PLANE | 0x301, 0xff54, 108),
        Key::Left => named(0x50, UNPRINTABLE_PLANE | 0x302, 0xff51, 105),
        Key::Right => named(0x4f, UNPRINTABLE_PLANE | 0x303, 0xff53, 106),
        Key::Up => named(0x52, UNPRINTABLE_PLANE | 0x304, 0xff52, 103),
        Key::End => named(0x4d, UNPRINTABLE_PLANE | 0x305, 0xff57, 107),
        Key::Home => named(0x4a, UNPRINTABLE_PLANE | 0x306, 0xff50, 102),
        Key::PageDown => named(0x4e, UNPRINTABLE_PLANE | 0x307, 0xff56, 109),
        Key::PageUp => named(0x4b, UNPRINTABLE_PLANE | 0x308, 0xff55, 104),
        Key::CapsLock => named(0x39, UNPRINTABLE_PLANE | 0x104, 0xffe5, 58),
        Key::Function(number) => {
            let number = number.clamp(1, 24) as u64;
            let hid = if number <= 12 { 0x3a + number - 1 } else { 0x68 + number - 13 };
            let evdev = match number {
                1..=10 => 59 + number as u32 - 1,
                11 => 87,
                12 => 88,
                _ => 183 + number as u32 - 13,
            };
            named(hid, UNPRINTABLE_PLANE | (0x800 + number), 0xffbe + number as u32 - 1, evdev)
        }
        Key::Control(side) => sided(side, 0xe0, 0xe4, 0x100, 0xffe3, 29, 97),
        Key::Shift(side) => sided(side, 0xe1, 0xe5, 0x102, 0xffe1, 42, 54),
        Key::Alt(side) => sided(side, 0xe2, 0xe6, 0x104, 0xffe9, 56, 100),
        Key::Meta(side) => sided(side, 0xe3, 0xe7, 0x106, 0xffeb, 125, 126),
    }
}

/// A modifier: left and right differ by one in every numbering but evdev.
fn sided(side: Side, left_hid: u64, right_hid: u64, left_logical: u64, left_gdk: u32, left_evdev: u32, right_evdev: u32) -> FlutterKey {
    let right = side == Side::Right;
    FlutterKey {
        physical: 0x00070000 | if right { right_hid } else { left_hid },
        logical: SYNONYM_PLANE | (left_logical + right as u64),
        gtk_key_code: left_gdk + right as u32,
        gtk_scan_code: if right { right_evdev } else { left_evdev } + 8,
    }
}

/// (HID usage, evdev code) of the US-layout key that types `character`,
/// shifted or not.
fn us_layout(character: char) -> Option<(u64, u32)> {
    let character = character.to_ascii_lowercase();
    const LETTER_EVDEV: [u32; 26] = [
        30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20, 22, 47, 17, 45, 21, 44,
    ];
    Some(match character {
        'a'..='z' => {
            let index = character as u64 - 'a' as u64;
            (0x04 + index, LETTER_EVDEV[index as usize])
        }
        '1'..='9' => {
            let index = character as u64 - '1' as u64;
            (0x1e + index, 2 + index as u32)
        }
        '0' | ')' => (0x27, 11),
        '!' => (0x1e, 2),
        '@' => (0x1f, 3),
        '#' => (0x20, 4),
        '$' => (0x21, 5),
        '%' => (0x22, 6),
        '^' => (0x23, 7),
        '&' => (0x24, 8),
        '*' => (0x25, 9),
        '(' => (0x26, 10),
        ' ' => (0x2c, 57),
        '-' | '_' => (0x2d, 12),
        '=' | '+' => (0x2e, 13),
        '[' | '{' => (0x2f, 26),
        ']' | '}' => (0x30, 27),
        '\\' | '|' => (0x31, 43),
        ';' | ':' => (0x33, 39),
        '\'' | '"' => (0x34, 40),
        '`' | '~' => (0x35, 41),
        ',' | '<' => (0x36, 51),
        '.' | '>' => (0x37, 52),
        '/' | '?' => (0x38, 53),
        _ => return None,
    })
}

pub fn gtk_modifiers(modifiers: Modifiers) -> u32 {
    (modifiers.shift as u32) | (modifiers.control as u32) << 2 | (modifiers.alt as u32) << 3 | (modifiers.meta as u32) << 26
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_use_hid_usages_and_lowercase_logical_ids() {
        assert_eq!(
            flutter_key(Key::Char('a')),
            FlutterKey { physical: 0x00070004, logical: 0x61, gtk_key_code: 0x61, gtk_scan_code: 30 + 8 }
        );
        assert_eq!(flutter_key(Key::Char('z')).physical, 0x0007001d);
        assert_eq!(flutter_key(Key::Char('q')).gtk_scan_code, 16 + 8);
    }

    #[test]
    fn digits_and_punctuation() {
        assert_eq!(flutter_key(Key::Char('1')).physical, 0x0007001e);
        assert_eq!(flutter_key(Key::Char('0')).physical, 0x00070027);
        assert_eq!(flutter_key(Key::Char('0')).gtk_scan_code, 11 + 8);
        assert_eq!(flutter_key(Key::Char(' ')), FlutterKey { physical: 0x0007002c, logical: 0x20, gtk_key_code: 0x20, gtk_scan_code: 57 + 8 });
        assert_eq!(flutter_key(Key::Char('/')).physical, 0x00070038);
        assert_eq!(flutter_key(Key::Char('`')).physical, 0x00070035);
    }

    #[test]
    fn editing_and_navigation_keys() {
        assert_eq!(flutter_key(Key::Enter), FlutterKey { physical: 0x00070028, logical: 0x010000000d, gtk_key_code: 0xff0d, gtk_scan_code: 28 + 8 });
        assert_eq!(flutter_key(Key::Escape).logical, 0x010000001b);
        assert_eq!(flutter_key(Key::Backspace).physical, 0x0007002a);
        assert_eq!(flutter_key(Key::Tab).logical, 0x0100000009);
        assert_eq!(flutter_key(Key::Delete).logical, 0x010000007f);
        assert_eq!(flutter_key(Key::Up), FlutterKey { physical: 0x00070052, logical: 0x0100000304, gtk_key_code: 0xff52, gtk_scan_code: 103 + 8 });
        assert_eq!(flutter_key(Key::Left).logical, 0x0100000302);
        assert_eq!(flutter_key(Key::PageDown).physical, 0x0007004e);
        assert_eq!(flutter_key(Key::Home).logical, 0x0100000306);
        assert_eq!(flutter_key(Key::Function(1)), FlutterKey { physical: 0x0007003a, logical: 0x0100000801, gtk_key_code: 0xffbe, gtk_scan_code: 59 + 8 });
        assert_eq!(flutter_key(Key::Function(12)).physical, 0x00070045);
        assert_eq!(flutter_key(Key::Function(12)).gtk_scan_code, 88 + 8);
    }

    #[test]
    fn modifiers_are_sided() {
        assert_eq!(flutter_key(Key::Shift(Side::Left)), FlutterKey { physical: 0x000700e1, logical: 0x0200000102, gtk_key_code: 0xffe1, gtk_scan_code: 42 + 8 });
        assert_eq!(flutter_key(Key::Shift(Side::Right)).logical, 0x0200000103);
        assert_eq!(flutter_key(Key::Control(Side::Left)).physical, 0x000700e0);
        assert_eq!(flutter_key(Key::Control(Side::Left)).logical, 0x0200000100);
        assert_eq!(flutter_key(Key::Alt(Side::Left)).logical, 0x0200000104);
        assert_eq!(flutter_key(Key::Meta(Side::Right)).physical, 0x000700e7);
    }

    #[test]
    fn a_character_off_the_us_layout_still_gets_ids() {
        let key = flutter_key(Key::Char('é'));
        assert_eq!(key.logical, 0xe9);
        assert_ne!(key.physical, 0);
        assert_eq!(key.gtk_key_code, 0xe9);
    }

    #[test]
    fn gtk_modifier_mask() {
        assert_eq!(gtk_modifiers(Modifiers::NONE), 0);
        assert_eq!(gtk_modifiers(Modifiers { shift: true, control: true, alt: true, meta: true }), 1 | 4 | 8 | (1 << 26));
    }
}
