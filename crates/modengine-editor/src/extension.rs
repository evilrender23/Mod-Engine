//! Extension editor: the metadata of `extensions/<name>/extension.toml` (name, version,
//! author, description, enabled, init/final hooks, constants, exported functions) and the
//! extension's script files, in load order, each opened in the script editor.

use crate::app::{Cx, Open};
use eframe::egui::{self, RichText};
use modengine::extension::{script_files, ExtFunction, ExtensionToml, META_FILE};
use std::path::{Path, PathBuf};

pub struct ExtensionEditor {
    pub name: String,
    dir: PathBuf,
    meta: ExtensionToml,
    saved: ExtensionToml,
    /// Constants being edited as text: (name, TOML value).
    consts: Vec<(String, String)>,
    new_file: String,
    error: Option<String>,
}

fn value_text(v: &toml::Value) -> String {
    v.to_string()
}

fn parse_value(s: &str) -> Result<toml::Value, String> {
    let t: toml::Table = toml::from_str(&format!("v = {s}")).map_err(|_| format!("valor no válido: {s} (usa 7, 2.5, true, \"texto\" o [1, 2])"))?;
    Ok(t["v"].clone())
}

/// Functions defined in a set of scripts: (name, parameters).
pub fn script_functions(sources: &[(String, String)]) -> Vec<(String, String)> {
    let mut out = vec![];
    for (rel, src) in sources {
        if let Ok(ast) = modscript::parse_file(rel, src) {
            for f in ast.functions {
                let params = f.params.iter().map(|p| p.name.to_string()).collect::<Vec<_>>().join(", ");
                out.push((f.name.to_string(), params));
            }
        }
    }
    out
}

impl ExtensionEditor {
    pub fn open(name: &str, dir: &Path) -> Result<Self, String> {
        let path = dir.join(META_FILE);
        let meta: ExtensionToml = if path.exists() {
            let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?
        } else {
            ExtensionToml { name: name.into(), ..Default::default() }
        };
        let consts = meta.constants.iter().map(|(k, v)| (k.clone(), value_text(v))).collect();
        Ok(ExtensionEditor { name: name.into(), dir: dir.into(), saved: meta.clone(), meta, consts, new_file: String::new(), error: None })
    }

    fn sync_consts(&mut self) -> Result<(), String> {
        let mut m = std::collections::BTreeMap::new();
        for (k, v) in &self.consts {
            if k.trim().is_empty() {
                continue;
            }
            if !crate::tomlw::valid_name(k.trim()) {
                return Err(format!("nombre de constante no válido: {k}"));
            }
            m.insert(k.trim().to_string(), parse_value(v)?);
        }
        self.meta.constants = m;
        Ok(())
    }

    pub fn dirty(&self) -> bool {
        let mut m = self.meta.clone();
        m.constants = self.consts.iter().filter_map(|(k, v)| Some((k.trim().to_string(), parse_value(v).ok()?))).filter(|(k, _)| !k.is_empty()).collect();
        m != self.saved
    }

    pub fn save(&mut self) -> Result<(), String> {
        self.sync_consts()?;
        let path = self.dir.join(META_FILE);
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        std::fs::write(&path, self.meta.to_toml()).map_err(|e| format!("{}: {e}", path.display()))?;
        self.saved = self.meta.clone();
        Ok(())
    }

