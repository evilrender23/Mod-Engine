//! GPU textures for sprites, plus helpers to draw a sprite frame with egui.

use crate::project::Project;
use eframe::egui::{self, Color32, Painter, Pos2, Rect, TextureHandle, TextureOptions, Vec2};
use modengine::project::Sprite;
use std::collections::HashMap;

#[derive(Default)]
pub struct TexCache {
    map: HashMap<String, (u64, TextureHandle)>,
}

pub struct SpriteTex<'a> {
    pub id: egui::TextureId,
    pub spr: &'a Sprite,
}

impl SpriteTex<'_> {
    pub fn uv(&self, frame: usize) -> Rect {
        let f = self.spr.frames[frame % self.spr.frames.len().max(1)];
        let (w, h) = (self.spr.sheet_w as f32, self.spr.sheet_h as f32);
        Rect::from_min_max(
            Pos2::new(f[0] as f32 / w, f[1] as f32 / h),
            Pos2::new((f[0] + f[2]) as f32 / w, (f[1] + f[3]) as f32 / h),
        )
    }

    pub fn frame_size(&self) -> Vec2 {
        Vec2::new(self.spr.fw as f32, self.spr.fh as f32)
    }

    pub fn origin(&self) -> Vec2 {
        Vec2::new(self.spr.origin.0 as f32, self.spr.origin.1 as f32)
    }

    /// Draws `frame` so that the sprite origin lands on `at`, scaled by `zoom`.
    pub fn draw(&self, p: &Painter, frame: usize, at: Pos2, zoom: f32, tint: Color32) {
        let min = at - self.origin() * zoom;
        p.image(self.id, Rect::from_min_size(min, self.frame_size() * zoom), self.uv(frame), tint);
    }

    /// Draws `frame` fitted inside `rect`, keeping proportions.
    pub fn draw_fit(&self, p: &Painter, frame: usize, rect: Rect) {
        let s = self.frame_size();
        let k = (rect.width() / s.x).min(rect.height() / s.y);
        let size = s * k;
        p.image(self.id, Rect::from_center_size(rect.center(), size), self.uv(frame), Color32::WHITE);
    }
}

impl TexCache {
    pub fn get<'a>(&mut self, ctx: &egui::Context, project: &'a Project, name: &str) -> Option<SpriteTex<'a>> {
        let entry = project.sprite_entry(name)?;
        let spr = entry.data.as_ref().ok()?;
        let fresh = matches!(self.map.get(name), Some((g, _)) if *g == entry.gen);
        if !fresh {
            let img = egui::ColorImage::from_rgba_unmultiplied([spr.sheet_w as usize, spr.sheet_h as usize], &spr.rgba);
            let tex = ctx.load_texture(format!("sprite:{name}"), img, TextureOptions::NEAREST);
            self.map.insert(name.to_string(), (entry.gen, tex));
        }
        let id = self.map[name].1.id();
        Some(SpriteTex { id, spr })
    }
}

pub fn checkerboard(p: &Painter, rect: Rect, cell: f32) {
    let pal = crate::theme::colors(p.ctx());
    p.rect_filled(rect, 0.0, pal.checker_a);
    let cell = cell.max(4.0);
    let cols = (rect.width() / cell).ceil() as i32;
    let rows = (rect.height() / cell).ceil() as i32;
    if cols * rows > 40_000 {
        return;
    }
    for r in 0..rows {
        for c in 0..cols {
            if (r + c) % 2 == 0 {
                let min = rect.min + Vec2::new(c as f32 * cell, r as f32 * cell);
                let cr = Rect::from_min_size(min, Vec2::splat(cell)).intersect(rect);
                p.rect_filled(cr, 0.0, pal.checker_b);
            }
        }
    }
}

pub fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}
