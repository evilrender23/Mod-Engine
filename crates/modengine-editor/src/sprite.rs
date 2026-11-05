//! Sprite editor: a pixel-art painter for text sprites, and slicing/origin/mask
//! settings for PNG sprites.

use crate::app::Cx;
use crate::tex::{checkerboard, rgb};
use crate::tomlw::{color_hex, num, parse_hex, q};
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, TextureHandle, TextureOptions, Vec2};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Deserialize, Default)]
#[serde(default)]
struct SpriteFile {
    image: Option<String>,
    frames: Option<u32>,
    frame_size: Option<[u32; 2]>,
    origin: Option<toml::Value>,
    fps: Option<f64>,
    mask: Option<[i32; 4]>,
    palette: BTreeMap<String, String>,
    pixels: Vec<String>,
}

#[derive(Clone, PartialEq, Debug)]
enum Origin {
    TopLeft,
    Named(String),
    At(f64, f64),
}

#[derive(Clone, PartialEq)]
struct Model {
    image: Option<String>,
    frames: Option<u32>,
    frame_size: Option<[u32; 2]>,
    origin: Origin,
    fps: Option<f64>,
    mask: Option<[i32; 4]>,
    palette: BTreeMap<char, [u8; 3]>,
    pix: Vec<Vec<Vec<char>>>,
    w: usize,
    h: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Pencil,
    Eraser,
    Fill,
    Pick,
    Origin,
}

pub struct SpriteEditor {
    pub name: String,
    path: PathBuf,
    is_toml: bool,
    m: Model,
    undo: Vec<Model>,
    redo: Vec<Model>,
    pub dirty: bool,
    frame: usize,
    tool: Tool,
    color: char,
    zoom: f32,
    onion: bool,
    grid: bool,
    play: bool,
    tex: Option<TextureHandle>,
    tex_ver: u64,
    ver: u64,
    stroke: Option<Model>,
    new_size: [usize; 2],
    add_color: [u8; 3],
    /// Screen rectangle of the image in the last frame.
    pub(crate) image_rect: Rect,
}

const CHAR_POOL: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789#@$%&*+=-~!?<>^/|:;";

impl Model {
    fn is_pixel(&self) -> bool {
        self.image.is_none()
    }

    fn origin_xy(&self) -> (f64, f64) {
        let (w, h) = (self.w as f64, self.h as f64);
        match &self.origin {
            Origin::TopLeft => (0.0, 0.0),
            Origin::At(x, y) => (*x, *y),
            Origin::Named(n) => match n.as_str() {
                "top" => (w / 2.0, 0.0),
                "center" | "middle" => (w / 2.0, h / 2.0),
                "bottom" => (w / 2.0, h),
                "left" => (0.0, h / 2.0),
                _ => (0.0, 0.0),
            },
        }
    }
}

fn parse_origin(v: &Option<toml::Value>) -> Origin {
    match v {
        Some(toml::Value::String(s)) => Origin::Named(s.clone()),
        Some(toml::Value::Array(a)) if a.len() == 2 => {
            let n = |v: &toml::Value| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)).unwrap_or(0.0);
            Origin::At(n(&a[0]), n(&a[1]))
        }
        _ => Origin::TopLeft,
    }
}

fn write_toml(m: &Model) -> String {
    let mut s = String::new();
    if let Some(img) = &m.image {
        s += &format!("image = {}\n", q(img));
        if let Some([w, h]) = m.frame_size {
            s += &format!("frame_size = [{w}, {h}]\n");
        } else if let Some(n) = m.frames.filter(|n| *n > 1) {
            s += &format!("frames = {n}\n");
        }
    }
    match &m.origin {
        Origin::TopLeft => {}
        Origin::Named(n) => s += &format!("origin = {}\n", q(n)),
        Origin::At(x, y) => s += &format!("origin = [{}, {}]\n", num(*x), num(*y)),
    }
    if let Some(f) = m.fps {
        s += &format!("fps = {}\n", num(f));
    }
    if let Some([l, t, r, b]) = m.mask {
        s += &format!("mask = [{l}, {t}, {r}, {b}]\n");
    }
    if m.is_pixel() {
        let used: std::collections::BTreeSet<char> = m.pix.iter().flatten().flatten().copied().collect();
        let local: Vec<String> = m.palette.iter().filter(|(c, _)| used.contains(c)).map(|(c, v)| format!("{} = {}", q(&c.to_string()), q(&color_hex(*v)))).collect();
        if !local.is_empty() {
            s += &format!("palette = {{ {} }}\n", local.join(", "));
        }
        let frames: Vec<String> = m.pix.iter().map(|f| format!("'''\n{}\n'''", f.iter().map(|r| r.iter().collect::<String>()).collect::<Vec<_>>().join("\n"))).collect();
        s += &format!("pixels = [{}]\n", frames.join(", "));
    }
    s
}

