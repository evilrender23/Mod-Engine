//! Data tables (`data/<table>.toml|json|csv`) in the editor: a grid of records × fields
//! with typed cells (numbers, text, checkboxes, enum and reference pickers, inline TOML
//! for lists and structs), validation markers from the engine's own checker, search,
//! and add / duplicate / delete / reorder rows.
//!
//! TOML files are saved with `toml_edit`, so comments, formatting and untouched values
//! stay as they were; JSON and CSV files are written again from the rows.

use crate::app::{Cx, Open};
use crate::project::Project;
use eframe::egui::{self, Color32, RichText};
use modengine::datatable::{self as dt, Diag, Format, RawRecord, RawTable, Schema, Ty};
use modscript::AssetKind;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// Index of the record in the file as last loaded (keeps its comments when saving).
    origin: Option<usize>,
    pub id: String,
    /// Fields other than `id`, in file order.
    pub cells: Vec<(String, toml::Value)>,
}

impl Row {
    pub fn get(&self, k: &str) -> Option<&toml::Value> {
        self.cells.iter().find(|c| c.0 == k).map(|c| &c.1)
    }
    pub fn set(&mut self, k: &str, v: toml::Value) {
        match self.cells.iter_mut().find(|c| c.0 == k) {
            Some(c) => c.1 = v,
            None => self.cells.push((k.to_string(), v)),
        }
    }
    fn unset(&mut self, k: &str) {
        self.cells.retain(|c| c.0 != k);
    }
}

#[derive(Clone, Debug)]
struct Col {
    name: String,
    ty: Ty,
    /// Declared in the schema (not guessed from the values).
    declared: bool,
    required: bool,
    default: Option<toml::Value>,
    doc: String,
}

pub struct TableEditor {
    pub name: String,
    path: PathBuf,
    format: Format,
    /// The file as last loaded or saved (TOML saves edit it in place).
    text: String,
    record_key: String,
    schema: Option<Schema>,
    schema_error: Option<String>,
    cols: Vec<Col>,
    pub rows: Vec<Row>,
    saved: Vec<Row>,
    pub dirty: bool,
    search: String,
    diags: Vec<Diag>,
    /// The file could not be read: shown instead of the grid.
    load_error: Option<String>,
    /// Text of the TOML cells being typed in: (row, field) → text.
    drafts: HashMap<(usize, String), String>,
    /// Ids of the other tables (reference pickers and validation).
    others: Vec<RawTable>,
    new_col: String,
}

/// Creates `data/<name>.toml` with an example record.
pub fn create(p: &mut Project, name: &str) -> Result<Open, String> {
    if p.data.iter().any(|d| d.name == name) {
        return Err(format!("ya hay una tabla '{name}'"));
    }
    let path = p.dir.join("data").join(format!("{name}.toml"));
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let key = dt::default_record_key(name);
    let text = format!(
        "# Tabla de datos '{name}': cada [[{key}]] es un registro con un id único.\n# En un script: data_get(\"{name}\", \"ejemplo\").nombre\n\n[[{key}]]\nid = \"ejemplo\"\nnombre = \"Ejemplo\"\nvalor = 1\n"
    );
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    p.reload_lists();
    Ok(Open::Data(name.to_string()))
}

/// Moves the table (and its schema) to `.trash`.
pub fn delete(p: &mut Project, name: &str) -> Result<(), String> {
    if let Some(d) = p.data.iter().find(|d| d.name == name) {
        let path = d.path.clone();
        p.trash(&path)?;
        let schema = dt::schema_path(&p.dir, name);
        if schema.exists() {
            p.trash(&schema)?;
        }
    }
    p.reload_lists();
    Ok(())
}

pub fn rows(p: &Project) -> Vec<crate::kinds::Row> {
    p.data.iter().map(|d| crate::kinds::Row::plain(&d.name, Open::Data(d.name.clone()))).collect()
}

/// Inspector lines of a table.
pub fn inspector_rows(p: &Project, name: &str) -> Vec<(&'static str, String)> {
    let mut out = vec![];
    if let Some(d) = p.data.iter().find(|d| d.name == name) {
        out.push(("fichero", d.path.strip_prefix(&p.dir).unwrap_or(&d.path).display().to_string()));
        if let Ok(ed) = TableEditor::open(p, name, &d.path) {
            out.push(("registros", ed.rows.len().to_string()));
            out.push(("campos", ed.cols.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", ")));
            out.push(("esquema", if ed.schema.is_some() { format!("data/{name}.schema.toml") } else { "no (tipos deducidos)".into() }));
            if !ed.diags.is_empty() {
                out.push(("problemas", ed.diags.len().to_string()));
            }
        }
    }
    out
}

fn format_of(path: &Path) -> Format {
    match path.extension().map(|e| e.to_string_lossy().to_lowercase()).as_deref() {
        Some("json") => Format::Json,
        Some("csv") => Format::Csv,
        _ => Format::Toml,
    }
}

