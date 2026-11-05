//! Notes (`notes/<name>.md`): Markdown text for the team (to-dos, design, credits), like
//! GameMaker's Note asset. Edited as plain text with a live preview; the game ignores them.

use crate::app::{Cx, Open};
use crate::project::Project;
use eframe::egui::{self, text::LayoutJob, Color32, FontId, RichText, TextFormat};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Debug)]
enum View {
    Edit,
    Split,
    Preview,
}

pub struct NoteEditor {
    pub name: String,
    path: PathBuf,
    text: String,
    saved: String,
    pub dirty: bool,
    view: View,
}

/// Creates `notes/<name>.md`.
pub fn create(p: &mut Project, name: &str) -> Result<Open, String> {
    let path = p.dir.join("notes").join(format!("{name}.md"));
    if path.exists() {
        return Err(format!("{} ya existe", path.display()));
    }
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&path, format!("# {name}\n\n- [ ] \n")).map_err(|e| e.to_string())?;
    p.reload_lists();
    Ok(Open::Note(name.to_string()))
}

impl NoteEditor {
    pub fn open(name: &str, path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(NoteEditor { name: name.into(), path: path.into(), saved: text.clone(), text, dirty: false, view: View::Split })
    }

    pub fn save(&mut self, _cx: &mut Cx) -> Result<(), String> {
        std::fs::write(&self.path, &self.text).map_err(|e| format!("{}: {e}", self.path.display()))?;
        self.saved = self.text.clone();
        self.dirty = false;
        Ok(())
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, _cx: &mut Cx) {
        ui.horizontal(|ui| {
            for (v, label) in [(View::Edit, "✎ Texto"), (View::Split, "◫ Dividido"), (View::Preview, "👁 Vista previa")] {
                ui.selectable_value(&mut self.view, v, label);
            }
            ui.separator();
            let words = self.text.split_whitespace().count();
            let (done, todo) = tasks(&self.text);
            let mut info = format!("{words} palabras");
            if done + todo > 0 {
                info += &format!(" · tareas {done}/{}", done + todo);
            }
            ui.label(RichText::new(info).weak());
        });
        ui.separator();
        let h = ui.available_height();
        match self.view {
            View::Edit => self.editor(ui, h),
            View::Preview => self.preview(ui, h),
            View::Split => {
                ui.columns(2, |cols| {
                    self.editor(&mut cols[0], h);
                    self.preview(&mut cols[1], h);
                });
            }
        }
        self.dirty = self.text != self.saved;
    }

    fn editor(&mut self, ui: &mut egui::Ui, h: f32) {
        egui::ScrollArea::vertical().id_salt("note_src").max_height(h).auto_shrink([false, false]).show(ui, |ui| {
            ui.add(egui::TextEdit::multiline(&mut self.text).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).desired_rows(20).hint_text("# Título\n\nTexto en Markdown: **negrita**, *cursiva*, `código`, - listas, - [ ] tareas"));
        });
    }

    fn preview(&mut self, ui: &mut egui::Ui, h: f32) {
        let mut toggle = None;
        egui::ScrollArea::vertical().id_salt("note_preview").max_height(h).auto_shrink([false, false]).show(ui, |ui| {
            toggle = render(ui, &self.text);
        });
        // clicking a task's box in the preview ticks it in the text
        if let Some(line) = toggle {
            self.text = toggle_task(&self.text, line);
        }
    }
}

/// (done, pending) `- [x]` / `- [ ]` tasks.
fn tasks(text: &str) -> (usize, usize) {
    let mut out = (0, 0);
    for l in text.lines() {
        match task(l) {
            Some((true, _)) => out.0 += 1,
            Some((false, _)) => out.1 += 1,
            None => {}
        }
    }
    out
}

/// `- [ ] text` → (done, text).
fn task(line: &str) -> Option<(bool, &str)> {
    let t = line.trim_start();
    let rest = t.strip_prefix("- ").or_else(|| t.strip_prefix("* "))?;
    if let Some(r) = rest.strip_prefix("[ ] ").or(rest.strip_prefix("[ ]")) {
        return Some((false, r));
    }
    rest.strip_prefix("[x] ").or(rest.strip_prefix("[X] ")).or(rest.strip_prefix("[x]")).map(|r| (true, r))
}