impl SpriteEditor {
    pub fn open(name: &str, path: &Path, is_toml: bool) -> Result<Self, String> {
        let f: SpriteFile = if is_toml {
            let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?
        } else {
            SpriteFile { image: path.file_name().map(|n| n.to_string_lossy().to_string()), ..Default::default() }
        };
        let mut palette = BTreeMap::new();
        for (k, v) in &f.palette {
            if let (Some(c), Some(col)) = (k.chars().next(), parse_hex(v)) {
                palette.insert(c, col);
            }
        }
        let mut pix: Vec<Vec<Vec<char>>> = f
            .pixels
            .iter()
            .map(|fr| fr.lines().map(|l| l.trim().chars().map(|c| if c == ' ' { '.' } else { c }).collect::<Vec<char>>()).filter(|l| !l.is_empty()).collect())
            .collect();
        let w = pix.iter().flat_map(|f| f.iter().map(|r| r.len())).max().unwrap_or(0);
        let h = pix.iter().map(|f| f.len()).max().unwrap_or(0);
        for fr in &mut pix {
            fr.resize(h, vec![]);
            for r in fr.iter_mut() {
                r.resize(w, '.');
            }
        }
        let (w, h) = if f.image.is_some() { (0, 0) } else { (w.max(1), h.max(1)) };
        if f.image.is_none() && pix.is_empty() {
            pix.push(vec![vec!['.'; w]; h]);
        }
        let m = Model { image: f.image, frames: f.frames, frame_size: f.frame_size, origin: parse_origin(&f.origin), fps: f.fps, mask: f.mask, palette, pix, w, h };
        Ok(SpriteEditor {
            name: name.into(),
            path: path.into(),
            is_toml,
            new_size: [m.w, m.h],
            m,
            undo: vec![],
            redo: vec![],
            dirty: false,
            frame: 0,
            tool: Tool::Pencil,
            color: '.',
            zoom: 0.0,
            onion: false,
            grid: true,
            play: true,
            tex: None,
            tex_ver: 0,
            ver: 1,
            stroke: None,
            add_color: [255, 255, 255],
            image_rect: Rect::NOTHING,
        })
    }

