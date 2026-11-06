//! Scripted input for automated runs: `--input "10-200:right,30:space"`.
//! `a-b:key` holds the key from step a until step b; `n:key` taps it at step n;
//! `n:text=hola` types "hola" at step n (for `keyboard_string`; no commas inside).
//! Mouse: `n:mouse=320,200` puts the pointer at GUI (screen) coordinates from step n on,
//! `a-b:mouse=10,10>300,200` moves it in a straight line over steps a..b,
//! `mouse_room=X,Y` uses room coordinates (the camera is taken into account),
//! `a-b:mb_left` holds a button (`mb_right`, `mb_middle`; `n:mb_left` is a click) and
//! `n:wheel_up` / `n:wheel_down` turn the wheel one notch (every step of a range).

use modengine::input::{key_index, Input};

/// Where the scripted pointer goes: from `from` (or straight to `to`) over steps a..b.
#[derive(Clone, Debug, PartialEq)]
struct Move {
    a: u64,
    b: u64,
    from: Option<(f64, f64)>,
    to: (f64, f64),
    room: bool,
}

pub struct ScriptedInput {
    spans: Vec<(u64, u64, usize)>,
    texts: Vec<(u64, String)>,
    buttons: Vec<(u64, u64, usize)>,
    wheel: Vec<(u64, u64, f64)>,
    moves: Vec<Move>,
}

fn mouse_button(name: &str) -> Option<usize> {
    match name {
        "mb_left" | "mouse_left" | "click" => Some(0),
        "mb_right" | "mouse_right" => Some(1),
        "mb_middle" | "mouse_middle" => Some(2),
        _ => None,
    }
}

fn point(s: &str, part: &str) -> Result<(f64, f64), String> {
    let (x, y) = s.split_once([',', ';']).ok_or_else(|| format!("bad mouse position in '{part}' (expected mouse=X,Y)"))?;
    let num = |v: &str| v.trim().parse::<f64>().map_err(|_| format!("bad mouse position in '{part}'"));
    Ok((num(x)?, num(y)?))
}

