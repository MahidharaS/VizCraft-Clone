//! Colours (`#RRGGBB` / `#RRGGBBAA`) and the built-in report themes.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 255 }
    }
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
        Color { r, g, b, a }
    }
    pub const WHITE: Color = Color::rgb(255, 255, 255);
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    pub const TRANSPARENT: Color = Color::rgba(0, 0, 0, 0);
    pub fn hex(&self) -> String {
        if self.a == 255 {
            format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
        } else {
            format!("#{:02X}{:02X}{:02X}{:02X}", self.r, self.g, self.b, self.a)
        }
    }
    /// CSS `rgba()`.
    pub fn css(&self) -> String {
        format!("rgba({},{},{},{:.3})", self.r, self.g, self.b, self.a as f32 / 255.0)
    }
    pub fn parse(s: &str) -> Option<Color> {
        let s = s.trim();
        if let Some(c) = named(s) {
            return Some(c);
        }
        let h = s.strip_prefix('#').unwrap_or(s);
        let p = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
        match h.len() {
            6 => Some(Color::rgb(p(0)?, p(2)?, p(4)?)),
            8 => Some(Color::rgba(p(0)?, p(2)?, p(4)?, p(6)?)),
            3 => {
                let q = |i: usize| u8::from_str_radix(h.get(i..i + 1)?, 16).ok().map(|v| v * 17);
                Some(Color::rgb(q(0)?, q(1)?, q(2)?))
            }
            _ => None,
        }
    }
    pub fn with_alpha(self, a: u8) -> Color {
        Color { a, ..self }
    }
    pub fn lerp(self, o: Color, t: f64) -> Color {
        let t = t.clamp(0.0, 1.0);
        let m = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * t).round() as u8;
        Color { r: m(self.r, o.r), g: m(self.g, o.g), b: m(self.b, o.b), a: m(self.a, o.a) }
    }
    /// Lighter (t > 0) or darker (t < 0).
    pub fn shade(self, t: f64) -> Color {
        if t >= 0.0 { self.lerp(Color::WHITE.with_alpha(self.a), t) } else { self.lerp(Color::BLACK.with_alpha(self.a), -t) }
    }
    pub fn luminance(&self) -> f64 {
        (0.2126 * self.r as f64 + 0.7152 * self.g as f64 + 0.0722 * self.b as f64) / 255.0
    }
    /// Black or white, whichever reads better on this colour.
    pub fn contrast_text(&self) -> Color {
        if self.luminance() > 0.6 { Color::rgb(0x25, 0x25, 0x25) } else { Color::WHITE }
    }
}

impl From<Color> for String {
    fn from(c: Color) -> String {
        c.hex()
    }
}

impl TryFrom<String> for Color {
    type Error = String;
    fn try_from(s: String) -> Result<Color, String> {
        Color::parse(&s).ok_or_else(|| format!("`{s}` is not a colour"))
    }
}

impl Default for Color {
    fn default() -> Self {
        Color::BLACK
    }
}

fn named(s: &str) -> Option<Color> {
    Some(match s.to_ascii_lowercase().as_str() {
        "red" => Color::rgb(0xD6, 0x45, 0x50),
        "green" => Color::rgb(0x1A, 0xAB, 0x40),
        "blue" => Color::rgb(0x11, 0x8D, 0xFF),
        "orange" => Color::rgb(0xE6, 0x6C, 0x37),
        "yellow" => Color::rgb(0xF2, 0xC8, 0x0F),
        "purple" => Color::rgb(0x74, 0x4E, 0xC2),
        "gray" | "grey" => Color::rgb(0x80, 0x80, 0x80),
        "black" => Color::BLACK,
        "white" => Color::WHITE,
        "transparent" | "none" => Color::TRANSPARENT,
        _ => return None,
    })
}

/// A report theme: data colours plus the default look of text and visual containers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Theme {
    pub name: String,
    pub data_colors: Vec<Color>,
    pub background: Color,
    pub foreground: Color,
    pub foreground_muted: Color,
    pub visual_background: Color,
    pub visual_border: Color,
    pub accent: Color,
    pub good: Color,
    pub neutral: Color,
    pub bad: Color,
    pub font_family: String,
    pub title_size: f32,
    pub label_size: f32,
    pub corner_radius: f32,
    pub shadow: bool,
}

impl Default for Theme {
    fn default() -> Self {
        Theme::by_name("VizCraft").unwrap_or_else(base)
    }
}

