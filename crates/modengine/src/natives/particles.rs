//! Particle functions (GameMaker's `part_system_*`, `part_type_*`, `part_emitter_*`,
//! `part_particles_*`). Systems, types and emitters are numbers.

use super::{Reg, A};
use crate::engine::Engine;
use crate::particles::{Distr, Emitter, PartType, RegionShape, System, Tex, SHAPE_NAMES};
use modscript::{AssetKind, RtError, RtResult, Value};

fn err<T>(msg: impl Into<String>) -> RtResult<T> {
    Err(RtError::new(msg))
}

fn rgb(v: f64) -> u32 {
    (v.max(0.0) as u32) & 0xFF_FFFF
}

/// System id argument.
fn sys(e: &mut Engine, a: &[Value], i: usize) -> RtResult<usize> {
    let id = A(a).int(i)?;
    if id >= 0 && e.parts.world.sys(id as usize).is_some() {
        Ok(id as usize)
    } else {
        err(format!("particle system {id} does not exist"))
    }
}

/// Particle type argument, mutable.
fn ty<'a>(e: &'a mut Engine, a: &[Value], i: usize) -> RtResult<&'a mut PartType> {
    let id = A(a).int(i)?;
    if id < 0 {
        return err(format!("particle type {id} does not exist"));
    }
    e.parts.world.type_get(id as usize).ok_or_else(|| RtError::new(format!("particle type {id} does not exist")))
}

fn ty_id(e: &mut Engine, a: &[Value], i: usize) -> RtResult<usize> {
    ty(e, a, i)?;
    Ok(A(a).int(i)? as usize)
}

/// (system, emitter) arguments.
fn em<'a>(e: &'a mut Engine, a: &[Value], i: usize) -> RtResult<&'a mut Emitter> {
    let s = sys(e, a, i)?;
    let k = A(a).int(i + 1)?;
    e.parts
        .world
        .sys(s)
        .and_then(|s| if k >= 0 { s.emitters.get_mut(k as usize).and_then(|x| x.as_mut()) } else { None })
        .ok_or_else(|| RtError::new(format!("emitter {k} does not exist in particle system {s}")))
}

/// Applies `f` to a particle type and returns null.
fn set_ty(e: &mut Engine, a: &[Value], f: impl FnOnce(&mut PartType, &A) -> RtResult<()>) -> RtResult<Value> {
    let p = A(a);
    let t = ty(e, a, 0)?;
    f(t, &p)?;
    Ok(Value::Null)
}

fn new_system(e: &mut Engine, a: &[Value], first: usize) -> RtResult<usize> {
    match A(a).opt_asset(first, AssetKind::ParticleSystem)? {
        Some(asset) => {
            let sprites = &e.sprites;
            let lookup = |n: &str| sprites.iter().position(|s| s.name == n).map(|i| i as u32);
            e.parts.world.system_from_asset(asset as usize, 0.0, 0.0, &lookup).map_err(RtError::new)
        }
        None => Ok(e.parts.world.system_create(System::new())),
    }
}

