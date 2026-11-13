//! The runner's configuration: `dartvel.terminal` in the application's
//! pubspec.yaml reaches the runner as `DARTVEL_TERMINAL_*` variables written
//! into the bundle's launcher, and a user can set the same variables to
//! override them.
//!
//! | Variable | Values | Default |
//! |---|---|---|
//! | `DARTVEL_TERMINAL_GRAPHICS` | `auto`, `kitty`, `sixel`, `cells`, `framebuffer` | `auto` |
//! | `DARTVEL_TERMINAL_COLORS` | `auto`, `truecolor`, `256`, `16` | `auto` |
//! | `DARTVEL_TERMINAL_CELLS` | `auto`, `octant`, `sextant`, `halfblock` | `auto` |
//! | `DARTVEL_TERMINAL_FPS` | a number: a fixed rate, no adaptation | adaptive |
//! | `DARTVEL_TERMINAL_MAX_FPS` | the full rate adaptation starts from | `60` |
//! | `DARTVEL_TERMINAL_ROW_HEIGHT` | logical pixels per terminal row | 20 (pixels), 8 (cells) |
//! | `DARTVEL_TERMINAL_HINT` | `1`: log a one-line hint when only cells are possible | off |
//! | `DARTVEL_TERMINAL_STATS` | a file: per-frame timings as CSV | off |

use crate::detect::{Output, Overrides};
use crate::frame::{CellShape, ColorDepth};

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub overrides: Overrides,
    pub fixed_fps: Option<u32>,
    pub max_fps: u32,
    pub row_height: Option<f64>,
    pub hint: bool,
    pub stats: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config { overrides: Overrides::default(), fixed_fps: None, max_fps: 60, row_height: None, hint: false, stats: None }
    }
}

impl Config {
    /// Reads the configuration through `lookup` (the process environment in
    /// the runner). Values it cannot read are ignored, never fatal: a typo
    /// in a variable must not stop an application from starting.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Config {
        let value = |key: &str| lookup(key).map(|value| value.trim().to_ascii_lowercase()).filter(|value| !value.is_empty());
        let graphics = value("DARTVEL_TERMINAL_GRAPHICS").and_then(|value| match value.as_str() {
            "kitty" => Some(Output::Kitty { placeholders: false }),
            "sixel" => Some(Output::Sixel),
            "cells" | "ansi" => Some(Output::Cells),
            "framebuffer" => Some(Output::Framebuffer),
            _ => None,
        });
        let color = value("DARTVEL_TERMINAL_COLORS").and_then(|value| match value.as_str() {
            "truecolor" | "24bit" => Some(ColorDepth::TrueColor),
            "256" => Some(ColorDepth::Ansi256),
            "16" => Some(ColorDepth::Ansi16 { bright_backgrounds: true }),
            _ => None,
        });
        let cells = value("DARTVEL_TERMINAL_CELLS").and_then(|value| match value.replace('-', "").as_str() {
            "octant" => Some(CellShape::Octant),
            "sextant" => Some(CellShape::Sextant),
            "halfblock" => Some(CellShape::HalfBlock),
            _ => None,
        });
        let rate = |key: &str| value(key).and_then(|value| value.parse::<u32>().ok()).filter(|&rate| rate > 0).map(|rate| rate.min(240));
        Config {
            overrides: Overrides { graphics, color, cells },
            fixed_fps: rate("DARTVEL_TERMINAL_FPS"),
            max_fps: rate("DARTVEL_TERMINAL_MAX_FPS").unwrap_or(60),
            row_height: value("DARTVEL_TERMINAL_ROW_HEIGHT").and_then(|value| value.parse::<f64>().ok()).filter(|&height| height > 0.0),
            hint: value("DARTVEL_TERMINAL_HINT").map_or(false, |value| value == "1" || value == "true"),
            stats: lookup("DARTVEL_TERMINAL_STATS").filter(|path| !path.is_empty()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn config(pairs: &[(&str, &str)]) -> Config {
        let map: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        Config::from_lookup(|key| map.get(key).cloned())
    }

    #[test]
    fn nothing_set_is_all_automatic_at_60() {
        assert_eq!(config(&[]), Config::default());
        assert_eq!(Config::default().max_fps, 60);
    }

    #[test]
    fn graphics_colours_and_cells_are_read() {
        let read = config(&[
            ("DARTVEL_TERMINAL_GRAPHICS", "Sixel"),
            ("DARTVEL_TERMINAL_COLORS", "256"),
            ("DARTVEL_TERMINAL_CELLS", "octant"),
        ]);
        assert_eq!(read.overrides.graphics, Some(Output::Sixel));
        assert_eq!(read.overrides.color, Some(ColorDepth::Ansi256));
        assert_eq!(read.overrides.cells, Some(CellShape::Octant));
        assert_eq!(config(&[("DARTVEL_TERMINAL_GRAPHICS", "kitty")]).overrides.graphics, Some(Output::Kitty { placeholders: false }));
        assert_eq!(config(&[("DARTVEL_TERMINAL_GRAPHICS", "cells")]).overrides.graphics, Some(Output::Cells));
        assert_eq!(config(&[("DARTVEL_TERMINAL_GRAPHICS", "framebuffer")]).overrides.graphics, Some(Output::Framebuffer));
        assert_eq!(config(&[("DARTVEL_TERMINAL_COLORS", "truecolor")]).overrides.color, Some(ColorDepth::TrueColor));
        assert_eq!(config(&[("DARTVEL_TERMINAL_COLORS", "16")]).overrides.color, Some(ColorDepth::Ansi16 { bright_backgrounds: true }));
        assert_eq!(config(&[("DARTVEL_TERMINAL_CELLS", "half-block")]).overrides.cells, Some(CellShape::HalfBlock));
        assert_eq!(config(&[("DARTVEL_TERMINAL_CELLS", "sextant")]).overrides.cells, Some(CellShape::Sextant));
    }

    #[test]
    fn auto_and_nonsense_mean_detect() {
        let read = config(&[("DARTVEL_TERMINAL_GRAPHICS", "auto"), ("DARTVEL_TERMINAL_COLORS", "lots"), ("DARTVEL_TERMINAL_FPS", "fast")]);
        assert_eq!(read, Config::default());
    }

    #[test]
    fn rates_are_read_and_bounded() {
        assert_eq!(config(&[("DARTVEL_TERMINAL_FPS", "24")]).fixed_fps, Some(24));
        assert_eq!(config(&[("DARTVEL_TERMINAL_MAX_FPS", "120")]).max_fps, 120);
        assert_eq!(config(&[("DARTVEL_TERMINAL_FPS", "0")]).fixed_fps, None);
        assert_eq!(config(&[("DARTVEL_TERMINAL_MAX_FPS", "100000")]).max_fps, 240);
    }

    #[test]
    fn row_height_hint_and_stats() {
        let read = config(&[("DARTVEL_TERMINAL_ROW_HEIGHT", "16.5"), ("DARTVEL_TERMINAL_HINT", "1"), ("DARTVEL_TERMINAL_STATS", "/tmp/s.csv")]);
        assert_eq!(read.row_height, Some(16.5));
        assert!(read.hint);
        assert_eq!(read.stats.as_deref(), Some("/tmp/s.csv"));
        assert_eq!(config(&[("DARTVEL_TERMINAL_ROW_HEIGHT", "-3")]).row_height, None);
    }
}
