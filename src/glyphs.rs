//! Every character the UI draws, with an ASCII fallback for terminals whose
//! font cannot manage the Unicode set.
//!
//! Colors are named `ratatui` colors only - never `Color::Rgb` - so the user's
//! terminal palette is the theme. Theme your terminal, theme this program.

use ratatui::style::Color;
use ratatui::symbols::border;

/// An all-ASCII frame. `--ascii` exists for terminals whose font cannot render
/// the Unicode set, and box-drawing characters are exactly what such a font
/// tends to be missing - so the border has to switch too, not just the glyphs.
pub const ASCII_BORDER: border::Set = border::Set {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: "-",
    horizontal_bottom: "-",
};

/// Group markers. A group's identity is shape *and* color, so the first 18
/// groups never repeat a shape and 18 x 7 = 126 groups never repeat a marker.
pub const SHAPES: [&str; 18] = [
    "\u{25A0}", "\u{25C6}", "\u{25B2}", "\u{25CF}", "\u{25BC}", "\u{25C0}", "\u{25B6}", "\u{2605}",
    "\u{2B22}", "\u{25FC}", "\u{25C7}", "\u{25B3}", "\u{25CB}", "\u{25BD}", "\u{25C1}", "\u{25B7}",
    "\u{2606}", "\u{2B21}",
];

pub const SHAPES_ASCII: [&str; 18] = [
    "#", "+", "^", "o", "v", "<", ">", "*", "@", "%", "=", "~", "O", "V", "{", "}", "&", "0",
];

pub const MARKER_COLORS: [Color; 7] = [
    Color::Cyan,
    Color::Magenta,
    Color::Blue,
    Color::Yellow,
    Color::Green,
    Color::Red,
    Color::White,
];

/// Eight heights for the heartbeat bar. Milestone 3 uses the full range to
/// encode latency; the last entry doubles as the solid "down" block.
pub const BEAT_LEVELS: [char; 8] = [
    '\u{2581}', '\u{2582}', '\u{2583}', '\u{2584}', '\u{2585}', '\u{2586}', '\u{2587}', '\u{2588}',
];

pub const BEAT_LEVELS_ASCII: [char; 8] = ['.', ':', '-', '=', '+', '*', '#', '%'];

/// The set of glyphs in use for this run. Pass `ascii: true` for `--ascii`.
#[derive(Debug, Copy, Clone)]
pub struct Glyphs {
    pub ascii: bool,
}

impl Glyphs {
    pub fn new(ascii: bool) -> Self {
        Glyphs { ascii }
    }

    /// Shape and color identifying a group. Walks every shape before reusing a
    /// color, so nearby groups differ by shape as well as by hue.
    pub fn marker(self, group: usize) -> (&'static str, Color) {
        let shapes = if self.ascii { &SHAPES_ASCII } else { &SHAPES };
        (
            shapes[group % SHAPES.len()],
            MARKER_COLORS[(group / SHAPES.len()) % MARKER_COLORS.len()],
        )
    }

    pub fn beat(self, level: usize) -> char {
        let levels = if self.ascii {
            &BEAT_LEVELS_ASCII
        } else {
            &BEAT_LEVELS
        };
        levels[level.min(levels.len() - 1)]
    }

    /// The per-monitor status dot, kept distinct from the group marker.
    pub fn dot(self) -> &'static str {
        if self.ascii { "o" } else { "\u{25CF}" }
    }

    pub fn ok(self) -> &'static str {
        if self.ascii { "OK" } else { "\u{2714}" }
    }

    pub fn bad(self) -> &'static str {
        if self.ascii { "!!" } else { "\u{2718}" }
    }

    /// U+2699 GEAR - single width. Deliberately not the wrench emoji, which is
    /// double width and would break every column to its right.
    pub fn maint(self) -> &'static str {
        if self.ascii { "MT" } else { "\u{2699}" }
    }

    pub fn pending(self) -> &'static str {
        if self.ascii { ".." } else { "\u{2026}" }
    }

    pub fn incident(self) -> &'static str {
        if self.ascii { "!" } else { "\u{25B2}" }
    }

    pub fn border(self) -> border::Set<'static> {
        if self.ascii {
            ASCII_BORDER
        } else {
            border::PLAIN
        }
    }

    /// Appended to a name that had to be cut short.
    pub fn ellipsis(self) -> &'static str {
        if self.ascii { "~" } else { "\u{2026}" }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_vary_by_shape_before_they_vary_by_color() {
        let g = Glyphs::new(false);
        assert_eq!(g.marker(0), ("\u{25A0}", Color::Cyan));
        assert_eq!(g.marker(1), ("\u{25C6}", Color::Cyan));
        assert_eq!(g.marker(17), ("\u{2B21}", Color::Cyan));
        // Shapes exhausted: same shape sequence, next color.
        assert_eq!(g.marker(18), ("\u{25A0}", Color::Magenta));
    }

    #[test]
    fn every_group_index_yields_a_marker() {
        let g = Glyphs::new(false);
        for i in 0..500 {
            let (shape, _) = g.marker(i);
            assert!(!shape.is_empty());
        }
    }

    #[test]
    fn ascii_mode_is_pure_ascii() {
        let g = Glyphs::new(true);
        let mut s = String::new();
        let b = g.border();
        for part in [
            b.top_left,
            b.top_right,
            b.bottom_left,
            b.bottom_right,
            b.vertical_left,
            b.vertical_right,
            b.horizontal_top,
            b.horizontal_bottom,
        ] {
            s.push_str(part);
        }
        for i in 0..40 {
            s.push_str(g.marker(i).0);
        }
        for l in 0..8 {
            s.push(g.beat(l));
        }
        s.push_str(g.dot());
        s.push_str(g.ok());
        s.push_str(g.bad());
        s.push_str(g.maint());
        s.push_str(g.pending());
        s.push_str(g.incident());
        s.push_str(g.ellipsis());
        assert!(s.is_ascii(), "ascii mode emitted non-ascii: {s}");
    }

    #[test]
    fn beat_level_is_clamped() {
        let g = Glyphs::new(false);
        assert_eq!(g.beat(0), '\u{2581}');
        assert_eq!(g.beat(7), '\u{2588}');
        assert_eq!(g.beat(99), '\u{2588}');
    }
}
