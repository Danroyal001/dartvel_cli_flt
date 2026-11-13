//! The contract between the runner and every way of drawing a frame.
//!
//! The runner renders a frame off-screen, works out which parts changed, and
//! asks one [FrameEncoder] for the bytes that bring the terminal up to date.
//! Kitty graphics, Sixel, the cell renderers (half-block, sextant, octant)
//! and the Linux framebuffer each implement it.
//!
//! Contract, which every encoder keeps:
//!
//! - `damage` lists rectangles in frame pixels. They are aligned to the cell
//!   grid (`x` and `width` multiples of `cell_width`, `y` and `height`
//!   multiples of `cell_height`, except that the last column or row may be
//!   cut short by the frame edge), lie inside the frame and do not overlap.
//!   An encoder draws exactly those regions; what it draws elsewhere is a bug.
//! - An empty `damage` means nothing changed: return no bytes.
//! - The encoder positions the cursor itself with CUP (`ESC [ row ; col H`,
//!   1-based), counted from `origin_row`/`origin_column`. It does not clear
//!   the screen, change modes, hide the cursor, or wrap output in
//!   synchronized-update markers: the runner owns all of that.
//! - [FrameEncoder::reset] forgets everything remembered between frames (a
//!   palette, a previous cell grid, image ids). The runner calls it after a
//!   resize or a screen clear and then sends the whole frame as damage.
//! - Output for a multiplexer (tmux or screen passthrough) is the encoder's
//!   job: it is told which one through its own options, not through here.

/// The byte order of a frame's pixels.
///
/// Flutter's software renderer hands over BGRA on Linux and Windows and RGBA
/// on macOS. Alpha is ignored: an application's frame is opaque.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Rgba8,
    Bgra8,
}

/// One rendered frame, borrowed from the renderer.
#[derive(Debug, Clone, Copy)]
pub struct Frame<'a> {
    pub width: u32,
    pub height: u32,
    /// Bytes from the start of one row to the start of the next; at least
    /// `width * 4`.
    pub stride: usize,
    pub format: PixelFormat,
    pub data: &'a [u8],
}

impl<'a> Frame<'a> {
    /// A frame over `data`, rows packed (`stride == width * 4`).
    pub fn packed(width: u32, height: u32, format: PixelFormat, data: &'a [u8]) -> Self {
        let frame = Frame { width, height, stride: width as usize * 4, format, data };
        assert!(data.len() >= frame.stride * height as usize, "frame data too short");
        frame
    }

    /// The pixel at `(x, y)` as `[r, g, b]`.
    #[inline]
    pub fn rgb(&self, x: u32, y: u32) -> [u8; 3] {
        let at = y as usize * self.stride + x as usize * 4;
        let p = &self.data[at..at + 4];
        match self.format {
            PixelFormat::Rgba8 => [p[0], p[1], p[2]],
            PixelFormat::Bgra8 => [p[2], p[1], p[0]],
        }
    }

    /// The whole frame as one rectangle.
    pub fn bounds(&self) -> Rect {
        Rect { x: 0, y: 0, width: self.width, height: self.height }
    }
}

/// A rectangle in frame pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// How frame pixels map onto terminal cells.
///
/// In a pixel mode (Kitty, Sixel, framebuffer) `cell_width`/`cell_height`
/// are the terminal's cell size in pixels, e.g. 9x18. In a cell mode they are
/// the sub-cell grid the frame was rendered at: 1x2 for half-blocks, 2x3 for
/// sextants, 2x4 for octants, so one frame pixel is one sub-cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellMetrics {
    pub cell_width: u32,
    pub cell_height: u32,
    /// The terminal area the frame covers, in cells.
    pub columns: u16,
    pub rows: u16,
    /// The terminal cell (0-based) the frame's top-left pixel sits in.
    pub origin_column: u16,
    pub origin_row: u16,
}

impl CellMetrics {
    /// The 1-based CUP sequence for the cell holding frame pixel `(x, y)`.
    pub fn cursor_to(&self, x: u32, y: u32) -> String {
        let column = self.origin_column as u32 + x / self.cell_width + 1;
        let row = self.origin_row as u32 + y / self.cell_height + 1;
        format!("\x1b[{row};{column}H")
    }
}

/// How many colours a cell renderer may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorDepth {
    /// 24-bit SGR (`38;2;r;g;b`).
    TrueColor,
    /// The xterm 256-colour palette (`38;5;n`).
    Ansi256,
    /// The 16 ANSI colours. `bright_backgrounds` is false on terminals with
    /// only eight background colours, such as the Linux VT.
    Ansi16 { bright_backgrounds: bool },
}

/// The glyphs a cell renderer draws with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellShape {
    /// `▀` with foreground and background: 1x2 sub-cells.
    HalfBlock,
    /// Unicode 13 sextants (U+1FB00 block): 2x3 sub-cells.
    Sextant,
    /// Unicode 16 octants (U+1CD00 block): 2x4 sub-cells.
    Octant,
}

impl CellShape {
    /// Sub-cells per cell, `(across, down)`.
    pub fn grid(self) -> (u32, u32) {
        match self {
            CellShape::HalfBlock => (1, 2),
            CellShape::Sextant => (2, 3),
            CellShape::Octant => (2, 4),
        }
    }
}

/// A terminal multiplexer between the application and the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Multiplexer {
    None,
    Tmux,
    Screen,
}

/// Turns the changed parts of a frame into terminal bytes.
pub trait FrameEncoder {
    /// Appends to `out` the bytes that redraw `damage` from `frame`.
    fn encode(&mut self, frame: &Frame, damage: &[Rect], cells: &CellMetrics, out: &mut Vec<u8>);

    /// Forgets state kept between frames; the next call gets full damage.
    fn reset(&mut self);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgra_and_rgba_read_the_same_colour() {
        let bgra = [30u8, 20, 10, 255];
        let rgba = [10u8, 20, 30, 255];
        assert_eq!(Frame::packed(1, 1, PixelFormat::Bgra8, &bgra).rgb(0, 0), [10, 20, 30]);
        assert_eq!(Frame::packed(1, 1, PixelFormat::Rgba8, &rgba).rgb(0, 0), [10, 20, 30]);
    }

    #[test]
    fn the_cursor_goes_to_the_cell_holding_a_pixel_counted_from_the_origin() {
        let cells = CellMetrics {
            cell_width: 9,
            cell_height: 18,
            columns: 80,
            rows: 24,
            origin_column: 0,
            origin_row: 1,
        };
        assert_eq!(cells.cursor_to(0, 0), "\x1b[2;1H");
        assert_eq!(cells.cursor_to(18, 36), "\x1b[4;3H");
    }

    #[test]
    fn sub_cell_grids_match_their_glyphs() {
        assert_eq!(CellShape::HalfBlock.grid(), (1, 2));
        assert_eq!(CellShape::Sextant.grid(), (2, 3));
        assert_eq!(CellShape::Octant.grid(), (2, 4));
    }
}
