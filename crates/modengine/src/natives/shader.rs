//! Shader and blend mode functions (GameMaker's `shader_*`, `texture_set_stage`,
//! `gpu_set_blendmode`). They record commands the backend applies while drawing.

use super::{Reg, A};
use crate::draw::DrawCmd;
use crate::engine::Engine;
use crate::shader::UType;
use modscript::{AssetKind, RtError, RtResult, Value};

fn err<T>(msg: impl Into<String>) -> RtResult<T> {
    Err(RtError::new(msg))
}

/// A uniform handle from `shader_get_uniform` / `shader_get_sampler_index`.
fn handle(e: &Engine, a: &[Value], i: usize) -> RtResult<(u32, String, UType, usize)> {
    let h = A(a).int(i)?;
    if h < 0 {
        // GameMaker silently ignores -1 (uniform not found); so do we
        return Ok((u32::MAX, String::new(), UType::Float(1), 0));
    }
    let (sh, name) = e.shaders.handles.get(h as usize).cloned().ok_or_else(|| RtError::new(format!("{h} is not a uniform handle")))?;
    let u = e.shaders.list[sh as usize].uniform(&name).cloned().ok_or_else(|| RtError::new(format!("uniform '{name}' not found")))?;
    Ok((sh, name, u.ty, u.floats()))
}

fn set_uniform(e: &mut Engine, a: &[Value], vals: Vec<f32>, want_int: bool) -> RtResult<Value> {
    let (sh, name, ty, n) = handle(e, a, 0)?;
    if sh == u32::MAX {
        return Ok(Value::Null);
    }
    let is_int = matches!(ty, UType::Int(_));
    if ty == UType::Sampler {
        return err(format!("'{name}' is a sampler: use texture_set_stage()"));
    }
    if is_int != want_int {
        return err(format!(
            "'{name}' is {} uniform: use {}",
            if is_int { "an int" } else { "a float" },
            if is_int { "shader_set_uniform_i()" } else { "shader_set_uniform_f()" }
        ));
    }
    if vals.len() != n {
        return err(format!("uniform '{name}' takes {n} value(s), got {}", vals.len()));
    }
    e.cmds.push(DrawCmd::Uniform { shader: sh, name, vals });
    Ok(Value::Null)
}

fn nums(a: &[Value], from: usize) -> RtResult<Vec<f32>> {
    (from..a.len()).map(|i| A(a).num(i).map(|v| v as f32)).collect()
}

fn array_nums(a: &[Value], i: usize) -> RtResult<Vec<f32>> {
    let arr = A(a).array(i)?;
    let v = arr.borrow();
    v.iter().map(|x| x.as_num().map(|n| n as f32).ok_or_else(|| RtError::new("the array must contain numbers"))).collect()
}

pub(super) fn register(r: &mut Reg) {
    r.f("shader_set", |e, _, a| {
        let sh = A(a).asset(0, AssetKind::Shader)?;
        e.shaders.current = Some(sh);
        e.cmds.push(DrawCmd::Shader(Some(sh)));
        Ok(Value::Null)
    });
    r.f("shader_reset", |e, _, _| {
        e.shaders.current = None;
        e.cmds.push(DrawCmd::Shader(None));
        Ok(Value::Null)
    });
    r.f("shader_current", |e, _, _| Ok(e.shaders.current.map(|s| Value::Asset(AssetKind::Shader, s)).unwrap_or(Value::Num(-1.0))));
    r.f("shader_is_compiled", |e, _, a| {
        let sh = A(a).asset(0, AssetKind::Shader)?;
        Ok(Value::Bool(e.shaders.is_compiled(sh)))
    });
    r.f("shader_get_uniform", |e, _, a| {
        let sh = A(a).asset(0, AssetKind::Shader)?;
        let name = A(a).text(1)?;
        Ok(Value::Num(e.shaders.handle(sh, &name) as f64))
    });
    r.f("shader_get_sampler_index", |e, _, a| {
        let sh = A(a).asset(0, AssetKind::Shader)?;
        let name = A(a).text(1)?;
        let ok = e.shaders.list[sh as usize].uniform(&name).is_some_and(|u| u.ty == UType::Sampler);
        Ok(Value::Num(if ok { e.shaders.handle(sh, &name) as f64 } else { -1.0 }))
    });
    r.f("shader_set_uniform_f", |e, _, a| {
        let v = nums(a, 1)?;
        set_uniform(e, a, v, false)
    });
    r.f("shader_set_uniform_i", |e, _, a| {
        let v = nums(a, 1)?.into_iter().map(|x| x.floor()).collect();
        set_uniform(e, a, v, true)
    });
    r.f("shader_set_uniform_f_array", |e, _, a| {
        let v = array_nums(a, 1)?;
        set_uniform(e, a, v, false)
    });
    r.f("shader_set_uniform_i_array", |e, _, a| {
        let v = array_nums(a, 1)?.into_iter().map(|x| x.floor()).collect();
        set_uniform(e, a, v, true)
    });
    r.f("shader_set_uniform_matrix", |e, _, a| {
        let v = array_nums(a, 1)?;
        set_uniform(e, a, v, false)
    });
    r.f("texture_set_stage", |e, _, a| {
        let h = A(a).int(0)?;
        if h < 0 {
            return Ok(Value::Null);
        }
        let spr = A(a).asset(1, AssetKind::Sprite)?;
        let (sh, name) = e.shaders.handles.get(h as usize).cloned().ok_or_else(|| RtError::new(format!("{h} is not a sampler index")))?;
        e.cmds.push(DrawCmd::Sampler { shader: sh, name, spr });
        Ok(Value::Null)
    });
    r.f("gpu_set_blendmode", |e, _, a| {
        let m = A(a).int(0)?;
        if !(0..=3).contains(&m) {
            return err("blend mode must be bm_normal, bm_add, bm_subtract or bm_max");
        }
        e.shaders.blend = m as u8;
        e.cmds.push(DrawCmd::Blend(m as u8));
        Ok(Value::Null)
    });
    r.f("gpu_get_blendmode", |e, _, _| Ok(Value::Num(e.shaders.blend as f64)));
}
