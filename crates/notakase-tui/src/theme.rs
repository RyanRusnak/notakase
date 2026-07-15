// theme.rs — the visual vocabulary: accent color (pulled from the live Omarchy
// theme) + icon set.
//
// Almost every color in the app is ANSI/indexed, so the terminal's Omarchy
// theme paints the UI. The one "choice" is the accent hue — used for the
// selection bar, brand, headings and links. We read it straight from the
// active theme's `colors.toml` (the `accent = "#rrggbb"` line) so notakase
// matches whatever theme the desktop is running. Override with NOTAKASE_ACCENT
// (a hex value, a color name, or a 0-255 index).
//
// Icons default to Nerd Font glyphs (Omarchy ships a Nerd Font). Set
// NOTAKASE_ASCII=1 to fall back to plain, widely-supported Unicode.

use std::path::PathBuf;
use std::sync::OnceLock;

use ratatui::style::Color;

pub struct Glyphs {
    pub brand: &'static str,
    pub folder_closed: &'static str,
    pub folder_open: &'static str,
    pub note: &'static str,
    /// Left bar drawn on the selected row.
    pub sel: &'static str,
    pub task_done: &'static str,
    pub task_open: &'static str,
    pub link: &'static str,
    pub image: &'static str,
    pub cloud: &'static str,
    pub lock: &'static str,
    pub server: &'static str,
    pub search: &'static str,
    pub command: &'static str,
}

// Nerd Font (Private Use Area) glyphs.
const NERD: Glyphs = Glyphs {
    brand: "\u{f02d}",         // book
    folder_closed: "\u{f07b}", // folder
    folder_open: "\u{f07c}",   // folder-open
    note: "\u{f15c}",          // file-text
    sel: "▎",
    task_done: "\u{f058}", // check-circle
    task_open: "\u{f10c}", // circle-o
    link: "\u{f0c1}",      // link
    image: "\u{f03e}",     // image
    cloud: "\u{f0c2}",     // cloud
    lock: "\u{f023}",      // lock
    server: "\u{f0ac}",    // globe
    search: "\u{f002}",    // magnifier
    command: "\u{f120}",   // terminal
};

// Plain-Unicode fallback — safe in any font.
const ASCII: Glyphs = Glyphs {
    brand: "≡",
    folder_closed: "▸",
    folder_open: "▾",
    note: "•",
    sel: "▎",
    task_done: "✓",
    task_open: "○",
    link: "↗",
    image: "▤",
    cloud: "↑",
    lock: "#",
    server: "@",
    search: "/",
    command: ":",
};

pub fn ascii() -> bool {
    static A: OnceLock<bool> = OnceLock::new();
    *A.get_or_init(|| {
        std::env::var("NOTAKASE_ASCII")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    })
}

pub fn glyphs() -> &'static Glyphs {
    if ascii() {
        &ASCII
    } else {
        &NERD
    }
}

/// The one accent hue. Precedence: NOTAKASE_ACCENT override → the live Omarchy
/// theme's `colors.toml` accent → magenta.
pub fn accent() -> Color {
    static A: OnceLock<Color> = OnceLock::new();
    *A.get_or_init(|| {
        if let Some(c) = std::env::var("NOTAKASE_ACCENT").ok().and_then(|s| parse_color(&s)) {
            return c;
        }
        omarchy_accent().unwrap_or(Color::Magenta)
    })
}

/// Read `accent = "#rrggbb"` from the active Omarchy theme's colors.toml.
fn omarchy_accent() -> Option<Color> {
    let path = omarchy_colors_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("accent") {
            // `accent = "#89b4fa"`
            let val = rest.trim_start().strip_prefix('=')?.trim().trim_matches('"');
            return parse_hex(val);
        }
    }
    None
}

fn omarchy_colors_path() -> Option<PathBuf> {
    let home = dirs_home()?;
    Some(home.join(".config/omarchy/current/theme/colors.toml"))
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim();
    if s.starts_with('#') {
        return parse_hex(s);
    }
    Some(match s.to_lowercase().as_str() {
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" | "purple" | "pink" => Color::Magenta,
        "cyan" => Color::Cyan,
        "white" => Color::White,
        "gray" | "grey" => Color::DarkGray,
        _ => return s.parse::<u8>().ok().map(Color::Indexed),
    })
}

fn parse_hex(s: &str) -> Option<Color> {
    let h = s.trim().strip_prefix('#')?;
    let (r, g, b) = match h.len() {
        6 => (
            u8::from_str_radix(&h[0..2], 16).ok()?,
            u8::from_str_radix(&h[2..4], 16).ok()?,
            u8::from_str_radix(&h[4..6], 16).ok()?,
        ),
        3 => {
            let d = |c: &str| u8::from_str_radix(c, 16).ok().map(|v| v.saturating_mul(17));
            (d(&h[0..1])?, d(&h[1..2])?, d(&h[2..3])?)
        }
        _ => return None,
    };
    Some(Color::Rgb(r, g, b))
}