    /// Script files (relative to the extension folder) in load order.
    fn ordered_files(&self) -> Vec<String> {
        let all = script_files(&self.dir);
        let mut out: Vec<String> = self.meta.files.iter().filter(|f| all.contains(f)).cloned().collect();
        for f in all {
            if !out.contains(&f) {
                out.push(f);
            }
        }
        out
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        let pal = crate::theme::colors(ui.ctx());
        let rel_dir = format!("extensions/{}", self.name);
        let files = self.ordered_files();
        let sources: Vec<(String, String)> = files
            .iter()
            .map(|f| {
                let rel = format!("{rel_dir}/{f}");
                let text = cx.project.script_index(&rel).map(|i| cx.project.scripts[i].text.clone()).unwrap_or_default();
                (rel, text)
            })
            .collect();
        let defined = script_functions(&sources);
        let fn_names: Vec<String> = defined.iter().map(|f| f.0.clone()).collect();

        egui::Panel::right(("ext_files", &self.name)).resizable(true).default_size(330.0).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.strong("Ficheros (orden de carga)");
                ui.label(RichText::new("Se cargan antes que los scripts del proyecto, en este orden.").weak());
                let mut order = files.clone();
                let mut moved = false;
                for (i, f) in files.iter().enumerate() {
                    let rel = format!("{rel_dir}/{f}");
                    ui.horizontal(|ui| {
                        ui.label(format!("{}.", i + 1));
                        if ui.link(f).on_hover_text("abrir en el editor de scripts").clicked() {
                            cx.open.push(Open::Script(rel.clone()));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add_enabled(i + 1 < files.len(), egui::Button::new("⏷").small()).on_hover_text("bajar").clicked() {
                                order.swap(i, i + 1);
                                moved = true;
                            }
                            if ui.add_enabled(i > 0, egui::Button::new("⏶").small()).on_hover_text("subir").clicked() {
                                order.swap(i, i - 1);
                                moved = true;
                            }
                        });
                    });
                }
                if moved {
                    self.meta.files = order;
                }
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.new_file).hint_text("nuevo.ms").desired_width(150.0));
                    if ui.button("+ Nuevo fichero").clicked() {
                        let f = self.new_file.trim().trim_end_matches(".ms").to_string();
                        if crate::tomlw::valid_name(&f) {
                            let rel = format!("{rel_dir}/{f}.ms");
                            if cx.project.script_index(&rel).is_none() {
                                let i = cx.project.add_script(&rel, format!("// {f}.ms: parte de la extensión {}\n", self.name));
                                match cx.project.save_script(i) {
                                    Ok(()) => {
                                        if !self.meta.files.is_empty() {
                                            self.meta.files.push(format!("{f}.ms"));
                                        }
                                        cx.open.push(Open::Script(rel));
                                        self.new_file.clear();
                                    }
                                    Err(e) => cx.error(e),
                                }
                            }
                        } else {
                            self.error = Some("nombre de fichero no válido".into());
                        }
                    }
                });
                ui.add_space(10.0);
                ui.strong("Funciones definidas");
                if defined.is_empty() {
                    ui.label(RichText::new("ninguna todavía").weak());
                }
                for (n, params) in &defined {
                    let exported = self.meta.functions.iter().any(|f| &f.name == n);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("{n}({params})")).monospace());
                        if !exported && ui.small_button("exportar").on_hover_text("añadir a las funciones exportadas").clicked() {
                            self.meta.functions.push(ExtFunction { name: n.clone(), args: params.clone(), help: String::new() });
                        }
                    });
                }
            });
        });

        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                if let Some(e) = &self.error {
                    ui.colored_label(pal.error, e);
                }
                egui::Grid::new(("ext_meta", &self.name)).num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label("Nombre");
                    ui.text_edit_singleline(&mut self.meta.name);
                    ui.end_row();
                    ui.label("Versión");
                    ui.text_edit_singleline(&mut self.meta.version);
                    ui.end_row();
                    ui.label("Autor");
                    ui.text_edit_singleline(&mut self.meta.author);
                    ui.end_row();
                    ui.label("Descripción");
                    ui.add(egui::TextEdit::multiline(&mut self.meta.description).desired_rows(2).desired_width(f32::INFINITY));
                    ui.end_row();
                    ui.label("Activa");
                    ui.checkbox(&mut self.meta.enabled, "cargar esta extensión");
                    ui.end_row();
                    for (label, hook, tip) in [("Al empezar", &mut self.meta.init, "función llamada una vez, al empezar el juego"), ("Al terminar", &mut self.meta.final_, "función llamada cuando se cierra el juego")] {
                        ui.label(label).on_hover_text(tip);
                        let text = hook.clone().unwrap_or_else(|| "(ninguna)".into());
                        egui::ComboBox::from_id_salt((label, &self.name)).selected_text(text).show_ui(ui, |ui| {
                            ui.selectable_value(hook, None, "(ninguna)");
                            for n in &fn_names {
                                ui.selectable_value(hook, Some(n.clone()), n);
                            }
                        });
                        ui.end_row();
                    }
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.strong("Constantes");
                    if ui.small_button("+").clicked() {
                        self.consts.push((format!("CONST_{}", self.consts.len() + 1), "0".into()));
                    }
                });
                let mut del = None;
                // (rows, not a Grid: a Grid sizes its text fields from the previous frame)
                for (i, (k, v)) in self.consts.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.add_sized([170.0, 20.0], egui::TextEdit::singleline(k));
                        ui.label("=");
                        let ok = parse_value(v).is_ok();
                        let r = ui.add_sized([150.0, 20.0], egui::TextEdit::singleline(v).text_color_opt((!ok).then_some(pal.error)));
                        if !ok {
                            r.on_hover_text("valor TOML: 7, 2.5, true, \"texto\", [1, 2]");
                        }
                        if ui.small_button("🗑").clicked() {
                            del = Some(i);
                        }
                    });
                }
                if let Some(i) = del {
                    self.consts.remove(i);
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.strong("Funciones exportadas");
                    if ui.small_button("+").clicked() {
                        self.meta.functions.push(ExtFunction::default());
                    }
                    if ui.small_button("exportar todas").on_hover_text("añade todas las funciones definidas en sus ficheros").clicked() {
                        for (n, params) in &defined {
                            if !self.meta.functions.iter().any(|f| &f.name == n) {
                                self.meta.functions.push(ExtFunction { name: n.clone(), args: params.clone(), help: String::new() });
                            }
                        }
                    }
                });
                ui.label(RichText::new("Documentan la API de la extensión; al cargar el juego se comprueba que existen.").weak());
                let mut del = None;
                ui.horizontal(|ui| {
                    ui.add_sized([150.0, 16.0], egui::Label::new(RichText::new("nombre").weak()));
                    ui.add_sized([110.0, 16.0], egui::Label::new(RichText::new("argumentos").weak()));
                    ui.label(RichText::new("ayuda").weak());
                });
                for (i, f) in self.meta.functions.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        let missing = !f.name.is_empty() && !fn_names.contains(&f.name);
                        let r = ui.add_sized([150.0, 20.0], egui::TextEdit::singleline(&mut f.name).text_color_opt(missing.then_some(pal.error)));
                        if missing {
                            r.on_hover_text("no está definida en los ficheros de la extensión");
                        }
                        ui.add_sized([110.0, 20.0], egui::TextEdit::singleline(&mut f.args));
                        let w = (ui.available_width() - 52.0).clamp(100.0, 260.0);
                        ui.add_sized([w, 20.0], egui::TextEdit::singleline(&mut f.help));
                        if ui.small_button("🗑").clicked() {
                            del = Some(i);
                        }
                    });
                }
                if let Some(i) = del {
                    self.meta.functions.remove(i);
                }
                ui.add_space(10.0);
                ui.label(
                    RichText::new("Las extensiones son ModScript: funciones y constantes globales que se cargan antes que los objetos. No se admiten extensiones nativas (DLL/.so) como en GameMaker.")
                        .weak()
                        .size(11.5),
                );
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_edit_and_save_round_trip() {
        let dir = crate::room::tests::temp_project("ext_ed");
        let mut p = crate::project::Project::open(&dir).unwrap();
        crate::fx::create(&mut p, crate::fx::FxKind::Extension, "mylib").unwrap();
        assert!(p.script_index("extensions/mylib/mylib.ms").is_some(), "its script is part of the project");
        let folder = p.fx.path(crate::fx::FxKind::Extension, "mylib").unwrap();
        let mut ed = ExtensionEditor::open("mylib", &folder).unwrap();
        assert_eq!(ed.meta.functions[0].name, "mylib_hello");
        ed.meta.author = "Ana".into();
        ed.consts.push(("MYLIB_K".into(), "42".into()));
        assert!(ed.dirty());
        ed.save().unwrap();
        assert!(!ed.dirty());
        let text = std::fs::read_to_string(folder.join(META_FILE)).unwrap();
        assert!(text.contains("author = \"Ana\"") && text.contains("MYLIB_K = 42"), "{text}");
        // the engine loads it: function + constant usable from the project
        std::fs::write(dir.join("scripts/use_lib.ms"), "global.k = MYLIB_K\nglobal.h = mylib_hello(\"x\")\n").unwrap();
        let mut e = modengine::Engine::load(&dir).unwrap();
        e.start();
        assert_eq!(e.globals[&modscript::intern("k")].to_string(), "42");
        assert_eq!(e.globals[&modscript::intern("h")].to_string(), "hola, x");
        let funcs = script_functions(&[("a.ms".into(), "function f(a, b) { return a }".into())]);
        assert_eq!(funcs, vec![("f".to_string(), "a, b".to_string())]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
