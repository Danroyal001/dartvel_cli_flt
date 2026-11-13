//! Which terminal this is and how to draw in it.
//!
//! At start the runner writes [probe_query] to the terminal, collects the
//! replies with [Replies::feed] until the primary device attributes answer
//! arrives (every terminal answers that one, so a terminal that ignores the
//! rest still ends the wait promptly), and hands them with the environment to
//! [decide]. Every decision is made from what the terminal said, the
//! environment, and explicit configuration, in that order of trust, with the
//! configuration winning. Nothing here stops the application: a terminal
//! with no image protocol gets the best cell mode it can show.

use crate::frame::{CellShape, ColorDepth, Multiplexer};

/// The image id the Kitty graphics query uses.
pub const KITTY_QUERY_ID: u32 = 31;

/// What the process knows about where it runs, before asking the terminal.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Environment {
    /// `$TERM`.
    pub term: String,
    /// `$TERM_PROGRAM`.
    pub term_program: String,
    /// `$COLORTERM`.
    pub colorterm: String,
    /// `$VTE_VERSION`, e.g. 7600 for VTE 0.76.
    pub vte_version: Option<u32>,
    /// Running inside tmux (`$TMUX` set).
    pub tmux: bool,
    /// The terminal outside tmux, from `tmux display -p '#{client_termname}
    /// #{client_termtype}'`. Empty when unknown.
    pub tmux_client_terminal: String,
    /// Running inside GNU screen (`$STY` set, `$TERM` screen*, no tmux).
    pub screen: bool,
    /// Windows Terminal (`$WT_SESSION` set).
    pub windows_terminal: bool,
    /// A Windows console with no virtual-terminal processing (before
    /// Windows 10 1511), driven through the console API.
    pub legacy_windows_console: bool,
    /// Connected over SSH (`$SSH_CONNECTION` or `$SSH_TTY`).
    pub ssh: bool,
    /// The controlling terminal is a Linux virtual console (`/dev/ttyN`).
    pub linux_console: bool,
    /// `/dev/fb0` (or `$FRAMEBUFFER`) can be opened for writing.
    pub framebuffer_writable: bool,
    /// Explicit configuration, which wins over everything detected.
    pub overrides: Overrides,
}

/// Configuration from the application (`dartvel.terminal` in pubspec.yaml,
/// passed to the runner as `DARTVEL_TERMINAL_*`) or the user's environment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Overrides {
    pub graphics: Option<Output>,
    pub color: Option<ColorDepth>,
    pub cells: Option<CellShape>,
}

/// How frames reach the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    /// The Kitty graphics protocol. `placeholders` when inside tmux, where
    /// images are transmitted through passthrough and placed with Unicode
    /// placeholder cells that tmux moves like text.
    Kitty { placeholders: bool },
    /// DEC Sixel.
    Sixel,
    /// The Linux framebuffer, on a local virtual console.
    Framebuffer,
    /// Coloured cells: octants, sextants or half-blocks.
    Cells,
}

/// Keyboard reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keyboard {
    /// The Kitty keyboard protocol: every key as an escape code, with
    /// releases, so keys go to Flutter as real down/up pairs.
    Kitty,
    /// Legacy bytes; releases are synthesized.
    Legacy,
}

/// Everything decided about the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    pub output: Output,
    pub color: ColorDepth,
    pub cells: CellShape,
    pub multiplexer: Multiplexer,
    pub keyboard: Keyboard,
    /// SGR pixel mouse reporting (mode 1016) is available.
    pub mouse_pixels: bool,
    /// The terminal's cell size in pixels, when it said.
    pub cell_pixels: Option<(u32, u32)>,
}

/// The bytes that ask the terminal what it can do.
///
/// In order: the Kitty graphics query (one pixel, `a=q`, wrapped for tmux),
/// the Kitty keyboard query, DECRQM for SGR pixel mouse (1016), the cell size
/// (`CSI 16 t`), a truecolor check (set a 24-bit colour, ask for it back with
/// DECRQSS, reset), and primary device attributes last, because every
/// terminal answers it and its answer ends the wait.
pub fn probe_query(environment: &Environment) -> String {
    let graphics = format!("\x1b_Gi={KITTY_QUERY_ID},s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\");
    let graphics = if environment.tmux { tmux_passthrough(&graphics) } else { graphics };
    format!(
        "{graphics}\
         \x1b[?u\
         \x1b[?1016$p\
         \x1b[16t\
         \x1b[38;2;3;141;59m\x1bP$qm\x1b\\\x1b[0m\
         \x1b[c"
    )
}

pub fn tmux_passthrough(sequence: &str) -> String {
    format!("\x1bPtmux;{}\x1b\\", sequence.replace('\x1b', "\x1b\x1b"))
}

/// The terminal's answers, collected as they arrive.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Replies {
    buffer: Vec<u8>,
    /// `Some(true)` for an OK to the graphics query, `Some(false)` for an
    /// error reply.
    pub kitty_graphics: Option<bool>,
    /// The flags the Kitty keyboard query reported.
    pub kitty_keyboard: Option<u32>,
    /// The DECRQM answer for mode 1016: recognised or not.
    pub sgr_pixels: Option<bool>,
    pub cell_pixels: Option<(u32, u32)>,
    /// The terminal echoed the 24-bit colour back through DECRQSS.
    pub truecolor: bool,
    /// Primary device attributes parameters (`CSI ? 64 ; 4 ; 22 c` gives
    /// `[64, 4, 22]`). Its arrival completes the probe.
    pub device_attributes: Option<Vec<u32>>,
}

