//! Typed object variables (`speed: num(0, 10) = 2`) in the editors: collecting them along
//! the parent chain, and one widget per type for the object editor (ModScript source) and
//! the room inspector (TOML values).

use crate::objsrc::{ItemKind, ObjectSrc};
use crate::project::Project;
use eframe::egui;
use modscript::{AssetKind, Expr, VarType};

/// A typed variable as seen from one object.
#[derive(Clone, Debug)]
pub struct VarInfo {
    pub name: String,
    pub ty: VarType,
    /// Initial value (ModScript source, without comments) after parents and overrides.
    pub value: String,
    /// Object that declares the type.
    pub owner: String,
    /// Object whose `name = value` line gives the current initial value.
    pub set_by: String,
}

/// Splits `value   // comment` into code and comment, ignoring `//` inside strings.
pub fn split_comment(v: &str) -> (&str, Option<&str>) {
    let b = v.as_bytes();
    let mut i = 0;
    let mut q = 0u8;
    while i < b.len() {
        let c = b[i];
        if q != 0 {
            if c == b'\\' {
                i += 1;
            } else if c == q {
                q = 0;
            }
        } else if c == b'"' || c == b'\'' {
            q = c;
        } else if c == b'/' && b.get(i + 1) == Some(&b'/') {
            return (v[..i].trim_end(), Some(&v[i..]));
        }
        i += 1;
    }
    (v.trim_end(), None)
}

fn chain(p: &Project, first: Option<&str>) -> Vec<(String, ObjectSrc)> {
    let mut out = Vec::new();
    let mut cur = first.map(String::from);
    while let Some(n) = cur {
        if out.len() > 32 || out.iter().any(|(o, _)| *o == n) {
            break;
        }
        let Some(o) = p.object(&n) else { break };
        cur = o.parent.clone();
        if let Some(m) = &o.model {
            out.push((n, m.clone()));
        }
    }
    out.reverse(); // root ancestor first
    out
}

fn fold(models: &[(String, ObjectSrc)]) -> Vec<VarInfo> {
    let mut vars: Vec<VarInfo> = Vec::new();
    for (obj, m) in models {
        for it in &m.items {
            let ItemKind::Prop { name, ty, value } = &it.kind else { continue };
            let value = split_comment(value).0.to_string();
            let existing = vars.iter_mut().find(|v| v.name == *name);
            match (ty.as_deref().map(modscript::parse_var_type), existing) {
                (Some(Ok(ty)), Some(v)) => {
                    v.ty = ty;
                    v.value = value;
                    v.set_by = obj.clone();
                }
                (Some(Ok(ty)), None) => vars.push(VarInfo { name: name.clone(), ty, value, owner: obj.clone(), set_by: obj.clone() }),
                (None, Some(v)) => {
                    v.value = value;
                    v.set_by = obj.clone();
                }
                _ => {}
            }
        }
    }
    vars
}

/// Typed variables declared by the ancestors of an object whose parent is `parent`.
pub fn inherited_vars(p: &Project, parent: Option<&str>) -> Vec<VarInfo> {
    fold(&chain(p, parent))
}

/// Every typed variable of object `name`, including inherited ones.
pub fn object_vars(p: &Project, name: &str) -> Vec<VarInfo> {
    fold(&chain(p, Some(name)))
}

// ------------------------------------------------------------------ plain values

#[derive(Clone, Debug, PartialEq)]
pub enum Plain {
    Num(f64),
    Bool(bool),
    Str(String),
    Color([u8; 3]),
    Asset(String),
}

pub fn from_source(ty: &VarType, src: &str) -> Option<Plain> {
    let e = modscript::parse_expr("v", src.trim()).ok()?;
    let num = match &e {
        Expr::Num(n) => Some(*n),
        Expr::Unary(modscript::UnOp::Neg, b) => match **b {
            Expr::Num(n) => Some(-n),
            _ => None,
        },
        _ => None,
    };
    Some(match (ty, e) {
        (VarType::Num { .. } | VarType::Int { .. }, _) => Plain::Num(num?),
        (VarType::Color, _) => {
            let c = num? as u32;
            Plain::Color([(c >> 16) as u8, (c >> 8) as u8, c as u8])
        }
        (VarType::Bool, Expr::Bool(b)) => Plain::Bool(b),
        (VarType::Str | VarType::List(_), Expr::Str(s)) => Plain::Str(s.to_string()),
        (VarType::Asset(_), Expr::Ident(s)) => Plain::Asset(s.name().to_string()),
        _ => return None,
    })
}

pub fn to_source(p: &Plain) -> String {
    match p {
        Plain::Num(n) => modscript::fmt_num(*n),
        Plain::Bool(b) => b.to_string(),
        Plain::Str(s) => format!("{s:?}"),
        Plain::Color(c) => format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]),
        Plain::Asset(a) => a.clone(),
    }
}

