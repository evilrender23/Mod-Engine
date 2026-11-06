//! Data tables: game content as data (`data/<table>.toml|json|csv`), each a list of
//! records with a unique `id`, optionally described by `data/<table>.schema.toml`.
//!
//! ```toml
//! # data/cards.toml
//! [[card]]
//! id = "slash"
//! cost = 1
//! effects = [{ type = "damage", amount = 6, target = "front" }]
//! ```
//!
//! ```toml
//! # data/cards.schema.toml
//! [fields]
//! cost = { type = "int", default = 1, min = 0 }
//! kind = { type = "enum", values = ["attack", "skill"] }
//! sprite = { type = "sprite" }
//! upgrade = { type = "ref", table = "cards" }
//! effects = { type = "list", of = { type = "struct", tag = "type", variants = { damage = { amount = "int" } } } }
//! ```
//!
//! Loading is two passes: [`parse_table`] / [`parse_schema`] read the files (no game
//! needed, the editor uses them too), then [`validate`] checks types, enums, duplicate
//! ids and references to assets and to other tables. Every problem is a [`Diag`] with
//! `file:line`. Scripts read the records with `data_get(table, id)` (see
//! `natives/datatable.rs`); a run from a project folder reloads them when a file changes.

use crate::engine::Engine;
use modscript::{intern, AssetKind, Sym, Value};
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::{Path, PathBuf};
use toml::Spanned;

// ------------------------------------------------------------------ diagnostics

/// A problem in a data file. `record` / `field` locate it for the editor's markers.
#[derive(Clone, Debug, PartialEq)]
pub struct Diag {
    pub file: String,
    pub line: u32,
    pub msg: String,
    /// Index of the record in its table (None: a file or schema problem).
    pub record: Option<usize>,
    pub field: Option<String>,
}

impl fmt::Display for Diag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.file, self.line, self.msg)
    }
}

fn diag(file: &str, line: u32, msg: impl Into<String>) -> Diag {
    Diag { file: file.to_string(), line, msg: msg.into(), record: None, field: None }
}

/// 1-based line of a byte offset.
pub fn line_of(text: &str, offset: usize) -> u32 {
    text.as_bytes()[..offset.min(text.len())].iter().filter(|b| **b == b'\n').count() as u32 + 1
}

/// `file:line` out of a toml error (its message already shows the line).
fn toml_err(file: &str, text: &str, e: &toml::de::Error) -> Diag {
    let line = e.span().map(|s| line_of(text, s.start)).unwrap_or(1);
    let msg = e.message().trim().to_string();
    diag(file, line, format!("TOML no válido: {msg}"))
}

// ------------------------------------------------------------------ schema

/// The type of a field.
#[derive(Clone, Debug, PartialEq)]
pub enum Ty {
    Any,
    Str,
    Int,
    Num,
    Bool,
    Enum(Vec<String>),
    /// The id of a record of another table (or this one).
    Ref(String),
    /// The name of a game asset (None: any kind). Becomes the asset in scripts.
    Asset(Option<AssetKind>),
    List(Box<Field>),
    Struct(StructSpec),
}

/// Fields of a struct, optionally tagged: `tag = "type"` picks one of `variants` by the
/// value of that field (the composable-effects pattern).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct StructSpec {
    pub fields: Vec<(String, Field)>,
    pub tag: Option<String>,
    pub variants: Vec<(String, Vec<(String, Field)>)>,
    /// Fields not in the schema are allowed (`extra_fields = true`).
    pub extra: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub ty: Ty,
    pub required: bool,
    pub default: Option<toml::Value>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// `doc = "..."`: shown as a tooltip by the editor.
    pub doc: String,
    pub line: u32,
}

impl Field {
    fn of(ty: Ty) -> Field {
        Field { ty, required: false, default: None, min: None, max: None, doc: String::new(), line: 0 }
    }
}

impl Ty {
    /// Short name for messages and the editor's column headers.
    pub fn label(&self) -> String {
        match self {
            Ty::Any => "any".into(),
            Ty::Str => "string".into(),
            Ty::Int => "int".into(),
            Ty::Num => "number".into(),
            Ty::Bool => "bool".into(),
            Ty::Enum(_) => "enum".into(),
            Ty::Ref(t) => format!("ref {t}"),
            Ty::Asset(Some(k)) => k.name().into(),
            Ty::Asset(None) => "asset".into(),
            Ty::List(f) => format!("list<{}>", f.ty.label()),
            Ty::Struct(_) => "struct".into(),
        }
    }
}

/// `data/<table>.schema.toml`.
#[derive(Clone, Debug, PartialEq)]
pub struct Schema {
    pub file: String,
    pub root: StructSpec,
}

impl Schema {
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.root.fields.iter().find(|(n, _)| n == name).map(|(_, f)| f)
    }
}

pub const ASSET_TYPES: [(&str, AssetKind); 12] = [
    ("sprite", AssetKind::Sprite),
    ("sound", AssetKind::Sound),
    ("object", AssetKind::Object),
    ("room", AssetKind::Room),
    ("font", AssetKind::Font),
    ("tileset", AssetKind::TileSet),
    ("path", AssetKind::Path),
    ("timeline", AssetKind::Timeline),
    ("animcurve", AssetKind::AnimCurve),
    ("sequence", AssetKind::Sequence),
    ("particles", AssetKind::ParticleSystem),
    ("shader", AssetKind::Shader),
];

const TYPE_NAMES: &str = "string, int, number, bool, enum, ref, list, struct, any, asset, sprite, sound, object, room, font, tileset, path, timeline, animcurve, sequence, particles, shader";