impl Replies {
    /// Takes the next bytes read from the terminal. Replies may arrive split
    /// anywhere; a partial one is kept until the rest comes.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
        let mut at = 0;
        let buffer = std::mem::take(&mut self.buffer);
        while at < buffer.len() {
            if buffer[at] != 0x1b {
                at += 1;
                continue;
            }
            match sequence_at(&buffer[at..]) {
                Sequence::Incomplete => break,
                Sequence::Other(length) => at += length,
                Sequence::String { kind, body, length } => {
                    self.read_string(kind, body);
                    at += length;
                }
                Sequence::Csi { body, length } => {
                    self.read_csi(body);
                    at += length;
                }
            }
        }
        self.buffer = buffer[at..].to_vec();
    }

    fn read_string(&mut self, kind: u8, body: &[u8]) {
        let text = String::from_utf8_lossy(body);
        match kind {
            b'_' => {
                // Kitty graphics: G<keys>;<message>
                let Some(rest) = text.strip_prefix('G') else { return };
                let Some((keys, message)) = rest.split_once(';') else { return };
                if keys.split(',').any(|key| key == format!("i={KITTY_QUERY_ID}")) {
                    self.kitty_graphics = Some(message.trim() == "OK");
                }
            }
            b'P' => {
                // DECRQSS: 1$r<SGR>m is valid, 0$r is not.
                if let Some(sgr) = text.strip_prefix("1$r") {
                    if sgr.contains("3:141:59") || sgr.contains("3;141;59") {
                        self.truecolor = true;
                    }
                }
            }
            _ => {}
        }
    }

    fn read_csi(&mut self, body: &[u8]) {
        let text = String::from_utf8_lossy(body);
        let numbers = |text: &str| -> Vec<u32> {
            text.split(';').filter_map(|part| part.parse().ok()).collect()
        };
        if let Some(rest) = text.strip_prefix('?') {
            if let Some(flags) = rest.strip_suffix('u') {
                self.kitty_keyboard = flags.parse().ok();
            } else if let Some(mode) = rest.strip_suffix("$y") {
                let values = numbers(mode);
                if values.first() == Some(&1016) {
                    self.sgr_pixels = Some(matches!(values.get(1), Some(1..=4)));
                }
            } else if let Some(attributes) = rest.strip_suffix('c') {
                self.device_attributes = Some(numbers(attributes));
            }
        } else if let Some(size) = text.strip_suffix('t') {
            let values = numbers(size);
            if values.len() == 3 && values[0] == 6 && values[1] > 0 && values[2] > 0 {
                self.cell_pixels = Some((values[2], values[1]));
            }
        }
    }

    /// Every query has been answered or passed over: the device attributes
    /// reply, which comes last, has arrived.
    pub fn complete(&self) -> bool {
        self.device_attributes.is_some()
    }

    /// Sixel, by the attribute every Sixel terminal reports in DA1.
    pub fn sixel(&self) -> bool {
        self.device_attributes.as_ref().map_or(false, |attributes| attributes.contains(&4))
    }
}

