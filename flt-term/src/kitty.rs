//! The Kitty graphics protocol.
//!
//! The frame is cut into fixed tiles, each its own image with a stable id.
//! When a tile changes it is transmitted again under the same id and
//! placement id, which the terminal treats as a replacement in place: no
//! flicker, no images piling up, and only the changed tiles cross the wire.
//! Pixels go as zlib-compressed RGB (`f=24,o=z`), base64 in chunks of at most
//! 4096 bytes as the protocol requires.
//!
//! Inside tmux images cannot be placed at the cursor -- tmux does not know
//! they are there and would draw over them. There the image is transmitted
//! through passthrough as a virtual placement (`U=1`) and shown by writing
//! Unicode placeholder cells (U+10EEEE with row and column diacritics,
//! coloured with the image id) that tmux moves like any text. Kitty and
//! Ghostty implement this.

use std::collections::HashSet;

use base64::prelude::*;

use crate::detect::tmux_passthrough;
use crate::frame::{CellMetrics, Frame, FrameEncoder, Multiplexer, Rect};

/// Base64 bytes per chunk, the protocol's maximum.
pub const CHUNK: usize = 4096;

/// Image ids must fit the 256-colour index placeholders encode them in.
const MAX_TILES: u32 = 255;

/// The placeholder character.
const PLACEHOLDER: char = '\u{10EEEE}';

