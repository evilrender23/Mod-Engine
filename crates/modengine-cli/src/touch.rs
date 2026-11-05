//! On-screen virtual gamepad for touch screens (`[android]` in game.toml):
//!
//! ```toml
//! [android]
//! virtual_pad = "arrows"                  # "none", "arrows" (left/right) or "dpad"
//! virtual_buttons = ["space:JUMP", "z"]   # key[:label], bottom right
//! ```
//!
//! Every touch inside a button holds its key, exactly like the keyboard would.
//! Layout and hit testing are plain math (tested); drawing uses macroquad.

use modengine::input::key_index;
use modengine::project::AndroidSection;

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Arrow(f32),
    Round(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct VButton {
    /// Index in `KEY_NAMES`.
    pub key: usize,
    pub shape: Shape,
    /// Center and radius in window pixels (set by `layout`).
    pub x: f32,
    pub y: f32,
    pub r: f32,
}

pub struct VirtualPad {
    pub buttons: Vec<VButton>,
    /// Keys held by touches this frame / last frame.
    pub held: Vec<bool>,
}

fn label_for(key: &str) -> String {
    match key {
        "space" => "A".into(),
        "enter" => "OK".into(),
        "escape" => "II".into(),
        k => k.to_uppercase().chars().take(3).collect(),
    }
}

impl VirtualPad {
    /// `None` when the game has no virtual controls configured.
    pub fn from_config(a: &AndroidSection) -> Option<VirtualPad> {
        let mut buttons = vec![];
        let mut arrow = |name: &str, angle: f32| {
            if let Some(k) = key_index(name) {
                buttons.push(VButton { key: k, shape: Shape::Arrow(angle), x: 0.0, y: 0.0, r: 0.0 });
            }
        };
        match a.virtual_pad.as_str() {
            "arrows" => {
                arrow("left", 180.0);
                arrow("right", 0.0);
            }
            "dpad" => {
                arrow("left", 180.0);
                arrow("right", 0.0);
                arrow("up", 90.0);
                arrow("down", 270.0);
            }
            _ => {}
        }
        for b in &a.virtual_buttons {
            let (key, label) = match b.split_once(':') {
                Some((k, l)) => (k.trim(), l.trim().to_string()),
                None => (b.trim(), label_for(b.trim())),
            };
            if let Some(k) = key_index(key) {
                buttons.push(VButton { key: k, shape: Shape::Round(label), x: 0.0, y: 0.0, r: 0.0 });
            }
        }
        if buttons.is_empty() {
            return None;
        }
        Some(VirtualPad { buttons, held: vec![false; modengine::input::KEY_NAMES.len()] })
    }

    /// Places the buttons for a window of `w`×`h` pixels: arrows bottom left, buttons
    /// bottom right.
    pub fn layout(&mut self, w: f32, h: f32) {
        let u = (w.min(h) * 0.085).max(24.0);
        let dpad = self.buttons.iter().filter(|b| matches!(b.shape, Shape::Arrow(_))).count() > 2;
        let mut round = 0;
        for b in &mut self.buttons {
            b.r = u;
            (b.x, b.y) = match &b.shape {
                Shape::Arrow(a) if dpad => {
                    let (cx, cy) = (u * 3.2, h - u * 3.2);
                    let rad = a.to_radians();
                    (cx + rad.cos() * u * 2.0, cy - rad.sin() * u * 2.0)
                }
                Shape::Arrow(a) => (if *a > 90.0 { u * 1.6 } else { u * 4.2 }, h - u * 1.8),
                Shape::Round(_) => {
                    let i = round as f32;
                    round += 1;
                    (w - u * 1.6 - i * u * 2.5, h - u * 1.8 - (i % 2.0) * u * 1.4)
                }
            };
        }
    }

    /// Which keys the touch points (window pixels) hold. Returns (pressed, released)
    /// edges since the last call; `held` keeps the current state.
    pub fn update(&mut self, points: &[(f32, f32)]) -> (Vec<usize>, Vec<usize>) {
        let mut now = vec![false; self.held.len()];
        for b in &self.buttons {
            // a little larger than drawn: thumbs are not precise
            if points.iter().any(|(x, y)| (x - b.x).powi(2) + (y - b.y).powi(2) <= (b.r * 1.3).powi(2)) {
                now[b.key] = true;
            }
        }
        let pressed = (0..now.len()).filter(|&i| now[i] && !self.held[i]).collect();
        let released = (0..now.len()).filter(|&i| !now[i] && self.held[i]).collect();
        self.held = now;
        (pressed, released)
    }

    pub fn draw(&self) {
        use macroquad::prelude::*;
        for b in &self.buttons {
            let on = self.held[b.key];
            let fill = Color::new(1.0, 1.0, 1.0, if on { 0.45 } else { 0.18 });
            let line = Color::new(1.0, 1.0, 1.0, 0.6);
            draw_circle(b.x, b.y, b.r, fill);
            draw_circle_lines(b.x, b.y, b.r, 2.0, line);
            match &b.shape {
                Shape::Arrow(a) => {
                    let rad = a.to_radians();
                    let (dx, dy) = (rad.cos(), -rad.sin());
                    let s = b.r * 0.45;
                    let tip = vec2(b.x + dx * s, b.y + dy * s);
                    let back = vec2(b.x - dx * s * 0.6, b.y - dy * s * 0.6);
                    let (px, py) = (-dy * s * 0.8, dx * s * 0.8);
                    draw_triangle(tip, vec2(back.x + px, back.y + py), vec2(back.x - px, back.y - py), line);
                }
                Shape::Round(label) => {
                    let size = (b.r * 0.8).round() as u16;
                    let m = measure_text(label, None, size, 1.0);
                    draw_text(label, b.x - m.width / 2.0, b.y + m.offset_y / 2.0, size as f32, line);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(pad: &str, buttons: &[&str]) -> AndroidSection {
        AndroidSection { virtual_pad: pad.into(), virtual_buttons: buttons.iter().map(|s| s.to_string()).collect(), ..Default::default() }
    }

    #[test]
    fn nothing_configured_means_no_pad() {
        assert!(VirtualPad::from_config(&AndroidSection::default()).is_none());
        assert!(VirtualPad::from_config(&cfg("none", &["not_a_key"])).is_none());
    }

    #[test]
    fn touches_hold_keys_with_edges() {
        let mut p = VirtualPad::from_config(&cfg("arrows", &["space:JUMP", "z"])).unwrap();
        assert_eq!(p.buttons.len(), 4);
        assert_eq!(p.buttons[2].shape, Shape::Round("JUMP".into()));
        assert_eq!(p.buttons[3].shape, Shape::Round("Z".into()));
        p.layout(1280.0, 720.0);
        let (left, right, jump) = (p.buttons[0].clone(), p.buttons[1].clone(), p.buttons[2].clone());
        assert!(left.x < right.x && right.x < 640.0 && jump.x > 640.0, "arrows left, buttons right");
        assert!(left.y > 360.0 && jump.y > 360.0, "bottom half");
        let k = |n| key_index(n).unwrap();
        let (pr, rel) = p.update(&[(left.x, left.y), (jump.x + 5.0, jump.y)]);
        assert_eq!((pr, rel), (vec![k("left"), k("space")], vec![]));
        assert!(p.held[k("left")] && p.held[k("space")] && !p.held[k("right")]);
        let (pr, rel) = p.update(&[(right.x, right.y)]);
        assert_eq!((pr, rel), (vec![k("right")], vec![k("left"), k("space")]));
        let (pr, rel) = p.update(&[(640.0, 100.0)]);
        assert_eq!((pr, rel), (vec![], vec![k("right")]));
    }

    #[test]
    fn dpad_has_four_directions_around_a_center() {
        let mut p = VirtualPad::from_config(&cfg("dpad", &[])).unwrap();
        p.layout(800.0, 480.0);
        let b = &p.buttons;
        assert_eq!(b.len(), 4);
        assert!(b[0].x < b[1].x && b[2].y < b[3].y);
        assert!((b[2].x - b[3].x).abs() < 0.01);
    }
}