/// Chooses the output, colours, cell glyphs and input modes.
pub fn decide(environment: &Environment, replies: &Replies) -> Decision {
    let multiplexer = if environment.tmux {
        Multiplexer::Tmux
    } else if environment.screen {
        Multiplexer::Screen
    } else {
        Multiplexer::None
    };
    let local_console = environment.linux_console && !environment.ssh;
    let framebuffer = local_console && environment.framebuffer_writable;

    let detected = if framebuffer {
        Output::Framebuffer
    } else if multiplexer == Multiplexer::Tmux && outer_terminal_has_placeholders(environment) {
        Output::Kitty { placeholders: true }
    } else if multiplexer == Multiplexer::None && replies.kitty_graphics == Some(true) {
        Output::Kitty { placeholders: false }
    } else if replies.sixel() {
        Output::Sixel
    } else {
        Output::Cells
    };
    let output = match environment.overrides.graphics {
        // Configuration cannot grant a permission: drawing nowhere is worse
        // than drawing cells.
        Some(Output::Framebuffer) if !framebuffer => Output::Cells,
        Some(Output::Kitty { .. }) => Output::Kitty { placeholders: multiplexer == Multiplexer::Tmux },
        Some(configured) => configured,
        None => detected,
    };

    Decision {
        output,
        color: environment.overrides.color.unwrap_or_else(|| color_depth(environment, replies)),
        cells: environment.overrides.cells.unwrap_or_else(|| cell_shape(environment)),
        multiplexer,
        keyboard: if replies.kitty_keyboard.is_some() && multiplexer == Multiplexer::None {
            Keyboard::Kitty
        } else {
            Keyboard::Legacy
        },
        mouse_pixels: replies.sgr_pixels == Some(true),
        cell_pixels: replies.cell_pixels,
    }
}

/// Kitty and Ghostty implement Unicode placeholders; through tmux nothing
/// else draws Kitty images.
fn outer_terminal_has_placeholders(environment: &Environment) -> bool {
    let outer = environment.tmux_client_terminal.to_ascii_lowercase();
    outer.contains("kitty") || outer.contains("ghostty")
}

fn color_depth(environment: &Environment, replies: &Replies) -> ColorDepth {
    let term = environment.term.to_ascii_lowercase();
    let program = environment.term_program.to_ascii_lowercase();
    let colorterm = environment.colorterm.to_ascii_lowercase();
    if environment.legacy_windows_console {
        return ColorDepth::Ansi16 { bright_backgrounds: true };
    }
    if term == "linux" {
        // The Linux VT: sixteen foregrounds, eight backgrounds.
        return ColorDepth::Ansi16 { bright_backgrounds: false };
    }
    if replies.truecolor
        || colorterm == "truecolor"
        || colorterm == "24bit"
        || term.ends_with("-direct")
        || environment.windows_terminal
        || environment.vte_version.map_or(false, |version| version >= 3600)
        || matches!(program.as_str(), "iterm.app" | "wezterm" | "ghostty" | "vscode")
        || term.contains("kitty")
        || term.contains("ghostty")
    {
        return ColorDepth::TrueColor;
    }
    if term.contains("256color") || program == "apple_terminal" {
        return ColorDepth::Ansi256;
    }
    ColorDepth::Ansi16 { bright_backgrounds: true }
}

/// The finest cell glyphs the terminal is known to draw itself. Terminals
/// that draw block elements with their own code rather than the font show
/// sextants and octants whatever the font is; elsewhere it is half-blocks,
/// which every font has, unless configured otherwise.
fn cell_shape(environment: &Environment) -> CellShape {
    let term = environment.term.to_ascii_lowercase();
    let program = environment.term_program.to_ascii_lowercase();
    if term.contains("kitty") || term.contains("ghostty") || program == "ghostty" {
        return CellShape::Octant;
    }
    if term.contains("alacritty")
        || term.starts_with("foot")
        || program == "wezterm"
        || environment.windows_terminal
        || environment.vte_version.map_or(false, |version| version >= 6400)
    {
        return CellShape::Sextant;
    }
    CellShape::HalfBlock
}

enum Sequence<'a> {
    /// The bytes so far are the start of a sequence.
    Incomplete,
    /// Something this does not read, this many bytes long.
    Other(usize),
    /// APC (`_`) or DCS (`P`), terminated by ST.
    String { kind: u8, body: &'a [u8], length: usize },
    Csi { body: &'a [u8], length: usize },
}

/// The escape sequence starting at `bytes[0] == ESC`.
fn sequence_at(bytes: &[u8]) -> Sequence<'_> {
    let Some(&kind) = bytes.get(1) else { return Sequence::Incomplete };
    match kind {
        b'_' | b'P' => {
            let body = &bytes[2..];
            match body.windows(2).position(|pair| pair == b"\x1b\\") {
                Some(end) => Sequence::String { kind, body: &body[..end], length: 2 + end + 2 },
                None => Sequence::Incomplete,
            }
        }
        b'[' => {
            let body = &bytes[2..];
            match body.iter().position(|&byte| (0x40..=0x7e).contains(&byte)) {
                Some(end) => Sequence::Csi { body: &body[..=end], length: 2 + end + 1 },
                None => Sequence::Incomplete,
            }
        }
        _ => Sequence::Other(2),
    }
}

#[cfg(test)]
mod tests;
