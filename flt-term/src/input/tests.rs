use super::*;

fn parse(bytes: &[u8]) -> Vec<InputEvent> {
    let mut parser = InputParser::new();
    let mut events = parser.feed(bytes);
    events.extend(parser.flush());
    events
}

fn press(key: Key, modifiers: Modifiers, text: Option<&str>) -> InputEvent {
    InputEvent::Key(KeyEvent { key, modifiers, kind: KeyKind::Press, text: text.map(String::from) })
}

fn kitty(key: Key, modifiers: Modifiers, kind: KeyKind, text: Option<&str>) -> InputEvent {
    InputEvent::Key(KeyEvent { key, modifiers, kind, text: text.map(String::from) })
}

fn mouse(x: u32, y: u32, kind: MouseKind, modifiers: Modifiers) -> InputEvent {
    InputEvent::Mouse(MouseEvent { x, y, kind, modifiers })
}

#[test]
fn plain_text_types_characters() {
    assert_eq!(
        parse(b"aZ1"),
        vec![
            press(Key::Char('a'), Modifiers::NONE, Some("a")),
            press(Key::Char('z'), Modifiers::SHIFT, Some("Z")),
            press(Key::Char('1'), Modifiers::NONE, Some("1")),
        ]
    );
}

#[test]
fn utf8_text_is_decoded_even_split_between_reads() {
    let mut parser = InputParser::new();
    let bytes = "é€".as_bytes();
    let mut events = parser.feed(&bytes[..1]);
    assert!(events.is_empty());
    events.extend(parser.feed(&bytes[1..4]));
    events.extend(parser.feed(&bytes[4..]));
    assert_eq!(
        events,
        vec![
            press(Key::Char('é'), Modifiers::NONE, Some("é")),
            press(Key::Char('€'), Modifiers::NONE, Some("€")),
        ]
    );
}

#[test]
fn control_bytes_are_keys() {
    assert_eq!(
        parse(b"\r\t\x7f\x08\x03\x00"),
        vec![
            press(Key::Enter, Modifiers::NONE, None),
            press(Key::Tab, Modifiers::NONE, None),
            press(Key::Backspace, Modifiers::NONE, None),
            press(Key::Backspace, Modifiers::NONE, None),
            press(Key::Char('c'), Modifiers::CONTROL, None),
            press(Key::Char(' '), Modifiers::CONTROL, None),
        ]
    );
}

#[test]
fn a_lone_escape_waits_for_the_pause() {
    let mut parser = InputParser::new();
    assert!(parser.feed(b"\x1b").is_empty());
    assert!(parser.has_pending());
    assert_eq!(parser.flush(), vec![press(Key::Escape, Modifiers::NONE, None)]);
    assert!(!parser.has_pending());
}

#[test]
fn escape_then_a_character_is_alt() {
    assert_eq!(parse(b"\x1bx"), vec![press(Key::Char('x'), Modifiers::ALT, None)]);
}

#[test]
fn legacy_cursor_and_editing_keys() {
    assert_eq!(
        parse(b"\x1b[A\x1b[B\x1b[C\x1b[D\x1b[H\x1b[F\x1bOA\x1b[2~\x1b[3~\x1b[5~\x1b[6~\x1b[1~\x1b[4~\x1b[Z"),
        vec![
            press(Key::Up, Modifiers::NONE, None),
            press(Key::Down, Modifiers::NONE, None),
            press(Key::Right, Modifiers::NONE, None),
            press(Key::Left, Modifiers::NONE, None),
            press(Key::Home, Modifiers::NONE, None),
            press(Key::End, Modifiers::NONE, None),
            press(Key::Up, Modifiers::NONE, None),
            press(Key::Insert, Modifiers::NONE, None),
            press(Key::Delete, Modifiers::NONE, None),
            press(Key::PageUp, Modifiers::NONE, None),
            press(Key::PageDown, Modifiers::NONE, None),
            press(Key::Home, Modifiers::NONE, None),
            press(Key::End, Modifiers::NONE, None),
            press(Key::Tab, Modifiers::SHIFT, None),
        ]
    );
}

#[test]
fn legacy_modified_keys_carry_their_modifiers() {
    assert_eq!(
        parse(b"\x1b[1;5C\x1b[3;2~"),
        vec![
            press(Key::Right, Modifiers::CONTROL, None),
            press(Key::Delete, Modifiers::SHIFT, None),
        ]
    );
}

#[test]
fn function_keys_both_ways() {
    assert_eq!(
        parse(b"\x1bOP\x1bOS\x1b[15~\x1b[24~"),
        vec![
            press(Key::Function(1), Modifiers::NONE, None),
            press(Key::Function(4), Modifiers::NONE, None),
            press(Key::Function(5), Modifiers::NONE, None),
            press(Key::Function(12), Modifiers::NONE, None),
        ]
    );
}

#[test]
fn kitty_keys_report_press_repeat_and_release_with_text() {
    // a pressed, repeated, released; shift+a typing "A"; ctrl+c.
    assert_eq!(
        parse(b"\x1b[97;;97u\x1b[97;1:2;97u\x1b[97;1:3u\x1b[97:65;2;65u\x1b[99;5u"),
        vec![
            kitty(Key::Char('a'), Modifiers::NONE, KeyKind::Press, Some("a")),
            kitty(Key::Char('a'), Modifiers::NONE, KeyKind::Repeat, Some("a")),
            kitty(Key::Char('a'), Modifiers::NONE, KeyKind::Release, None),
            kitty(Key::Char('a'), Modifiers::SHIFT, KeyKind::Press, Some("A")),
            kitty(Key::Char('c'), Modifiers::CONTROL, KeyKind::Press, None),
        ]
    );
}

