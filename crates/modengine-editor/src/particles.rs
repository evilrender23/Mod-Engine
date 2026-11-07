//! Particle system editor: emitters and particle types on the left, a live animated preview
//! on the right. The preview runs the engine's own simulation ([`modengine::particles`]) and
//! paints its draw list with egui (additive particles use premultiplied colour with alpha 0).

use crate::app::Cx;
use crate::motion::{History, View};
use crate::project::Project;
use eframe::egui::{self, epaint::Vertex, Color32, Mesh, Pos2, Rect, RichText, Sense, Shape, Stroke, TextureHandle, TextureOptions, Vec2};
use modengine::particles::{Distr, EmitterToml, PartDraw, PartSysToml, PartTypeToml, PartWorld, Range2, RegionShape, Tex, SHAPE_NAMES, SHAPE_SIZE};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Debug)]
enum Sel {
    System,
    Emitter(usize),
    Type(usize),
}

pub struct ParticleEditor {
    pub name: String,
    path: PathBuf,
    data: PartSysToml,
    saved: PartSysToml,
    hist: History<PartSysToml>,
    sel: Sel,
    // preview
    world: PartWorld,
    sys: Option<usize>,
    built: Option<PartSysToml>,
    sim_error: Option<String>,
    playing: bool,
    speed: f32,
    acc: f32,
    view: View,
    bg: [u8; 3],
    show_regions: bool,
    shapes: Vec<TextureHandle>,
    /// Region being dragged: (emitter, handle 0 = move / 1 = resize, region at drag start, pointer start).
    drag: Option<(usize, u8, [f64; 4], Pos2)>,
    /// Next rebuild starts empty (Reiniciar) instead of pre-running a second of simulation.
    cold: bool,
}

fn rgb_of(s: &str) -> [u8; 3] {
    crate::tomlw::parse_hex(s).unwrap_or([255, 255, 255])
}

/// Engine colour (0xRRGGBB) and alpha → egui colour; additive = premultiplied with alpha 0.
pub fn part_color(c: u32, a: f64, additive: bool) -> Color32 {
    let a = a.clamp(0.0, 1.0) as f32;
    let (r, g, b) = (((c >> 16) & 255) as f32, ((c >> 8) & 255) as f32, (c & 255) as f32);
    let pm = |v: f32| (v * a).round() as u8;
    if additive {
        Color32::from_rgba_premultiplied(pm(r), pm(g), pm(b), 0)
    } else {
        Color32::from_rgba_premultiplied(pm(r), pm(g), pm(b), (a * 255.0).round() as u8)
    }
}

/// Textures of the built-in shapes (created once per context).
pub fn shape_textures(ctx: &egui::Context) -> Vec<TextureHandle> {
    (0..SHAPE_NAMES.len())
        .map(|k| {
            let img = egui::ColorImage::from_rgba_unmultiplied([SHAPE_SIZE as usize; 2], &modengine::particles::shape_image(k));
            ctx.load_texture(format!("pt_shape_{}", SHAPE_NAMES[k]), img, TextureOptions::LINEAR)
        })
        .collect()
}

fn sprite_lookup(p: &Project) -> impl Fn(&str) -> Option<u32> + '_ {
    move |n: &str| p.sprites.iter().position(|s| s.name == n && s.data.is_ok()).map(|i| i as u32)
}

/// Paints a particle draw list. `to_s` maps world to screen, `zoom` scales sizes.
pub fn paint(painter: &egui::Painter, list: &[PartDraw], shapes: &[TextureHandle], cx_tex: &mut crate::tex::TexCache, p: &Project, to_s: &dyn Fn(f64, f64) -> Pos2, zoom: f32) {
    let ctx = painter.ctx().clone();
    let mut mesh = Mesh::default();
    let flush = |mesh: &mut Mesh| {
        if !mesh.is_empty() {
            painter.add(Shape::mesh(std::mem::take(mesh)));
        }
    };
    for d in list {
        // texture, uv, size in px and origin (0..1)
        let (tex, uv, size, origin) = match d.tex {
            Tex::Shape(k) => match shapes.get(k as usize) {
                Some(t) => (t.id(), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Vec2::splat(SHAPE_SIZE as f32), Vec2::splat(0.5)),
                None => continue,
            },
            Tex::Sprite(i) => {
                let Some(name) = p.sprites.get(i as usize).map(|s| s.name.clone()) else { continue };
                let Some(st) = cx_tex.get(&ctx, p, &name) else { continue };
                let fs = st.frame_size();
                (st.id, st.uv(d.frame as usize), fs, st.origin() / fs)
            }
        };
        if mesh.texture_id != tex {
            flush(&mut mesh);
            mesh = Mesh::with_texture(tex);
        }
        let c = to_s(d.x, d.y);
        let (w, h) = (size.x * d.sx as f32 * zoom, size.y * d.sy as f32 * zoom);
        if w.abs() < 0.05 && h.abs() < 0.05 {
            continue;
        }
        let (sa, ca) = (-(d.angle as f32).to_radians()).sin_cos();
        let col = part_color(d.color, d.alpha, d.additive);
        let base = mesh.vertices.len() as u32;
        for (u, v) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
            let (lx, ly) = ((u - origin.x) * w, (v - origin.y) * h);
            let pos = Pos2::new(c.x + lx * ca - ly * sa, c.y + lx * sa + ly * ca);
            let uvp = Pos2::new(uv.min.x + (uv.max.x - uv.min.x) * u, uv.min.y + (uv.max.y - uv.min.y) * v);
            mesh.vertices.push(Vertex { pos, uv: uvp, color: col });
        }
        mesh.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    flush(&mut mesh);
}

