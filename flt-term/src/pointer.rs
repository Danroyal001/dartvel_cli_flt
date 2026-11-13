//! Terminal mouse reports to Flutter pointer events.
//!
//! A terminal reports presses, releases, motion and wheel steps in cells (or
//! pixels with mode 1016). Flutter wants a mouse that is added before it
//! hovers, goes down when the first button does, moves while any is held,
//! and comes up when the last is released, with the full button mask on
//! every event and positions in physical pixels.

use crate::input::{Button, MouseEvent, MouseKind};

/// Flutter's pointer phases, as far as a mouse goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Add,
    Hover,
    Down,
    Move,
    Up,
}

/// One event for the engine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PointerAction {
    Pointer { phase: Phase, x: f64, y: f64, buttons: i64 },
    Scroll { x: f64, y: f64, dx: f64, dy: f64 },
}

/// Where reports land in the frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerGeometry {
    /// Frame pixels per reported unit: the cell size, or 1 with pixel
    /// reports.
    pub unit_width: f64,
    pub unit_height: f64,
    /// Reports are cells (aim at the centre of the cell) rather than pixels.
    pub cells: bool,
    /// Physical pixels one wheel step scrolls.
    pub scroll_step: f64,
}

#[derive(Debug, Default)]
pub struct PointerState {
    added: bool,
    buttons: i64,
}

fn bit(button: Button) -> i64 {
    match button {
        Button::Left => 1,
        Button::Right => 2,
        Button::Middle => 4,
        Button::Back => 8,
        Button::Forward => 16,
    }
}

impl PointerState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn translate(&mut self, event: &MouseEvent, geometry: &PointerGeometry) -> Vec<PointerAction> {
        let centre = if geometry.cells { 0.5 } else { 0.0 };
        let x = (event.x as f64 + centre) * geometry.unit_width;
        let y = (event.y as f64 + centre) * geometry.unit_height;
        let mut actions = Vec::new();
        if !self.added {
            self.added = true;
            actions.push(PointerAction::Pointer { phase: Phase::Add, x, y, buttons: 0 });
        }
        match event.kind {
            MouseKind::Scroll { right, down } => actions.push(PointerAction::Scroll {
                x,
                y,
                dx: right as f64 * geometry.scroll_step,
                dy: down as f64 * geometry.scroll_step,
            }),
            MouseKind::Press(button) => {
                let was_up = self.buttons == 0;
                self.buttons |= bit(button);
                let phase = if was_up { Phase::Down } else { Phase::Move };
                actions.push(PointerAction::Pointer { phase, x, y, buttons: self.buttons });
            }
            MouseKind::Release(button) => {
                if self.buttons & bit(button) != 0 {
                    self.buttons &= !bit(button);
                    let phase = if self.buttons == 0 { Phase::Up } else { Phase::Move };
                    actions.push(PointerAction::Pointer { phase, x, y, buttons: self.buttons });
                }
            }
            MouseKind::Move { .. } => {
                let phase = if self.buttons == 0 { Phase::Hover } else { Phase::Move };
                actions.push(PointerAction::Pointer { phase, x, y, buttons: self.buttons });
            }
        }
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Modifiers;

    const CELLS: PointerGeometry = PointerGeometry { unit_width: 10.0, unit_height: 20.0, cells: true, scroll_step: 60.0 };
    const PIXELS: PointerGeometry = PointerGeometry { unit_width: 1.0, unit_height: 1.0, cells: false, scroll_step: 60.0 };

    fn event(x: u32, y: u32, kind: MouseKind) -> MouseEvent {
        MouseEvent { x, y, kind, modifiers: Modifiers::NONE }
    }

    fn pointer(phase: Phase, x: f64, y: f64, buttons: i64) -> PointerAction {
        PointerAction::Pointer { phase, x, y, buttons }
    }

