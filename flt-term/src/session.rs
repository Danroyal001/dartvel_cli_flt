//! Taking over the terminal and giving it back.
//!
//! The runner switches to the alternate screen, hides the cursor, turns on
//! mouse, focus and paste reporting and (where offered) the Kitty keyboard
//! protocol, and puts the terminal in raw mode. All of it is undone on a
//! normal exit, on Ctrl+C, on SIGTERM/SIGHUP, and on a crash -- a panic or a
//! fatal signal from the engine -- because a shell left with mouse reporting
//! on and echo off is the failure people remember.

use crate::detect::{Decision, Keyboard, Output};
use crate::frame::Multiplexer;

/// Kitty keyboard flags: disambiguate (1), report event types (2), report
/// all keys as escape codes (8), report associated text (16).
pub const KITTY_KEYBOARD_FLAGS: u32 = 1 | 2 | 8 | 16;

/// Begin and end of a synchronized update (mode 2026): the terminal shows a
/// frame whole instead of half-drawn. Terminals without it ignore both.
pub const BEGIN_FRAME: &str = "\x1b[?2026h";
pub const END_FRAME: &str = "\x1b[?2026l";

/// The bytes that take over the terminal for `decision`.
pub fn setup_sequence(decision: &Decision) -> String {
    if decision.output == Output::Framebuffer {
        return "\x1b[?25l\x1b[2J".into();
    }
    let mut setup = String::from("\x1b[?1049h\x1b[?25l\x1b[?7l\x1b[2J");
    setup.push_str("\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h");
    if decision.mouse_pixels {
        setup.push_str("\x1b[?1016h");
    }
    setup.push_str("\x1b[?1004h\x1b[?2004h");
    if decision.output == Output::Sixel {
        setup.push_str("\x1b[?8452h");
    }
    if decision.keyboard == Keyboard::Kitty {
        setup.push_str(&format!("\x1b[>{KITTY_KEYBOARD_FLAGS}u"));
    }
    setup
}

pub fn restore_sequence(decision: &Decision) -> String {
    if decision.output == Output::Framebuffer {
        return "\x1b[0m\x1b[2J\x1b[H\x1b[?25h".into();
    }
    let mut restore = String::from(END_FRAME);
    if let Output::Kitty { .. } = decision.output {
        let delete = "\x1b_Ga=d,d=A,q=2\x1b\\";
        if decision.multiplexer == Multiplexer::Tmux {
            restore.push_str(&crate::detect::tmux_passthrough(delete));
        } else {
            restore.push_str(delete);
        }
    }
    if decision.keyboard == Keyboard::Kitty {
        restore.push_str("\x1b[<u");
    }
    restore.push_str("\x1b[?2004l\x1b[?1004l");
    if decision.mouse_pixels {
        restore.push_str("\x1b[?1016l");
    }
    restore.push_str("\x1b[?1006l\x1b[?1003l\x1b[?1002l\x1b[?1000l");
    if decision.output == Output::Sixel {
        restore.push_str("\x1b[?8452l");
    }
    restore.push_str(&pointer_sequence("default"));
    restore.push_str("\x1b[0m\x1b[?7h\x1b[?25h\x1b[?1049l");
    restore
}

pub fn pointer_shape(flutter_kind: &str) -> Option<&'static str> {
    Some(match flutter_kind {
        "basic" => "default",
        "click" => "pointer",
        "text" | "verticalText" => "text",
        "forbidden" => "not-allowed",
        "wait" => "wait",
        "progress" => "progress",
        "help" => "help",
        "move" | "allScroll" => "move",
        "grab" => "grab",
        "grabbing" => "grabbing",
        "precise" | "cell" => "crosshair",
        "contextMenu" => "context-menu",
        "copy" => "copy",
        "alias" => "alias",
        "noDrop" => "no-drop",
        "zoomIn" => "zoom-in",
        "zoomOut" => "zoom-out",
        "resizeLeftRight" | "resizeColumn" => "ew-resize",
        "resizeUpDown" | "resizeRow" => "ns-resize",
        "resizeUpLeftDownRight" => "nwse-resize",
        "resizeUpRightDownLeft" => "nesw-resize",
        "resizeLeft" => "w-resize",
        "resizeRight" => "e-resize",
        "resizeUp" => "n-resize",
        "resizeDown" => "s-resize",
        "resizeUpLeft" => "nw-resize",
        "resizeUpRight" => "ne-resize",
        "resizeDownLeft" => "sw-resize",
        "resizeDownRight" => "se-resize",
        _ => return None,
    })
}

