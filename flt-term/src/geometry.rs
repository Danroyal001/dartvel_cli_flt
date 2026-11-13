//! How big the frame is and how it maps onto the terminal.

use crate::detect::{Decision, Output};
use crate::frame::CellMetrics;
use crate::pointer::PointerGeometry;

/// Logical pixels one terminal row stands for in a pixel mode: the app
/// lays out as if each text row were this tall, so a 24-row terminal is a
/// 480-logical-pixel-tall window.
pub const DEFAULT_ROW_HEIGHT: f64 = 20.0;
/// The same for cell modes, where a row is 2-4 frame pixels: smaller, so
/// the app's layout still fits in a terminal-sized grid.
pub const DEFAULT_CELL_ROW_HEIGHT: f64 = 8.0;
/// A terminal that reports no pixel size gets the most common cell.
pub const FALLBACK_CELL: (u32, u32) = (8, 16);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    /// Physical frame size the engine renders at.
    pub frame_width: u32,
    pub frame_height: u32,
    pub cells: CellMetrics,
    /// Flutter's device pixel ratio.
    pub pixel_ratio: f64,
    pub pointer: PointerGeometry,
}

/// The terminal's size as the runner reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalSize {
    pub columns: u16,
    pub rows: u16,
    /// Window pixels (TIOCGWINSZ), 0 when the terminal does not say.
    pub width_pixels: u16,
    pub height_pixels: u16,
}

