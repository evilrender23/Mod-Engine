//! Animation curve editor: a graph with draggable points (and bezier handles) for each
//! channel, presets (ease in/out, bounce, elastic...) and a live preview of the motion.

use crate::app::Cx;
use crate::motion::{self, History};
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use modengine::animcurve::{preset, write_toml, AnimCurve, Channel, CurvePoint, CurveType, MORE_PRESETS, PRESETS};
use std::path::{Path, PathBuf};

const CHANNEL_COLORS: [Color32; 6] = [
    Color32::from_rgb(255, 196, 64),
    Color32::from_rgb(96, 200, 255),
    Color32::from_rgb(140, 230, 120),
    Color32::from_rgb(255, 120, 160),
    Color32::from_rgb(190, 150, 255),
    Color32::from_rgb(255, 150, 90),
];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Grab {
    Point(usize),
    In(usize),
    Out(usize),
}

pub struct CurveEditor {
    pub name: String,
    file: PathBuf,
    pub data: AnimCurve,
    saved: AnimCurve,
    pub hist: History<AnimCurve>,
    pub chan: usize,
    pub sel: Option<usize>,
    grab: Option<Grab>,
    /// Vertical range shown (frozen while dragging so the graph doesn't jump).
    yr: (f64, f64),
    graph: Rect,
    preview: bool,
    duration: f64,
}

fn auto_range(c: &AnimCurve) -> (f64, f64) {
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    for ch in &c.channels {
        for i in 0..=100 {
            let v = ch.eval(i as f64 / 100.0);
            lo = lo.min(v);
            hi = hi.max(v);
        }
        for p in &ch.points {
            lo = lo.min(p.y);
            hi = hi.max(p.y);
        }
    }
    let pad = (hi - lo).max(1e-6) * 0.08;
    (lo - pad, hi + pad)
}

impl CurveEditor {
    pub fn open(name: &str, file: &Path) -> Result<Self, String> {
        let data = AnimCurve::load(name, file)?;
        Ok(CurveEditor {
            name: name.into(),
            file: file.to_path_buf(),
            hist: History::new(&data),
            saved: data.clone(),
            yr: auto_range(&data),
            data,
            chan: 0,
            sel: None,
            grab: None,
            graph: Rect::NOTHING,
            preview: true,
            duration: 1.5,
        })
    }

    pub fn dirty(&self) -> bool {
        self.data != self.saved
    }

    pub fn save(&mut self, _cx: &mut Cx) -> Result<(), String> {
        for c in &mut self.data.channels {
            c.sort();
        }
        motion::write_file(&self.file, &write_toml(&self.data.to_toml()))?;
        self.saved = self.data.clone();
        Ok(())
    }

    /// Screen position of a graph point (tests).
    #[cfg(test)]
    pub(crate) fn screen_of(&self, x: f64, y: f64) -> Pos2 {
        self.to_s(x, y)
    }

    fn to_s(&self, x: f64, y: f64) -> Pos2 {
        let g = self.graph;
        Pos2::new(g.left() + x as f32 * g.width(), g.bottom() - ((y - self.yr.0) / (self.yr.1 - self.yr.0)) as f32 * g.height())
    }

    fn to_w(&self, p: Pos2) -> (f64, f64) {
        let g = self.graph;
        (((p.x - g.left()) / g.width()) as f64, self.yr.0 + ((g.bottom() - p.y) / g.height()) as f64 * (self.yr.1 - self.yr.0))
    }