/// Ready-made effects (GameMaker's particle presets): (label, types, emitters).
pub fn presets() -> Vec<(&'static str, Vec<PartTypeToml>, Vec<EmitterToml>)> {
    let t = |name: &str| PartTypeToml { name: name.into(), ..Default::default() };
    let e = |name: &str, ty: &str| EmitterToml { name: name.into(), ty: ty.into(), ..Default::default() };
    let c = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    vec![
        (
            concat!(fa!(fire), " Fuego"),
            vec![PartTypeToml { shape: "flare".into(), size: Range2([0.25, 0.45]), size_incr: -0.006, speed: Range2([0.4, 0.9]), direction: Range2([80.0, 100.0]), direction_wiggle: 6.0, colors: c(&["#ffe066", "#ff7a00", "#b13e53"]), alphas: vec![0.9, 0.7, 0.0], additive: true, life: Range2([25.0, 40.0]), ..t("fuego") }],
            vec![EmitterToml { shape: "ellipse".into(), distribution: "gaussian".into(), region: [-6.0, -3.0, 6.0, 3.0], count: 2, ..e("fuego", "fuego") }],
        ),
        (
            concat!(fa!(wind), " Humo"),
            vec![PartTypeToml { shape: "smoke".into(), size: Range2([0.3, 0.6]), size_incr: 0.01, speed: Range2([0.3, 0.6]), direction: Range2([80.0, 100.0]), orientation: Range2([0.0, 360.0]), orientation_incr: 0.5, colors: c(&["#94b0c2", "#333c57"]), alphas: vec![0.0, 0.5, 0.0], life: Range2([60.0, 90.0]), ..t("humo") }],
            vec![EmitterToml { shape: "ellipse".into(), region: [-8.0, -2.0, 8.0, 2.0], count: -2, ..e("humo", "humo") }],
        ),
        (
            concat!(fa!(wand_magic_sparkles), " Chispas"),
            vec![PartTypeToml { shape: "spark".into(), size: Range2([0.08, 0.16]), speed: Range2([1.5, 3.0]), direction: Range2([45.0, 135.0]), gravity: 0.1, orientation: Range2([0.0, 360.0]), orientation_incr: 8.0, colors: c(&["#ffffff", "#ffcd75"]), alphas: vec![1.0, 0.0], additive: true, life: Range2([30.0, 50.0]), ..t("chispa") }],
            vec![EmitterToml { region: [-2.0, -2.0, 2.0, 2.0], mode: "burst".into(), count: 12, interval: Range2([40.0, 60.0]), ..e("chispas", "chispa") }],
        ),
        (
            concat!(fa!(snowflake), " Nieve"),
            vec![PartTypeToml { shape: "snow".into(), size: Range2([0.06, 0.16]), speed: Range2([0.3, 0.7]), direction: Range2([260.0, 280.0]), direction_wiggle: 15.0, orientation: Range2([0.0, 360.0]), orientation_incr: 1.0, colors: c(&["#ffffff"]), alphas: vec![0.0, 0.9, 0.0], life: Range2([200.0, 300.0]), ..t("copo") }],
            vec![EmitterToml { shape: "line".into(), region: [-160.0, -90.0, 160.0, -90.0], count: -3, ..e("cielo", "copo") }],
        ),
        (
            concat!(fa!(cloud_rain), " Lluvia"),
            vec![PartTypeToml { shape: "line".into(), size: Range2([0.12, 0.2]), scale: [1.0, 0.5], speed: Range2([5.0, 7.0]), direction: Range2([255.0, 260.0]), orientation_relative: true, colors: c(&["#73eff7"]), alphas: vec![0.6], life: Range2([35.0, 45.0]), ..t("gota") }],
            vec![EmitterToml { shape: "line".into(), region: [-160.0, -100.0, 200.0, -100.0], count: 3, ..e("nubes", "gota") }],
        ),
        (
            concat!(fa!(burst), " Explosión"),
            vec![
                PartTypeToml { shape: "explosion".into(), size: Range2([0.3, 0.5]), size_incr: 0.02, speed: Range2([0.2, 1.0]), direction: Range2([0.0, 360.0]), orientation: Range2([0.0, 360.0]), colors: c(&["#ffffff", "#ffcd75", "#b13e53"]), alphas: vec![1.0, 0.0], additive: true, life: Range2([20.0, 30.0]), death_type: Some("humo_exp".into()), death_count: 1, ..t("bola") },
                PartTypeToml { shape: "smoke".into(), size: Range2([0.3, 0.5]), size_incr: 0.01, speed: Range2([0.2, 0.4]), direction: Range2([80.0, 100.0]), colors: c(&["#566c86"]), alphas: vec![0.5, 0.0], life: Range2([40.0, 60.0]), ..t("humo_exp") },
            ],
            vec![EmitterToml { shape: "ellipse".into(), region: [-6.0, -6.0, 6.0, 6.0], mode: "burst".into(), count: 16, ..e("explosion", "bola") }],
        ),
        (
            concat!(fa!(wand_sparkles), " Magia"),
            vec![PartTypeToml { shape: "star".into(), size: Range2([0.06, 0.14]), size_wiggle: 0.04, speed: Range2([0.2, 0.6]), direction: Range2([0.0, 360.0]), direction_incr: 3.0, orientation: Range2([0.0, 360.0]), orientation_incr: 4.0, colors: c(&["#41a6f6", "#b55088"]), color_mode: "mix".into(), alphas: vec![0.0, 1.0, 0.0], additive: true, life: Range2([40.0, 70.0]), ..t("brillo") }],
            vec![EmitterToml { shape: "ellipse".into(), distribution: "invgaussian".into(), region: [-16.0, -16.0, 16.0, 16.0], count: 1, ..e("aura", "brillo") }],
        ),
    ]
}

