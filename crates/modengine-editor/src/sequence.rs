//! Sequence editor: a canvas preview with play / pause / scrub, a dopesheet of tracks and
//! keyframes, and an inspector for the selected track, keyframe and the moments.

use crate::app::Cx;
use crate::motion::{self, History, View};
use crate::project::Project;
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use modengine::sequence::{sample, write_toml, Key, Playback, SeqMomentToml, SequenceToml, TrackKind, TrackToml, PROPS};
use std::path::{Path, PathBuf};

const LANE_H: f32 = 24.0;
const NAMES_W: f32 = 170.0;
const PROP_LABELS: [&str; 7] = ["x", "y", "ángulo", "escala x", "escala y", "alpha", "frame"];
const EASES: [&str; 8] = ["linear", "smooth", "step", "ease_in", "ease_out", "ease_in_out", "bounce", ""];

pub struct SequenceEditor {
    pub name: String,
    file: PathBuf,
    pub data: SequenceToml,
    saved: SequenceToml,
    pub hist: History<SequenceToml>,
    pub head: f64,
    pub playing: bool,
    dir: f64,
    last_time: Option<f64>,
    pub track: Option<usize>,
    pub key: Option<usize>,
    view: View,
    canvas_drag: Option<(usize, Vec2)>,
    key_drag: Option<(usize, usize)>,
    /// Screen rectangle of the key lanes (frame ↔ x mapping).
    lanes: Rect,
    curves: Vec<modengine::animcurve::AnimCurve>,
    curves_at: Option<f64>,
}

fn track_color(k: TrackKind) -> Color32 {
    match k {
        TrackKind::Sprite => Color32::from_rgb(96, 170, 255),
        TrackKind::Object => Color32::from_rgb(140, 220, 120),
        TrackKind::Sound => Color32::from_rgb(255, 170, 80),
    }
}

fn track_icon(k: TrackKind) -> &'static str {
    match k {
        TrackKind::Sprite => "▣",
        TrackKind::Object => "◆",
        TrackKind::Sound => "♪",
    }
}

/// Sprite drawn for a track (the sprite itself or the object's sprite).
fn track_sprite(p: &Project, t: &TrackToml) -> Option<String> {
    match t.kind {
        TrackKind::Sprite => Some(t.asset.clone()),
        TrackKind::Object => p.object_sprite(&t.asset),
        TrackKind::Sound => None,
    }
}

impl SequenceEditor {
    pub fn open(name: &str, file: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
        let mut data: SequenceToml = toml::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;
        for t in &mut data.tracks {
            t.sort();
        }
        Ok(SequenceEditor {
            name: name.into(),
            file: file.to_path_buf(),
            hist: History::new(&data),
            saved: data.clone(),
            data,
            head: 0.0,
            playing: false,
            dir: 1.0,
            last_time: None,
            track: None,
            key: None,
            view: View::default(),
            canvas_drag: None,
            key_drag: None,
            lanes: Rect::NOTHING,
            curves: vec![],
            curves_at: None,
        })
    }

    pub fn dirty(&self) -> bool {
        self.data != self.saved
    }

    pub fn save(&mut self, _cx: &mut Cx) -> Result<(), String> {
        for t in &mut self.data.tracks {
            t.sort();
        }
        motion::write_file(&self.file, &write_toml(&self.data)?)?;
        self.saved = self.data.clone();
        Ok(())
    }

    fn canvas_size(&self, p: &Project) -> Vec2 {
        let [w, h] = self.data.canvas.unwrap_or([p.game.display.width, p.game.display.height]);
        Vec2::new(w as f32, h as f32)
    }

    /// Adds a track of `kind` showing `asset`, with a first key in the middle of the canvas.
    pub fn add_track(&mut self, kind: TrackKind, asset: &str, p: &Project) {
        let c = self.canvas_size(p) / 2.0;
        let n = self.data.tracks.iter().filter(|t| t.kind == kind).count() + 1;
        let key = Key { frame: 0.0, x: Some(c.x as f64), y: Some(c.y as f64), ..Default::default() };
        let keys = if kind == TrackKind::Sound { vec![] } else { vec![key] };
        self.data.tracks.push(TrackToml { name: format!("{}{n}", kind.name()), kind, asset: asset.into(), start: 0.0, end: None, keys });
        self.track = Some(self.data.tracks.len() - 1);
        self.key = None;
    }