    pub fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.m, prev));
            self.after_change();
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.m, next));
            self.after_change();
        }
    }

    fn after_change(&mut self) {
        self.ver += 1;
        self.dirty = true;
        self.frame = self.frame.min(self.m.pix.len().saturating_sub(1));
    }

    fn snapshot(&mut self) {
        self.undo.push(self.m.clone());
        self.redo.clear();
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
    }

    pub fn save(&mut self, cx: &mut Cx) -> Result<(), String> {
        let path = if self.is_toml { self.path.clone() } else { self.path.with_extension("toml") };
        std::fs::write(&path, write_toml(&self.m)).map_err(|e| format!("{}: {e}", path.display()))?;
        self.path = path;
        self.is_toml = true;
        self.dirty = false;
        cx.project.reload_sprites();
        Ok(())
    }

    fn colors(&self, cx: &Cx) -> BTreeMap<char, [u8; 3]> {
        let mut all = BTreeMap::new();
        for (k, v) in &cx.project.game.palette {
            if let (Some(c), Some(col)) = (k.chars().next(), parse_hex(v)) {
                all.insert(c, col);
            }
        }
        all.extend(self.m.palette.iter().map(|(k, v)| (*k, *v)));
        all
    }

    fn rebuild_texture(&mut self, ctx: &egui::Context, colors: &BTreeMap<char, [u8; 3]>) {
        if self.tex.is_some() && self.tex_ver == self.ver {
            return;
        }
        let (w, h, n) = (self.m.w, self.m.h, self.m.pix.len().max(1));
        let mut rgba = vec![0u8; w * n * h * 4];
        for (fi, fr) in self.m.pix.iter().enumerate() {
            for (y, row) in fr.iter().enumerate() {
                for (x, c) in row.iter().enumerate() {
                    if let Some(col) = colors.get(c) {
                        let i = (y * w * n + fi * w + x) * 4;
                        rgba[i..i + 4].copy_from_slice(&[col[0], col[1], col[2], 255]);
                    } else if *c != '.' {
                        let i = (y * w * n + fi * w + x) * 4;
                        rgba[i..i + 4].copy_from_slice(&[255, 0, 255, 255]);
                    }
                }
            }
        }
        let img = egui::ColorImage::from_rgba_unmultiplied([w * n, h], &rgba);
        self.tex = Some(ctx.load_texture(format!("pixedit:{}", self.name), img, TextureOptions::NEAREST));
        self.tex_ver = self.ver;
    }

    fn frame_uv(&self, f: usize) -> Rect {
        let n = self.m.pix.len().max(1) as f32;
        Rect::from_min_max(Pos2::new(f as f32 / n, 0.0), Pos2::new((f + 1) as f32 / n, 1.0))
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        if !self.m.is_pixel() {
            return self.png_ui(ui, cx);
        }
        let colors = self.colors(cx);
        self.rebuild_texture(ui.ctx(), &colors);

        egui::Panel::left("sprite_side").resizable(true).default_size(250.0).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.side(ui, &colors));
        });
        egui::Panel::bottom("sprite_frames").resizable(false).exact_size(92.0).show(ui, |ui| self.frames_strip(ui));
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                for (t, label, tip) in [
                    (Tool::Pencil, "✏ Lápiz", "pinta con el color elegido (clic derecho borra)"),
                    (Tool::Eraser, "⌫ Borrar", "deja el píxel transparente"),
                    (Tool::Fill, "🪣 Rellenar", "rellena la zona del mismo color"),
                    (Tool::Pick, "💧 Cuentagotas", "toma el color de un píxel"),
                    (Tool::Origin, "✛ Origen", "fija el punto de origen del sprite"),
                ] {
                    ui.selectable_value(&mut self.tool, t, label).on_hover_text(tip);
                }
                ui.separator();
                ui.checkbox(&mut self.grid, "rejilla");
                ui.checkbox(&mut self.onion, "papel cebolla");
                if ui.button("↔").on_hover_text("voltear horizontal").clicked() {
                    self.snapshot();
                    for r in &mut self.m.pix[self.frame] {
                        r.reverse();
                    }
                    self.after_change();
                }
                if ui.button("↕").on_hover_text("voltear vertical").clicked() {
                    self.snapshot();
                    self.m.pix[self.frame].reverse();
                    self.after_change();
                }
            });
            self.canvas(ui);
        });
    }

    fn side(&mut self, ui: &mut egui::Ui, colors: &BTreeMap<char, [u8; 3]>) {
        ui.heading(&self.name);
        ui.label(RichText::new(format!("{}×{} px, {} fotograma(s)", self.m.w, self.m.h, self.m.pix.len())).weak());
        ui.add_space(6.0);

        // animated preview
        let (rect, _) = ui.allocate_exact_size(Vec2::new(120.0, 120.0), Sense::hover());
        checkerboard(ui.painter(), rect, 10.0);
        let fps = self.m.fps.unwrap_or(10.0);
        let n = self.m.pix.len().max(1);
        let f = if self.play && fps > 0.0 { ((ui.input(|i| i.time) * fps) as usize) % n } else { self.frame };
        if let Some(t) = &self.tex {
            let k = (rect.width() / self.m.w as f32).min(rect.height() / self.m.h as f32).floor().max(1.0);
            let size = Vec2::new(self.m.w as f32, self.m.h as f32) * k;
            ui.painter().image(t.id(), Rect::from_center_size(rect.center(), size), self.frame_uv(f), Color32::WHITE);
        }
        if self.play && n > 1 {
            ui.ctx().request_repaint();
        }
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.play, "animar");
            ui.label("fps");
            let mut v = fps;
            if ui.add(egui::DragValue::new(&mut v).range(0.0..=60.0)).changed() {
                self.snapshot();
                self.m.fps = Some(v);
                self.after_change();
            }
        });

        ui.separator();
        ui.strong("Colores");
        ui.horizontal_wrapped(|ui| {
            let mut swatch = |ui: &mut egui::Ui, c: char, col: Option<[u8; 3]>| {
                let (r, resp) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
                match col {
                    Some(col) => {
                        ui.painter().rect_filled(r, 3.0, rgb(col));
                    }
                    None => checkerboard(ui.painter(), r, 6.0),
                }
                if self.color == c {
                    ui.painter().rect_stroke(r, 3.0, Stroke::new(2.0, Color32::WHITE), StrokeKind::Outside);
                }
                if resp.on_hover_text(format!("'{c}'")).clicked() {
                    self.color = c;
                    if matches!(self.tool, Tool::Eraser | Tool::Pick | Tool::Origin) {
                        self.tool = Tool::Pencil;
                    }
                }
            };
            swatch(ui, '.', None);
            for (c, col) in colors {
                swatch(ui, *c, Some(*col));
            }
        });
        ui.horizontal(|ui| {
            ui.color_edit_button_srgb(&mut self.add_color);
            if ui.button("Añadir color al sprite").clicked() {
                if let Some(c) = CHAR_POOL.chars().find(|c| !colors.contains_key(c)) {
                    self.snapshot();
                    self.m.palette.insert(c, self.add_color);
                    self.color = c;
                    self.after_change();
                }
            }
        });
        if let Some(col) = self.m.palette.get(&self.color).copied() {
            let mut c = col;
            ui.horizontal(|ui| {
                ui.label(format!("'{}' (de este sprite):", self.color));
                if ui.color_edit_button_srgb(&mut c).changed() {
                    self.m.palette.insert(self.color, c);
                    self.after_change();
                }
            });
        }

        ui.separator();
        ui.strong("Tamaño");
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut self.new_size[0]).range(1..=256));
            ui.label("×");
            ui.add(egui::DragValue::new(&mut self.new_size[1]).range(1..=256));
            if ui.add_enabled(self.new_size != [self.m.w, self.m.h], egui::Button::new("Aplicar")).clicked() {
                self.snapshot();
                let [w, h] = self.new_size;
                for f in &mut self.m.pix {
                    f.resize(h, vec!['.'; w]);
                    for r in f.iter_mut() {
                        r.resize(w, '.');
                    }
                }
                self.m.w = w;
                self.m.h = h;
                self.after_change();
            }
        });

        ui.separator();
        self.origin_mask_ui(ui);
    }

    fn origin_mask_ui(&mut self, ui: &mut egui::Ui) {
        ui.strong("Origen");
        ui.label(RichText::new("Punto que coincide con x, y de la instancia (y alrededor del que rota).").weak());
        let before = self.m.clone();
        ui.horizontal_wrapped(|ui| {
            for (label, o) in [
                ("arriba-izq.", Origin::TopLeft),
                ("centro", Origin::Named("center".into())),
                ("abajo", Origin::Named("bottom".into())),
                ("arriba", Origin::Named("top".into())),
            ] {
                if ui.selectable_label(self.m.origin == o, label).clicked() {
                    self.m.origin = o;
                }
            }
        });
        let (mut ox, mut oy) = self.m.origin_xy();
        ui.horizontal(|ui| {
            ui.label("x");
            let a = ui.add(egui::DragValue::new(&mut ox).speed(0.5));
            ui.label("y");
            let b = ui.add(egui::DragValue::new(&mut oy).speed(0.5));
            if a.changed() || b.changed() {
                self.m.origin = Origin::At(ox, oy);
            }
        });
        ui.add_space(6.0);
        ui.strong("Máscara de colisión");
        let mut manual = self.m.mask.is_some();
        if ui.checkbox(&mut manual, "caja manual (si no, la de los píxeles visibles)").changed() {
            self.m.mask = if manual { Some([0, 0, self.m.w.max(1) as i32 - 1, self.m.h.max(1) as i32 - 1]) } else { None };
        }
        if let Some(mask) = &mut self.m.mask {
            ui.horizontal(|ui| {
                for (i, l) in ["izq", "arr", "der", "abj"].iter().enumerate() {
                    ui.label(*l);
                    ui.add(egui::DragValue::new(&mut mask[i]));
                }
            });
        }
        if self.m != before {
            self.undo.push(before);
            self.redo.clear();
            self.after_change();
        }
    }

    fn frames_strip(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("+ nuevo").clicked() {
                self.snapshot();
                self.m.pix.insert(self.frame + 1, vec![vec!['.'; self.m.w]; self.m.h]);
                self.frame += 1;
                self.after_change();
            }
            if ui.button("⧉ duplicar").clicked() {
                self.snapshot();
                let f = self.m.pix[self.frame].clone();
                self.m.pix.insert(self.frame + 1, f);
                self.frame += 1;
                self.after_change();
            }
            if ui.add_enabled(self.m.pix.len() > 1, egui::Button::new("🗑 eliminar")).clicked() {
                self.snapshot();
                self.m.pix.remove(self.frame);
                self.after_change();
            }
            if ui.add_enabled(self.frame > 0, egui::Button::new("◀")).clicked() {
                self.snapshot();
                self.m.pix.swap(self.frame, self.frame - 1);
                self.frame -= 1;
                self.after_change();
            }
            if ui.add_enabled(self.frame + 1 < self.m.pix.len(), egui::Button::new("▶")).clicked() {
                self.snapshot();
                self.m.pix.swap(self.frame, self.frame + 1);
                self.frame += 1;
                self.after_change();
            }
        });
        egui::ScrollArea::horizontal().show(ui, |ui| {
            ui.horizontal(|ui| {
                for f in 0..self.m.pix.len() {
                    let (r, resp) = ui.allocate_exact_size(Vec2::splat(52.0), Sense::click());
                    checkerboard(ui.painter(), r, 6.0);
                    if let Some(t) = &self.tex {
                        let k = (48.0 / self.m.w as f32).min(48.0 / self.m.h as f32);
                        ui.painter().image(t.id(), Rect::from_center_size(r.center(), Vec2::new(self.m.w as f32, self.m.h as f32) * k), self.frame_uv(f), Color32::WHITE);
                    }
                    let stroke = if f == self.frame { Stroke::new(2.0, crate::theme::colors(ui.ctx()).selection) } else { Stroke::new(1.0, crate::theme::colors(ui.ctx()).frame_border) };
                    ui.painter().rect_stroke(r, 2.0, stroke, StrokeKind::Inside);
                    ui.painter().text(r.left_top() + Vec2::new(3.0, 1.0), egui::Align2::LEFT_TOP, f.to_string(), egui::FontId::proportional(10.0), Color32::WHITE);
                    if resp.clicked() {
                        self.frame = f;
                        self.play = false;
                    }
                }
            });
        });
    }

    fn canvas(&mut self, ui: &mut egui::Ui) {
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let area = resp.rect;
        let (w, h) = (self.m.w as f32, self.m.h as f32);
        if self.zoom <= 0.0 {
            self.zoom = ((area.width() - 40.0) / w).min((area.height() - 40.0) / h).floor().clamp(1.0, 48.0);
        }
        if resp.hovered() {
            let factor = ui.input(|i| crate::zoom::factor(i.smooth_scroll_delta.y, i.zoom_delta()));
            if factor != 1.0 {
                self.zoom = (self.zoom * factor).clamp(1.0, 64.0);
            }
        }
        let z = self.zoom;
        let img = Rect::from_center_size(area.center(), Vec2::new(w * z, h * z));
        self.image_rect = img;
        checkerboard(&painter, img, z.max(8.0));
        if let Some(t) = &self.tex {
            if self.onion && self.frame > 0 {
                painter.image(t.id(), img, self.frame_uv(self.frame - 1), Color32::from_white_alpha(70));
            }
            painter.image(t.id(), img, self.frame_uv(self.frame), Color32::WHITE);
        }
        if self.grid && z >= 6.0 {
            let st = Stroke::new(1.0, Color32::from_black_alpha(60));
            for x in 0..=self.m.w {
                let px = img.left() + x as f32 * z;
                painter.line_segment([Pos2::new(px, img.top()), Pos2::new(px, img.bottom())], st);
            }
            for y in 0..=self.m.h {
                let py = img.top() + y as f32 * z;
                painter.line_segment([Pos2::new(img.left(), py), Pos2::new(img.right(), py)], st);
            }
        }
        painter.rect_stroke(img, 0.0, Stroke::new(1.0, crate::theme::colors(ui.ctx()).canvas_border), StrokeKind::Outside);
        if let Some([l, t, r, b]) = self.m.mask {
            let mr = Rect::from_min_max(img.min + Vec2::new(l as f32, t as f32) * z, img.min + Vec2::new((r + 1) as f32, (b + 1) as f32) * z);
            painter.rect_stroke(mr, 0.0, Stroke::new(1.5, Color32::from_rgb(255, 90, 90)), StrokeKind::Middle);
        }
        let (ox, oy) = self.m.origin_xy();
        let op = img.min + Vec2::new(ox as f32, oy as f32) * z;
        let cs = Stroke::new(1.5, crate::theme::colors(ui.ctx()).camera);
        painter.line_segment([op - Vec2::new(8.0, 0.0), op + Vec2::new(8.0, 0.0)], cs);
        painter.line_segment([op - Vec2::new(0.0, 8.0), op + Vec2::new(0.0, 8.0)], cs);

        // ---- painting
        let pointer = resp.hover_pos().or(resp.interact_pointer_pos());
        let cell = pointer.map(|p| ((p.x - img.left()) / z, (p.y - img.top()) / z));
        let (primary, secondary, pressed, released) = ui.input(|i| (i.pointer.primary_down(), i.pointer.secondary_down(), i.pointer.any_pressed(), i.pointer.any_released()));
        if let Some((fx, fy)) = cell {
            let (cx_, cy_) = (fx.floor() as i64, fy.floor() as i64);
            let inside = cx_ >= 0 && cy_ >= 0 && (cx_ as usize) < self.m.w && (cy_ as usize) < self.m.h;
            if inside && resp.hovered() {
                let r = Rect::from_min_size(img.min + Vec2::new(cx_ as f32, cy_ as f32) * z, Vec2::splat(z));
                painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Inside);
            }
            let active = resp.is_pointer_button_down_on() || resp.dragged();
            if pressed && resp.hovered() && self.stroke.is_none() {
                self.stroke = Some(self.m.clone());
            }
            if active && (primary || secondary) {
                let tool = if secondary { Tool::Eraser } else { self.tool };
                match tool {
                    Tool::Origin => {
                        self.m.origin = Origin::At((fx as f64).round(), (fy as f64).round());
                        self.ver += 1;
                    }
                    _ if !inside => {}
                    Tool::Pencil | Tool::Eraser => {
                        let c = if tool == Tool::Eraser { '.' } else { self.color };
                        let px = &mut self.m.pix[self.frame][cy_ as usize][cx_ as usize];
                        if *px != c {
                            *px = c;
                            self.ver += 1;
                        }
                    }
                    Tool::Fill => {
                        if flood(&mut self.m.pix[self.frame], cx_ as usize, cy_ as usize, self.color) {
                            self.ver += 1;
                        }
                    }
                    Tool::Pick => {
                        self.color = self.m.pix[self.frame][cy_ as usize][cx_ as usize];
                        self.tool = Tool::Pencil;
                    }
                }
            }
        }
        if released || !(primary || secondary) {
            if let Some(before) = self.stroke.take() {
                if before != self.m {
                    self.undo.push(before);
                    self.redo.clear();
                    self.dirty = true;
                }
            }
        }
        painter.text(area.left_bottom() + Vec2::new(6.0, -6.0), egui::Align2::LEFT_BOTTOM, format!("zoom {z}×  ·  rueda: zoom  ·  clic derecho: borrar"), egui::FontId::proportional(12.0), crate::theme::colors(ui.ctx()).canvas_text);
    }

    fn png_ui(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        let before = self.m.clone();
        ui.heading(&self.name);
        ui.label(format!("Imagen: {}", self.m.image.clone().unwrap_or_default()));
        ui.label(RichText::new("Los cambios de corte se ven al guardar (Ctrl+S).").weak());
        ui.add_space(6.0);
        let mut grid = self.m.frame_size.is_some();
        ui.horizontal(|ui| {
            ui.radio_value(&mut grid, false, "tira horizontal");
            ui.radio_value(&mut grid, true, "rejilla");
        });
        if grid {
            let mut fs = self.m.frame_size.unwrap_or([16, 16]);
            ui.horizontal(|ui| {
                ui.label("celda");
                ui.add(egui::DragValue::new(&mut fs[0]).range(1..=4096));
                ui.label("×");
                ui.add(egui::DragValue::new(&mut fs[1]).range(1..=4096));
            });
            self.m.frame_size = Some(fs);
        } else {
            self.m.frame_size = None;
            let mut n = self.m.frames.unwrap_or(1);
            ui.horizontal(|ui| {
                ui.label("fotogramas");
                ui.add(egui::DragValue::new(&mut n).range(1..=256));
            });
            self.m.frames = Some(n);
        }
        let mut fps = self.m.fps.unwrap_or(10.0);
        ui.horizontal(|ui| {
            ui.label("fps");
            ui.add(egui::DragValue::new(&mut fps).range(0.0..=60.0));
        });
        self.m.fps = Some(fps);
        if let Some(t) = cx.tex.get(ui.ctx(), cx.project, &self.name) {
            self.m.w = t.spr.fw as usize;
            self.m.h = t.spr.fh as usize;
        }
        self.origin_mask_ui(ui);
        ui.separator();
        if let Some(t) = cx.tex.get(ui.ctx(), cx.project, &self.name) {
            egui::ScrollArea::both().show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for f in 0..t.spr.frames.len() {
                        let size = t.frame_size() * 4.0_f32.min(160.0 / t.frame_size().max_elem());
                        let (r, _) = ui.allocate_exact_size(size, Sense::hover());
                        checkerboard(ui.painter(), r, 8.0);
                        ui.painter().image(t.id, r, t.uv(f), Color32::WHITE);
                        let k = r.width() / t.frame_size().x;
                        let o = t.origin();
                        ui.painter().circle_filled(r.min + o * k, 3.0, crate::theme::colors(ui.ctx()).camera);
                    }
                });
            });
        }
        if self.m != before {
            self.dirty = true;
        }
    }
}