fn range_ui(ui: &mut egui::Ui, v: &mut Range2, speed: f64) {
    ui.add(egui::DragValue::new(&mut v.0[0]).speed(speed).prefix("min "));
    ui.add(egui::DragValue::new(&mut v.0[1]).speed(speed).prefix("máx "));
}

fn drag(ui: &mut egui::Ui, v: &mut f64, speed: f64) {
    ui.add(egui::DragValue::new(v).speed(speed));
}

fn type_combo(ui: &mut egui::Ui, id: impl std::hash::Hash + std::fmt::Debug, v: &mut Option<String>, names: &[String]) {
    let text = v.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| "(ninguno)".into());
    egui::ComboBox::from_id_salt(id).selected_text(text).show_ui(ui, |ui| {
        ui.selectable_value(v, None, "(ninguno)");
        for n in names {
            ui.selectable_value(v, Some(n.clone()), n);
        }
    });
}

impl ParticleEditor {
    pub fn open(name: &str, path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let data = PartSysToml::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(ParticleEditor {
            name: name.into(),
            path: path.into(),
            hist: History::new(&data),
            saved: data.clone(),
            sel: if data.emitters.is_empty() { Sel::System } else { Sel::Emitter(0) },
            data,
            world: PartWorld::default(),
            sys: None,
            built: None,
            sim_error: None,
            playing: true,
            speed: 1.0,
            acc: 0.0,
            view: View::default(),
            bg: [26, 28, 44],
            show_regions: true,
            shapes: vec![],
            drag: None,
            cold: false,
        })
    }

    pub fn dirty(&self) -> bool {
        self.data != self.saved
    }

    pub fn save(&mut self) -> Result<(), String> {
        std::fs::write(&self.path, self.data.to_toml()).map_err(|e| format!("{}: {e}", self.path.display()))?;
        self.saved = self.data.clone();
        Ok(())
    }

    pub fn undo(&mut self) {
        self.hist.undo(&mut self.data);
    }
    pub fn redo(&mut self) {
        self.hist.redo(&mut self.data);
    }

    /// Number of live particles in the preview.
    pub fn live(&self) -> usize {
        self.sys.map(|s| self.world.count(s)).unwrap_or(0)
    }

    /// Restarts the preview simulation from the current data, pre-running `warm` steps.
    fn rebuild(&mut self, p: &Project, warm: u32) {
        self.world = PartWorld::new(vec![(self.name.clone(), self.data.clone())]);
        let lookup = sprite_lookup(p);
        match self.world.system_from_asset(0, 0.0, 0.0, &lookup) {
            Ok(s) => {
                self.sys = Some(s);
                self.sim_error = None;
                for _ in 0..warm {
                    self.world.system_update(s);
                }
            }
            Err(e) => {
                self.sys = None;
                self.sim_error = Some(e);
            }
        }
        self.built = Some(self.data.clone());
    }

    /// Advances the preview by `dt` seconds of 60-steps-per-second game time.
    pub fn advance(&mut self, p: &Project, dt: f32) {
        if self.built.as_ref() != Some(&self.data) {
            let warm = if std::mem::take(&mut self.cold) { 0 } else { 60 };
            self.rebuild(p, warm);
        }
        if !self.playing {
            return;
        }
        self.acc += dt * self.speed;
        let mut n = 0;
        while self.acc >= 1.0 / 60.0 && n < 8 {
            self.acc -= 1.0 / 60.0;
            if let Some(s) = self.sys {
                self.world.system_update(s);
            }
            n += 1;
        }
        if n == 8 {
            self.acc = 0.0;
        }
        // one-shot bursts repeat in the preview once everything has died
        if n > 0 && self.live() == 0 {
            self.burst_again();
        }
    }

