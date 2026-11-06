//! Card piles (`deck_*`): ordered lists of any values (ids, structs) with the moves a
//! deckbuilder needs: draw from the top, add at the top/bottom/a random place, shuffle
//! with a seeded `rng_create()` stream, move cards between piles and save them.
//!
//! Index 0 is the top of the pile; `deck_to_array` lists the cards from the top.

use super::{err, rand01, A, Reg};
use crate::engine::Engine;
use modscript::{intern, RtResult, Value};

#[derive(Default)]
pub struct Decks {
    list: Vec<Option<Vec<Value>>>,
}

impl Decks {
    pub fn clear(&mut self) {
        self.list.clear();
    }
    /// Live piles.
    pub fn count(&self) -> usize {
        self.list.iter().filter(|d| d.is_some()).count()
    }
}

pub const TOP: f64 = 0.0;
pub const BOTTOM: f64 = -1.0;
pub const RANDOM: f64 = -2.0;

fn handle(a: &A, i: usize) -> RtResult<usize> {
    let n = a.num(i)?;
    if n < 0.0 || n.fract() != 0.0 {
        return err(format!("{} no es un mazo de deck_create()", Value::Num(n)));
    }
    Ok(n as usize)
}

fn deck<'a>(e: &'a mut Engine, a: &A, i: usize) -> RtResult<&'a mut Vec<Value>> {
    let h = handle(a, i)?;
    match e.decks.list.get_mut(h) {
        Some(Some(d)) => Ok(d),
        _ => err(format!("{h} no es un mazo de deck_create() (¿ya destruido?)")),
    }
}

/// The random state to use: a `rng_create()` stream or the game's default one.
fn rng_state<'a>(e: &'a mut Engine, a: &A, i: usize) -> RtResult<&'a mut u64> {
    match a.opt(i) {
        None => Ok(&mut e.rng),
        Some(_) => {
            let r = a.num(i)?;
            e.rngs.state_mut(r.max(0.0) as usize).filter(|_| r >= 0.0).ok_or_else(|| modscript::RtError::new(format!("{} no es un generador de rng_create()", Value::Num(r))))
        }
    }
}

/// Where `pos` puts a card in a pile of `len` cards (`rng` only for deck_random).
fn insert_index(pos: f64, len: usize, r: Option<&mut u64>) -> RtResult<usize> {
    if pos == RANDOM {
        let r = r.ok_or_else(|| modscript::RtError::new("deck_random needs a random stream"))?;
        return Ok(((rand01(r) * (len + 1) as f64) as usize).min(len));
    }
    if pos == BOTTOM {
        return Ok(len);
    }
    if pos < 0.0 {
        return err(format!("posición {} no válida: usa deck_top, deck_bottom, deck_random o un índice desde arriba (0 = arriba)", Value::Num(pos)));
    }
    Ok((pos as usize).min(len))
}

fn put(e: &mut Engine, a: &A, deck_arg: usize, card: Value, pos_arg: usize, rng_arg: usize) -> RtResult<()> {
    let pos = a.num_or(pos_arg, BOTTOM)?;
    let len = deck(e, a, deck_arg)?.len();
    let at = if pos == RANDOM { insert_index(pos, len, Some(rng_state(e, a, rng_arg)?))? } else { insert_index(pos, len, None)? };
    deck(e, a, deck_arg)?.insert(at, card);
    Ok(())
}

fn new_deck(e: &mut Engine, cards: Vec<Value>) -> Value {
    let i = match e.decks.list.iter().position(|d| d.is_none()) {
        Some(i) => {
            e.decks.list[i] = Some(cards);
            i
        }
        None => {
            e.decks.list.push(Some(cards));
            e.decks.list.len() - 1
        }
    };
    Value::Num(i as f64)
}

fn find(d: &[Value], card: &Value) -> Option<usize> {
    d.iter().position(|c| c.loose_eq(card))
}

pub(crate) fn shuffle(v: &mut [Value], st: &mut u64) {
    for i in (1..v.len()).rev() {
        let j = (rand01(st) * (i + 1) as f64) as usize;
        v.swap(i, j.min(i));
    }
}

fn copy(v: &Value) -> Value {
    crate::datatable::deep_copy(v)
}

