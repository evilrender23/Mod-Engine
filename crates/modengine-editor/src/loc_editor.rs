//! Idiomas: the string tables of `lang/*.toml` as one table, keys × languages. Missing
//! translations are highlighted; keys and languages can be added, rows searched, and the
//! whole table exported to / imported from CSV for translators.
//!
//! Plural entries show as one line per form (`one: {n} moneda`).

use crate::app::Cx;
use eframe::egui::{self, Color32, RichText};
use modengine::loc::{Entry, Table, PLURAL_KEYS};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub text: String,
    pub plural: bool,
}

impl Cell {
    fn from_entry(e: &Entry) -> Cell {
        match e {
            Entry::Text(s) => Cell { text: s.clone(), plural: false },
            Entry::Plural(f) => Cell { text: f.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join("\n"), plural: true },
        }
    }
    /// Back to a table entry; a plural cell whose lines aren't `form: text` stays plain text.
    fn to_entry(&self) -> Entry {
        if self.plural || self.text.lines().all(|l| l.split_once(": ").is_some_and(|(k, _)| PLURAL_KEYS.contains(&k))) && self.text.lines().any(|l| l.starts_with("other: ")) {
            let mut f = BTreeMap::new();
            for l in self.text.lines() {
                if let Some((k, v)) = l.split_once(": ").filter(|(k, _)| PLURAL_KEYS.contains(k)) {
                    f.insert(k.to_string(), v.to_string());
                }
            }
            if f.contains_key("other") {
                return Entry::Plural(f);
            }
        }
        Entry::Text(self.text.clone())
    }
}

pub struct LangEditor {
    pub name: String,
    dir: PathBuf,
    pub langs: Vec<String>,
    pub keys: Vec<String>,
    /// (language, key) → text
    pub cells: HashMap<(String, String), Cell>,
    pub default: String,
    pub dirty: bool,
    pub search: String,
    pub only_missing: bool,
    new_key: String,
    new_lang: String,
    status: String,
}

/// Name of the single "asset" that stands for the whole table in the browser.
pub const TABLE_NAME: &str = "textos";

pub fn valid_key(k: &str) -> bool {
    !k.is_empty() && !k.starts_with('.') && !k.ends_with('.') && !k.contains("..") && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
}