/// The text with the task on line `n` ticked or unticked.
fn toggle_task(text: &str, n: usize) -> String {
    let mut out: Vec<String> = text.split('\n').map(String::from).collect();
    if let Some(l) = out.get_mut(n) {
        if l.contains("[ ]") {
            *l = l.replacen("[ ]", "[x]", 1);
        } else if l.contains("[x]") {
            *l = l.replacen("[x]", "[ ]", 1);
        } else if l.contains("[X]") {
            *l = l.replacen("[X]", "[ ]", 1);
        }
    }
    out.join("\n")
}

/// Inline Markdown (`**bold**`, `*italic*`, `` `code` ``) into a layout job.
fn inline(job: &mut LayoutJob, s: &str, size: f32, color: Color32, strong: bool) {
    // egui's default fonts have no bold weight: bold text uses the strongest text colour
    let strong_color = if color.r() as u32 + color.g() as u32 + color.b() as u32 > 380 { Color32::WHITE } else { Color32::BLACK };
    let mut bold = strong;
    let mut italic = false;
    let mut code = false;
    let mut buf = String::new();
    let flush = |job: &mut LayoutJob, buf: &mut String, bold: bool, italic: bool, code: bool| {
        if buf.is_empty() {
            return;
        }
        let font = if code { FontId::monospace(size * 0.92) } else { FontId::proportional(size) };
        let mut f = TextFormat { font_id: font, color: if bold { strong_color } else { color }, italics: italic, ..Default::default() };
        if code {
            f.background = Color32::from_gray(128).gamma_multiply(0.25);
        }
        job.append(&std::mem::take(buf), 0.0, f);
    };
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '`' => {
                flush(job, &mut buf, bold, italic, code);
                code = !code;
            }
            '*' | '_' if !code => {
                if chars.peek() == Some(&c) {
                    chars.next();
                    flush(job, &mut buf, bold, italic, code);
                    bold = !bold;
                } else {
                    flush(job, &mut buf, bold, italic, code);
                    italic = !italic;
                }
            }
            _ => buf.push(c),
        }
    }
    flush(job, &mut buf, bold, italic, code);
}

