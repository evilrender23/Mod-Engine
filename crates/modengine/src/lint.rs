//! Static analysis for `modengine check`: finds unknown variables, functions, methods
//! and assets, and instance references used without `exists()`, without running the game.
//!
//! It is deliberately lenient (few false positives): a name counts as known if it is
//! assigned *anywhere* in the project (any object, `with` block, struct field...), so
//! what it reports is almost always a typo or a leftover.
//!
//! Errors are things that fail at runtime when the line runs (calling a function or
//! method that exists nowhere, `super.m()` with no parent method); warnings are probable
//! bugs (reading a variable nobody sets, stored references used unguarded).

use crate::engine::Engine;
use crate::instance::BUILTIN_VARS;
use modscript::ast::{AssignOp, Expr, FuncDef, Stmt, StmtKind};
use modscript::{intern, AssetKind, Sym, Value};
use crate::object::Ev;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub severity: Severity,
    pub file: String,
    pub line: u32,
    pub msg: String,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "aviso",
        };
        write!(f, "{}:{}: {s}: {}", self.file, self.line, self.msg)
    }
}

/// Runs every static check on a loaded project. Findings are sorted by file and line.
pub fn check(e: &Engine) -> Vec<Finding> {
    let mut l = Lint::new(e);
    l.pass = Pass::Collect;
    l.walk_all();
    l.pass = Pass::Check;
    l.walk_all();
    let mut out = l.out;
    out.sort_by(|a, b| (&a.file, a.line, a.severity, &a.msg).cmp(&(&b.file, b.line, b.severity, &b.msg)));
    out.dedup();
    out
}

/// Functions whose result is an instance that may be `noone` or die later.
fn is_query(name: &str) -> bool {
    matches!(name, "nearest" | "furthest" | "find" | "inst_at" | "collider_at" | "instance_place" | "instance_position" | "instance_nearest" | "instance_find")
        || (name.starts_with("collision_") && !name.ends_with("_list"))
}

fn is_destroy(name: &str) -> bool {
    matches!(name, "destroy" | "instance_destroy")
}

fn is_exists(name: &str) -> bool {
    matches!(name, "exists" | "instance_exists")
}

#[derive(PartialEq, Clone, Copy)]
enum Pass {
    Collect,
    Check,
}

/// What `self` is in the code being walked.
#[derive(Clone, Copy, PartialEq)]
enum Owner {
    Global,
    /// A script function: called from anywhere.
    Script,
    Object(u32),
}

struct Lint<'a> {
    e: &'a Engine,
    pass: Pass,
    out: Vec<Finding>,
    // ---- collected
    /// Plain names assigned anywhere (instance variables, globals, methods).
    assigned: HashSet<Sym>,
    /// Field names: `a.b = ...`, struct literal keys, `with` assignments, spawn vars...
    fields: HashSet<Sym>,
    /// `global.x = ...` and plain assignments in global code.
    globals: HashSet<Sym>,
    /// Variables each object's own code assigns (not inside `with`).
    obj_vars: Vec<HashSet<Sym>>,
    /// Instance variables per object that store a query result (nearest(), find()...),
    /// with the object searched.
    query_refs: Vec<HashMap<Sym, u32>>,
    /// Objects whose own code destroys the instance (`destroy()`).
    self_destroy: HashSet<u32>,
    /// Objects destroyed by name: `destroy(Obj)`, `with Obj destroy()`, `destroy(other)` in collision(Obj).
    named_destroy: HashSet<u32>,
    /// Event being walked (for `destroy(other)` in collision events).
    cur_event: Option<Ev>,
    /// Object targets of the `with` blocks being walked (None: not an object name).
    with_targets: Vec<Option<u32>>,
    builtins: HashSet<Sym>,
    specials: HashSet<Sym>,
    // ---- walking state
    file: Rc<str>,
    line: u32,
    owner: Owner,
    /// Locals of the functions being walked (innermost last): closures see outer ones.
    scopes: Vec<HashSet<Sym>>,
    with_depth: u32,
    /// Names destroyed earlier in the current block chain (`destroy(v)`).
    dead: Vec<Sym>,
    /// Names mentioned in conditions / exists() of each function being walked.
    guarded: Vec<HashSet<Sym>>,
    /// Locals of each function being walked that hold a query result (`var j = nearest(P)`).
    local_queries: Vec<HashMap<Sym, u32>>,
}

