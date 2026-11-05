//! Path editor: points on a canvas (optionally over a room), straight or smooth,
//! open or closed, with a live preview of something following the path.

use crate::app::Cx;
use crate::motion::{self, History, View};
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Stroke, Vec2};
use modengine::path::{write_toml, PathDef, PathPoint, PathToml};
use std::path::{Path, PathBuf};

pub struct PathEditor {
    pub name: String,
    file: PathBuf,
    pub data: PathToml,
    saved: PathToml,
    pub hist: History<PathToml>,
    pub sel: Option<usize>,
    drag: Option<usize>,
    view: View,
    snap: bool,
    grid: f32,
    preview: bool,
    /// Preview speed in pixels per step (at the game's step rate).
    preview_speed: f64,
    preview_sprite: Option<String>,
}

impl PathEditor {
    pub fn open(name: &str, file: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
        let data: PathToml = toml::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;
        Ok(PathEditor {
            name: name.into(),
            file: file.to_path_buf(),
            hist: History::new(&data),
            saved: data.clone(),
            data,
            sel: None,
            drag: None,
            view: View::default(),
            snap: true,
            grid: 8.0,
            preview: true,
            preview_speed: 2.0,
            preview_sprite: None,
        })
    }

    pub fn dirty(&self) -> bool {
        self.data != self.saved
    }

    pub fn save(&mut self, _cx: &mut Cx) -> Result<(), String> {
        motion::write_file(&self.file, &write_toml(&self.data))?;
        self.saved = self.data.clone();
        Ok(())
    }

