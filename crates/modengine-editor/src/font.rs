//! Font editor (`fonts/<name>.toml`), like GameMaker's: the face (a TTF/OTF of the project
//! or the system, or a sprite font), size, bold, italic, antialiasing, character ranges and
//! spacing, with a live preview baked by the engine's own font code (what the game shows).

use crate::app::{Cx, Open};
use crate::project::Project;
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, TextureHandle, TextureOptions, Vec2};
use modengine::font::{bake_sprite, bake_ttf, FontToml, GlyphAtlas, DEFAULT_RANGES};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Source {
    File,
    Sprite,
    Builtin,
}

pub struct FontEditor {
    pub name: String,
    path: PathBuf,
    t: FontToml,
    saved: FontToml,
    pub dirty: bool,
    /// A font chosen from outside the project, copied into `fonts/` on save.
    pending: Option<PathBuf>,
    sample: String,
    zoom: f32,
    color: [u8; 3],
    show_page: bool,
    filter: String,
    new_range: [u32; 2],
    /// Baked preview and the settings it was baked with.
    baked: Option<(String, Result<GlyphAtlas, String>)>,
    tex: Option<TextureHandle>,
}

/// Fonts installed on the system (scanned once).
fn system_fonts() -> &'static [PathBuf] {
    static FONTS: OnceLock<Vec<PathBuf>> = OnceLock::new();
    FONTS.get_or_init(modengine::font::system_fonts)
}

/// A sensible default face for new fonts (free fonts that come with most systems).
fn default_system_font() -> Option<PathBuf> {
    let all = system_fonts();
    for want in ["dejavusans.ttf", "liberationsans-regular.ttf", "notosans-regular.ttf", "arial.ttf", "helvetica.ttc"] {
        if let Some(p) = all.iter().find(|p| p.file_name().is_some_and(|f| f.to_string_lossy().to_lowercase() == want)) {
            return Some(p.clone());
        }
    }
    None
}

/// Creates `fonts/<name>.toml`, copying `file` (or a default system font) into `fonts/`.
pub fn create(p: &mut Project, name: &str, file: &str) -> Result<Open, String> {
    let dir = p.dir.join("fonts");
    let toml_path = dir.join(format!("{name}.toml"));
    if toml_path.exists() {
        return Err(format!("{} ya existe", toml_path.display()));
    }
    let src = if file.trim().is_empty() { default_system_font() } else { Some(crate::import::expand(file)) };
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut t = FontToml { size: Some(16.0), ..Default::default() };
    if let Some(src) = src {
        if !src.is_file() {
            return Err(format!("no existe el fichero '{}'", src.display()));
        }
        t.file = Some(copy_in(&dir, name, &src)?);
    }
    std::fs::write(&toml_path, modengine::font::write_toml(&t)).map_err(|e| e.to_string())?;
    p.reload_lists();
    Ok(Open::Font(name.to_string()))
}

/// Copies a TTF/OTF into `fonts/` as `<name>.<ext>`; returns the file name.
fn copy_in(dir: &Path, name: &str, src: &Path) -> Result<String, String> {
    let ext = src.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_else(|| "ttf".into());
    let file = format!("{name}.{ext}");
    let dst = dir.join(&file);
    if src.canonicalize().ok() != dst.canonicalize().ok() {
        std::fs::copy(src, &dst).map_err(|e| format!("{}: {e}", src.display()))?;
    }
    Ok(file)
}

fn ranges_of(t: &FontToml) -> Vec<[u32; 2]> {
    if t.ranges.is_empty() {
        DEFAULT_RANGES.to_vec()
    } else {
        t.ranges.clone()
    }
}

/// Sorted, merged character ranges.
fn merge(mut r: Vec<[u32; 2]>) -> Vec<[u32; 2]> {
    r.sort();
    let mut out: Vec<[u32; 2]> = vec![];
    for x in r {
        match out.last_mut() {
            Some(l) if x[0] <= l[1] + 1 => l[1] = l[1].max(x[1]),
            _ => out.push(x),
        }
    }
    out
}

