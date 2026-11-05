//! Runtime values of ModScript.

use crate::ast::FuncDef;
use crate::interp::Scope;
use crate::sym::Sym;
use indexmap::IndexMap;
use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

/// Kinds of game assets a script can refer to by name.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum AssetKind {
    Object,
    Sprite,
    Sound,
    Room,
    Font,
}

impl AssetKind {
    pub fn name(self) -> &'static str {
        match self {
            AssetKind::Object => "object",
            AssetKind::Sprite => "sprite",
            AssetKind::Sound => "sound",
            AssetKind::Room => "room",
            AssetKind::Font => "font",
        }
    }
}

pub type ArrayRef = Rc<RefCell<Vec<Value>>>;
pub type StructRef = Rc<RefCell<StructObj>>;

#[derive(Default, Debug)]
pub struct StructObj {
    pub fields: IndexMap<Sym, Value>,
}

/// A script function plus the scope it is bound to (if any).
pub struct Closure {
    pub def: Rc<FuncDef>,
    pub this: Option<Scope>,
}

#[derive(Clone, Default)]
pub enum Value {
    #[default]
    Null,
    Bool(bool),
    Num(f64),
    Str(Rc<str>),
    Array(ArrayRef),
    Struct(StructRef),
    Func(Rc<Closure>),
    /// A built-in function provided by the host.
    Native(Sym),
    /// An instance id.
    Inst(u32),
    Asset(AssetKind, u32),
    /// The `all` keyword: every instance.
    All,
}

impl Value {
    pub fn str(s: &str) -> Value {
        Value::Str(Rc::from(s))
    }
    pub fn string(s: String) -> Value {
        Value::Str(Rc::from(s.as_str()))
    }
    pub fn array(v: Vec<Value>) -> Value {
        Value::Array(Rc::new(RefCell::new(v)))
    }
    pub fn new_struct(fields: impl IntoIterator<Item = (Sym, Value)>) -> Value {
        Value::Struct(Rc::new(RefCell::new(StructObj { fields: fields.into_iter().collect() })))
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn truthy(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Num(n) => *n != 0.0 && !n.is_nan(),
            Value::Str(s) => !s.is_empty(),
            _ => true,
        }
    }

    pub fn as_num(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Num(_) => "number",
            Value::Str(_) => "string",
            Value::Array(_) => "array",
            Value::Struct(_) => "struct",
            Value::Func(_) | Value::Native(_) => "function",
            Value::Inst(_) | Value::All => "instance",
            Value::Asset(k, _) => k.name(),
        }
    }

    pub fn loose_eq(&self, other: &Value) -> bool {
        use Value::*;
        match (self, other) {
            (Null, Null) => true,
            (Num(a), Num(b)) => a == b,
            (Bool(a), Bool(b)) => a == b,
            (Bool(_), Num(_)) | (Num(_), Bool(_)) => self.as_num() == other.as_num(),
            (Str(a), Str(b)) => a == b,
            (Array(a), Array(b)) => Rc::ptr_eq(a, b),
            (Struct(a), Struct(b)) => Rc::ptr_eq(a, b),
            (Func(a), Func(b)) => Rc::ptr_eq(&a.def, &b.def),
            (Native(a), Native(b)) => a == b,
            (Inst(a), Inst(b)) => a == b,
            (Asset(k1, a), Asset(k2, b)) => k1 == k2 && a == b,
            (All, All) => true,
            _ => false,
        }
    }

    fn write(&self, f: &mut fmt::Formatter<'_>, depth: u32, quoted: bool) -> fmt::Result {
        match self {
            Value::Null => f.write_str("null"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Num(n) => f.write_str(&fmt_num(*n)),
            Value::Str(s) => {
                if quoted {
                    write!(f, "{s:?}")
                } else {
                    f.write_str(s)
                }
            }
            Value::Array(a) => {
                if depth > 6 {
                    return f.write_str("[...]");
                }
                f.write_str("[")?;
                for (i, v) in a.borrow().iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    v.write(f, depth + 1, true)?;
                }
                f.write_str("]")
            }
            Value::Struct(s) => {
                if depth > 6 {
                    return f.write_str("{...}");
                }
                f.write_str("{")?;
                for (i, (k, v)) in s.borrow().fields.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{k}: ")?;
                    v.write(f, depth + 1, true)?;
                }
                f.write_str("}")
            }
            Value::Func(c) => write!(f, "<function {}>", c.def.name),
            Value::Native(s) => write!(f, "<builtin {s}>"),
            Value::Inst(id) => write!(f, "<instance {id}>"),
            Value::Asset(k, id) => write!(f, "<{} {id}>", k.name()),
            Value::All => f.write_str("<all>"),
        }
    }
}

/// Formats numbers the friendly way: integers without decimals, others trimmed.
pub fn fmt_num(n: f64) -> String {
    if n.is_nan() {
        return "nan".into();
    }
    if n.is_infinite() {
        return if n > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if n.fract() == 0.0 && n.abs() < 1e15 {
        return format!("{}", n as i64);
    }
    let s = format!("{n:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_string()
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write(f, 0, false)
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write(f, 0, true)
    }
}

impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Value::Num(n)
    }
}
impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}
impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::str(s)
    }
}
impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::string(s)
    }
}
