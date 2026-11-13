use super::*;

const KITTY_REPLIES: &[u8] = b"\x1b_Gi=31;OK\x1b\\\x1b[?15u\x1b[?1016;2$y\x1b[6;18;9t\x1bP1$r0;38:2:3:141:59m\x1b\\\x1b[?62;52c";
const XTERM_SIXEL_REPLIES: &[u8] = b"\x1b[?1016;2$y\x1b[6;17;8t\x1bP1$r0;38:2::3:141:59m\x1b\\\x1b[?63;1;2;4;6;9;15;16;22;28c";
const WINDOWS_TERMINAL_REPLIES: &[u8] = b"\x1b[?61;4;6;7;14;21;22;23;24;28;32;42c";
const APPLE_TERMINAL_REPLIES: &[u8] = b"\x1b[?1;2c";
const LINUX_CONSOLE_REPLIES: &[u8] = b"\x1b[?6c";

fn replies(bytes: &[u8]) -> Replies {
    let mut replies = Replies::default();
    replies.feed(bytes);
    replies
}

fn env(term: &str) -> Environment {
    Environment { term: term.into(), ..Environment::default() }
}

#[test]
fn the_probe_asks_everything_and_ends_with_device_attributes() {
    assert_eq!(
        probe_query(&env("xterm-256color")),
        "\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\
         \x1b[?u\
         \x1b[?1016$p\
         \x1b[16t\
         \x1b[38;2;3;141;59m\x1bP$qm\x1b\\\x1b[0m\
         \x1b[c"
    );
}

#[test]
fn inside_tmux_the_graphics_query_goes_through_passthrough() {
    let query = probe_query(&Environment { tmux: true, ..env("tmux-256color") });
    assert!(query.starts_with("\x1bPtmux;\x1b\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\x1b\\\x1b\\"), "{query:?}");
    assert!(query.ends_with("\x1b[c"));
}

#[test]
fn tmux_passthrough_doubles_every_escape() {
    assert_eq!(tmux_passthrough("\x1b_Ga=d\x1b\\"), "\x1bPtmux;\x1b\x1b_Ga=d\x1b\x1b\\\x1b\\");
}

#[test]
fn kitty_replies_are_read() {
    let replies = replies(KITTY_REPLIES);
    assert_eq!(replies.kitty_graphics, Some(true));
    assert_eq!(replies.kitty_keyboard, Some(15));
    assert_eq!(replies.sgr_pixels, Some(true));
    assert_eq!(replies.cell_pixels, Some((9, 18)));
    assert!(replies.truecolor);
    assert_eq!(replies.device_attributes, Some(vec![62, 52]));
    assert!(replies.complete());
    assert!(!replies.sixel());
}

#[test]
fn replies_split_at_any_byte_read_the_same() {
    let whole = replies(KITTY_REPLIES);
    for split in 1..KITTY_REPLIES.len() {
        let mut parts = Replies::default();
        parts.feed(&KITTY_REPLIES[..split]);
        parts.feed(&KITTY_REPLIES[split..]);
        assert_eq!(parts.kitty_graphics, whole.kitty_graphics, "split at {split}");
        assert_eq!(parts.kitty_keyboard, whole.kitty_keyboard, "split at {split}");
        assert_eq!(parts.sgr_pixels, whole.sgr_pixels, "split at {split}");
        assert_eq!(parts.cell_pixels, whole.cell_pixels, "split at {split}");
        assert_eq!(parts.truecolor, whole.truecolor, "split at {split}");
        assert_eq!(parts.device_attributes, whole.device_attributes, "split at {split}");
    }
}

#[test]
fn a_partial_reply_is_not_complete() {
    let mut partial = Replies::default();
    partial.feed(b"\x1b_Gi=31;OK\x1b\\\x1b[?62;");
    assert!(!partial.complete());
    partial.feed(b"c");
    assert!(partial.complete());
}

#[test]
fn a_graphics_error_or_another_id_is_not_kitty() {
    assert_eq!(replies(b"\x1b_Gi=31;ENOTSUPPORTED:no\x1b\\\x1b[?62c").kitty_graphics, Some(false));
    assert_eq!(replies(b"\x1b_Gi=7;OK\x1b\\\x1b[?62c").kitty_graphics, None);
}

#[test]
fn a_decrqss_error_is_not_truecolor() {
    assert!(!replies(b"\x1bP0$r\x1b\\\x1b[?62c").truecolor);
}

#[test]
fn decrqm_not_recognised_is_no_pixel_mouse() {
    assert_eq!(replies(b"\x1b[?1016;0$y\x1b[?62c").sgr_pixels, Some(false));
}