    /// Sets (or creates) a key of track `ti` at the current frame with every property
    /// sampled from the animation, then applies `f` to it. Returns the key index.
    pub fn key_here(&mut self, ti: usize, f: impl FnOnce(&mut Key)) -> usize {
        let frame = self.head.round();
        let t = &mut self.data.tracks[ti];
        let i = match t.keys.iter().position(|k| k.frame == frame) {
            Some(i) => i,
            None => {
                let none = |_: &str, _: f64| None;
                let p = sample(t, frame, &none);
                let ease = t.keys.iter().rev().find(|k| k.frame < frame).map(|k| k.ease.clone()).unwrap_or_default();
                t.keys.push(Key { frame, x: Some(p.x), y: Some(p.y), angle: Some(p.angle), scale_x: Some(p.scale_x), scale_y: Some(p.scale_y), alpha: Some(p.alpha), image: p.image, ease });
                t.sort();
                t.keys.iter().position(|k| k.frame == frame).unwrap_or(0)
            }
        };
        f(&mut t.keys[i]);
        i
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        self.advance(ui);
        egui::Panel::top(("seq_bar", &self.name)).resizable(false).show(ui, |ui| self.toolbar(ui, cx));
        egui::Panel::right(("seq_insp", &self.name)).resizable(true).default_size(270.0).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.inspector(ui, cx));
        });
        egui::Panel::bottom(("seq_dope", &self.name)).resizable(true).default_size(190.0).show(ui, |ui| self.dopesheet(ui, cx));
        egui::CentralPanel::default().show(ui, |ui| self.canvas(ui, cx));
        self.data.length = self.data.length.max(1.0).round();
        self.data.fps = self.data.fps.max(1.0);
        self.head = self.head.clamp(0.0, self.data.length);
        let busy = self.canvas_drag.is_some() || self.key_drag.is_some();
        self.hist.track(&self.data, busy);
    }

    /// Moves the playhead while playing, following the playback mode.
    fn advance(&mut self, ui: &egui::Ui) {
        let now = motion::now(ui);
        if self.playing {
            if let Some(t) = self.last_time {
                let len = self.data.length;
                self.head += (now - t) * self.data.fps * self.dir;
                match self.data.playback {
                    Playback::Oneshot => {
                        if self.head >= len {
                            self.head = len;
                            self.playing = false;
                        }
                    }
                    Playback::Loop => self.head = self.head.rem_euclid(len),
                    Playback::Pingpong => {
                        if self.head >= len {
                            self.head = 2.0 * len - self.head;
                            self.dir = -1.0;
                        } else if self.head <= 0.0 {
                            self.head = -self.head;
                            self.dir = 1.0;
                        }
                    }
                }
            }
            ui.ctx().request_repaint();
        }
        self.last_time = Some(now);
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        ui.horizontal(|ui| {
            ui.heading(&self.name);
            ui.separator();
            if ui.button(if self.playing { "⏸" } else { "▶" }).on_hover_text("reproducir / pausar").clicked() {
                if !self.playing && self.head >= self.data.length && self.data.playback == Playback::Oneshot {
                    self.head = 0.0;
                }
                self.playing = !self.playing;
                self.dir = 1.0;
            }
            if ui.button("⏮").on_hover_text("al principio").clicked() {
                self.head = 0.0;
            }
            if ui.button("⏭").on_hover_text("al final").clicked() {
                self.head = self.data.length;
            }
            ui.add(egui::DragValue::new(&mut self.head).range(0.0..=self.data.length).speed(0.25).max_decimals(1).prefix("frame "));
            ui.label(format!("/ {}", self.data.length));
            ui.separator();
            ui.add(egui::DragValue::new(&mut self.data.length).range(1.0..=100_000.0).speed(1.0).prefix("duración "));
            ui.add(egui::DragValue::new(&mut self.data.fps).range(1.0..=240.0).speed(0.5).prefix("fps "));
            egui::ComboBox::from_id_salt(("seq_play", &self.name)).selected_text(match self.data.playback {
                Playback::Oneshot => "una vez",
                Playback::Loop => "bucle",
                Playback::Pingpong => "ping-pong",
            }).show_ui(ui, |ui| {
                ui.selectable_value(&mut self.data.playback, Playback::Oneshot, "una vez");
                ui.selectable_value(&mut self.data.playback, Playback::Loop, "bucle");
                ui.selectable_value(&mut self.data.playback, Playback::Pingpong, "ping-pong");
            });
            ui.separator();
            let mut size = self.canvas_size(cx.project);
            ui.label("lienzo");
            let a = ui.add(egui::DragValue::new(&mut size.x).range(16.0..=4096.0));
            let b = ui.add(egui::DragValue::new(&mut size.y).range(16.0..=4096.0));
            if a.changed() || b.changed() {
                self.data.canvas = Some([size.x as u32, size.y as u32]);
            }
            if ui.button("⤢").on_hover_text("encajar el lienzo").clicked() {
                self.view.fitted = false;
            }
        });
    }

    fn canvas(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        let (resp, painter) = motion::canvas(ui);
        let size = self.canvas_size(cx.project);
        let world = Rect::from_min_size(Pos2::ZERO, size);
        self.view.update(ui, &resp, world.expand(16.0));
        let pal = crate::theme::colors(ui.ctx());
        painter.rect_filled(resp.rect, 0.0, pal.canvas);
        let v = self.view;
        let crect = Rect::from_min_max(v.to_s(Pos2::ZERO), v.to_s(Pos2::ZERO + size));
        crate::tex::checkerboard(&painter, crect, (16.0 * v.zoom).max(6.0));
        painter.rect_stroke(crect, 0.0, Stroke::new(1.0, pal.canvas_border), egui::StrokeKind::Outside);
        let o = v.to_s(Pos2::new(self.data.origin[0] as f32, self.data.origin[1] as f32));
        painter.line_segment([o - Vec2::new(8.0, 0.0), o + Vec2::new(8.0, 0.0)], Stroke::new(1.0, Color32::from_rgb(255, 80, 80)));
        painter.line_segment([o - Vec2::new(0.0, 8.0), o + Vec2::new(0.0, 8.0)], Stroke::new(1.0, Color32::from_rgb(255, 80, 80)));

        // ---- the tracks at the playhead, first track at the back
        let ctx = ui.ctx().clone();
        // animation curves used as easings (re-read from disk at most once a second)
        let now = motion::now(ui);
        if self.curves_at.is_none_or(|t| now - t > 1.0) {
            self.curves = cx.project.motion.items.iter().filter(|(k, _)| *k == motion::AnimKind::Curve).filter_map(|(_, f)| modengine::animcurve::AnimCurve::load(&f.name, &f.path).ok()).collect();
            self.curves_at = Some(now);
        }
        let curves = std::mem::take(&mut self.curves);
        let curve_ease = |name: &str, t: f64| modengine::sequence::curve_ease(&curves, name, t);
        let mut boxes: Vec<(usize, Rect)> = vec![];
        for (ti, t) in self.data.tracks.iter().enumerate() {
            if t.kind == TrackKind::Sound || !t.active(self.head, self.data.length) {
                continue;
            }
            let p = sample(t, self.head, &curve_ease);
            let at = v.to_s(Pos2::new(p.x as f32, p.y as f32));
            let alpha = (p.alpha.clamp(0.0, 1.0) * 255.0) as u8;
            let spr = track_sprite(cx.project, t);
            let drawn = spr.as_deref().and_then(|s| cx.tex.get(&ctx, cx.project, s)).map(|tex| {
                let n = tex.spr.frames.len().max(1);
                let frame = match p.image {
                    Some(f) => f.rem_euclid(n as f64) as usize,
                    None => (((self.head - t.start) / self.data.fps * tex.spr.fps).floor().max(0.0) as usize) % n,
                };
                let fs = tex.frame_size() * v.zoom;
                let org = tex.origin() * v.zoom;
                let (sx, sy) = (p.scale_x as f32, p.scale_y as f32);
                let mut mesh = egui::Mesh::with_texture(tex.id);
                let min = at - Vec2::new(org.x * sx, org.y * sy);
                let r = Rect::from_two_pos(min, min + Vec2::new(fs.x * sx, fs.y * sy));
                let mut uv = tex.uv(frame);
                if sx < 0.0 {
                    std::mem::swap(&mut uv.min.x, &mut uv.max.x);
                }
                if sy < 0.0 {
                    std::mem::swap(&mut uv.min.y, &mut uv.max.y);
                }
                mesh.add_rect_with_uv(r, uv, Color32::from_white_alpha(alpha));
                // ModScript angles are counter-clockwise
                mesh.rotate(egui::emath::Rot2::from_angle(-(p.angle as f32).to_radians()), at);
                let bb = mesh.calc_bounds();
                painter.add(egui::Shape::mesh(mesh));
                bb
            });
            let bb = drawn.unwrap_or_else(|| {
                painter.circle_filled(at, 6.0, track_color(t.kind).gamma_multiply(p.alpha as f32));
                Rect::from_center_size(at, Vec2::splat(12.0))
            });
            if t.kind == TrackKind::Object {
                painter.text(bb.left_top(), egui::Align2::LEFT_BOTTOM, &t.asset, egui::FontId::proportional(10.0), Color32::from_white_alpha(160));
            }
            boxes.push((ti, bb));
        }
        self.curves = curves.clone();
        if let Some((_, bb)) = boxes.iter().find(|(ti, _)| Some(*ti) == self.track) {
            painter.rect_stroke(bb.expand(2.0), 0.0, Stroke::new(1.5, pal.selection), egui::StrokeKind::Outside);
        }
        // motion trail of the selected track
        if let Some(t) = self.track.and_then(|i| self.data.tracks.get(i)).filter(|t| t.kind != TrackKind::Sound && t.keys.len() > 1) {
            let n = (self.data.length as usize).clamp(2, 400);
            let pts: Vec<Pos2> = (0..=n).map(|i| i as f64 * self.data.length / n as f64).map(|f| sample(t, f, &curve_ease)).map(|p| v.to_s(Pos2::new(p.x as f32, p.y as f32))).collect();
            painter.add(egui::Shape::dashed_line(&pts, Stroke::new(1.0, pal.selection.gamma_multiply(0.7)), 3.0, 3.0));
            for k in &t.keys {
                let p = sample(t, k.frame, &curve_ease);
                painter.circle_stroke(v.to_s(Pos2::new(p.x as f32, p.y as f32)), 3.0, Stroke::new(1.0, pal.selection));
            }
        }

        // ---- drag a track on the canvas: keys its position at the current frame
        if View::panning(ui) {
            return;
        }
        if resp.drag_started_by(egui::PointerButton::Primary) {
            if let Some(pos) = ui.input(|i| i.pointer.press_origin()) {
                if let Some((ti, _)) = boxes.iter().rev().find(|(_, bb)| bb.expand(3.0).contains(pos)) {
                    let p = sample(&self.data.tracks[*ti], self.head, &curve_ease);
                    let grab = v.to_w(pos) - Pos2::new(p.x as f32, p.y as f32);
                    self.canvas_drag = Some((*ti, grab));
                    self.track = Some(*ti);
                    self.playing = false;
                }
            }
        }
        if let (Some((ti, grab)), Some(pos)) = (self.canvas_drag, resp.interact_pointer_pos()) {
            if resp.dragged_by(egui::PointerButton::Primary) {
                let w = v.to_w(pos) - grab;
                let k = self.key_here(ti, |k| {
                    k.x = Some(w.x.round() as f64);
                    k.y = Some(w.y.round() as f64);
                });
                self.key = Some(k);
            }
        }
        if resp.drag_stopped() {
            self.canvas_drag = None;
        }
        if resp.clicked_by(egui::PointerButton::Primary) {
            let hit = resp.interact_pointer_pos().and_then(|pos| boxes.iter().rev().find(|(_, bb)| bb.expand(3.0).contains(pos)).map(|(ti, _)| *ti));
            self.track = hit;
            self.key = None;
        }
    }

    fn dopesheet(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        let pal = crate::theme::colors(ui.ctx());
        ui.horizontal(|ui| {
            ui.strong("Pistas");
            ui.menu_button("+ pista", |ui| {
                let p: &Project = cx.project;
                ui.menu_button("▣ Sprite", |ui| {
                    for s in p.sprite_names() {
                        if ui.button(&s).clicked() {
                            self.add_track(TrackKind::Sprite, &s, p);
                            ui.close();
                        }
                    }
                });
                ui.menu_button("◆ Objeto", |ui| {
                    for o in p.object_names() {
                        if ui.button(&o).clicked() {
                            self.add_track(TrackKind::Object, &o, p);
                            ui.close();
                        }
                    }
                });
                ui.menu_button("♪ Sonido", |ui| {
                    for s in p.sounds.iter().map(|s| s.name.clone()) {
                        if ui.button(&s).clicked() {
                            self.add_track(TrackKind::Sound, &s, p);
                            ui.close();
                        }
                    }
                });
            });
            if let Some(ti) = self.track.filter(|&t| t < self.data.tracks.len() && self.data.tracks[t].kind != TrackKind::Sound) {
                if ui.button("◆ clave aquí").on_hover_text("añade una clave en el frame actual con los valores interpolados").clicked() {
                    let k = self.key_here(ti, |_| {});
                    self.key = Some(k);
                }
            }
            ui.label(RichText::new("ⓘ").weak()).on_hover_text(
                "clic: elegir pista o mover el cursor\ndoble clic en una pista: clave en ese frame\ndoble clic en «momentos»: momento\narrastrar ◆: mover la clave · clic derecho ◆: borrarla\nen el lienzo, arrastrar una pista crea/mueve su clave del frame actual",
            );
        });
        let rows = self.data.tracks.len() + 1; // + moments row
        let height = (rows as f32 * LANE_H + 22.0).max(ui.available_height());
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let (resp, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), height), Sense::click_and_drag());
            let r = resp.rect;
            painter.rect_filled(r, 0.0, pal.canvas);
            let lanes = Rect::from_min_max(Pos2::new(r.left() + NAMES_W, r.top() + 20.0), Pos2::new(r.right() - 10.0, r.bottom()));
            self.lanes = lanes;
            let len = self.data.length;
            let x_of = |f: f64| lanes.left() + (f / len) as f32 * lanes.width();
            let f_of = |x: f32| (((x - lanes.left()) / lanes.width()) as f64 * len).round().clamp(0.0, len);
            let font = egui::FontId::proportional(10.0);
            // ruler
            let every = [1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0].into_iter().find(|e| lanes.width() as f64 / (len / e) >= 36.0).unwrap_or(600.0);
            let mut f = 0.0;
            while f <= len {
                let x = x_of(f);
                painter.line_segment([Pos2::new(x, r.top() + 12.0), Pos2::new(x, r.bottom())], Stroke::new(1.0, Color32::from_white_alpha(16)));
                painter.text(Pos2::new(x, r.top() + 2.0), egui::Align2::CENTER_TOP, format!("{f:.0}"), font.clone(), pal.weak);
                f += every;
            }
            // moments row
            let row_rect = |i: usize| Rect::from_min_size(Pos2::new(r.left(), lanes.top() + i as f32 * LANE_H), Vec2::new(r.width(), LANE_H));
            let mr = row_rect(0);
            painter.text(Pos2::new(r.left() + 8.0, mr.center().y), egui::Align2::LEFT_CENTER, "⚑ momentos", font.clone(), pal.canvas_text);
            for m in &self.data.moments {
                let c = Pos2::new(x_of(m.frame), mr.center().y);
                painter.add(egui::Shape::convex_polygon(vec![c + Vec2::new(-5.0, -7.0), c + Vec2::new(5.0, -7.0), c + Vec2::new(0.0, 6.0)], Color32::from_rgb(255, 220, 90), Stroke::new(1.0, Color32::BLACK)));
            }
            // track rows
            for (ti, t) in self.data.tracks.iter().enumerate() {
                let rr = row_rect(ti + 1);
                if Some(ti) == self.track {
                    painter.rect_filled(rr, 0.0, pal.selection.gamma_multiply(0.18));
                }
                let col = track_color(t.kind);
                painter.text(Pos2::new(r.left() + 8.0, rr.center().y), egui::Align2::LEFT_CENTER, format!("{} {}", track_icon(t.kind), t.name), font.clone(), col);
                let bar = Rect::from_min_max(Pos2::new(x_of(t.start), rr.top() + 5.0), Pos2::new(x_of(t.end_or(len).min(len)), rr.bottom() - 5.0));
                painter.rect_filled(bar, 3.0, col.gamma_multiply(0.22));
                for (ki, k) in t.keys.iter().enumerate() {
                    let c = Pos2::new(x_of(k.frame), rr.center().y);
                    let sel = Some(ti) == self.track && Some(ki) == self.key;
                    let d = if sel { 7.0 } else { 5.5 };
                    let fill = if sel { pal.selection } else { col };
                    painter.add(egui::Shape::convex_polygon(vec![c + Vec2::new(0.0, -d), c + Vec2::new(d, 0.0), c + Vec2::new(0.0, d), c + Vec2::new(-d, 0.0)], fill, Stroke::new(1.0, Color32::BLACK)));
                }
                if t.kind == TrackKind::Sound {
                    painter.text(Pos2::new(x_of(t.start) + 4.0, rr.center().y), egui::Align2::LEFT_CENTER, format!("♪ {}", t.asset), font.clone(), col);
                }
            }
            // playhead
            let hx = x_of(self.head);
            painter.line_segment([Pos2::new(hx, r.top()), Pos2::new(hx, r.bottom())], Stroke::new(2.0, Color32::from_rgb(255, 80, 80)));
            painter.line_segment([Pos2::new(lanes.left(), r.top()), Pos2::new(lanes.left(), r.bottom())], Stroke::new(1.0, pal.canvas_border));

            // ---- interaction
            let row_at = |y: f32| -> Option<usize> {
                if y < lanes.top() {
                    return None;
                }
                Some(((y - lanes.top()) / LANE_H) as usize)
            };
            let key_at = |s: &Self, pos: Pos2| -> Option<(usize, usize)> {
                let row = row_at(pos.y)?;
                let ti = row.checked_sub(1)?;
                let t = s.data.tracks.get(ti)?;
                t.keys.iter().position(|k| (x_of(k.frame) - pos.x).abs() <= 7.0).map(|ki| (ti, ki))
            };
            if resp.drag_started_by(egui::PointerButton::Primary) {
                if let Some(pos) = ui.input(|i| i.pointer.press_origin()) {
                    self.key_drag = key_at(self, pos);
                    if let Some((ti, ki)) = self.key_drag {
                        self.track = Some(ti);
                        self.key = Some(ki);
                    }
                }
            }
            if let Some(pos) = resp.interact_pointer_pos() {
                if resp.dragged_by(egui::PointerButton::Primary) {
                    match self.key_drag {
                        Some((ti, ki)) => {
                            let f = f_of(pos.x);
                            let t = &mut self.data.tracks[ti];
                            if !t.keys.iter().enumerate().any(|(i, k)| i != ki && k.frame == f) {
                                t.keys[ki].frame = f;
                                let moved = t.keys[ki].clone();
                                t.sort();
                                let ni = t.keys.iter().position(|k| *k == moved).unwrap_or(ki);
                                self.key_drag = Some((ti, ni));
                                self.key = Some(ni);
                            }
                            self.head = f;
                        }
                        None if pos.x >= lanes.left() => {
                            self.head = ((pos.x - lanes.left()) / lanes.width()) as f64 * len;
                            self.playing = false;
                        }
                        None => {}
                    }
                }
                if resp.clicked_by(egui::PointerButton::Primary) {
                    match key_at(self, pos) {
                        Some((ti, ki)) => {
                            self.track = Some(ti);
                            self.key = Some(ki);
                            self.head = self.data.tracks[ti].keys[ki].frame;
                        }
                        None => {
                            if let Some(ti) = row_at(pos.y).and_then(|r| r.checked_sub(1)).filter(|&t| t < self.data.tracks.len()) {
                                self.track = Some(ti);
                                self.key = None;
                            }
                            if pos.x >= lanes.left() {
                                self.head = f_of(pos.x);
                                self.playing = false;
                            }
                        }
                    }
                }
                if resp.double_clicked() && pos.x >= lanes.left() {
                    self.head = f_of(pos.x);
                    match row_at(pos.y) {
                        Some(0) => {
                            let frame = self.head;
                            if !self.data.moments.iter().any(|m| m.frame == frame) {
                                self.data.moments.push(SeqMomentToml { frame, code: String::new() });
                            }
                        }
                        Some(r) if r - 1 < self.data.tracks.len() && self.data.tracks[r - 1].kind != TrackKind::Sound => {
                            let k = self.key_here(r - 1, |_| {});
                            self.track = Some(r - 1);
                            self.key = Some(k);
                        }
                        _ => {}
                    }
                }
                if resp.clicked_by(egui::PointerButton::Secondary) {
                    if let Some((ti, ki)) = key_at(self, pos) {
                        self.data.tracks[ti].keys.remove(ki);
                        self.key = None;
                    }
                }
            }
            if resp.drag_stopped() {
                self.key_drag = None;
            }
        });
    }

    fn inspector(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        let curves = cx.project.motion.names(motion::AnimKind::Curve);
        let Some(ti) = self.track.filter(|&t| t < self.data.tracks.len()) else {
            ui.strong("Secuencia");
            ui.label(RichText::new(format!("{} pistas · {} frames a {} fps ({:.2} s)", self.data.tracks.len(), self.data.length, self.data.fps, self.data.length / self.data.fps)).weak());
            ui.horizontal(|ui| {
                ui.label("origen");
                ui.add(egui::DragValue::new(&mut self.data.origin[0]).speed(1.0));
                ui.add(egui::DragValue::new(&mut self.data.origin[1]).speed(1.0));
            });
            ui.label(RichText::new("Elige una pista en el lienzo o en la hoja de claves.").weak());
            self.moments(ui);
            return;
        };
        let p: &Project = cx.project;
        let mut delete = false;
        {
            let t = &mut self.data.tracks[ti];
            ui.horizontal(|ui| {
                ui.strong(format!("{} Pista", track_icon(t.kind)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("🗑").on_hover_text("borrar la pista").clicked() {
                        delete = true;
                    }
                });
            });
            ui.horizontal(|ui| {
                ui.label("nombre");
                ui.text_edit_singleline(&mut t.name);
            });
            let options = match t.kind {
                TrackKind::Sprite => p.sprite_names(),
                TrackKind::Object => p.object_names(),
                TrackKind::Sound => p.sounds.iter().map(|s| s.name.clone()).collect(),
            };
            ui.horizontal(|ui| {
                ui.label(t.kind.name());
                let mut v = Some(t.asset.clone());
                if motion::combo(ui, ("seq_asset", ti), &mut v, &options, "") {
                    t.asset = v.unwrap_or_default();
                }
            });
            ui.horizontal(|ui| {
                ui.label("desde");
                ui.add(egui::DragValue::new(&mut t.start).range(0.0..=self.data.length).speed(1.0));
                let mut has_end = t.end.is_some();
                if ui.checkbox(&mut has_end, "hasta").changed() {
                    t.end = if has_end { Some(self.data.length) } else { None };
                }
                if let Some(e) = &mut t.end {
                    ui.add(egui::DragValue::new(e).range(0.0..=self.data.length).speed(1.0));
                }
            });
            if t.kind == TrackKind::Sound {
                ui.label(RichText::new("El sonido suena cuando el cursor pasa por «desde».").weak());
            }
        }
        if delete {
            self.data.tracks.remove(ti);
            self.track = None;
            self.key = None;
            return;
        }
        ui.separator();
        let t = &mut self.data.tracks[ti];
        match self.key.filter(|&k| k < t.keys.len()) {
            Some(ki) => {
                ui.horizontal(|ui| {
                    ui.strong("◆ Clave");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("🗑").on_hover_text("borrar la clave").clicked() {
                            t.keys.remove(ki);
                            self.key = None;
                        }
                    });
                });
                if self.key.is_none() {
                    return;
                }
                let len = self.data.length;
                let k = &mut t.keys[ki];
                ui.add(egui::DragValue::new(&mut k.frame).range(0.0..=len).speed(1.0).prefix("frame "));
                egui::Grid::new(("seq_key", ti, ki)).num_columns(2).spacing([6.0, 3.0]).show(ui, |ui| {
                    for (pi, label) in PROP_LABELS.iter().enumerate() {
                        let slot = k.slot(pi);
                        let mut on = slot.is_some();
                        if ui.checkbox(&mut on, *label).on_hover_text(format!("propiedad «{}» con clave aquí", PROPS[pi])).changed() {
                            *slot = if on { Some([0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0][pi]) } else { None };
                        }
                        if let Some(v) = slot {
                            let speed = if pi == 5 { 0.01 } else if pi >= 3 && pi <= 4 { 0.02 } else { 1.0 };
                            ui.add(egui::DragValue::new(v).speed(speed).max_decimals(3));
                        } else {
                            ui.label(RichText::new("—").weak());
                        }
                        ui.end_row();
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("suavizado →");
                    egui::ComboBox::from_id_salt(("seq_ease", ti, ki)).selected_text(if k.ease.is_empty() { "linear".to_string() } else { k.ease.clone() }).show_ui(ui, |ui| {
                        for e in EASES.iter().filter(|e| !e.is_empty()) {
                            ui.selectable_value(&mut k.ease, if *e == "linear" { String::new() } else { e.to_string() }, *e);
                        }
                        ui.menu_button("más (GameMaker) ▸", |ui| {
                            egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                                for e in modengine::animcurve::MORE_PRESETS {
                                    if ui.selectable_label(k.ease == *e, *e).clicked() {
                                        k.ease = e.to_string();
                                        ui.close();
                                    }
                                }
                            });
                        });
                        if !curves.is_empty() {
                            ui.separator();
                            ui.label(RichText::new("curvas de animación").weak());
                            for c in &curves {
                                ui.selectable_value(&mut k.ease, c.clone(), format!("📈 {c}"));
                            }
                        }
                    });
                });
                ui.label(RichText::new("cómo se llega a la clave siguiente").weak().small());
            }
            None => {
                ui.label(RichText::new(format!("{} claves. Haz clic en una ◆ para editarla.", t.keys.len())).weak());
            }
        }
        ui.separator();
        self.moments(ui);
    }

    fn moments(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("⚑ Momentos");
            if ui.small_button("+").on_hover_text("código ModScript que se ejecuta al pasar por el frame actual").clicked() {
                let frame = self.head.round();
                if !self.data.moments.iter().any(|m| m.frame == frame) {
                    self.data.moments.push(SeqMomentToml { frame, code: String::new() });
                }
            }
        });
        let mut del = None;
        let len = self.data.length;
        for (i, m) in self.data.moments.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut m.frame).range(0.0..=len).speed(1.0).prefix("frame "));
                if ui.small_button("×").clicked() {
                    del = Some(i);
                }
            });
            crate::highlight::code_editor(ui, &mut m.code, ("seq_moment", &self.name, i), 2);
            if let Err(e) = modscript::parse_stmts("<moment>", &m.code) {
                ui.colored_label(crate::theme::colors(ui.ctx()).error, format!("línea {}: {}", e.line, e.msg));
            }
        }
        if let Some(i) = del {
            self.data.moments.remove(i);
        }
        self.data.moments.sort_by(|a, b| a.frame.partial_cmp(&b.frame).unwrap_or(std::cmp::Ordering::Equal));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{LogLine, Open};
    use crate::tex::TexCache;
    use egui_kittest::kittest::Queryable;
    use egui_kittest::Harness;

    struct St {
        ed: SequenceEditor,
        project: Project,
        tex: TexCache,
        log: Vec<LogLine>,
        open: Vec<Open>,
    }

    #[test]
    fn tracks_keys_canvas_drag_and_save() {
        let dir = crate::room::tests::temp_project("seq_ed");
        let mut project = Project::open(&dir).unwrap();
        motion::create(&mut project, motion::AnimKind::Sequence, "intro").unwrap();
        let file = project.motion.path(motion::AnimKind::Sequence, "intro").unwrap();
        let mut ed = SequenceEditor::open("intro", &file).unwrap();
        let sprite = project.sprite_names()[0].clone();
        ed.add_track(TrackKind::Sprite, &sprite, &project);
        let st = St { ed, project, tex: TexCache::default(), log: vec![], open: vec![] };
        let mut h = Harness::builder().with_size([1200.0, 760.0]).allow_missing_glyphs().with_accessibility_check(false).build_ui_state(
            |ui, s: &mut St| {
                let mut cx = Cx { project: &mut s.project, tex: &mut s.tex, log: &mut s.log, open: &mut s.open, active: true };
                s.ed.ui(ui, &mut cx);
            },
            st,
        );
        h.run_steps(3);
        // click the track lane at frame 30 (selects the track, moves the head), then key it
        let (lanes, len) = (h.state().ed.lanes, h.state().ed.data.length);
        let at = Pos2::new(lanes.left() + (30.0 / len) as f32 * lanes.width(), lanes.top() + LANE_H * 1.5);
        h.event(egui::Event::PointerMoved(at));
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run_steps(2);
        assert_eq!(h.state().ed.head, 30.0);
        assert_eq!(h.state().ed.track, Some(0));
        h.get_by_label("◆ clave aquí").click();
        h.run_steps(2);
        assert_eq!(h.state().ed.data.tracks[0].keys.len(), 2, "a key at the current frame");
        // drag the sprite on the canvas at frame 30: moves the key
        let k = h.state().ed.data.tracks[0].keys[1].clone();
        let from = h.state().ed.view.to_s(Pos2::new(k.x.unwrap() as f32, k.y.unwrap() as f32));
        let to = from + Vec2::new(60.0, -30.0);
        crate::room::tests::drag(&mut h, from, to);
        let k2 = h.state().ed.data.tracks[0].keys[1].clone();
        assert_eq!(k2.frame, 30.0);
        assert!(k2.x.unwrap() > k.x.unwrap() + 5.0 && k2.y.unwrap() < k.y.unwrap() - 2.0, "{k:?} → {k2:?}");
        // halfway the position is interpolated
        let none = |_: &str, _: f64| None;
        let mid = sample(&h.state().ed.data.tracks[0], 15.0, &none);
        let k0 = &h.state().ed.data.tracks[0].keys[0];
        assert!((mid.x - (k0.x.unwrap() + k2.x.unwrap()) / 2.0).abs() < 1e-6);
        // play advances the head
        h.state_mut().ed.data.moments.push(SeqMomentToml { frame: 10.0, code: "global.hit = true".into() });
        {
            let s = h.state_mut();
            let mut cx = Cx { project: &mut s.project, tex: &mut s.tex, log: &mut s.log, open: &mut s.open, active: true };
            s.ed.save(&mut cx).unwrap();
        }
        let e = modengine::Engine::load(&dir).expect("the saved sequence loads");
        assert_eq!(e.sequences[0].data.tracks[0].keys.len(), 2);
        assert_eq!(e.sequences[0].moments.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
