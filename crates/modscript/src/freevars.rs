//! Free names of a function: what an anonymous function may capture from the
//! locals of the code that creates it.

use crate::ast::*;
use crate::sym::Sym;

impl FuncDef {
    /// Every name the body (including nested functions) uses as a plain identifier,
    /// minus this function's own parameters. Cached after the first call.
    pub fn free_names(&self) -> &[Sym] {
        self.free.get_or_init(|| {
            let mut out = Vec::new();
            for p in &self.params {
                if let Some(d) = &p.default {
                    expr(d, &mut out);
                }
            }
            stmts(&self.body, &mut out);
            out.retain(|s| !self.params.iter().any(|p| p.name == *s));
            out
        })
    }
}

fn add(out: &mut Vec<Sym>, s: Sym) {
    if !out.contains(&s) {
        out.push(s);
    }
}

fn stmts(b: &[Stmt], out: &mut Vec<Sym>) {
    for s in b {
        stmt(s, out);
    }
}

fn stmt(s: &Stmt, out: &mut Vec<Sym>) {
    match &s.kind {
        StmtKind::Expr(e) => expr(e, out),
        StmtKind::Var(d) => {
            for (n, e) in d {
                add(out, *n);
                if let Some(e) = e {
                    expr(e, out);
                }
            }
        }
        StmtKind::Assign(t, _, v) => {
            expr(t, out);
            expr(v, out);
        }
        StmtKind::Incr(t, _) => expr(t, out),
        StmtKind::If(c, t, e) => {
            expr(c, out);
            stmt(t, out);
            if let Some(e) = e {
                stmt(e, out);
            }
        }
        StmtKind::While(c, b) | StmtKind::Repeat(c, b) | StmtKind::With(c, b) => {
            expr(c, out);
            stmt(b, out);
        }
        StmtKind::DoUntil(b, c) => {
            stmt(b, out);
            expr(c, out);
        }
        StmtKind::For(i, c, st, b) => {
            if let Some(i) = i {
                stmt(i, out);
            }
            if let Some(c) = c {
                expr(c, out);
            }
            if let Some(st) = st {
                stmt(st, out);
            }
            stmt(b, out);
        }
        StmtKind::ForIn(a, b, c, body) => {
            add(out, *a);
            if let Some(b) = b {
                add(out, *b);
            }
            expr(c, out);
            stmt(body, out);
        }
        StmtKind::Switch(v, cases) => {
            expr(v, out);
            for c in cases {
                if let Some(e) = &c.value {
                    expr(e, out);
                }
                stmts(&c.body, out);
            }
        }
        StmtKind::Block(b) => stmts(b, out),
        StmtKind::Return(Some(e)) => expr(e, out),
        StmtKind::FuncDecl(_, d) => {
            for s in d.free_names() {
                add(out, *s);
            }
        }
        StmtKind::Break | StmtKind::Continue | StmtKind::Return(None) | StmtKind::Exit => {}
    }
}

fn expr(e: &Expr, out: &mut Vec<Sym>) {
    match e {
        Expr::Ident(s) => add(out, *s),
        Expr::Array(v) => v.iter().for_each(|x| expr(x, out)),
        Expr::Struct(f) => f.iter().for_each(|(_, x)| expr(x, out)),
        Expr::Unary(_, a) | Expr::Field(a, _) => expr(a, out),
        Expr::Binary(_, a, b) | Expr::And(a, b) | Expr::Or(a, b) | Expr::Coalesce(a, b) | Expr::Index(a, b) => {
            expr(a, out);
            expr(b, out);
        }
        Expr::Ternary(a, b, c) => {
            expr(a, out);
            expr(b, out);
            expr(c, out);
        }
        Expr::Call(f, args) => {
            expr(f, out);
            args.iter().for_each(|x| expr(x, out));
        }
        Expr::Func(d) => {
            for s in d.free_names() {
                add(out, *s);
            }
        }
        Expr::Num(_) | Expr::Str(_) | Expr::Bool(_) | Expr::Null | Expr::SelfE | Expr::Other | Expr::All | Expr::Global | Expr::Super(_) => {}
    }
}
