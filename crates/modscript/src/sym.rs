//! String interning. Every identifier in a script becomes a `Sym` (a small integer),
//! so variable lookups compare integers instead of strings.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Sym(pub u32);

#[derive(Default)]
struct Interner {
    map: HashMap<Rc<str>, Sym>,
    names: Vec<Rc<str>>,
}

thread_local! {
    static INTERNER: RefCell<Interner> = RefCell::new(Interner::default());
}

/// Returns the symbol for `s`, creating it if needed.
pub fn intern(s: &str) -> Sym {
    INTERNER.with(|i| {
        let mut i = i.borrow_mut();
        if let Some(&sym) = i.map.get(s) {
            return sym;
        }
        let rc: Rc<str> = Rc::from(s);
        let sym = Sym(i.names.len() as u32);
        i.names.push(rc.clone());
        i.map.insert(rc, sym);
        sym
    })
}

impl Sym {
    pub fn name(self) -> Rc<str> {
        INTERNER.with(|i| i.borrow().names[self.0 as usize].clone())
    }
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for Sym {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name())
    }
}

impl fmt::Debug for Sym {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "`{}`", self.name())
    }
}