/// Parses a schema file.
pub fn parse_schema(file: &str, text: &str) -> Result<Schema, Vec<Diag>> {
    let doc: BTreeMap<String, Spanned<toml::Value>> = toml::from_str(text).map_err(|e| vec![toml_err(file, text, &e)])?;
    let mut errs = vec![];
    let mut root = StructSpec::default();
    for (k, v) in &doc {
        let line = line_of(text, v.span().start);
        match k.as_str() {
            "fields" => {}
            "extra_fields" => match v.get_ref() {
                toml::Value::Boolean(b) => root.extra = *b,
                _ => errs.push(diag(file, line, "extra_fields debe ser true o false")),
            },
            "description" | "doc" => {}
            other => errs.push(diag(file, line, format!("clave desconocida '{other}' (usa [fields], extra_fields o description)"))),
        }
    }
    // the fields in the order they are written (spans), not alphabetical
    if let Some(f) = doc.get("fields") {
        #[derive(serde::Deserialize)]
        struct Doc {
            #[serde(default)]
            fields: BTreeMap<String, Spanned<toml::Value>>,
        }
        if !f.get_ref().is_table() {
            errs.push(diag(file, line_of(text, f.span().start), "[fields] debe ser una tabla: nombre = { type = \"...\" }"));
        } else if let Ok(d) = toml::from_str::<Doc>(text) {
            let mut list: Vec<(&String, &Spanned<toml::Value>)> = d.fields.iter().collect();
            list.sort_by_key(|(_, v)| v.span().start);
            for (name, v) in list {
                let line = line_of(text, v.span().start);
                if name == "id" {
                    continue; // always a string, always required
                }
                match field_spec(v.get_ref(), line) {
                    Ok(f) => root.fields.push((name.clone(), f)),
                    Err(m) => errs.push(diag(file, line, format!("campo '{name}': {m}"))),
                }
            }
        }
    }
    if errs.is_empty() {
        Ok(Schema { file: file.to_string(), root })
    } else {
        Err(errs)
    }
}

fn num_of(v: &toml::Value) -> Option<f64> {
    v.as_float().or_else(|| v.as_integer().map(|i| i as f64))
}

/// A field spec: `"int"` or `{ type = "int", default = 1, ... }`.
fn field_spec(v: &toml::Value, line: u32) -> Result<Field, String> {
    let t = match v {
        toml::Value::String(s) => {
            let mut f = Field::of(simple_type(s).ok_or_else(|| format!("tipo desconocido '{s}' (tipos: {TYPE_NAMES})"))?);
            f.line = line;
            return Ok(f);
        }
        toml::Value::Table(t) => t,
        _ => return Err("se esperaba un tipo (\"int\") o una tabla { type = \"int\", ... }".into()),
    };
    let ty_name = match t.get("type") {
        Some(toml::Value::String(s)) => s.as_str(),
        Some(_) => return Err("type debe ser un texto".into()),
        None => {
            if t.contains_key("fields") || t.contains_key("variants") {
                "struct"
            } else {
                return Err("falta type = \"...\"".into());
            }
        }
    };
    let ty = match ty_name {
        "enum" => match t.get("values") {
            Some(toml::Value::Array(a)) if !a.is_empty() => Ty::Enum(a.iter().map(|x| x.as_str().map(String::from).ok_or("los valores de un enum deben ser textos")).collect::<Result<_, _>>()?),
            _ => return Err("un enum necesita values = [\"a\", \"b\"]".into()),
        },
        "ref" => match t.get("table") {
            Some(toml::Value::String(s)) => Ty::Ref(s.clone()),
            _ => return Err("una referencia necesita table = \"nombre_de_tabla\"".into()),
        },
        "list" => match t.get("of") {
            Some(of) => Ty::List(Box::new(field_spec(of, line)?)),
            None => Ty::List(Box::new(Field::of(Ty::Any))),
        },
        "struct" => Ty::Struct(struct_spec(t, line)?),
        other => simple_type(other).ok_or_else(|| format!("tipo desconocido '{other}' (tipos: {TYPE_NAMES})"))?,
    };
    for k in t.keys() {
        if !matches!(k.as_str(), "type" | "values" | "table" | "of" | "fields" | "tag" | "variants" | "extra_fields" | "required" | "default" | "min" | "max" | "doc" | "description") {
            return Err(format!("opción desconocida '{k}' (usa type, required, default, min, max, values, table, of, fields, tag, variants, doc)"));
        }
    }
    let mut f = Field::of(ty);
    f.line = line;
    f.required = t.get("required").and_then(|v| v.as_bool()).unwrap_or(false);
    f.default = t.get("default").cloned();
    f.min = t.get("min").and_then(num_of);
    f.max = t.get("max").and_then(num_of);
    f.doc = t.get("doc").or(t.get("description")).and_then(|v| v.as_str()).unwrap_or("").to_string();
    if let Some(d) = &f.default {
        let mut bad = None;
        check_value(d, &f, &mut |m| bad = Some(m), "default");
        if let Some(m) = bad {
            return Err(format!("default no válido: {m}"));
        }
    }
    Ok(f)
}

fn simple_type(s: &str) -> Option<Ty> {
    Some(match s {
        "any" => Ty::Any,
        "string" | "str" | "text" => Ty::Str,
        "int" | "integer" => Ty::Int,
        "number" | "float" | "num" => Ty::Num,
        "bool" | "boolean" => Ty::Bool,
        "asset" => Ty::Asset(None),
        "list" | "array" => Ty::List(Box::new(Field::of(Ty::Any))),
        "struct" => Ty::Struct(StructSpec { extra: true, ..Default::default() }),
        other => Ty::Asset(Some(ASSET_TYPES.iter().find(|(n, _)| *n == other)?.1)),
    })
}

fn fields_of(v: Option<&toml::Value>, line: u32) -> Result<Vec<(String, Field)>, String> {
    match v {
        None => Ok(vec![]),
        Some(toml::Value::Table(t)) => t.iter().map(|(k, v)| Ok((k.clone(), field_spec(v, line).map_err(|m| format!("{k}: {m}"))?))).collect(),
        Some(_) => Err("fields debe ser una tabla { nombre = \"tipo\" }".into()),
    }
}