/// A type guessed from a value (tables without a schema).
fn guess(v: &toml::Value) -> Ty {
    match v {
        toml::Value::String(_) => Ty::Str,
        toml::Value::Integer(_) => Ty::Int,
        toml::Value::Float(_) => Ty::Num,
        toml::Value::Boolean(_) => Ty::Bool,
        _ => Ty::Any,
    }
}

/// The value a new cell of this type starts with.
fn zero(ty: &Ty, default: Option<&toml::Value>) -> toml::Value {
    if let Some(d) = default {
        return d.clone();
    }
    match ty {
        Ty::Int => toml::Value::Integer(0),
        Ty::Num => toml::Value::Float(0.0),
        Ty::Bool => toml::Value::Boolean(false),
        Ty::Enum(v) => toml::Value::String(v.first().cloned().unwrap_or_default()),
        Ty::List(_) => toml::Value::Array(vec![]),
        Ty::Struct(_) => toml::Value::Table(Default::default()),
        _ => toml::Value::String(String::new()),
    }
}

/// Names of the assets of a kind (reference pickers).
fn asset_names(p: &Project, k: Option<AssetKind>) -> Vec<String> {
    use crate::fx::FxKind;
    use crate::motion::AnimKind;
    let Some(k) = k else {
        let mut all = vec![];
        for (_, k) in dt::ASSET_TYPES {
            all.extend(asset_names(p, Some(k)));
        }
        all.sort();
        return all;
    };
    match k {
        AssetKind::Sprite => p.sprite_names(),
        AssetKind::Sound => p.sounds.iter().map(|s| s.name.clone()).collect(),
        AssetKind::Object => p.object_names(),
        AssetKind::Room => p.room_names(),
        AssetKind::Font => p.fonts.iter().map(|s| s.name.clone()).collect(),
        AssetKind::TileSet => p.tilesets.iter().map(|s| s.name.clone()).collect(),
        AssetKind::Path => p.motion.names(AnimKind::Path),
        AssetKind::Timeline => p.motion.names(AnimKind::Timeline),
        AssetKind::AnimCurve => p.motion.names(AnimKind::Curve),
        AssetKind::Sequence => p.motion.names(AnimKind::Sequence),
        AssetKind::ParticleSystem => p.fx.names(FxKind::Particles),
        AssetKind::Shader => p.fx.names(FxKind::Shader),
    }
}

