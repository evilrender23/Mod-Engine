//! Engine glue for particle systems, shaders and extensions: loading them with the project,
//! stepping and drawing particles, and the extension `init`/`final` hooks.

use crate::draw::{DrawCmd, Rgba};
use crate::engine::{toml_to_value, Engine};
use crate::extension::Extension;
use crate::particles::{PartWorld, Tex, SHAPE_NAMES, SHAPE_SIZE};
use crate::project::Sprite;
use crate::shader::Shaders;
use modscript::{intern, parse_stmts, AssetKind, Value};
use std::rc::Rc;
use std::sync::OnceLock;

/// Particle state of a running game.
#[derive(Clone, Debug, Default)]
pub struct EngineParts {
    pub world: PartWorld,
    /// Index of the first built-in shape sprite in `Engine::sprites`.
    pub shape_base: u32,
}

fn shape_images() -> &'static Vec<Vec<u8>> {
    static IMGS: OnceLock<Vec<Vec<u8>>> = OnceLock::new();
    IMGS.get_or_init(|| (0..SHAPE_NAMES.len()).map(crate::particles::shape_image).collect())
}

/// The built-in particle shapes as sprites (origin in the centre).
pub fn shape_sprites() -> Vec<Sprite> {
    let n = SHAPE_SIZE;
    shape_images()
        .iter()
        .enumerate()
        .map(|(k, rgba)| Sprite {
            name: format!("__pt_shape_{}", SHAPE_NAMES[k]),
            fw: n,
            fh: n,
            frames: vec![[0, 0, n, n]],
            sheet_w: n,
            sheet_h: n,
            rgba: rgba.clone(),
            origin: (n as f64 / 2.0, n as f64 / 2.0),
            bbox: [0.0, 0.0, n as f64, n as f64],
            fps: 0.0,
        })
        .collect()
}

/// GameMaker's particle, blend mode and shape constants.
pub fn constants() -> Vec<(String, f64)> {
    let mut v: Vec<(String, f64)> = SHAPE_NAMES.iter().enumerate().map(|(i, s)| (format!("pt_shape_{s}"), i as f64)).collect();
    for (i, s) in crate::particles::RegionShape::ALL.iter().enumerate() {
        v.push((format!("ps_shape_{s}"), i as f64));
    }
    for (i, s) in crate::particles::Distr::ALL.iter().enumerate() {
        v.push((format!("ps_distr_{s}"), i as f64));
    }
    for (i, s) in ["normal", "add", "subtract", "max"].iter().enumerate() {
        v.push((format!("bm_{s}"), i as f64));
    }
    v
}

/// Loads `particles/`, `shaders/` and `extensions/` into the engine (called from
/// `Engine::from_project`, before objects and rooms are built).
pub(crate) fn link(e: &mut Engine, errors: &mut Vec<String>) {
    let dir = e.dir.clone();
    let register = |e: &mut Engine, name: &str, v: Value, errors: &mut Vec<String>| {
        let s = intern(name);
        if let Some(prev) = e.assets.get(&s) {
            errors.push(format!("duplicate asset name '{name}' ({} and {})", prev.type_name(), v.type_name()));
        }
        e.assets.insert(s, v);
    };

    // ---- particle systems
    match crate::particles::load_assets(&dir) {
        Ok(list) => {
            let mut assets = vec![];
            for (i, (name, path, t)) in list.into_iter().enumerate() {
                let sprite_ok = |n: &str| matches!(e.assets.get(&intern(n)), Some(Value::Asset(AssetKind::Sprite, _)));
                for p in t.validate(&sprite_ok) {
                    errors.push(format!("{}: {p}", path.display()));
                }
                register(e, &name, Value::Asset(AssetKind::ParticleSystem, i as u32), errors);
                assets.push((name, t));
            }
            e.parts.world = PartWorld::new(assets);
        }
        Err(m) => errors.push(m),
    }
    e.parts.shape_base = e.sprites.len() as u32;
    e.sprites.extend(shape_sprites());

    // ---- shaders
    match crate::shader::load_assets(&dir) {
        Ok(list) => {
            for (i, s) in list.iter().enumerate() {
                register(e, &s.name, Value::Asset(AssetKind::Shader, i as u32), errors);
                // static check (the GPU compiles them when the game starts)
                for p in s.problems() {
                    e.report(modscript::RtError::new(format!("shader '{}': {p}", s.name)));
                }
            }
            e.shaders = Shaders::new(list);
        }
        Err(m) => errors.push(m),
    }
    for (k, v) in constants() {
        e.consts.insert(intern(&k), Value::Num(v));
    }

    // ---- extensions (their scripts were put first by Project::load)
    match crate::extension::load_all(&dir) {
        Ok(list) => {
            for x in list.iter().filter(|x| x.meta.enabled) {
                for (k, v) in &x.meta.constants {
                    match toml_to_value(v) {
                        Ok(val) => {
                            let s = intern(k);
                            if e.consts.contains_key(&s) || e.assets.contains_key(&s) {
                                errors.push(format!("extension '{}': constant '{k}' clashes with another name", x.name));
                            }
                            e.consts.insert(s, val);
                        }
                        Err(m) => errors.push(format!("extension '{}': constant '{k}': {m}", x.name)),
                    }
                }
                for f in &x.meta.functions {
                    if !e.functions.contains_key(&intern(&f.name)) {
                        errors.push(format!("extension '{}': exported function '{}' is not defined in its scripts", x.name, f.name));
                    }
                }
                for hook in [&x.meta.init, &x.meta.final_].into_iter().flatten() {
                    if !e.functions.contains_key(&intern(hook)) {
                        errors.push(format!("extension '{}': function '{hook}' (init/final) is not defined", x.name));
                    }
                }
                // `init` runs right after the extension's own top-level code
                if let Some(init) = &x.meta.init {
                    let file: Rc<str> = Rc::from(format!("extensions/{}/{}", x.name, crate::extension::META_FILE).as_str());
                    let pos = x
                        .scripts
                        .iter()
                        .filter_map(|rel| e.init_code.iter().position(|(f, _)| &**f == rel.as_str()))
                        .max()
                        .map(|p| p + 1)
                        .unwrap_or_else(|| e.init_code.iter().take_while(|(f, _)| f.starts_with("extensions/")).count());
                    if let Ok(stmts) = parse_stmts(&file, &format!("{init}()")) {
                        e.init_code.insert(pos.min(e.init_code.len()), (file, Rc::new(stmts)));
                    }
                }
            }
            e.extensions = list;
        }
        Err(m) => errors.push(m),
    }
}