fn flood(grid: &mut [Vec<char>], x: usize, y: usize, to: char) -> bool {
    let from = grid[y][x];
    if from == to {
        return false;
    }
    let mut stack = vec![(x, y)];
    while let Some((x, y)) = stack.pop() {
        if grid[y][x] != from {
            continue;
        }
        grid[y][x] = to;
        if x > 0 {
            stack.push((x - 1, y));
        }
        if y > 0 {
            stack.push((x, y - 1));
        }
        if x + 1 < grid[y].len() {
            stack.push((x + 1, y));
        }
        if y + 1 < grid.len() {
            stack.push((x, y + 1));
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{LogLine, Open};
    use crate::project::Project;
    use crate::tex::TexCache;
    use eframe::egui::{Event, Modifiers, PointerButton};
    use egui_kittest::Harness;

    struct St {
        ed: SpriteEditor,
        project: Project,
        tex: TexCache,
        log: Vec<LogLine>,
        open: Vec<Open>,
    }

    #[test]
    fn pencil_and_undo() {
        let dir = crate::room::tests::temp_project("sprite");
        let project = Project::open(&dir).unwrap();
        let e = project.sprite_entry("heart").unwrap();
        let ed = SpriteEditor::open("heart", &e.path, e.is_toml).unwrap();
        let st = St { ed, project, tex: TexCache::default(), log: vec![], open: vec![] };
        let mut h = Harness::builder().with_size([1200.0, 800.0]).allow_missing_glyphs().with_accessibility_check(false).build_ui_state(
            |ui, s: &mut St| {
                let mut cx = Cx { project: &mut s.project, tex: &mut s.tex, log: &mut s.log, open: &mut s.open, active: true };
                s.ed.ui(ui, &mut cx);
            },
            st,
        );
        h.run_steps(3);
        h.state_mut().ed.color = 'y';
        assert_eq!(h.state().ed.m.pix[0][0][0], '.');
        let r = h.state().ed.image_rect;
        let z = h.state().ed.zoom;
        let p = r.min + Vec2::splat(z * 0.5);
        h.event(Event::PointerMoved(p));
        h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(2);
        h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(2);
        assert_eq!(h.state().ed.m.pix[0][0][0], 'y');
        assert!(h.state().ed.dirty);
        h.state_mut().ed.undo();
        assert_eq!(h.state().ed.m.pix[0][0][0], '.');
        h.state_mut().ed.redo();
        {
            let s = h.state_mut();
            let mut cx = Cx { project: &mut s.project, tex: &mut s.tex, log: &mut s.log, open: &mut s.open, active: true };
            s.ed.save(&mut cx).unwrap();
        }
        let spr = h.state().project.sprite("heart").expect("saved sprite reloads");
        assert_eq!(&spr.rgba[0..4], &[0xff, 0xcd, 0x75, 255], "top-left pixel is now yellow");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