/// Works out the frame for `decision` in a terminal of `size`.
/// `framebuffer` is the framebuffer's size in framebuffer mode; `row_height`
/// overrides the logical height of a row.
pub fn geometry(decision: &Decision, size: TerminalSize, framebuffer: Option<(u32, u32)>, row_height: Option<f64>) -> Geometry {
    let columns = if size.columns == 0 { 80 } else { size.columns };
    let rows = if size.rows == 0 { 24 } else { size.rows };
    let cells_of = |cell_width: u32, cell_height: u32| CellMetrics {
        cell_width,
        cell_height,
        columns,
        rows,
        origin_column: 0,
        origin_row: 0,
    };
    // The cell size in pixels: the window's pixels over its cells when the
    // terminal fills them in, else what it answered, else the usual 8x16.
    let window_cell = (size.width_pixels > 0 && size.height_pixels > 0)
        .then(|| (size.width_pixels as u32 / columns as u32, size.height_pixels as u32 / rows as u32))
        .filter(|&(width, height)| width > 0 && height > 0);
    let cell_pixels = window_cell.or(decision.cell_pixels).unwrap_or(FALLBACK_CELL);

    match decision.output {
        Output::Framebuffer => {
            let (width, height) = framebuffer.unwrap_or((columns as u32 * 8, rows as u32 * 16));
            let ratio = row_height.map_or(1.0, |row| 16.0 / row);
            Geometry {
                frame_width: width,
                frame_height: height,
                cells: CellMetrics { columns: (width / 8) as u16, rows: (height / 16) as u16, ..cells_of(8, 16) },
                pixel_ratio: ratio,
                pointer: PointerGeometry { unit_width: 1.0, unit_height: 1.0, cells: false, scroll_step: 3.0 * DEFAULT_ROW_HEIGHT * ratio },
            }
        }
        Output::Cells => {
            let (across, down) = decision.cells.grid();
            let ratio = down as f64 / row_height.unwrap_or(DEFAULT_CELL_ROW_HEIGHT);
            let pointer = if decision.mouse_pixels {
                PointerGeometry {
                    unit_width: across as f64 / cell_pixels.0 as f64,
                    unit_height: down as f64 / cell_pixels.1 as f64,
                    cells: false,
                    scroll_step: 3.0 * down as f64,
                }
            } else {
                PointerGeometry { unit_width: across as f64, unit_height: down as f64, cells: true, scroll_step: 3.0 * down as f64 }
            };
            Geometry {
                frame_width: columns as u32 * across,
                frame_height: rows as u32 * down,
                cells: cells_of(across, down),
                pixel_ratio: ratio,
                pointer,
            }
        }
        Output::Kitty { .. } | Output::Sixel => {
            let (cell_width, cell_height) = cell_pixels;
            let pointer = if decision.mouse_pixels {
                PointerGeometry { unit_width: 1.0, unit_height: 1.0, cells: false, scroll_step: 3.0 * cell_height as f64 }
            } else {
                PointerGeometry { unit_width: cell_width as f64, unit_height: cell_height as f64, cells: true, scroll_step: 3.0 * cell_height as f64 }
            };
            Geometry {
                frame_width: columns as u32 * cell_width,
                frame_height: rows as u32 * cell_height,
                cells: cells_of(cell_width, cell_height),
                pixel_ratio: cell_height as f64 / row_height.unwrap_or(DEFAULT_ROW_HEIGHT),
                pointer,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::Keyboard;
    use crate::frame::{CellShape, ColorDepth, Multiplexer};

    fn decision(output: Output, cells: CellShape, mouse_pixels: bool, cell_pixels: Option<(u32, u32)>) -> Decision {
        Decision { output, color: ColorDepth::TrueColor, cells, multiplexer: Multiplexer::None, keyboard: Keyboard::Legacy, mouse_pixels, cell_pixels }
    }

    fn size(columns: u16, rows: u16, width: u16, height: u16) -> TerminalSize {
        TerminalSize { columns, rows, width_pixels: width, height_pixels: height }
    }

    #[test]
    fn pixel_modes_cover_every_cell_at_the_reported_cell_size() {
        let geometry = geometry(&decision(Output::Kitty { placeholders: false }, CellShape::HalfBlock, true, None), size(80, 24, 724, 440), None, None);
        // 724/80 = 9.05 -> 9; 440/24 = 18.3 -> 18: whole cells, padding dropped.
        assert_eq!((geometry.frame_width, geometry.frame_height), (720, 432));
        assert_eq!((geometry.cells.cell_width, geometry.cells.cell_height), (9, 18));
        assert_eq!((geometry.cells.columns, geometry.cells.rows), (80, 24));
        assert!((geometry.pixel_ratio - 0.9).abs() < 1e-9);
        assert_eq!(geometry.pointer, PointerGeometry { unit_width: 1.0, unit_height: 1.0, cells: false, scroll_step: 54.0 });
    }

    #[test]
    fn without_pixel_mouse_reports_land_in_cell_centres() {
        let geometry = geometry(&decision(Output::Sixel, CellShape::HalfBlock, false, None), size(80, 24, 640, 384), None, None);
        assert_eq!(geometry.pointer.unit_width, 8.0);
        assert_eq!(geometry.pointer.unit_height, 16.0);
        assert!(geometry.pointer.cells);
    }

    #[test]
    fn a_terminal_without_window_pixels_uses_the_reported_cell_then_the_fallback() {
        let reported = geometry(&decision(Output::Sixel, CellShape::HalfBlock, false, Some((10, 20))), size(80, 24, 0, 0), None, None);
        assert_eq!((reported.frame_width, reported.frame_height), (800, 480));
        let fallback = geometry(&decision(Output::Sixel, CellShape::HalfBlock, false, None), size(80, 24, 0, 0), None, None);
        assert_eq!((fallback.frame_width, fallback.frame_height), (640, 384));
    }

    #[test]
    fn cell_modes_render_one_pixel_per_sub_cell() {
        let half = geometry(&decision(Output::Cells, CellShape::HalfBlock, false, None), size(80, 24, 0, 0), None, None);
        assert_eq!((half.frame_width, half.frame_height), (80, 48));
        assert!((half.pixel_ratio - 0.25).abs() < 1e-9);
        assert_eq!(half.pointer, PointerGeometry { unit_width: 1.0, unit_height: 2.0, cells: true, scroll_step: 6.0 });
        let octant = geometry(&decision(Output::Cells, CellShape::Octant, false, None), size(200, 60, 0, 0), None, None);
        assert_eq!((octant.frame_width, octant.frame_height), (400, 240));
        assert_eq!((octant.cells.cell_width, octant.cells.cell_height), (2, 4));
    }

    #[test]
    fn the_framebuffer_renders_at_its_own_resolution() {
        let geometry = geometry(&decision(Output::Framebuffer, CellShape::HalfBlock, false, None), size(128, 48, 0, 0), Some((1024, 768)), None);
        assert_eq!((geometry.frame_width, geometry.frame_height), (1024, 768));
        assert_eq!(geometry.pixel_ratio, 1.0);
    }

    #[test]
    fn the_row_height_can_be_configured() {
        let geometry = geometry(&decision(Output::Kitty { placeholders: false }, CellShape::HalfBlock, true, None), size(80, 24, 720, 432), None, Some(18.0));
        assert!((geometry.pixel_ratio - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_zero_sized_terminal_still_gives_a_frame() {
        let geometry = geometry(&decision(Output::Cells, CellShape::HalfBlock, false, None), size(0, 0, 0, 0), None, None);
        assert_eq!((geometry.cells.columns, geometry.cells.rows), (80, 24));
    }
}