    #[test]
    fn the_first_report_adds_the_mouse_then_hovers_at_the_cell_centre() {
        let mut state = PointerState::new();
        assert_eq!(
            state.translate(&event(2, 1, MouseKind::Move { held: None }), &CELLS),
            vec![pointer(Phase::Add, 25.0, 30.0, 0), pointer(Phase::Hover, 25.0, 30.0, 0)]
        );
        assert_eq!(
            state.translate(&event(3, 1, MouseKind::Move { held: None }), &CELLS),
            vec![pointer(Phase::Hover, 35.0, 30.0, 0)]
        );
    }

    #[test]
    fn a_click_is_down_then_up_with_the_button_mask() {
        let mut state = PointerState::new();
        state.translate(&event(0, 0, MouseKind::Move { held: None }), &PIXELS);
        assert_eq!(state.translate(&event(5, 6, MouseKind::Press(Button::Left)), &PIXELS), vec![pointer(Phase::Down, 5.0, 6.0, 1)]);
        assert_eq!(state.translate(&event(7, 6, MouseKind::Move { held: Some(Button::Left) }), &PIXELS), vec![pointer(Phase::Move, 7.0, 6.0, 1)]);
        assert_eq!(state.translate(&event(7, 6, MouseKind::Release(Button::Left)), &PIXELS), vec![pointer(Phase::Up, 7.0, 6.0, 0)]);
    }

    #[test]
    fn a_second_button_while_one_is_held_is_a_move_and_the_last_release_is_up() {
        let mut state = PointerState::new();
        state.translate(&event(1, 1, MouseKind::Press(Button::Left)), &PIXELS);
        assert_eq!(state.translate(&event(1, 1, MouseKind::Press(Button::Right)), &PIXELS), vec![pointer(Phase::Move, 1.0, 1.0, 3)]);
        assert_eq!(state.translate(&event(1, 1, MouseKind::Release(Button::Left)), &PIXELS), vec![pointer(Phase::Move, 1.0, 1.0, 2)]);
        assert_eq!(state.translate(&event(1, 1, MouseKind::Release(Button::Right)), &PIXELS), vec![pointer(Phase::Up, 1.0, 1.0, 0)]);
    }

    #[test]
    fn a_press_as_the_first_report_still_adds_first() {
        let mut state = PointerState::new();
        assert_eq!(
            state.translate(&event(1, 1, MouseKind::Press(Button::Left)), &PIXELS),
            vec![pointer(Phase::Add, 1.0, 1.0, 0), pointer(Phase::Down, 1.0, 1.0, 1)]
        );
    }

    #[test]
    fn a_release_with_nothing_held_is_ignored() {
        // Some terminals report a release for a press that went to another window.
        let mut state = PointerState::new();
        state.translate(&event(1, 1, MouseKind::Move { held: None }), &PIXELS);
        assert!(state.translate(&event(1, 1, MouseKind::Release(Button::Left)), &PIXELS).is_empty());
    }

    #[test]
    fn wheel_steps_scroll_by_the_step_in_both_directions() {
        let mut state = PointerState::new();
        state.translate(&event(0, 0, MouseKind::Move { held: None }), &CELLS);
        assert_eq!(
            state.translate(&event(1, 1, MouseKind::Scroll { right: 0, down: 1 }), &CELLS),
            vec![PointerAction::Scroll { x: 15.0, y: 30.0, dx: 0.0, dy: 60.0 }]
        );
        assert_eq!(
            state.translate(&event(1, 1, MouseKind::Scroll { right: -1, down: 0 }), &CELLS),
            vec![PointerAction::Scroll { x: 15.0, y: 30.0, dx: -60.0, dy: 0.0 }]
        );
    }

    #[test]
    fn back_and_forward_buttons_have_their_own_bits() {
        let mut state = PointerState::new();
        state.translate(&event(0, 0, MouseKind::Move { held: None }), &PIXELS);
        assert_eq!(state.translate(&event(0, 0, MouseKind::Press(Button::Back)), &PIXELS), vec![pointer(Phase::Down, 0.0, 0.0, 8)]);
        assert_eq!(state.translate(&event(0, 0, MouseKind::Press(Button::Forward)), &PIXELS), vec![pointer(Phase::Move, 0.0, 0.0, 24)]);
    }
}