/// The OSC 22 sequence that sets the pointer to `shape`.
pub fn pointer_sequence(shape: &str) -> String {
    format!("\x1b]22;{shape}\x1b\\")
}

#[cfg(unix)]
pub mod tty;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{CellShape, ColorDepth};

    fn decision(output: Output, keyboard: Keyboard, mouse_pixels: bool) -> Decision {
        Decision {
            output,
            color: ColorDepth::TrueColor,
            cells: CellShape::HalfBlock,
            multiplexer: Multiplexer::None,
            keyboard,
            mouse_pixels,
            cell_pixels: None,
        }
    }

    #[test]
    fn kitty_setup_takes_everything_it_needs() {
        assert_eq!(
            setup_sequence(&decision(Output::Kitty { placeholders: false }, Keyboard::Kitty, true)),
            "\x1b[?1049h\x1b[?25l\x1b[?7l\x1b[2J\
             \x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h\x1b[?1016h\
             \x1b[?1004h\x1b[?2004h\
             \x1b[>27u"
        );
    }

    #[test]
    fn kitty_restore_undoes_it_in_reverse_and_deletes_the_images() {
        assert_eq!(
            restore_sequence(&decision(Output::Kitty { placeholders: false }, Keyboard::Kitty, true)),
            "\x1b[?2026l\x1b_Ga=d,d=A,q=2\x1b\\\
             \x1b[<u\
             \x1b[?2004l\x1b[?1004l\
             \x1b[?1016l\x1b[?1006l\x1b[?1003l\x1b[?1002l\x1b[?1000l\
             \x1b]22;default\x1b\\\
             \x1b[0m\x1b[?7h\x1b[?25h\x1b[?1049l"
        );
    }

    #[test]
    fn legacy_cell_terminals_get_no_kitty_keyboard_or_pixel_mouse() {
        let setup = setup_sequence(&decision(Output::Cells, Keyboard::Legacy, false));
        assert!(!setup.contains("\x1b[>"), "{setup:?}");
        assert!(!setup.contains("1016"), "{setup:?}");
        let restore = restore_sequence(&decision(Output::Cells, Keyboard::Legacy, false));
        assert!(!restore.contains("\x1b[<u"), "{restore:?}");
        assert!(!restore.contains("\x1b_G"), "{restore:?}");
        assert!(restore.ends_with("\x1b[0m\x1b[?7h\x1b[?25h\x1b[?1049l"));
    }

    #[test]
    fn sixel_keeps_the_cursor_beside_the_image_and_restores_it() {
        let setup = setup_sequence(&decision(Output::Sixel, Keyboard::Legacy, false));
        assert!(setup.contains("\x1b[?8452h"), "{setup:?}");
        assert!(restore_sequence(&decision(Output::Sixel, Keyboard::Legacy, false)).contains("\x1b[?8452l"));
    }

    #[test]
    fn inside_tmux_the_image_cleanup_goes_through_passthrough() {
        let mut inside = decision(Output::Kitty { placeholders: true }, Keyboard::Legacy, false);
        inside.multiplexer = Multiplexer::Tmux;
        assert!(restore_sequence(&inside).contains("\x1bPtmux;\x1b\x1b_Ga=d,d=A,q=2\x1b\x1b\\\x1b\\"));
    }

    #[test]
    fn the_framebuffer_takes_no_mouse_reporting() {
        // On a VT the mouse comes from /dev/input/mice, not escape codes.
        let setup = setup_sequence(&decision(Output::Framebuffer, Keyboard::Legacy, false));
        assert_eq!(setup, "\x1b[?25l\x1b[2J");
        assert_eq!(restore_sequence(&decision(Output::Framebuffer, Keyboard::Legacy, false)), "\x1b[0m\x1b[2J\x1b[H\x1b[?25h");
    }

    #[test]
    fn flutter_cursors_map_to_pointer_shapes() {
        assert_eq!(pointer_shape("click"), Some("pointer"));
        assert_eq!(pointer_shape("text"), Some("text"));
        assert_eq!(pointer_shape("basic"), Some("default"));
        assert_eq!(pointer_shape("forbidden"), Some("not-allowed"));
        assert_eq!(pointer_shape("resizeLeftRight"), Some("ew-resize"));
        assert_eq!(pointer_shape("grab"), Some("grab"));
        assert_eq!(pointer_shape("none"), None);
        assert_eq!(pointer_sequence("pointer"), "\x1b]22;pointer\x1b\\");
    }
}
