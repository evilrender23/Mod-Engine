//! # ModScript
//!
//! A small dynamically typed, C-like scripting language designed for writing
//! game objects: event blocks, instance variables, `with`, `self`/`other`.
//!
//! The crate is engine-agnostic: a game engine implements [`Host`] to expose
//! instances, assets and built-in functions.

pub mod ast;
pub mod interp;
pub mod lexer;
pub mod parser;
pub mod sym;
pub mod value;

pub use ast::{EventDecl, Expr, FileAst, FuncDef, ObjectDecl, Stmt, UnOp, VarDecl, VarType};
pub use interp::{call_value, eval_expr, run_def, run_stmts, CallCx, Host, RtError, RtResult, Scope};
pub use parser::{parse_expr, parse_file, parse_stmts, parse_var_type, CompileError};
pub use sym::{intern, Sym};
pub use value::{fmt_num, ArrayRef, AssetKind, Closure, StructObj, StructRef, Value};

#[cfg(test)]
mod tests;