pub fn from_toml(ty: &VarType, v: &toml::Value) -> Option<Plain> {
    Some(match (ty, v) {
        (VarType::Num { .. } | VarType::Int { .. }, toml::Value::Integer(i)) => Plain::Num(*i as f64),
        (VarType::Num { .. } | VarType::Int { .. }, toml::Value::Float(f)) => Plain::Num(*f),
        (VarType::Bool, toml::Value::Boolean(b)) => Plain::Bool(*b),
        (VarType::Str | VarType::List(_), toml::Value::String(s)) => Plain::Str(s.clone()),
        (VarType::Asset(_), toml::Value::String(s)) => Plain::Asset(s.clone()),
        (VarType::Color, toml::Value::String(s)) => {
            let c = modengine::draw::parse_color(s)?;
            Plain::Color([(c >> 16) as u8, (c >> 8) as u8, c as u8])
        }
        (VarType::Color, toml::Value::Integer(c)) => Plain::Color([(c >> 16) as u8, (c >> 8) as u8, *c as u8]),
        _ => return None,
    })
}

pub fn to_toml(ty: &VarType, p: &Plain) -> toml::Value {
    match p {
        Plain::Num(n) if matches!(ty, VarType::Int { .. }) || n.fract() == 0.0 && n.abs() < 1e15 => toml::Value::Integer(*n as i64),
        Plain::Num(n) => toml::Value::Float(*n),
        Plain::Bool(b) => toml::Value::Boolean(*b),
        Plain::Str(s) | Plain::Asset(s) => toml::Value::String(s.clone()),
        Plain::Color(_) => toml::Value::String(to_source(p)),
    }
}

/// A value that fits `ty` (used when the type of a variable changes).
pub fn default_for(ty: &VarType, p: &Project) -> Plain {
    match ty {
        VarType::Num { min, max } | VarType::Int { min, max } => Plain::Num(0f64.max(min.unwrap_or(f64::MIN)).min(max.unwrap_or(f64::MAX))),
        VarType::Bool => Plain::Bool(false),
        VarType::Str => Plain::Str(String::new()),
        VarType::Color => Plain::Color([255, 255, 255]),
        VarType::List(o) => Plain::Str(o.first().cloned().unwrap_or_default()),
        VarType::Asset(k) => Plain::Asset(asset_names(p, *k).into_iter().next().unwrap_or_else(|| "null".into())),
    }
}

pub fn asset_names(p: &Project, k: AssetKind) -> Vec<String> {
    match k {
        AssetKind::Sprite => p.sprite_names(),
        AssetKind::Object => p.object_names(),
        AssetKind::Sound => p.sounds.iter().map(|s| s.name.clone()).collect(),
        AssetKind::Room => p.room_names(),
        AssetKind::Font => p.fonts.iter().map(|s| s.name.clone()).collect(),
        AssetKind::Path => p.motion.names(crate::motion::AnimKind::Path),
        AssetKind::Timeline => p.motion.names(crate::motion::AnimKind::Timeline),
        AssetKind::AnimCurve => p.motion.names(crate::motion::AnimKind::Curve),
        AssetKind::Sequence => p.motion.names(crate::motion::AnimKind::Sequence),
        AssetKind::TileSet => p.tilesets.iter().map(|s| s.name.clone()).collect(),
        AssetKind::ParticleSystem => p.fx.names(crate::fx::FxKind::Particles),
        AssetKind::Shader => p.fx.names(crate::fx::FxKind::Shader),
        AssetKind::Prefab => p.prefab_names(),
    }
}

/// Edits `v` with the widget for `ty`. Returns true when it changed.
pub fn widget(ui: &mut egui::Ui, id: impl std::hash::Hash + std::fmt::Debug + Copy, ty: &VarType, v: &mut Plain, p: &Project) -> bool {
    let before = v.clone();
    match (ty, &mut *v) {
        (VarType::Num { min: Some(a), max: Some(b) }, Plain::Num(n)) => {
            ui.add(egui::Slider::new(n, *a..=*b));
        }
        (VarType::Int { min: Some(a), max: Some(b) }, Plain::Num(n)) => {
            ui.add(egui::Slider::new(n, *a..=*b).integer());
        }
        (VarType::Num { min, max }, Plain::Num(n)) => {
            ui.add(egui::DragValue::new(n).speed(0.1).range(min.unwrap_or(f64::NEG_INFINITY)..=max.unwrap_or(f64::INFINITY)));
        }
        (VarType::Int { min, max }, Plain::Num(n)) => {
            ui.add(egui::DragValue::new(n).speed(0.25).fixed_decimals(0).range(min.unwrap_or(f64::NEG_INFINITY)..=max.unwrap_or(f64::INFINITY)));
            *n = n.round();
        }
        (VarType::Bool, Plain::Bool(b)) => {
            ui.checkbox(b, "");
        }
        (VarType::Str, Plain::Str(s)) => {
            ui.add(egui::TextEdit::singleline(s).desired_width(200.0));
        }
        (VarType::Color, Plain::Color(c)) => {
            ui.color_edit_button_srgb(c);
            ui.label(egui::RichText::new(to_source(&Plain::Color(*c))).monospace().weak());
        }
        (VarType::List(opts), Plain::Str(s)) => {
            egui::ComboBox::from_id_salt(id).selected_text(s.clone()).show_ui(ui, |ui| {
                for o in opts {
                    ui.selectable_value(s, o.clone(), o);
                }
            });
        }
        (VarType::Asset(k), Plain::Asset(a)) => {
            egui::ComboBox::from_id_salt(id).selected_text(a.clone()).show_ui(ui, |ui| {
                ui.selectable_value(a, "null".to_string(), "(ninguno)");
                for n in asset_names(p, *k) {
                    ui.selectable_value(a, n.clone(), n);
                }
            });
        }
        _ => {
            ui.label("?");
        }
    }
    *v != before
}