impl TableEditor {
    pub fn open(p: &Project, name: &str, path: &Path) -> Result<TableEditor, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let format = format_of(path);
        let file = path.strip_prefix(&p.dir).unwrap_or(path).to_string_lossy().replace('\\', "/");
        let sp = dt::schema_path(&p.dir, name);
        let (schema, schema_error) = match std::fs::read_to_string(&sp) {
            Ok(st) => match dt::parse_schema(&format!("data/{name}.schema.toml"), &st) {
                Ok(s) => (Some(s), None),
                Err(d) => (None, Some(d.iter().map(|d| d.to_string()).collect::<Vec<_>>().join("\n"))),
            },
            Err(_) => (None, None),
        };
        let mut ed = TableEditor {
            name: name.to_string(),
            path: path.to_path_buf(),
            format,
            text: text.clone(),
            record_key: dt::default_record_key(name),
            schema,
            schema_error,
            cols: vec![],
            rows: vec![],
            saved: vec![],
            dirty: false,
            search: String::new(),
            diags: vec![],
            load_error: None,
            drafts: HashMap::new(),
            others: vec![],
            new_col: String::new(),
        };
        match dt::parse_table(name, &file, &text, format, ed.schema.as_ref()) {
            Ok(t) => {
                ed.record_key = t.record_key.clone();
                ed.rows = t.records.iter().enumerate().map(|(i, r)| Row { origin: Some(i), id: r.id.clone(), cells: r.fields.iter().map(|(k, v, _)| (k.clone(), v.clone())).collect() }).collect();
            }
            Err(d) => ed.load_error = Some(d.iter().map(|d| d.to_string()).collect::<Vec<_>>().join("\n")),
        }
        ed.saved = ed.rows.clone();
        ed.others = dt::load_raw(&p.dir).0.into_iter().filter(|t| t.name != name).collect();
        ed.rebuild_cols();
        ed.validate(p);
        Ok(ed)
    }

    fn rebuild_cols(&mut self) {
        let mut cols: Vec<Col> = match &self.schema {
            Some(s) => s.root.fields.iter().map(|(n, f)| Col { name: n.clone(), ty: f.ty.clone(), declared: true, required: f.required, default: f.default.clone(), doc: f.doc.clone() }).collect(),
            None => vec![],
        };
        for r in &self.rows {
            for (k, v) in &r.cells {
                if !cols.iter().any(|c| &c.name == k) {
                    cols.push(Col { name: k.clone(), ty: guess(v), declared: false, required: false, default: None, doc: String::new() });
                }
            }
        }
        self.cols = cols;
    }

    /// The rows as the engine's raw table (unsaved state).
    fn raw(&self) -> RawTable {
        RawTable {
            name: self.name.clone(),
            file: format!("data/{}.{}", self.name, self.format.ext()),
            path: self.path.clone(),
            format: self.format,
            record_key: self.record_key.clone(),
            records: self.rows.iter().map(|r| RawRecord { id: r.id.clone(), line: 0, fields: r.cells.iter().map(|(k, v)| (k.clone(), v.clone(), 0)).collect() }).collect(),
            schema: self.schema.clone(),
        }
    }

    /// Runs the engine's checks on the unsaved rows.
    fn validate(&mut self, p: &Project) {
        let mut tables = vec![self.raw()];
        tables.extend(self.others.iter().cloned());
        let mut names: HashMap<Option<AssetKind>, Vec<String>> = HashMap::new();
        for (_, k) in dt::ASSET_TYPES {
            names.insert(Some(k), asset_names(p, Some(k)));
        }
        let check = |k: Option<AssetKind>, n: &str| match k {
            Some(_) => names.get(&k).is_some_and(|v| v.iter().any(|x| x == n)),
            None => names.values().any(|v| v.iter().any(|x| x == n)),
        };
        let file = tables[0].file.clone();
        let mut d: Vec<Diag> = dt::validate(&tables, &check).into_iter().filter(|d| d.file == file).collect();
        for (i, r) in self.rows.iter().enumerate() {
            if r.id.trim().is_empty() {
                d.push(Diag { file: file.clone(), line: 0, msg: "registro sin id".into(), record: Some(i), field: Some("id".into()) });
            }
        }
        self.diags = d;
    }

    fn cell_diag(&self, row: usize, field: &str) -> Option<String> {
        let msgs: Vec<&str> = self.diags.iter().filter(|d| d.record == Some(row) && d.field.as_deref() == Some(field)).map(|d| d.msg.as_str()).collect();
        (!msgs.is_empty()).then(|| msgs.join("\n"))
    }

    fn changed(&mut self, p: &Project) {
        self.dirty = self.rows != self.saved;
        self.rebuild_cols();
        self.validate(p);
    }

    // ------------------------------------------------------------ row operations

    fn unique_id(&self, base: &str) -> String {
        let base = if base.is_empty() { "nuevo" } else { base };
        if !self.rows.iter().any(|r| r.id == base) {
            return base.to_string();
        }
        (2..).map(|n| format!("{base}_{n}")).find(|c| !self.rows.iter().any(|r| &r.id == c)).unwrap()
    }

    pub fn add_row(&mut self, p: &Project) {
        let id = self.unique_id("nuevo");
        let cells = self.cols.iter().filter(|c| c.required || c.default.is_some()).map(|c| (c.name.clone(), zero(&c.ty, c.default.as_ref()))).collect();
        self.rows.push(Row { origin: None, id, cells });
        self.drafts.clear();
        self.changed(p);
    }

    pub fn duplicate_row(&mut self, i: usize, p: &Project) {
        let mut r = self.rows[i].clone();
        r.origin = None;
        r.id = self.unique_id(&format!("{}_copia", r.id));
        self.rows.insert(i + 1, r);
        self.drafts.clear();
        self.changed(p);
    }

    pub fn delete_row(&mut self, i: usize, p: &Project) {
        self.rows.remove(i);
        self.drafts.clear();
        self.changed(p);
    }

    pub fn move_row(&mut self, i: usize, up: bool, p: &Project) {
        let j = if up { i.wrapping_sub(1) } else { i + 1 };
        if j < self.rows.len() {
            self.rows.swap(i, j);
            self.drafts.clear();
            self.changed(p);
        }
    }

    // ------------------------------------------------------------ saving

    pub fn save(&mut self, cx: &mut Cx) -> Result<(), String> {
        self.rebuild_cols();
        let text = match self.format {
            Format::Toml => self.to_toml()?,
            Format::Json => self.to_json(),
            Format::Csv => self.to_csv(),
        };
        std::fs::write(&self.path, &text).map_err(|e| format!("{}: {e}", self.path.display()))?;
        let fresh = TableEditor::open(cx.project, &self.name, &self.path)?;
        let search = std::mem::take(&mut self.search);
        *self = fresh;
        self.search = search;
        Ok(())
    }

    /// TOML with the original comments and formatting of untouched records and values.
    pub fn to_toml(&self) -> Result<String, String> {
        use toml_edit::{ArrayOfTables, DocumentMut, Item, Table};
        let mut doc: DocumentMut = self.text.parse().unwrap_or_default();
        let old: Vec<Table> = doc.get(&self.record_key).and_then(|i| i.as_array_of_tables()).map(|a| a.iter().cloned().collect()).unwrap_or_default();
        let first_pos = old.iter().filter_map(|t| t.position()).min().unwrap_or(1);
        let mut out = ArrayOfTables::new();
        for (n, r) in self.rows.iter().enumerate() {
            let mut t = r.origin.and_then(|i| old.get(i).cloned()).unwrap_or_else(|| {
                let mut t = Table::new();
                if n > 0 || !old.is_empty() {
                    t.decor_mut().set_prefix("\n");
                }
                t
            });
            set_value(&mut t, "id", &toml::Value::String(r.id.clone()))?;
            for (k, v) in &r.cells {
                set_value(&mut t, k, v)?;
            }
            let keep: Vec<String> = t.iter().map(|(k, _)| k.to_string()).filter(|k| k != "id" && r.get(k).is_none()).collect();
            for k in keep {
                t.remove(&k);
            }
            t.set_position(first_pos + n);
            out.push(t);
        }
        doc.insert(&self.record_key, Item::ArrayOfTables(out));
        Ok(doc.to_string())
    }

    fn to_json(&self) -> String {
        let rec = |r: &Row| {
            let mut parts = vec![format!("\"id\": {}", serde_json::Value::String(r.id.clone()))];
            for (k, v) in &r.cells {
                parts.push(format!("{}: {}", serde_json::Value::String(k.clone()), serde_json::to_string(&toml_to_json(v)).unwrap_or_default()));
            }
            format!("  {{{}}}", parts.join(", "))
        };
        let body = self.rows.iter().map(rec).collect::<Vec<_>>().join(",\n");
        let wrapped = serde_json::from_str::<serde_json::Value>(&self.text).ok().is_some_and(|j| j.is_object());
        if wrapped {
            format!("{{\"{}\": [\n{body}\n]}}\n", self.record_key)
        } else {
            format!("[\n{body}\n]\n")
        }
    }

    fn to_csv(&self) -> String {
        let q = |s: String| if s.contains([',', '"', '\n']) || s.starts_with(' ') { format!("\"{}\"", s.replace('"', "\"\"")) } else { s };
        let mut out = String::new();
        let names: Vec<&str> = std::iter::once("id").chain(self.cols.iter().map(|c| c.name.as_str())).collect();
        out += &names.iter().map(|n| q(n.to_string())).collect::<Vec<_>>().join(",");
        out.push('\n');
        for r in &self.rows {
            let mut cells = vec![q(r.id.clone())];
            for c in &self.cols {
                cells.push(match r.get(&c.name) {
                    None => String::new(),
                    Some(toml::Value::String(s)) => q(s.clone()),
                    Some(v) => q(dt::inline_toml(v)),
                });
            }
            out += &cells.join(",");
            out.push('\n');
        }
        out
    }

    /// Writes `data/<table>.schema.toml` from the current columns (their guessed types).
    pub fn write_schema(&mut self, p: &Project) -> Result<(), String> {
        let path = dt::schema_path(&p.dir, &self.name);
        if path.exists() {
            return Err(format!("{} ya existe", path.display()));
        }
        let mut s = format!("# Esquema de data/{}: tipo de cada campo. Tipos: string, int, number, bool, enum, ref, list, struct,\n# sprite, sound, object... Opciones: required, default, min, max, values, table, of, doc.\n\n[fields]\n", self.name);
        for c in &self.cols {
            let ty = match &c.ty {
                Ty::Any => "\"any\"".to_string(),
                t => format!("\"{}\"", t.label()),
            };
            s += &format!("{} = {{ type = {ty} }}\n", dt::toml_key(&c.name));
        }
        std::fs::write(&path, s).map_err(|e| e.to_string())?;
        let fresh = TableEditor::open(p, &self.name, &self.path)?;
        self.schema = fresh.schema;
        self.schema_error = fresh.schema_error;
        self.changed(p);
        Ok(())
    }

    // ------------------------------------------------------------ UI

    pub fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        if let Some(e) = &self.load_error {
            ui.colored_label(Color32::from_rgb(230, 90, 80), format!("No se puede editar la tabla: {e}"));
            ui.label(RichText::new("Corrige el fichero en un editor de texto y vuelve a abrirla.").weak());
            return;
        }
        let mut action: Option<(usize, &'static str)> = None;
        ui.horizontal(|ui| {
            if ui.button("+ Fila").on_hover_text("Añade un registro al final").clicked() {
                action = Some((usize::MAX, "add"));
            }
            ui.separator();
            ui.label("🔍");
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("buscar en la tabla").desired_width(160.0));
            ui.separator();
            ui.add(egui::TextEdit::singleline(&mut self.new_col).hint_text("campo nuevo").desired_width(100.0));
            if ui.add_enabled(!self.new_col.trim().is_empty(), egui::Button::new("+ Columna")).clicked() {
                action = Some((usize::MAX, "col"));
            }
            ui.separator();
            let info = format!("{} registros · {} · [[{}]]", self.rows.len(), self.format.ext().to_uppercase(), self.record_key);
            ui.label(RichText::new(info).weak());
            if self.schema.is_none() && self.schema_error.is_none() && ui.button("Crear esquema").on_hover_text("Escribe data/<tabla>.schema.toml con los tipos deducidos de las columnas").clicked() {
                action = Some((usize::MAX, "schema"));
            }
        });
        if let Some(e) = &self.schema_error {
            ui.colored_label(Color32::from_rgb(230, 90, 80), format!("Esquema con errores (se ignora): {e}"));
        }
        ui.separator();
        let bottom = if self.diags.is_empty() { 0.0 } else { 70.0 };
        let h = (ui.available_height() - bottom).max(80.0);
        let needle = self.search.to_lowercase();
        let visible: Vec<usize> = (0..self.rows.len())
            .filter(|&i| {
                needle.is_empty() || {
                    let r = &self.rows[i];
                    r.id.to_lowercase().contains(&needle) || r.cells.iter().any(|(_, v)| dt::inline_toml(v).to_lowercase().contains(&needle))
                }
            })
            .collect();
        let mut edits: Vec<(usize, String, Option<toml::Value>)> = vec![];
        let mut id_edits: Vec<(usize, String)> = vec![];
        egui::ScrollArea::both().id_salt(("datatable", &self.name)).max_height(h).auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new(("datatable_grid", &self.name)).striped(true).spacing([8.0, 4.0]).show(ui, |ui| {
                ui.label("");
                ui.label(RichText::new("id").strong()).on_hover_text("texto único: data_get(tabla, id)");
                for c in &self.cols {
                    let mut t = RichText::new(&c.name).strong();
                    if !c.declared && self.schema.is_some() {
                        t = t.color(Color32::from_rgb(230, 160, 60));
                    }
                    let mut tip = format!("tipo: {}", c.ty.label());
                    if c.required {
                        tip += " · obligatorio";
                    }
                    if let Some(d) = &c.default {
                        tip += &format!(" · por defecto {}", dt::inline_toml(d));
                    }
                    if !c.doc.is_empty() {
                        tip += &format!("\n{}", c.doc);
                    }
                    if !c.declared {
                        tip += if self.schema.is_some() { "\nno está en el esquema" } else { "\n(deducido de los valores)" };
                    }
                    ui.label(t).on_hover_text(tip);
                }
                ui.end_row();
                for &i in &visible {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        if ui.small_button("⬆").on_hover_text("Subir").clicked() {
                            action = Some((i, "up"));
                        }
                        if ui.small_button("⬇").on_hover_text("Bajar").clicked() {
                            action = Some((i, "down"));
                        }
                        if ui.small_button("⧉").on_hover_text("Duplicar").clicked() {
                            action = Some((i, "dup"));
                        }
                        if ui.small_button("🗑").on_hover_text("Borrar").clicked() {
                            action = Some((i, "del"));
                        }
                    });
                    let mut id = self.rows[i].id.clone();
                    let bad = self.cell_diag(i, "id");
                    let r = ui.add_sized([110.0, 20.0], egui::TextEdit::singleline(&mut id).text_color_opt(bad.as_ref().map(|_| Color32::from_rgb(240, 110, 100))));
                    if let Some(m) = &bad {
                        r.clone().on_hover_text(m);
                    }
                    if id != self.rows[i].id {
                        id_edits.push((i, id));
                    }
                    for c in self.cols.clone() {
                        let cur = self.rows[i].get(&c.name).cloned();
                        let bad = self.cell_diag(i, &c.name);
                        let out = self.cell(ui, cx.project, i, &c, cur.as_ref(), bad.as_deref());
                        if let Some(v) = out {
                            edits.push((i, c.name.clone(), v));
                        }
                    }
                    ui.end_row();
                }
            });
        });
        if !self.diags.is_empty() {
            ui.separator();
            egui::ScrollArea::vertical().id_salt(("datatable_diags", &self.name)).max_height(bottom - 10.0).show(ui, |ui| {
                for d in &self.diags {
                    ui.colored_label(Color32::from_rgb(230, 110, 90), format!("⚠ {}", d.msg));
                }
            });
        }
        let mut any = false;
        for (i, id) in id_edits {
            self.rows[i].id = id;
            any = true;
        }
        for (i, k, v) in edits {
            match v {
                Some(v) => self.rows[i].set(&k, v),
                None => self.rows[i].unset(&k),
            }
            any = true;
        }
        if any {
            self.changed(cx.project);
        }
        match action {
            Some((_, "add")) => self.add_row(cx.project),
            Some((_, "col")) => {
                let name = self.new_col.trim().to_string();
                if !self.cols.iter().any(|c| c.name == name) && name != "id" {
                    if let Some(r) = self.rows.first_mut() {
                        r.set(&name, toml::Value::String(String::new()));
                    }
                    self.new_col.clear();
                    self.changed(cx.project);
                }
            }
            Some((_, "schema")) => {
                if let Err(e) = self.write_schema(cx.project) {
                    cx.error(e);
                }
            }
            Some((i, "up")) => self.move_row(i, true, cx.project),
            Some((i, "down")) => self.move_row(i, false, cx.project),
            Some((i, "dup")) => self.duplicate_row(i, cx.project),
            Some((i, "del")) => self.delete_row(i, cx.project),
            _ => {}
        }
    }

    /// One cell. Returns `Some(new value)` when edited (`Some(None)`: emptied).
    fn cell(&mut self, ui: &mut egui::Ui, p: &Project, row: usize, c: &Col, cur: Option<&toml::Value>, bad: Option<&str>) -> Option<Option<toml::Value>> {
        let red = Color32::from_rgb(240, 110, 100);
        let mut out = None;
        let Some(v) = cur else {
            let b = egui::Button::new(RichText::new("—").weak()).small();
            let r = ui.add(b).on_hover_text(match bad {
                Some(m) => format!("{m}\nclic: rellenar"),
                None => "vacío: clic para darle un valor".into(),
            });
            if bad.is_some() {
                ui.painter().rect_stroke(r.rect, 2.0, egui::Stroke::new(1.0, red), egui::StrokeKind::Outside);
            }
            if r.clicked() {
                out = Some(Some(zero(&c.ty, c.default.as_ref())));
            }
            return out;
        };
        let resp = match (&c.ty, v) {
            (Ty::Int, toml::Value::Integer(n)) => {
                let mut n = *n;
                let r = ui.add(egui::DragValue::new(&mut n).speed(0.2));
                if r.changed() {
                    out = Some(Some(toml::Value::Integer(n)));
                }
                r
            }
            (Ty::Num, toml::Value::Float(_) | toml::Value::Integer(_)) => {
                let mut x = v.as_float().unwrap_or_else(|| v.as_integer().unwrap_or(0) as f64);
                let r = ui.add(egui::DragValue::new(&mut x).speed(0.05).max_decimals(4));
                if r.changed() {
                    out = Some(Some(toml::Value::Float(x)));
                }
                r
            }
            (Ty::Bool, toml::Value::Boolean(b)) => {
                let mut b = *b;
                let r = ui.checkbox(&mut b, "");
                if r.changed() {
                    out = Some(Some(toml::Value::Boolean(b)));
                }
                r
            }
            (Ty::Enum(vals), toml::Value::String(s)) => {
                let mut sel = s.clone();
                let r = egui::ComboBox::from_id_salt(("dt_enum", row, &c.name)).selected_text(&sel).width(100.0).show_ui(ui, |ui| {
                    for x in vals {
                        ui.selectable_value(&mut sel, x.clone(), x);
                    }
                });
                if &sel != s {
                    out = Some(Some(toml::Value::String(sel)));
                }
                r.response
            }
            (Ty::Ref(_) | Ty::Asset(_), toml::Value::String(s)) => {
                let options: Vec<String> = match &c.ty {
                    Ty::Ref(t) if t == &self.name => self.rows.iter().map(|r| r.id.clone()).collect(),
                    Ty::Ref(t) => self.others.iter().find(|x| &x.name == t).map(|x| x.records.iter().map(|r| r.id.clone()).collect()).unwrap_or_default(),
                    Ty::Asset(k) => asset_names(p, *k),
                    _ => vec![],
                };
                let mut sel = s.clone();
                let r = egui::ComboBox::from_id_salt(("dt_ref", row, &c.name)).selected_text(if sel.is_empty() { "(elegir)" } else { sel.as_str() }).width(110.0).show_ui(ui, |ui| {
                    for x in &options {
                        ui.selectable_value(&mut sel, x.clone(), x);
                    }
                });
                if &sel != s {
                    out = Some(Some(toml::Value::String(sel)));
                }
                r.response
            }
            (Ty::Str | Ty::Any, toml::Value::String(s)) => {
                let mut t = s.clone();
                let r = ui.add_sized([130.0, 20.0], egui::TextEdit::singleline(&mut t));
                if r.changed() {
                    out = Some(Some(toml::Value::String(t)));
                }
                r
            }
            _ => {
                // lists, structs and values of the wrong type: inline TOML text
                let key = (row, c.name.clone());
                let mut t = self.drafts.get(&key).cloned().unwrap_or_else(|| dt::inline_toml(v));
                let parsed = dt::parse_inline(&t);
                let w = (dt::inline_toml(v).chars().count() as f32 * 7.5 + 16.0).clamp(120.0, 420.0);
                let r = ui.add_sized([w, 20.0], egui::TextEdit::singleline(&mut t).font(egui::TextStyle::Monospace).text_color_opt(parsed.is_err().then_some(red)));
                if r.changed() {
                    match dt::parse_inline(&t) {
                        Ok(nv) => {
                            self.drafts.remove(&key);
                            if &nv != v {
                                out = Some(Some(nv));
                            }
                        }
                        Err(_) => {
                            self.drafts.insert(key, t);
                        }
                    }
                }
                if let Err(e) = parsed {
                    r.clone().on_hover_text(format!("TOML no válido: {e}"));
                }
                r
            }
        };
        if let Some(m) = bad {
            ui.painter().rect_stroke(resp.rect, 2.0, egui::Stroke::new(1.0, red), egui::StrokeKind::Outside);
            resp.clone().on_hover_text(m);
        }
        resp.context_menu(|ui| {
            if ui.button("Vaciar (quitar el campo de este registro)").clicked() {
                out = Some(None);
                ui.close();
            }
            if ui.button("Editar como TOML").clicked() {
                self.drafts.insert((row, c.name.clone()), dt::inline_toml(v));
                ui.close();
            }
        });
        out
    }
}

