//! Tree-walking interpreter.
//!
//! The interpreter knows nothing about games: everything about instances,
//! assets and built-in functions is delegated to a [`Host`].

use crate::ast::*;
use crate::sym::{intern, Sym};
use crate::value::*;
use std::cell::Cell;
use std::fmt;
use std::rc::Rc;

/// Who `self` / `other` are while code runs.
#[derive(Clone)]
pub enum Scope {
    /// Top-level and room code: plain assignments create global variables.
    Global,
    Inst(u32),
    Struct(StructRef),
}

impl Scope {
    pub fn to_value(&self) -> Value {
        match self {
            Scope::Global => Value::Null,
            Scope::Inst(id) => Value::Inst(*id),
            Scope::Struct(s) => Value::Struct(s.clone()),
        }
    }
    pub fn inst(&self) -> Option<u32> {
        match self {
            Scope::Inst(id) => Some(*id),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct CallCx {
    pub self_: Scope,
    pub other: Scope,
}

impl CallCx {
    pub fn global() -> Self {
        CallCx { self_: Scope::Global, other: Scope::Global }
    }
}

#[derive(Debug, Clone)]
pub struct RtError {
    pub msg: String,
    pub file: Rc<str>,
    pub line: u32,
    pub trace: Vec<String>,
}

impl RtError {
    pub fn new(msg: impl Into<String>) -> Self {
        RtError { msg: msg.into(), file: Rc::from(""), line: 0, trace: Vec::new() }
    }
}

impl fmt::Display for RtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line > 0 {
            write!(f, "{}:{}: {}", self.file, self.line, self.msg)?;
        } else {
            f.write_str(&self.msg)?;
        }
        for t in &self.trace {
            write!(f, "\n    {t}")?;
        }
        Ok(())
    }
}

pub type RtResult<T> = Result<T, RtError>;

fn err<T>(msg: impl Into<String>) -> RtResult<T> {
    Err(RtError::new(msg))
}

/// Everything the interpreter needs from the game engine.
pub trait Host {
    fn user_function(&self, name: Sym) -> Option<Rc<FuncDef>>;
    fn has_native(&self, name: Sym) -> bool;
    fn call_native(&mut self, name: Sym, cx: &CallCx, args: Vec<Value>) -> RtResult<Value>;
    fn global_get(&mut self, name: Sym) -> Option<Value>;
    fn global_set(&mut self, name: Sym, v: Value);
    /// `Ok(None)` means the instance exists but has no such variable.
    fn inst_get(&mut self, id: u32, name: Sym) -> RtResult<Option<Value>>;
    fn inst_set(&mut self, id: u32, name: Sym, v: Value) -> RtResult<()>;
    fn inst_alive(&self, id: u32) -> bool;
    /// Instances selected by an object, `all` or an instance id.
    fn instances_of(&mut self, v: &Value) -> Vec<u32>;
    /// Read-only names: assets, constants, dynamic values like `mouse_x`.
    fn constant(&mut self, name: Sym) -> Option<Value>;
}

pub enum Flow {
    Normal,
    Break,
    Continue,
    Return(Value),
    Exit,
}

struct Frame {
    locals: Vec<(Sym, Value)>,
    cx: CallCx,
    file: Rc<str>,
}

impl Frame {
    fn local_index(&self, s: Sym) -> Option<usize> {
        self.locals.iter().rposition(|(k, _)| *k == s)
    }
    fn set_local(&mut self, s: Sym, v: Value) {
        match self.local_index(s) {
            Some(i) => self.locals[i].1 = v,
            None => self.locals.push((s, v)),
        }
    }
}

thread_local! {
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}
const MAX_DEPTH: u32 = 256;

// ------------------------------------------------------------------ public API

/// Runs a function definition with the given `self`/`other` and arguments.
pub fn run_def<H: Host>(h: &mut H, def: &Rc<FuncDef>, self_: Scope, other: Scope, args: Vec<Value>) -> RtResult<Value> {
    let depth = DEPTH.with(|d| d.get());
    if depth >= MAX_DEPTH {
        return err("stack overflow: too much recursion");
    }
    DEPTH.with(|d| d.set(depth + 1));
    let mut fr = Frame { locals: Vec::with_capacity(def.params.len() + 4), cx: CallCx { self_, other }, file: def.file.clone() };
    let result = (|| {
        let mut args = args.into_iter();
        for p in &def.params {
            let v = match (args.next(), &p.default) {
                (Some(v), None) => v,
                (Some(v), Some(_)) if !v.is_null() => v,
                (_, Some(d)) => eval(h, &mut fr, d)?,
                (None, None) => Value::Null,
            };
            fr.locals.push((p.name, v));
        }
        exec_block(h, &mut fr, &def.body)
    })();
    DEPTH.with(|d| d.set(depth));
    match result {
        Ok(Flow::Return(v)) => Ok(v),
        Ok(_) => Ok(Value::Null),
        Err(mut e) => {
            if e.line == 0 {
                e.file = def.file.clone();
                e.line = def.line;
            }
            e.trace.push(format!("in {} ({}:{})", def.name, def.file, def.line));
            Err(e)
        }
    }
}

/// Runs a list of statements (object properties, room code, top-level init).
pub fn run_stmts<H: Host>(h: &mut H, stmts: &[Stmt], file: &Rc<str>, cx: CallCx) -> RtResult<()> {
    let mut fr = Frame { locals: Vec::new(), cx, file: file.clone() };
    match exec_block(h, &mut fr, stmts)? {
        Flow::Break | Flow::Continue => err("'break'/'continue' outside of a loop"),
        _ => Ok(()),
    }
}

/// Evaluates a standalone expression.
pub fn eval_expr<H: Host>(h: &mut H, e: &Expr, file: &Rc<str>, cx: CallCx) -> RtResult<Value> {
    let mut fr = Frame { locals: Vec::new(), cx, file: file.clone() };
    eval(h, &mut fr, e)
}

/// Calls any callable value (script function, bound method or builtin).
pub fn call_value<H: Host>(h: &mut H, f: &Value, cx: &CallCx, args: Vec<Value>) -> RtResult<Value> {
    call_with(h, f, cx, args, None)
}

fn call_with<H: Host>(h: &mut H, f: &Value, cx: &CallCx, args: Vec<Value>, this: Option<Scope>) -> RtResult<Value> {
    match f {
        Value::Func(c) => {
            let bound = c.this.clone().or(this);
            match bound {
                Some(s) => run_def(h, &c.def, s, cx.self_.clone(), args),
                None => run_def(h, &c.def, cx.self_.clone(), cx.other.clone(), args),
            }
        }
        Value::Native(s) => match this {
            Some(t) => h.call_native(*s, &CallCx { self_: t, other: cx.self_.clone() }, args),
            None => h.call_native(*s, cx, args),
        },
        other => err(format!("{} is not a function", other.type_name())),
    }
}

// ------------------------------------------------------------------ statements

fn exec_block<H: Host>(h: &mut H, fr: &mut Frame, stmts: &[Stmt]) -> RtResult<Flow> {
    for s in stmts {
        match exec(h, fr, s)? {
            Flow::Normal => {}
            f => return Ok(f),
        }
    }
    Ok(Flow::Normal)
}

fn exec<H: Host>(h: &mut H, fr: &mut Frame, s: &Stmt) -> RtResult<Flow> {
    exec_inner(h, fr, s).map_err(|mut e| {
        if e.line == 0 {
            e.line = s.line;
            e.file = fr.file.clone();
        }
        e
    })
}

fn exec_inner<H: Host>(h: &mut H, fr: &mut Frame, s: &Stmt) -> RtResult<Flow> {
    match &s.kind {
        StmtKind::Expr(e) => {
            eval(h, fr, e)?;
        }
        StmtKind::Var(decls) => {
            for (name, init) in decls {
                let v = match init {
                    Some(e) => eval(h, fr, e)?,
                    None => Value::Null,
                };
                fr.set_local(*name, v);
            }
        }
        StmtKind::Assign(target, op, rhs) => {
            let place = resolve_place(h, fr, target)?;
            match op {
                AssignOp::Set => {
                    let v = eval(h, fr, rhs)?;
                    place_set(h, fr, &place, v)?;
                }
                AssignOp::Op(bop) => {
                    let cur = place_get_req(h, fr, &place, target)?;
                    let v = eval(h, fr, rhs)?;
                    let r = binary(*bop, &cur, &v)?;
                    place_set(h, fr, &place, r)?;
                }
                AssignOp::Coalesce => {
                    let cur = place_get(h, fr, &place)?;
                    if cur.map(|c| c.is_null()).unwrap_or(true) {
                        let v = eval(h, fr, rhs)?;
                        place_set(h, fr, &place, v)?;
                    }
                }
            }
        }
        StmtKind::Incr(target, d) => {
            let place = resolve_place(h, fr, target)?;
            let cur = place_get_req(h, fr, &place, target)?;
            let r = binary(BinOp::Add, &cur, &Value::Num(*d))?;
            place_set(h, fr, &place, r)?;
        }
        StmtKind::If(c, t, e) => {
            if eval(h, fr, c)?.truthy() {
                return exec(h, fr, t);
            } else if let Some(e) = e {
                return exec(h, fr, e);
            }
        }
        StmtKind::While(c, body) => {
            while eval(h, fr, c)?.truthy() {
                match exec(h, fr, body)? {
                    Flow::Break => break,
                    Flow::Normal | Flow::Continue => {}
                    f => return Ok(f),
                }
            }
        }
        StmtKind::DoUntil(body, c) => loop {
            match exec(h, fr, body)? {
                Flow::Break => break,
                Flow::Normal | Flow::Continue => {}
                f => return Ok(f),
            }
            if eval(h, fr, c)?.truthy() {
                break;
            }
        },
        StmtKind::For(init, cond, step, body) => {
            if let Some(i) = init {
                exec(h, fr, i)?;
            }
            loop {
                if let Some(c) = cond {
                    if !eval(h, fr, c)?.truthy() {
                        break;
                    }
                }
                match exec(h, fr, body)? {
                    Flow::Break => break,
                    Flow::Normal | Flow::Continue => {}
                    f => return Ok(f),
                }
                if let Some(st) = step {
                    exec(h, fr, st)?;
                }
            }
        }
        StmtKind::Repeat(n, body) => {
            let n = num(&eval(h, fr, n)?)?.floor().max(0.0) as u64;
            for _ in 0..n {
                match exec(h, fr, body)? {
                    Flow::Break => break,
                    Flow::Normal | Flow::Continue => {}
                    f => return Ok(f),
                }
            }
        }
        StmtKind::Switch(v, cases) => {
            let v = eval(h, fr, v)?;
            let mut start = None;
            for (i, c) in cases.iter().enumerate() {
                if let Some(e) = &c.value {
                    if v.loose_eq(&eval(h, fr, e)?) {
                        start = Some(i);
                        break;
                    }
                }
            }
            if start.is_none() {
                start = cases.iter().position(|c| c.value.is_none());
            }
            if let Some(start) = start {
                for c in &cases[start..] {
                    match exec_block(h, fr, &c.body)? {
                        Flow::Normal => {}
                        Flow::Break => break,
                        f => return Ok(f),
                    }
                }
            }
        }
        StmtKind::Block(b) => return exec_block(h, fr, b),
        StmtKind::Break => return Ok(Flow::Break),
        StmtKind::Continue => return Ok(Flow::Continue),
        StmtKind::Exit => return Ok(Flow::Exit),
        StmtKind::Return(v) => {
            let v = match v {
                Some(e) => eval(h, fr, e)?,
                None => Value::Null,
            };
            return Ok(Flow::Return(v));
        }
        StmtKind::With(target, body) => {
            let t = eval(h, fr, target)?;
            let scopes: Vec<Scope> = match &t {
                Value::Null => vec![],
                Value::Struct(s) => vec![Scope::Struct(s.clone())],
                Value::Inst(_) | Value::All | Value::Asset(AssetKind::Object, _) => {
                    h.instances_of(&t).into_iter().map(Scope::Inst).collect()
                }
                other => return err(format!("with() needs an object, instance or struct, got {}", other.type_name())),
            };
            for sc in scopes {
                if let Scope::Inst(id) = sc {
                    if !h.inst_alive(id) {
                        continue;
                    }
                }
                let saved = fr.cx.clone();
                fr.cx = CallCx { self_: sc, other: saved.self_.clone() };
                let r = exec(h, fr, body);
                fr.cx = saved;
                match r? {
                    Flow::Break => break,
                    Flow::Normal | Flow::Continue => {}
                    f => return Ok(f),
                }
            }
        }
        StmtKind::FuncDecl(name, def) => {
            let this = match &fr.cx.self_ {
                Scope::Global => None,
                s => Some(s.clone()),
            };
            let f = Value::Func(Rc::new(Closure { def: def.clone(), this }));
            let place = match &fr.cx.self_ {
                Scope::Global => Place::Global(*name),
                s => Place::Var(s.clone(), *name),
            };
            place_set(h, fr, &place, f)?;
        }
    }
    Ok(Flow::Normal)
}

// ------------------------------------------------------------------ places (assignment targets)

enum Place {
    Local(usize),
    Global(Sym),
    Var(Scope, Sym),
    Vars(Vec<u32>, Sym),
    Elem(ArrayRef, usize),
    Field(StructRef, Sym),
}

fn resolve_place<H: Host>(h: &mut H, fr: &mut Frame, target: &Expr) -> RtResult<Place> {
    Ok(match target {
        Expr::Ident(s) => match fr.local_index(*s) {
            Some(i) => Place::Local(i),
            None => match &fr.cx.self_ {
                Scope::Global => Place::Global(*s),
                sc => Place::Var(sc.clone(), *s),
            },
        },
        Expr::Field(base, name) => {
            if let Expr::Global = **base {
                return Ok(Place::Global(*name));
            }
            let b = eval(h, fr, base)?;
            match b {
                Value::Struct(s) => Place::Field(s, *name),
                Value::Inst(id) => Place::Var(Scope::Inst(id), *name),
                Value::All | Value::Asset(AssetKind::Object, _) => Place::Vars(h.instances_of(&b), *name),
                other => return err(format!("cannot set field '{name}' on {}", other.type_name())),
            }
        }
        Expr::Index(base, idx) => {
            // auto-create arrays: `list[0] = 5` works even if `list` was unset
            let container = if matches!(**base, Expr::Ident(_) | Expr::Field(..) | Expr::Index(..)) {
                let bp = resolve_place(h, fr, base)?;
                match place_get(h, fr, &bp)? {
                    Some(v) if !v.is_null() => v,
                    _ => {
                        let a = Value::array(vec![]);
                        place_set(h, fr, &bp, a.clone())?;
                        a
                    }
                }
            } else {
                eval(h, fr, base)?
            };
            let i = eval(h, fr, idx)?;
            match container {
                Value::Array(a) => Place::Elem(a, index_of(&i)?),
                Value::Struct(s) => Place::Field(s, key_of(&i)?),
                other => return err(format!("cannot index into {}", other.type_name())),
            }
        }
        _ => return err("invalid assignment target"),
    })
}

fn scope_get<H: Host>(h: &mut H, sc: &Scope, name: Sym) -> RtResult<Option<Value>> {
    match sc {
        Scope::Global => Ok(h.global_get(name)),
        Scope::Inst(id) => h.inst_get(*id, name),
        Scope::Struct(s) => Ok(s.borrow().fields.get(&name).cloned()),
    }
}

fn scope_set<H: Host>(h: &mut H, sc: &Scope, name: Sym, v: Value) -> RtResult<()> {
    match sc {
        Scope::Global => {
            h.global_set(name, v);
            Ok(())
        }
        Scope::Inst(id) => h.inst_set(*id, name, v),
        Scope::Struct(s) => {
            s.borrow_mut().fields.insert(name, v);
            Ok(())
        }
    }
}

fn place_get<H: Host>(h: &mut H, fr: &mut Frame, p: &Place) -> RtResult<Option<Value>> {
    Ok(match p {
        Place::Local(i) => Some(fr.locals[*i].1.clone()),
        Place::Global(s) => h.global_get(*s),
        Place::Var(sc, s) => scope_get(h, sc, *s)?,
        Place::Vars(ids, s) => match ids.first() {
            Some(id) => h.inst_get(*id, *s)?,
            None => None,
        },
        Place::Elem(a, i) => a.borrow().get(*i).cloned(),
        Place::Field(st, s) => st.borrow().fields.get(s).cloned(),
    })
}

fn place_get_req<H: Host>(h: &mut H, fr: &mut Frame, p: &Place, target: &Expr) -> RtResult<Value> {
    match place_get(h, fr, p)? {
        Some(v) => Ok(v),
        None => err(format!("variable {} is not set yet", describe_target(target))),
    }
}

fn place_set<H: Host>(h: &mut H, fr: &mut Frame, p: &Place, v: Value) -> RtResult<()> {
    match p {
        Place::Local(i) => fr.locals[*i].1 = v,
        Place::Global(s) => h.global_set(*s, v),
        Place::Var(sc, s) => scope_set(h, sc, *s, v)?,
        Place::Vars(ids, s) => {
            for id in ids {
                h.inst_set(*id, *s, v.clone())?;
            }
        }
        Place::Elem(a, i) => {
            let mut a = a.borrow_mut();
            if *i >= a.len() {
                if *i > a.len() + 1_000_000 {
                    return err("array index too large");
                }
                a.resize(*i + 1, Value::Null);
            }
            a[*i] = v;
        }
        Place::Field(st, s) => {
            st.borrow_mut().fields.insert(*s, v);
        }
    }
    Ok(())
}

fn describe_target(e: &Expr) -> String {
    match e {
        Expr::Ident(s) => format!("'{s}'"),
        Expr::Field(b, s) => match **b {
            Expr::Global => format!("'global.{s}'"),
            _ => format!("'.{s}'"),
        },
        _ => "element".into(),
    }
}

fn index_of(v: &Value) -> RtResult<usize> {
    match v.as_num() {
        Some(n) if n >= 0.0 => Ok(n as usize),
        Some(n) => err(format!("negative array index {n}")),
        None => err(format!("array index must be a number, got {}", v.type_name())),
    }
}

fn key_of(v: &Value) -> RtResult<Sym> {
    match v {
        Value::Str(s) => Ok(intern(s)),
        other => Ok(intern(&other.to_string())),
    }
}

// ------------------------------------------------------------------ expressions

fn eval<H: Host>(h: &mut H, fr: &mut Frame, e: &Expr) -> RtResult<Value> {
    Ok(match e {
        Expr::Num(n) => Value::Num(*n),
        Expr::Str(s) => Value::Str(s.clone()),
        Expr::Bool(b) => Value::Bool(*b),
        Expr::Null => Value::Null,
        Expr::Ident(s) => read_ident(h, fr, *s)?,
        Expr::SelfE => fr.cx.self_.to_value(),
        Expr::Other => fr.cx.other.to_value(),
        Expr::All => Value::All,
        Expr::Global => return err("'global' must be followed by .name"),
        Expr::Array(items) => {
            let mut v = Vec::with_capacity(items.len());
            for i in items {
                v.push(eval(h, fr, i)?);
            }
            Value::array(v)
        }
        Expr::Struct(fields) => {
            let st: StructRef = Default::default();
            for (k, ve) in fields {
                let v = match ve {
                    Expr::Func(def) => Value::Func(Rc::new(Closure { def: def.clone(), this: Some(Scope::Struct(st.clone())) })),
                    other => eval(h, fr, other)?,
                };
                st.borrow_mut().fields.insert(*k, v);
            }
            Value::Struct(st)
        }
        Expr::Unary(op, a) => {
            let a = eval(h, fr, a)?;
            match op {
                UnOp::Neg => Value::Num(-num(&a)?),
                UnOp::Not => Value::Bool(!a.truthy()),
                UnOp::BitNot => Value::Num(!(num(&a)? as i64) as f64),
            }
        }
        Expr::Binary(op, a, b) => {
            let a = eval(h, fr, a)?;
            let b = eval(h, fr, b)?;
            binary(*op, &a, &b)?
        }
        Expr::And(a, b) => {
            if !eval(h, fr, a)?.truthy() {
                Value::Bool(false)
            } else {
                Value::Bool(eval(h, fr, b)?.truthy())
            }
        }
        Expr::Or(a, b) => {
            if eval(h, fr, a)?.truthy() {
                Value::Bool(true)
            } else {
                Value::Bool(eval(h, fr, b)?.truthy())
            }
        }
        Expr::Coalesce(a, b) => {
            let a = eval(h, fr, a)?;
            if a.is_null() {
                eval(h, fr, b)?
            } else {
                a
            }
        }
        Expr::Ternary(c, a, b) => {
            if eval(h, fr, c)?.truthy() {
                eval(h, fr, a)?
            } else {
                eval(h, fr, b)?
            }
        }
        Expr::Field(base, name) => {
            if let Expr::Global = **base {
                return match h.global_get(*name) {
                    Some(v) => Ok(v),
                    None => err(format!("global variable 'global.{name}' is not set yet")),
                };
            }
            let b = eval(h, fr, base)?;
            get_field(h, &b, *name)?
        }
        Expr::Index(base, idx) => {
            let b = eval(h, fr, base)?;
            let i = eval(h, fr, idx)?;
            index_get(&b, &i)?
        }
        Expr::Call(callee, args) => eval_call(h, fr, callee, args)?,
        Expr::Func(def) => {
            let this = match &fr.cx.self_ {
                Scope::Global => None,
                s => Some(s.clone()),
            };
            Value::Func(Rc::new(Closure { def: def.clone(), this }))
        }
    })
}

fn read_ident<H: Host>(h: &mut H, fr: &mut Frame, s: Sym) -> RtResult<Value> {
    if let Some(i) = fr.local_index(s) {
        return Ok(fr.locals[i].1.clone());
    }
    let self_ = fr.cx.self_.clone();
    if let Some(v) = scope_get(h, &self_, s)? {
        return Ok(v);
    }
    if let Some(def) = h.user_function(s) {
        return Ok(Value::Func(Rc::new(Closure { def, this: None })));
    }
    if let Some(v) = h.constant(s) {
        return Ok(v);
    }
    if h.has_native(s) {
        return Ok(Value::Native(s));
    }
    err(format!("unknown variable '{s}' (not set before reading it?)"))
}

fn get_field<H: Host>(h: &mut H, b: &Value, name: Sym) -> RtResult<Value> {
    match b {
        Value::Struct(s) => match s.borrow().fields.get(&name) {
            Some(v) => Ok(v.clone()),
            None => err(format!("struct has no field '{name}'")),
        },
        Value::Inst(id) => match h.inst_get(*id, name)? {
            Some(v) => Ok(v),
            None => err(format!("instance {id} has no variable '{name}'")),
        },
        Value::Asset(AssetKind::Object, _) | Value::All => {
            let ids = h.instances_of(b);
            match ids.first() {
                Some(id) => match h.inst_get(*id, name)? {
                    Some(v) => Ok(v),
                    None => err(format!("instance {id} has no variable '{name}'")),
                },
                None => err(format!("cannot read '.{name}': no instance of that object exists")),
            }
        }
        Value::Null => err(format!("cannot read field '{name}' of null")),
        other => err(format!("cannot read field '{name}' of {}", other.type_name())),
    }
}

fn index_get(b: &Value, i: &Value) -> RtResult<Value> {
    match b {
        Value::Array(a) => {
            let idx = index_of(i)?;
            let a = a.borrow();
            match a.get(idx) {
                Some(v) => Ok(v.clone()),
                None => err(format!("array index {idx} out of range (length {})", a.len())),
            }
        }
        Value::Struct(s) => {
            let k = key_of(i)?;
            Ok(s.borrow().fields.get(&k).cloned().unwrap_or(Value::Null))
        }
        Value::Str(s) => {
            let idx = index_of(i)?;
            match s.chars().nth(idx) {
                Some(c) => Ok(Value::string(c.to_string())),
                None => err(format!("string index {idx} out of range")),
            }
        }
        other => err(format!("cannot index into {}", other.type_name())),
    }
}

fn eval_args<H: Host>(h: &mut H, fr: &mut Frame, args: &[Expr]) -> RtResult<Vec<Value>> {
    let mut v = Vec::with_capacity(args.len());
    for a in args {
        v.push(eval(h, fr, a)?);
    }
    Ok(v)
}

fn eval_call<H: Host>(h: &mut H, fr: &mut Frame, callee: &Expr, args: &[Expr]) -> RtResult<Value> {
    match callee {
        Expr::Ident(s) => {
            let s = *s;
            if let Some(i) = fr.local_index(s) {
                let f = fr.locals[i].1.clone();
                let a = eval_args(h, fr, args)?;
                return call_with(h, &f, &fr.cx.clone(), a, None);
            }
            // instance / struct methods shadow global functions
            if !matches!(fr.cx.self_, Scope::Global) {
                let self_ = fr.cx.self_.clone();
                if let Some(f) = scope_get(h, &self_, s)? {
                    if matches!(f, Value::Func(_) | Value::Native(_)) {
                        let a = eval_args(h, fr, args)?;
                        return call_with(h, &f, &fr.cx.clone(), a, Some(self_));
                    }
                }
            }
            if let Some(def) = h.user_function(s) {
                let a = eval_args(h, fr, args)?;
                return run_def(h, &def, fr.cx.self_.clone(), fr.cx.other.clone(), a);
            }
            if h.has_native(s) {
                let a = eval_args(h, fr, args)?;
                return h.call_native(s, &fr.cx.clone(), a);
            }
            if let Some(Value::Func(c)) = h.global_get(s) {
                let a = eval_args(h, fr, args)?;
                return call_with(h, &Value::Func(c), &fr.cx.clone(), a, None);
            }
            err(format!("unknown function '{s}'"))
        }
        Expr::Field(base, name) => {
            let (f, this) = if let Expr::Global = **base {
                match h.global_get(*name) {
                    Some(v) => (v, None),
                    None => return err(format!("global function 'global.{name}' is not set")),
                }
            } else {
                let b = eval(h, fr, base)?;
                let f = get_field(h, &b, *name)?;
                let this = match &b {
                    Value::Inst(id) => Some(Scope::Inst(*id)),
                    Value::Struct(s) => Some(Scope::Struct(s.clone())),
                    Value::All | Value::Asset(AssetKind::Object, _) => h.instances_of(&b).first().map(|id| Scope::Inst(*id)),
                    _ => None,
                };
                (f, this)
            };
            let a = eval_args(h, fr, args)?;
            call_with(h, &f, &fr.cx.clone(), a, this)
        }
        other => {
            let f = eval(h, fr, other)?;
            let a = eval_args(h, fr, args)?;
            call_with(h, &f, &fr.cx.clone(), a, None)
        }
    }
}

// ------------------------------------------------------------------ operators

pub fn num(v: &Value) -> RtResult<f64> {
    match v.as_num() {
        Some(n) => Ok(n),
        None => err(format!("expected a number, got {} ({v:?})", v.type_name())),
    }
}

pub fn binary(op: BinOp, a: &Value, b: &Value) -> RtResult<Value> {
    use BinOp::*;
    Ok(match op {
        Add => match (a, b) {
            (Value::Num(x), Value::Num(y)) => Value::Num(x + y),
            (Value::Str(x), _) => Value::string(format!("{x}{b}")),
            (_, Value::Str(y)) => Value::string(format!("{a}{y}")),
            _ => Value::Num(num(a)? + num(b)?),
        },
        Sub => Value::Num(num(a)? - num(b)?),
        Mul => match (a, b) {
            (Value::Str(s), Value::Num(n)) => Value::string(s.repeat(n.max(0.0) as usize)),
            _ => Value::Num(num(a)? * num(b)?),
        },
        Div => {
            let d = num(b)?;
            if d == 0.0 {
                return err("division by zero");
            }
            Value::Num(num(a)? / d)
        }
        IDiv => {
            let d = num(b)?;
            if d == 0.0 {
                return err("division by zero");
            }
            Value::Num((num(a)? / d).trunc())
        }
        Mod => {
            let d = num(b)?;
            if d == 0.0 {
                return err("modulo by zero");
            }
            Value::Num(num(a)? % d)
        }
        Eq => Value::Bool(a.loose_eq(b)),
        Ne => Value::Bool(!a.loose_eq(b)),
        Lt | Le | Gt | Ge => {
            let ord = match (a, b) {
                (Value::Str(x), Value::Str(y)) => x.cmp(y),
                _ => {
                    let (x, y) = (num(a)?, num(b)?);
                    match x.partial_cmp(&y) {
                        Some(o) => o,
                        None => return Ok(Value::Bool(false)),
                    }
                }
            };
            Value::Bool(match op {
                Lt => ord.is_lt(),
                Le => ord.is_le(),
                Gt => ord.is_gt(),
                _ => ord.is_ge(),
            })
        }
        BitAnd => Value::Num(((num(a)? as i64) & (num(b)? as i64)) as f64),
        BitOr => Value::Num(((num(a)? as i64) | (num(b)? as i64)) as f64),
        BitXor => Value::Num(((num(a)? as i64) ^ (num(b)? as i64)) as f64),
        Shl => Value::Num(((num(a)? as i64) << (num(b)? as i64).clamp(0, 63)) as f64),
        Shr => Value::Num(((num(a)? as i64) >> (num(b)? as i64).clamp(0, 63)) as f64),
    })
}