fn struct_spec(t: &toml::Table, line: u32) -> Result<StructSpec, String> {
    let mut s = StructSpec { fields: fields_of(t.get("fields"), line)?, ..Default::default() };
    s.extra = t.get("extra_fields").and_then(|v| v.as_bool()).unwrap_or(false);
    if let Some(v) = t.get("variants") {
        let toml::Value::Table(vt) = v else { return Err("variants debe ser una tabla { nombre = { campo = \"tipo\" } }".into()) };
        for (name, fv) in vt {
            let fields = match fv {
                toml::Value::Table(ft) => ft.iter().map(|(k, v)| Ok((k.clone(), field_spec(v, line).map_err(|m| format!("variante {name}.{k}: {m}"))?))).collect::<Result<Vec<_>, String>>()?,
                _ => return Err(format!("la variante '{name}' debe ser una tabla de campos")),
            };
            s.variants.push((name.clone(), fields));
        }
        s.tag = Some(t.get("tag").and_then(|v| v.as_str()).unwrap_or("type").to_string());
    } else if let Some(tag) = t.get("tag").and_then(|v| v.as_str()) {
        s.tag = Some(tag.to_string());
    }
    Ok(s)
}

// ------------------------------------------------------------------ tables

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Toml,
    Json,
    Csv,
}

impl Format {
    pub fn ext(self) -> &'static str {
        match self {
            Format::Toml => "toml",
            Format::Json => "json",
            Format::Csv => "csv",
        }
    }
}

/// One record as read from the file (`id` apart).
#[derive(Clone, Debug, PartialEq)]
pub struct RawRecord {
    pub id: String,
    pub line: u32,
    /// (field, value, line) in file order.
    pub fields: Vec<(String, toml::Value, u32)>,
}

impl RawRecord {
    pub fn get(&self, k: &str) -> Option<&toml::Value> {
        self.fields.iter().find(|(n, _, _)| n == k).map(|(_, v, _)| v)
    }
}

#[derive(Clone, Debug)]
pub struct RawTable {
    pub name: String,
    /// Relative to the project: `data/cards.toml`.
    pub file: String,
    pub path: PathBuf,
    pub format: Format,
    /// `card` in `[[card]]` (TOML) / the wrapping key (JSON).
    pub record_key: String,
    pub records: Vec<RawRecord>,
    pub schema: Option<Schema>,
}

/// Turns the id value into text (numbers are allowed and become "12").
fn id_text(v: &toml::Value) -> Option<String> {
    match v {
        toml::Value::String(s) => Some(s.clone()),
        toml::Value::Integer(i) => Some(i.to_string()),
        _ => None,
    }
}

/// Parses one table file (records only; [`validate`] checks them).
pub fn parse_table(name: &str, file: &str, text: &str, format: Format, schema: Option<&Schema>) -> Result<RawTable, Vec<Diag>> {
    let mut t = RawTable { name: name.to_string(), file: file.to_string(), path: PathBuf::new(), format, record_key: default_record_key(name), records: vec![], schema: None };
    let mut errs = vec![];
    match format {
        Format::Toml => {
            type Rec = Spanned<BTreeMap<String, Spanned<toml::Value>>>;
            let doc: BTreeMap<String, Spanned<toml::Value>> = toml::from_str(text).map_err(|e| vec![toml_err(file, text, &e)])?;
            let lists: Vec<&String> = doc.iter().filter(|(_, v)| matches!(v.get_ref(), toml::Value::Array(a) if a.iter().all(|x| x.is_table()))).map(|(k, _)| k).collect();
            for (k, v) in &doc {
                if !lists.contains(&k) {
                    errs.push(diag(file, line_of(text, v.span().start), format!("'{k}' no es una lista de registros: escribe cada registro como [[{}]] con id = \"...\"", t.record_key)));
                }
            }
            if lists.len() > 1 {
                errs.push(diag(file, 1, format!("una tabla usa un solo tipo de registro, aquí hay varios: {}", lists.iter().map(|k| format!("[[{k}]]")).collect::<Vec<_>>().join(", "))));
            }
            if let (Some(key), true) = (lists.first(), errs.is_empty()) {
                t.record_key = (*key).clone();
                // read again with a span for every record and field
                let d: BTreeMap<String, Vec<Rec>> = toml::from_str(text).map_err(|e| vec![toml_err(file, text, &e)])?;
                for rec in d.get(*key).map(|v| v.as_slice()).unwrap_or(&[]) {
                    let line = line_of(text, rec.span().start);
                    let mut fields: Vec<(String, toml::Value, u32)> = rec.get_ref().iter().map(|(k, v)| (k.clone(), v.get_ref().clone(), line_of(text, v.span().start))).collect();
                    fields.sort_by_key(|f| f.2);
                    push_record(&mut t, &mut errs, line, fields);
                }
            }
        }
        Format::Json => {
            let j: serde_json::Value = serde_json::from_str(text).map_err(|e| vec![diag(file, e.line() as u32, format!("JSON no válido: {e}"))])?;
            let list = match &j {
                serde_json::Value::Array(a) => Some(a),
                serde_json::Value::Object(m) if m.len() == 1 => {
                    let (k, v) = m.iter().next().unwrap();
                    t.record_key = k.clone();
                    v.as_array()
                }
                _ => None,
            };
            let Some(list) = list else {
                return Err(vec![diag(file, 1, "un JSON de datos es una lista de objetos [{\"id\": ...}, ...] o {\"registros\": [...]}")]);
            };
            let offsets = json_element_offsets(text);
            for (i, item) in list.iter().enumerate() {
                let line = offsets.get(i).map(|o| line_of(text, *o)).unwrap_or(1);
                let serde_json::Value::Object(m) = item else {
                    errs.push(diag(file, line, format!("el elemento {} no es un objeto {{\"id\": ...}}", i + 1)));
                    continue;
                };
                let fields = m.iter().filter_map(|(k, v)| json_to_toml(v).map(|tv| (k.clone(), tv, line))).collect();
                push_record(&mut t, &mut errs, line, fields);
            }
        }
        Format::Csv => {
            let rows = parse_csv(text).map_err(|(line, m)| vec![diag(file, line, m)])?;
            let Some((header, _)) = rows.first() else { return Ok(t) };
            let header: Vec<String> = header.iter().map(|h| h.trim().to_string()).collect();
            if !header.iter().any(|h| h == "id") {
                return Err(vec![diag(file, 1, "la primera fila del CSV son los nombres de los campos y debe incluir id")]);
            }
            for (cells, line) in rows.iter().skip(1) {
                if cells.iter().all(|c| c.trim().is_empty()) {
                    continue;
                }
                let mut fields = vec![];
                for (i, c) in cells.iter().enumerate() {
                    let Some(name) = header.get(i) else {
                        errs.push(diag(file, *line, format!("la fila tiene más columnas ({}) que la cabecera ({})", cells.len(), header.len())));
                        break;
                    };
                    if c.is_empty() {
                        continue;
                    }
                    let ty = if name == "id" { Some(&Ty::Str) } else { schema.and_then(|s| s.field(name)).map(|f| &f.ty) };
                    match csv_cell(c, ty) {
                        Ok(v) => fields.push((name.clone(), v, *line)),
                        Err(m) => errs.push(Diag { record: Some(t.records.len()), field: Some(name.clone()), ..diag(file, *line, format!("campo '{name}': {m}")) }),
                    }
                }
                push_record(&mut t, &mut errs, *line, fields);
            }
        }
    }
    if errs.is_empty() {
        Ok(t)
    } else {
        Err(errs)
    }
}

