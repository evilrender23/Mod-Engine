//! Scripted input for automated runs: `--input "10-200:right,30:space"`.
//! `a-b:key` holds the key from step a until step b; `n:key` taps it at step n.

use modengine::input::{key_index, Input};

pub struct ScriptedInput {
    spans: Vec<(u64, u64, usize)>,
}

impl ScriptedInput {
    pub fn parse(spec: &str) -> Result<Self, String> {
        let mut spans = Vec::new();
        for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let (when, key) = part.split_once(':').ok_or_else(|| format!("bad input '{part}', expected step:key"))?;
            let k = key_index(key.trim()).ok_or_else(|| format!("unknown key '{key}'"))?;
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
        Ok(ScriptedInput { spans })
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
    }
}
