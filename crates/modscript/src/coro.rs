//! Coroutines: a function that can stop half-way with `yield()` / `wait(n)` and go on
//! later from the same place, with its locals intact.
//!
//! The tree-walking interpreter cannot suspend a Rust call stack, so a coroutine body is
//! run by a small resumable executor: statements that (directly or in nested blocks /
//! loops) contain a suspension call keep their position on an explicit stack; every other
//! statement is run by the normal interpreter in one go. Suspension calls are only
//! recognised as whole statements of the coroutine's own body (not inside functions it
//! calls, not inside `switch`/`with`).

use crate::ast::*;
use crate::interp::{co_bind, co_cx, co_eval, co_exec, co_frame, co_has_local, CallCx, CoFrame, Flow, Host, RtError, RtResult};
use crate::sym::{intern, Sym};
use crate::value::{AssetKind, Value};
use std::collections::HashMap;
use std::rc::Rc;

/// Names that suspend a coroutine when used as a statement of its body.
pub const SUSPEND_NAMES: [&str; 4] = ["yield", "wait", "wait_seconds", "wait_until"];

thread_local! {
    static NAMES: [Sym; 4] = SUSPEND_NAMES.map(intern);
}

/// What a coroutine did when resumed.
pub enum Resume {
    /// Stopped at `name(args...)`; the host decides when to resume it.
    Suspended(Sym, Vec<Value>),
    /// Reached the end (or `return v` / `exit`).
    Done(Value),
}

// Positions inside the body. The pointers point into the `FuncDef` kept alive by
// `Coroutine::def`; the syntax tree is never modified after parsing.
enum Cont {
    Seq { stmts: *const [Stmt], i: usize },
    While(*const Stmt),
    DoUntil(*const Stmt),
    For(*const Stmt),
    Repeat(*const Stmt, u64),
    ForIn { s: *const Stmt, items: Vec<(Value, Value)>, i: usize },
}

pub struct Coroutine {
    def: Rc<FuncDef>,
    fr: CoFrame,
    stack: Vec<Cont>,
    has_yield: HashMap<*const Stmt, bool>,
    done: bool,
}

fn suspend_name<H: Host>(h: &H, fr: &CoFrame, s: &Stmt) -> Option<Sym> {
    if let StmtKind::Expr(Expr::Call(callee, _)) = &s.kind {
        if let Expr::Ident(n) = &**callee {
            if NAMES.with(|ns| ns.contains(n)) && !co_has_local(fr, *n) && h.user_function(*n).is_none() {
                return Some(*n);
            }
        }
    }
    None
}

fn body_slice(b: &Stmt) -> *const [Stmt] {
    std::slice::from_ref(b) as *const [Stmt]
}

fn located(mut e: RtError, s: &Stmt, file: &Rc<str>) -> RtError {
    if e.line == 0 {
        e.line = s.line;
        e.file = file.clone();
    }
    e
}

impl Coroutine {
    /// Prepares `f` (a script function value) to run as a coroutine with `args`.
    /// `cx` gives `self`/`other` for unbound functions. Nothing runs until [`resume`].
    pub fn new<H: Host>(h: &mut H, f: &Value, cx: &CallCx, args: Vec<Value>) -> RtResult<Coroutine> {
        let (fr, def) = co_frame(h, f, cx, args)?;
        let stmts: *const [Stmt] = &def.body[..];
        Ok(Coroutine { stack: vec![Cont::Seq { stmts, i: 0 }], def, fr, has_yield: HashMap::new(), done: false })
    }

    pub fn is_done(&self) -> bool {
        self.done
    }

    /// `self` of the coroutine.
    pub fn cx(&self) -> &CallCx {
        co_cx(&self.fr)
    }

    pub fn name(&self) -> String {
        format!("{} ({}:{})", self.def.name, self.def.file, self.def.line)
    }

    fn contains_yield<H: Host>(&mut self, h: &H, s: &Stmt) -> bool {
        let key = s as *const Stmt;
        if let Some(&b) = self.has_yield.get(&key) {
            return b;
        }
        let b = self.scan(h, s);
        self.has_yield.insert(key, b);
        b
    }

    fn scan<H: Host>(&self, h: &H, s: &Stmt) -> bool {
        if suspend_name(h, &self.fr, s).is_some() {
            return true;
        }
        match &s.kind {
            StmtKind::Block(b) => b.iter().any(|x| self.scan(h, x)),
            StmtKind::If(_, t, e) => self.scan(h, t) || e.as_ref().is_some_and(|e| self.scan(h, e)),
            StmtKind::While(_, b) | StmtKind::DoUntil(b, _) | StmtKind::Repeat(_, b) | StmtKind::With(_, b) => self.scan(h, b),
            StmtKind::For(_, _, _, b) | StmtKind::ForIn(_, _, _, b) => self.scan(h, b),
            StmtKind::Switch(_, cases) => cases.iter().any(|c| c.body.iter().any(|x| self.scan(h, x))),
            _ => false,
        }
    }