/// Row and column numbers as combining diacritics, from Kitty's
/// rowcolumn-diacritics.txt (the first 40; tiles never get that tall).
const DIACRITICS: [u32; 40] = [
    0x0305, 0x030D, 0x030E, 0x0310, 0x0312, 0x033D, 0x033E, 0x033F, 0x0346, 0x034A, 0x034B,
    0x034C, 0x0350, 0x0351, 0x0352, 0x0357, 0x035B, 0x0363, 0x0364, 0x0365, 0x0366, 0x0367,
    0x0368, 0x0369, 0x036A, 0x036B, 0x036C, 0x036D, 0x036E, 0x036F, 0x0483, 0x0484, 0x0485,
    0x0486, 0x0487, 0x0592, 0x0593, 0x0594, 0x0595, 0x0597,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KittyOptions {
    /// Tile size in cells. Smaller tiles send less on small changes and more
    /// escape overhead on large ones; 16x8 suits typical UI changes.
    pub tile_columns: u16,
    pub tile_rows: u16,
    /// Unicode placeholders (inside tmux).
    pub placeholders: bool,
    pub multiplexer: Multiplexer,
    /// Compress pixels with zlib. Off only for tests that read the bytes.
    pub compress: bool,
}

impl Default for KittyOptions {
    fn default() -> Self {
        KittyOptions {
            tile_columns: 16,
            tile_rows: 8,
            placeholders: false,
            multiplexer: Multiplexer::None,
            compress: true,
        }
    }
}

pub struct KittyEncoder {
    options: KittyOptions,
    /// Tiles whose placeholder cells are on screen.
    placed: HashSet<u32>,
}

impl KittyEncoder {
    pub fn new(options: KittyOptions) -> Self {
        KittyEncoder { options, placed: HashSet::new() }
    }

    /// Deletes every image this encoder put up, freeing the terminal's
    /// memory. Written when the application exits.
    pub fn clear_all(&self) -> Vec<u8> {
        self.wrap("\x1b_Ga=d,d=A,q=2\x1b\\").into_bytes()
    }

    fn wrap(&self, command: &str) -> String {
        if self.options.multiplexer == Multiplexer::Tmux {
            tmux_passthrough(command)
        } else {
            command.to_string()
        }
    }

    /// The tile size in cells actually used for `cells`, grown until the
    /// grid has at most [MAX_TILES] tiles.
    pub fn tile_cells(&self, cells: &CellMetrics) -> (u32, u32) {
        let (mut columns, mut rows) = (self.options.tile_columns.max(1) as u32, self.options.tile_rows.max(1) as u32);
        let count = |columns: u32, rows: u32| {
            (cells.columns as u32).div_ceil(columns) * (cells.rows as u32).div_ceil(rows)
        };
        while count(columns, rows) > MAX_TILES {
            // Grow the shorter side first, keeping tiles roughly square in pixels.
            if columns <= rows * 2 {
                columns += 1;
            } else {
                rows += 1;
            }
        }
        (columns, rows)
    }

    /// Sends one tile: its pixels, then (with placeholders, the first time)
    /// the cells that show it.
    fn send_tile(&mut self, frame: &Frame, cells: &CellMetrics, tile: Rect, id: u32, out: &mut Vec<u8>) {
        let mut rgb = Vec::with_capacity((tile.width * tile.height * 3) as usize);
        for y in tile.y..tile.y + tile.height {
            for x in tile.x..tile.x + tile.width {
                rgb.extend_from_slice(&frame.rgb(x, y));
            }
        }
        let compressed = self.options.compress;
        let payload = if compressed { miniz_oxide::deflate::compress_to_vec_zlib(&rgb, 1) } else { rgb };
        let encoded = BASE64_STANDARD.encode(payload);
        let compression = if compressed { "o=z," } else { "" };
        let columns = tile.width.div_ceil(cells.cell_width);
        let rows = tile.height.div_ceil(cells.cell_height);
        let keys = if self.options.placeholders {
            format!(
                "a=T,U=1,f=24,{compression}s={},v={},i={id},c={columns},r={rows},q=2",
                tile.width, tile.height
            )
        } else {
            format!("a=T,f=24,{compression}s={},v={},i={id},p=1,q=2,C=1", tile.width, tile.height)
        };
        if !self.options.placeholders {
            out.extend_from_slice(cells.cursor_to(tile.x, tile.y).as_bytes());
        }
        let chunks: Vec<&[u8]> = if encoded.is_empty() { vec![&[][..]] } else { encoded.as_bytes().chunks(CHUNK).collect() };
        let last = chunks.len() - 1;
        for (index, chunk) in chunks.iter().enumerate() {
            let more = if chunks.len() == 1 { String::new() } else if index == last { "m=0".into() } else { "m=1".into() };
            let control = match (index, more.is_empty()) {
                (0, true) => keys.clone(),
                (0, false) => format!("{keys},{more}"),
                _ => more,
            };
            let command = format!("\x1b_G{control};{}\x1b\\", std::str::from_utf8(chunk).unwrap());
            out.extend_from_slice(self.wrap(&command).as_bytes());
        }
        if self.options.placeholders && self.placed.insert(id) {
            for row in 0..rows {
                out.extend_from_slice(cells.cursor_to(tile.x, tile.y + row * cells.cell_height).as_bytes());
                let mut line = format!("\x1b[38;5;{id}m{PLACEHOLDER}");
                line.push(char::from_u32(DIACRITICS[row as usize]).unwrap());
                line.push(char::from_u32(DIACRITICS[0]).unwrap());
                for _ in 1..columns {
                    line.push(PLACEHOLDER);
                }
                line.push_str("\x1b[39m");
                out.extend_from_slice(line.as_bytes());
            }
        }
    }
}

impl FrameEncoder for KittyEncoder {
    fn encode(&mut self, frame: &Frame, damage: &[Rect], cells: &CellMetrics, out: &mut Vec<u8>) {
        if damage.is_empty() {
            return;
        }
        let (tile_columns, tile_rows) = self.tile_cells(cells);
        let tile_width = tile_columns * cells.cell_width;
        let tile_height = tile_rows * cells.cell_height;
        let tiles_across = frame.width.div_ceil(tile_width);
        let mut tiles = std::collections::BTreeSet::new();
        for rect in damage.iter().filter(|rect| !rect.is_empty()) {
            for tile_y in rect.y / tile_height..(rect.y + rect.height).div_ceil(tile_height) {
                for tile_x in rect.x / tile_width..(rect.x + rect.width).div_ceil(tile_width) {
                    tiles.insert((tile_y, tile_x));
                }
            }
        }
        for (tile_y, tile_x) in tiles {
            let x = tile_x * tile_width;
            let y = tile_y * tile_height;
            if x >= frame.width || y >= frame.height {
                continue;
            }
            let tile = Rect {
                x,
                y,
                width: tile_width.min(frame.width - x),
                height: tile_height.min(frame.height - y),
            };
            let id = tile_y * tiles_across + tile_x + 1;
            self.send_tile(frame, cells, tile, id, out);
        }
    }

    fn reset(&mut self) {
        self.placed.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::PixelFormat;

    /// 4x2 pixels, BGRA: left half red, right half blue.
    fn two_tile_frame() -> Vec<u8> {
        let red = [0u8, 0, 255, 255];
        let blue = [255u8, 0, 0, 255];
        [red, red, blue, blue, red, red, blue, blue].concat()
    }

    fn cells_2x2(columns: u16, rows: u16) -> CellMetrics {
        CellMetrics { cell_width: 2, cell_height: 2, columns, rows, origin_column: 0, origin_row: 0 }
    }

    fn plain() -> KittyOptions {
        KittyOptions { tile_columns: 1, tile_rows: 1, compress: false, ..KittyOptions::default() }
    }

    fn text(bytes: &[u8]) -> String {
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[test]
    fn each_damaged_tile_is_one_image_at_its_cell_replacing_the_last() {
        let data = two_tile_frame();
        let frame = Frame::packed(4, 2, PixelFormat::Bgra8, &data);
        let mut encoder = KittyEncoder::new(plain());
        let mut out = Vec::new();
        encoder.encode(&frame, &[frame.bounds()], &cells_2x2(2, 1), &mut out);
        let red = BASE64_STANDARD.encode([255u8, 0, 0, 255, 0, 0, 255, 0, 0, 255, 0, 0]);
        let blue = BASE64_STANDARD.encode([0u8, 0, 255, 0, 0, 255, 0, 0, 255, 0, 0, 255]);
        assert_eq!(
            text(&out),
            format!(
                "\x1b[1;1H\x1b_Ga=T,f=24,s=2,v=2,i=1,p=1,q=2,C=1;{red}\x1b\\\
                 \x1b[1;2H\x1b_Ga=T,f=24,s=2,v=2,i=2,p=1,q=2,C=1;{blue}\x1b\\"
            )
        );
    }

    #[test]
    fn only_tiles_touched_by_damage_are_sent() {
        let data = two_tile_frame();
        let frame = Frame::packed(4, 2, PixelFormat::Bgra8, &data);
        let mut encoder = KittyEncoder::new(plain());
        let mut out = Vec::new();
        encoder.encode(&frame, &[Rect { x: 2, y: 0, width: 2, height: 2 }], &cells_2x2(2, 1), &mut out);
        let sent = text(&out);
        assert!(sent.starts_with("\x1b[1;2H\x1b_Ga=T,f=24,s=2,v=2,i=2,"), "{sent:?}");
        assert_eq!(sent.matches("\x1b_G").count(), 1);
    }

    #[test]
    fn no_damage_sends_nothing() {
        let data = two_tile_frame();
        let frame = Frame::packed(4, 2, PixelFormat::Bgra8, &data);
        let mut out = Vec::new();
        KittyEncoder::new(plain()).encode(&frame, &[], &cells_2x2(2, 1), &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn compressed_pixels_inflate_back_to_the_tile() {
        let data = two_tile_frame();
        let frame = Frame::packed(4, 2, PixelFormat::Bgra8, &data);
        let mut encoder = KittyEncoder::new(KittyOptions { compress: true, ..plain() });
        let mut out = Vec::new();
        encoder.encode(&frame, &[Rect { x: 0, y: 0, width: 2, height: 2 }], &cells_2x2(2, 1), &mut out);
        let sent = text(&out);
        assert!(sent.contains("a=T,f=24,o=z,s=2,v=2,i=1,"), "{sent:?}");
        let command = sent.split("\x1b_G").nth(1).unwrap();
        let payload = command.split(';').nth(1).unwrap().trim_end_matches("\x1b\\");
        let inflated = miniz_oxide::inflate::decompress_to_vec_zlib(&BASE64_STANDARD.decode(payload).unwrap()).unwrap();
        assert_eq!(inflated, vec![255, 0, 0, 255, 0, 0, 255, 0, 0, 255, 0, 0]);
    }

    #[test]
    fn large_payloads_are_chunked_at_4096_with_keys_only_on_the_first() {
        // 64x64 noise-ish pixels, uncompressed: 12288 bytes of RGB.
        let data: Vec<u8> = (0..64 * 64 * 4).map(|i| (i * 7 % 251) as u8).collect();
        let frame = Frame::packed(64, 64, PixelFormat::Bgra8, &data);
        let cells = CellMetrics { cell_width: 64, cell_height: 64, columns: 1, rows: 1, origin_column: 0, origin_row: 0 };
        let mut out = Vec::new();
        KittyEncoder::new(plain()).encode(&frame, &[frame.bounds()], &cells, &mut out);
        let sent = text(&out);
        let chunks: Vec<&str> = sent.split("\x1b_G").skip(1).collect();
        assert_eq!(chunks.len(), 4, "16384 base64 bytes in four chunks");
        assert!(chunks[0].starts_with("a=T,f=24,s=64,v=64,i=1,p=1,q=2,C=1,m=1;"));
        assert!(chunks[1].starts_with("m=1;"));
        assert!(chunks[3].starts_with("m=0;"));
        for chunk in &chunks {
            let payload = chunk.split(';').nth(1).unwrap().trim_end_matches("\x1b\\");
            assert!(payload.len() <= CHUNK && payload.len() % 4 == 0, "{}", payload.len());
        }
    }

    #[test]
    fn a_tile_cut_by_the_frame_edge_sends_only_what_exists() {
        // 3x2 pixels with 2x2 cells: the second tile is one pixel wide.
        let data = vec![9u8; 3 * 2 * 4];
        let frame = Frame::packed(3, 2, PixelFormat::Bgra8, &data);
        let mut out = Vec::new();
        KittyEncoder::new(plain()).encode(
            &frame,
            &[Rect { x: 2, y: 0, width: 1, height: 2 }],
            &cells_2x2(2, 1),
            &mut out,
        );
        assert!(text(&out).contains("s=1,v=2,i=2,"));
    }

    #[test]
    fn the_origin_offsets_the_cursor() {
        let data = two_tile_frame();
        let frame = Frame::packed(4, 2, PixelFormat::Bgra8, &data);
        let cells = CellMetrics { origin_row: 2, origin_column: 3, ..cells_2x2(2, 1) };
        let mut out = Vec::new();
        KittyEncoder::new(plain()).encode(&frame, &[Rect { x: 2, y: 0, width: 2, height: 2 }], &cells, &mut out);
        assert!(text(&out).starts_with("\x1b[3;5H"));
    }

    #[test]
    fn inside_tmux_images_go_through_passthrough_and_placeholders_are_text() {
        let data = two_tile_frame();
        let frame = Frame::packed(4, 2, PixelFormat::Bgra8, &data);
        let options = KittyOptions { placeholders: true, multiplexer: Multiplexer::Tmux, ..plain() };
        let mut encoder = KittyEncoder::new(options);
        let mut out = Vec::new();
        encoder.encode(&frame, &[Rect { x: 0, y: 0, width: 2, height: 2 }], &cells_2x2(2, 1), &mut out);
        let red = BASE64_STANDARD.encode([255u8, 0, 0, 255, 0, 0, 255, 0, 0, 255, 0, 0]);
        assert_eq!(
            text(&out),
            format!(
                "\x1bPtmux;\x1b\x1b_Ga=T,U=1,f=24,s=2,v=2,i=1,c=1,r=1,q=2;{red}\x1b\x1b\\\x1b\\\
                 \x1b[1;1H\x1b[38;5;1m\u{10EEEE}\u{0305}\u{0305}\x1b[39m"
            )
        );
        // Sent again, the image is replaced and the placeholder text is not
        // rewritten: it is still on screen.
        let mut again = Vec::new();
        encoder.encode(&frame, &[Rect { x: 0, y: 0, width: 2, height: 2 }], &cells_2x2(2, 1), &mut again);
        assert!(!text(&again).contains('\u{10EEEE}'));
        encoder.reset();
        let mut after_reset = Vec::new();
        encoder.encode(&frame, &[Rect { x: 0, y: 0, width: 2, height: 2 }], &cells_2x2(2, 1), &mut after_reset);
        assert!(text(&after_reset).contains('\u{10EEEE}'));
    }

    #[test]
    fn placeholder_rows_name_the_row_and_let_columns_follow() {
        // One tile of 3x2 cells.
        let data = vec![0u8; 6 * 4 * 4];
        let frame = Frame::packed(6, 4, PixelFormat::Bgra8, &data);
        let options = KittyOptions { tile_columns: 3, tile_rows: 2, placeholders: true, multiplexer: Multiplexer::Tmux, compress: false };
        let mut out = Vec::new();
        KittyEncoder::new(options).encode(&frame, &[frame.bounds()], &cells_2x2(3, 2), &mut out);
        let sent = text(&out);
        assert!(sent.contains("\x1b[1;1H\x1b[38;5;1m\u{10EEEE}\u{0305}\u{0305}\u{10EEEE}\u{10EEEE}\x1b[39m"), "{sent:?}");
        assert!(sent.contains("\x1b[2;1H\x1b[38;5;1m\u{10EEEE}\u{030D}\u{0305}\u{10EEEE}\u{10EEEE}\x1b[39m"), "{sent:?}");
    }

    #[test]
    fn tiles_grow_until_ids_fit_in_256_colours() {
        let encoder = KittyEncoder::new(KittyOptions { tile_columns: 1, tile_rows: 1, ..KittyOptions::default() });
        let (columns, rows) = encoder.tile_cells(&cells_2x2(200, 60));
        let count = 200u32.div_ceil(columns) * 60u32.div_ceil(rows);
        assert!(count <= 255, "{columns}x{rows} gives {count}");
        let default = KittyEncoder::new(KittyOptions::default());
        assert_eq!(default.tile_cells(&cells_2x2(80, 24)), (16, 8));
    }

    #[test]
    fn clearing_deletes_every_image() {
        assert_eq!(KittyEncoder::new(plain()).clear_all(), b"\x1b_Ga=d,d=A,q=2\x1b\\".to_vec());
        let tmux = KittyEncoder::new(KittyOptions { multiplexer: Multiplexer::Tmux, ..plain() });
        assert_eq!(tmux.clear_all(), b"\x1bPtmux;\x1b\x1b_Ga=d,d=A,q=2\x1b\x1b\\\x1b\\".to_vec());
    }
}
