//! Keyboard / gamepad style focus for menus (`ui_focus_*`): rectangles registered with an
//! id in named groups, focus moved to the nearest rectangle in a direction, optional
//! wrap-around, and a stack of groups for nested panels (opening a panel pushes its group,
//! closing it pops back to the previous group with its focus intact).
//!
//! Rectangles are in whatever coordinates the game draws them (room or GUI): the module
//! only compares them with each other.

use modscript::Value;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

impl Dir {
    pub fn parse(s: &str) -> Option<Dir> {
        Some(match s {
            "left" | "izquierda" => Dir::Left,
            "right" | "derecha" => Dir::Right,
            "up" | "arriba" => Dir::Up,
            "down" | "abajo" => Dir::Down,
            _ => return None,
        })
    }
    fn vec(self) -> (f64, f64) {
        match self {
            Dir::Left => (-1.0, 0.0),
            Dir::Right => (1.0, 0.0),
            Dir::Up => (0.0, -1.0),
            Dir::Down => (0.0, 1.0),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Item {
    pub id: Value,
    /// x, y, w, h
    pub rect: [f64; 4],
}

impl Item {
    fn center(&self) -> (f64, f64) {
        (self.rect[0] + self.rect[2] / 2.0, self.rect[1] + self.rect[3] / 2.0)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Group {
    pub name: String,
    pub items: Vec<Item>,
    pub focus: Option<Value>,
    pub wrap: bool,
}

impl Group {
    pub fn index_of(&self, id: &Value) -> Option<usize> {
        self.items.iter().position(|it| it.id.loose_eq(id))
    }
    pub fn focused(&self) -> Option<&Item> {
        self.focus.as_ref().and_then(|f| self.index_of(f)).map(|i| &self.items[i])
    }

    /// Index of the item to focus when moving `dir` from the focused one.
    pub fn neighbour(&self, dir: Dir) -> Option<usize> {
        let cur = self.focus.as_ref().and_then(|f| self.index_of(f));
        let Some(ci) = cur else { return if self.items.is_empty() { None } else { Some(0) } };
        let (cx, cy) = self.items[ci].center();
        let (dx, dy) = dir.vec();
        let mut best: Option<(f64, usize)> = None;
        for (i, it) in self.items.iter().enumerate() {
            if i == ci {
                continue;
            }
            let (x, y) = it.center();
            let (vx, vy) = (x - cx, y - cy);
            let along = vx * dx + vy * dy;
            let across = (vx * dy - vy * dx).abs();
            // only inside a cone of about ±63° around the direction (a button below the
            // end of a row is not "to the right" of its last card)
            if along <= 1e-6 || across > along * 2.0 {
                continue;
            }
            // straight ahead beats diagonal: sideways distance counts double
            let score = along + across * 2.0;
            if best.is_none_or(|b| score < b.0 - 1e-9) {
                best = Some((score, i));
            }
        }
        if let Some((_, i)) = best {
            return Some(i);
        }
        if !self.wrap {
            return None;
        }
        // wrap: the item furthest the other way, among the most aligned ones
        let mut best: Option<((f64, f64), usize)> = None;
        for (i, it) in self.items.iter().enumerate() {
            if i == ci {
                continue;
            }
            let (x, y) = it.center();
            let (vx, vy) = (x - cx, y - cy);
            let along = vx * dx + vy * dy;
            let across = (vx * dy - vy * dx).abs();
            let key = ((across / 4.0).round(), along);
            if best.is_none_or(|b| key.0 < b.0 .0 || (key.0 == b.0 .0 && key.1 < b.0 .1)) {
                best = Some((key, i));
            }
        }
        best.map(|b| b.1)
    }
}

/// Key lists of each action (indices of `key.*`).
#[derive(Clone, Debug)]
pub struct Keys {
    pub left: Vec<usize>,
    pub right: Vec<usize>,
    pub up: Vec<usize>,
    pub down: Vec<usize>,
    pub confirm: Vec<usize>,
    pub back: Vec<usize>,
}

impl Default for Keys {
    fn default() -> Self {
        let k = |names: &[&str]| names.iter().filter_map(|n| crate::input::key_index(n)).collect();
        Keys {
            left: k(&["left", "a"]),
            right: k(&["right", "d"]),
            up: k(&["up", "w"]),
            down: k(&["down", "s"]),
            confirm: k(&["enter", "space", "z"]),
            back: k(&["escape", "backspace", "x"]),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Focus {
    pub groups: Vec<Group>,
    /// Active groups, the last one on top (`ui_focus_push` / `ui_focus_pop`).
    pub stack: Vec<String>,
    pub keys: Keys,
}

impl Focus {
    pub fn clear(&mut self) {
        let keys = self.keys.clone();
        *self = Focus { keys, ..Default::default() };
    }
    pub fn group(&self, name: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.name == name)
    }
    pub fn group_mut(&mut self, name: &str) -> &mut Group {
        match self.groups.iter().position(|g| g.name == name) {
            Some(i) => &mut self.groups[i],
            None => {
                self.groups.push(Group { name: name.to_string(), ..Default::default() });
                if self.stack.is_empty() {
                    self.stack.push(name.to_string());
                }
                self.groups.last_mut().unwrap()
            }
        }
    }
    /// The group keyboard navigation acts on (top of the stack).
    pub fn active(&self) -> Option<&str> {
        self.stack.last().map(|s| s.as_str())
    }
    pub fn add(&mut self, group: &str, id: Value, rect: [f64; 4]) {
        let g = self.group_mut(group);
        match g.index_of(&id) {
            Some(i) => g.items[i].rect = rect,
            None => g.items.push(Item { id: id.clone(), rect }),
        }
        if g.focus.as_ref().and_then(|f| g.index_of(f)).is_none() {
            g.focus = Some(g.items[0].id.clone());
        }
    }
    pub fn remove(&mut self, group: &str, id: &Value) {
        let g = self.group_mut(group);
        if let Some(i) = g.index_of(id) {
            g.items.remove(i);
            if g.focus.as_ref().is_some_and(|f| f.loose_eq(id)) {
                // focus the neighbour that took its place
                g.focus = g.items.get(i.min(g.items.len().saturating_sub(1))).map(|it| it.id.clone());
            }
        }
    }
    /// Moves the focus of `group`; returns the newly focused id (None: it did not move).
    pub fn step(&mut self, group: &str, dir: Dir) -> Option<Value> {
        let g = self.group_mut(group);
        let i = g.neighbour(dir)?;
        let id = g.items[i].id.clone();
        if g.focus.as_ref().is_some_and(|f| f.loose_eq(&id)) {
            return None;
        }
        g.focus = Some(id.clone());
        Some(id)
    }
    pub fn push(&mut self, group: &str) {
        self.group_mut(group);
        self.stack.retain(|g| g != group);
        self.stack.push(group.to_string());
    }
    pub fn pop(&mut self) -> Option<String> {
        if self.stack.len() <= 1 {
            return None;
        }
        self.stack.pop()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(v: f64) -> Value {
        Value::Num(v)
    }

    #[test]
    fn nearest_in_direction_and_wrap() {
        let mut f = Focus::default();
        // a row of three cards and one button below the middle one
        for (i, x) in [0.0, 40.0, 80.0].iter().enumerate() {
            f.add("hand", n(i as f64), [*x, 0.0, 30.0, 40.0]);
        }
        f.add("hand", Value::str("end"), [40.0, 80.0, 30.0, 10.0]);
        assert!(f.group("hand").unwrap().focus.as_ref().unwrap().loose_eq(&n(0.0)), "first added gets the focus");
        assert!(f.step("hand", Dir::Left).is_none(), "no wrap: stays");
        assert!(f.step("hand", Dir::Right).unwrap().loose_eq(&n(1.0)));
        assert!(f.step("hand", Dir::Down).unwrap().loose_eq(&Value::str("end")));
        assert!(f.step("hand", Dir::Up).unwrap().loose_eq(&n(1.0)), "back up to the aligned card");
        f.step("hand", Dir::Right);
        assert!(f.step("hand", Dir::Right).is_none());
        f.group_mut("hand").wrap = true;
        assert!(f.step("hand", Dir::Right).unwrap().loose_eq(&n(0.0)), "wraps to the first of the row");
    }

    #[test]
    fn stack_restores_focus() {
        let mut f = Focus::default();
        f.add("menu", n(1.0), [0.0, 0.0, 10.0, 10.0]);
        f.add("menu", n(2.0), [0.0, 20.0, 10.0, 10.0]);
        f.step("menu", Dir::Down);
        f.push("dialog");
        f.add("dialog", Value::str("yes"), [0.0, 0.0, 10.0, 10.0]);
        assert_eq!(f.active(), Some("dialog"));
        assert_eq!(f.pop().as_deref(), Some("dialog"));
        assert_eq!(f.active(), Some("menu"));
        assert!(f.group("menu").unwrap().focus.as_ref().unwrap().loose_eq(&n(2.0)));
        assert!(f.pop().is_none(), "the base group stays");
    }
}