/// Draws Markdown (headings, lists, tasks, quotes, code blocks, rules, links). Returns the
/// line of a task whose box was clicked.
fn render(ui: &mut egui::Ui, text: &str) -> Option<usize> {
    let pal = crate::theme::colors(ui.ctx());
    let base = ui.visuals().text_color();
    let strong = ui.visuals().strong_text_color();
    let mut clicked = None;
    let mut in_code = false;
    let mut code = String::new();
    for (n, line) in text.split('\n').enumerate() {
        let t = line.trim_end();
        if t.trim_start().starts_with("```") {
            if in_code {
                egui::Frame::new().fill(ui.visuals().extreme_bg_color).inner_margin(6).corner_radius(4).show(ui, |ui| {
                    ui.label(RichText::new(code.trim_end()).monospace());
                });
                code.clear();
            }
            in_code = !in_code;
            continue;
        }
        if in_code {
            code += t;
            code.push('\n');
            continue;
        }
        let trimmed = t.trim_start();
        if trimmed.is_empty() {
            ui.add_space(6.0);
            continue;
        }
        if trimmed == "---" || trimmed == "***" {
            ui.separator();
            continue;
        }
        let level = trimmed.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&level) && trimmed[level..].starts_with(' ') {
            let size = [24.0, 20.0, 17.0, 15.0, 14.0, 14.0][level - 1];
            let mut job = LayoutJob::default();
            inline(&mut job, trimmed[level..].trim(), size, strong, true);
            ui.add_space(if level == 1 { 4.0 } else { 2.0 });
            ui.label(job);
            continue;
        }
        if let Some((done, rest)) = task(t) {
            ui.horizontal_wrapped(|ui| {
                let mut d = done;
                if ui.checkbox(&mut d, "").changed() {
                    clicked = Some(n);
                }
                let mut job = LayoutJob::default();
                inline(&mut job, rest, 14.0, if done { pal.weak } else { base }, false);
                if done {
                    for s in &mut job.sections {
                        s.format.strikethrough = egui::Stroke::new(1.0, pal.weak);
                    }
                }
                ui.label(job);
            });
            continue;
        }
        let indent = (t.len() - trimmed.len()) as f32 * 6.0;
        let (bullet, rest) = if let Some(r) = trimmed.strip_prefix("- ").or(trimmed.strip_prefix("* ")).or(trimmed.strip_prefix("+ ")) {
            ("•".to_string(), r)
        } else if let Some((num, r)) = trimmed.split_once(". ").filter(|(a, _)| !a.is_empty() && a.chars().all(|c| c.is_ascii_digit())) {
            (format!("{num}."), r)
        } else if let Some(r) = trimmed.strip_prefix("> ") {
            ("▌".to_string(), r)
        } else {
            (String::new(), trimmed)
        };
        ui.horizontal_wrapped(|ui| {
            ui.add_space(indent);
            if !bullet.is_empty() {
                ui.label(RichText::new(&bullet).color(if bullet == "▌" { pal.weak } else { base }));
            }
            // [text](url) links
            let mut rest = rest;
            while let Some(a) = rest.find('[') {
                let Some(b) = rest[a..].find("](").map(|b| a + b) else { break };
                let Some(c) = rest[b..].find(')').map(|c| b + c) else { break };
                let mut job = LayoutJob::default();
                inline(&mut job, &rest[..a], 14.0, base, false);
                ui.label(job);
                ui.hyperlink_to(&rest[a + 1..b], &rest[b + 2..c]);
                rest = &rest[c + 1..];
            }
            let mut job = LayoutJob::default();
            inline(&mut job, rest, 14.0, if bullet == "▌" { pal.weak } else { base }, false);
            ui.label(job);
        });
    }
    if in_code && !code.is_empty() {
        ui.label(RichText::new(code.trim_end()).monospace());
    }
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tasks_are_counted_and_toggled() {
        let t = "# Por hacer\n- [ ] enemigos\n- [x] menú\n* [ ] sonido\ntexto";
        assert_eq!(tasks(t), (1, 2));
        let t2 = toggle_task(t, 1);
        assert_eq!(tasks(&t2), (2, 1));
        assert_eq!(toggle_task(&t2, 1), t);
        assert_eq!(toggle_task(t, 4), t, "not a task: unchanged");
    }

    #[test]
    fn create_save_and_render() {
        let dir = crate::room::tests::temp_project("note");
        let mut p = Project::open(&dir).unwrap();
        assert_eq!(create(&mut p, "ideas").unwrap(), Open::Note("ideas".into()));
        assert!(create(&mut p, "ideas").is_err());
        assert!(p.taken_names().contains("ideas"));
        let path = p.notes.iter().find(|n| n.name == "ideas").unwrap().path.clone();
        let mut ed = NoteEditor::open("ideas", &path).unwrap();
        ed.text += "\n## Jefes\n- [ ] **dragón** con `fuego`\n1. uno\n> cita\n```\ncode\n```\n[web](https://example.com)\n";
        // render both views in a headless frame and save
        let ctx = egui::Context::default();
        for view in [View::Split, View::Preview, View::Edit] {
            ed.view = view;
            let mut tex = crate::tex::TexCache::default();
            let mut log = vec![];
            let mut open = vec![];
            let _ = ctx.run_ui(Default::default(), |ui| {
                let mut cx = Cx { project: &mut p, tex: &mut tex, log: &mut log, open: &mut open, active: true };
                ed.ui(ui, &mut cx);
            });
        }
        assert!(ed.dirty);
        let mut tex = crate::tex::TexCache::default();
        let (mut log, mut open) = (vec![], vec![]);
        let mut cx = Cx { project: &mut p, tex: &mut tex, log: &mut log, open: &mut open, active: true };
        ed.save(&mut cx).unwrap();
        assert!(!ed.dirty);
        assert!(std::fs::read_to_string(&path).unwrap().contains("dragón"));
        // the engine ignores notes
        modengine::Engine::load(&dir).expect("notes don't break the game");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
