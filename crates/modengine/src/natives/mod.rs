//! Built-in functions available to every script.

mod core;
mod game;
mod gfx;
mod particles;
mod shader;
mod extension;
mod font;
pub mod tilemap;
mod animcurve;
mod path;
mod sequence;
mod timeline;
mod palette;

use crate::draw::NAMED_COLORS;
use crate::engine::Engine;
use modscript::{intern, ArrayRef, AssetKind, CallCx, RtError, RtResult, StructRef, Sym, Value};
use std::collections::HashMap;
use std::rc::Rc;

pub type NativeFn = fn(&mut Engine, &CallCx, &[Value]) -> RtResult<Value>;

pub(crate) struct Reg<'a>(&'a mut HashMap<Sym, NativeFn>);

impl Reg<'_> {
    pub fn f(&mut self, name: &str, f: NativeFn) {
        self.0.insert(intern(name), f);
    }
}

pub fn registry() -> HashMap<Sym, NativeFn> {
    let mut m = HashMap::new();
    let mut r = Reg(&mut m);
    core::register(&mut r);
    game::register(&mut r);
    gfx::register(&mut r);
    particles::register(&mut r);
    shader::register(&mut r);
    extension::register(&mut r);
    font::register(&mut r);
    tilemap::register(&mut r);
    animcurve::register(&mut r);
    path::register(&mut r);
    sequence::register(&mut r);
    timeline::register(&mut r);
    palette::register(&mut r);
    m
}

/// Names of every builtin function (for docs / tooling).
pub fn names() -> Vec<String> {
    let mut v: Vec<String> = registry().keys().map(|s| s.name().to_string()).collect();
    v.sort();
    v
}

pub fn constants() -> HashMap<Sym, Value> {
    let mut c = HashMap::new();
    c.insert(intern("key"), Value::new_struct(Engine::key_struct_names()));
    c.insert(intern("mouse"), Value::new_struct(Engine::mouse_struct_names()));
    c.insert(
        intern("color"),
        Value::new_struct(NAMED_COLORS.iter().map(|(n, v)| (intern(n), Value::Num(*v as f64)))),
    );
    c.insert(
        intern("align"),
        Value::new_struct([
            (intern("left"), Value::Num(0.0)),
            (intern("center"), Value::Num(1.0)),
            (intern("right"), Value::Num(2.0)),
            (intern("top"), Value::Num(0.0)),
            (intern("middle"), Value::Num(1.0)),
            (intern("bottom"), Value::Num(2.0)),
        ]),
    );
    c.insert(
        intern("path_action"),
        Value::new_struct([
            (intern("stop"), Value::Num(0.0)),
            (intern("restart"), Value::Num(1.0)),
            (intern("continue"), Value::Num(2.0)),
            (intern("reverse"), Value::Num(3.0)),
        ]),
    );
    c.insert(intern("pi"), Value::Num(std::f64::consts::PI));
    c.insert(intern("infinity"), Value::Num(f64::INFINITY));
    c
}

/// xorshift64* in [0, 1)
pub fn rand01(state: &mut u64) -> f64 {
    let mut x = *state;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    *state = x;
    let r = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
    (r >> 11) as f64 / (1u64 << 53) as f64
}

// ------------------------------------------------------------------ argument helpers

pub(crate) struct A<'a>(pub &'a [Value]);

fn err<T>(msg: impl Into<String>) -> RtResult<T> {
    Err(RtError::new(msg))
}

impl<'a> A<'a> {
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn opt(&self, i: usize) -> Option<&'a Value> {
        self.0.get(i).filter(|v| !v.is_null())
    }
    pub fn val(&self, i: usize) -> RtResult<&'a Value> {
        self.0.get(i).ok_or_else(|| RtError::new(format!("missing argument #{}", i + 1)))
    }
    pub fn num(&self, i: usize) -> RtResult<f64> {
        let v = self.val(i)?;
        v.as_num().ok_or_else(|| RtError::new(format!("argument #{} must be a number, got {}", i + 1, v.type_name())))
    }
    pub fn num_or(&self, i: usize, d: f64) -> RtResult<f64> {
        match self.opt(i) {
            Some(_) => self.num(i),
            None => Ok(d),
        }
    }
    pub fn int(&self, i: usize) -> RtResult<i64> {
        Ok(self.num(i)?.floor() as i64)
    }
    pub fn bool_or(&self, i: usize, d: bool) -> bool {
        self.opt(i).map(|v| v.truthy()).unwrap_or(d)
    }
    pub fn str(&self, i: usize) -> RtResult<Rc<str>> {
        match self.val(i)? {
            Value::Str(s) => Ok(s.clone()),
            other => err(format!("argument #{} must be a string, got {}", i + 1, other.type_name())),
        }
    }
    /// Any value converted to text.
    pub fn text(&self, i: usize) -> RtResult<String> {
        Ok(self.val(i)?.to_string())
    }
    pub fn array(&self, i: usize) -> RtResult<ArrayRef> {
        match self.val(i)? {
            Value::Array(a) => Ok(a.clone()),
            other => err(format!("argument #{} must be an array, got {}", i + 1, other.type_name())),
        }
    }
    pub fn strukt(&self, i: usize) -> RtResult<StructRef> {
        match self.val(i)? {
            Value::Struct(s) => Ok(s.clone()),
            other => err(format!("argument #{} must be a struct, got {}", i + 1, other.type_name())),
        }
    }
    pub fn asset(&self, i: usize, kind: AssetKind) -> RtResult<u32> {
        match self.val(i)? {
            Value::Asset(k, id) if *k == kind => Ok(*id),
            other => err(format!("argument #{} must be a {}, got {}", i + 1, kind.name(), other.type_name())),
        }
    }
    pub fn opt_asset(&self, i: usize, kind: AssetKind) -> RtResult<Option<u32>> {
        match self.opt(i) {
            None => Ok(None),
            Some(_) => self.asset(i, kind).map(Some),
        }
    }
    /// An object, instance or `all` (a collision / query target).
    pub fn target(&self, i: usize) -> RtResult<Value> {
        match self.val(i)? {
            v @ (Value::Asset(AssetKind::Object, _) | Value::Inst(_) | Value::All) => Ok(v.clone()),
            other => err(format!("argument #{} must be an object, instance or all, got {}", i + 1, other.type_name())),
        }
    }
}

/// Index of the calling instance.
pub(crate) fn me(e: &Engine, cx: &CallCx) -> RtResult<usize> {
    match cx.self_ {
        modscript::Scope::Inst(id) => e.idx(id).ok_or_else(|| RtError::new("the calling instance no longer exists")),
        _ => err("this function must be called from an instance (object event or method)"),
    }
}