impl FontEditor {
    pub fn open(name: &str, path: &Path, is_toml: bool) -> Result<Self, String> {
        let t: FontToml = if is_toml {
            let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
            toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?
        } else {
            FontToml { file: path.file_name().map(|f| f.to_string_lossy().to_string()), ..Default::default() }
        };
        Ok(FontEditor {
            zoom: if t.antialias == Some(false) || t.sprite.is_some() { 3.0 } else { 1.0 },
            name: name.into(),
            path: if is_toml { path.into() } else { path.with_extension("toml") },
            saved: if is_toml { t.clone() } else { FontToml::default() },
            dirty: !is_toml,
            t,
            pending: None,
            sample: "El veloz murciélago hindú comía feliz cardillo y kiwi.\nABCDEFGHIJ abcdefghij 0123456789 ¡¿?! @#&".into(),
            color: [255, 255, 255],
            show_page: false,
            filter: String::new(),
            new_range: [160, 255],
            baked: None,
            tex: None,
        })
    }

    fn source(&self) -> Source {
        if self.t.sprite.is_some() {
            Source::Sprite
        } else if self.t.file.is_some() || self.pending.is_some() {
            Source::File
        } else {
            Source::Builtin
        }
    }

    fn dir(&self) -> PathBuf {
        self.path.parent().map(Path::to_path_buf).unwrap_or_default()
    }

    /// The TTF/OTF file the preview reads.
    fn face_path(&self) -> Option<PathBuf> {
        self.pending.clone().or_else(|| self.t.file.as_ref().map(|f| self.dir().join(f)))
    }

    pub fn save(&mut self, cx: &mut Cx) -> Result<(), String> {
        let dir = self.dir();
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        if let Some(src) = self.pending.take() {
            self.t.file = Some(copy_in(&dir, &self.name, &src)?);
            self.t.sprite = None;
        }
        std::fs::write(&self.path, modengine::font::write_toml(&self.t)).map_err(|e| format!("{}: {e}", self.path.display()))?;
        self.saved = self.t.clone();
        self.dirty = false;
        cx.project.reload_lists();
        Ok(())
    }