    fn burst_again(&mut self) {
        if let Some(s) = self.sys.and_then(|s| self.world.sys(s)) {
            for e in s.emitters.iter_mut().flatten() {
                if e.burst != 0 {
                    e.burst_due = true;
                    e.wait = 0.0;
                }
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        if self.shapes.is_empty() {
            self.shapes = shape_textures(ui.ctx());
        }
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        self.advance(cx.project, dt);
        if self.playing {
            ui.ctx().request_repaint();
        }
        egui::Panel::left(("parts_side", &self.name)).resizable(true).default_size(340.0).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.side(ui, cx.project));
        });
        egui::CentralPanel::default().show(ui, |ui| {
            self.toolbar(ui);
            self.canvas(ui, cx);
        });
        let busy = ui.input(|i| i.pointer.any_down());
        self.hist.track(&self.data, busy);
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui.button(if self.playing { concat!(fa!(pause), " Pausa") } else { concat!(fa!(play), " Reproducir") }).clicked() {
                self.playing = !self.playing;
            }
            if ui.button(concat!(fa!(rotate_left), " Reiniciar")).on_hover_text("vacía el sistema y vuelve a empezar").clicked() {
                self.built = None;
                self.cold = true;
                self.acc = 0.0;
            }
            if ui.button(concat!(fa!(burst), " Ráfaga")).on_hover_text("vuelve a disparar los emisores de ráfaga").clicked() {
                self.burst_again();
            }
            ui.label("velocidad");
            ui.add(egui::Slider::new(&mut self.speed, 0.1..=3.0).max_decimals(2));
            ui.label("fondo");
            ui.color_edit_button_srgb(&mut self.bg);
            ui.checkbox(&mut self.show_regions, "regiones");
            ui.label(RichText::new(format!("{} partículas", self.live())).weak());
        });
    }

    fn canvas(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = resp.rect;
        painter.rect_filled(rect, 0.0, crate::tex::rgb(self.bg));
        // fit the emitter regions (at least 160×120 around the origin) the first time
        let mut world = Rect::from_center_size(Pos2::ZERO, Vec2::new(160.0, 120.0));
        for e in &self.data.emitters {
            let [x1, y1, x2, y2] = e.region;
            world = world.union(Rect::from_two_pos(Pos2::new(x1 as f32, y1 as f32), Pos2::new(x2 as f32, y2 as f32)).expand(24.0));
        }
        self.view.update(ui, &resp, world);
        let view = self.view;
        let to_s = |x: f64, y: f64| view.to_s(Pos2::new(x as f32, y as f32));
        let p = ui.painter_at(rect);
        // origin cross
        let o = to_s(0.0, 0.0);
        let axis = Color32::from_rgba_unmultiplied(255, 255, 255, 40);
        p.line_segment([Pos2::new(rect.left(), o.y), Pos2::new(rect.right(), o.y)], Stroke::new(1.0, axis));
        p.line_segment([Pos2::new(o.x, rect.top()), Pos2::new(o.x, rect.bottom())], Stroke::new(1.0, axis));

        // particles
        if let Some(s) = self.sys {
            let sprites = &cx.project.sprites;
            let frames = |i: u32| sprites.get(i as usize).and_then(|s| s.data.as_ref().ok()).map(|s| s.frames.len() as u32).unwrap_or(1);
            let list = self.world.draw_list(s, &frames);
            paint(&p, &list, &self.shapes, cx.tex, cx.project, &to_s, view.zoom);
        }

        // emitter regions (the selected one can be moved and resized)
        let pointer = resp.interact_pointer_pos();
        if self.show_regions {
            for (i, e) in self.data.emitters.iter().enumerate() {
                let selected = self.sel == Sel::Emitter(i);
                let col = if selected { Color32::from_rgb(120, 200, 255) } else { Color32::from_rgba_unmultiplied(255, 255, 255, 90) };
                let st = Stroke::new(if selected { 1.5 } else { 1.0 }, col);
                let [x1, y1, x2, y2] = e.region;
                let (a, b) = (to_s(x1, y1), to_s(x2, y2));
                let r = Rect::from_two_pos(a, b);
                match RegionShape::parse(&e.shape).unwrap_or(RegionShape::Rectangle) {
                    RegionShape::Rectangle => {
                        p.rect_stroke(r, 0.0, st, egui::StrokeKind::Middle);
                    }
                    RegionShape::Ellipse => {
                        let pts: Vec<Pos2> = (0..48)
                            .map(|k| {
                                let t = k as f32 / 48.0 * std::f32::consts::TAU;
                                Pos2::new(r.center().x + t.cos() * r.width() / 2.0, r.center().y + t.sin() * r.height() / 2.0)
                            })
                            .collect();
                        p.add(Shape::closed_line(pts, st));
                    }
                    RegionShape::Diamond => {
                        p.add(Shape::closed_line(vec![r.center_top(), r.right_center(), r.center_bottom(), r.left_center()], st));
                    }
                    RegionShape::Line => {
                        p.line_segment([a, b], st);
                    }
                }
                if selected {
                    p.circle_filled(b, 4.0, col);
                    p.text(r.left_top() + Vec2::new(0.0, -14.0), egui::Align2::LEFT_TOP, &e.name, egui::FontId::proportional(11.0), col);
                }
            }
        }
        // dragging the selected region
        if resp.drag_started_by(egui::PointerButton::Primary) && !ui.input(|i| i.key_down(egui::Key::Space)) {
            let origin = ui.input(|i| i.pointer.press_origin()).or(pointer);
            if let (Sel::Emitter(i), Some(pt)) = (self.sel, origin) {
                if let Some(e) = self.data.emitters.get(i) {
                    let [x1, y1, x2, y2] = e.region;
                    let corner = to_s(x2, y2);
                    let r = Rect::from_two_pos(to_s(x1, y1), corner).expand(4.0);
                    if corner.distance(pt) < 8.0 {
                        self.drag = Some((i, 1, e.region, pt));
                    } else if r.contains(pt) {
                        self.drag = Some((i, 0, e.region, pt));
                    }
                }
            }
        }
        if let (Some((i, h, start, from)), Some(pt)) = (self.drag, pointer) {
            let d = (pt - from) / view.zoom;
            let (dx, dy) = (d.x.round() as f64, d.y.round() as f64);
            if let Some(e) = self.data.emitters.get_mut(i) {
                e.region = if h == 0 { [start[0] + dx, start[1] + dy, start[2] + dx, start[3] + dy] } else { [start[0], start[1], start[2] + dx, start[3] + dy] };
            }
        }
        if !resp.dragged() {
            self.drag = None;
        }
        if let Some(e) = &self.sim_error {
            p.text(rect.left_top() + Vec2::new(8.0, 8.0), egui::Align2::LEFT_TOP, e, egui::FontId::proportional(13.0), crate::theme::colors(ui.ctx()).error);
        }
        p.text(
            rect.left_bottom() + Vec2::new(8.0, -6.0),
            egui::Align2::LEFT_BOTTOM,
            "rueda: zoom · botón central o Espacio+arrastrar: mover vista · arrastra la región del emisor elegido (esquina: tamaño)",
            egui::FontId::proportional(11.0),
            Color32::from_rgba_unmultiplied(255, 255, 255, 110),
        );
    }

    fn side(&mut self, ui: &mut egui::Ui, p: &Project) {
        let pal = crate::theme::colors(ui.ctx());
        let sprite_ok = |n: &str| p.sprite(n).is_some();
        for e in self.data.validate(&sprite_ok) {
            ui.colored_label(pal.error, format!(concat!(fa!(triangle_exclamation), " {e}"), e = e));
        }
        // ---- lists
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.sel, Sel::System, RichText::new(concat!(fa!(gear), " Sistema")).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button(concat!(fa!(plus), " Añadir preset"), |ui| {
                    for (label, types, emitters) in presets() {
                        if ui.button(label).clicked() {
                            self.add_preset(types, emitters);
                            ui.close();
                        }
                    }
                });
            });
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.strong("Emisores");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("+").on_hover_text("nuevo emisor").clicked() {
                    let ty = self.data.types.first().map(|t| t.name.clone()).unwrap_or_default();
                    let n = self.unique(&format!("emisor{}", self.data.emitters.len() + 1), true);
                    self.data.emitters.push(EmitterToml { name: n, ty, ..Default::default() });
                    self.sel = Sel::Emitter(self.data.emitters.len() - 1);
                }
            });
        });
        let mut action: Option<(bool, usize, u8)> = None; // (is_emitter, index, 0 dup / 1 delete / 2 up)
        for (i, e) in self.data.emitters.iter().enumerate() {
            ui.horizontal(|ui| {
                let label = format!("{} {}  ·  {} {}", if e.enabled { fa!(circle_dot) } else { fa!(circle) }, e.name, if e.mode == "burst" { "ráfaga" } else { "flujo" }, e.count);
                let label = if e.enabled { RichText::new(label) } else { crate::icons::regular(label) };
                if ui.selectable_label(self.sel == Sel::Emitter(i), label).clicked() {
                    self.sel = Sel::Emitter(i);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button(fa!(trash_can)).on_hover_text("eliminar").clicked() {
                        action = Some((true, i, 1));
                    }
                    if ui.small_button(fa!(clone)).on_hover_text("duplicar").clicked() {
                        action = Some((true, i, 0));
                    }
                });
            });
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.strong("Tipos de partícula");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("+").on_hover_text("nuevo tipo").clicked() {
                    let n = self.unique(&format!("tipo{}", self.data.types.len() + 1), false);
                    self.data.types.push(PartTypeToml { name: n, ..Default::default() });
                    self.sel = Sel::Type(self.data.types.len() - 1);
                }
            });
        });
        for (i, t) in self.data.types.iter().enumerate() {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                ui.painter().rect_filled(r, 2.0, Color32::from_rgb(30, 32, 48));
                if let Some(k) = modengine::particles::shape_index(&t.shape).filter(|_| t.sprite.is_none()) {
                    let c = t.colors.first().map(|c| rgb_of(c)).unwrap_or([255; 3]);
                    ui.painter().image(self.shapes[k].id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), crate::tex::rgb(c));
                }
                if ui.selectable_label(self.sel == Sel::Type(i), &t.name).clicked() {
                    self.sel = Sel::Type(i);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button(fa!(trash_can)).on_hover_text("eliminar").clicked() {
                        action = Some((false, i, 1));
                    }
                    if ui.small_button(fa!(clone)).on_hover_text("duplicar").clicked() {
                        action = Some((false, i, 0));
                    }
                });
            });
        }
        match action {
            Some((true, i, 0)) => {
                let mut e = self.data.emitters[i].clone();
                e.name = self.unique(&e.name, true);
                self.data.emitters.insert(i + 1, e);
                self.sel = Sel::Emitter(i + 1);
            }
            Some((true, i, _)) => {
                self.data.emitters.remove(i);
                self.sel = if self.data.emitters.is_empty() { Sel::System } else { Sel::Emitter(i.min(self.data.emitters.len() - 1)) };
            }
            Some((false, i, 0)) => {
                let mut t = self.data.types[i].clone();
                t.name = self.unique(&t.name, false);
                self.data.types.insert(i + 1, t);
                self.sel = Sel::Type(i + 1);
            }
            Some((false, i, _)) => {
                self.data.types.remove(i);
                self.sel = if self.data.types.is_empty() { Sel::System } else { Sel::Type(i.min(self.data.types.len() - 1)) };
            }
            None => {}
        }
        ui.separator();
        // ---- properties
        match self.sel {
            Sel::System => self.system_form(ui),
            Sel::Emitter(i) if i < self.data.emitters.len() => self.emitter_form(ui, i),
            Sel::Type(i) if i < self.data.types.len() => self.type_form(ui, i, p),
            _ => self.sel = Sel::System,
        }
    }

    /// Adds a preset's types and emitters, renaming them when the names are taken.
    pub fn add_preset(&mut self, types: Vec<PartTypeToml>, emitters: Vec<EmitterToml>) {
        let mut renames: Vec<(String, String)> = vec![];
        for mut t in types {
            let n = self.unique(&t.name, false);
            renames.push((t.name.clone(), n.clone()));
            t.name = n;
            self.data.types.push(t);
        }
        let ren = |s: &str| renames.iter().find(|(a, _)| a == s).map(|(_, b)| b.clone()).unwrap_or_else(|| s.to_string());
        let start = self.data.types.len() - renames.len();
        for t in &mut self.data.types[start..] {
            t.step_type = t.step_type.as_deref().map(ren);
            t.death_type = t.death_type.as_deref().map(ren);
        }
        for mut e in emitters {
            e.name = self.unique(&e.name, true);
            e.ty = ren(&e.ty);
            self.data.emitters.push(e);
        }
        if !self.data.emitters.is_empty() {
            self.sel = Sel::Emitter(self.data.emitters.len() - 1);
        }
    }

    fn unique(&self, base: &str, emitter: bool) -> String {
        let taken: Vec<&str> = if emitter { self.data.emitters.iter().map(|e| e.name.as_str()).collect() } else { self.data.types.iter().map(|t| t.name.as_str()).collect() };
        if !taken.contains(&base) {
            return base.to_string();
        }
        (2..).map(|n| format!("{base}_{n}")).find(|n| !taken.contains(&n.as_str())).unwrap()
    }

    fn system_form(&mut self, ui: &mut egui::Ui) {
        ui.strong("Sistema");
        egui::Grid::new(("ps_sys", &self.name)).num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label("Profundidad");
            drag(ui, &mut self.data.depth, 1.0);
            ui.end_row();
            ui.label("Orden de dibujo");
            ui.checkbox(&mut self.data.new_below, "las nuevas debajo");
            ui.end_row();
        });
        ui.add_space(8.0);
        ui.label(RichText::new(format!("En ModScript:\nvar ps = part_system_create({})\npart_system_position(ps, x, y)", self.name)).monospace().weak());
    }

    fn emitter_form(&mut self, ui: &mut egui::Ui, i: usize) {
        let types: Vec<String> = self.data.types.iter().map(|t| t.name.clone()).collect();
        let e = &mut self.data.emitters[i];
        ui.strong(format!("Emisor «{}»", e.name));
        egui::Grid::new(("ps_em", i)).num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label("Nombre");
            ui.text_edit_singleline(&mut e.name);
            ui.end_row();
            ui.label("Activo");
            ui.checkbox(&mut e.enabled, "");
            ui.end_row();
            ui.label("Tipo");
            egui::ComboBox::from_id_salt(("em_ty", i)).selected_text(e.ty.clone()).show_ui(ui, |ui| {
                for n in &types {
                    ui.selectable_value(&mut e.ty, n.clone(), n);
                }
            });
            ui.end_row();
            ui.label("Modo");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut e.mode, "stream".to_string(), "flujo").on_hover_text("emite en cada paso");
                ui.selectable_value(&mut e.mode, "burst".to_string(), "ráfaga").on_hover_text("emite de golpe (una vez o cada intervalo)");
            });
            ui.end_row();
            ui.label("Cantidad");
            ui.add(egui::DragValue::new(&mut e.count).speed(0.2).range(-100..=2000))
                .on_hover_text("partículas por paso o por ráfaga; negativo en flujo = una con probabilidad 1/n");
            ui.end_row();
            ui.label("Forma");
            egui::ComboBox::from_id_salt(("em_shape", i)).selected_text(e.shape.clone()).show_ui(ui, |ui| {
                for (s, l) in RegionShape::ALL.iter().zip(["rectángulo", "elipse", "rombo", "línea"]) {
                    ui.selectable_value(&mut e.shape, s.to_string(), l);
                }
            });
            ui.end_row();
            ui.label("Distribución");
            egui::ComboBox::from_id_salt(("em_distr", i)).selected_text(e.distribution.clone()).show_ui(ui, |ui| {
                for (s, l) in Distr::ALL.iter().zip(["uniforme", "gaussiana (centro)", "gaussiana inversa (bordes)"]) {
                    ui.selectable_value(&mut e.distribution, s.to_string(), l);
                }
            });
            ui.end_row();
            ui.label("Región x1, y1");
            ui.horizontal(|ui| {
                drag(ui, &mut e.region[0], 0.5);
                drag(ui, &mut e.region[1], 0.5);
            });
            ui.end_row();
            ui.label("Región x2, y2");
            ui.horizontal(|ui| {
                drag(ui, &mut e.region[2], 0.5);
                drag(ui, &mut e.region[3], 0.5);
            });
            ui.end_row();
            ui.label("Retardo (pasos)");
            ui.horizontal(|ui| range_ui(ui, &mut e.delay, 1.0));
            ui.end_row();
            ui.label("Intervalo (pasos)");
            ui.horizontal(|ui| range_ui(ui, &mut e.interval, 1.0)).response.on_hover_text("0 = la ráfaga solo se dispara una vez; en flujo, emite cada tantos pasos");
            ui.end_row();
        });
    }

    fn type_form(&mut self, ui: &mut egui::Ui, i: usize, p: &Project) {
        let names: Vec<String> = self.data.types.iter().map(|t| t.name.clone()).collect();
        let old_name = self.data.types[i].name.clone();
        let shapes = self.shapes.clone();
        let t = &mut self.data.types[i];
        ui.strong(format!("Tipo «{}»", t.name));
        ui.horizontal(|ui| {
            ui.label("Nombre");
            ui.text_edit_singleline(&mut t.name);
        });
        ui.add_space(4.0);
        // ---- look
        egui::CollapsingHeader::new("Aspecto").id_salt(("pt_look", i)).default_open(true).show(ui, |ui| {
            let mut use_sprite = t.sprite.is_some();
            ui.horizontal(|ui| {
                ui.radio_value(&mut use_sprite, false, "forma");
                ui.radio_value(&mut use_sprite, true, "sprite");
            });
            if use_sprite && t.sprite.is_none() {
                t.sprite = Some(p.sprite_names().first().cloned().unwrap_or_default());
            } else if !use_sprite {
                t.sprite = None;
            }
            match &mut t.sprite {
                None => {
                    ui.horizontal_wrapped(|ui| {
                        for (k, n) in SHAPE_NAMES.iter().enumerate() {
                            let img = egui::Image::new(egui::load::SizedTexture::new(shapes[k].id(), Vec2::splat(24.0)));
                            if ui.add(egui::Button::image(img).selected(t.shape == *n)).on_hover_text(*n).clicked() {
                                t.shape = n.to_string();
                            }
                        }
                    });
                }
                Some(s) => {
                    egui::ComboBox::from_id_salt(("pt_spr", i)).selected_text(s.clone()).show_ui(ui, |ui| {
                        for n in p.sprite_names() {
                            ui.selectable_value(s, n.clone(), n);
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut t.animate, "animar");
                        ui.checkbox(&mut t.stretch, "estirar a la vida");
                        ui.checkbox(&mut t.random_frame, "frame al azar");
                    });
                }
            }
            egui::Grid::new(("pt_look_g", i)).num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                ui.label("Tamaño");
                ui.horizontal(|ui| range_ui(ui, &mut t.size, 0.01));
                ui.end_row();
                ui.label("  incremento / oscilación");
                ui.horizontal(|ui| {
                    drag(ui, &mut t.size_incr, 0.001);
                    drag(ui, &mut t.size_wiggle, 0.005);
                });
                ui.end_row();
                ui.label("Escala x, y");
                ui.horizontal(|ui| {
                    drag(ui, &mut t.scale[0], 0.01);
                    drag(ui, &mut t.scale[1], 0.01);
                });
                ui.end_row();
                ui.label("Orientación");
                ui.horizontal(|ui| range_ui(ui, &mut t.orientation, 1.0));
                ui.end_row();
                ui.label("  incremento / oscilación");
                ui.horizontal(|ui| {
                    drag(ui, &mut t.orientation_incr, 0.1);
                    drag(ui, &mut t.orientation_wiggle, 0.1);
                });
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut t.orientation_relative, "relativa a la dirección");
                ui.end_row();
            });
        });
        // ---- colour
        egui::CollapsingHeader::new("Color y transparencia").id_salt(("pt_col", i)).default_open(true).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Colores");
                let mut n = t.colors.len().clamp(1, 3);
                for k in 1..=3 {
                    ui.selectable_value(&mut n, k, k.to_string());
                }
                while t.colors.len() < n {
                    let last = t.colors.last().cloned().unwrap_or_else(|| "#ffffff".into());
                    t.colors.push(last);
                }
                t.colors.truncate(n);
            });
            ui.horizontal(|ui| {
                for c in t.colors.iter_mut() {
                    let mut rgb = rgb_of(c);
                    if ui.color_edit_button_srgb(&mut rgb).changed() {
                        *c = crate::tomlw::color_hex(rgb);
                    }
                }
                if t.colors.len() >= 2 {
                    let mut mix = t.color_mode == "mix";
                    ui.checkbox(&mut mix, "mezcla al azar").on_hover_text("color fijo al azar entre los dos primeros (part_type_color_mix)");
                    t.color_mode = if mix { "mix".into() } else { "fade".into() };
                }
            });
            ui.horizontal(|ui| {
                ui.label("Alpha");
                let mut n = t.alphas.len().clamp(1, 3);
                for k in 1..=3 {
                    ui.selectable_value(&mut n, k, k.to_string());
                }
                while t.alphas.len() < n {
                    let last = t.alphas.last().copied().unwrap_or(1.0);
                    t.alphas.push(last);
                }
                t.alphas.truncate(n);
            });
            ui.horizontal(|ui| {
                for a in t.alphas.iter_mut() {
                    ui.add(egui::DragValue::new(a).speed(0.01).range(0.0..=1.0));
                }
            });
            ui.checkbox(&mut t.additive, "mezcla aditiva (brillo)");
        });
        // ---- motion
        egui::CollapsingHeader::new("Movimiento y vida").id_salt(("pt_mov", i)).default_open(true).show(ui, |ui| {
            egui::Grid::new(("pt_mov_g", i)).num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                ui.label("Vida (pasos)");
                ui.horizontal(|ui| range_ui(ui, &mut t.life, 1.0));
                ui.end_row();
                ui.label("Velocidad");
                ui.horizontal(|ui| range_ui(ui, &mut t.speed, 0.05));
                ui.end_row();
                ui.label("  incremento / oscilación");
                ui.horizontal(|ui| {
                    drag(ui, &mut t.speed_incr, 0.005);
                    drag(ui, &mut t.speed_wiggle, 0.01);
                });
                ui.end_row();
                ui.label("Dirección");
                ui.horizontal(|ui| range_ui(ui, &mut t.direction, 1.0));
                ui.end_row();
                ui.label("  incremento / oscilación");
                ui.horizontal(|ui| {
                    drag(ui, &mut t.direction_incr, 0.1);
                    drag(ui, &mut t.direction_wiggle, 0.2);
                });
                ui.end_row();
                ui.label("Gravedad / dirección");
                ui.horizontal(|ui| {
                    drag(ui, &mut t.gravity, 0.005);
                    drag(ui, &mut t.gravity_dir, 1.0);
                });
                ui.end_row();
            });
        });
        // ---- spawning
        egui::CollapsingHeader::new("Subpartículas").id_salt(("pt_sub", i)).default_open(false).show(ui, |ui| {
            let others: Vec<String> = names.iter().filter(|n| **n != old_name).cloned().collect();
            egui::Grid::new(("pt_sub_g", i)).num_columns(3).spacing([8.0, 4.0]).show(ui, |ui| {
                ui.label("En cada paso");
                type_combo(ui, ("pt_step", i), &mut t.step_type, &others);
                ui.add(egui::DragValue::new(&mut t.step_count).range(-100..=50));
                ui.end_row();
                ui.label("Al morir");
                type_combo(ui, ("pt_death", i), &mut t.death_type, &others);
                ui.add(egui::DragValue::new(&mut t.death_count).range(-100..=200));
                ui.end_row();
            });
            ui.label(RichText::new("cantidad negativa: una partícula con probabilidad 1/n").weak());
        });
        // a renamed type keeps its emitters and sub-particles pointing at it
        let new_name = self.data.types[i].name.clone();
        if new_name != old_name && !new_name.is_empty() {
            for e in &mut self.data.emitters {
                if e.ty == old_name {
                    e.ty = new_name.clone();
                }
            }
            for t in &mut self.data.types {
                for r in [&mut t.step_type, &mut t.death_type] {
                    if r.as_deref() == Some(old_name.as_str()) {
                        *r = Some(new_name.clone());
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{kittest::Queryable, Harness};

    fn temp_editor(tag: &str) -> (PathBuf, crate::project::Project) {
        let dir = crate::room::tests::temp_project(tag);
        crate::fx::create(&mut crate::project::Project::open(&dir).unwrap(), crate::fx::FxKind::Particles, "ps_test").unwrap();
        let p = crate::project::Project::open(&dir).unwrap();
        (dir, p)
    }

    #[test]
    fn preview_simulates_and_edits_round_trip() {
        let (dir, p) = temp_editor("parts_sim");
        let path = p.fx.path(crate::fx::FxKind::Particles, "ps_test").unwrap();
        let mut ed = ParticleEditor::open("ps_test", &path).unwrap();
        // one second of preview at 60 steps per second
        for _ in 0..60 {
            ed.advance(&p, 1.0 / 60.0);
        }
        assert!(ed.live() > 50, "the template streams 2 particles per step, got {}", ed.live());
        // renaming a type keeps the emitter pointing at it (done by the form) and saving writes TOML
        ed.data.emitters[0].count = 5;
        assert!(ed.dirty());
        ed.save().unwrap();
        let again = PartSysToml::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(again.emitters[0].count, 5);
        ed.hist.track(&ed.data, false); // what ui() does every frame
        ed.undo();
        assert_eq!(ed.data.emitters[0].count, 2, "undo goes back to the opened state");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn presets_are_valid_and_merge_with_unique_names() {
        let (dir, p) = temp_editor("parts_presets");
        let path = p.fx.path(crate::fx::FxKind::Particles, "ps_test").unwrap();
        let mut ed = ParticleEditor::open("ps_test", &path).unwrap();
        for (label, types, emitters) in presets() {
            ed.add_preset(types.clone(), emitters.clone());
            ed.add_preset(types, emitters); // twice: the second copy gets new names
            let errs = ed.data.validate(&|_| false);
            assert!(errs.is_empty(), "{label}: {errs:?}");
        }
        for _ in 0..30 {
            ed.advance(&p, 1.0 / 60.0);
        }
        assert!(ed.live() > 0);
        // a burst-only system keeps repeating in the preview
        ed.data = PartSysToml::default();
        let (_, types, emitters) = presets().into_iter().find(|p| p.0.contains("Explosi")).unwrap();
        ed.add_preset(types, emitters);
        let mut seen_empty_then_full = false;
        let mut was_empty = false;
        for _ in 0..400 {
            ed.advance(&p, 1.0 / 60.0);
            if ed.live() == 0 {
                was_empty = true;
            } else if was_empty {
                seen_empty_then_full = true;
            }
        }
        assert!(!was_empty || seen_empty_then_full);
        assert!(ed.live() > 0, "the explosion fires again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    struct St {
        ed: ParticleEditor,
        project: crate::project::Project,
        tex: crate::tex::TexCache,
        log: Vec<crate::app::LogLine>,
        open: Vec<crate::app::Open>,
    }

    #[test]
    fn editor_ui_renders_and_drags_regions() {
        let (dir, project) = temp_editor("parts_ui");
        let path = project.fx.path(crate::fx::FxKind::Particles, "ps_test").unwrap();
        let ed = ParticleEditor::open("ps_test", &path).unwrap();
        let st = St { ed, project, tex: crate::tex::TexCache::default(), log: vec![], open: vec![] };
        let mut h = Harness::builder().with_size([1000.0, 640.0]).allow_missing_glyphs().with_accessibility_check(false).build_ui_state(
            |ui, s: &mut St| {
                let mut cx = Cx { project: &mut s.project, tex: &mut s.tex, log: &mut s.log, open: &mut s.open, active: true };
                s.ed.ui(ui, &mut cx);
            },
            st,
        );
        h.run_steps(3);
        h.get_by_label("Tipos de partícula");
        assert!(h.state().ed.live() > 0, "the preview runs");
        // drag the selected emitter's region by 20 world px to the right
        let before = h.state().ed.data.emitters[0].region;
        let v = h.state().ed.view;
        let from = v.to_s(Pos2::new(0.0, 0.0));
        let to = from + Vec2::new(20.0 * v.zoom, 0.0);
        crate::room::tests::drag(&mut h, from, to);
        let after = h.state().ed.data.emitters[0].region;
        assert_eq!(after[0], before[0] + 20.0, "{before:?} -> {after:?}");
        assert_eq!(after[3], before[3]);
        drop(h);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
