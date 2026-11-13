//! Which parts of a frame changed since the last one.
//!
//! Flutter's software renderer hands over the whole frame every time. The
//! terminal is the slow part, so the runner compares each frame with the one
//! before it, tile by tile, and only the tiles that differ are encoded and
//! written. A blinking cursor in a text field costs one tile, not a screen.

use crate::frame::{Frame, Rect};

/// Remembers the previous frame and reports the tiles that changed.
#[derive(Default)]
pub struct DamageTracker {
    previous: Vec<u8>,
    width: u32,
    height: u32,
    valid: bool,
}

impl DamageTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget the previous frame: the next [DamageTracker::diff] reports
    /// every tile. Called after a resize, a screen clear, or anything else
    /// that put pixels on the terminal the tracker did not see.
    pub fn invalidate(&mut self) {
        self.valid = false;
    }

    /// The tiles of `tile_width` x `tile_height` pixels (cut short at the
    /// frame edge) that differ from the previous frame, in row-major order.
    /// The frame is then remembered for the next call.
    pub fn diff(&mut self, frame: &Frame, tile_width: u32, tile_height: u32) -> Vec<Rect> {
        assert!(tile_width > 0 && tile_height > 0, "tiles must have an area");
        let row_bytes = frame.width as usize * 4;
        let same_size = self.valid && self.width == frame.width && self.height == frame.height;
        let mut damage = Vec::new();
        let mut tile_y = 0;
        while tile_y < frame.height {
            let height = tile_height.min(frame.height - tile_y);
            let mut tile_x = 0;
            while tile_x < frame.width {
                let width = tile_width.min(frame.width - tile_x);
                let changed = !same_size || {
                    let start = tile_x as usize * 4;
                    let end = start + width as usize * 4;
                    (tile_y..tile_y + height).any(|y| {
                        let current = y as usize * frame.stride;
                        let previous = y as usize * row_bytes;
                        frame.data[current + start..current + end]
                            != self.previous[previous + start..previous + end]
                    })
                };
                if changed {
                    damage.push(Rect { x: tile_x, y: tile_y, width, height });
                }
                tile_x += tile_width;
            }
            tile_y += tile_height;
        }
        // Remember the frame, rows packed, padding dropped.
        self.previous.resize(row_bytes * frame.height as usize, 0);
        for y in 0..frame.height as usize {
            let current = y * frame.stride;
            self.previous[y * row_bytes..(y + 1) * row_bytes]
                .copy_from_slice(&frame.data[current..current + row_bytes]);
        }
        self.width = frame.width;
        self.height = frame.height;
        self.valid = true;
        damage
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::PixelFormat;

    fn frame_of(width: u32, height: u32, fill: u8) -> Vec<u8> {
        vec![fill; (width * height * 4) as usize]
    }

    fn set(data: &mut [u8], width: u32, x: u32, y: u32, value: u8) {
        let at = ((y * width + x) * 4) as usize;
        data[at] = value;
    }

    #[test]
    fn the_first_frame_is_all_damage_cut_at_the_edges() {
        let data = frame_of(10, 5, 0);
        let mut tracker = DamageTracker::new();
        let damage = tracker.diff(&Frame::packed(10, 5, PixelFormat::Bgra8, &data), 4, 4);
        assert_eq!(
            damage,
            vec![
                Rect { x: 0, y: 0, width: 4, height: 4 },
                Rect { x: 4, y: 0, width: 4, height: 4 },
                Rect { x: 8, y: 0, width: 2, height: 4 },
                Rect { x: 0, y: 4, width: 4, height: 1 },
                Rect { x: 4, y: 4, width: 4, height: 1 },
                Rect { x: 8, y: 4, width: 2, height: 1 },
            ]
        );
    }

    #[test]
    fn an_unchanged_frame_has_no_damage() {
        let data = frame_of(8, 8, 7);
        let mut tracker = DamageTracker::new();
        tracker.diff(&Frame::packed(8, 8, PixelFormat::Bgra8, &data), 4, 4);
        assert!(tracker.diff(&Frame::packed(8, 8, PixelFormat::Bgra8, &data), 4, 4).is_empty());
    }

    #[test]
    fn one_changed_pixel_damages_only_its_tile() {
        let mut data = frame_of(8, 8, 7);
        let mut tracker = DamageTracker::new();
        tracker.diff(&Frame::packed(8, 8, PixelFormat::Bgra8, &data), 4, 4);
        set(&mut data, 8, 5, 6, 9);
        assert_eq!(
            tracker.diff(&Frame::packed(8, 8, PixelFormat::Bgra8, &data), 4, 4),
            vec![Rect { x: 4, y: 4, width: 4, height: 4 }]
        );
        // And it is remembered: the same frame again is clean.
        assert!(tracker.diff(&Frame::packed(8, 8, PixelFormat::Bgra8, &data), 4, 4).is_empty());
    }

    #[test]
    fn a_change_in_the_last_byte_of_a_tile_row_is_seen() {
        // The silent failure: comparing one byte short per row.
        let mut data = frame_of(8, 4, 0);
        let mut tracker = DamageTracker::new();
        tracker.diff(&Frame::packed(8, 4, PixelFormat::Bgra8, &data), 4, 4);
        let at = ((2 * 8 + 3) * 4 + 3) as usize; // alpha byte of pixel (3, 2)
        data[at] = 1;
        assert_eq!(
            tracker.diff(&Frame::packed(8, 4, PixelFormat::Bgra8, &data), 4, 4),
            vec![Rect { x: 0, y: 0, width: 4, height: 4 }]
        );
    }

    #[test]
    fn a_resize_or_an_invalidation_damages_everything() {
        let small = frame_of(4, 4, 0);
        let large = frame_of(8, 4, 0);
        let mut tracker = DamageTracker::new();
        tracker.diff(&Frame::packed(4, 4, PixelFormat::Bgra8, &small), 4, 4);
        assert_eq!(tracker.diff(&Frame::packed(8, 4, PixelFormat::Bgra8, &large), 4, 4).len(), 2);
        tracker.invalidate();
        assert_eq!(tracker.diff(&Frame::packed(8, 4, PixelFormat::Bgra8, &large), 4, 4).len(), 2);
    }

    #[test]
    fn a_padded_stride_is_respected() {
        // Rows of 2 pixels stored 3 pixels apart; the padding changes and must
        // not count as damage.
        let mut data = vec![0u8; 3 * 4 * 2];
        fn frame(data: &[u8]) -> Frame<'_> {
            Frame { width: 2, height: 2, stride: 12, format: PixelFormat::Bgra8, data }
        }
        let mut tracker = DamageTracker::new();
        tracker.diff(&frame(&data.clone()), 2, 2);
        data[8] = 99; // padding of row 0
        assert!(tracker.diff(&frame(&data.clone()), 2, 2).is_empty());
    }
}