    fn eval<H: Host>(&mut self, h: &mut H, e: &Expr, at: &Stmt) -> RtResult<Value> {
        co_eval(h, &mut self.fr, e).map_err(|err| located(err, at, &self.def.file))
    }

    /// Runs until the next suspension point or the end.
    pub fn resume<H: Host>(&mut self, h: &mut H) -> RtResult<Resume> {
        if self.done {
            return Ok(Resume::Done(Value::Null));
        }
        let r = self.run(h);
        match r {
            Ok(Resume::Done(v)) => {
                self.finish();
                Ok(Resume::Done(v))
            }
            Ok(r) => Ok(r),
            Err(mut e) => {
                self.finish();
                e.trace.push(format!("in coroutine {}", self.name()));
                Err(e)
            }
        }
    }

    fn finish(&mut self) {
        self.done = true;
        self.stack.clear();
    }

    fn run<H: Host>(&mut self, h: &mut H) -> RtResult<Resume> {
        loop {
            let Some(top) = self.stack.last_mut() else { return Ok(Resume::Done(Value::Null)) };
            let Cont::Seq { stmts, i } = top else {
                // a loop whose body just finished
                self.next_pass(h)?;
                continue;
            };
            // SAFETY: points into self.def (kept alive, never mutated)
            let list: &[Stmt] = unsafe { &**stmts };
            if *i >= list.len() {
                self.stack.pop();
                continue;
            }
            let s: &Stmt = &list[*i];
            *i += 1;
            if !self.contains_yield(h, s) {
                match co_exec(h, &mut self.fr, s)? {
                    Flow::Normal => {}
                    Flow::Break => self.unwind(s, true)?,
                    Flow::Continue => self.unwind(s, false)?,
                    Flow::Return(v) => return Ok(Resume::Done(v)),
                    Flow::Exit => return Ok(Resume::Done(Value::Null)),
                }
                continue;
            }
            if let Some(name) = suspend_name(h, &self.fr, s) {
                let StmtKind::Expr(Expr::Call(_, args)) = &s.kind else { unreachable!() };
                let mut vals = Vec::with_capacity(args.len());
                for a in args {
                    vals.push(self.eval(h, a, s)?);
                }
                return Ok(Resume::Suspended(name, vals));
            }
            self.enter(h, s)?;
        }
    }

    /// Starts a compound statement that contains a suspension point.
    fn enter<H: Host>(&mut self, h: &mut H, s: &Stmt) -> RtResult<()> {
        match &s.kind {
            StmtKind::Block(b) => self.stack.push(Cont::Seq { stmts: &b[..], i: 0 }),
            StmtKind::If(c, t, e) => {
                if self.eval(h, c, s)?.truthy() {
                    self.stack.push(Cont::Seq { stmts: body_slice(t), i: 0 });
                } else if let Some(e) = e {
                    self.stack.push(Cont::Seq { stmts: body_slice(e), i: 0 });
                }
            }
            StmtKind::While(c, b) => {
                if self.eval(h, c, s)?.truthy() {
                    self.stack.push(Cont::While(s));
                    self.stack.push(Cont::Seq { stmts: body_slice(b), i: 0 });
                }
            }
            StmtKind::DoUntil(b, _) => {
                self.stack.push(Cont::DoUntil(s));
                self.stack.push(Cont::Seq { stmts: body_slice(b), i: 0 });
            }
            StmtKind::For(init, cond, _, b) => {
                if let Some(init) = init {
                    co_exec(h, &mut self.fr, init)?;
                }
                let go = match cond {
                    Some(c) => self.eval(h, c, s)?.truthy(),
                    None => true,
                };
                if go {
                    self.stack.push(Cont::For(s));
                    self.stack.push(Cont::Seq { stmts: body_slice(b), i: 0 });
                }
            }
            StmtKind::Repeat(n, b) => {
                let v = self.eval(h, n, s)?;
                let n = v.as_num().ok_or_else(|| located(RtError::new(format!("repeat needs a number, got {}", v.type_name())), s, &self.def.file))?;
                let n = n.floor().max(0.0) as u64;
                if n > 0 {
                    self.stack.push(Cont::Repeat(s, n));
                    self.stack.push(Cont::Seq { stmts: body_slice(b), i: 0 });
                }
            }
            StmtKind::ForIn(_, _, coll, _) => {
                let c = self.eval(h, coll, s)?;
                let items: Vec<(Value, Value)> = match &c {
                    Value::Array(arr) => arr.borrow().iter().enumerate().map(|(i, v)| (Value::Num(i as f64), v.clone())).collect(),
                    Value::Struct(st) => st.borrow().fields.iter().map(|(k, v)| (Value::Str(k.name()), v.clone())).collect(),
                    Value::Str(t) => t.chars().enumerate().map(|(i, ch)| (Value::Num(i as f64), Value::string(ch.to_string()))).collect(),
                    Value::Inst(_) | Value::All | Value::Asset(AssetKind::Object, _) => {
                        h.instances_of(&c).into_iter().enumerate().map(|(i, id)| (Value::Num(i as f64), Value::Inst(id))).collect()
                    }
                    other => {
                        return Err(located(RtError::new(format!("for-in needs an array, struct, string or object, got {}", other.type_name())), s, &self.def.file))
                    }
                };
                self.stack.push(Cont::ForIn { s, items, i: 0 });
                self.bind_pass(h)?;
            }
            StmtKind::Switch(..) | StmtKind::With(..) => {
                return Err(located(
                    RtError::new("wait()/yield() no se puede usar dentro de switch o with en una corrutina (usa if, o llama a una función normal)"),
                    s,
                    &self.def.file,
                ))
            }
            _ => unreachable!("statement without suspension points"),
        }
        Ok(())
    }