fn push_record(t: &mut RawTable, errs: &mut Vec<Diag>, line: u32, mut fields: Vec<(String, toml::Value, u32)>) {
    let idx = t.records.len();
    let Some(pos) = fields.iter().position(|(k, _, _)| k == "id") else {
        errs.push(Diag { record: Some(idx), field: Some("id".into()), ..diag(&t.file, line, "registro sin id: cada registro necesita id = \"...\"") });
        return;
    };
    let (_, idv, _) = fields.remove(pos);
    match id_text(&idv) {
        Some(id) if !id.trim().is_empty() => t.records.push(RawRecord { id, line, fields }),
        _ => errs.push(Diag { record: Some(idx), field: Some("id".into()), ..diag(&t.file, line, "el id debe ser un texto no vacío") }),
    }
}

/// Default record name of a new table: `cards` → `card`.
pub fn default_record_key(table: &str) -> String {
    match table.strip_suffix('s') {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => table.to_string(),
    }
}

fn json_to_toml(v: &serde_json::Value) -> Option<toml::Value> {
    Some(match v {
        serde_json::Value::Null => return None,
        serde_json::Value::Bool(b) => toml::Value::Boolean(*b),
        serde_json::Value::Number(n) => match n.as_i64() {
            Some(i) => toml::Value::Integer(i),
            None => toml::Value::Float(n.as_f64().unwrap_or(0.0)),
        },
        serde_json::Value::String(s) => toml::Value::String(s.clone()),
        serde_json::Value::Array(a) => toml::Value::Array(a.iter().filter_map(json_to_toml).collect()),
        serde_json::Value::Object(m) => toml::Value::Table(m.iter().filter_map(|(k, v)| Some((k.clone(), json_to_toml(v)?))).collect()),
    })
}

/// Byte offsets of the elements of the record list of a JSON file (for line numbers).
fn json_element_offsets(text: &str) -> Vec<usize> {
    let b = text.as_bytes();
    let mut depth = 0i32;
    let mut list_depth: Option<i32> = None;
    let mut out = vec![];
    let mut i = 0;
    let mut in_str = false;
    while i < b.len() {
        let c = b[i];
        if in_str {
            if c == b'\\' {
                i += 1;
            } else if c == b'"' {
                in_str = false;
            }
        } else {
            match c {
                b'"' => in_str = true,
                b'[' | b'{' => {
                    if c == b'[' && list_depth.is_none() {
                        list_depth = Some(depth + 1);
                    } else if c == b'{' && Some(depth) == list_depth {
                        out.push(i);
                    }
                    depth += 1;
                }
                b']' | b'}' => depth -= 1,
                _ => {}
            }
        }
        i += 1;
    }
    out
}

/// RFC-4180-ish CSV: `,` separated, `"..."` quoting with `""` escapes. Returns the rows
/// with the line where each starts.
pub fn parse_csv(text: &str) -> Result<Vec<(Vec<String>, u32)>, (u32, String)> {
    let mut rows = vec![];
    let mut row: Vec<String> = vec![];
    let mut cell = String::new();
    let mut line = 1u32;
    let mut row_line = 1u32;
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    let mut any = false;
    while let Some(c) = chars.next() {
        any = true;
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    cell.push('"');
                }
                '"' => quoted = false,
                '\n' => {
                    line += 1;
                    cell.push('\n');
                }
                _ => cell.push(c),
            }
            continue;
        }
        match c {
            '"' if cell.trim().is_empty() => {
                cell.clear();
                quoted = true;
            }
            ',' => row.push(std::mem::take(&mut cell)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut cell));
                rows.push((std::mem::take(&mut row), row_line));
                line += 1;
                row_line = line;
                any = false;
            }
            _ => cell.push(c),
        }
    }
    if quoted {
        return Err((row_line, "comillas sin cerrar en el CSV".into()));
    }
    if any {
        row.push(cell);
        rows.push((row, row_line));
    }
    Ok(rows)
}

/// A CSV cell typed by the schema (or guessed: number, true/false, else text).
fn csv_cell(c: &str, ty: Option<&Ty>) -> Result<toml::Value, String> {
    let s = c.trim();
    let inline = || -> Result<toml::Value, String> {
        let t: toml::Table = toml::from_str(&format!("v = {s}")).map_err(|e| format!("valor TOML no válido '{s}': {}", e.message()))?;
        Ok(t.get("v").cloned().unwrap_or(toml::Value::String(s.into())))
    };
    match ty {
        Some(Ty::Str | Ty::Enum(_) | Ty::Ref(_) | Ty::Asset(_)) => Ok(toml::Value::String(c.to_string())),
        Some(Ty::Int) => s.parse::<i64>().map(toml::Value::Integer).map_err(|_| format!("'{s}' no es un entero")),
        Some(Ty::Num) => s.parse::<f64>().map(toml::Value::Float).map_err(|_| format!("'{s}' no es un número")),
        Some(Ty::Bool) => match s {
            "true" | "1" | "sí" | "si" | "yes" => Ok(toml::Value::Boolean(true)),
            "false" | "0" | "no" => Ok(toml::Value::Boolean(false)),
            _ => Err(format!("'{s}' no es true/false")),
        },
        Some(Ty::List(_) | Ty::Struct(_)) => inline(),
        Some(Ty::Any) | None => {
            if let Ok(i) = s.parse::<i64>() {
                Ok(toml::Value::Integer(i))
            } else if let Ok(f) = s.parse::<f64>() {
                Ok(toml::Value::Float(f))
            } else if s == "true" || s == "false" {
                Ok(toml::Value::Boolean(s == "true"))
            } else if s.starts_with('[') || s.starts_with('{') {
                inline()
            } else {
                Ok(toml::Value::String(c.to_string()))
            }
        }
    }
}