#[test]
fn kitty_draws_pixels_with_kitty_keyboard_and_pixel_mouse() {
    let decision = decide(&env("xterm-kitty"), &replies(KITTY_REPLIES));
    assert_eq!(decision.output, Output::Kitty { placeholders: false });
    assert_eq!(decision.keyboard, Keyboard::Kitty);
    assert!(decision.mouse_pixels);
    assert_eq!(decision.color, ColorDepth::TrueColor);
    assert_eq!(decision.cell_pixels, Some((9, 18)));
    assert_eq!(decision.multiplexer, Multiplexer::None);
}

#[test]
fn xterm_with_sixel_in_its_attributes_draws_sixel() {
    let decision = decide(&env("xterm-256color"), &replies(XTERM_SIXEL_REPLIES));
    assert_eq!(decision.output, Output::Sixel);
    assert_eq!(decision.keyboard, Keyboard::Legacy);
    assert_eq!(decision.color, ColorDepth::TrueColor);
}

#[test]
fn windows_terminal_draws_sixel_in_truecolor() {
    let environment = Environment { windows_terminal: true, ..env("") };
    let decision = decide(&environment, &replies(WINDOWS_TERMINAL_REPLIES));
    assert_eq!(decision.output, Output::Sixel);
    assert_eq!(decision.color, ColorDepth::TrueColor);
}

#[test]
fn vte_with_sixel_draws_sixel_and_without_it_draws_sextants() {
    let vte = Environment { vte_version: Some(7600), colorterm: "truecolor".into(), ..env("xterm-256color") };
    assert_eq!(decide(&vte, &replies(b"\x1b[?65;1;4;9c")).output, Output::Sixel);
    let without = decide(&vte, &replies(b"\x1b[?65;1;9c"));
    assert_eq!(without.output, Output::Cells);
    assert_eq!(without.cells, CellShape::Sextant);
    assert_eq!(without.color, ColorDepth::TrueColor);
}

#[test]
fn apple_terminal_keeps_rendering_in_cells_at_256_colours() {
    let environment = Environment { term_program: "Apple_Terminal".into(), ..env("xterm-256color") };
    let decision = decide(&environment, &replies(APPLE_TERMINAL_REPLIES));
    assert_eq!(decision.output, Output::Cells);
    assert_eq!(decision.color, ColorDepth::Ansi256);
    assert_eq!(decision.cells, CellShape::HalfBlock);
}

#[test]
fn a_local_linux_console_with_framebuffer_access_draws_to_the_framebuffer() {
    let environment = Environment { linux_console: true, framebuffer_writable: true, ..env("linux") };
    assert_eq!(decide(&environment, &replies(LINUX_CONSOLE_REPLIES)).output, Output::Framebuffer);
}

#[test]
fn a_linux_console_without_access_or_over_ssh_draws_eight_background_cells() {
    for environment in [
        Environment { linux_console: true, framebuffer_writable: false, ..env("linux") },
        Environment { linux_console: false, ssh: true, framebuffer_writable: true, ..env("linux") },
    ] {
        let decision = decide(&environment, &replies(LINUX_CONSOLE_REPLIES));
        assert_eq!(decision.output, Output::Cells, "{environment:?}");
        assert_eq!(decision.color, ColorDepth::Ansi16 { bright_backgrounds: false });
        assert_eq!(decision.cells, CellShape::HalfBlock);
    }
}

#[test]
fn tmux_with_kitty_outside_uses_placeholders() {
    let environment = Environment {
        tmux: true,
        tmux_client_terminal: "xterm-kitty".into(),
        ..env("tmux-256color")
    };
    let decision = decide(&environment, &replies(b"\x1b[?1;2c"));
    assert_eq!(decision.output, Output::Kitty { placeholders: true });
    assert_eq!(decision.multiplexer, Multiplexer::Tmux);
}

#[test]
fn tmux_with_ghostty_outside_uses_placeholders_too() {
    let environment = Environment { tmux: true, tmux_client_terminal: "xterm-ghostty".into(), ..env("tmux-256color") };
    assert_eq!(decide(&environment, &replies(b"\x1b[?1;2c")).output, Output::Kitty { placeholders: true });
}

#[test]
fn tmux_reporting_sixel_draws_sixel_and_plain_tmux_draws_cells() {
    let environment = Environment { tmux: true, tmux_client_terminal: "foot".into(), ..env("tmux-256color") };
    let sixel = decide(&environment, &replies(b"\x1b[?1;2;4c"));
    assert_eq!(sixel.output, Output::Sixel);
    assert_eq!(sixel.multiplexer, Multiplexer::Tmux);
    let plain = Environment { tmux: true, ..env("tmux-256color") };
    assert_eq!(decide(&plain, &replies(b"\x1b[?1;2c")).output, Output::Cells);
}