impl Engine {
    /// Calls the `final` function of every enabled extension (when the game ends).
    pub fn extensions_final(&mut self) {
        let hooks: Vec<String> = self.extensions.iter().filter(|x| x.meta.enabled).filter_map(|x| x.meta.final_.clone()).collect();
        for h in hooks {
            if let Ok(stmts) = parse_stmts("<extension final>", &format!("{h}()")) {
                if let Err(err) = modscript::run_stmts(self, &stmts, &Rc::from("<extension final>"), modscript::CallCx::global()) {
                    self.report(err);
                }
            }
        }
    }

    pub fn extension(&self, name: &str) -> Option<&Extension> {
        self.extensions.iter().find(|x| x.name == name || x.meta.name == name)
    }

    /// Steps every particle system with automatic update on (once per game step).
    pub fn step_particles(&mut self) {
        self.parts.world.step();
    }

    /// (depth, system) of every system drawn automatically.
    pub fn particle_draw_items(&self) -> Vec<(f64, usize)> {
        self.parts.world.systems.iter().enumerate().filter_map(|(i, s)| s.as_ref().filter(|s| s.auto_draw).map(|s| (s.depth, i))).collect()
    }

    /// Records the draw commands of particle system `sys`.
    pub fn draw_particles(&mut self, sys: usize) {
        let sprites = &self.sprites;
        let list = self.parts.world.draw_list(sys, &|s| sprites.get(s as usize).map(|s| s.frames.len() as u32).unwrap_or(1));
        let base = self.parts.shape_base;
        let restore = self.shaders.blend;
        let mut cur = restore;
        for p in list {
            let want = if p.additive { 1 } else { restore };
            if want != cur {
                self.cmds.push(DrawCmd::Blend(want));
                cur = want;
            }
            let spr = match p.tex {
                Tex::Shape(k) => base + k as u32,
                Tex::Sprite(s) => s,
            };
            if spr as usize >= self.sprites.len() {
                continue;
            }
            self.cmds.push(DrawCmd::Sprite {
                spr,
                frame: p.frame,
                x: p.x as f32,
                y: p.y as f32,
                sx: p.sx as f32,
                sy: p.sy as f32,
                angle: p.angle as f32,
                color: Rgba::from_rgb(p.color, p.alpha),
            });
        }
        if cur != restore {
            self.cmds.push(DrawCmd::Blend(restore));
        }
    }

    /// Sprite index resolver for particle types (by asset name).
    pub fn sprite_index(&self, name: &str) -> Option<u32> {
        match self.assets.get(&intern(name)) {
            Some(Value::Asset(AssetKind::Sprite, i)) => Some(*i),
            _ => None,
        }
    }
}