    /// Screen position of a world point (tests).
    #[cfg(test)]
    pub(crate) fn screen_of(&self, x: f32, y: f32) -> Pos2 {
        self.view.to_s(Pos2::new(x, y))
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        egui::Panel::left(("path_side", &self.name)).resizable(true).default_size(250.0).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.side(ui, cx));
        });
        egui::CentralPanel::default().show(ui, |ui| {
            self.toolbar(ui);
            self.canvas(ui, cx);
        });
        self.data.precision = self.data.precision.clamp(1, 8);
        self.hist.track(&self.data, self.drag.is_some());
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.snap, "ajustar a rejilla");
            ui.add(egui::DragValue::new(&mut self.grid).range(1.0..=128.0).prefix("rejilla ").speed(0.2));
            if ui.button("⤢ encajar").clicked() {
                self.view.fitted = false;
            }
            ui.label(RichText::new(format!("{}%", (self.view.zoom * 100.0).round())).weak());
            ui.label(RichText::new("ⓘ").weak()).on_hover_text("clic: añadir punto al final\nCtrl+clic en un tramo: insertar ahí\narrastrar un punto: moverlo\nclic derecho o Supr: borrar\nrueda: zoom · botón central o Espacio+arrastrar: desplazar");
        });
    }

    fn side(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        ui.heading(&self.name);
        let def = PathDef::new(&self.name, self.data.clone());
        ui.label(RichText::new(format!("{} puntos · longitud {:.1} px", self.data.points.len(), def.length())).weak());
        ui.add_space(4.0);
        ui.checkbox(&mut self.data.closed, "Cerrada").on_hover_text("el último punto se une con el primero");
        ui.checkbox(&mut self.data.smooth, "Curva suave").on_hover_text("redondea las esquinas (como «Smooth curve» en GameMaker)");
        ui.add_enabled(self.data.smooth, egui::Slider::new(&mut self.data.precision, 1..=8).text("precisión"));
        ui.horizontal(|ui| {
            ui.label("Room de fondo:");
            let rooms = cx.project.room_names();
            motion::combo(ui, ("path_room", &self.name), &mut self.data.room, &rooms, "—");
        });
        ui.separator();
        ui.strong("Vista previa");
        ui.checkbox(&mut self.preview, "▶ animar un objeto por la trayectoria");
        ui.add(egui::Slider::new(&mut self.preview_speed, 0.25..=16.0).text("px/paso").logarithmic(true));
        ui.horizontal(|ui| {
            ui.label("Sprite:");
            let sprites = cx.project.sprite_names();
            motion::combo(ui, ("path_prev_spr", &self.name), &mut self.preview_sprite, &sprites, "punto");
        });
        ui.separator();
        ui.strong("Transformar");
        ui.horizontal_wrapped(|ui| {
            if ui.button("⇄ Invertir").on_hover_text("recorrer al revés (path_reverse)").clicked() {
                self.data.points.reverse();
            }
            let center = || {
                let b = def.bounds().unwrap_or_default();
                ((b.0 + b.2) / 2.0, (b.1 + b.3) / 2.0)
            };
            if ui.button("↔ Espejo").on_hover_text("refleja en horizontal (path_mirror)").clicked() {
                let (cx_, _) = center();
                self.data.points.iter_mut().for_each(|p| p.x = 2.0 * cx_ - p.x);
            }
            if ui.button("↕ Voltear").on_hover_text("refleja en vertical (path_flip)").clicked() {
                let (_, cy) = center();
                self.data.points.iter_mut().for_each(|p| p.y = 2.0 * cy - p.y);
            }
        });
        ui.separator();
        ui.strong("Puntos");
        let mut remove = None;
        egui::Grid::new(("path_points", &self.name)).num_columns(5).spacing([4.0, 2.0]).striped(true).show(ui, |ui| {
            ui.label(RichText::new("#").weak());
            ui.label(RichText::new("x").weak());
            ui.label(RichText::new("y").weak());
            ui.label(RichText::new("vel. %").weak()).on_hover_text("velocidad relativa en este punto (100 = la de path_start)");
            ui.label("");
            ui.end_row();
            for (i, p) in self.data.points.iter_mut().enumerate() {
                if ui.selectable_label(self.sel == Some(i), i.to_string()).clicked() {
                    self.sel = Some(i);
                }
                ui.add(egui::DragValue::new(&mut p.x).speed(1.0));
                ui.add(egui::DragValue::new(&mut p.y).speed(1.0));
                ui.add(egui::DragValue::new(&mut p.speed).range(0.0..=1000.0).speed(1.0));
                if ui.small_button("×").on_hover_text("borrar punto").clicked() {
                    remove = Some(i);
                }
                ui.end_row();
            }
        });
        if let Some(i) = remove {
            self.remove(i);
        }
        if ui.button("+ punto").clicked() {
            let last = self.data.points.last().copied().unwrap_or(PathPoint { x: 0.0, y: 0.0, speed: 100.0 });
            self.data.points.push(PathPoint { x: last.x + 32.0, y: last.y, speed: 100.0 });
            self.sel = Some(self.data.points.len() - 1);
        }
        ui.add_space(6.0);
        ui.label(RichText::new(format!("En ModScript: path_start({}, 2, path_action.restart, true)", self.name)).weak().small().monospace());
    }

    fn remove(&mut self, i: usize) {
        if i < self.data.points.len() {
            self.data.points.remove(i);
        }
        self.sel = None;
        self.drag = None;
    }

    fn world_rect(&self, cx: &Cx) -> Rect {
        let d = &cx.project.game.display;
        let room = self.data.room.as_deref().and_then(|r| motion::room_toml(cx.project, r)).map(|rt| Vec2::new(rt.width.unwrap_or(d.width) as f32, rt.height.unwrap_or(d.height) as f32));
        let mut r = Rect::from_min_size(Pos2::ZERO, room.unwrap_or(Vec2::new(d.width as f32, d.height as f32)));
        for p in &self.data.points {
            r.extend_with(Pos2::new(p.x as f32, p.y as f32));
        }
        r.expand(8.0)
    }

    fn canvas(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        let (resp, painter) = motion::canvas(ui);
        let world = self.world_rect(cx);
        self.view.update(ui, &resp, world);
        let pal = crate::theme::colors(ui.ctx());
        painter.rect_filled(resp.rect, 0.0, pal.canvas);
        let ctx = ui.ctx().clone();
        let screen = Vec2::new(cx.project.game.display.width as f32, cx.project.game.display.height as f32);
        match self.data.room.clone() {
            Some(r) => {
                motion::draw_room(&painter, cx, &ctx, &self.view, &r, 0.55);
            }
            None => {
                painter.rect_filled(Rect::from_min_max(self.view.to_s(Pos2::ZERO), self.view.to_s(Pos2::ZERO + screen)), 0.0, Color32::from_black_alpha(90));
            }
        }
        motion::grid(&painter, &self.view, self.grid, Color32::from_white_alpha(14));
        let v = self.view;
        let s = |x: f64, y: f64| v.to_s(Pos2::new(x as f32, y as f32));

        // ---- the path itself
        let def = PathDef::new(&self.name, self.data.clone());
        let accent = Color32::from_rgb(255, 196, 64);
        if self.data.smooth && self.data.points.len() > 2 {
            let mut ctrl: Vec<Pos2> = self.data.points.iter().map(|p| s(p.x, p.y)).collect();
            if self.data.closed {
                ctrl.push(ctrl[0]);
            }
            painter.add(egui::Shape::dashed_line(&ctrl, Stroke::new(1.0, Color32::from_white_alpha(70)), 4.0, 4.0));
        }
        let line: Vec<Pos2> = def.poly.iter().map(|p| s(p.0, p.1)).collect();
        if line.len() > 1 {
            painter.add(egui::Shape::line(line.clone(), Stroke::new(2.5, accent)));
            // direction arrows along the path
            let len = def.length();
            if len > 0.0 {
                let n = ((len * v.zoom as f64) / 90.0).clamp(1.0, 24.0) as usize;
                for k in 0..n {
                    let t = (k as f64 + 0.5) / n as f64;
                    let a = def.at(t);
                    let b = def.at((t + 0.5 / len.max(1.0)).min(1.0));
                    let (pa, pb) = (s(a.0, a.1), s(b.0, b.1));
                    let dir = (pb - pa).normalized();
                    if dir.is_finite() && dir != Vec2::ZERO {
                        let side = Vec2::new(-dir.y, dir.x) * 4.0;
                        painter.add(egui::Shape::convex_polygon(vec![pa + dir * 6.0, pa - dir * 3.0 + side, pa - dir * 3.0 - side], accent, Stroke::NONE));
                    }
                }
            }
        }
        let font = egui::FontId::proportional(11.0);
        for (i, p) in self.data.points.iter().enumerate() {
            let at = s(p.x, p.y);
            let selected = self.sel == Some(i);
            let fill = if i == 0 { Color32::from_rgb(90, 220, 120) } else if selected { pal.selection } else { Color32::WHITE };
            painter.circle(at, if selected { 6.5 } else { 5.0 }, fill, Stroke::new(1.5, Color32::BLACK));
            painter.text(at + Vec2::new(7.0, -7.0), egui::Align2::LEFT_BOTTOM, i.to_string(), font.clone(), Color32::from_white_alpha(200));
        }

        // ---- preview: something walking the path at the chosen speed
        if self.preview && def.length() > 0.0 {
            let rate = cx.project.game.display.step_rate.max(1.0);
            let secs = def.length() / (self.preview_speed * rate);
            let t = (motion::now(ui) / secs.max(0.01)).fract();
            let (x, y, _) = def.at(t);
            let at = s(x, y);
            match self.preview_sprite.as_deref().and_then(|n| cx.tex.get(&ctx, cx.project, n)) {
                Some(tex) => tex.draw(&painter, 0, at, v.zoom, Color32::WHITE),
                None => {
                    painter.circle_filled(at, 7.0, Color32::from_rgb(255, 90, 90));
                }
            }
            ui.ctx().request_repaint();
        }

        // ---- editing
        if View::panning(ui) {
            return;
        }
        let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
        let hit = |pos: Pos2, pts: &[PathPoint]| pts.iter().enumerate().filter(|(_, p)| s(p.x, p.y).distance(pos) <= 8.0).map(|(i, _)| i).last();
        let snap = |w: Pos2, on: bool, g: f32| if on { Pos2::new((w.x / g).round() * g, (w.y / g).round() * g) } else { Pos2::new(w.x.round(), w.y.round()) };
        if resp.drag_started_by(egui::PointerButton::Primary) {
            // where the button went down, not where the drag was recognised
            if let Some(pos) = ui.input(|i| i.pointer.press_origin()).or(pointer) {
                self.drag = hit(pos, &self.data.points);
                if self.drag.is_some() {
                    self.sel = self.drag;
                }
            }
        }
        if let (Some(i), Some(pos)) = (self.drag, pointer) {
            if resp.dragged_by(egui::PointerButton::Primary) {
                let w = snap(v.to_w(pos), self.snap, self.grid);
                if let Some(p) = self.data.points.get_mut(i) {
                    p.x = w.x as f64;
                    p.y = w.y as f64;
                }
            }
        }
        if resp.drag_stopped() {
            self.drag = None;
        }
        if resp.clicked_by(egui::PointerButton::Primary) {
            if let Some(pos) = pointer {
                match hit(pos, &self.data.points) {
                    Some(i) => self.sel = Some(i),
                    None => {
                        let w = snap(v.to_w(pos), self.snap, self.grid);
                        let pt = PathPoint { x: w.x as f64, y: w.y as f64, speed: 100.0 };
                        let ctrl = ui.input(|i| i.modifiers.command);
                        let at = if ctrl { nearest_segment(&self.data, pos, &s).map(|i| i + 1) } else { None };
                        match at {
                            Some(i) if i <= self.data.points.len() => {
                                self.data.points.insert(i, pt);
                                self.sel = Some(i);
                            }
                            _ => {
                                self.data.points.push(pt);
                                self.sel = Some(self.data.points.len() - 1);
                            }
                        }
                    }
                }
            }
        }
        if resp.clicked_by(egui::PointerButton::Secondary) {
            if let Some(i) = pointer.and_then(|pos| hit(pos, &self.data.points)) {
                self.remove(i);
            }
        }
        if cx.active && ui.input(|i| i.key_pressed(egui::Key::Delete)) && ui.memory(|m| m.focused().is_none()) {
            if let Some(i) = self.sel {
                self.remove(i);
            }
        }
        if let Some(pos) = resp.hover_pos() {
            let w = v.to_w(pos);
            painter.text(resp.rect.left_bottom() + Vec2::new(6.0, -4.0), egui::Align2::LEFT_BOTTOM, format!("{:.0}, {:.0}", w.x, w.y), font, pal.canvas_text);
        }
    }
}