// ------------------------------------------------------------------ files

/// Table files of `data/`: (table name, path, format). Schemas are not tables.
pub fn table_files(dir: &Path) -> Vec<(String, PathBuf, Format)> {
    let mut out = vec![];
    let Ok(rd) = crate::vfs::read_dir(&dir.join("data")) else { return out };
    let mut files: Vec<PathBuf> = rd.into_iter().filter(|(_, d)| !d).map(|(p, _)| p).collect();
    files.sort();
    for p in files {
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if name.starts_with('.') || name.ends_with(".schema.toml") {
            continue;
        }
        let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let format = match p.extension().map(|e| e.to_string_lossy().to_lowercase()).as_deref() {
            Some("toml") => Format::Toml,
            Some("json") => Format::Json,
            Some("csv") => Format::Csv,
            _ => continue,
        };
        out.push((stem, p, format));
    }
    out
}

/// `data/<table>.schema.toml`.
pub fn schema_path(dir: &Path, table: &str) -> PathBuf {
    dir.join("data").join(format!("{table}.schema.toml"))
}

fn rel(dir: &Path, p: &Path) -> String {
    p.strip_prefix(dir).unwrap_or(p).to_string_lossy().replace('\\', "/")
}

/// Reads every table and schema of a project (parse errors in the Vec).
pub fn load_raw(dir: &Path) -> (Vec<RawTable>, Vec<Diag>) {
    let mut tables: Vec<RawTable> = vec![];
    let mut errs = vec![];
    let files = table_files(dir);
    for (name, path, format) in &files {
        let file = rel(dir, path);
        if let Some(prev) = tables.iter().find(|t| &t.name == name) {
            errs.push(diag(&file, 1, format!("la tabla '{name}' ya está en {}: usa un solo fichero por tabla", prev.file)));
            continue;
        }
        let sp = schema_path(dir, name);
        let schema = if crate::vfs::is_file(&sp) {
            let sf = rel(dir, &sp);
            match crate::vfs::read_to_string(&sp) {
                Ok(text) => match parse_schema(&sf, &text) {
                    Ok(s) => Some(s),
                    Err(d) => {
                        errs.extend(d);
                        None
                    }
                },
                Err(e) => {
                    errs.push(diag(&sf, 1, e.to_string()));
                    None
                }
            }
        } else {
            None
        };
        let text = match crate::vfs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                errs.push(diag(&file, 1, e.to_string()));
                continue;
            }
        };
        match parse_table(name, &file, &text, *format, schema.as_ref()) {
            Ok(mut t) => {
                t.path = path.clone();
                t.schema = schema;
                tables.push(t);
            }
            Err(d) => errs.extend(d),
        }
    }
    // schemas without a table
    if let Ok(rd) = crate::vfs::read_dir(&dir.join("data")) {
        for (p, _) in rd {
            let n = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if let Some(t) = n.strip_suffix(".schema.toml") {
                if !files.iter().any(|(name, _, _)| name == t) {
                    errs.push(diag(&rel(dir, &p), 1, format!("esquema sin tabla: falta data/{t}.toml (o .json / .csv)")));
                }
            }
        }
    }
    (tables, errs)
}

// ------------------------------------------------------------------ validation

/// What validation needs to know about the game: does an asset of this kind exist?
/// (`None` kind: any asset.)
pub type AssetCheck<'a> = &'a dyn Fn(Option<AssetKind>, &str) -> bool;

fn type_name(v: &toml::Value) -> &'static str {
    match v {
        toml::Value::String(_) => "texto",
        toml::Value::Integer(_) => "entero",
        toml::Value::Float(_) => "número",
        toml::Value::Boolean(_) => "bool",
        toml::Value::Array(_) => "lista",
        toml::Value::Table(_) => "struct",
        toml::Value::Datetime(_) => "fecha",
    }
}

/// Checks a value against a field spec (no references: see [`validate`]).
fn check_value(v: &toml::Value, f: &Field, report: &mut dyn FnMut(String), path: &str) {
    let bad = |want: &str| format!("{path} debe ser {want}, no {} ({})", type_name(v), short(v));
    match (&f.ty, v) {
        (Ty::Any, _) => {}
        (Ty::Str | Ty::Ref(_) | Ty::Asset(_), toml::Value::String(_)) => {}
        (Ty::Str, _) => report(bad("un texto")),
        (Ty::Ref(t), _) => report(bad(&format!("el id (texto) de un registro de '{t}'"))),
        (Ty::Asset(k), _) => report(bad(&format!("el nombre de un {}", k.map(|k| k.name()).unwrap_or("asset")))),
        (Ty::Int, toml::Value::Integer(_)) => {}
        (Ty::Int, toml::Value::Float(x)) if x.fract() == 0.0 => {}
        (Ty::Int, _) => report(bad("un entero")),
        (Ty::Num, toml::Value::Integer(_) | toml::Value::Float(_)) => {}
        (Ty::Num, _) => report(bad("un número")),
        (Ty::Bool, toml::Value::Boolean(_)) => {}
        (Ty::Bool, _) => report(bad("true o false")),
        (Ty::Enum(vals), toml::Value::String(s)) => {
            if !vals.contains(s) {
                report(format!("{path}: '{s}' no es un valor válido (valores: {})", vals.join(", ")));
            }
        }
        (Ty::Enum(vals), _) => report(bad(&format!("uno de: {}", vals.join(", ")))),
        (Ty::List(item), toml::Value::Array(a)) => {
            for (i, x) in a.iter().enumerate() {
                check_value(x, item, report, &format!("{path}[{i}]"));
            }
        }
        (Ty::List(_), _) => report(bad("una lista [...]")),
        (Ty::Struct(s), toml::Value::Table(t)) => check_struct(t, s, report, path),
        (Ty::Struct(_), _) => report(bad("un struct { ... }")),
    }
    if let (Some(x), Some(min)) = (num_of(v), f.min) {
        if x < min {
            report(format!("{path} = {} es menor que el mínimo {}", short(v), modscript::fmt_num(min)));
        }
    }
    if let (Some(x), Some(max)) = (num_of(v), f.max) {
        if x > max {
            report(format!("{path} = {} es mayor que el máximo {}", short(v), modscript::fmt_num(max)));
        }
    }
}