    /// Binds the loop variables of the current for-in pass and pushes its body, or ends
    /// the loop. Skips instances destroyed meanwhile.
    fn bind_pass<H: Host>(&mut self, h: &mut H) -> RtResult<()> {
        loop {
            let Some(Cont::ForIn { s, items, i }) = self.stack.last_mut() else { return Ok(()) };
            // SAFETY: see Cont
            let st: &Stmt = unsafe { &**s };
            let StmtKind::ForIn(a, bname, _, body) = &st.kind else { unreachable!() };
            if *i >= items.len() {
                self.stack.pop();
                return Ok(());
            }
            let (k, v) = items[*i].clone();
            *i += 1;
            if let Value::Inst(id) = v {
                if !h.inst_alive(id) {
                    continue;
                }
            }
            // structs give (key, value); `for (x in struct)` walks the keys
            let keys_first = matches!(k, Value::Str(_));
            match bname {
                None => co_bind(&mut self.fr, *a, if keys_first { k } else { v }),
                Some(b) => {
                    co_bind(&mut self.fr, *a, k);
                    co_bind(&mut self.fr, *b, v);
                }
            }
            self.stack.push(Cont::Seq { stmts: body_slice(body), i: 0 });
            return Ok(());
        }
    }

    /// The loop on top of the stack finished one pass of its body: start the next or end.
    fn next_pass<H: Host>(&mut self, h: &mut H) -> RtResult<()> {
        let top = self.stack.last_mut().expect("loop");
        match top {
            Cont::While(s) => {
                // SAFETY: see Cont
                let st: &Stmt = unsafe { &**s };
                let StmtKind::While(c, b) = &st.kind else { unreachable!() };
                if self.eval(h, c, st)?.truthy() {
                    self.stack.push(Cont::Seq { stmts: body_slice(b), i: 0 });
                } else {
                    self.stack.pop();
                }
            }
            Cont::DoUntil(s) => {
                let st: &Stmt = unsafe { &**s };
                let StmtKind::DoUntil(b, c) = &st.kind else { unreachable!() };
                if self.eval(h, c, st)?.truthy() {
                    self.stack.pop();
                } else {
                    self.stack.push(Cont::Seq { stmts: body_slice(b), i: 0 });
                }
            }
            Cont::For(s) => {
                let st: &Stmt = unsafe { &**s };
                let StmtKind::For(_, cond, step, b) = &st.kind else { unreachable!() };
                if let Some(step) = step {
                    co_exec(h, &mut self.fr, step)?;
                }
                let go = match cond {
                    Some(c) => self.eval(h, c, st)?.truthy(),
                    None => true,
                };
                if go {
                    self.stack.push(Cont::Seq { stmts: body_slice(b), i: 0 });
                } else {
                    self.stack.pop();
                }
            }
            Cont::Repeat(s, left) => {
                *left -= 1;
                if *left == 0 {
                    self.stack.pop();
                } else {
                    let st: &Stmt = unsafe { &**s };
                    let StmtKind::Repeat(_, b) = &st.kind else { unreachable!() };
                    self.stack.push(Cont::Seq { stmts: body_slice(b), i: 0 });
                }
            }
            Cont::ForIn { .. } => self.bind_pass(h)?,
            Cont::Seq { .. } => unreachable!(),
        }
        Ok(())
    }

    /// `break` (`brk`) or `continue` ran inside a body on the stack.
    fn unwind(&mut self, at: &Stmt, brk: bool) -> RtResult<()> {
        while let Some(Cont::Seq { .. }) = self.stack.last() {
            self.stack.pop();
        }
        if self.stack.is_empty() {
            return Err(located(RtError::new("'break'/'continue' outside of a loop"), at, &self.def.file));
        }
        if brk {
            self.stack.pop();
        }
        // on continue the loop stays on top and run() starts its next pass
        Ok(())
    }
}