    /// Bakes the preview again when a setting changed.
    fn bake(&mut self, ctx: &egui::Context, p: &Project) {
        let key = format!("{:?}|{:?}", self.t, self.pending);
        if self.baked.as_ref().is_some_and(|(k, _)| *k == key) {
            return;
        }
        let atlas = match self.source() {
            Source::Sprite => {
                let s = self.t.sprite.clone().unwrap_or_default();
                match p.sprite(&s) {
                    Some(spr) => Ok(bake_sprite(spr, &self.t.chars.clone().unwrap_or_else(|| (32u8..127).map(|c| c as char).collect()), self.t.proportional.unwrap_or(true), self.t.spacing)),
                    None => Err(format!("el sprite '{s}' no existe")),
                }
            }
            Source::File => match self.face_path().map(|f| std::fs::read(&f).map_err(|e| format!("{}: {e}", f.display()))) {
                Some(Ok(data)) => bake_ttf(&data, &ranges_of(&self.t), self.t.size.unwrap_or(16.0).clamp(1.0, 512.0), self.t.bold, self.t.italic, self.t.antialias.unwrap_or(true), self.t.spacing),
                Some(Err(e)) => Err(e),
                None => Err("sin fichero".into()),
            },
            Source::Builtin => Err("La fuente integrada del motor no tiene vista previa exacta: elige un fichero TTF/OTF para controlar el aspecto.".into()),
        };
        self.tex = atlas.as_ref().ok().map(|a| {
            let img = egui::ColorImage::from_rgba_unmultiplied([a.w as usize, a.h as usize], &a.rgba);
            let filter = if self.t.antialias == Some(false) || self.source() == Source::Sprite { TextureOptions::NEAREST } else { TextureOptions::LINEAR };
            ctx.load_texture(format!("font_preview:{}", self.name), img, filter)
        });
        self.baked = Some((key, atlas));
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        self.bake(ui.ctx(), cx.project);
        egui::Panel::left(ui.scope_id().with("font_props")).resizable(true).default_size(300.0).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.props(ui, cx.project));
        });
        egui::CentralPanel::default().show(ui, |ui| self.preview(ui));
        // a loose .ttf has no .toml yet: saving writes one
        self.dirty = self.t != self.saved || self.pending.is_some() || !self.path.exists();
    }

    fn props(&mut self, ui: &mut egui::Ui, p: &Project) {
        ui.heading(&self.name);
        let mut src = self.source();
        ui.horizontal(|ui| {
            ui.label("Origen:");
            for (s, label) in [(Source::File, "TTF/OTF"), (Source::Sprite, "Sprite"), (Source::Builtin, "Integrada")] {
                if ui.selectable_value(&mut src, s, label).changed() {
                    match s {
                        Source::File => {
                            self.t.sprite = None;
                            if self.t.file.is_none() && self.pending.is_none() {
                                self.pending = default_system_font();
                            }
                        }
                        Source::Sprite => {
                            self.t.file = None;
                            self.pending = None;
                            self.t.sprite = p.sprite_names().first().cloned();
                            self.t.chars.get_or_insert_with(|| "0123456789".into());
                        }
                        Source::Builtin => {
                            self.t.file = None;
                            self.t.sprite = None;
                            self.pending = None;
                        }
                    }
                }
            }
        });
        ui.separator();
        match self.source() {
            Source::File => self.file_props(ui, p),
            Source::Sprite => self.sprite_props(ui, p),
            Source::Builtin => {
                ui.label(RichText::new("Usa la fuente por defecto del juego al tamaño indicado.").weak());
                size_row(ui, &mut self.t);
            }
        }
        ui.horizontal(|ui| {
            ui.label("Espaciado extra");
            ui.add(egui::DragValue::new(&mut self.t.spacing).range(-8.0..=32.0).speed(0.1).suffix(" px"));
        });
    }

    fn file_props(&mut self, ui: &mut egui::Ui, p: &Project) {
        let shown = match (&self.pending, &self.t.file) {
            (Some(p), _) => format!("{}  (se copiará a fonts/ al guardar)", p.file_name().unwrap_or_default().to_string_lossy()),
            (None, Some(f)) => format!("fonts/{f}"),
            _ => "—".into(),
        };
        ui.label(RichText::new(shown).monospace());
        size_row(ui, &mut self.t);
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.t.bold, "Negrita");
            ui.checkbox(&mut self.t.italic, "Cursiva");
            let mut aa = self.t.antialias.unwrap_or(true);
            if ui.checkbox(&mut aa, "Antialias").on_hover_text("desactívalo para fuentes pixel art nítidas").changed() {
                self.t.antialias = if aa { None } else { Some(false) };
            }
        });
        ui.add_space(4.0);
        ui.strong("Rangos de caracteres");
        let mut ranges = ranges_of(&self.t);
        let mut remove = None;
        for (i, r) in ranges.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut r[0]).range(0..=0x10FFFF));
                ui.label("–");
                ui.add(egui::DragValue::new(&mut r[1]).range(0..=0x10FFFF));
                let show = |c: u32| char::from_u32(c).filter(|c| !c.is_control()).map(|c| c.to_string()).unwrap_or_else(|| "·".into());
                ui.label(RichText::new(format!("{} … {}", show(r[0]), show(r[1]))).weak());
                if ui.small_button("🗑").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            ranges.remove(i);
        }
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut self.new_range[0]).range(0..=0x10FFFF));
            ui.add(egui::DragValue::new(&mut self.new_range[1]).range(0..=0x10FFFF));
            if ui.button("+ rango").clicked() {
                ranges.push([self.new_range[0].min(self.new_range[1]), self.new_range[0].max(self.new_range[1])]);
            }
        });
        ui.horizontal_wrapped(|ui| {
            if ui.small_button("ASCII").on_hover_text("32–127").clicked() {
                ranges = DEFAULT_RANGES.to_vec();
            }
            if ui.small_button("+ Latín-1 (ñ, á, ¿…)").on_hover_text("160–255").clicked() {
                ranges.push([160, 255]);
            }
            if ui.small_button("+ del texto de prueba").on_hover_text("añade los caracteres del texto de prueba que falten").clicked() {
                ranges.extend(self.sample.chars().filter(|c| !c.is_control()).map(|c| [c as u32, c as u32]));
            }
        });
        let ranges = merge(ranges.into_iter().map(|r| [r[0].min(r[1]), r[0].max(r[1])]).collect());
        self.t.ranges = if ranges == DEFAULT_RANGES { vec![] } else { ranges };

        ui.add_space(6.0);
        ui.strong("Elegir fuente");
        ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("buscar en el proyecto y el sistema"));
        let f = self.filter.to_lowercase();
        let mut pick: Option<PathBuf> = None;
        let mut pick_project: Option<String> = None;
        egui::ScrollArea::vertical().id_salt("font_pick").max_height(220.0).show(ui, |ui| {
            let fonts_dir = p.dir.join("fonts");
            if let Ok(rd) = std::fs::read_dir(&fonts_dir) {
                let mut files: Vec<String> = rd.flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| {
                    let l = n.to_lowercase();
                    (l.ends_with(".ttf") || l.ends_with(".otf")) && l.contains(&f)
                }).collect();
                files.sort();
                for n in files {
                    let sel = self.pending.is_none() && self.t.file.as_deref() == Some(n.as_str());
                    if ui.selectable_label(sel, format!("📁 {n}")).on_hover_text("del proyecto").clicked() {
                        pick_project = Some(n);
                    }
                }
            }
            let list: Vec<&PathBuf> = system_fonts().iter().filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase().contains(&f))).take(400).collect();
            for sp in list {
                let n = sp.file_name().unwrap_or_default().to_string_lossy().to_string();
                if ui.selectable_label(self.pending.as_ref() == Some(sp), format!("🖥 {n}")).on_hover_text(sp.display().to_string()).clicked() {
                    pick = Some(sp.clone());
                }
            }
        });
        if let Some(n) = pick_project {
            self.t.file = Some(n);
            self.pending = None;
        }
        if let Some(sp) = pick {
            self.pending = Some(sp);
        }
    }

    fn sprite_props(&mut self, ui: &mut egui::Ui, p: &Project) {
        egui::Grid::new(ui.scope_id().with("sprfont")).num_columns(2).show(ui, |ui| {
            ui.label("Sprite");
            let cur = self.t.sprite.clone().unwrap_or_default();
            egui::ComboBox::from_id_salt(ui.scope_id().with("font_sprite")).selected_text(cur).show_ui(ui, |ui| {
                for s in p.sprite_names() {
                    ui.selectable_value(&mut self.t.sprite, Some(s.clone()), s);
                }
            });
            ui.end_row();
            ui.label("Caracteres");
            let mut chars = self.t.chars.clone().unwrap_or_default();
            if ui.add(egui::TextEdit::singleline(&mut chars).desired_width(180.0)).on_hover_text("un fotograma por carácter, en este orden").changed() {
                self.t.chars = Some(chars);
            }
            ui.end_row();
            ui.label("Proporcional");
            let mut prop = self.t.proportional.unwrap_or(true);
            if ui.checkbox(&mut prop, "recortar cada glifo").changed() {
                self.t.proportional = Some(prop);
            }
            ui.end_row();
        });
        if let Some(spr) = self.t.sprite.as_deref().and_then(|s| p.sprite(s)) {
            let n = self.t.chars.as_deref().unwrap_or("").chars().count();
            let msg = format!("{} fotogramas, {} caracteres", spr.frames.len(), n);
            ui.label(RichText::new(msg).weak());
        }
    }

    fn preview(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Zoom");
            ui.add(egui::Slider::new(&mut self.zoom, 1.0..=6.0).step_by(0.5));
            ui.color_edit_button_srgb(&mut self.color);
            ui.checkbox(&mut self.show_page, "ver página de glifos");
        });
        ui.add(egui::TextEdit::multiline(&mut self.sample).desired_rows(2).desired_width(f32::INFINITY));
        let Some((_, atlas)) = &self.baked else { return };
        let atlas = match atlas {
            Ok(a) => a,
            Err(e) => {
                ui.colored_label(crate::theme::colors(ui.ctx()).error, e);
                return;
            }
        };
        let Some(tex) = &self.tex else { return };
        let (w, h) = atlas.measure(&self.sample, 1.0);
        let missing: String = {
            let mut m: Vec<char> = self.sample.chars().filter(|c| !c.is_control() && !atlas.glyphs.contains_key(c)).collect();
            m.sort();
            m.dedup();
            m.into_iter().collect()
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} glifos · línea {} px · texto {} × {} px", atlas.glyphs.len(), atlas.line_height, w.round(), h.round())).weak());
            if !missing.is_empty() {
                ui.colored_label(crate::theme::colors(ui.ctx()).error, format!("faltan: {missing}")).on_hover_text("no están en los rangos de caracteres (o la fuente no los tiene): el juego no los dibuja");
            }
        });
        let z = self.zoom;
        let tint = Color32::from_rgb(self.color[0], self.color[1], self.color[2]);
        egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
            let size = if self.show_page { Vec2::new(atlas.w as f32, atlas.h as f32) * z } else { Vec2::new(w.max(1.0), h.max(1.0)) * z + Vec2::splat(16.0) };
            let (rect, _) = ui.allocate_exact_size(size.max(ui.available_size()), Sense::hover());
            let painter = ui.painter_at(rect);
            let bg = if self.color.iter().map(|c| *c as u32).sum::<u32>() > 380 { Color32::from_rgb(34, 32, 52) } else { Color32::from_rgb(230, 230, 236) };
            painter.rect_filled(rect, 4.0, bg);
            let uv = |g: &modengine::font::Glyph| Rect::from_min_max(Pos2::new(g.x as f32 / atlas.w as f32, g.y as f32 / atlas.h as f32), Pos2::new((g.x + g.w) as f32 / atlas.w as f32, (g.y + g.h) as f32 / atlas.h as f32));
            if self.show_page {
                crate::tex::checkerboard(&painter, Rect::from_min_size(rect.min, Vec2::new(atlas.w as f32, atlas.h as f32) * z), 8.0);
                painter.image(tex.id(), Rect::from_min_size(rect.min, Vec2::new(atlas.w as f32, atlas.h as f32) * z), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), tint);
                return;
            }
            let origin = rect.min + Vec2::splat(8.0);
            for (k, line) in self.sample.split('\n').enumerate() {
                let top = origin.y + k as f32 * atlas.line_height * z;
                for q in atlas.layout_line(line) {
                    let r = Rect::from_min_size(Pos2::new(origin.x + q.x * z, top + q.y * z), Vec2::new(q.glyph.w as f32, q.glyph.h as f32) * z);
                    painter.image(tex.id(), r, uv(&q.glyph), tint);
                }
            }
        });
    }
}

