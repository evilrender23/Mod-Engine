//! Extension queries.

use super::{Reg, A};
use modscript::{intern, Value};

pub(super) fn register(r: &mut Reg) {
    r.f("extension_exists", |e, _, a| {
        let n = A(a).text(0)?;
        Ok(Value::Bool(e.extension(&n).is_some_and(|x| x.meta.enabled)))
    });
    r.f("extension_get_info", |e, _, a| {
        let n = A(a).text(0)?;
        Ok(match e.extension(&n) {
            None => Value::Null,
            Some(x) => Value::new_struct([
                (intern("name"), Value::str(if x.meta.name.is_empty() { &x.name } else { &x.meta.name })),
                (intern("version"), Value::str(&x.meta.version)),
                (intern("author"), Value::str(&x.meta.author)),
                (intern("description"), Value::str(&x.meta.description)),
                (intern("enabled"), Value::Bool(x.meta.enabled)),
                (intern("files"), Value::array(x.scripts.iter().map(|s| Value::str(s)).collect())),
                (intern("functions"), Value::array(x.meta.functions.iter().map(|f| Value::str(&f.name)).collect())),
            ]),
        })
    });
}
