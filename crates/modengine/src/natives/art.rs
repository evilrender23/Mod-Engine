//! Sprite look at run time: `sprite_set_rotation` / `sprite_get_rotation` (RotSprite,
//! nearest or smooth rotation, angle steps; see `rotsprite.rs`).

use super::{err, Reg, A};
use crate::rotsprite::RotMode;
use modscript::{AssetKind, Value};

pub(crate) fn register(r: &mut Reg) {
    r.f("sprite_set_rotation", |e, _, a| {
        // sprite_set_rotation(sprite, "nearest" | "rotsprite" | "smooth", [steps])
        let p = A(a);
        let s = p.asset(0, AssetKind::Sprite)?;
        let mode = match RotMode::parse(&p.text(1)?) {
            Ok(m) => m,
            Err(msg) => return err(format!("sprite_set_rotation: {msg}")),
        };
        if mode == RotMode::Smooth && e.sprites[s as usize].rotation.mode != RotMode::Smooth {
            // the texture filter is chosen when the game starts
            return err("sprite_set_rotation: \"smooth\" only works from the sprite's TOML (rotation = \"smooth\")");
        }
        let steps = p.num_or(2, e.sprites[s as usize].rotation.steps as f64)?;
        let r = &mut e.sprites[s as usize].rotation;
        r.mode = mode;
        r.steps = steps.clamp(0.0, 3600.0) as u32;
        Ok(Value::Null)
    });
    r.f("sprite_get_rotation", |e, _, a| {
        let s = A(a).asset(0, AssetKind::Sprite)?;
        Ok(Value::str(e.sprites[s as usize].rotation.mode.name()))
    });
    r.f("sprite_get_rotation_steps", |e, _, a| {
        let s = A(a).asset(0, AssetKind::Sprite)?;
        Ok(Value::Num(e.sprites[s as usize].rotation.steps as f64))
    });
}