fn size_row(ui: &mut egui::Ui, t: &mut FontToml) {
    ui.horizontal(|ui| {
        ui.label("Tamaño");
        let mut s = t.size.unwrap_or(16.0);
        if ui.add(egui::DragValue::new(&mut s).range(4.0..=256.0).speed(0.25).suffix(" px")).changed() {
            t.size = Some(s);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;

    #[test]
    fn ranges_merge() {
        assert_eq!(merge(vec![[160, 255], [32, 127], [100, 140], [241, 241]]), vec![[32, 140], [160, 255]]);
        assert_eq!(merge(vec![[65, 65], [66, 66]]), vec![[65, 66]]);
    }

    #[test]
    fn create_edit_save_and_load_in_the_engine() {
        let Some(sys) = default_system_font().or_else(|| system_fonts().first().cloned()) else { return };
        let dir = crate::room::tests::temp_project("fontedit");
        let mut p = Project::open(&dir).unwrap();
        assert_eq!(create(&mut p, "fnt_ui", &sys.display().to_string()).unwrap(), Open::Font("fnt_ui".into()));
        let entry = p.fonts.iter().find(|f| f.name == "fnt_ui").unwrap();
        assert!(entry.is_toml);
        let mut ed = FontEditor::open("fnt_ui", &entry.path.clone(), true).unwrap();
        ed.t.size = Some(24.0);
        ed.t.antialias = Some(false);
        ed.t.ranges = vec![[32, 127], [160, 255]];
        struct St {
            ed: FontEditor,
            p: Project,
            tex: crate::tex::TexCache,
        }
        let mut h = Harness::builder().with_size([1000.0, 640.0]).allow_missing_glyphs().with_accessibility_check(false).build_ui_state(
            |ui, s: &mut St| {
                let (mut log, mut open) = (vec![], vec![]);
                let mut cx = Cx { project: &mut s.p, tex: &mut s.tex, log: &mut log, open: &mut open, active: true };
                s.ed.ui(ui, &mut cx);
            },
            St { ed, p, tex: Default::default() },
        );
        h.run_steps(3);
        let s = h.state_mut();
        assert!(s.ed.dirty);
        let atlas = s.ed.baked.as_ref().unwrap().1.as_ref().expect("preview baked");
        assert!(atlas.glyphs.contains_key(&'ñ'));
        assert!(atlas.rgba.chunks(4).all(|px| px[3] == 0 || px[3] == 255), "no antialias");
        let (mut log, mut open) = (vec![], vec![]);
        let mut cx = Cx { project: &mut s.p, tex: &mut s.tex, log: &mut log, open: &mut open, active: true };
        s.ed.save(&mut cx).unwrap();
        assert!(!s.ed.dirty);
        let e = modengine::Engine::load(&dir).unwrap();
        let f = e.fonts.iter().find(|f| f.name == "fnt_ui").unwrap();
        assert_eq!(f.size, 24.0);
        assert!(!f.antialias);
        assert!(f.atlas.as_ref().unwrap().glyphs.contains_key(&'¿'));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
