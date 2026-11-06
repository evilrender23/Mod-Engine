//! `ui_focus_*`: focus and keyboard/gamepad-style navigation for menus. See `uifocus.rs`.

use super::{err, Reg, A};
use crate::engine::Engine;
use crate::uifocus::Dir;
use modscript::{intern, RtResult, Value};

/// Group argument `i`, or the active group (top of the stack) when omitted.
fn group(e: &Engine, a: &A, i: usize) -> RtResult<String> {
    match a.opt(i) {
        Some(v) => Ok(v.to_string()),
        None => match e.feel.focus.active() {
            Some(g) => Ok(g.to_string()),
            None => err("there is no focus group yet (ui_focus_add(group, id, x, y, w, h) creates one)"),
        },
    }
}

fn keys(v: &Value) -> RtResult<Vec<usize>> {
    let list = match v {
        Value::Array(a) => a.borrow().clone(),
        o => vec![o.clone()],
    };
    list.iter()
        .map(|k| match k.as_num() {
            Some(n) if n >= 0.0 && (n as usize) < crate::input::KEY_NAMES.len() => Ok(n as usize),
            _ => err(format!("{k} is not a key (use key.left, key.enter...)")),
        })
        .collect()
}

fn pressed(e: &Engine, ks: &[usize]) -> bool {
    ks.iter().any(|k| e.input.pressed.get(*k).copied().unwrap_or(false))
}

fn rect_value(r: [f64; 4]) -> Value {
    Value::new_struct([(intern("x"), Value::Num(r[0])), (intern("y"), Value::Num(r[1])), (intern("w"), Value::Num(r[2])), (intern("h"), Value::Num(r[3]))])
}

fn focused(e: &Engine, g: &str) -> Value {
    e.feel.focus.group(g).and_then(|g| g.focused()).map(|it| it.id.clone()).unwrap_or(Value::Null)
}

