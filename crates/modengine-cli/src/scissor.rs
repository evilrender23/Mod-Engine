//! `gpu_set_scissor` on macroquad: the engine's clip rectangle (room or GUI coordinates)
//! becomes a scissor rectangle in pixels of the current render target. Also the system
//! mouse cursor (`window_set_cursor`, hidden while a `cursor_sprite` is drawn).

use crate::canvas::Canvas;
use macroquad::miniquad::{window, CursorIcon};
use macroquad::prelude::*;
use modengine::Engine;

/// Pixel rectangle (x, y from the top, w, h) of `r` inside a target of `px` pixels showing
/// the `view` rectangle. Rounded outwards to whole pixels, clamped to the target.
pub fn to_pixels(r: [f32; 4], view: (f32, f32, f32, f32), px: (f32, f32)) -> (i32, i32, i32, i32) {
    let (sx, sy) = (px.0 / view.2.max(1e-6), px.1 / view.3.max(1e-6));
    let x1 = ((r[0] - view.0) * sx).round().clamp(0.0, px.0);
    let y1 = ((r[1] - view.1) * sy).round().clamp(0.0, px.1);
    let x2 = ((r[0] + r[2] - view.0) * sx).round().clamp(0.0, px.0);
    let y2 = ((r[1] + r[3] - view.1) * sy).round().clamp(0.0, px.1);
    (x1 as i32, y1 as i32, (x2 - x1).max(0.0) as i32, (y2 - y1).max(0.0) as i32)
}

/// Sets (or clears) the scissor for the following draws.
pub fn set(r: Option<[f32; 4]>, cv: &Canvas) {
    let clip = r.map(|r| {
        let (view, px) = cv.clip_space();
        let (x, y, w, h) = to_pixels(r, view, px);
        // render targets keep the image's top row at GL y = 0, and macroquad flips the
        // scissor's y for the window's bottom-up framebuffer: undo that flip
        (x, px.1 as i32 - y - h, w, h)
    });
    unsafe { get_internal_gl() }.quad_gl.scissor(clip);
}

/// Applies `window_set_cursor` / `cursor_sprite` (only when they change).
pub struct CursorState {
    last: Option<(bool, i32)>,
}

impl CursorState {
    pub fn new() -> Self {
        CursorState { last: None }
    }

    pub fn update(&mut self, e: &Engine) {
        let now = (e.ui.system_cursor_visible(), e.ui.cursor);
        if self.last == Some(now) {
            return;
        }
        self.last = Some(now);
        window::show_mouse(now.0);
        if now.0 {
            window::set_mouse_cursor(icon(now.1));
        }
    }
}

impl Default for CursorState {
    fn default() -> Self {
        Self::new()
    }
}

/// GameMaker cursor (`cr_*`) → the closest system cursor.
fn icon(cr: i32) -> CursorIcon {
    match cr {
        -3 => CursorIcon::Crosshair,
        -4 => CursorIcon::Text,
        -6 => CursorIcon::NESWResize,
        -7 => CursorIcon::NSResize,
        -8 => CursorIcon::NWSEResize,
        -9 => CursorIcon::EWResize,
        -11 | -19 => CursorIcon::Wait,
        -12 | -22 => CursorIcon::Move,
        -21 => CursorIcon::Pointer,
        _ => CursorIcon::Default,
    }
}

#[cfg(test)]
mod tests {
    use super::to_pixels;

    #[test]
    fn room_rect_to_target_pixels() {
        // a 320×180 view at (100, 50) drawn into a 640×360 target: 2 px per unit
        assert_eq!(to_pixels([110.0, 60.0, 50.0, 20.0], (100.0, 50.0, 320.0, 180.0), (640.0, 360.0)), (20, 20, 100, 40));
        // clamped to the target
        assert_eq!(to_pixels([-10.0, -10.0, 30.0, 30.0], (0.0, 0.0, 100.0, 100.0), (100.0, 100.0)), (0, 0, 20, 20));
    }
}
