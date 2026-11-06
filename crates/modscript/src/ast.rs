//! Syntax tree produced by the parser and walked by the interpreter.

use crate::sym::Sym;
use std::rc::Rc;

#[derive(Debug)]
pub struct FuncDef {
    pub name: Sym,
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
    pub file: Rc<str>,
    pub line: u32,
    /// Names this function (or a function nested in it) reads or writes that are
    /// not its own parameters: candidates to capture from the enclosing locals.
    /// Computed on first use.
    pub free: std::cell::OnceCell<Vec<Sym>>,
}

#[derive(Debug)]
pub struct Param {
    pub name: Sym,
    pub default: Option<Expr>,
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum UnOp {
    Neg,
    Not,
    BitNot,
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    IDiv,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

#[derive(Debug)]
pub enum Expr {
    Num(f64),
    Str(Rc<str>),
    Bool(bool),
    Null,
    Ident(Sym),
    SelfE,
    Other,
    All,
    Global,
    Array(Vec<Expr>),
    Struct(Vec<(Sym, Expr)>),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Coalesce(Box<Expr>, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    Field(Box<Expr>, Sym),
    Index(Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    Func(Rc<FuncDef>),
    /// `super.name`: the parent object's version of method `name`.
    Super(Sym),
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum AssignOp {
    Set,
    Op(BinOp),
    Coalesce,
}

#[derive(Debug)]
pub struct Stmt {
    pub kind: StmtKind,
    pub line: u32,
}

#[derive(Debug)]
pub struct SwitchCase {
    pub value: Option<Expr>,
    pub body: Vec<Stmt>,
}

#[derive(Debug)]
pub enum StmtKind {
    Expr(Expr),
    Var(Vec<(Sym, Option<Expr>)>),
    Assign(Expr, AssignOp, Expr),
    Incr(Expr, f64),
    If(Expr, Box<Stmt>, Option<Box<Stmt>>),
    While(Expr, Box<Stmt>),
    DoUntil(Box<Stmt>, Expr),
    For(Option<Box<Stmt>>, Option<Expr>, Option<Box<Stmt>>, Box<Stmt>),
    Repeat(Expr, Box<Stmt>),
    /// `for (x in e)` / `for (k, v in e)`: (first name, second name, collection, body).
    ForIn(Sym, Option<Sym>, Expr, Box<Stmt>),
    Switch(Expr, Vec<SwitchCase>),
    Block(Vec<Stmt>),
    Break,
    Continue,
    Return(Option<Expr>),
    Exit,
    With(Expr, Box<Stmt>),
    FuncDecl(Sym, Rc<FuncDef>),
}

/// `on <name>(<arg>) { ... }` inside an object declaration.
#[derive(Debug)]
pub struct EventDecl {
    pub name: Sym,
    pub arg: Option<Expr>,
    pub body: Rc<FuncDef>,
    pub line: u32,
}

/// Declared type of an object variable: `speed: num(0, 10) = 2`.
#[derive(Debug, Clone, PartialEq)]
pub enum VarType {
    Num { min: Option<f64>, max: Option<f64> },
    Int { min: Option<f64>, max: Option<f64> },
    Bool,
    Str,
    /// `#rrggbb`, stored as a number.
    Color,
    Asset(crate::value::AssetKind),
    /// One of a fixed set of strings.
    List(Vec<String>),
}

impl VarType {
    /// Canonical source form, e.g. `num(0, 10)` or `list("a", "b")`.
    pub fn to_source(&self) -> String {
        use crate::value::fmt_num;
        let range = |n: &str, min: &Option<f64>, max: &Option<f64>| match (min, max) {
            (None, None) => n.to_string(),
            (a, b) => format!("{n}({}, {})", a.map(fmt_num).unwrap_or_else(|| "null".into()), b.map(fmt_num).unwrap_or_else(|| "null".into())),
        };
        match self {
            VarType::Num { min, max } => range("num", min, max),
            VarType::Int { min, max } => range("int", min, max),
            VarType::Bool => "bool".into(),
            VarType::Str => "str".into(),
            VarType::Color => "color".into(),
            VarType::Asset(k) => k.name().into(),
            VarType::List(v) => format!("list({})", v.iter().map(|s| format!("{s:?}")).collect::<Vec<_>>().join(", ")),
        }
    }

    /// `Ok(())` when `v` is a valid value of this type.
    pub fn check(&self, v: &crate::value::Value) -> Result<(), String> {
        use crate::value::{fmt_num, Value};
        let in_range = |n: f64, min: &Option<f64>, max: &Option<f64>| -> Result<(), String> {
            if min.is_some_and(|m| n < m) || max.is_some_and(|m| n > m) {
                let lo = min.map(fmt_num).unwrap_or_else(|| "-∞".into());
                let hi = max.map(fmt_num).unwrap_or_else(|| "∞".into());
                return Err(format!("{} is outside {lo}..{hi}", fmt_num(n)));
            }
            Ok(())
        };
        match (self, v) {
            (VarType::Num { min, max }, Value::Num(n)) => in_range(*n, min, max),
            (VarType::Int { min, max }, Value::Num(n)) if n.fract() == 0.0 => in_range(*n, min, max),
            (VarType::Bool, Value::Bool(_)) | (VarType::Str, Value::Str(_)) => Ok(()),
            (VarType::Color, Value::Num(n)) if *n >= 0.0 && *n <= 16777215.0 && n.fract() == 0.0 => Ok(()),
            (VarType::Asset(k), Value::Asset(k2, _)) if k == k2 => Ok(()),
            (VarType::List(opts), Value::Str(s)) if opts.iter().any(|o| **o == **s) => Ok(()),
            (VarType::List(opts), Value::Str(s)) => Err(format!("\"{s}\" is not one of {}", opts.join(", "))),
            (t, v) => Err(format!("expected {}, got {} {v}", t.to_source(), v.type_name())),
        }
    }
}

/// `name: type = value` inside an object.
#[derive(Debug, Clone)]
pub struct VarDecl {
    pub name: Sym,
    pub ty: VarType,
    pub line: u32,
}

/// `object Name : Parent { ... }`
#[derive(Debug)]
pub struct ObjectDecl {
    pub name: Sym,
    pub parent: Option<Sym>,
    /// Typed variables declared by this object (their initial values are in `props`).
    pub vars: Vec<VarDecl>,
    /// Property assignments and method declarations, run on every new instance
    /// (parents first) before its `create` event.
    pub props: Vec<Stmt>,
    pub events: Vec<EventDecl>,
    pub file: Rc<str>,
    pub line: u32,
}

#[derive(Debug, Default)]
pub struct FileAst {
    pub objects: Vec<ObjectDecl>,
    pub functions: Vec<Rc<FuncDef>>,
    /// Loose top-level statements: run once, in the global scope, when the game starts.
    pub init: Vec<Stmt>,
}