pub fn valid_lang(c: &str) -> bool {
    !c.is_empty() && c.len() <= 12 && c != modengine::loc::PSEUDO && c.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Parses CSV (RFC 4180 quoting) into rows of fields.
pub fn parse_csv(src: &str) -> Vec<Vec<String>> {
    let mut rows = vec![];
    let mut row = vec![];
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = src.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => quoted = false,
                c => field.push(c),
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            c => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

impl LangEditor {
    pub fn open(dir: &Path) -> LangEditor {
        let l = modengine::loc::load(dir);
        let mut ed = LangEditor {
            name: TABLE_NAME.into(),
            dir: dir.to_path_buf(),
            langs: l.tables.keys().cloned().collect(),
            keys: vec![],
            cells: HashMap::new(),
            default: l.default.clone(),
            dirty: false,
            search: String::new(),
            only_missing: false,
            new_key: String::new(),
            new_lang: String::new(),
            status: l.errors.join("\n"),
        };
        // the default language first
        if let Some(i) = ed.langs.iter().position(|c| *c == ed.default) {
            let d = ed.langs.remove(i);
            ed.langs.insert(0, d);
        }
        let mut keys = std::collections::BTreeSet::new();
        for (code, t) in &l.tables {
            for (k, v) in t {
                keys.insert(k.clone());
                ed.cells.insert((code.clone(), k.clone()), Cell::from_entry(v));
            }
        }
        ed.keys = keys.into_iter().collect();
        ed
    }

    pub fn missing(&self, lang: &str, key: &str) -> bool {
        self.cells.get(&(lang.to_string(), key.to_string())).is_none_or(|c| c.text.is_empty())
    }

    pub fn missing_count(&self) -> usize {
        self.langs.iter().map(|l| self.keys.iter().filter(|k| self.missing(l, k)).count()).sum()
    }

    pub fn set(&mut self, lang: &str, key: &str, text: &str) {
        if !self.langs.iter().any(|l| l == lang) {
            self.langs.push(lang.to_string());
        }
        if !self.keys.iter().any(|k| k == key) {
            self.keys.push(key.to_string());
            self.keys.sort();
        }
        let plural = self.cells.get(&(lang.to_string(), key.to_string())).is_some_and(|c| c.plural);
        self.cells.insert((lang.to_string(), key.to_string()), Cell { text: text.to_string(), plural });
        self.dirty = true;
    }

    pub fn add_key(&mut self, key: &str) -> Result<(), String> {
        let key = key.trim();
        if !valid_key(key) {
            return Err(format!("'{key}' no vale como clave: letras, números, _ - y puntos (menu.jugar)"));
        }
        if self.keys.iter().any(|k| k == key) {
            return Err(format!("la clave '{key}' ya existe"));
        }
        if let Some(clash) = self.keys.iter().find(|k| k.starts_with(&format!("{key}.")) || key.starts_with(&format!("{k}."))) {
            return Err(format!("'{key}' choca con '{clash}' (una clave no puede ser a la vez texto y grupo)"));
        }
        self.keys.push(key.to_string());
        self.keys.sort();
        self.dirty = true;
        Ok(())
    }

    pub fn add_lang(&mut self, code: &str) -> Result<(), String> {
        let code = code.trim();
        if !valid_lang(code) {
            return Err(format!("'{code}' no vale como código de idioma (es, en, pt-BR...)"));
        }
        if self.langs.iter().any(|l| l == code) {
            return Err(format!("el idioma '{code}' ya existe"));
        }
        self.langs.push(code.to_string());
        self.dirty = true;
        Ok(())
    }

    pub fn remove_key(&mut self, key: &str) {
        self.keys.retain(|k| k != key);
        self.cells.retain(|(_, k), _| k != key);
        self.dirty = true;
    }

    /// One table per language, as the engine reads it.
    pub fn tables(&self) -> BTreeMap<String, Table> {
        let mut out = BTreeMap::new();
        for l in &self.langs {
            let mut t = Table::new();
            for k in &self.keys {
                if let Some(c) = self.cells.get(&(l.clone(), k.clone())).filter(|c| !c.text.is_empty()) {
                    t.insert(k.clone(), c.to_entry());
                }
            }
            out.insert(l.clone(), t);
        }
        out
    }

    /// Writes `lang/<code>.toml` for every language.
    pub fn save_files(&mut self) -> Result<(), String> {
        let folder = self.dir.join("lang");
        std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
        for (code, t) in self.tables() {
            let path = folder.join(format!("{code}.toml"));
            std::fs::write(&path, modengine::loc::write_table(&t)).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        self.dirty = false;
        Ok(())
    }

    pub fn save(&mut self, cx: &mut Cx) -> Result<(), String> {
        self.save_files()?;
        cx.project.fx = crate::fx::Lists::load(&cx.project.dir);
        Ok(())
    }

    /// `key,es,en...` with one row per key (plural forms as `one: ...` lines).
    pub fn to_csv(&self) -> String {
        let mut s = String::from("key");
        for l in &self.langs {
            s.push(',');
            s.push_str(&csv_field(l));
        }
        s.push('\n');
        for k in &self.keys {
            s.push_str(&csv_field(k));
            for l in &self.langs {
                s.push(',');
                s.push_str(&csv_field(self.cells.get(&(l.clone(), k.clone())).map(|c| c.text.as_str()).unwrap_or("")));
            }
            s.push('\n');
        }
        s
    }

    /// Merges a CSV made by [`to_csv`] (or a translator): new keys and languages are added,
    /// non-empty cells replace the current text. Returns how many cells changed.
    pub fn import_csv(&mut self, src: &str) -> Result<usize, String> {
        let rows = parse_csv(src);
        let Some(head) = rows.first() else { return Err("el CSV está vacío".into()) };
        if head.first().map(|h| h.trim().to_lowercase()) != Some("key".into()) && head.first().map(|h| h.trim().to_lowercase()) != Some("clave".into()) {
            return Err("la primera columna del CSV debe llamarse key (o clave)".into());
        }
        let langs: Vec<String> = head[1..].iter().map(|l| l.trim().to_string()).collect();
        if let Some(bad) = langs.iter().find(|l| !valid_lang(l)) {
            return Err(format!("columna '{bad}': no es un código de idioma"));
        }
        let mut n = 0;
        for row in &rows[1..] {
            let Some(key) = row.first().map(|k| k.trim()).filter(|k| !k.is_empty()) else { continue };
            if !valid_key(key) {
                return Err(format!("'{key}' no vale como clave"));
            }
            for (i, l) in langs.iter().enumerate() {
                if let Some(t) = row.get(i + 1).filter(|t| !t.is_empty()) {
                    if self.cells.get(&(l.clone(), key.to_string())).map(|c| &c.text) != Some(t) {
                        self.set(l, key, t);
                        n += 1;
                    }
                }
            }
            if !self.keys.iter().any(|k| k == key) {
                self.keys.push(key.to_string());
                self.keys.sort();
            }
        }
        for l in &langs {
            if !self.langs.contains(l) {
                self.langs.push(l.clone());
            }
        }
        if n > 0 {
            self.dirty = true;
        }
        Ok(n)
    }

    fn visible(&self, key: &str) -> bool {
        if self.only_missing && !self.langs.iter().any(|l| self.missing(l, key)) {
            return false;
        }
        let q = self.search.trim().to_lowercase();
        q.is_empty() || key.to_lowercase().contains(&q) || self.langs.iter().any(|l| self.cells.get(&(l.clone(), key.to_string())).is_some_and(|c| c.text.to_lowercase().contains(&q)))
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        ui.horizontal_wrapped(|ui| {
            ui.label("🔍");
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("buscar clave o texto").desired_width(180.0));
            ui.checkbox(&mut self.only_missing, "solo con huecos");
            ui.separator();
            ui.add(egui::TextEdit::singleline(&mut self.new_key).hint_text("menu.jugar").desired_width(120.0));
            if ui.button("＋ Clave").clicked() {
                let k = self.new_key.clone();
                match self.add_key(&k) {
                    Ok(()) => {
                        self.new_key.clear();
                        self.status = format!("clave '{k}' añadida");
                    }
                    Err(e) => self.status = e,
                }
            }
            ui.add(egui::TextEdit::singleline(&mut self.new_lang).hint_text("fr").desired_width(48.0));
            if ui.button("＋ Idioma").clicked() {
                let c = self.new_lang.clone();
                match self.add_lang(&c) {
                    Ok(()) => {
                        self.new_lang.clear();
                        self.status = format!("idioma '{c}' añadido (se crea lang/{c}.toml al guardar)");
                    }
                    Err(e) => self.status = e,
                }
            }
            ui.separator();
            if ui.button("⬆ Exportar CSV").on_hover_text("Una fila por clave y una columna por idioma, para traductores").clicked() {
                if let Some(path) = rfd::FileDialog::new().set_file_name("traducciones.csv").add_filter("CSV", &["csv"]).set_directory(&self.dir).save_file() {
                    self.status = match std::fs::write(&path, self.to_csv()) {
                        Ok(()) => format!("exportado a {}", path.display()),
                        Err(e) => format!("{}: {e}", path.display()),
                    };
                }
            }
            if ui.button("⬇ Importar CSV").on_hover_text("Añade las claves e idiomas nuevos y reemplaza las celdas no vacías").clicked() {
                if let Some(path) = rfd::FileDialog::new().add_filter("CSV", &["csv"]).set_directory(&self.dir).pick_file() {
                    self.status = match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|s| self.import_csv(&s)) {
                        Ok(n) => format!("{n} celda(s) importadas de {}", path.display()),
                        Err(e) => cx_err(cx, e),
                    };
                }
            }
        });
        let missing = self.missing_count();
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} claves × {} idiomas", self.keys.len(), self.langs.len())).weak());
            if missing > 0 {
                ui.label(RichText::new(format!("· {missing} huecos sin traducir")).color(Color32::from_rgb(230, 120, 100)));
            } else if !self.keys.is_empty() {
                ui.label(RichText::new("· todo traducido ✔").color(Color32::from_rgb(120, 200, 120)));
            }
            if !self.status.is_empty() {
                ui.label(RichText::new(&self.status).italics());
            }
        });
        ui.label(RichText::new("En el código: loc(\"clave\", { n: 3 }). Plurales: una línea por forma (one: …, other: …). Guardar reescribe lang/*.toml.").weak().small());
        ui.separator();
        if self.langs.is_empty() {
            ui.label("No hay idiomas: escribe un código (es, en…) y pulsa ＋ Idioma.");
            return;
        }
        let keys: Vec<String> = self.keys.iter().filter(|k| self.visible(k)).cloned().collect();
        let langs = self.langs.clone();
        let mut remove = None;
        let col_w = ((ui.available_width() - 190.0) / langs.len() as f32).clamp(150.0, 420.0);
        egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new("loc_table").striped(true).spacing([6.0, 4.0]).min_col_width(60.0).show(ui, |ui| {
                ui.label(RichText::new("clave").strong());
                for l in &langs {
                    let t = if *l == self.default { format!("{l} (base)") } else { l.clone() };
                    ui.label(RichText::new(t).strong());
                }
                ui.label("");
                ui.end_row();
                for k in &keys {
                    ui.label(RichText::new(k).monospace());
                    for l in &langs {
                        let id = (l.clone(), k.clone());
                        let gap = self.missing(l, k);
                        let fill = if gap { Color32::from_rgba_unmultiplied(200, 60, 50, 60) } else { Color32::TRANSPARENT };
                        let stroke = if gap { egui::Stroke::new(1.5, Color32::from_rgb(220, 90, 70)) } else { egui::Stroke::NONE };
                        egui::Frame::new().fill(fill).stroke(stroke).inner_margin(1.0).corner_radius(3.0).show(ui, |ui| {
                            let cell = self.cells.entry(id.clone()).or_insert_with(|| Cell { text: String::new(), plural: false });
                            let lines = cell.text.lines().count().max(1);
                            let te = if lines > 1 || cell.plural {
                                egui::TextEdit::multiline(&mut cell.text).desired_rows(lines).desired_width(col_w)
                            } else {
                                egui::TextEdit::singleline(&mut cell.text).desired_width(col_w)
                            };
                            let te = te.min_size(egui::vec2(col_w, 0.0)).id_salt(("loc_cell", l, k)).hint_text(RichText::new("falta").color(Color32::from_rgb(230, 120, 100)));
                            if ui.add(te).changed() {
                                self.dirty = true;
                            }
                        });
                    }
                    if ui.small_button("🗑").on_hover_text("borrar la clave en todos los idiomas").clicked() {
                        remove = Some(k.clone());
                    }
                    ui.end_row();
                }
            });
        });
        // empty cells created for editing are not translations
        self.cells.retain(|_, c| !c.text.is_empty());
        if let Some(k) = remove {
            self.remove_key(&k);
        }
    }
}

