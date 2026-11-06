//! Object methods by owner, for `super.metodo()` and `inherited()` inside methods.
//!
//! A method is a `function name() {}` (or `name = function() {}`) in an object body.
//! Every method and event body is mapped to the object that declares it, so the parent's
//! version is found from the code that is running, not from the instance's object: in a
//! chain A → B → C, `inherited()` in B's method always reaches A's, even on a C.

use crate::engine::Engine;
use crate::object::ObjectDef;
use modscript::{Expr, FuncDef, StmtKind, SuperMethod, Sym};
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Default)]
pub struct MethodTable {
    /// Methods each object declares itself (index = object).
    own: Vec<HashMap<Sym, Rc<FuncDef>>>,
    /// Code body (by address) → (declaring object, method name or None for events).
    owner: HashMap<usize, (u32, Option<Sym>)>,
}

/// `(name, body)` of every method declared directly in an object's body.
pub fn declared_methods(props: &[modscript::Stmt]) -> Vec<(Sym, Rc<FuncDef>)> {
    let mut out = Vec::new();
    for s in props {
        match &s.kind {
            StmtKind::FuncDecl(n, d) => out.push((*n, d.clone())),
            StmtKind::Assign(Expr::Ident(n), _, Expr::Func(d)) => out.push((*n, d.clone())),
            _ => {}
        }
    }
    out
}

impl MethodTable {
    pub fn build(objects: &[ObjectDef]) -> MethodTable {
        let mut t = MethodTable::default();
        for (i, o) in objects.iter().enumerate() {
            let mut own = HashMap::new();
            for (n, d) in declared_methods(&o.props) {
                t.owner.insert(Rc::as_ptr(&d) as usize, (i as u32, Some(n)));
                own.insert(n, d);
            }
            for d in o.own_events.values() {
                t.owner.insert(Rc::as_ptr(d) as usize, (i as u32, None));
            }
            t.own.push(own);
        }
        t
    }

    /// Does object `obj` (or an ancestor) declare method `name`?
    pub fn has(&self, objects: &[ObjectDef], obj: u32, name: Sym) -> bool {
        objects[obj as usize].ancestors.iter().any(|&a| self.own.get(a as usize).is_some_and(|m| m.contains_key(&name)))
    }
}

pub fn super_method(e: &Engine, current: &Rc<FuncDef>, name: Option<Sym>) -> SuperMethod {
    let t = &e.methods;
    let Some(&(obj, method)) = t.owner.get(&(Rc::as_ptr(current) as usize)) else { return SuperMethod::NotInObject };
    let name = match (name, method) {
        (Some(n), _) => n,
        (None, Some(m)) => m,
        // inherited() in an event: the engine's event version handles it
        (None, None) => return SuperMethod::NotInObject,
    };
    let mut cur = e.objects[obj as usize].parent;
    while let Some(p) = cur {
        if let Some(d) = t.own[p as usize].get(&name) {
            return SuperMethod::Found(d.clone());
        }
        cur = e.objects[p as usize].parent;
    }
    SuperMethod::Missing
}
