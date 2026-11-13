//! Terminal output and input for the Dartvel terminal runner.
//!
//! Nothing here depends on the Flutter engine. The runner (`flt`) hands a
//! rendered frame and the regions that changed to a [frame::FrameEncoder],
//! and writes the bytes it returns to the terminal.
pub mod frame;
pub mod damage;
pub mod pacing;
pub mod detect;
pub mod input;
pub mod kitty;
pub mod framebuffer;
pub mod session;
pub mod keymap;
pub mod text_editing;