    pub fn apply_preset(&mut self, key: &str) {
        if let (Some((kind, pts)), Some(ch)) = (preset(key), self.data.channels.get_mut(self.chan)) {
            ch.kind = kind;
            ch.points = pts;
            self.sel = None;
            self.yr = auto_range(&self.data);
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, _cx: &mut Cx) {
        if self.data.channels.is_empty() {
            self.data.channels.push(Channel::new("curve1", CurveType::Linear, vec![CurvePoint::new(0.0, 0.0), CurvePoint::new(1.0, 1.0)]));
        }
        self.chan = self.chan.min(self.data.channels.len() - 1);
        egui::Panel::left(("curve_side", &self.name)).resizable(true).default_size(250.0).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.side(ui));
        });
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.preview, "▶ vista previa");
                ui.add(egui::Slider::new(&mut self.duration, 0.25..=6.0).text("s").logarithmic(true));
                if ui.button("⤢ ajustar eje Y").clicked() {
                    self.yr = auto_range(&self.data);
                }
                ui.label(RichText::new("ⓘ").weak()).on_hover_text("clic en el gráfico: añadir punto\narrastrar un punto o un tirador (bezier): moverlo\nclic derecho en un punto: borrarlo");
            });
            self.graph_ui(ui);
        });
        self.hist.track(&self.data, self.grab.is_some());
    }

    fn side(&mut self, ui: &mut egui::Ui) {
        ui.heading(&self.name);
        ui.strong("Canales");
        let mut remove = None;
        for (i, c) in self.data.channels.iter().enumerate() {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                ui.painter().circle_filled(r.center(), 4.0, CHANNEL_COLORS[i % CHANNEL_COLORS.len()]);
                if ui.selectable_label(i == self.chan, if c.name.is_empty() { "(sin nombre)" } else { &c.name }).clicked() {
                    self.chan = i;
                    self.sel = None;
                }
                if self.data.channels.len() > 1 && ui.small_button("×").on_hover_text("borrar canal").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            self.data.channels.remove(i);
            self.chan = 0;
            self.sel = None;
            return;
        }
        if ui.button("+ canal").clicked() {
            let n = self.data.channels.len() + 1;
            self.data.channels.push(Channel::new(&format!("curve{n}"), CurveType::Linear, vec![CurvePoint::new(0.0, 0.0), CurvePoint::new(1.0, 1.0)]));
            self.chan = self.data.channels.len() - 1;
            self.sel = None;
        }
        ui.separator();
        let ch = &mut self.data.channels[self.chan];
        ui.horizontal(|ui| {
            ui.label("Nombre:");
            ui.add(egui::TextEdit::singleline(&mut ch.name).desired_width(140.0));
        });
        ui.horizontal(|ui| {
            ui.label("Tipo:");
            for (t, label, tip) in [(CurveType::Linear, "Lineal", "rectas entre puntos"), (CurveType::Smooth, "Suave", "Catmull-Rom: pasa por todos los puntos"), (CurveType::Bezier, "Bezier", "con tiradores en cada punto")] {
                ui.selectable_value(&mut ch.kind, t, label).on_hover_text(tip);
            }
        });
        ui.separator();
        ui.strong("Presets");
        let mut chosen = None;
        egui::Grid::new(("curve_presets", &self.name)).num_columns(2).spacing([4.0, 4.0]).show(ui, |ui| {
            for (i, (k, label)) in PRESETS.iter().enumerate() {
                let (kind, pts) = preset(k).unwrap_or_default();
                let thumb = Channel::new(k, kind, pts);
                // the preset's shape on the left of the button
                let resp = ui.add(egui::Button::new(format!("        {label}")).min_size(Vec2::new(118.0, 22.0)));
                let r = resp.rect.shrink2(Vec2::new(5.0, 4.0));
                let r = Rect::from_min_size(r.min, Vec2::new(22.0, r.height()));
                let (lo, hi) = (-0.3, 1.3);
                let pts: Vec<Pos2> = (0..=24).map(|s| s as f64 / 24.0).map(|x| Pos2::new(r.left() + x as f32 * r.width(), r.bottom() - ((thumb.eval(x) - lo) / (hi - lo)) as f32 * r.height())).collect();
                ui.painter().add(egui::Shape::line(pts, Stroke::new(1.2, CHANNEL_COLORS[0])));
                if resp.on_hover_text("sustituye los puntos del canal").clicked() {
                    chosen = Some(*k);
                }
                if i % 2 == 1 {
                    ui.end_row();
                }
            }
        });
        ui.menu_button("Más curvas de GameMaker ▾", |ui| {
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                for k in MORE_PRESETS {
                    if ui.button(*k).clicked() {
                        chosen = Some(*k);
                        ui.close();
                    }
                }
            });
        });
        if let Some(k) = chosen {
            self.apply_preset(k);
        }
        ui.separator();
        ui.strong("Puntos");
        let ch = &mut self.data.channels[self.chan];
        let mut del = None;
        egui::Grid::new(("curve_pts", &self.name)).num_columns(3).striped(true).spacing([4.0, 2.0]).show(ui, |ui| {
            ui.label(RichText::new("x (0–1)").weak());
            ui.label(RichText::new("valor").weak());
            ui.label("");
            ui.end_row();
            for (i, p) in ch.points.iter_mut().enumerate() {
                let r = ui.add(egui::DragValue::new(&mut p.x).range(0.0..=1.0).speed(0.005).max_decimals(3));
                if r.clicked() || r.dragged() {
                    self.sel = Some(i);
                }
                ui.add(egui::DragValue::new(&mut p.y).speed(0.01).max_decimals(3));
                if ui.small_button("×").clicked() {
                    del = Some(i);
                }
                ui.end_row();
            }
        });
        if let Some(i) = del {
            if ch.points.len() > 1 {
                ch.points.remove(i);
                self.sel = None;
            }
        }
        if self.grab.is_none() {
            ch.sort();
        }
        ui.add_space(6.0);
        let cname = ch.name.clone();
        ui.label(RichText::new(format!("animcurve_evaluate({}, \"{cname}\", x)\nanimcurve_channel_evaluate(animcurve_get_channel({}, \"{cname}\"), x)", self.name, self.name)).weak().small().monospace());
    }

    fn graph_ui(&mut self, ui: &mut egui::Ui) {
        let preview_h = if self.preview { 64.0 } else { 0.0 };
        let (resp, painter) = ui.allocate_painter(ui.available_size() - Vec2::new(0.0, preview_h), Sense::click_and_drag());
        let area = resp.rect;
        let pal = crate::theme::colors(ui.ctx());
        painter.rect_filled(area, 4.0, pal.canvas);
        self.graph = area.shrink2(Vec2::new(46.0, 18.0));
        let g = self.graph;
        if self.yr.1 - self.yr.0 < 1e-9 {
            self.yr = auto_range(&self.data);
        }
        // grid: x quarters, y nice steps
        let font = egui::FontId::proportional(10.0);
        for q in 0..=4 {
            let x = q as f64 / 4.0;
            let a = self.to_s(x, self.yr.0);
            painter.line_segment([Pos2::new(a.x, g.top()), Pos2::new(a.x, g.bottom())], Stroke::new(1.0, Color32::from_white_alpha(if q % 4 == 0 { 50 } else { 18 })));
            painter.text(Pos2::new(a.x, g.bottom() + 3.0), egui::Align2::CENTER_TOP, format!("{x}"), font.clone(), pal.weak);
        }
        let span = self.yr.1 - self.yr.0;
        let step = [0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 25.0, 50.0, 100.0].into_iter().find(|s| span / s <= 8.0).unwrap_or(100.0);
        let mut y = (self.yr.0 / step).ceil() * step;
        while y <= self.yr.1 {
            let p = self.to_s(0.0, y);
            let strong = y.abs() < 1e-9 || (y - 1.0).abs() < 1e-9;
            painter.line_segment([Pos2::new(g.left(), p.y), Pos2::new(g.right(), p.y)], Stroke::new(1.0, Color32::from_white_alpha(if strong { 50 } else { 18 })));
            painter.text(Pos2::new(g.left() - 4.0, p.y), egui::Align2::RIGHT_CENTER, format!("{}", (y * 1000.0).round() / 1000.0), font.clone(), pal.weak);
            y += step;
        }
        // curves (the selected one last, on top)
        let mut order: Vec<usize> = (0..self.data.channels.len()).filter(|&i| i != self.chan).collect();
        order.push(self.chan);
        for ci in order {
            let ch = &self.data.channels[ci];
            let col = CHANNEL_COLORS[ci % CHANNEL_COLORS.len()];
            let col = if ci == self.chan { col } else { col.gamma_multiply(0.35) };
            let n = (g.width() as usize / 2).clamp(32, 600);
            let pts: Vec<Pos2> = (0..=n).map(|i| i as f64 / n as f64).map(|x| self.to_s(x, ch.eval(x))).collect();
            painter.add(egui::Shape::line(pts, Stroke::new(if ci == self.chan { 2.5 } else { 1.5 }, col)));
        }
        let ch = self.data.channels[self.chan].clone();
        if ch.kind == CurveType::Bezier {
            if let Some(i) = self.sel.filter(|&i| i < ch.points.len()) {
                let p = &ch.points[i];
                let (hin, hout) = ch.handles(i);
                let at = self.to_s(p.x, p.y);
                for (h, show) in [(hin, i > 0), (hout, i + 1 < ch.points.len())] {
                    if show {
                        let hp = self.to_s(p.x + h.0, p.y + h.1);
                        painter.line_segment([at, hp], Stroke::new(1.0, Color32::from_white_alpha(160)));
                        painter.rect_filled(Rect::from_center_size(hp, Vec2::splat(7.0)), 1.0, Color32::from_rgb(200, 200, 255));
                    }
                }
            }
        }
        for (i, p) in ch.points.iter().enumerate() {
            let selected = self.sel == Some(i);
            painter.circle(self.to_s(p.x, p.y), if selected { 6.5 } else { 5.0 }, if selected { pal.selection } else { Color32::WHITE }, Stroke::new(1.5, Color32::BLACK));
        }

        // ---- live preview: a playhead on the graph and a ball moved by the value
        if self.preview {
            let t = (motion::now(ui) / self.duration).fract();
            let v = ch.eval(t);
            let at = self.to_s(t, v);
            painter.line_segment([Pos2::new(at.x, g.top()), Pos2::new(at.x, g.bottom())], Stroke::new(1.0, Color32::from_rgb(255, 80, 80)));
            painter.circle_filled(at, 4.0, Color32::from_rgb(255, 80, 80));
            let (strip, sp) = ui.allocate_painter(Vec2::new(ui.available_width(), preview_h - 4.0), Sense::hover());
            let r = strip.rect;
            sp.rect_filled(r, 4.0, pal.canvas);
            let lane = r.shrink2(Vec2::new(46.0, 0.0));
            let k = ((v - self.yr.0) / (self.yr.1 - self.yr.0)) as f32;
            let base = |kk: f32| Pos2::new(lane.left() + kk * lane.width(), r.center().y);
            sp.line_segment([base(((0.0 - self.yr.0) / span) as f32), base(((1.0 - self.yr.0) / span) as f32)], Stroke::new(2.0, Color32::from_white_alpha(40)));
            let size = 14.0 + 10.0 * k.clamp(0.0, 1.5);
            sp.rect_filled(Rect::from_center_size(base(k), Vec2::splat(size)), 3.0, CHANNEL_COLORS[self.chan % CHANNEL_COLORS.len()]);
            sp.text(r.left_center() + Vec2::new(6.0, 0.0), egui::Align2::LEFT_CENTER, format!("x={t:.2}\ny={v:.2}"), font.clone(), pal.canvas_text);
            ui.ctx().request_repaint();
        }

        // ---- editing
        if motion::View::panning(ui) {
            return;
        }
        let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
        let near = |s: &Self, pos: Pos2| -> Option<Grab> {
            let ch = &s.data.channels[s.chan];
            if ch.kind == CurveType::Bezier {
                if let Some(i) = s.sel.filter(|&i| i < ch.points.len()) {
                    let p = &ch.points[i];
                    let (hin, hout) = ch.handles(i);
                    if i + 1 < ch.points.len() && s.to_s(p.x + hout.0, p.y + hout.1).distance(pos) <= 7.0 {
                        return Some(Grab::Out(i));
                    }
                    if i > 0 && s.to_s(p.x + hin.0, p.y + hin.1).distance(pos) <= 7.0 {
                        return Some(Grab::In(i));
                    }
                }
            }
            ch.points.iter().position(|p| s.to_s(p.x, p.y).distance(pos) <= 8.0).map(Grab::Point)
        };
        if resp.drag_started_by(egui::PointerButton::Primary) {
            if let Some(pos) = ui.input(|i| i.pointer.press_origin()).or(pointer) {
                self.grab = near(self, pos);
                if let Some(Grab::Point(i)) = self.grab {
                    self.sel = Some(i);
                }
            }
        }
        if let (Some(grab), Some(pos)) = (self.grab, pointer) {
            if resp.dragged_by(egui::PointerButton::Primary) {
                let (x, y) = self.to_w(pos);
                let ch = &mut self.data.channels[self.chan];
                match grab {
                    Grab::Point(i) => {
                        let lo = if i > 0 { ch.points[i - 1].x } else { 0.0 };
                        let hi = if i + 1 < ch.points.len() { ch.points[i + 1].x } else { 1.0 };
                        let p = &mut ch.points[i];
                        p.x = ((x * 1000.0).round() / 1000.0).clamp(lo, hi);
                        p.y = (y * 1000.0).round() / 1000.0;
                    }
                    Grab::In(i) | Grab::Out(i) => {
                        let (hin, hout) = ch.handles(i);
                        let p = &mut ch.points[i];
                        let off = (x - p.x, y - p.y);
                        if matches!(grab, Grab::In(_)) {
                            p.h_in = Some((off.0.min(0.0), off.1));
                            p.h_out = Some(hout);
                        } else {
                            p.h_out = Some((off.0.max(0.0), off.1));
                            p.h_in = Some(hin);
                        }
                    }
                }
            }
        }
        if resp.drag_stopped() {
            self.grab = None;
        }
        if resp.clicked_by(egui::PointerButton::Primary) {
            if let Some(pos) = pointer {
                match near(self, pos) {
                    Some(Grab::Point(i)) => self.sel = Some(i),
                    Some(_) => {}
                    None => {
                        let (x, y) = self.to_w(pos);
                        if (0.0..=1.0).contains(&x) {
                            let ch = &mut self.data.channels[self.chan];
                            ch.points.push(CurvePoint::new((x * 1000.0).round() / 1000.0, (y * 1000.0).round() / 1000.0));
                            ch.sort();
                            self.sel = ch.points.iter().position(|p| (p.x - (x * 1000.0).round() / 1000.0).abs() < 1e-9);
                        }
                    }
                }
            }
        }
        if resp.clicked_by(egui::PointerButton::Secondary) {
            if let Some(Grab::Point(i)) = pointer.and_then(|pos| near(self, pos)) {
                let ch = &mut self.data.channels[self.chan];
                if ch.points.len() > 1 {
                    ch.points.remove(i);
                    self.sel = None;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{LogLine, Open};
    use crate::project::Project;
    use crate::tex::TexCache;
    use eframe::egui::{Event, Modifiers, PointerButton};
    use egui_kittest::kittest::Queryable;
    use egui_kittest::Harness;

    struct St {
        ed: CurveEditor,
        project: Project,
        tex: TexCache,
        log: Vec<LogLine>,
        open: Vec<Open>,
    }

    #[test]
    fn presets_points_and_save() {
        let dir = crate::room::tests::temp_project("curve_ed");
        let mut project = Project::open(&dir).unwrap();
        motion::create(&mut project, motion::AnimKind::Curve, "jump").unwrap();
        let file = project.motion.path(motion::AnimKind::Curve, "jump").unwrap();
        let mut ed = CurveEditor::open("jump", &file).unwrap();
        ed.preview = false;
        let st = St { ed, project, tex: TexCache::default(), log: vec![], open: vec![] };
        let mut h = Harness::builder().with_size([1000.0, 640.0]).allow_missing_glyphs().with_accessibility_check(false).build_ui_state(
            |ui, s: &mut St| {
                let mut cx = Cx { project: &mut s.project, tex: &mut s.tex, log: &mut s.log, open: &mut s.open, active: true };
                s.ed.ui(ui, &mut cx);
            },
            st,
        );
        h.run_steps(2);
        // the "Lineal" type button and the "Lineal" preset share the label: use the preset API
        h.state_mut().ed.apply_preset("linear");
        h.state_mut().ed.yr = (0.0, 1.0);
        h.run_steps(2);
        assert_eq!(h.state().ed.data.channels[0].points.len(), 2);
        // click in the middle of the graph adds a point at (0.5, ~0.75)
        let p = h.state().ed.screen_of(0.5, 0.75);
        h.event(Event::PointerMoved(p));
        h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(2);
        let ch = &h.state().ed.data.channels[0];
        assert_eq!(ch.points.len(), 3);
        assert!((ch.points[1].x - 0.5).abs() < 0.01 && (ch.points[1].y - 0.75).abs() < 0.02, "{:?}", ch.points[1]);
        // drag it down to 0.25
        let from = h.state().ed.screen_of(ch.points[1].x, ch.points[1].y);
        let to = h.state().ed.screen_of(0.5, 0.25);
        crate::room::tests::drag(&mut h, from, to);
        let y = h.state().ed.data.channels[0].points[1].y;
        assert!((y - 0.25).abs() < 0.02, "{y}");
        h.get_by_label("+ canal").click();
        h.run_steps(2);
        assert_eq!(h.state().ed.data.channels.len(), 2);
        h.state_mut().ed.apply_preset("bounce");
        h.run_steps(2);
        {
            let s = h.state_mut();
            let mut cx = Cx { project: &mut s.project, tex: &mut s.tex, log: &mut s.log, open: &mut s.open, active: true };
            s.ed.save(&mut cx).unwrap();
        }
        let e = modengine::Engine::load(&dir).expect("the saved curve loads");
        let c = &e.curves[0];
        assert_eq!(c.channels.len(), 2);
        assert!((c.channels[0].eval(0.5) - 0.25).abs() < 0.02);
        assert!((c.channels[1].eval(1.0) - 1.0).abs() < 1e-6);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