pub(super) fn register(r: &mut Reg) {
    // ---------------------------------------------------------------- systems
    r.f("part_system_create", |e, _, a| Ok(Value::Num(new_system(e, a, 0)? as f64)));
    r.f("part_system_create_layer", |e, _, a| {
        let layer = A(a).text(0)?;
        let depth = match e.layer_index(Some(&layer)) {
            Some(li) => e.layers[li].depth,
            None => return err(format!("there is no layer named '{layer}'")),
        };
        let s = new_system(e, a, 2)?;
        e.parts.world.sys(s).unwrap().depth = depth;
        Ok(Value::Num(s as f64))
    });
    r.f("part_system_destroy", |e, _, a| {
        let s = sys(e, a, 0)?;
        e.parts.world.system_destroy(s);
        Ok(Value::Null)
    });
    r.f("part_system_exists", |e, _, a| {
        let id = A(a).int(0)?;
        Ok(Value::Bool(id >= 0 && e.parts.world.sys(id as usize).is_some()))
    });
    r.f("part_system_clear", |e, _, a| {
        let s = sys(e, a, 0)?;
        let s = e.parts.world.sys(s).unwrap();
        s.parts.clear();
        s.emitters.clear();
        Ok(Value::Null)
    });
    r.f("part_system_position", |e, _, a| {
        let (x, y) = (A(a).num(1)?, A(a).num(2)?);
        let s = sys(e, a, 0)?;
        let s = e.parts.world.sys(s).unwrap();
        s.x = x;
        s.y = y;
        Ok(Value::Null)
    });
    r.f("part_system_depth", |e, _, a| {
        let d = A(a).num(1)?;
        let s = sys(e, a, 0)?;
        e.parts.world.sys(s).unwrap().depth = d;
        Ok(Value::Null)
    });
    r.f("part_system_layer", |e, _, a| {
        let layer = A(a).text(1)?;
        let li = e.layer_index(Some(&layer)).ok_or_else(|| RtError::new(format!("there is no layer named '{layer}'")))?;
        let d = e.layers[li].depth;
        let s = sys(e, a, 0)?;
        e.parts.world.sys(s).unwrap().depth = d;
        Ok(Value::Null)
    });
    r.f("part_system_angle", |e, _, a| {
        let v = A(a).num(1)?;
        let s = sys(e, a, 0)?;
        e.parts.world.sys(s).unwrap().angle = v;
        Ok(Value::Null)
    });
    fn sys_color(e: &mut Engine, _: &modscript::CallCx, a: &[Value]) -> RtResult<Value> {
        let (c, al) = (rgb(A(a).num(1)?), A(a).num_or(2, 1.0)?);
        let s = sys(e, a, 0)?;
        let s = e.parts.world.sys(s).unwrap();
        s.tint = c;
        s.alpha = al;
        Ok(Value::Null)
    }
    r.f("part_system_color", sys_color);
    r.f("part_system_colour", sys_color);
    r.f("part_system_automatic_update", |e, _, a| {
        let v = A(a).bool_or(1, true);
        let s = sys(e, a, 0)?;
        e.parts.world.sys(s).unwrap().auto_update = v;
        Ok(Value::Null)
    });
    r.f("part_system_automatic_draw", |e, _, a| {
        let v = A(a).bool_or(1, true);
        let s = sys(e, a, 0)?;
        e.parts.world.sys(s).unwrap().auto_draw = v;
        Ok(Value::Null)
    });
    r.f("part_system_draw_order", |e, _, a| {
        let old_to_new = A(a).bool_or(1, true);
        let s = sys(e, a, 0)?;
        e.parts.world.sys(s).unwrap().new_below = !old_to_new;
        Ok(Value::Null)
    });
    r.f("part_system_update", |e, _, a| {
        let s = sys(e, a, 0)?;
        e.parts.world.system_update(s);
        Ok(Value::Null)
    });
    r.f("part_system_drawit", |e, _, a| {
        let s = sys(e, a, 0)?;
        e.draw_particles(s);
        Ok(Value::Null)
    });
    r.f("part_system_get_info", |e, _, a| {
        let s = sys(e, a, 0)?;
        let w = &mut e.parts.world;
        let asset_name = w.sys(s).unwrap().asset.and_then(|i| w.assets.get(i as usize)).map(|(n, _)| n.clone());
        let s = w.sys(s).unwrap();
        let ems: Vec<Value> = s
            .emitters
            .iter()
            .enumerate()
            .filter_map(|(i, x)| x.as_ref().map(|x| (i, x)))
            .map(|(i, x)| {
                Value::new_struct([
                    (modscript::intern("ind"), Value::Num(i as f64)),
                    (modscript::intern("name"), Value::str(&x.name)),
                    (modscript::intern("enabled"), Value::Bool(x.enabled)),
                    (modscript::intern("type"), x.ty.map(|t| Value::Num(t as f64)).unwrap_or(Value::Null)),
                ])
            })
            .collect();
        Ok(Value::new_struct([
            (modscript::intern("name"), asset_name.map(|n| Value::string(n)).unwrap_or(Value::Null)),
            (modscript::intern("x"), Value::Num(s.x)),
            (modscript::intern("y"), Value::Num(s.y)),
            (modscript::intern("depth"), Value::Num(s.depth)),
            (modscript::intern("particle_count"), Value::Num(s.parts.len() as f64)),
            (modscript::intern("emitters"), Value::array(ems)),
        ]))
    });

    // ---------------------------------------------------------------- particles
    r.f("part_particles_create", |e, _, a| {
        let s = sys(e, a, 0)?;
        let (x, y) = (A(a).num(1)?, A(a).num(2)?);
        let t = ty_id(e, a, 3)?;
        let n = A(a).num_or(4, 1.0)? as i32;
        e.parts.world.create(s, x, y, t, n, None);
        Ok(Value::Null)
    });
    fn create_color(e: &mut Engine, _: &modscript::CallCx, a: &[Value]) -> RtResult<Value> {
        let s = sys(e, a, 0)?;
        let (x, y) = (A(a).num(1)?, A(a).num(2)?);
        let t = ty_id(e, a, 3)?;
        let c = rgb(A(a).num(4)?);
        let n = A(a).num_or(5, 1.0)? as i32;
        e.parts.world.create(s, x, y, t, n, Some(c));
        Ok(Value::Null)
    }
    r.f("part_particles_create_color", create_color);
    r.f("part_particles_create_colour", create_color);
    r.f("part_particles_burst", |e, _, a| {
        // every emitter of a particle system asset fires once at (x, y), into system `s`
        let s = sys(e, a, 0)?;
        let (x, y) = (A(a).num(1)?, A(a).num(2)?);
        let asset = A(a).asset(3, AssetKind::ParticleSystem)?;
        let sprites = &e.sprites;
        let lookup = |n: &str| sprites.iter().position(|s| s.name == n).map(|i| i as u32);
        let w = &mut e.parts.world;
        let tmp = w.system_from_asset(asset as usize, 0.0, 0.0, &lookup).map_err(RtError::new)?;
        let ems: Vec<Emitter> = w.sys(tmp).unwrap().emitters.iter().flatten().cloned().collect();
        let own = w.sys(tmp).unwrap().own_types.clone();
        w.sys(tmp).unwrap().own_types.clear();
        w.system_destroy(tmp);
        // the burst's types now belong to the target system
        w.sys(s).unwrap().own_types.extend(own);
        for mut em in ems {
            em.region = [em.region[0] + x, em.region[1] + x, em.region[2] + y, em.region[3] + y];
            let Some(t) = em.ty else { continue };
            let n = if em.burst != 0 { em.burst } else { em.stream.max(1) };
            let k = {
                let sy = w.sys(s).unwrap();
                sy.emitters.push(Some(em));
                sy.emitters.len() - 1
            };
            w.emitter_emit(s, k, t, n);
            w.sys(s).unwrap().emitters[k] = None;
        }
        Ok(Value::Null)
    });
    r.f("part_particles_clear", |e, _, a| {
        let s = sys(e, a, 0)?;
        e.parts.world.sys(s).unwrap().parts.clear();
        Ok(Value::Null)
    });
    r.f("part_particles_count", |e, _, a| {
        let s = sys(e, a, 0)?;
        Ok(Value::Num(e.parts.world.count(s) as f64))
    });

    // ---------------------------------------------------------------- types
    r.f("part_type_create", |e, _, _| Ok(Value::Num(e.parts.world.type_create(PartType::default()) as f64)));
    r.f("part_type_destroy", |e, _, a| {
        let t = ty_id(e, a, 0)?;
        e.parts.world.type_destroy(t);
        Ok(Value::Null)
    });
    r.f("part_type_exists", |e, _, a| {
        let id = A(a).int(0)?;
        Ok(Value::Bool(id >= 0 && e.parts.world.type_get(id as usize).is_some()))
    });
    r.f("part_type_clear", |e, _, a| set_ty(e, a, |t, _| {
        *t = PartType::default();
        Ok(())
    }));
    r.f("part_type_shape", |e, _, a| set_ty(e, a, |t, p| {
        let k = p.int(1)?;
        if k < 0 || k as usize >= SHAPE_NAMES.len() {
            return err(format!("unknown particle shape {k} (use pt_shape_pixel … pt_shape_snow)"));
        }
        t.tex = Tex::Shape(k as u8);
        Ok(())
    }));
    r.f("part_type_sprite", |e, _, a| {
        let spr = A(a).asset(1, AssetKind::Sprite)?;
        set_ty(e, a, |t, p| {
            t.tex = Tex::Sprite(spr);
            t.animate = p.bool_or(2, false);
            t.stretch = p.bool_or(3, false);
            t.random_frame = p.bool_or(4, false);
            Ok(())
        })
    });
    r.f("part_type_size", |e, _, a| set_ty(e, a, |t, p| {
        t.size = [p.num(1)?, p.num(2)?];
        t.size_incr = p.num_or(3, 0.0)?;
        t.size_wiggle = p.num_or(4, 0.0)?;
        Ok(())
    }));
    r.f("part_type_scale", |e, _, a| set_ty(e, a, |t, p| {
        t.scale = [p.num(1)?, p.num(2)?];
        Ok(())
    }));
    r.f("part_type_speed", |e, _, a| set_ty(e, a, |t, p| {
        t.speed = [p.num(1)?, p.num(2)?];
        t.speed_incr = p.num_or(3, 0.0)?;
        t.speed_wiggle = p.num_or(4, 0.0)?;
        Ok(())
    }));
    r.f("part_type_direction", |e, _, a| set_ty(e, a, |t, p| {
        t.dir = [p.num(1)?, p.num(2)?];
        t.dir_incr = p.num_or(3, 0.0)?;
        t.dir_wiggle = p.num_or(4, 0.0)?;
        Ok(())
    }));
    r.f("part_type_gravity", |e, _, a| set_ty(e, a, |t, p| {
        t.grav = p.num(1)?;
        t.grav_dir = p.num_or(2, 270.0)?;
        Ok(())
    }));
    r.f("part_type_orientation", |e, _, a| set_ty(e, a, |t, p| {
        t.orient = [p.num(1)?, p.num(2)?];
        t.orient_incr = p.num_or(3, 0.0)?;
        t.orient_wiggle = p.num_or(4, 0.0)?;
        t.orient_rel = p.bool_or(5, false);
        Ok(())
    }));
    fn colors(t: &mut PartType, p: &A, n: usize) -> RtResult<()> {
        t.colors = (0..n).map(|i| p.num(1 + i).map(rgb)).collect::<RtResult<_>>()?;
        t.color_mix = false;
        t.color_rgb = None;
        t.color_hsv = None;
        Ok(())
    }
    for (name, n) in [("1", 1usize), ("2", 2), ("3", 3)] {
        let f: super::NativeFn = match n {
            1 => |e, _, a| set_ty(e, a, |t, p| colors(t, p, 1)),
            2 => |e, _, a| set_ty(e, a, |t, p| colors(t, p, 2)),
            _ => |e, _, a| set_ty(e, a, |t, p| colors(t, p, 3)),
        };
        r.f(&format!("part_type_color{name}"), f);
        r.f(&format!("part_type_colour{name}"), f);
    }
    fn mix(e: &mut Engine, _: &modscript::CallCx, a: &[Value]) -> RtResult<Value> {
        set_ty(e, a, |t, p| {
            colors(t, p, 2)?;
            t.color_mix = true;
            Ok(())
        })
    }
    r.f("part_type_color_mix", mix);
    r.f("part_type_colour_mix", mix);
    fn ranges(p: &A) -> RtResult<[f64; 6]> {
        let mut v = [0.0; 6];
        for (i, x) in v.iter_mut().enumerate() {
            *x = p.num(1 + i)?;
        }
        Ok(v)
    }
    fn crgb(e: &mut Engine, _: &modscript::CallCx, a: &[Value]) -> RtResult<Value> {
        set_ty(e, a, |t, p| {
            t.color_rgb = Some(ranges(p)?);
            t.color_hsv = None;
            Ok(())
        })
    }
    fn chsv(e: &mut Engine, _: &modscript::CallCx, a: &[Value]) -> RtResult<Value> {
        set_ty(e, a, |t, p| {
            t.color_hsv = Some(ranges(p)?);
            t.color_rgb = None;
            Ok(())
        })
    }
    r.f("part_type_color_rgb", crgb);
    r.f("part_type_colour_rgb", crgb);
    r.f("part_type_color_hsv", chsv);
    r.f("part_type_colour_hsv", chsv);
    r.f("part_type_alpha1", |e, _, a| set_ty(e, a, |t, p| {
        t.alphas = vec![p.num(1)?];
        Ok(())
    }));
    r.f("part_type_alpha2", |e, _, a| set_ty(e, a, |t, p| {
        t.alphas = vec![p.num(1)?, p.num(2)?];
        Ok(())
    }));
    r.f("part_type_alpha3", |e, _, a| set_ty(e, a, |t, p| {
        t.alphas = vec![p.num(1)?, p.num(2)?, p.num(3)?];
        Ok(())
    }));
    r.f("part_type_blend", |e, _, a| set_ty(e, a, |t, p| {
        t.additive = p.bool_or(1, false);
        Ok(())
    }));
    r.f("part_type_life", |e, _, a| set_ty(e, a, |t, p| {
        let lo = p.num(1)?;
        t.life = [lo, p.num_or(2, lo)?];
        Ok(())
    }));
    r.f("part_type_step", |e, _, a| {
        let n = A(a).num(1)? as i32;
        let other = ty_id(e, a, 2)?;
        set_ty(e, a, |t, _| {
            t.step = (n, Some(other));
            Ok(())
        })
    });
    r.f("part_type_death", |e, _, a| {
        let n = A(a).num(1)? as i32;
        let other = ty_id(e, a, 2)?;
        set_ty(e, a, |t, _| {
            t.death = (n, Some(other));
            Ok(())
        })
    });

    // ---------------------------------------------------------------- emitters
    r.f("part_emitter_create", |e, _, a| {
        let s = sys(e, a, 0)?;
        let sy = e.parts.world.sys(s).unwrap();
        sy.emitters.push(Some(Emitter::new()));
        Ok(Value::Num((sy.emitters.len() - 1) as f64))
    });
    r.f("part_emitter_destroy", |e, _, a| {
        em(e, a, 0)?;
        let (s, k) = (A(a).int(0)? as usize, A(a).int(1)? as usize);
        e.parts.world.sys(s).unwrap().emitters[k] = None;
        Ok(Value::Null)
    });
    r.f("part_emitter_destroy_all", |e, _, a| {
        let s = sys(e, a, 0)?;
        e.parts.world.sys(s).unwrap().emitters.clear();
        Ok(Value::Null)
    });
    r.f("part_emitter_exists", |e, _, a| Ok(Value::Bool(em(e, a, 0).is_ok())));
    r.f("part_emitter_clear", |e, _, a| {
        let x = em(e, a, 0)?;
        let name = std::mem::take(&mut x.name);
        *x = Emitter { name, ..Emitter::new() };
        Ok(Value::Null)
    });
    r.f("part_emitter_find", |e, _, a| {
        let s = sys(e, a, 0)?;
        let name = A(a).text(1)?;
        let i = e.parts.world.sys(s).unwrap().emitters.iter().position(|x| x.as_ref().is_some_and(|x| x.name == name));
        Ok(Value::Num(i.map(|i| i as f64).unwrap_or(-1.0)))
    });
    r.f("part_emitter_region", |e, _, a| {
        let p = A(a);
        let (x1, x2, y1, y2) = (p.num(2)?, p.num(3)?, p.num(4)?, p.num(5)?);
        let shape = RegionShape::from_num(p.num_or(6, 0.0)? as i64);
        let distr = Distr::from_num(p.num_or(7, 0.0)? as i64);
        let x = em(e, a, 0)?;
        x.region = if shape == RegionShape::Line { [x1, x2, y1, y2] } else { [x1.min(x2), x1.max(x2), y1.min(y2), y1.max(y2)] };
        x.shape = shape;
        x.distr = distr;
        Ok(Value::Null)
    });
    r.f("part_emitter_burst", |e, _, a| {
        let t = ty_id(e, a, 2)?;
        let n = A(a).num(3)? as i32;
        em(e, a, 0)?;
        let (s, k) = (A(a).int(0)? as usize, A(a).int(1)? as usize);
        e.parts.world.emitter_emit(s, k, t, n);
        Ok(Value::Null)
    });
    r.f("part_emitter_stream", |e, _, a| {
        let t = ty_id(e, a, 2)?;
        let n = A(a).num(3)? as i32;
        let x = em(e, a, 0)?;
        x.ty = Some(t);
        x.stream = n;
        Ok(Value::Null)
    });
    r.f("part_emitter_enable", |e, _, a| {
        let v = A(a).bool_or(2, true);
        em(e, a, 0)?.enabled = v;
        Ok(Value::Null)
    });
    r.f("part_emitter_interval", |e, _, a| {
        let (lo, hi) = (A(a).num(2)?, A(a).num_or(3, A(a).num(2)?)?);
        let x = em(e, a, 0)?;
        x.interval = [lo, hi];
        if x.burst != 0 {
            x.burst_due = true;
        }
        Ok(Value::Null)
    });
    r.f("part_emitter_delay", |e, _, a| {
        let (lo, hi) = (A(a).num(2)?, A(a).num_or(3, A(a).num(2)?)?);
        let w = lo + (hi - lo) * e.parts.world.rand();
        em(e, a, 0)?.wait = w.max(0.0);
        Ok(Value::Null)
    });
}
