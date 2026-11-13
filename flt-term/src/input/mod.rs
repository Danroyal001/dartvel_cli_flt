//! Terminal input bytes to key, mouse, paste and focus events.
//!
//! Reads both what a modern terminal sends with the Kitty keyboard protocol
//! enabled (every key an escape code, with releases and the text it types)
//! and the legacy bytes every other terminal sends, plus SGR mouse reports
//! (mode 1006, and 1016 where coordinates are pixels), bracketed paste and
//! focus reports. Sequences may arrive split anywhere; a partial one waits for
//! the rest. A lone ESC is ambiguous until a short time passes with nothing
//! after it, so the runner calls [InputParser::flush] after that pause.

/// Modifier keys held during an event.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
    pub meta: bool,
}

impl Modifiers {
    pub const NONE: Modifiers = Modifiers { shift: false, alt: false, control: false, meta: false };
    pub const SHIFT: Modifiers = Modifiers { shift: true, ..Modifiers::NONE };
    pub const ALT: Modifiers = Modifiers { alt: true, ..Modifiers::NONE };
    pub const CONTROL: Modifiers = Modifiers { control: true, ..Modifiers::NONE };

    /// From the xterm/Kitty modifier parameter (1 + bit mask).
    pub fn from_parameter(value: u32) -> Modifiers {
        let bits = value.saturating_sub(1);
        Modifiers {
            shift: bits & 1 != 0,
            alt: bits & 2 != 0,
            control: bits & 4 != 0,
            // Super (8) and Meta (32) both land on Flutter's meta.
            meta: bits & (8 | 32) != 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        *self == Modifiers::NONE
    }
}

/// Left or right, for modifier keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Left,
    Right,
}