/// Puts `v` in `t[k]`, keeping the key's formatting and comments when the value did not change.
fn set_value(t: &mut toml_edit::Table, k: &str, v: &toml::Value) -> Result<(), String> {
    let new: toml_edit::Value = dt::inline_toml(v).parse().map_err(|e| format!("{k}: {e}"))?;
    match t.get_mut(k) {
        Some(item) => {
            if let Some(old) = item.as_value() {
                if dt::parse_inline(&old.to_string()).ok().as_ref() == Some(v) {
                    return Ok(());
                }
                let decor = old.decor().clone();
                let mut new = new;
                *new.decor_mut() = decor;
                *item = toml_edit::Item::Value(new);
            } else {
                *item = toml_edit::value(new);
            }
        }
        None => {
            t.insert(k, toml_edit::value(new));
        }
    }
    Ok(())
}

fn toml_to_json(v: &toml::Value) -> serde_json::Value {
    match v {
        toml::Value::String(s) => serde_json::Value::String(s.clone()),
        toml::Value::Integer(i) => serde_json::Value::from(*i),
        toml::Value::Float(f) => serde_json::Number::from_f64(*f).map(serde_json::Value::Number).unwrap_or(serde_json::Value::Null),
        toml::Value::Boolean(b) => serde_json::Value::Bool(*b),
        toml::Value::Datetime(d) => serde_json::Value::String(d.to_string()),
        toml::Value::Array(a) => serde_json::Value::Array(a.iter().map(toml_to_json).collect()),
        toml::Value::Table(t) => serde_json::Value::Object(t.iter().map(|(k, v)| (k.clone(), toml_to_json(v))).collect()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tex::TexCache;
    use egui_kittest::{kittest::Queryable, Harness};

    const CARDS: &str = "# Cartas del mazo inicial\n\n[[card]]\nid = \"slash\"  # el ataque básico\ncost = 1\nkind = \"attack\"\neffects = [{ type = \"damage\", amount = 6 }]\n\n# defensa\n[[card]]\nid = \"guard\"\ncost = 1\nkind = \"skill\"\neffects = [{ type = \"block\", amount = 5 }]\n";
    const SCHEMA: &str = "[fields]\ncost = { type = \"int\", default = 1, min = 0 }\nkind = { type = \"enum\", values = [\"attack\", \"skill\"] }\nsprite = { type = \"sprite\" }\neffects = { type = \"list\", of = \"struct\" }\n";

    struct St {
        ed: TableEditor,
        project: Project,
        tex: TexCache,
        log: Vec<crate::app::LogLine>,
        open: Vec<Open>,
    }

    fn setup(tag: &str, files: &[(&str, &str)]) -> (PathBuf, Project) {
        let dir = crate::room::tests::temp_project(tag);
        for (n, t) in files {
            let p = dir.join(n);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        }
        let p = Project::open(&dir).unwrap();
        (dir, p)
    }

    #[test]
    fn grid_add_edit_save_reload_keeps_comments() {
        let (dir, project) = setup("datatable", &[("data/cards.toml", CARDS), ("data/cards.schema.toml", SCHEMA)]);
        assert_eq!(project.data.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), vec!["cards"]);
        let path = project.data[0].path.clone();
        let ed = TableEditor::open(&project, "cards", &path).unwrap();
        assert_eq!(ed.rows.len(), 2);
        assert!(ed.diags.is_empty(), "{:?}", ed.diags);
        let st = St { ed, project, tex: TexCache::default(), log: vec![], open: vec![] };
        let mut h = Harness::builder().with_size([1200.0, 600.0]).allow_missing_glyphs().with_accessibility_check(false).build_ui_state(
            |ui, s: &mut St| {
                let mut cx = Cx { project: &mut s.project, tex: &mut s.tex, log: &mut s.log, open: &mut s.open, active: true };
                s.ed.ui(ui, &mut cx);
            },
            st,
        );
        h.run_steps(3);
        h.get_by_label("+ Fila").click();
        h.run_steps(3);
        assert_eq!(h.state().ed.rows.len(), 3);
        assert_eq!(h.state().ed.rows[2].id, "nuevo");
        assert_eq!(h.state().ed.rows[2].get("cost"), Some(&toml::Value::Integer(1)), "schema default");
        assert!(h.state().ed.dirty);
        // edit cells (as the widgets do) and check validation markers
        {
            let s = h.state_mut();
            s.ed.rows[2].id = "fireball".into();
            s.ed.rows[2].set("kind", toml::Value::String("spell".into()));
            s.ed.rows[0].set("cost", toml::Value::Integer(2));
            s.ed.changed(&s.project);
        }
        h.run_steps(2);
        assert!(h.state().ed.cell_diag(2, "kind").is_some_and(|m| m.contains("'spell' no es un valor válido")), "{:?}", h.state().ed.diags);
        {
            let s = h.state_mut();
            s.ed.rows[2].set("kind", toml::Value::String("skill".into()));
            s.ed.changed(&s.project);
        }
        h.run_steps(2);
        assert!(h.state().ed.diags.is_empty(), "{:?}", h.state().ed.diags);
        // search hides rows; duplicate and reorder
        h.state_mut().ed.duplicate_row(0, &Project::open(&dir).unwrap());
        assert_eq!(h.state().ed.rows[1].id, "slash_copia");
        h.state_mut().ed.move_row(1, false, &Project::open(&dir).unwrap());
        assert_eq!(h.state().ed.rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec!["slash", "guard", "slash_copia", "fireball"]);
        h.state_mut().ed.delete_row(2, &Project::open(&dir).unwrap());
        let before = h.state().ed.rows.clone();
        {
            let s = h.state_mut();
            let mut cx = Cx { project: &mut s.project, tex: &mut s.tex, log: &mut s.log, open: &mut s.open, active: true };
            s.ed.save(&mut cx).unwrap();
        }
        assert!(!h.state().ed.dirty);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# Cartas del mazo inicial\n"), "{text}");
        assert!(text.contains("id = \"slash\"  # el ataque básico\ncost = 2\n"), "{text}");
        assert!(text.contains("# defensa\n[[card]]\nid = \"guard\""), "{text}");
        assert!(text.contains("[[card]]\nid = \"fireball\""), "{text}");
        // reloading gives the same rows, and the engine reads the file
        let again = TableEditor::open(&Project::open(&dir).unwrap(), "cards", &path).unwrap();
        let strip = |v: &[Row]| v.iter().map(|r| (r.id.clone(), { let mut c = r.cells.clone(); c.sort_by(|a, b| a.0.cmp(&b.0)); c })).collect::<Vec<_>>();
        assert_eq!(strip(&again.rows), strip(&before));
        let e = modengine::Engine::load(&dir).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(e.data.table("cards").unwrap().ids, vec!["slash", "guard", "fireball"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn json_and_csv_tables_round_trip() {
        let json = "[\n  {\"id\": \"bat\", \"hp\": 12, \"tags\": [\"fly\"]}\n]\n";
        let csv = "id,name,price\nring,\"Anillo, de oro\",50\n";
        let (dir, project) = setup("datatable_fmt", &[("data/enemies.json", json), ("data/relics.csv", csv)]);
        for name in ["enemies", "relics"] {
            let path = project.data.iter().find(|d| d.name == name).unwrap().path.clone();
            let mut ed = TableEditor::open(&project, name, &path).unwrap();
            ed.add_row(&project);
            ed.rows[1].id = "new_one".into();
            ed.rows[1].set("hp", toml::Value::Integer(3));
            let mut log = vec![];
            let mut p2 = Project::open(&dir).unwrap();
            let mut cx = Cx { project: &mut p2, tex: &mut TexCache::default(), log: &mut log, open: &mut vec![], active: true };
            ed.save(&mut cx).unwrap();
            assert_eq!(ed.rows.len(), 2, "{name}");
            assert_eq!(ed.rows[1].get("hp"), Some(&toml::Value::Integer(3)), "{name}");
        }
        let text = std::fs::read_to_string(dir.join("data/relics.csv")).unwrap();
        assert!(text.starts_with("id,name,price,hp\nring,\"Anillo, de oro\",50,\nnew_one,,,3\n"), "{text}");
        let e = modengine::Engine::load(&dir).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(e.data.table("enemies").unwrap().ids, vec!["bat", "new_one"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_writes_a_loadable_table_and_schema() {
        let (dir, mut project) = setup("datatable_new", &[]);
        let open = create(&mut project, "items").unwrap();
        assert_eq!(open, Open::Data("items".into()));
        assert!(create(&mut project, "items").is_err());
        let path = project.data[0].path.clone();
        let mut ed = TableEditor::open(&project, "items", &path).unwrap();
        ed.write_schema(&project).unwrap();
        let schema = std::fs::read_to_string(dir.join("data/items.schema.toml")).unwrap();
        assert!(schema.contains("nombre = { type = \"string\" }") && schema.contains("valor = { type = \"int\" }"), "{schema}");
        let e = modengine::Engine::load(&dir).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(e.data.table("items").unwrap().ids, vec!["ejemplo"]);
        delete(&mut project, "items").unwrap();
        assert!(project.data.is_empty() && !dir.join("data/items.schema.toml").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