impl<'a> Lint<'a> {
    fn new(e: &'a Engine) -> Lint<'a> {
        let mut specials: HashSet<Sym> = ["argument", "argument_count", "async_load", "mouse_x", "mouse_y", "room", "room_width", "room_height", "inherited"]
            .iter()
            .map(|s| intern(s))
            .collect();
        for i in 0..16 {
            specials.insert(intern(&format!("argument{i}")));
        }
        let mut fields = HashSet::new();
        for r in &e.rooms {
            for s in &r.spawns {
                fields.extend(s.vars.iter().map(|(k, _)| *k));
            }
        }
        Lint {
            e,
            pass: Pass::Collect,
            out: vec![],
            assigned: HashSet::new(),
            fields,
            globals: HashSet::new(),
            obj_vars: vec![HashSet::new(); e.objects.len()],
            query_refs: vec![HashMap::new(); e.objects.len()],
            self_destroy: HashSet::new(),
            named_destroy: HashSet::new(),
            cur_event: None,
            with_targets: vec![],
            builtins: BUILTIN_VARS.iter().map(|(n, _)| intern(n)).collect(),
            specials,
            file: Rc::from(""),
            line: 0,
            owner: Owner::Global,
            scopes: vec![],
            with_depth: 0,
            dead: vec![],
            guarded: vec![],
            local_queries: vec![],
        }
    }

    fn walk_all(&mut self) {
        let e = self.e;
        for (i, o) in e.objects.iter().enumerate() {
            self.file = o.file.clone();
            self.owner = Owner::Object(i as u32);
            // object body: instance scope, no locals
            self.scopes = vec![HashSet::new()];
            self.guarded = vec![HashSet::new()];
            self.local_queries = vec![HashMap::new()];
            self.dead.clear();
            self.cur_event = None;
            self.block(&o.props);
            let mut evs: Vec<(&Ev, &Rc<FuncDef>)> = o.own_events.iter().collect();
            evs.sort_by_key(|(_, d)| d.line);
            for (ev, d) in evs {
                self.scopes.clear();
                self.guarded.clear();
                self.cur_event = Some(*ev);
                self.func(d);
            }
            self.cur_event = None;
        }
        let mut fs: Vec<&Rc<FuncDef>> = e.functions.values().collect();
        fs.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
        for d in fs {
            self.file = d.file.clone();
            self.owner = Owner::Script;
            self.scopes.clear();
            self.guarded.clear();
            self.func(d);
        }
        for (file, code) in &e.init_code {
            self.global_code(file.clone(), code);
        }
        for r in &e.rooms {
            if let Some(code) = &r.code {
                self.global_code(r.file.clone(), code);
            }
        }
    }

    fn global_code(&mut self, file: Rc<str>, code: &[Stmt]) {
        self.file = file;
        self.owner = Owner::Global;
        let mut locals = HashSet::new();
        declared(code, &mut locals);
        let mut g = HashSet::new();
        guards_in(code, &mut g);
        self.scopes = vec![locals];
        self.guarded = vec![g];
        self.local_queries = vec![HashMap::new()];
        self.dead.clear();
        self.block(code);
    }

    fn func(&mut self, d: &Rc<FuncDef>) {
        let mut locals: HashSet<Sym> = d.params.iter().map(|p| p.name).collect();
        declared(&d.body, &mut locals);
        let mut g = HashSet::new();
        guards_in(&d.body, &mut g);
        self.scopes.push(locals);
        self.guarded.push(g);
        self.local_queries.push(HashMap::new());
        let dead = std::mem::take(&mut self.dead);
        let with = std::mem::replace(&mut self.with_depth, 0);
        let line = self.line;
        for p in &d.params {
            if let Some(x) = &p.default {
                self.expr_at(x, d.line);
            }
        }
        self.block(&d.body);
        self.line = line;
        self.with_depth = with;
        self.dead = dead;
        self.local_queries.pop();
        self.guarded.pop();
        self.scopes.pop();
    }

    fn is_local(&self, s: Sym) -> bool {
        self.scopes.iter().any(|sc| sc.contains(&s))
    }

    fn report(&mut self, severity: Severity, line: u32, msg: String) {
        if self.pass == Pass::Check {
            self.out.push(Finding { severity, file: self.file.to_string(), line, msg });
        }
    }

    // ------------------------------------------------------------ collecting

    fn note_assign(&mut self, s: Sym) {
        if self.pass != Pass::Collect || self.is_local(s) {
            return;
        }
        self.assigned.insert(s);
        if self.with_depth > 0 {
            self.fields.insert(s);
            return;
        }
        match self.owner {
            Owner::Global => {
                self.globals.insert(s);
            }
            Owner::Script => {
                self.fields.insert(s);
            }
            Owner::Object(o) => {
                self.obj_vars[o as usize].insert(s);
            }
        }
    }

    // ------------------------------------------------------------ statements

    fn block(&mut self, b: &[Stmt]) {
        let mark = self.dead.len();
        for s in b {
            self.stmt(s);
            if let Some(v) = destroyed_target(s) {
                self.dead.push(v);
            }
        }
        self.dead.truncate(mark);
    }

    fn stmt(&mut self, s: &Stmt) {
        let line = s.line;
        self.line = line;
        match &s.kind {
            StmtKind::Expr(e) => self.expr_at(e, line),
            StmtKind::Var(d) => {
                for (n, init) in d {
                    self.dead.retain(|d| d != n);
                    let target = init.as_ref().and_then(|x| self.query_target(x));
                    if let Some(q) = self.local_queries.last_mut() {
                        match target {
                            Some(o) => q.insert(*n, o),
                            None => q.remove(n),
                        };
                    }
                    if let Some(x) = init {
                        self.expr_at(x, line);
                    }
                }
            }
            StmtKind::Assign(t, op, v) => {
                if let Expr::Ident(n) = t {
                    self.dead.retain(|d| d != n);
                    let target = self.query_target(v);
                    if self.scopes.last().is_some_and(|s| s.contains(n)) {
                        if let Some(q) = self.local_queries.last_mut() {
                            match target {
                                Some(o) => {
                                    q.insert(*n, o);
                                }
                                None if *op == AssignOp::Set => {
                                    q.remove(n);
                                }
                                None => {}
                            }
                        }
                    }
                    if let (Owner::Object(o), Pass::Collect, false, Some(t)) = (self.owner, self.pass, self.is_local(*n), target) {
                        self.query_refs[o as usize].insert(*n, t);
                    }
                }
                self.target(t, line, *op != AssignOp::Set);
                self.expr_at(v, line);
            }
            StmtKind::Incr(t, _) => self.target(t, line, true),
            StmtKind::If(c, t, e) => {
                self.expr_at(c, line);
                self.stmt(t);
                if let Some(e) = e {
                    self.stmt(e);
                }
            }
            StmtKind::While(c, b) | StmtKind::Repeat(c, b) => {
                self.expr_at(c, line);
                self.stmt(b);
            }
            StmtKind::With(c, b) => {
                self.expr_at(c, line);
                let t = match c {
                    Expr::Ident(n) => self.object_named(*n),
                    _ => None,
                };
                self.with_targets.push(t);
                self.with_depth += 1;
                self.stmt(b);
                self.with_depth -= 1;
                self.with_targets.pop();
            }
            StmtKind::DoUntil(b, c) => {
                self.stmt(b);
                self.expr_at(c, line);
            }
            StmtKind::For(i, c, st, b) => {
                if let Some(i) = i {
                    self.stmt(i);
                }
                if let Some(c) = c {
                    self.expr_at(c, line);
                }
                if let Some(st) = st {
                    self.stmt(st);
                }
                self.stmt(b);
            }
            StmtKind::ForIn(_, _, c, b) => {
                self.expr_at(c, line);
                self.stmt(b);
            }
            StmtKind::Switch(v, cases) => {
                self.expr_at(v, line);
                for c in cases {
                    if let Some(x) = &c.value {
                        self.expr_at(x, line);
                    }
                    self.block(&c.body);
                }
            }
            StmtKind::Block(b) => self.block(b),
            StmtKind::Return(Some(x)) => self.expr_at(x, line),
            StmtKind::FuncDecl(n, d) => {
                self.note_assign(*n);
                if self.pass == Pass::Collect {
                    self.fields.insert(*n);
                }
                self.func(d);
            }
            StmtKind::Break | StmtKind::Continue | StmtKind::Return(None) | StmtKind::Exit => {}
        }
    }

    /// An assignment target; `reads` for `+=` / `++` (the old value is read too).
    fn target(&mut self, t: &Expr, line: u32, reads: bool) {
        match t {
            Expr::Ident(s) => {
                // `x += 1` / `x++` alone never creates `x`: it must be set somewhere first
                if reads {
                    self.ident(*s, line);
                } else {
                    self.note_assign(*s);
                }
            }
            Expr::Field(b, name) => {
                if self.pass == Pass::Collect {
                    if let Expr::Global = **b {
                        self.globals.insert(*name);
                    }
                    self.fields.insert(*name);
                }
                if !matches!(**b, Expr::Global) {
                    self.expr_at(b, line);
                }
            }
            Expr::Index(b, i) => {
                // `list[0] = v` creates `list` if unset
                match &**b {
                    Expr::Ident(s) => self.note_assign(*s),
                    other => self.target(other, line, false),
                }
                self.expr_at(i, line);
            }
            other => self.expr_at(other, line),
        }
    }

    // ------------------------------------------------------------ expressions

    fn expr_at(&mut self, x: &Expr, line: u32) {
        self.line = line;
        self.expr(x);
    }

    fn expr(&mut self, x: &Expr) {
        let line = self.line;
        match x {
            Expr::Ident(s) => self.ident(*s, line),
            Expr::Array(v) => v.iter().for_each(|i| self.expr(i)),
            Expr::Struct(f) => {
                for (k, v) in f {
                    if self.pass == Pass::Collect {
                        self.fields.insert(*k);
                    }
                    match v {
                        // struct methods run with the struct as self: its keys are their variables
                        Expr::Func(d) => {
                            self.with_depth += 1;
                            self.func(d);
                            self.with_depth -= 1;
                        }
                        other => self.expr(other),
                    }
                }
            }
            Expr::Unary(_, a) => self.expr(a),
            Expr::Binary(_, a, b) | Expr::And(a, b) | Expr::Or(a, b) | Expr::Coalesce(a, b) | Expr::Index(a, b) => {
                self.expr(a);
                self.expr(b);
            }
            Expr::Ternary(a, b, c) => {
                self.expr(a);
                self.expr(b);
                self.expr(c);
            }
            Expr::Field(b, name) => {
                if let Expr::Global = **b {
                    if self.pass == Pass::Check && !self.globals.contains(name) && !self.assigned.contains(name) {
                        let sugg = suggest(*name, self.globals.iter().copied());
                        self.report(Severity::Warning, line, format!("variable global desconocida 'global.{name}' (nadie la asigna){sugg}"));
                    }
                    return;
                }
                self.field_use(b, *name, line, false);
                self.expr(b);
            }
            Expr::Call(f, args) => {
                self.call(f, args, line);
                for a in args {
                    self.expr(a);
                }
            }
            Expr::Func(d) => self.func(d),
            Expr::Super(name) => self.super_use(*name, line),
            Expr::Num(_) | Expr::Str(_) | Expr::Bool(_) | Expr::Null | Expr::SelfE | Expr::Other | Expr::All | Expr::Global => {}
        }
    }

    fn call(&mut self, f: &Expr, args: &[Expr], line: u32) {
        if self.pass == Pass::Collect {
            if let Expr::Ident(n) = f {
                if is_destroy(&n.name()) {
                    self.note_destroy(args.first());
                }
            }
        }
        if self.pass == Pass::Collect {
            if let Expr::Ident(n) = f {
                // set(struct, "name", v) (and GameMaker-style variable_*_set) create names
                let nn = n.name();
                if &*nn == "set" || (nn.starts_with("variable_") && nn.ends_with("_set")) {
                    for a in args {
                        if let Expr::Str(s) = a {
                            let s = intern(s);
                            self.fields.insert(s);
                            self.assigned.insert(s);
                            self.globals.insert(s);
                        }
                    }
                }
            }
        }
        match f {
            Expr::Ident(s) => {
                let s = *s;
                if self.pass != Pass::Check || self.is_local(s) || self.known(s) {
                    return;
                }
                let sugg = suggest(s, self.callable_names().into_iter());
                self.report(Severity::Error, line, format!("función o método desconocido '{s}'{sugg}"));
            }
            Expr::Field(b, name) if !matches!(**b, Expr::Global) => {
                self.field_use(b, *name, line, true);
                self.expr(b);
            }
            Expr::Super(name) => self.super_use(*name, line),
            other => self.expr(other),
        }
    }

    fn super_use(&mut self, name: Sym, line: u32) {
        if self.pass != Pass::Check {
            return;
        }
        let Owner::Object(o) = self.owner else {
            self.report(Severity::Error, line, format!("super.{name} solo funciona en métodos y eventos de un objeto"));
            return;
        };
        let e = self.e;
        let od = &e.objects[o as usize];
        let found = od.parent.is_some_and(|p| e.methods.has(&e.objects, p, name));
        if !found {
            let msg = match od.parent {
                None => format!("super.{name}: '{}' no tiene objeto padre", od.name),
                Some(p) => format!("super.{name}: ni '{}' ni sus padres definen el método '{name}'", e.objects[p as usize].name),
            };
            self.report(Severity::Error, line, msg);
        }
    }

    /// Is `s` (not a local) defined somewhere?
    fn known(&self, s: Sym) -> bool {
        let e = self.e;
        self.assigned.contains(&s)
            || self.fields.contains(&s)
            || self.globals.contains(&s)
            || self.builtins.contains(&s)
            || self.specials.contains(&s)
            || e.functions.contains_key(&s)
            || e.natives.contains_key(&s)
            || e.assets.contains_key(&s)
            || e.consts.contains_key(&s)
    }

    fn ident(&mut self, s: Sym, line: u32) {
        if self.pass != Pass::Check || self.is_local(s) || self.known(s) {
            return;
        }
        let names = self.all_names();
        let msg = match closest(s, names.into_iter()) {
            Some(c) if self.e.assets.contains_key(&c) => format!("asset o variable desconocida '{s}' (¿quizás quisiste decir '{c}'?)"),
            Some(c) => format!("variable desconocida '{s}' (nadie la asigna en el proyecto) (¿quizás quisiste decir '{c}'?)"),
            None => format!("variable desconocida '{s}' (nadie la asigna en el proyecto)"),
        };
        self.report(Severity::Warning, line, msg);
    }

    /// `base.name` read (`call` = false) or called.
    fn field_use(&mut self, base: &Expr, name: Sym, line: u32, call: bool) {
        if self.pass != Pass::Check {
            return;
        }
        let Expr::Ident(b) = base else { return };
        let b = *b;
        if self.dead.contains(&b) {
            self.report(Severity::Warning, line, format!("'{b}.{name}' después de destroy({b}): la instancia ya no recibe eventos y desaparece al acabar el paso; léelo antes de destruirla o comprueba exists({b})"));
        }
        if self.is_local(b) {
            let unguarded = !self.guarded.last().is_some_and(|g| g.contains(&b));
            let target = self.local_queries.last().and_then(|q| q.get(&b)).copied();
            if target.is_some_and(|t| self.may_die(t)) && unguarded && self.with_depth == 0 {
                self.report(
                    Severity::Warning,
                    line,
                    format!("'{b}.{name}': '{b}' viene de una búsqueda de instancias y puede ser null; compruébalo antes (`if {b} != null`, exists({b}))"),
                );
            }
            if let Some(t) = target {
                self.member_check(t, name, line, call, &format!("'{b}' (una instancia de {})", self.e.objects[t as usize].name));
            }
            return;
        }
        // stored query results (nearest(), find()...) may be null or destroyed later
        if let Owner::Object(o) = self.owner {
            let target = self.query_refs[o as usize].get(&b).copied();
            if target.is_some_and(|t| self.may_die(t)) && self.with_depth == 0 && !self.guarded.iter().any(|g| g.contains(&b)) {
                self.report(
                    Severity::Warning,
                    line,
                    format!("'{b}.{name}': '{b}' guarda el resultado de una búsqueda de instancias (puede ser null o haberse destruido); compruébalo con exists({b}) o `if {b}` en esta función"),
                );
            }
        }
        if let Owner::Object(o) = self.owner {
            if let Some(&t) = self.query_refs[o as usize].get(&b) {
                self.member_check(t, name, line, call, &format!("'{b}' (una instancia de {})", self.e.objects[t as usize].name));
                return;
            }
        }
        // ObjectName.var / ObjectName.method()
        let e = self.e;
        let Some(Value::Asset(AssetKind::Object, o)) = e.assets.get(&b) else { return };
        if self.assigned.contains(&b) {
            return; // a variable shadows the object name
        }
        let o = *o;
        self.member_check(o, name, line, call, &format!("el objeto '{}'", e.objects[o as usize].name));
    }

    /// Does an instance of `o` have variable / method `name`? `who` names it in the message.
    fn member_check(&mut self, o: u32, name: Sym, line: u32, call: bool, who: &str) {
        let e = self.e;
        if self.builtins.contains(&name) || self.fields.contains(&name) {
            return;
        }
        let fam = family(e, o);
        if fam.iter().any(|&a| self.obj_vars[a as usize].contains(&name) || e.methods.has(&e.objects, a, name)) {
            return;
        }
        let cands: Vec<Sym> = fam.iter().flat_map(|&a| self.obj_vars[a as usize].iter().copied()).collect();
        let sugg = suggest(name, tiers(&[cands]).into_iter());
        if call {
            self.report(Severity::Error, line, format!("{who} no tiene el método '{name}'{sugg}"));
        } else {
            self.report(Severity::Warning, line, format!("{who} no tiene la variable '{name}'{sugg}"));
        }
    }

    fn object_named(&self, n: Sym) -> Option<u32> {
        match self.e.assets.get(&n) {
            Some(Value::Asset(AssetKind::Object, o)) if !self.is_local(n) => Some(*o),
            _ => None,
        }
    }

    /// Object searched by a query call (`nearest(Enemy)`, `find(Boss, 0)`...): the first
    /// argument that names an object.
    fn query_target(&self, x: &Expr) -> Option<u32> {
        let Expr::Call(f, args) = x else { return None };
        let Expr::Ident(n) = &**f else { return None };
        if !is_query(&n.name()) {
            return None;
        }
        args.iter().find_map(|a| match a {
            Expr::Ident(s) => self.object_named(*s),
            _ => None,
        })
    }

    /// Records what a `destroy(...)` call can destroy.
    fn note_destroy(&mut self, arg: Option<&Expr>) {
        match arg {
            None | Some(Expr::SelfE) => {
                if let Some(Some(t)) = self.with_targets.last() {
                    self.named_destroy.insert(*t);
                } else if let (Owner::Object(o), 0) = (self.owner, self.with_depth) {
                    self.self_destroy.insert(o);
                }
            }
            Some(Expr::Ident(n)) => {
                if let Some(o) = self.object_named(*n) {
                    self.named_destroy.insert(o);
                }
            }
            Some(Expr::Other) => {
                if let Some(Ev::Collision(t)) = self.cur_event {
                    self.named_destroy.insert(t);
                }
            }
            _ => {}
        }
    }

    /// Can an instance found by searching `o` (or its children) be destroyed by the game?
    fn may_die(&self, o: u32) -> bool {
        let e = self.e;
        family(e, o).into_iter().filter(|&d| d == o || e.objects[d as usize].ancestors.contains(&o)).any(|d| {
            let chain = &e.objects[d as usize].ancestors;
            chain.iter().any(|a| self.self_destroy.contains(a) || self.named_destroy.contains(a)) || (chain.is_empty() && (self.self_destroy.contains(&d) || self.named_destroy.contains(&d)))
        })
    }

    /// Candidate names for suggestions, most likely first (the project's own names before
    /// the engine's), each group sorted so suggestions are deterministic.
    fn all_names(&self) -> Vec<Sym> {
        let e = self.e;
        tiers(&[
            self.scopes.iter().flat_map(|s| s.iter().copied()).collect(),
            self.assigned.iter().chain(&self.fields).copied().collect(),
            e.assets.keys().chain(e.functions.keys()).copied().collect(),
            self.builtins.iter().chain(e.natives.keys()).chain(e.consts.keys()).copied().collect(),
        ])
    }

    fn callable_names(&self) -> Vec<Sym> {
        let e = self.e;
        tiers(&[
            self.assigned.iter().chain(&self.fields).chain(e.functions.keys()).copied().collect(),
            e.natives.keys().copied().collect(),
        ])
    }
}

fn tiers(groups: &[Vec<Sym>]) -> Vec<Sym> {
    let mut out = Vec::new();
    for g in groups {
        let mut g: Vec<(Rc<str>, Sym)> = g.iter().map(|s| (s.name(), *s)).collect();
        g.sort();
        out.extend(g.into_iter().map(|(_, s)| s));
    }
    out
}

/// An object, its ancestors and its descendants (whose instances `Object.x` may read).
fn family(e: &Engine, o: u32) -> Vec<u32> {
    let mut v: Vec<u32> = e.objects[o as usize].ancestors.clone();
    if v.is_empty() {
        v.push(o);
    }
    for (i, d) in e.objects.iter().enumerate() {
        if d.ancestors.contains(&o) && !v.contains(&(i as u32)) {
            v.push(i as u32);
        }
    }
    v
}

/// Names a function declares with `var` or for-in, at any depth (not in nested functions).
fn declared(b: &[Stmt], out: &mut HashSet<Sym>) {
    for s in b {
        declared_stmt(s, out);
    }
}

fn declared_stmt(s: &Stmt, out: &mut HashSet<Sym>) {
    match &s.kind {
        StmtKind::Var(d) => out.extend(d.iter().map(|(n, _)| *n)),
        StmtKind::ForIn(a, b, _, body) => {
            out.insert(*a);
            if let Some(b) = b {
                out.insert(*b);
            }
            declared_stmt(body, out);
        }
        StmtKind::If(_, t, e) => {
            declared_stmt(t, out);
            if let Some(e) = e {
                declared_stmt(e, out);
            }
        }
        StmtKind::While(_, b) | StmtKind::Repeat(_, b) | StmtKind::With(_, b) | StmtKind::DoUntil(b, _) => declared_stmt(b, out),
        StmtKind::For(i, _, st, b) => {
            if let Some(i) = i {
                declared_stmt(i, out);
            }
            if let Some(st) = st {
                declared_stmt(st, out);
            }
            declared_stmt(b, out);
        }
        StmtKind::Switch(_, cases) => cases.iter().for_each(|c| declared(&c.body, out)),
        StmtKind::Block(b) => declared(b, out),
        _ => {}
    }
}

/// Names a body checks before use: any name in an `if`/`while`/ternary/`&&` condition,
/// and the arguments of `exists()` anywhere.
fn guards_in(b: &[Stmt], out: &mut HashSet<Sym>) {
    fn idents(x: &Expr, out: &mut HashSet<Sym>) {
        match x {
            Expr::Ident(s) => {
                out.insert(*s);
            }
            Expr::Unary(_, a) => idents(a, out),
            Expr::Binary(_, a, b) | Expr::And(a, b) | Expr::Or(a, b) | Expr::Coalesce(a, b) => {
                idents(a, out);
                idents(b, out);
            }
            Expr::Call(_, args) => args.iter().for_each(|a| idents(a, out)),
            _ => {}
        }
    }
    fn exprs(x: &Expr, out: &mut HashSet<Sym>) {
        match x {
            Expr::Call(f, args) => {
                if let Expr::Ident(n) = &**f {
                    if is_exists(&n.name()) {
                        args.iter().for_each(|a| idents(a, out));
                    }
                }
                args.iter().for_each(|a| exprs(a, out));
            }
            Expr::Ternary(c, a, b) => {
                idents(c, out);
                exprs(a, out);
                exprs(b, out);
            }
            Expr::And(a, b) | Expr::Or(a, b) => {
                idents(a, out);
                idents(b, out);
            }
            Expr::Unary(_, a) | Expr::Field(a, _) => exprs(a, out),
            Expr::Binary(_, a, b) | Expr::Coalesce(a, b) | Expr::Index(a, b) => {
                exprs(a, out);
                exprs(b, out);
            }
            _ => {}
        }
    }
    for s in b {
        match &s.kind {
            StmtKind::If(c, t, e) => {
                idents(c, out);
                guards_in(std::slice::from_ref(t), out);
                if let Some(e) = e {
                    guards_in(std::slice::from_ref(e), out);
                }
            }
            StmtKind::While(c, b) | StmtKind::DoUntil(b, c) => {
                idents(c, out);
                guards_in(std::slice::from_ref(b), out);
            }
            StmtKind::Repeat(_, b) | StmtKind::With(_, b) | StmtKind::ForIn(_, _, _, b) => guards_in(std::slice::from_ref(b), out),
            StmtKind::For(_, c, _, b) => {
                if let Some(c) = c {
                    idents(c, out);
                }
                guards_in(std::slice::from_ref(b), out);
            }
            StmtKind::Switch(v, cases) => {
                idents(v, out);
                cases.iter().for_each(|c| guards_in(&c.body, out));
            }
            StmtKind::Block(b) => guards_in(b, out),
            StmtKind::Expr(x) | StmtKind::Return(Some(x)) | StmtKind::Assign(_, _, x) => exprs(x, out),
            StmtKind::Var(d) => d.iter().filter_map(|(_, x)| x.as_ref()).for_each(|x| exprs(x, out)),
            _ => {}
        }
    }
}

/// `destroy(v)` / `with (v) destroy()` as a statement: the name `v`.
fn destroyed_target(s: &Stmt) -> Option<Sym> {
    let is_destroy = |f: &Expr| matches!(f, Expr::Ident(n) if is_destroy(&n.name()));
    let destroy_self = |s: &Stmt| matches!(&s.kind, StmtKind::Expr(Expr::Call(f, args)) if is_destroy(f) && args.is_empty());
    match &s.kind {
        StmtKind::Expr(Expr::Call(f, args)) if is_destroy(f) => match args.first() {
            Some(Expr::Ident(v)) => Some(*v),
            _ => None,
        },
        StmtKind::With(Expr::Ident(v), body) => match &body.kind {
            _ if destroy_self(body) => Some(*v),
            StmtKind::Block(b) if b.len() == 1 && destroy_self(&b[0]) => Some(*v),
            _ => None,
        },
        _ => None,
    }
}

// ------------------------------------------------------------------ suggestions

/// Edit distance counting a swap of two neighbours as one edit (`scroe` → `score`).
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let w = b.len() + 1;
    let mut d = vec![0usize; (a.len() + 1) * w];
    for i in 0..=a.len() {
        d[i * w] = i;
    }
    for (j, x) in d.iter_mut().enumerate().take(w) {
        *x = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let c = usize::from(a[i - 1] != b[j - 1]);
            let mut v = (d[(i - 1) * w + j] + 1).min(d[i * w + j - 1] + 1).min(d[(i - 1) * w + j - 1] + c);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(d[(i - 2) * w + j - 2] + 1);
            }
            d[i * w + j] = v;
        }
    }
    d[a.len() * w + b.len()]
}