/// Index of the first point of the segment closest to `pos` (screen space), within 10 px.
fn nearest_segment(d: &PathToml, pos: Pos2, s: &dyn Fn(f64, f64) -> Pos2) -> Option<usize> {
    let n = d.points.len();
    let segs = if d.closed { n } else { n.saturating_sub(1) };
    (0..segs)
        .map(|i| {
            let (a, b) = (d.points[i], d.points[(i + 1) % n]);
            let (a, b) = (s(a.x, a.y), s(b.x, b.y));
            let ab = b - a;
            let t = ((pos - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
            (i, (a + ab * t).distance(pos))
        })
        .filter(|(_, d)| *d <= 10.0)
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, _)| i)
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
        ed: PathEditor,
        project: Project,
        tex: TexCache,
        log: Vec<LogLine>,
        open: Vec<Open>,
    }

    #[test]
    fn add_drag_delete_points_and_save() {
        let dir = crate::room::tests::temp_project("path_ed");
        let mut project = Project::open(&dir).unwrap();
        motion::create(&mut project, motion::AnimKind::Path, "patrol").unwrap();
        let file = project.motion.path(motion::AnimKind::Path, "patrol").unwrap();
        let mut ed = PathEditor::open("patrol", &file).unwrap();
        ed.preview = false;
        ed.data.points.clear();
        ed.data.smooth = false;
        let st = St { ed, project, tex: TexCache::default(), log: vec![], open: vec![] };
        let mut h = Harness::builder().with_size([1100.0, 700.0]).allow_missing_glyphs().with_accessibility_check(false).build_ui_state(
            |ui, s: &mut St| {
                let mut cx = Cx { project: &mut s.project, tex: &mut s.tex, log: &mut s.log, open: &mut s.open, active: true };
                s.ed.ui(ui, &mut cx);
            },
            st,
        );
        h.run_steps(3);
        let click = |h: &mut Harness<St>, p: Pos2, b: PointerButton| {
            h.event(Event::PointerMoved(p));
            h.event(Event::PointerButton { pos: p, button: b, pressed: true, modifiers: Modifiers::NONE });
            h.event(Event::PointerButton { pos: p, button: b, pressed: false, modifiers: Modifiers::NONE });
            h.run_steps(2);
        };
        // three clicks = three points (snapped to the 8 px grid)
        for (x, y) in [(40.0, 40.0), (120.0, 40.0), (120.0, 120.0)] {
            let p = h.state().ed.screen_of(x, y);
            click(&mut h, p, PointerButton::Primary);
        }
        assert_eq!(h.state().ed.data.points.len(), 3);
        assert_eq!((h.state().ed.data.points[1].x, h.state().ed.data.points[1].y), (120.0, 40.0));
        // drag the middle point
        let from = h.state().ed.screen_of(120.0, 40.0);
        let to = h.state().ed.screen_of(160.0, 64.0);
        crate::room::tests::drag(&mut h, from, to);
        assert_eq!((h.state().ed.data.points[1].x, h.state().ed.data.points[1].y), (160.0, 64.0));
        // right click deletes it; undo brings it back
        let p = h.state().ed.screen_of(160.0, 64.0);
        click(&mut h, p, PointerButton::Secondary);
        assert_eq!(h.state().ed.data.points.len(), 2);
        {
            let ed = &mut h.state_mut().ed;
            ed.hist.undo(&mut ed.data);
            assert_eq!(ed.data.points.len(), 3);
        }
        assert!(h.state().ed.dirty());
        {
            let s = h.state_mut();
            let mut cx = Cx { project: &mut s.project, tex: &mut s.tex, log: &mut s.log, open: &mut s.open, active: true };
            s.ed.save(&mut cx).unwrap();
        }
        assert!(!h.state().ed.dirty());
        let e = modengine::Engine::load(&dir).expect("the saved path loads");
        let p = &e.paths[0];
        assert_eq!(p.data.points.len(), 3);
        // (40,40) → (160,64) → (120,120)
        let want = 120.0f64.hypot(24.0) + 40.0f64.hypot(56.0);
        assert!((p.length() - want).abs() < 1e-9, "{} vs {want}", p.length());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