/// The fields a struct value must follow (common + its variant's).
pub fn struct_fields<'a>(s: &'a StructSpec, t: &toml::Table) -> (Vec<&'a (String, Field)>, Result<Option<&'a str>, String>) {
    let mut fields: Vec<&(String, Field)> = s.fields.iter().collect();
    let mut variant = Ok(None);
    if let (Some(tag), false) = (&s.tag, s.variants.is_empty()) {
        match t.get(tag) {
            Some(toml::Value::String(name)) => match s.variants.iter().find(|(n, _)| n == name) {
                Some((n, vf)) => {
                    fields.extend(vf.iter());
                    variant = Ok(Some(n.as_str()));
                }
                None => variant = Err(format!("{tag} = '{name}' no es uno de: {}", s.variants.iter().map(|v| v.0.as_str()).collect::<Vec<_>>().join(", "))),
            },
            _ => variant = Err(format!("falta {tag} = \"...\" (uno de: {})", s.variants.iter().map(|v| v.0.as_str()).collect::<Vec<_>>().join(", "))),
        }
    }
    (fields, variant)
}

fn check_struct(t: &toml::Table, s: &StructSpec, report: &mut dyn FnMut(String), path: &str) {
    let (fields, variant) = struct_fields(s, t);
    if let Err(m) = &variant {
        report(format!("{path}: {m}"));
    }
    for (name, f) in &fields {
        match t.get(name) {
            Some(v) => check_value(v, f, report, &format!("{path}.{name}")),
            None if f.required => report(format!("{path}: falta el campo obligatorio '{name}'")),
            None => {}
        }
    }
    if !s.extra && (s.fields.len() + s.variants.len()) > 0 && variant.is_ok() {
        for k in t.keys() {
            if s.tag.as_deref() == Some(k.as_str()) || fields.iter().any(|(n, _)| n == k) {
                continue;
            }
            report(format!("{path}: campo desconocido '{k}'{}", suggest(k, fields.iter().map(|(n, _)| n.as_str()))));
        }
    }
}

fn short(v: &toml::Value) -> String {
    let s = inline_toml(v);
    if s.chars().count() > 40 {
        format!("{}…", s.chars().take(40).collect::<String>())
    } else {
        s
    }
}

/// `(¿quizás 'x'?)` for a misspelt name.
fn suggest<'a>(name: &str, options: impl Iterator<Item = &'a str>) -> String {
    let mut best: Option<(usize, &str)> = None;
    for o in options {
        let d = levenshtein(name, o);
        if d <= 2.max(name.len() / 3) && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, o));
        }
    }
    best.map(|(_, o)| format!(" (¿quizás '{o}'?)")).unwrap_or_default()
}

