//! Shaders and blend modes on the GPU: one macroquad material per (shader, blend mode) pair,
//! created on demand, plus the uniform values and sampler textures scripts set.

use macroquad::miniquad::{BlendFactor, BlendState, BlendValue, Equation};
use macroquad::prelude::*;
use modengine::draw::DrawCmd;
use modengine::shader::{prepare, ShaderAsset, Stage, UType, DEFAULT_FSH, DEFAULT_VSH};
use modengine::Engine;
use std::collections::HashMap;

pub struct GpuFx {
    shaders: Vec<ShaderAsset>,
    /// (shader, blend) → material; None when that shader failed to compile.
    mats: HashMap<(Option<u32>, u8), Option<Material>>,
    /// Last values set for each (shader, uniform), applied to materials created later.
    values: HashMap<(u32, String), Vec<f32>>,
    samplers: HashMap<(u32, String), Texture2D>,
    cur_shader: Option<u32>,
    cur_blend: u8,
}

fn pipeline(blend: u8) -> PipelineParams {
    let sa = BlendFactor::Value(BlendValue::SourceAlpha);
    let (eq, s, d) = match blend {
        1 => (Equation::Add, sa, BlendFactor::One),
        2 => (Equation::ReverseSubtract, sa, BlendFactor::One),
        3 => (Equation::Add, sa, BlendFactor::OneMinusValue(BlendValue::SourceColor)),
        _ => (Equation::Add, sa, BlendFactor::OneMinusValue(BlendValue::SourceAlpha)),
    };
    PipelineParams {
        color_blend: Some(BlendState::new(eq, s, d)),
        alpha_blend: Some(BlendState::new(Equation::Add, BlendFactor::Zero, BlendFactor::One)),
        ..Default::default()
    }
}

/// Compiles `vsh`/`fsh` (GameMaker-style sources) into a material.
pub fn compile(s: Option<&ShaderAsset>, blend: u8) -> Result<Material, String> {
    let (vsh, fsh) = match s {
        Some(s) => (s.vsh.as_str(), s.fsh.as_str()),
        None => (DEFAULT_VSH, DEFAULT_FSH),
    };
    let mut uniforms = vec![];
    let mut textures = vec![];
    for u in s.map(|s| s.uniforms.as_slice()).unwrap_or(&[]) {
        let ty = match u.ty {
            UType::Float(1) => UniformType::Float1,
            UType::Float(2) => UniformType::Float2,
            UType::Float(3) => UniformType::Float3,
            UType::Float(_) => UniformType::Float4,
            UType::Int(1) => UniformType::Int1,
            UType::Int(2) => UniformType::Int2,
            UType::Int(3) => UniformType::Int3,
            UType::Int(_) => UniformType::Int4,
            UType::Mat4 => UniformType::Mat4,
            UType::Sampler => {
                textures.push(u.name.clone());
                continue;
            }
        };
        let d = UniformDesc::new(&u.name, ty);
        uniforms.push(if u.count > 1 { d.array(u.count) } else { d });
    }
    let vertex = prepare(vsh, Stage::Vertex);
    let fragment = prepare(fsh, Stage::Fragment);
    load_material(
        ShaderSource::Glsl { vertex: &vertex, fragment: &fragment },
        MaterialParams { pipeline_params: pipeline(blend), uniforms, textures },
    )
    .map_err(|e| match e {
        macroquad::Error::ShaderError(ShaderError::CompilationError { shader_type, error_message }) => {
            let stage = match shader_type {
                macroquad::miniquad::ShaderType::Vertex => "vsh",
                macroquad::miniquad::ShaderType::Fragment => "fsh",
            };
            error_message.lines().filter(|l| !l.trim().is_empty()).map(|l| format!("{stage}: {}", l.trim())).collect::<Vec<_>>().join("\n")
        }
        other => format!("{other:?}"),
    })
}

impl GpuFx {
    /// Compiles every shader of the game, recording the result in `e.shaders.status` and
    /// reporting failures as runtime errors.
    pub fn new(e: &mut Engine) -> Self {
        let mut fx = GpuFx {
            shaders: e.shaders.list.clone(),
            mats: HashMap::new(),
            values: HashMap::new(),
            samplers: HashMap::new(),
            cur_shader: None,
            cur_blend: 0,
        };
        for i in 0..fx.shaders.len() {
            let r = compile(Some(&fx.shaders[i]), 0);
            let status = match &r {
                Ok(_) => Ok(()),
                Err(msg) => {
                    e.report(modscript::RtError::new(format!("shader '{}' does not compile:\n{msg}", fx.shaders[i].name)));
                    Err(msg.clone())
                }
            };
            e.shaders.status[i] = Some(status);
            fx.mats.insert((Some(i as u32), 0), r.ok());
        }
        fx
    }

    fn material(&mut self, sh: Option<u32>, blend: u8) -> Option<&Material> {
        if !self.mats.contains_key(&(sh, blend)) {
            let m = compile(sh.and_then(|i| self.shaders.get(i as usize)), blend).ok();
            if let (Some(m), Some(i)) = (&m, sh) {
                for ((s, name), v) in &self.values {
                    if *s == i {
                        set_vals(m, &self.shaders[i as usize], name, v);
                    }
                }
                for ((s, name), t) in &self.samplers {
                    if *s == i {
                        m.set_texture(name, t.clone());
                    }
                }
            }
            self.mats.insert((sh, blend), m);
        }
        self.mats.get(&(sh, blend)).and_then(|m| m.as_ref())
    }

    fn use_current(&mut self) {
        let (sh, blend) = (self.cur_shader, self.cur_blend);
        if sh.is_none() && blend == 0 {
            gl_use_default_material();
            return;
        }
        match self.material(sh, blend) {
            Some(m) => gl_use_material(m),
            // a shader that didn't compile draws with the default one
            None => match self.material(None, blend) {
                Some(m) => gl_use_material(m),
                None => gl_use_default_material(),
            },
        }
    }

    pub fn begin_frame(&mut self) {
        self.cur_shader = None;
        self.cur_blend = 0;
        gl_use_default_material();
    }

    pub fn end_frame(&mut self) {
        gl_use_default_material();
    }

    pub fn apply(&mut self, c: &DrawCmd, textures: &[Texture2D]) {
        match c {
            DrawCmd::Shader(s) => {
                self.cur_shader = *s;
                self.use_current();
            }
            DrawCmd::Blend(b) => {
                self.cur_blend = *b;
                self.use_current();
            }
            DrawCmd::Uniform { shader, name, vals } => {
                self.values.insert((*shader, name.clone()), vals.clone());
                if let Some(s) = self.shaders.get(*shader as usize) {
                    for ((k, _), m) in self.mats.iter() {
                        if *k == Some(*shader) {
                            if let Some(m) = m {
                                set_vals(m, s, name, vals);
                            }
                        }
                    }
                }
            }
            DrawCmd::Sampler { shader, name, spr } => {
                if let Some(t) = textures.get(*spr as usize) {
                    self.samplers.insert((*shader, name.clone()), t.clone());
                    for ((k, _), m) in self.mats.iter() {
                        if *k == Some(*shader) {
                            if let Some(m) = m {
                                m.set_texture(name, t.clone());
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn set_vals(m: &Material, s: &ShaderAsset, name: &str, v: &[f32]) {
    match s.uniform(name).map(|u| u.ty) {
        Some(UType::Int(_)) => {
            let ints: Vec<i32> = v.iter().map(|x| *x as i32).collect();
            m.set_uniform_array(name, &ints[..]);
        }
        Some(_) => m.set_uniform_array(name, v),
        None => {}
    }
}