/// Small type editor: kind combo plus range or list options. Returns the new type when it changed.
pub fn type_editor(ui: &mut egui::Ui, id: impl std::hash::Hash + std::fmt::Debug + Copy, ty: Option<&VarType>) -> Option<Option<VarType>> {
    const KINDS: &[&str] = &["—", "num", "int", "bool", "str", "color", "list", "sprite", "object", "sound", "room", "font"];
    let kind = match ty {
        None => "—",
        Some(VarType::Num { .. }) => "num",
        Some(VarType::Int { .. }) => "int",
        Some(VarType::List(_)) => "list",
        Some(t) => KINDS.iter().find(|k| **k == t.to_source()).copied().unwrap_or("—"),
    };
    let mut out = None;
    egui::ComboBox::from_id_salt(("kind", id)).width(70.0).selected_text(kind).show_ui(ui, |ui| {
        for k in KINDS {
            if ui.selectable_label(*k == kind, *k).clicked() && *k != kind {
                out = Some(match *k {
                    "—" => None,
                    "list" => Some(VarType::List(vec!["a".into(), "b".into()])),
                    other => modscript::parse_var_type(other).ok(),
                });
            }
        }
    });
    if let Some(t @ (VarType::Num { min, max } | VarType::Int { min, max })) = ty {
        let fmt = |x: &Option<f64>| x.map(modscript::fmt_num).unwrap_or_default();
        let mut lo = ui.data_mut(|d| d.get_temp::<String>(egui::Id::unique(("lo", id))).unwrap_or_else(|| fmt(min)));
        let mut hi = ui.data_mut(|d| d.get_temp::<String>(egui::Id::unique(("hi", id))).unwrap_or_else(|| fmt(max)));
        let r1 = ui.add(egui::TextEdit::singleline(&mut lo).hint_text("mín").desired_width(36.0));
        let r2 = ui.add(egui::TextEdit::singleline(&mut hi).hint_text("máx").desired_width(36.0));
        ui.data_mut(|d| {
            d.insert_temp(egui::Id::unique(("lo", id)), lo.clone());
            d.insert_temp(egui::Id::unique(("hi", id)), hi.clone());
        });
        if r1.changed() || r2.changed() {
            let p = |s: &str| if s.trim().is_empty() { Some(None) } else { s.trim().parse::<f64>().ok().map(Some) };
            if let (Some(a), Some(b)) = (p(&lo), p(&hi)) {
                if a.zip(b).is_none_or(|(a, b)| a <= b) {
                    out = Some(Some(match t {
                        VarType::Int { .. } => VarType::Int { min: a, max: b },
                        _ => VarType::Num { min: a, max: b },
                    }));
                }
            }
        }
    }
    if let Some(VarType::List(opts)) = ty {
        let key = egui::Id::unique(("opts", id));
        let mut s = ui.data_mut(|d| d.get_temp::<String>(key).unwrap_or_else(|| opts.join(", ")));
        let r = ui.add(egui::TextEdit::singleline(&mut s).hint_text("a, b, c").desired_width(120.0));
        ui.data_mut(|d| d.insert_temp(key, s.clone()));
        if r.changed() {
            let v: Vec<String> = s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
            if !v.is_empty() {
                out = Some(Some(VarType::List(v)));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_roundtrip() {
        let t = |s: &str| modscript::parse_var_type(s).unwrap();
        assert_eq!(from_source(&t("num"), "-2.5"), Some(Plain::Num(-2.5)));
        assert_eq!(from_source(&t("color"), "#ff8800"), Some(Plain::Color([255, 136, 0])));
        assert_eq!(to_source(&Plain::Color([255, 136, 0])), "#ff8800");
        assert_eq!(from_source(&t("sprite"), "coin"), Some(Plain::Asset("coin".into())));
        assert_eq!(from_source(&t("list(\"a\", \"b\")"), "\"b\""), Some(Plain::Str("b".into())));
        assert_eq!(from_source(&t("num"), "room_width / 2"), None, "expressions fall back to text");
        assert_eq!(to_toml(&t("color"), &Plain::Color([1, 2, 3])), toml::Value::String("#010203".into()));
        assert_eq!(split_comment("\"a//b\"  // c"), ("\"a//b\"", Some("// c")));
    }
}