pub(super) fn register(r: &mut Reg) {
    // ui_focus_add(group, id, x, y, w, h): registers (or moves) a focusable rectangle
    r.f("ui_focus_add", |e, _, a| {
        let p = A(a);
        let g = p.text(0)?;
        let id = p.val(1)?.clone();
        let rect = [p.num(2)?, p.num(3)?, p.num(4)?.max(0.0), p.num(5)?.max(0.0)];
        e.feel.focus.add(&g, id, rect);
        Ok(Value::Null)
    });
    r.f("ui_focus_remove", |e, _, a| {
        let p = A(a);
        let g = p.text(0)?;
        e.feel.focus.remove(&g, p.val(1)?);
        Ok(Value::Null)
    });
    // ui_focus_clear([group]): forgets the rectangles of a group (or every group)
    r.f("ui_focus_clear", |e, _, a| {
        let p = A(a);
        match p.opt(0) {
            Some(v) => {
                let g = v.to_string();
                let gr = e.feel.focus.group_mut(&g);
                gr.items.clear();
                gr.focus = None;
            }
            None => e.feel.focus.clear(),
        }
        Ok(Value::Null)
    });
    r.f("ui_focus_get", |e, _, a| {
        let p = A(a);
        if p.opt(0).is_none() && e.feel.focus.active().is_none() {
            return Ok(Value::Null);
        }
        let g = group(e, &p, 0)?;
        Ok(focused(e, &g))
    });
    r.f("ui_focus_set", |e, _, a| {
        let p = A(a);
        let g = p.text(0)?;
        let id = p.val(1)?.clone();
        let gr = e.feel.focus.group_mut(&g);
        if gr.index_of(&id).is_none() {
            return Ok(Value::Bool(false));
        }
        gr.focus = Some(id);
        Ok(Value::Bool(true))
    });
    r.f("ui_focus_is", |e, _, a| {
        let p = A(a);
        let g = p.text(0)?;
        let id = p.val(1)?;
        Ok(Value::Bool(focused(e, &g).loose_eq(id) && !focused(e, &g).is_null()))
    });
    r.f("ui_focus_rect", |e, _, a| {
        let p = A(a);
        if p.opt(0).is_none() && e.feel.focus.active().is_none() {
            return Ok(Value::Null);
        }
        let g = group(e, &p, 0)?;
        Ok(e.feel.focus.group(&g).and_then(|g| g.focused()).map(|it| rect_value(it.rect)).unwrap_or(Value::Null))
    });
    // ui_focus_at(x, y, [group]): id of the rectangle under a point (mouse / touch)
    r.f("ui_focus_at", |e, _, a| {
        let p = A(a);
        let (x, y) = (p.num(0)?, p.num(1)?);
        if p.opt(2).is_none() && e.feel.focus.active().is_none() {
            return Ok(Value::Null);
        }
        let g = group(e, &p, 2)?;
        Ok(e.feel.focus.group(&g).and_then(|g| g.items.iter().rev().find(|it| x >= it.rect[0] && y >= it.rect[1] && x < it.rect[0] + it.rect[2] && y < it.rect[1] + it.rect[3])).map(|it| it.id.clone()).unwrap_or(Value::Null))
    });
    // ui_focus_move(dir, [group]): "left" | "right" | "up" | "down"; returns the new id or null
    r.f("ui_focus_move", |e, _, a| {
        let p = A(a);
        let d = p.text(0)?;
        let Some(dir) = Dir::parse(&d) else { return err(format!("'{d}' is not a direction (left, right, up, down)")) };
        let g = group(e, &p, 1)?;
        Ok(e.feel.focus.step(&g, dir).unwrap_or(Value::Null))
    });
    r.f("ui_focus_wrap", |e, _, a| {
        let p = A(a);
        let g = p.text(0)?;
        e.feel.focus.group_mut(&g).wrap = p.bool_or(1, true);
        Ok(Value::Null)
    });
    // ui_focus_update(): moves the active group's focus with the navigation keys;
    // returns the focused id (null without groups)
    r.f("ui_focus_update", |e, _, _| {
        let Some(g) = e.feel.focus.active().map(|s| s.to_string()) else { return Ok(Value::Null) };
        let k = e.feel.focus.keys.clone();
        for (ks, dir) in [(&k.left, Dir::Left), (&k.right, Dir::Right), (&k.up, Dir::Up), (&k.down, Dir::Down)] {
            if pressed(e, ks) {
                e.feel.focus.step(&g, dir);
            }
        }
        Ok(focused(e, &g))
    });
    r.f("ui_focus_confirm", |e, _, _| {
        let k = e.feel.focus.keys.confirm.clone();
        Ok(Value::Bool(e.feel.focus.active().is_some() && pressed(e, &k)))
    });
    r.f("ui_focus_back", |e, _, _| {
        let k = e.feel.focus.keys.back.clone();
        Ok(Value::Bool(pressed(e, &k)))
    });
    // ui_focus_keys({ left, right, up, down, confirm, back }): each a key or an array of keys
    r.f("ui_focus_keys", |e, _, a| {
        let p = A(a);
        let s = p.strukt(0)?;
        let mut k = e.feel.focus.keys.clone();
        for (name, v) in s.borrow().fields.iter() {
            let list = keys(v)?;
            match &*name.name() {
                "left" => k.left = list,
                "right" => k.right = list,
                "up" => k.up = list,
                "down" => k.down = list,
                "confirm" => k.confirm = list,
                "back" => k.back = list,
                other => return err(format!("ui_focus_keys(): unknown action '{other}' (left, right, up, down, confirm, back)")),
            }
        }
        e.feel.focus.keys = k;
        Ok(Value::Null)
    });
    r.f("ui_focus_push", |e, _, a| {
        let g = A(a).text(0)?;
        e.feel.focus.push(&g);
        Ok(Value::Null)
    });
    r.f("ui_focus_pop", |e, _, _| Ok(e.feel.focus.pop().map(Value::string).unwrap_or(Value::Null)));
    r.f("ui_focus_group", |e, _, _| Ok(e.feel.focus.active().map(Value::str).unwrap_or(Value::Null)));
}