impl ScriptedInput {
    pub fn parse(spec: &str) -> Result<Self, String> {
        let mut spans = Vec::new();
        let mut texts = Vec::new();
        let (mut buttons, mut wheel, mut moves) = (Vec::new(), Vec::new(), Vec::new());
        // `mouse=320,200` has a comma of its own: a piece without `step:` continues the last one
        let mut parts: Vec<String> = Vec::new();
        for p in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            match parts.last_mut() {
                Some(last) if !p.contains(':') => {
                    last.push(',');
                    last.push_str(p);
                }
                _ => parts.push(p.to_string()),
            }
        }
        for part in parts.iter().map(String::as_str) {
            let (when, key) = part.split_once(':').ok_or_else(|| format!("bad input '{part}', expected step:key"))?;
            if let Some(text) = key.strip_prefix("text=") {
                let n = when.trim().parse::<u64>().map_err(|_| format!("bad step in '{part}' (text needs one step: 30:text=hola)"))?;
                texts.push((n, text.to_string()));
                continue;
            }
            let key = key.trim();
            let range = || -> Result<(u64, u64), String> {
                let bad = || format!("bad step in '{part}'");
                Ok(match when.split_once('-') {
                    Some((a, b)) => {
                        let (a, b) = (a.trim().parse::<u64>().map_err(|_| bad())?, b.trim().parse::<u64>().map_err(|_| bad())?);
                        (a, b.max(a + 1))
                    }
                    None => {
                        let a = when.trim().parse::<u64>().map_err(|_| bad())?;
                        (a, a + 1)
                    }
                })
            };
            let mouse = key.strip_prefix("mouse_room=").map(|v| (v, true)).or_else(|| key.strip_prefix("mouse_gui=").or_else(|| key.strip_prefix("mouse=")).map(|v| (v, false)));
            if let Some((v, room)) = mouse {
                let (a, b) = range()?;
                let (from, to) = match v.split_once('>') {
                    Some((f, t)) => (Some(point(f, part)?), point(t, part)?),
                    None => (None, point(v, part)?),
                };
                moves.push(Move { a, b, from, to, room });
                continue;
            }
            if let Some(btn) = mouse_button(key) {
                let (a, b) = range()?;
                buttons.push((a, b, btn));
                continue;
            }
            if key == "wheel_up" || key == "wheel_down" {
                let (a, b) = range()?;
                wheel.push((a, b, if key == "wheel_up" { 1.0 } else { -1.0 }));
                continue;
            }
            let k = key_index(key).ok_or_else(|| format!("unknown key '{key}' (mouse: mouse=X,Y, mouse_room=X,Y, mb_left, mb_right, mb_middle, wheel_up, wheel_down)"))?;
            let (a, b) = match when.split_once('-') {
                Some((a, b)) => (a.trim().parse::<u64>(), b.trim().parse::<u64>()),
                None => {
                    let a = when.trim().parse::<u64>();
                    let b = a.clone().map(|a| a + 1);
                    (a, b)
                }
            };
            let (a, b) = (a.map_err(|_| format!("bad step in '{part}'"))?, b.map_err(|_| format!("bad step in '{part}'"))?);
            spans.push((a, b.max(a + 1), k));
        }
        Ok(ScriptedInput { spans, texts, buttons, wheel, moves })
    }

    /// Scripted pointer position at `step`: (x, y, in room coordinates?). None before the
    /// first `mouse=`; afterwards the pointer stays where it was last put.
    fn mouse_at(&self, step: u64) -> Option<(f64, f64, bool)> {
        let m = self.moves.iter().filter(|m| m.a <= step).max_by_key(|m| m.a)?;
        let (x, y) = match m.from {
            Some((fx, fy)) if step < m.b && m.b > m.a + 1 => {
                let t = (step - m.a) as f64 / (m.b - 1 - m.a) as f64;
                (fx + (m.to.0 - fx) * t, fy + (m.to.1 - fy) * t)
            }
            _ => m.to,
        };
        Some((x, y, m.room))
    }

    /// Like [`ScriptedInput::apply`], also placing a `mouse_room=` pointer with the camera.
    pub fn apply_to(&self, step: u64, e: &mut modengine::Engine) {
        self.apply(step, &mut e.input);
        if let Some((x, y, true)) = self.mouse_at(step) {
            e.input.mouse_x = x - e.cam.x;
            e.input.mouse_y = y - e.cam.y;
        }
    }

    /// Sets the scripted keys for logic step `step` (ORed with real input).
    pub fn apply(&self, step: u64, inp: &mut Input) {
        for &(a, b, k) in &self.spans {
            if step >= a && step < b {
                inp.down[k] = true;
            }
            if step == a {
                inp.pressed[k] = true;
            }
            if step == b {
                inp.released[k] = true;
            }
        }
        for (n, t) in &self.texts {
            if *n == step {
                inp.typed.push_str(t);
            }
        }
        for &(a, b, k) in &self.buttons {
            if step >= a && step < b {
                inp.mouse_down[k] = true;
            }
            if step == a {
                inp.mouse_pressed[k] = true;
            }
            if step == b {
                inp.mouse_released[k] = true;
            }
        }
        for &(a, b, d) in &self.wheel {
            if step >= a && step < b {
                inp.wheel += d;
            }
        }
        if let Some((x, y, false)) = self.mouse_at(step) {
            inp.mouse_x = x;
            inp.mouse_y = y;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_moves_buttons_and_wheel() {
        let si = ScriptedInput::parse("10:mouse=320,200, 20-30:mouse=0,0>90,45, 40:mouse_room=500,60, 12-15:mb_left, 16:mb_right, 18:wheel_up, 19-21:wheel_down, 5:space").unwrap();
        let at = |s: u64| {
            let mut i = Input::default();
            si.apply(s, &mut i);
            i
        };
        assert_eq!((at(5).mouse_x, at(5).pressed[key_index("space").unwrap()]), (0.0, true));
        assert_eq!((at(10).mouse_x, at(10).mouse_y), (320.0, 200.0));
        assert_eq!((at(19).mouse_x, at(19).mouse_y), (320.0, 200.0));
        assert_eq!((at(20).mouse_x, at(20).mouse_y), (0.0, 0.0));
        assert_eq!((at(25).mouse_x, at(25).mouse_y), (50.0, 25.0));
        assert_eq!((at(29).mouse_x, at(30).mouse_x, at(35).mouse_y), (90.0, 90.0, 45.0));
        assert_eq!(si.mouse_at(40), Some((500.0, 60.0, true)));
        let (b12, b14, b15) = (at(12), at(14), at(15));
        assert!(b12.mouse_pressed[0] && b12.mouse_down[0] && b14.mouse_down[0] && !b14.mouse_pressed[0]);
        assert!(!b15.mouse_down[0] && b15.mouse_released[0]);
        assert!(at(16).mouse_pressed[1] && at(17).mouse_released[1]);
        assert_eq!((at(18).wheel, at(19).wheel, at(20).wheel, at(21).wheel), (1.0, -1.0, -1.0, 0.0));
        assert!(ScriptedInput::parse("10:mouse=3").is_err());
        assert!(ScriptedInput::parse("10:mb_nope").is_err());
    }
}