#[test]
fn screen_is_reported_and_never_gets_kitty() {
    let environment = Environment { screen: true, ..env("screen-256color") };
    let decision = decide(&environment, &replies(KITTY_REPLIES));
    assert_eq!(decision.multiplexer, Multiplexer::Screen);
    assert_ne!(decision.output, Output::Kitty { placeholders: false });
    assert_ne!(decision.output, Output::Kitty { placeholders: true });
}

#[test]
fn a_legacy_windows_console_draws_sixteen_colour_half_blocks() {
    let environment = Environment { legacy_windows_console: true, ..env("") };
    let decision = decide(&environment, &Replies::default());
    assert_eq!(decision.output, Output::Cells);
    assert_eq!(decision.color, ColorDepth::Ansi16 { bright_backgrounds: true });
    assert_eq!(decision.cells, CellShape::HalfBlock);
}

#[test]
fn a_terminal_that_never_answers_still_gets_cells() {
    let decision = decide(&env("xterm-256color"), &Replies::default());
    assert_eq!(decision.output, Output::Cells);
    assert_eq!(decision.color, ColorDepth::Ansi256);
    assert_eq!(decision.keyboard, Keyboard::Legacy);
    assert!(!decision.mouse_pixels);
}

#[test]
fn colour_depth_comes_from_the_terminal_then_the_environment() {
    let none = Replies::default();
    assert_eq!(decide(&Environment { colorterm: "truecolor".into(), ..env("xterm") }, &none).color, ColorDepth::TrueColor);
    assert_eq!(decide(&Environment { colorterm: "24bit".into(), ..env("xterm") }, &none).color, ColorDepth::TrueColor);
    assert_eq!(decide(&env("xterm-direct"), &none).color, ColorDepth::TrueColor);
    assert_eq!(decide(&env("xterm-256color"), &none).color, ColorDepth::Ansi256);
    assert_eq!(decide(&env("xterm"), &none).color, ColorDepth::Ansi16 { bright_backgrounds: true });
    // The DECRQSS echo beats a TERM that undersells the terminal.
    assert_eq!(decide(&env("xterm-256color"), &replies(XTERM_SIXEL_REPLIES)).color, ColorDepth::TrueColor);
}

#[test]
fn cell_glyphs_follow_what_the_terminal_draws() {
    let none = Replies::default();
    let cells_of = |environment: Environment| decide(&environment, &none).cells;
    assert_eq!(cells_of(Environment { term_program: "ghostty".into(), ..env("xterm-ghostty") }), CellShape::Octant);
    assert_eq!(cells_of(env("xterm-kitty")), CellShape::Octant);
    assert_eq!(cells_of(env("alacritty")), CellShape::Sextant);
    assert_eq!(cells_of(env("foot")), CellShape::Sextant);
    assert_eq!(cells_of(Environment { term_program: "WezTerm".into(), ..env("xterm-256color") }), CellShape::Sextant);
    assert_eq!(cells_of(Environment { windows_terminal: true, ..env("") }), CellShape::Sextant);
    assert_eq!(cells_of(Environment { vte_version: Some(6400), ..env("xterm-256color") }), CellShape::Sextant);
    assert_eq!(cells_of(Environment { vte_version: Some(6000), ..env("xterm-256color") }), CellShape::HalfBlock);
    assert_eq!(cells_of(env("xterm-256color")), CellShape::HalfBlock);
}

#[test]
fn configuration_wins_over_detection() {
    let overrides = Overrides {
        graphics: Some(Output::Cells),
        color: Some(ColorDepth::Ansi256),
        cells: Some(CellShape::Octant),
    };
    let decision = decide(&Environment { overrides, ..env("xterm-kitty") }, &replies(KITTY_REPLIES));
    assert_eq!(decision.output, Output::Cells);
    assert_eq!(decision.color, ColorDepth::Ansi256);
    assert_eq!(decision.cells, CellShape::Octant);
}

#[test]
fn configured_graphics_are_used_even_when_the_terminal_did_not_answer() {
    let overrides = Overrides { graphics: Some(Output::Sixel), ..Overrides::default() };
    assert_eq!(decide(&Environment { overrides, ..env("xterm") }, &Replies::default()).output, Output::Sixel);
}

#[test]
fn a_configured_framebuffer_without_access_falls_back_to_cells() {
    // Configuration cannot grant a permission; drawing nowhere is worse than cells.
    let overrides = Overrides { graphics: Some(Output::Framebuffer), ..Overrides::default() };
    let environment = Environment { overrides, framebuffer_writable: false, ..env("linux") };
    assert_eq!(decide(&environment, &Replies::default()).output, Output::Cells);
}