/// A key, by what it is rather than what it types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    /// A key that types a character, as its unshifted base: `a`, not `A`.
    Char(char),
    Enter,
    Escape,
    Backspace,
    Tab,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    /// F1 to F35.
    Function(u8),
    Shift(Side),
    Control(Side),
    Alt(Side),
    Meta(Side),
    CapsLock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    Press,
    Repeat,
    Release,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: Key,
    pub modifiers: Modifiers,
    pub kind: KeyKind,
    /// What the key types, if anything: `A` for shift+a.
    pub text: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
    Back,
    Forward,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseKind {
    Press(Button),
    Release(Button),
    /// Motion; `held` is the button being dragged with, if any.
    Move { held: Option<Button> },
    /// Wheel steps: positive `down` is towards the user, positive `right`
    /// is right.
    Scroll { right: i32, down: i32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseEvent {
    /// 0-based, in cells -- or pixels with SGR pixel reporting (1016).
    pub x: u32,
    pub y: u32,
    pub kind: MouseKind,
    pub modifiers: Modifiers,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEvent {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Paste(String),
    Focus(bool),
}

/// Turns input bytes into events.
#[derive(Debug, Default)]
pub struct InputParser {
    pending: Vec<u8>,
    paste: Option<Vec<u8>>,
}

impl InputParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bytes read from the terminal; the events they complete.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<InputEvent> {
        self.pending.extend_from_slice(bytes);
        let mut events = Vec::new();
        let buffer = std::mem::take(&mut self.pending);
        let mut at = 0;
        while at < buffer.len() {
            if let Some(paste) = self.paste.as_mut() {
                // Inside a paste everything is text until the end marker.
                const END: &[u8] = b"\x1b[201~";
                match find(&buffer[at..], END) {
                    Some(end) => {
                        paste.extend_from_slice(&buffer[at..at + end]);
                        let text = String::from_utf8_lossy(paste).into_owned();
                        self.paste = None;
                        events.push(InputEvent::Paste(text));
                        at += end + END.len();
                    }
                    None => {
                        // Keep a tail that could be the start of the marker.
                        let keep = partial_suffix(&buffer[at..], END);
                        let end = buffer.len() - keep;
                        paste.extend_from_slice(&buffer[at..end]);
                        at = end;
                        break;
                    }
                }
                continue;
            }
            match self.parse_one(&buffer[at..], &mut events) {
                Some(used) => at += used,
                None => break,
            }
        }
        self.pending = buffer[at..].to_vec();
        events
    }

    pub fn flush(&mut self) -> Vec<InputEvent> {
        let pending = std::mem::take(&mut self.pending);
        let mut events = Vec::new();
        match pending.as_slice() {
            [] => {}
            [0x1b] => events.push(press(Key::Escape, Modifiers::NONE, None)),
            [0x1b, second, rest @ ..] => {
                events.push(press(Key::Char(*second as char), Modifiers::ALT, None));
                events.extend(self.feed(rest));
                events.extend(self.flush());
            }
            // An incomplete UTF-8 character that never completed.
            _ => {}
        }
        events
    }

    /// Parses one event or sequence from the start of `bytes`, returning how
    /// many bytes it used, or None when they are the start of something
    /// incomplete.
    fn parse_one(&mut self, bytes: &[u8], events: &mut Vec<InputEvent>) -> Option<usize> {
        let first = bytes[0];
        if first != 0x1b {
            return parse_plain(bytes, events);
        }
        let Some(&second) = bytes.get(1) else { return None };
        match second {
            b'[' => {
                let body = &bytes[2..];
                let end = body.iter().position(|&byte| (0x40..=0x7e).contains(&byte))?;
                let used = 2 + end + 1;
                let params = &body[..end];
                let last = body[end];
                if params == b"200" && last == b'~' {
                    self.paste = Some(Vec::new());
                } else if let Some(event) = parse_csi(params, last) {
                    events.push(event);
                }
                Some(used)
            }
            b'O' => {
                let &last = bytes.get(2)?;
                let key = match last {
                    b'A' => Some(Key::Up),
                    b'B' => Some(Key::Down),
                    b'C' => Some(Key::Right),
                    b'D' => Some(Key::Left),
                    b'H' => Some(Key::Home),
                    b'F' => Some(Key::End),
                    b'P' => Some(Key::Function(1)),
                    b'Q' => Some(Key::Function(2)),
                    b'R' => Some(Key::Function(3)),
                    b'S' => Some(Key::Function(4)),
                    _ => None,
                };
                if let Some(key) = key {
                    events.push(press(key, Modifiers::NONE, None));
                }
                Some(3)
            }
            // APC and DCS: terminal replies, never typing.
            b'_' | b'P' | b']' => {
                let body = &bytes[2..];
                let end = find(body, b"\x1b\\").map(|end| end + 2).or_else(|| {
                    // OSC may end with BEL.
                    (second == b']').then(|| body.iter().position(|&b| b == 0x07).map(|end| end + 1)).flatten()
                })?;
                Some(2 + end)
            }
            0x1b => {
                // ESC ESC: an escape, then whatever follows.
                events.push(press(Key::Escape, Modifiers::NONE, None));
                Some(1)
            }
            _ => {
                // Alt plus whatever the next key is.
                let mut inner = Vec::new();
                let used = parse_plain(&bytes[1..], &mut inner)?;
                for event in inner {
                    if let InputEvent::Key(mut key) = event {
                        key.modifiers.alt = true;
                        key.text = None;
                        events.push(InputEvent::Key(key));
                    }
                }
                Some(1 + used)
            }
        }
    }

    /// Whether bytes are waiting for the rest of a sequence (the runner
    /// arms the ESC timer only then).
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }
}

fn press(key: Key, modifiers: Modifiers, text: Option<String>) -> InputEvent {
    InputEvent::Key(KeyEvent { key, modifiers, kind: KeyKind::Press, text })
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

/// How many trailing bytes of `bytes` are a proper prefix of `marker`.
fn partial_suffix(bytes: &[u8], marker: &[u8]) -> usize {
    (1..marker.len()).rev().find(|&length| bytes.ends_with(&marker[..length])).unwrap_or(0)
}

/// A byte or UTF-8 character that is not part of an escape sequence.
fn parse_plain(bytes: &[u8], events: &mut Vec<InputEvent>) -> Option<usize> {
    let first = bytes[0];
    let event = match first {
        b'\r' | b'\n' => press(Key::Enter, Modifiers::NONE, None),
        b'\t' => press(Key::Tab, Modifiers::NONE, None),
        0x7f | 0x08 => press(Key::Backspace, Modifiers::NONE, None),
        0x00 => press(Key::Char(' '), Modifiers::CONTROL, None),
        0x01..=0x1a => press(Key::Char((b'a' + first - 1) as char), Modifiers::CONTROL, None),
        0x1c..=0x1f => press(Key::Char((first + 0x40) as char), Modifiers::CONTROL, None),
        0x20..=0x7e => {
            let character = first as char;
            if character.is_ascii_uppercase() {
                press(Key::Char(character.to_ascii_lowercase()), Modifiers::SHIFT, Some(character.to_string()))
            } else {
                press(Key::Char(character), Modifiers::NONE, Some(character.to_string()))
            }
        }
        _ => {
            let length = match first {
                0xc0..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf7 => 4,
                _ => return Some(1), // a stray continuation byte
            };
            if bytes.len() < length {
                return None;
            }
            match std::str::from_utf8(&bytes[..length]) {
                Ok(text) => {
                    let character = text.chars().next().unwrap();
                    let lower = character.to_lowercase().next().unwrap_or(character);
                    let modifiers = if lower != character { Modifiers::SHIFT } else { Modifiers::NONE };
                    events.push(press(Key::Char(lower), modifiers, Some(text.to_string())));
                    return Some(length);
                }
                Err(_) => return Some(1),
            }
        }
    };
    events.push(event);
    Some(1)
}

/// `params` are the bytes between `CSI` and the final byte `last`.
fn parse_csi(params: &[u8], last: u8) -> Option<InputEvent> {
    let text = std::str::from_utf8(params).ok()?;
    if let Some(mouse) = text.strip_prefix('<') {
        return parse_sgr_mouse(mouse, last);
    }
    // Replies to queries (`CSI ? ... c/u/y`, `CSI 6;h;w t`, `CSI r;c R`) are
    // not input.
    if text.starts_with('?') || text.starts_with('>') || last == b't' || last == b'y' {
        return None;
    }
    let fields: Vec<Vec<u32>> = text
        .split(';')
        .map(|field| field.split(':').map(|part| part.parse().unwrap_or(0)).collect())
        .collect();
    let number = |field: usize, part: usize| -> Option<u32> {
        fields.get(field).and_then(|parts| parts.get(part)).copied().filter(|&value| value != 0)
    };
    let modifiers = Modifiers::from_parameter(number(1, 0).unwrap_or(1));
    let kind = match number(1, 1) {
        Some(2) => KeyKind::Repeat,
        Some(3) => KeyKind::Release,
        _ => KeyKind::Press,
    };
    let key = match last {
        b'I' if text.is_empty() => return Some(InputEvent::Focus(true)),
        b'O' if text.is_empty() => return Some(InputEvent::Focus(false)),
        b'A' => Key::Up,
        b'B' => Key::Down,
        b'C' => Key::Right,
        b'D' => Key::Left,
        b'H' => Key::Home,
        b'F' => Key::End,
        b'P' => Key::Function(1),
        b'Q' => Key::Function(2),
        // `CSI row ; col R` is a cursor position report; F3 is `CSI R` or
        // `CSI 1 ; mods R`.
        b'R' if fields.len() == 1 && text.is_empty() || number(0, 0) == Some(1) => Key::Function(3),
        b'S' => Key::Function(4),
        b'Z' => return Some(press(Key::Tab, Modifiers { shift: true, ..modifiers }, None)),
        b'~' => match number(0, 0)? {
            1 | 7 => Key::Home,
            2 => Key::Insert,
            3 => Key::Delete,
            4 | 8 => Key::End,
            5 => Key::PageUp,
            6 => Key::PageDown,
            11..=15 => Key::Function((number(0, 0)? - 10) as u8),
            17..=21 => Key::Function((number(0, 0)? - 11) as u8),
            23..=26 => Key::Function((number(0, 0)? - 12) as u8),
            28 | 29 => Key::Function((number(0, 0)? - 13) as u8),
            31..=34 => Key::Function((number(0, 0)? - 14) as u8),
            _ => return None,
        },
        b'u' => kitty_key(number(0, 0)?)?,
        _ => return None,
    };
    let typed: String = fields
        .get(2)
        .map(|parts| parts.iter().filter_map(|&code| char::from_u32(code)).collect())
        .unwrap_or_default();
    let text = (kind != KeyKind::Release && !typed.is_empty()).then_some(typed);
    Some(InputEvent::Key(KeyEvent { key, modifiers, kind, text }))
}

/// The Kitty keyboard protocol's key codes.
fn kitty_key(code: u32) -> Option<Key> {
    Some(match code {
        9 => Key::Tab,
        13 => Key::Enter,
        27 => Key::Escape,
        127 => Key::Backspace,
        57358 => Key::CapsLock,
        57364..=57398 => Key::Function((code - 57364 + 13) as u8),
        57441 => Key::Shift(Side::Left),
        57442 => Key::Control(Side::Left),
        57443 => Key::Alt(Side::Left),
        57444 | 57446 => Key::Meta(Side::Left),
        57447 => Key::Shift(Side::Right),
        57448 => Key::Control(Side::Right),
        57449 => Key::Alt(Side::Right),
        57450 | 57452 => Key::Meta(Side::Right),
        // Other functional keys in the private-use range have no Flutter
        // counterpart worth guessing at.
        57344..=63743 => return None,
        _ => Key::Char(char::from_u32(code)?),
    })
}

fn parse_sgr_mouse(params: &str, last: u8) -> Option<InputEvent> {
    let mut values = params.split(';').map(|value| value.parse::<u32>().ok());
    let code = values.next()??;
    let x = values.next()??.saturating_sub(1);
    let y = values.next()??.saturating_sub(1);
    let modifiers = Modifiers {
        shift: code & 4 != 0,
        alt: code & 8 != 0,
        control: code & 16 != 0,
        meta: false,
    };
    let motion = code & 32 != 0;
    let button = match (code & 0b1100_0011, code & 64 != 0) {
        (_, true) => None,
        (0, _) => Some(Button::Left),
        (1, _) => Some(Button::Middle),
        (2, _) => Some(Button::Right),
        (128, _) => Some(Button::Back),
        (129, _) => Some(Button::Forward),
        _ => None,
    };
    let kind = if code & 64 != 0 {
        match code & 3 {
            0 => MouseKind::Scroll { right: 0, down: -1 },
            1 => MouseKind::Scroll { right: 0, down: 1 },
            2 => MouseKind::Scroll { right: -1, down: 0 },
            _ => MouseKind::Scroll { right: 1, down: 0 },
        }
    } else if motion {
        MouseKind::Move { held: button }
    } else if last == b'm' {
        MouseKind::Release(button?)
    } else {
        MouseKind::Press(button?)
    };
    Some(InputEvent::Mouse(MouseEvent { x, y, kind, modifiers }))
}

#[cfg(test)]
mod tests;
