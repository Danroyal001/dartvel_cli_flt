//! Drawing straight to the Linux framebuffer.
//!
//! On a local virtual console with no display server there is no terminal
//! emulator to speak an image protocol, but there is the framebuffer the
//! console itself draws on. Writing the frame there gives full pixels where
//! cells would give blocks. The runner opens `/dev/fb0` (or `$FRAMEBUFFER`),
//! switches the console to graphics mode so its text and cursor stay off the
//! picture, and switches it back on exit and on a crash.
//!
//! The pure part -- converting damaged regions of a frame into the
//! framebuffer's pixel layout -- is here and tested on plain memory. The
//! device part is in [device].

use crate::frame::{CellMetrics, Frame, FrameEncoder, Rect};

/// One colour channel's place in a framebuffer pixel (`fb_bitfield`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Channel {
    pub offset: u32,
    pub length: u32,
}

/// How the framebuffer stores pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub width: u32,
    pub height: u32,
    /// Bytes per line (`line_length`), at least `width * bytes_per_pixel`.
    pub line_length: usize,
    pub bits_per_pixel: u32,
    pub red: Channel,
    pub green: Channel,
    pub blue: Channel,
}

impl Layout {
    /// The common 32-bit XRGB8888 layout.
    pub fn xrgb8888(width: u32, height: u32) -> Layout {
        Layout {
            width,
            height,
            line_length: width as usize * 4,
            bits_per_pixel: 32,
            red: Channel { offset: 16, length: 8 },
            green: Channel { offset: 8, length: 8 },
            blue: Channel { offset: 0, length: 8 },
        }
    }

    /// RGB565, still found on small panels.
    pub fn rgb565(width: u32, height: u32) -> Layout {
        Layout {
            width,
            height,
            line_length: width as usize * 2,
            bits_per_pixel: 16,
            red: Channel { offset: 11, length: 5 },
            green: Channel { offset: 5, length: 6 },
            blue: Channel { offset: 0, length: 5 },
        }
    }

    /// Whether this layout can be written. 16, 24 and 32 bits per pixel,
    /// little-endian, with channels inside the pixel.
    pub fn supported(&self) -> bool {
        matches!(self.bits_per_pixel, 16 | 24 | 32)
            && [self.red, self.green, self.blue]
                .iter()
                .all(|channel| channel.length >= 1 && channel.length <= 8 && channel.offset + channel.length <= self.bits_per_pixel)
    }
}

/// Copies `rect` of `frame` into `memory` laid out as `layout`, with the
/// frame's top-left at framebuffer pixel `(left, top)`. Whatever falls
/// outside the framebuffer is dropped.
pub fn blit(frame: &Frame, rect: Rect, layout: &Layout, memory: &mut [u8], left: u32, top: u32) {
    let bytes = (layout.bits_per_pixel / 8) as usize;
    let pack = |value: u8, channel: Channel| -> u32 {
        ((value as u32) >> (8 - channel.length)) << channel.offset
    };
    for y in rect.y..(rect.y + rect.height).min(frame.height) {
        let screen_y = top + y;
        if screen_y >= layout.height {
            break;
        }
        let line = screen_y as usize * layout.line_length;
        for x in rect.x..(rect.x + rect.width).min(frame.width) {
            let screen_x = left + x;
            if screen_x >= layout.width {
                break;
            }
            let [r, g, b] = frame.rgb(x, y);
            let pixel = pack(r, layout.red) | pack(g, layout.green) | pack(b, layout.blue);
            let at = line + screen_x as usize * bytes;
            if at + bytes <= memory.len() {
                memory[at..at + bytes].copy_from_slice(&pixel.to_le_bytes()[..bytes]);
            }
        }
    }
}

/// A [FrameEncoder] that writes into framebuffer memory instead of bytes
/// for the terminal: `out` stays empty.
pub struct FramebufferEncoder<'a> {
    pub layout: Layout,
    pub memory: &'a mut [u8],
    /// Where the frame's top-left goes, to centre a frame smaller than the
    /// screen.
    pub left: u32,
    pub top: u32,
}

impl FrameEncoder for FramebufferEncoder<'_> {
    fn encode(&mut self, frame: &Frame, damage: &[Rect], _cells: &CellMetrics, _out: &mut Vec<u8>) {
        for rect in damage {
            blit(frame, *rect, &self.layout, self.memory, self.left, self.top);
        }
    }

    fn reset(&mut self) {}
}

/// A PS/2 mouse packet from `/dev/input/mice`: three bytes, relative motion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MousePacket {
    pub left: bool,
    pub right: bool,
    pub middle: bool,
    /// Rightwards.
    pub dx: i32,
    /// Downwards (the device reports upwards; this is screen direction).
    pub dy: i32,
}