/// Closest name to `s`, if close enough to be a typo.
fn closest(s: Sym, cands: impl Iterator<Item = Sym>) -> Option<Sym> {
    let name = s.name();
    let n = name.chars().count();
    let limit = match n {
        0..=2 => 0,
        3..=4 => 1,
        5..=8 => 2,
        _ => 3,
    };
    let lower = name.to_lowercase();
    let mut best: Option<(usize, Sym)> = None;
    let mut seen = HashSet::new();
    for c in cands {
        if c == s || !seen.insert(c) {
            continue;
        }
        let cn = c.name();
        if cn.chars().count().abs_diff(n) > limit.max(1) {
            continue;
        }
        let d = if cn.to_lowercase() == lower { 0 } else { edit_distance(&name, &cn) };
        // earlier candidates win ties (the caller orders them by likelihood)
        if d <= limit && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, c));
        }
    }
    best.map(|b| b.1)
}

fn suggest(s: Sym, cands: impl Iterator<Item = Sym>) -> String {
    match closest(s, cands) {
        Some(c) => format!(" (¿quizás quisiste decir '{c}'?)"),
        None => String::new(),
    }
}

// ------------------------------------------------------------------ asset name clashes

/// Project file that defines asset `name` of `kind`, for duplicate-name errors.
pub fn asset_origin(dir: &std::path::Path, kind: AssetKind, name: &str, scripts: &[(String, String)]) -> String {
    let folders: &[(&str, &[&str])] = match kind {
        AssetKind::Sprite => &[("sprites", &["toml", "png"])],
        AssetKind::Sound => &[("sounds", &["toml", "wav", "ogg"])],
        AssetKind::Font => &[("fonts", &["toml", "ttf", "otf"])],
        AssetKind::TileSet => &[("tilesets", &["toml"])],
        AssetKind::Room => &[("rooms", &["toml"])],
        AssetKind::Path => &[("paths", &["toml"])],
        AssetKind::Timeline => &[("timelines", &["toml"])],
        AssetKind::AnimCurve => &[("curves", &["toml"])],
        AssetKind::Sequence => &[("sequences", &["toml"])],
        AssetKind::ParticleSystem => &[("particles", &["toml"])],
        AssetKind::Shader => &[("shaders", &["toml", "fsh", "frag", "glsl"])],
        AssetKind::Object => {
            for (file, src) in scripts {
                for (i, l) in src.lines().enumerate() {
                    if let Some(rest) = l.trim_start().strip_prefix("object ") {
                        let n: String = rest.trim_start().chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                        if n == name {
                            return format!("{file}:{}", i + 1);
                        }
                    }
                }
            }
            return "un script".into();
        }
    };
    for (folder, exts) in folders {
        for ext in *exts {
            let rel = format!("{folder}/{name}.{ext}");
            if crate::vfs::is_file(&dir.join(&rel)) {
                return rel;
            }
        }
    }
    format!("{}/{name}", folders[0].0)
}

/// Message for two assets with the same name, naming both files.
pub fn clash_message(dir: &std::path::Path, name: &str, a: &Value, b: &Value, scripts: &[(String, String)]) -> String {
    let origin = |v: &Value| match v {
        Value::Asset(k, _) => format!("{} `{}`", k.name(), asset_origin(dir, *k, name, scripts)),
        other => other.type_name().to_string(),
    };
    format!("nombre de asset duplicado '{name}': {} y {} (todos los assets comparten un mismo espacio de nombres; renombra uno)", origin(a), origin(b))
}