fn base() -> Theme {
    Theme {
        name: "VizCraft".into(),
        data_colors: palette(&["#2B6CE0", "#14B8A6", "#F97316", "#8B5CF6", "#EF4444", "#EAB308", "#0EA5E9", "#EC4899", "#22C55E", "#64748B"]),
        background: Color::rgb(0xF4, 0xF5, 0xF7),
        foreground: Color::rgb(0x1F, 0x23, 0x2B),
        foreground_muted: Color::rgb(0x6B, 0x72, 0x80),
        visual_background: Color::WHITE,
        visual_border: Color::rgb(0xE3, 0xE6, 0xEB),
        accent: Color::rgb(0x2B, 0x6C, 0xE0),
        good: Color::rgb(0x16, 0xA3, 0x4A),
        neutral: Color::rgb(0xEA, 0xB3, 0x08),
        bad: Color::rgb(0xDC, 0x26, 0x26),
        font_family: "Inter".into(),
        title_size: 13.0,
        label_size: 10.0,
        corner_radius: 8.0,
        shadow: true,
    }
}

fn palette(h: &[&str]) -> Vec<Color> {
    h.iter().filter_map(|s| Color::parse(s)).collect()
}

impl Theme {
    pub const NAMES: &'static [&'static str] = &["VizCraft", "Classic", "Midnight", "Ocean", "Sunset", "Forest", "Monochrome", "High contrast", "Pastel", "Executive"];

    pub fn by_name(name: &str) -> Option<Theme> {
        let mut t = base();
        t.name = name.to_string();
        match name {
            "VizCraft" => {}
            "Classic" => {
                t.data_colors = palette(&["#118DFF", "#12239E", "#E66C37", "#6B007B", "#E044A7", "#744EC2", "#D9B300", "#D64550", "#197278", "#1AAB40"]);
                t.corner_radius = 0.0;
                t.shadow = false;
                t.background = Color::WHITE;
            }
            "Midnight" => {
                t.data_colors = palette(&["#60A5FA", "#34D399", "#FBBF24", "#F472B6", "#A78BFA", "#F87171", "#22D3EE", "#A3E635"]);
                t.background = Color::rgb(0x0F, 0x14, 0x1F);
                t.visual_background = Color::rgb(0x17, 0x1E, 0x2E);
                t.visual_border = Color::rgb(0x26, 0x30, 0x45);
                t.foreground = Color::rgb(0xE5, 0xE9, 0xF0);
                t.foreground_muted = Color::rgb(0x94, 0xA3, 0xB8);
                t.accent = Color::rgb(0x60, 0xA5, 0xFA);
            }
            "Ocean" => t.data_colors = palette(&["#0B4F6C", "#01BAEF", "#20BF55", "#0E7C7B", "#17BEBB", "#4C6085", "#39A0ED", "#13505B"]),
            "Sunset" => t.data_colors = palette(&["#FF6B6B", "#F7B267", "#F79D65", "#F4845F", "#F27059", "#C44569", "#6C5B7B", "#355C7D"]),
            "Forest" => t.data_colors = palette(&["#2D6A4F", "#40916C", "#52B788", "#74C69D", "#95D5B2", "#1B4332", "#B7E4C7", "#081C15"]),
            "Monochrome" => t.data_colors = palette(&["#1F2937", "#374151", "#4B5563", "#6B7280", "#9CA3AF", "#D1D5DB"]),
            "High contrast" => {
                t.data_colors = palette(&["#FFFF00", "#00FFFF", "#FF00FF", "#00FF00", "#FF8000", "#FFFFFF"]);
                t.background = Color::BLACK;
                t.visual_background = Color::BLACK;
                t.visual_border = Color::WHITE;
                t.foreground = Color::WHITE;
                t.foreground_muted = Color::rgb(0xDD, 0xDD, 0xDD);
            }
            "Pastel" => t.data_colors = palette(&["#A0C4FF", "#FFADAD", "#CAFFBF", "#FFD6A5", "#BDB2FF", "#9BF6FF", "#FDFFB6", "#FFC6FF"]),
            "Executive" => {
                t.data_colors = palette(&["#1E3A5F", "#C9A227", "#5B7DB1", "#8C8C8C", "#2E8B57", "#B22222"]);
                t.corner_radius = 2.0;
            }
            _ => return None,
        }
        Some(t)
    }

    pub fn series_color(&self, i: usize) -> Color {
        if self.data_colors.is_empty() {
            return self.accent;
        }
        let n = self.data_colors.len();
        let c = self.data_colors.get(i % n).copied().unwrap_or(self.accent);
        // Past the palette, cycle with lighter shades.
        let round = i / n;
        if round == 0 { c } else { c.shade(0.25 * round as f64).with_alpha(255) }
    }
}