/// `(¿quizás 'x'?)` for scripts' error messages.
pub(crate) fn suggest_pub<'a>(name: &str, options: impl Iterator<Item = &'a str>) -> String {
    suggest(name, options)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            cur.push((prev[j] + (ca != *cb) as usize).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Walks a value and its spec, collecting references: (path, Ty::Ref table or asset kind, value).
fn refs<'a>(v: &'a toml::Value, f: &'a Field, path: String, out: &mut Vec<(String, &'a Ty, &'a str)>) {
    match (&f.ty, v) {
        (Ty::Ref(_) | Ty::Asset(_), toml::Value::String(s)) => out.push((path, &f.ty, s)),
        (Ty::List(item), toml::Value::Array(a)) => {
            for (i, x) in a.iter().enumerate() {
                refs(x, item, format!("{path}[{i}]"), out);
            }
        }
        (Ty::Struct(s), toml::Value::Table(t)) => {
            let (fields, _) = struct_fields(s, t);
            for (name, sf) in fields {
                if let Some(x) = t.get(name) {
                    refs(x, sf, format!("{path}.{name}"), out);
                }
            }
        }
        _ => {}
    }
}

/// Checks every table: types, required fields, enums, ranges, duplicate ids, references
/// to other tables and to assets.
pub fn validate(tables: &[RawTable], has_asset: AssetCheck) -> Vec<Diag> {
    let mut out = vec![];
    for t in tables {
        let mut seen: HashMap<&str, u32> = HashMap::new();
        for (ri, r) in t.records.iter().enumerate() {
            let mut at = |line: u32, field: Option<&str>, msg: String| out.push(Diag { file: t.file.clone(), line, msg: format!("'{}': {msg}", r.id), record: Some(ri), field: field.map(String::from) });
            if let Some(first) = seen.insert(r.id.as_str(), r.line) {
                at(r.line, Some("id"), format!("id repetido (ya está en la línea {first})"));
            }
            let Some(schema) = &t.schema else { continue };
            for (name, f) in &schema.root.fields {
                match r.get(name) {
                    Some(v) => {
                        let line = r.fields.iter().find(|x| &x.0 == name).map(|x| x.2).unwrap_or(r.line);
                        let mut msgs = vec![];
                        check_value(v, f, &mut |m| msgs.push(m), name);
                        for m in msgs {
                            at(line, Some(name), m);
                        }
                        let mut rs = vec![];
                        refs(v, f, name.clone(), &mut rs);
                        for (path, ty, s) in rs {
                            match ty {
                                Ty::Ref(target) => match tables.iter().find(|x| &x.name == target) {
                                    None => at(line, Some(name), format!("{path}: la tabla '{target}' no existe (data/{target}.toml)")),
                                    Some(tt) => {
                                        if !tt.records.iter().any(|x| x.id == s) {
                                            at(line, Some(name), format!("{path}: '{s}' no es un id de la tabla '{target}'{}", suggest(s, tt.records.iter().map(|x| x.id.as_str()))));
                                        }
                                    }
                                },
                                Ty::Asset(k) => {
                                    if !has_asset(*k, s) {
                                        at(line, Some(name), format!("{path}: no hay ningún {} llamado '{s}'", k.map(|k| k.name()).unwrap_or("asset")));
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    None if f.required && f.default.is_none() => at(r.line, Some(name), format!("falta el campo obligatorio '{name}'")),
                    None => {}
                }
            }
            if !schema.root.extra {
                for (k, _, line) in &r.fields {
                    if schema.field(k).is_none() {
                        let hint = suggest(k, schema.root.fields.iter().map(|(n, _)| n.as_str()));
                        at(*line, Some(k), format!("campo '{k}' no está en {}{hint} (añádelo al esquema o pon extra_fields = true)", schema.file));
                    }
                }
            }
        }
    }
    out
}

// ------------------------------------------------------------------ runtime values

/// A field value as a script value: assets become assets, structs/lists recurse and
/// schema defaults fill what is missing.
fn to_script(v: &toml::Value, f: Option<&Field>, assets: &HashMap<Sym, Value>) -> Value {
    match (f.map(|f| &f.ty), v) {
        (Some(Ty::Asset(_)), toml::Value::String(s)) => assets.get(&intern(s)).cloned().unwrap_or_else(|| Value::str(s)),
        (Some(Ty::List(item)), toml::Value::Array(a)) => Value::array(a.iter().map(|x| to_script(x, Some(item), assets)).collect()),
        (Some(Ty::Struct(s)), toml::Value::Table(t)) => struct_value(t.iter().map(|(k, v)| (k.as_str(), v)).collect(), s, assets),
        (_, toml::Value::String(s)) => Value::str(s),
        (_, toml::Value::Integer(i)) => Value::Num(*i as f64),
        (_, toml::Value::Float(x)) => Value::Num(*x),
        (_, toml::Value::Boolean(b)) => Value::Bool(*b),
        (_, toml::Value::Array(a)) => Value::array(a.iter().map(|x| to_script(x, None, assets)).collect()),
        (_, toml::Value::Table(t)) => Value::new_struct(t.iter().map(|(k, v)| (intern(k), to_script(v, None, assets)))),
        (_, toml::Value::Datetime(d)) => Value::string(d.to_string()),
    }
}

/// A struct with the spec's fields first (defaults filled, null when missing), then the
/// rest in file order.
fn struct_value(given: Vec<(&str, &toml::Value)>, s: &StructSpec, assets: &HashMap<Sym, Value>) -> Value {
    let table: toml::Table = given.iter().map(|(k, v)| (k.to_string(), (*v).clone())).collect();
    let (fields, _) = struct_fields(s, &table);
    let mut out: Vec<(Sym, Value)> = vec![];
    if let Some(tag) = &s.tag {
        if let Some((_, v)) = given.iter().find(|(k, _)| k == tag) {
            out.push((intern(tag), to_script(v, None, assets)));
        }
    }
    for (name, f) in fields {
        let v = match given.iter().find(|(k, _)| k == name) {
            Some((_, v)) => to_script(v, Some(f), assets),
            None => f.default.as_ref().map(|d| to_script(d, Some(f), assets)).unwrap_or(Value::Null),
        };
        out.push((intern(name), v));
    }
    for (k, v) in &given {
        if !out.iter().any(|(s, _)| &*s.name() == *k) {
            out.push((intern(k), to_script(v, None, assets)));
        }
    }
    Value::new_struct(out)
}

/// One record as a script struct: `id` first.
pub fn record_value(r: &RawRecord, schema: Option<&Schema>, assets: &HashMap<Sym, Value>) -> Value {
    let given: Vec<(&str, &toml::Value)> = r.fields.iter().map(|(k, v, _)| (k.as_str(), v)).collect();
    let body = match schema {
        Some(s) => struct_value(given, &s.root, assets),
        None => Value::new_struct(given.iter().map(|(k, v)| (intern(k), to_script(v, None, assets)))),
    };
    let Value::Struct(b) = &body else { return body };
    let mut fields = vec![(intern("id"), Value::str(&r.id))];
    fields.extend(b.borrow().fields.iter().map(|(k, v)| (*k, v.clone())));
    Value::new_struct(fields)
}

/// A table ready for scripts.
pub struct Table {
    pub name: String,
    pub file: String,
    pub ids: Vec<String>,
    pub index: HashMap<String, usize>,
    /// One struct per record (never handed out directly: `data_get` copies).
    pub values: Vec<Value>,
}

struct Watch {
    dir: PathBuf,
    stamps: Vec<(PathBuf, std::time::SystemTime, u64)>,
}

/// Every table of the game (`Engine::data`).
#[derive(Default)]
pub struct DataTables {
    pub tables: Vec<Table>,
    /// Bumped on every reload (`data_version()`).
    pub version: u64,
    watch: Option<Watch>,
}

impl DataTables {
    pub fn table(&self, name: &str) -> Option<&Table> {
        self.tables.iter().find(|t| t.name == name)
    }
}

/// Copy of arrays and structs (records are read-only: scripts get their own copy).
pub fn deep_copy(v: &Value) -> Value {
    match v {
        Value::Array(a) => Value::array(a.borrow().iter().map(deep_copy).collect()),
        Value::Struct(s) => Value::new_struct(s.borrow().fields.iter().map(|(k, v)| (*k, deep_copy(v)))),
        o => o.clone(),
    }
}

/// Does `assets` have an asset of this kind called `name`?
pub fn asset_exists(assets: &HashMap<Sym, Value>, kind: Option<AssetKind>, name: &str) -> bool {
    match assets.get(&intern(name)) {
        Some(Value::Asset(k, _)) => kind.is_none_or(|want| *k == want),
        _ => false,
    }
}

/// Loads, validates and converts every table of `dir` (errors as `file:line: ...`).
pub fn load(dir: &Path, assets: &HashMap<Sym, Value>) -> Result<Vec<Table>, Vec<Diag>> {
    let (raw, mut errs) = load_raw(dir);
    errs.extend(validate(&raw, &|k, n| asset_exists(assets, k, n)));
    if !errs.is_empty() {
        return Err(errs);
    }
    Ok(raw
        .iter()
        .map(|t| Table {
            name: t.name.clone(),
            file: t.file.clone(),
            ids: t.records.iter().map(|r| r.id.clone()).collect(),
            index: t.records.iter().enumerate().map(|(i, r)| (r.id.clone(), i)).collect(),
            values: t.records.iter().map(|r| record_value(r, t.schema.as_ref(), assets)).collect(),
        })
        .collect())
}

fn stamps(dir: &Path) -> Vec<(PathBuf, std::time::SystemTime, u64)> {
    let mut v = vec![];
    if let Ok(rd) = std::fs::read_dir(dir.join("data")) {
        for e in rd.flatten() {
            if let Ok(m) = e.metadata() {
                if m.is_file() {
                    v.push((e.path(), m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len()));
                }
            }
        }
    }
    v.sort();
    v
}

/// Called by `Engine::from_project`: loads the tables (errors stop the game from loading,
/// like script errors) and starts watching `data/` when running from a project folder.
pub(crate) fn attach(e: &mut Engine, errors: &mut Vec<String>) {
    match load(&e.dir, &e.assets) {
        Ok(t) => e.data.tables = t,
        Err(d) => errors.extend(d.iter().map(|d| d.to_string())),
    }
    if !crate::vfs::is_mounted(&e.dir) && std::fs::metadata(e.dir.join("game.toml")).is_ok() {
        e.data.watch = Some(Watch { dir: e.dir.clone(), stamps: stamps(&e.dir) });
    }
}

/// Reads the tables again. On errors the old data stays and the errors are returned.
pub fn reload(e: &mut Engine) -> Result<(), String> {
    match load(&e.dir, &e.assets) {
        Ok(t) => {
            e.data.tables = t;
            e.data.version += 1;
            Ok(())
        }
        Err(d) => Err(d.iter().map(|d| d.to_string()).collect::<Vec<_>>().join("\n")),
    }
}

/// Hot reload: every half second, reloads the tables if a file of `data/` changed.
pub(crate) fn step(e: &mut Engine) {
    if e.steps % 30 != 0 {
        return;
    }
    let Some(w) = &e.data.watch else { return };
    let now = stamps(&w.dir);
    if now == w.stamps {
        return;
    }
    if let Some(w) = &mut e.data.watch {
        w.stamps = now;
    }
    match reload(e) {
        Ok(()) => eprintln!("[datos] tablas recargadas (data/)"),
        Err(msg) => e.report(modscript::RtError::new(format!("data/ cambió pero tiene errores (se siguen usando los datos anteriores):\n{msg}"))),
    }
}

// ------------------------------------------------------------------ writing

/// A value as inline TOML (`{ type = "damage", amount = 6 }`).
pub fn inline_toml(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => toml::Value::String(s.clone()).to_string(),
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Float(f) => {
            let s = f.to_string();
            if s.contains('.') || s.contains('e') || s.contains("inf") || s.contains("nan") {
                s
            } else {
                format!("{s}.0")
            }
        }
        toml::Value::Boolean(b) => b.to_string(),
        toml::Value::Datetime(d) => d.to_string(),
        toml::Value::Array(a) => format!("[{}]", a.iter().map(inline_toml).collect::<Vec<_>>().join(", ")),
        toml::Value::Table(t) => {
            if t.is_empty() {
                "{}".into()
            } else {
                // a `type` tag reads best first: { type = "damage", amount = 6 }
                let mut keys: Vec<&String> = t.keys().collect();
                keys.sort_by_key(|k| k.as_str() != "type");
                format!("{{ {} }}", keys.iter().map(|k| format!("{} = {}", toml_key(k), inline_toml(&t[k.as_str()]))).collect::<Vec<_>>().join(", "))
            }
        }
    }
}

/// A key, quoted when it is not a bare key.
pub fn toml_key(k: &str) -> String {
    if !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        k.to_string()
    } else {
        toml::Value::String(k.to_string()).to_string()
    }
}

/// Parses an inline TOML value typed in a cell (`[1, 2]`, `{ a = 1 }`, `"x"`, `3`).
pub fn parse_inline(s: &str) -> Result<toml::Value, String> {
    let t: toml::Table = toml::from_str(&format!("v = {s}")).map_err(|e| e.message().to_string())?;
    t.get("v").cloned().ok_or_else(|| "vacío".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_quotes_and_lines() {
        let rows = parse_csv("id,name\na,\"x, y\"\nb,\"multi\nline\"\nc,z").unwrap();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[1].0, vec!["a", "x, y"]);
        assert_eq!(rows[2].0[1], "multi\nline");
        assert_eq!(rows[3].1, 5, "row after a two-line cell starts on line 5");
    }

    #[test]
    fn json_offsets() {
        let t = "[\n {\"id\": \"a\", \"x\": {\"y\": 1}},\n {\"id\": \"b\"}\n]";
        let o = json_element_offsets(t);
        assert_eq!(o.iter().map(|o| line_of(t, *o)).collect::<Vec<_>>(), vec![2, 3]);
        let t = "{\"cards\": [\n{\"id\": \"a\"},\n\n{\"id\": \"b\"}]}";
        assert_eq!(json_element_offsets(t).iter().map(|o| line_of(t, *o)).collect::<Vec<_>>(), vec![2, 4]);
    }

    #[test]
    fn inline_round_trip() {
        let v = parse_inline("[{ type = \"damage\", amount = 6 }, { \"odd key\" = 1.5 }]").unwrap();
        assert_eq!(parse_inline(&inline_toml(&v)).unwrap(), v);
    }

    #[test]
    fn suggestions() {
        assert_eq!(suggest("cots", ["cost", "name"].into_iter()), " (¿quizás 'cost'?)");
        assert_eq!(suggest("zzzzzz", ["cost"].into_iter()), "");
    }
}