/// Reads one packet; None when the first byte is not a packet start.
pub fn read_mouse_packet(bytes: [u8; 3]) -> Option<MousePacket> {
    let [flags, x, y] = bytes;
    if flags & 0x08 == 0 {
        return None;
    }
    let signed = |value: u8, negative: bool| -> i32 {
        if negative { value as i32 - 256 } else { value as i32 }
    };
    Some(MousePacket {
        left: flags & 0x01 != 0,
        right: flags & 0x02 != 0,
        middle: flags & 0x04 != 0,
        dx: signed(x, flags & 0x10 != 0),
        dy: -signed(y, flags & 0x20 != 0),
    })
}

#[cfg(target_os = "linux")]
pub mod device;

/// Switches a console back to text mode; a no-op where there is no framebuffer.
pub fn device_text_mode(console_fd: i32) {
    #[cfg(target_os = "linux")]
    device::text_mode(console_fd);
    #[cfg(not(target_os = "linux"))]
    let _ = console_fd;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::PixelFormat;

    #[test]
    fn xrgb8888_gets_the_pixels_little_endian() {
        let bgra = [30u8, 20, 10, 255, 3, 2, 1, 255];
        let frame = Frame::packed(2, 1, PixelFormat::Bgra8, &bgra);
        let layout = Layout::xrgb8888(2, 1);
        let mut memory = vec![0u8; 8];
        blit(&frame, frame.bounds(), &layout, &mut memory, 0, 0);
        // 0x000A141E little-endian is 1E 14 0A 00.
        assert_eq!(memory, vec![30, 20, 10, 0, 3, 2, 1, 0]);
    }

    #[test]
    fn rgb565_packs_the_top_bits() {
        let rgba = [0xFFu8, 0x80, 0x08, 255];
        let frame = Frame::packed(1, 1, PixelFormat::Rgba8, &rgba);
        let mut memory = vec![0u8; 2];
        blit(&frame, frame.bounds(), &Layout::rgb565(1, 1), &mut memory, 0, 0);
        // r=31, g=32, b=1 -> 0b11111_100000_00001 = 0xFC01.
        assert_eq!(memory, vec![0x01, 0xFC]);
    }

    #[test]
    fn only_the_damaged_rect_is_written_at_its_offset_with_line_padding() {
        let rgba: Vec<u8> = (0..4).flat_map(|i| [i * 10, 0, 0, 255]).collect();
        let frame = Frame::packed(2, 2, PixelFormat::Rgba8, &rgba);
        let layout = Layout { line_length: 16, ..Layout::xrgb8888(3, 3) };
        let mut memory = vec![0xEEu8; 16 * 3];
        blit(&frame, Rect { x: 1, y: 1, width: 1, height: 1 }, &layout, &mut memory, 1, 0);
        // Frame pixel (1,1) red=30 lands at framebuffer (2,1).
        let at = 16 + 2 * 4;
        assert_eq!(&memory[at..at + 4], &[0, 0, 30, 0]);
        let changed: Vec<usize> = (0..memory.len()).filter(|&index| memory[index] != 0xEE).collect();
        assert_eq!(changed, vec![at, at + 1, at + 2, at + 3], "nothing else touched");
    }

    #[test]
    fn pixels_past_the_framebuffer_edge_are_dropped() {
        let rgba = vec![200u8; 4 * 4 * 4];
        let frame = Frame::packed(4, 4, PixelFormat::Rgba8, &rgba);
        let layout = Layout::xrgb8888(2, 2);
        let mut memory = vec![0u8; 16];
        blit(&frame, frame.bounds(), &layout, &mut memory, 1, 1);
        assert_eq!(memory[..12], [0u8; 12]);
        assert_eq!(&memory[12..], &[200, 200, 200, 0]);
    }

    #[test]
    fn odd_layouts_are_refused() {
        assert!(Layout::xrgb8888(1, 1).supported());
        assert!(Layout::rgb565(1, 1).supported());
        assert!(!Layout { bits_per_pixel: 8, ..Layout::xrgb8888(1, 1) }.supported());
        assert!(!Layout { bits_per_pixel: 1, ..Layout::xrgb8888(1, 1) }.supported());
    }

    #[test]
    fn mouse_packets_read_buttons_and_signed_motion() {
        assert_eq!(
            read_mouse_packet([0x09, 5, 3]),
            Some(MousePacket { left: true, right: false, middle: false, dx: 5, dy: -3 })
        );
        // x negative (0x10), y negative (0x20) means moving left and down.
        assert_eq!(
            read_mouse_packet([0x08 | 0x10 | 0x20 | 0x02, 0xFB, 0xFE]),
            Some(MousePacket { left: false, right: true, middle: false, dx: -5, dy: 2 })
        );
        assert_eq!(read_mouse_packet([0x00, 1, 1]), None, "bit 3 is always set in a first byte");
    }
}