fn cx_err(cx: &mut Cx, e: String) -> String {
    cx.error(format!("Idiomas: {e}"));
    e
}

// ------------------------------------------------------------------ browser / inspector

pub fn langs_of(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut v: Vec<(String, PathBuf)> = std::fs::read_dir(dir.join("lang"))
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "toml")).filter_map(|p| Some((p.file_stem()?.to_string_lossy().to_string(), p))).collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// Inspector details of the table.
pub fn inspector_rows(dir: &Path) -> Vec<(&'static str, String)> {
    let ed = LangEditor::open(dir);
    let mut out = vec![("carpeta", "lang/".to_string()), ("idiomas", ed.langs.join(", ")), ("base", ed.default.clone()), ("claves", ed.keys.len().to_string())];
    out.push(("sin traducir", ed.missing_count().to_string()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Project;
    use egui_kittest::{kittest::Queryable, Harness};

    fn project(name: &str) -> PathBuf {
        let dir = crate::room::tests::temp_project(name);
        std::fs::create_dir_all(dir.join("lang")).unwrap();
        std::fs::write(dir.join("lang/es.toml"), "[menu]\nplay = \"Jugar\"\nquit = \"Salir\"\n[coins]\none = \"{n} moneda\"\nother = \"{n} monedas\"\n").unwrap();
        std::fs::write(dir.join("lang/en.toml"), "[menu]\nplay = \"Play\"\n").unwrap();
        let g = std::fs::read_to_string(dir.join("game.toml")).unwrap();
        std::fs::write(dir.join("game.toml"), format!("{g}\n[localization]\ndefault = \"es\"\nfallback = \"en\"\n")).unwrap();
        dir
    }

    #[test]
    fn csv_roundtrip_and_validation() {
        let rows = parse_csv("key,es\n\"a,b\",\"di \"\"hola\"\"\nlinea 2\"\nc,d\n");
        assert_eq!(rows, vec![vec!["key".to_string(), "es".into()], vec!["a,b".into(), "di \"hola\"\nlinea 2".into()], vec!["c".into(), "d".into()]]);
        let dir = project("loced_csv");
        let mut ed = LangEditor::open(&dir);
        assert_eq!(ed.langs, vec!["es", "en"], "the base language first");
        assert_eq!(ed.missing_count(), 2, "en lacks menu.quit and coins");
        let csv = ed.to_csv();
        let mut other = LangEditor::open(&dir);
        other.keys.clear();
        other.cells.clear();
        assert_eq!(other.import_csv(&csv).unwrap(), 4, "es: 3 cells, en: 1");
        assert_eq!(other.tables(), ed.tables(), "CSV keeps every cell, plurals included");
        assert_eq!(ed.import_csv("key,en,fr\nmenu.quit,Quit,Quitter\n").unwrap(), 2);
        assert!(ed.langs.contains(&"fr".to_string()) && !ed.missing("en", "menu.quit"));
        assert!(ed.import_csv("nombre,es\n").is_err());
        assert!(ed.add_key("menu").is_err(), "clashes with the menu group");
        assert!(ed.add_key("bad key").is_err());
        assert!(ed.add_lang("pseudo").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn edit_cell_save_and_reload_in_the_engine() {
        let dir = project("loced_ui");
        let p = Project::open(&dir).unwrap();
        struct St {
            ed: LangEditor,
            p: Project,
            tex: crate::tex::TexCache,
        }
        let mut h = Harness::builder().with_size([1000.0, 500.0]).allow_missing_glyphs().with_accessibility_check(false).build_ui_state(
            |ui, s: &mut St| {
                let (mut log, mut open) = (vec![], vec![]);
                let mut cx = Cx { project: &mut s.p, tex: &mut s.tex, log: &mut log, open: &mut open, active: true };
                s.ed.ui(ui, &mut cx);
            },
            St { ed: LangEditor::open(&dir), p, tex: Default::default() },
        );
        h.run();
        h.get_by_label_contains("2 huecos sin traducir");
        // type the missing English text of menu.quit into its (empty, highlighted) cell
        let quit_row = h.state().ed.keys.iter().position(|k| k == "menu.quit").unwrap();
        assert_eq!(quit_row, 2);
        assert!(h.query_all_by_value("").count() >= 2, "missing cells are empty inputs");
        h.query_all_by_value("Salir").next().unwrap().focus();
        h.run();
        h.query_all_by_value("Salir").next().unwrap().type_text("!");
        h.run();
        let typed = h.state().ed.cells[&("es".to_string(), "menu.quit".to_string())].text.clone();
        assert!(typed == "Salir!" || typed == "!Salir", "typed into the cell: {typed}");
        assert!(h.state().ed.dirty);
        // add a key with the toolbar and fill it
        h.state_mut().ed.new_key = "menu.options".into();
        h.run();
        h.get_by_label("＋ Clave").click();
        h.run();
        assert!(h.state().ed.keys.contains(&"menu.options".to_string()));
        h.state_mut().ed.set("en", "menu.quit", "Quit");
        h.state_mut().ed.set("es", "menu.options", "Opciones");
        h.run();
        h.get_by_label_contains("2 huecos sin traducir");
        let s = h.state_mut();
        let (mut log, mut open) = (vec![], vec![]);
        let mut cx = Cx { project: &mut s.p, tex: &mut s.tex, log: &mut log, open: &mut open, active: true };
        s.ed.save(&mut cx).unwrap();
        assert!(!s.ed.dirty);
        // reload: the editor and the engine see the new texts, plurals intact
        let again = LangEditor::open(&dir);
        assert_eq!(again.cells[&("es".to_string(), "menu.quit".to_string())].text, typed);
        assert!(again.cells[&("es".to_string(), "coins".to_string())].plural);
        let mut e = modengine::Engine::load(&dir).unwrap();
        e.start();
        e.run_code("global.a = loc(\"menu.quit\")\nloc_set_language(\"en\")\nglobal.b = loc(\"menu.quit\")\nglobal.c = loc(\"coins\", { n: 2 })").unwrap();
        let gl = |n: &str| e.globals.get(&modengine::modscript::intern(n)).unwrap().to_string();
        assert_eq!(gl("a"), typed);
        assert_eq!(gl("b"), "Quit");
        assert_eq!(gl("c"), "2 monedas", "en has no plural: falls back to es");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
