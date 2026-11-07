//! Timeline editor: the moments (step → ModScript code) on a ruler, the list of moments
//! and a highlighted code editor, with a live playhead preview of when each one fires.

use crate::app::Cx;
use crate::highlight::code_editor;
use crate::motion::{self, History};
use eframe::egui::{self, Color32, Pos2, RichText, Sense, Stroke, Vec2};
use modengine::timeline::{write_toml, MomentToml, TimelineToml};
use std::path::{Path, PathBuf};

pub struct TimelineEditor {
    pub name: String,
    file: PathBuf,
    pub data: TimelineToml,
    saved: TimelineToml,
    pub hist: History<TimelineToml>,
    pub sel: usize,
    new_step: u32,
    playing: bool,
    looping: bool,
    speed: f64,
    /// Preview playhead position (steps) and the time it was last advanced.
    head: f64,
    last_time: Option<f64>,
}

/// Syntax check of one moment's code.
pub fn moment_error(code: &str) -> Option<(u32, String)> {
    modscript::parse_stmts("<moment>", code).err().map(|e| (e.line, e.msg))
}

impl TimelineEditor {
    pub fn open(name: &str, file: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
        let mut data: TimelineToml = toml::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;
        data.sorted();
        Ok(TimelineEditor {
            name: name.into(),
            file: file.to_path_buf(),
            hist: History::new(&data),
            saved: data.clone(),
            data,
            sel: 0,
            new_step: 0,
            playing: true,
            looping: true,
            speed: 1.0,
            head: 0.0,
            last_time: None,
        })
    }

    pub fn dirty(&self) -> bool {
        self.data != self.saved
    }

    pub fn save(&mut self, _cx: &mut Cx) -> Result<(), String> {
        self.data.sorted();
        motion::write_file(&self.file, &write_toml(&self.data))?;
        self.saved = self.data.clone();
        Ok(())
    }

    fn max_step(&self) -> u32 {
        self.data.moments.iter().map(|m| m.step).max().unwrap_or(0)
    }

    /// Adds a moment at `step` (or selects the existing one).
    pub fn add_moment(&mut self, step: u32) {
        if let Some(i) = self.data.moments.iter().position(|m| m.step == step) {
            self.sel = i;
            return;
        }
        self.data.moments.push(MomentToml { step, code: String::new() });
        self.data.sorted();
        self.sel = self.data.moments.iter().position(|m| m.step == step).unwrap_or(0);
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        let rate = cx.project.game.display.step_rate.max(1.0);
        egui::Panel::top(("tl_ruler", &self.name)).resizable(false).show(ui, |ui| self.ruler(ui, rate));
        egui::Panel::left(("tl_list", &self.name)).resizable(true).default_size(250.0).show(ui, |ui| self.list(ui));
        egui::CentralPanel::default().show(ui, |ui| self.code(ui));
        self.hist.track(&self.data, false);
    }

