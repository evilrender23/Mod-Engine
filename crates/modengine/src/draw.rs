//! Renderer-agnostic draw list. The engine records commands during the draw
//! events; a backend (macroquad, headless...) replays them.

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    /// `c` is 0xRRGGBB.
    pub fn from_rgb(c: u32, alpha: f64) -> Self {
        Rgba {
            r: ((c >> 16) & 255) as f32 / 255.0,
            g: ((c >> 8) & 255) as f32 / 255.0,
            b: (c & 255) as f32 / 255.0,
            a: alpha.clamp(0.0, 1.0) as f32,
        }
    }
}

#[derive(Clone, Debug)]
pub enum DrawCmd {
    /// Following commands are in room coordinates seen through this camera rectangle.
    View { x: f32, y: f32, w: f32, h: f32 },
    /// Following commands are in GUI coordinates (0,0 = top-left of the game screen).
    Gui,
    Clear(Rgba),
    Sprite { spr: u32, frame: u32, x: f32, y: f32, sx: f32, sy: f32, angle: f32, color: Rgba },
    Stretched { spr: u32, frame: u32, x: f32, y: f32, w: f32, h: f32, color: Rgba },
    Rect { x: f32, y: f32, w: f32, h: f32, color: Rgba, outline: bool },
    Circle { x: f32, y: f32, r: f32, color: Rgba, outline: bool },
    Line { x1: f32, y1: f32, x2: f32, y2: f32, width: f32, color: Rgba },
    Tri { pts: [f32; 6], color: Rgba, outline: bool },
    Text { x: f32, y: f32, text: String, font: Option<u32>, scale: f32, color: Rgba, halign: u8, valign: u8 },
    /// Following commands use this shader (None = the default one). See `shader.rs`.
    Shader(Option<u32>),
    /// Sets uniform `name` of a shader (floats, or ints for int uniforms).
    Uniform { shader: u32, name: String, vals: Vec<f32> },
    /// Binds sprite `spr`'s texture to sampler uniform `name` of a shader.
    Sampler { shader: u32, name: String, spr: u32 },
    /// Blend mode of following commands: 0 normal, 1 add, 2 subtract, 3 max.
    Blend(u8),
}

/// Current drawing settings (`set_color`, `set_alpha`, `set_font`, `set_align`).
#[derive(Clone, Debug)]
pub struct DrawState {
    pub color: u32,
    pub alpha: f64,
    pub font: Option<u32>,
    pub halign: u8,
    pub valign: u8,
}

impl Default for DrawState {
    fn default() -> Self {
        DrawState { color: 0xFFFFFF, alpha: 1.0, font: None, halign: 0, valign: 0 }
    }
}

impl DrawState {
    pub fn rgba(&self) -> Rgba {
        Rgba::from_rgb(self.color, self.alpha)
    }
}

/// Parses "#rrggbb" / "rrggbb" / "#rgb" / named colours.
pub fn parse_color(s: &str) -> Option<u32> {
    let t = s.trim().trim_start_matches('#');
    if let Some(c) = named_color(t) {
        return Some(c);
    }
    match t.len() {
        6 => u32::from_str_radix(t, 16).ok(),
        3 => {
            let v = u32::from_str_radix(t, 16).ok()?;
            let (r, g, b) = ((v >> 8) & 15, (v >> 4) & 15, v & 15);
            Some((r * 17) << 16 | (g * 17) << 8 | (b * 17))
        }
        _ => None,
    }
}

pub const NAMED_COLORS: &[(&str, u32)] = &[
    ("white", 0xFFFFFF),
    ("black", 0x000000),
    ("red", 0xE53935),
    ("green", 0x43A047),
    ("blue", 0x1E88E5),
    ("yellow", 0xFDD835),
    ("orange", 0xFB8C00),
    ("purple", 0x8E24AA),
    ("pink", 0xF06292),
    ("cyan", 0x00ACC1),
    ("magenta", 0xD81B60),
    ("brown", 0x795548),
    ("lime", 0xC0CA33),
    ("navy", 0x1A237E),
    ("teal", 0x00897B),
    ("gray", 0x808080),
    ("light_gray", 0xC0C0C0),
    ("dark_gray", 0x404040),
    ("silver", 0xBDBDBD),
    ("gold", 0xFFC107),
];

fn named_color(s: &str) -> Option<u32> {
    NAMED_COLORS.iter().find(|(n, _)| *n == s).map(|(_, c)| *c)
}