#[test]
fn kitty_special_and_modifier_keys() {
    assert_eq!(
        parse(b"\x1b[13u\x1b[27u\x1b[127;1:3u\x1b[57441u\x1b[57441;2:3u\x1b[57448;5u\x1b[1;1:3A\x1b[3;1:2~"),
        vec![
            kitty(Key::Enter, Modifiers::NONE, KeyKind::Press, None),
            kitty(Key::Escape, Modifiers::NONE, KeyKind::Press, None),
            kitty(Key::Backspace, Modifiers::NONE, KeyKind::Release, None),
            kitty(Key::Shift(Side::Left), Modifiers::NONE, KeyKind::Press, None),
            kitty(Key::Shift(Side::Left), Modifiers::SHIFT, KeyKind::Release, None),
            kitty(Key::Control(Side::Right), Modifiers::CONTROL, KeyKind::Press, None),
            kitty(Key::Up, Modifiers::NONE, KeyKind::Release, None),
            kitty(Key::Delete, Modifiers::NONE, KeyKind::Repeat, None),
        ]
    );
}

#[test]
fn kitty_text_with_several_code_points() {
    assert_eq!(
        parse(b"\x1b[101;;101:769u"),
        vec![kitty(Key::Char('e'), Modifiers::NONE, KeyKind::Press, Some("e\u{301}"))]
    );
}

#[test]
fn sgr_mouse_buttons_motion_and_release() {
    assert_eq!(
        parse(b"\x1b[<0;10;5M\x1b[<32;11;5M\x1b[<0;11;5m\x1b[<35;12;6M\x1b[<2;1;1M\x1b[<1;3;3m"),
        vec![
            mouse(9, 4, MouseKind::Press(Button::Left), Modifiers::NONE),
            mouse(10, 4, MouseKind::Move { held: Some(Button::Left) }, Modifiers::NONE),
            mouse(10, 4, MouseKind::Release(Button::Left), Modifiers::NONE),
            mouse(11, 5, MouseKind::Move { held: None }, Modifiers::NONE),
            mouse(0, 0, MouseKind::Press(Button::Right), Modifiers::NONE),
            mouse(2, 2, MouseKind::Release(Button::Middle), Modifiers::NONE),
        ]
    );
}

#[test]
fn sgr_mouse_wheel_and_modifiers() {
    assert_eq!(
        parse(b"\x1b[<64;5;5M\x1b[<65;5;5M\x1b[<66;5;5M\x1b[<67;5;5M\x1b[<16;2;2M\x1b[<4;2;2M"),
        vec![
            mouse(4, 4, MouseKind::Scroll { right: 0, down: -1 }, Modifiers::NONE),
            mouse(4, 4, MouseKind::Scroll { right: 0, down: 1 }, Modifiers::NONE),
            mouse(4, 4, MouseKind::Scroll { right: -1, down: 0 }, Modifiers::NONE),
            mouse(4, 4, MouseKind::Scroll { right: 1, down: 0 }, Modifiers::NONE),
            mouse(1, 1, MouseKind::Press(Button::Left), Modifiers::CONTROL),
            mouse(1, 1, MouseKind::Press(Button::Left), Modifiers::SHIFT),
        ]
    );
}

#[test]
fn sgr_mouse_extra_buttons() {
    assert_eq!(
        parse(b"\x1b[<128;4;4M\x1b[<129;4;4m"),
        vec![
            mouse(3, 3, MouseKind::Press(Button::Back), Modifiers::NONE),
            mouse(3, 3, MouseKind::Release(Button::Forward), Modifiers::NONE),
        ]
    );
}

#[test]
fn pixel_coordinates_pass_through_large() {
    assert_eq!(
        parse(b"\x1b[<0;1203;611M"),
        vec![mouse(1202, 610, MouseKind::Press(Button::Left), Modifiers::NONE)]
    );
}

#[test]
fn a_mouse_report_split_anywhere_is_one_event() {
    let bytes = b"\x1b[<0;10;5M";
    for split in 1..bytes.len() {
        let mut parser = InputParser::new();
        let mut events = parser.feed(&bytes[..split]);
        events.extend(parser.feed(&bytes[split..]));
        assert_eq!(events, vec![mouse(9, 4, MouseKind::Press(Button::Left), Modifiers::NONE)], "split at {split}");
    }
}

#[test]
fn bracketed_paste_is_one_event_whatever_it_contains() {
    assert_eq!(
        parse(b"\x1b[200~hi \x1b[A there\r\x1b[201~x"),
        vec![
            InputEvent::Paste("hi \x1b[A there\r".into()),
            press(Key::Char('x'), Modifiers::NONE, Some("x")),
        ]
    );
}

#[test]
fn focus_reports() {
    assert_eq!(parse(b"\x1b[I\x1b[O"), vec![InputEvent::Focus(true), InputEvent::Focus(false)]);
}

#[test]
fn late_terminal_replies_are_not_typed() {
    // A probe answer arriving after the probe gave up must not become keys.
    assert_eq!(
        parse(b"\x1b[?62;4c\x1b_Gi=31;OK\x1b\\\x1bP1$r0m\x1b\\\x1b[6;18;9t\x1b[?1016;2$yq"),
        vec![press(Key::Char('q'), Modifiers::NONE, Some("q"))]
    );
}