    fn ruler(&mut self, ui: &mut egui::Ui, rate: f64) {
        ui.horizontal(|ui| {
            ui.heading(&self.name);
            ui.separator();
            let label = if self.playing { concat!(fa!(pause), " Pausa") } else { concat!(fa!(play), " Reproducir") };
            if ui.button(label).on_hover_text("vista previa: el cursor avanza como timeline_position en el juego").clicked() {
                self.playing = !self.playing;
            }
            if ui.button(fa!(backward_step)).on_hover_text("volver al paso 0").clicked() {
                self.head = 0.0;
            }
            ui.checkbox(&mut self.looping, "bucle (timeline_loop)");
            ui.add(egui::DragValue::new(&mut self.speed).range(-8.0..=8.0).speed(0.05).prefix("timeline_speed "));
            ui.label(RichText::new(format!("paso {:.0} · {:.2} s", self.head.floor(), self.head / rate)).monospace());
        });
        let max = self.max_step();
        let span = (max as f64 + 10.0).max(60.0);
        let (resp, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), 54.0), Sense::click_and_drag());
        let r = resp.rect;
        let pal = crate::theme::colors(ui.ctx());
        painter.rect_filled(r, 4.0, pal.canvas);
        let x_of = |s: f64| r.left() + 12.0 + (s / span) as f32 * (r.width() - 24.0);
        let s_of = |x: f32| (((x - r.left() - 12.0) / (r.width() - 24.0)) as f64 * span).round().max(0.0);
        // ticks every 10 steps (labels every 30 or so)
        let every = [1.0, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0].into_iter().find(|e| (r.width() as f64 - 24.0) / (span / e) >= 40.0).unwrap_or(600.0);
        let mut s = 0.0;
        while s <= span {
            let x = x_of(s);
            painter.line_segment([Pos2::new(x, r.bottom() - 10.0), Pos2::new(x, r.bottom())], Stroke::new(1.0, pal.weak));
            painter.text(Pos2::new(x, r.bottom() - 12.0), egui::Align2::CENTER_BOTTOM, format!("{s:.0}"), egui::FontId::proportional(10.0), pal.weak);
            s += every;
        }
        // preview playhead
        let now = motion::now(ui);
        if self.playing {
            if let Some(t) = self.last_time {
                self.head += (now - t) * rate * self.speed;
                if self.looping && !self.data.moments.is_empty() {
                    self.head = self.head.rem_euclid(max as f64 + 1.0);
                } else if self.head > span {
                    self.head = span;
                } else if self.head < 0.0 {
                    self.head = 0.0;
                }
            }
            ui.ctx().request_repaint();
        }
        self.last_time = Some(now);
        for (i, m) in self.data.moments.iter().enumerate() {
            let x = x_of(m.step as f64);
            let fresh = self.head >= m.step as f64 && self.head - (m.step as f64) < rate * 0.25;
            let err = moment_error(&m.code).is_some();
            let col = if err {
                pal.error
            } else if fresh {
                Color32::from_rgb(255, 230, 90)
            } else if i == self.sel {
                pal.selection
            } else {
                Color32::from_rgb(120, 180, 255)
            };
            let c = Pos2::new(x, r.top() + 18.0);
            let d = if fresh { 8.0 } else { 6.0 };
            painter.add(egui::Shape::convex_polygon(vec![c + Vec2::new(0.0, -d), c + Vec2::new(d, 0.0), c + Vec2::new(0.0, d), c + Vec2::new(-d, 0.0)], col, Stroke::new(1.0, Color32::BLACK)));
        }
        let hx = x_of(self.head);
        painter.line_segment([Pos2::new(hx, r.top()), Pos2::new(hx, r.bottom())], Stroke::new(2.0, Color32::from_rgb(255, 80, 80)));
        if let Some(p) = resp.interact_pointer_pos() {
            if resp.clicked() || resp.dragged() {
                let step = s_of(p.x);
                match self.data.moments.iter().position(|m| (x_of(m.step as f64) - p.x).abs() < 8.0) {
                    Some(i) if resp.clicked() => self.sel = i,
                    _ => {
                        self.head = step;
                        self.new_step = step as u32;
                    }
                }
            }
            if resp.double_clicked() {
                self.add_moment(s_of(p.x) as u32);
            }
        }
        resp.on_hover_text("clic: elegir momento o mover el cursor · doble clic: nuevo momento en ese paso");
    }

    fn list(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut self.new_step).range(0..=1_000_000).prefix("paso "));
            if ui.button("+ Momento").clicked() {
                self.add_moment(self.new_step);
            }
        });
        ui.separator();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if self.data.moments.is_empty() {
                ui.label(RichText::new("Sin momentos. Añade uno con «+ Momento» o haz doble clic en la regla.").weak());
            }
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            let mut pick = None;
            for (i, m) in self.data.moments.iter().enumerate() {
                // first line of real code (comments only when there is nothing else)
                let lines = || m.code.lines().map(str::trim).filter(|l| !l.is_empty());
                let first = lines().find(|l| !l.starts_with("//")).or_else(|| lines().next()).unwrap_or("(vacío)");
                let mut short: String = first.chars().take(20).collect();
                if first.chars().count() > 20 {
                    short.push('…');
                }
                let warn = if moment_error(&m.code).is_some() { concat!(fa!(triangle_exclamation), " ") } else { "" };
                let text = RichText::new(format!("{warn}{} {:>4}   {short}", fa!(stopwatch), m.step)).monospace();
                if ui.selectable_label(i == self.sel, text).clicked() {
                    pick = Some(i);
                }
            }
            if let Some(i) = pick {
                self.sel = i;
            }
        });
    }

    fn code(&mut self, ui: &mut egui::Ui) {
        let n = self.data.moments.len();
        if n == 0 {
            ui.label("Crea un momento para escribir su código.");
            return;
        }
        self.sel = self.sel.min(n - 1);
        let mut action = None;
        ui.horizontal(|ui| {
            let mut step = self.data.moments[self.sel].step;
            ui.label("Momento en el paso");
            if ui.add(egui::DragValue::new(&mut step).range(0..=1_000_000)).changed() && !self.data.moments.iter().any(|m| m.step == step) {
                self.data.moments[self.sel].step = step;
                let code = self.data.moments[self.sel].code.clone();
                self.data.sorted();
                self.sel = self.data.moments.iter().position(|m| m.step == step && m.code == code).unwrap_or(0);
            }
            if ui.button(concat!(fa!(clone), " Duplicar")).on_hover_text("copia el momento al primer paso libre siguiente").clicked() {
                action = Some(0);
            }
            if ui.button(concat!(fa!(trash_can), " Borrar")).clicked() {
                action = Some(1);
            }
        });
        match action {
            Some(0) => {
                let m = self.data.moments[self.sel].clone();
                let mut s = m.step + 1;
                while self.data.moments.iter().any(|x| x.step == s) {
                    s += 1;
                }
                self.data.moments.push(MomentToml { step: s, code: m.code });
                self.data.sorted();
                self.sel = self.data.moments.iter().position(|x| x.step == s).unwrap_or(0);
                return;
            }
            Some(_) => {
                self.data.moments.remove(self.sel);
                self.sel = self.sel.saturating_sub(1);
                return;
            }
            None => {}
        }
        let i = self.sel;
        match moment_error(&self.data.moments[i].code) {
            Some((line, msg)) => ui.colored_label(crate::theme::colors(ui.ctx()).error, format!("línea {line}: {msg}")),
            None => ui.colored_label(crate::theme::colors(ui.ctx()).ok, concat!(fa!(check), " sin errores de sintaxis · se ejecuta en la instancia (self) que lleva la timeline")),
        };
        let step = self.data.moments[i].step;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            code_editor(ui, &mut self.data.moments[i].code, ("timeline", &self.name, step), 18);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{LogLine, Open};
    use crate::project::Project;
    use crate::tex::TexCache;
    use egui_kittest::kittest::Queryable;
    use egui_kittest::Harness;

    struct St {
        ed: TimelineEditor,
        project: Project,
        tex: TexCache,
        log: Vec<LogLine>,
        open: Vec<Open>,
    }

    #[test]
    fn add_moments_edit_code_and_save() {
        let dir = crate::room::tests::temp_project("tl_ed");
        let mut project = Project::open(&dir).unwrap();
        motion::create(&mut project, motion::AnimKind::Timeline, "intro").unwrap();
        let file = project.motion.path(motion::AnimKind::Timeline, "intro").unwrap();
        let mut ed = TimelineEditor::open("intro", &file).unwrap();
        ed.playing = false;
        let st = St { ed, project, tex: TexCache::default(), log: vec![], open: vec![] };
        let mut h = Harness::builder().with_size([1000.0, 600.0]).allow_missing_glyphs().with_accessibility_check(false).build_ui_state(
            |ui, s: &mut St| {
                let mut cx = Cx { project: &mut s.project, tex: &mut s.tex, log: &mut s.log, open: &mut s.open, active: true };
                s.ed.ui(ui, &mut cx);
            },
            st,
        );
        h.run_steps(2);
        h.state_mut().ed.new_step = 30;
        h.get_by_label("+ Momento").click();
        h.run_steps(2);
        assert_eq!(h.state().ed.data.moments.iter().map(|m| m.step).collect::<Vec<_>>(), vec![0, 30]);
        assert_eq!(h.state().ed.sel, 1);
        h.state_mut().ed.data.moments[1].code = "hspeed = -2\ntimeline_running = false".into();
        h.run_steps(2);
        assert!(moment_error(&h.state().ed.data.moments[1].code).is_none());
        assert!(moment_error("x = (").is_some());
        h.get_by_label(concat!(fa!(clone), " Duplicar")).click();
        h.run_steps(2);
        assert_eq!(h.state().ed.data.moments.iter().map(|m| m.step).collect::<Vec<_>>(), vec![0, 30, 31]);
        {
            let s = h.state_mut();
            let mut cx = Cx { project: &mut s.project, tex: &mut s.tex, log: &mut s.log, open: &mut s.open, active: true };
            s.ed.save(&mut cx).unwrap();
        }
        let e = modengine::Engine::load(&dir).expect("the saved timeline loads");
        assert_eq!(e.timelines[0].moments.len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