pub(crate) fn register(r: &mut Reg) {
    // deck_create([cards]): a new pile (the array's first card is the top)
    r.f("deck_create", |e, _, a| {
        let cards = match A(a).opt(0) {
            Some(Value::Array(arr)) => arr.borrow().clone(),
            Some(o) => return err(format!("deck_create([cartas]): se esperaba un array, no {}", o.type_name())),
            None => vec![],
        };
        Ok(new_deck(e, cards))
    });
    r.f("deck_destroy", |e, _, a| {
        let h = handle(&A(a), 0)?;
        if let Some(d) = e.decks.list.get_mut(h) {
            *d = None;
        }
        Ok(Value::Null)
    });
    r.f("deck_exists", |e, _, a| {
        let n = A(a).num(0)?;
        Ok(Value::Bool(n >= 0.0 && matches!(e.decks.list.get(n as usize), Some(Some(_)))))
    });
    r.f("deck_size", |e, _, a| Ok(Value::Num(deck(e, &A(a), 0)?.len() as f64)));
    r.f("deck_clear", |e, _, a| {
        deck(e, &A(a), 0)?.clear();
        Ok(Value::Null)
    });
    // deck_add(deck, card, [pos = deck_bottom], [rng]): pos is deck_top, deck_bottom,
    // deck_random or an index from the top
    r.f("deck_add", |e, _, a| {
        let p = A(a);
        let card = p.val(1)?.clone();
        put(e, &p, 0, card, 2, 3)?;
        Ok(Value::Null)
    });
    // deck_draw(deck): the top card (null if empty); deck_draw(deck, n): an array of up to n
    r.f("deck_draw", |e, _, a| {
        let p = A(a);
        let n = p.opt(1).map(|_| p.int(1)).transpose()?;
        let d = deck(e, &p, 0)?;
        Ok(match n {
            None => {
                if d.is_empty() {
                    Value::Null
                } else {
                    d.remove(0)
                }
            }
            Some(n) => {
                let k = (n.max(0) as usize).min(d.len());
                Value::array(d.drain(..k).collect())
            }
        })
    });
    r.f("deck_peek", |e, _, a| {
        let p = A(a);
        let i = p.num_or(1, 0.0)?;
        let d = deck(e, &p, 0)?;
        let idx = if i < 0.0 { d.len() as f64 + i } else { i };
        Ok(if idx >= 0.0 { d.get(idx as usize).cloned().unwrap_or(Value::Null) } else { Value::Null })
    });
    r.f("deck_find", |e, _, a| {
        let p = A(a);
        let card = p.val(1)?.clone();
        Ok(Value::Num(find(deck(e, &p, 0)?, &card).map(|i| i as f64).unwrap_or(-1.0)))
    });
    // deck_remove(deck, card): removes the first copy, true if there was one
    r.f("deck_remove", |e, _, a| {
        let p = A(a);
        let card = p.val(1)?.clone();
        let d = deck(e, &p, 0)?;
        Ok(Value::Bool(match find(d, &card) {
            Some(i) => {
                d.remove(i);
                true
            }
            None => false,
        }))
    });
    // deck_remove_at(deck, index): the removed card (null if out of range)
    r.f("deck_remove_at", |e, _, a| {
        let p = A(a);
        let i = p.int(1)?;
        let d = deck(e, &p, 0)?;
        let i = if i < 0 { d.len() as i64 + i } else { i };
        Ok(if i >= 0 && (i as usize) < d.len() { d.remove(i as usize) } else { Value::Null })
    });
    // deck_move(from, to, [card = top card], [pos = deck_bottom], [rng]): the moved card or null
    r.f("deck_move", |e, _, a| {
        let p = A(a);
        let card = p.opt(2).cloned();
        deck(e, &p, 1)?; // check `to` before taking the card out
        let from = deck(e, &p, 0)?;
        let idx = match &card {
            None => (!from.is_empty()).then_some(0),
            Some(c) => find(from, c),
        };
        let Some(i) = idx else { return Ok(Value::Null) };
        let c = from.remove(i);
        put(e, &p, 1, c.clone(), 3, 4)?;
        Ok(c)
    });
    // deck_move_at(from, index, to, [pos], [rng])
    r.f("deck_move_at", |e, _, a| {
        let p = A(a);
        let i = p.int(1)?;
        deck(e, &p, 2)?;
        let from = deck(e, &p, 0)?;
        let i = if i < 0 { from.len() as i64 + i } else { i };
        if i < 0 || i as usize >= from.len() {
            return Ok(Value::Null);
        }
        let c = from.remove(i as usize);
        put(e, &p, 2, c.clone(), 3, 4)?;
        Ok(c)
    });
    // deck_move_all(from, to, [pos = deck_bottom]): moves every card keeping their order
    r.f("deck_move_all", |e, _, a| {
        let p = A(a);
        deck(e, &p, 1)?;
        let cards: Vec<Value> = std::mem::take(deck(e, &p, 0)?);
        let n = cards.len();
        let pos = p.num_or(2, BOTTOM)?;
        let to = deck(e, &p, 1)?;
        if pos == RANDOM {
            return err("deck_move_all no admite deck_random: mueve y luego usa deck_shuffle");
        }
        let at = insert_index(pos, to.len(), None)?;
        to.splice(at..at, cards);
        Ok(Value::Num(n as f64))
    });
    // deck_shuffle(deck, [rng]): Fisher-Yates with a rng_create() stream (same order as
    // rng_shuffle) or the default random numbers
    r.f("deck_shuffle", |e, _, a| {
        let p = A(a);
        let mut cards = std::mem::take(deck(e, &p, 0)?);
        let res = rng_state(e, &p, 1).map(|st| shuffle(&mut cards, st));
        *deck(e, &p, 0)? = cards;
        res?;
        Ok(Value::Null)
    });
    r.f("deck_to_array", |e, _, a| Ok(Value::array(deck(e, &A(a), 0)?.clone())));
    // deck_save(deck): a copy of the cards for save_data / to_json
    r.f("deck_save", |e, _, a| Ok(Value::array(deck(e, &A(a), 0)?.iter().map(copy).collect())));
    // deck_load(cards, [deck]): fills `deck` (or a new pile) with saved cards
    r.f("deck_load", |e, _, a| {
        let p = A(a);
        let cards: Vec<Value> = p.array(0)?.borrow().iter().map(copy).collect();
        match p.opt(1) {
            Some(_) => {
                *deck(e, &p, 1)? = cards;
                Ok(p.val(1)?.clone())
            }
            None => Ok(new_deck(e, cards)),
        }
    });
}

pub(crate) fn constants(c: &mut std::collections::HashMap<modscript::Sym, Value>) {
    c.insert(intern("deck_top"), Value::Num(TOP));
    c.insert(intern("deck_bottom"), Value::Num(BOTTOM));
    c.insert(intern("deck_random"), Value::Num(RANDOM));
}
