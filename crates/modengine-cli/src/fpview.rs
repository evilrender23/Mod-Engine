//! GPU side of `modengine::fpview`: textured 2D meshes (`draw_primitive_*`,
//! `draw_sprite_pos`), first-person views (real perspective + depth buffer in a render
//! target of the view's resolution, then drawn like a surface) and the pixelate / posterize
//! / dither / palette post-process over application_surface.

use crate::canvas::{camera_for, Canvas};
use crate::gpu_fx::GpuFx;
use macroquad::miniquad::{BlendFactor, BlendState, BlendValue, Comparison, CullFace, Equation, PipelineParams};
use macroquad::models::Vertex;
use macroquad::prelude::*;
use modengine::draw::DrawCmd;
use modengine::fpview::{Face, FpFrame, MVert, MAX_PALETTE};
use std::collections::HashMap;

const VSH_3D: &str = "#version 100
attribute vec3 position;
attribute vec2 texcoord;
attribute vec4 color0;
varying lowp vec4 color;
varying highp vec2 uv;
varying highp vec3 wpos;
uniform mat4 Model;
uniform mat4 Projection;
void main() {
    gl_Position = Projection * Model * vec4(position, 1.0);
    color = color0 / 255.0;
    uv = texcoord;
    wpos = position;
}
";

const FSH_3D: &str = "#version 100
precision highp float;
varying lowp vec4 color;
varying highp vec2 uv;
varying highp vec3 wpos;
uniform sampler2D Texture;
uniform vec4 fogColor;
uniform vec2 fogRange;
uniform vec3 eye;
void main() {
    vec4 c = texture2D(Texture, uv) * color;
    if (c.a < 0.02) discard;
    float d = length(wpos.xz - eye.xz);
    float f = clamp((d - fogRange.x) / max(fogRange.y - fogRange.x, 0.0001), 0.0, 1.0);
    gl_FragColor = vec4(mix(c.rgb, fogColor.rgb, f), c.a);
}
";

const VSH_2D: &str = "#version 100
attribute vec3 position;
attribute vec2 texcoord;
varying vec2 uv;
uniform mat4 Model;
uniform mat4 Projection;
void main() {
    gl_Position = Projection * Model * vec4(position, 1.0);
    uv = texcoord;
}
";

const FSH_COPY: &str = "#version 100
precision mediump float;
varying vec2 uv;
uniform sampler2D Texture;
void main() {
    gl_FragColor = vec4(texture2D(Texture, uv).rgb, 1.0);
}
";

/// Point-samples the source at the centre of each output pixel, then posterize (with an
/// ordered 4×4 Bayer dither) and / or snaps to the nearest palette colour.
const FSH_POST: &str = "#version 100
precision highp float;
varying vec2 uv;
uniform sampler2D Texture;
uniform vec2 srcSize;
uniform float levels;
uniform float dither;
uniform float palCount;
uniform vec3 palette[64];
float bayer2(vec2 a) { a = floor(a); return fract(a.x / 2.0 + a.y * a.y * 0.75); }
float bayer4(vec2 a) { return bayer2(0.5 * a) * 0.25 + bayer2(a); }
void main() {
    vec2 st = (floor(uv * srcSize) + 0.5) / srcSize;
    vec3 c = texture2D(Texture, st).rgb;
    float b = bayer4(gl_FragCoord.xy) - 0.5;
    if (levels >= 2.0) {
        float q = levels - 1.0;
        c = clamp(floor(c * q + 0.5 + b * dither) / q, 0.0, 1.0);
    } else if (palCount > 0.5) {
        c = c + vec3(b * dither * 0.25);
    }
    if (palCount > 0.5) {
        vec3 best = c;
        float bd = 1e9;
        for (int i = 0; i < 64; i++) {
            if (float(i) >= palCount) break;
            vec3 d = (c - palette[i]) * vec3(0.55, 0.75, 0.4);
            float dd = dot(d, d);
            if (dd < bd) { bd = dd; best = palette[i]; }
        }
        c = best;
    }
    gl_FragColor = vec4(c, 1.0);
}
";

fn blend_params() -> PipelineParams {
    let sa = BlendFactor::Value(BlendValue::SourceAlpha);
    PipelineParams {
        color_blend: Some(BlendState::new(Equation::Add, sa, BlendFactor::OneMinusValue(BlendValue::SourceAlpha))),
        alpha_blend: Some(BlendState::new(Equation::Add, BlendFactor::One, BlendFactor::OneMinusValue(BlendValue::SourceAlpha))),
        depth_test: Comparison::LessOrEqual,
        depth_write: true,
        cull_face: CullFace::Nothing,
        ..Default::default()
    }
}

fn material_3d() -> Material {
    let uniforms = vec![UniformDesc::new("fogColor", UniformType::Float4), UniformDesc::new("fogRange", UniformType::Float2), UniformDesc::new("eye", UniformType::Float3)];
    load_material(ShaderSource::Glsl { vertex: VSH_3D, fragment: FSH_3D }, MaterialParams { pipeline_params: blend_params(), uniforms, textures: vec![] }).expect("fpview shader")
}

/// A camera with an explicit matrix that renders into a render target with depth.
struct Cam3 {
    m: Mat4,
    pass: RenderPass,
}

impl Camera for Cam3 {
    fn matrix(&self) -> Mat4 {
        self.m
    }
    fn depth_enabled(&self) -> bool {
        true
    }
    fn render_pass(&self) -> Option<RenderPass> {
        Some(self.pass.clone())
    }
    fn viewport(&self) -> Option<(i32, i32, i32, i32)> {
        None
    }
}

pub struct FpGpu {
    /// Walls, floors and billboards. miniquad turns the depth *test* off when depth_write
    /// is false, so billboards write depth too (drawn far to near, transparent texels discarded).
    world: Material,
    post: Material,
    copy: Material,
    /// Render targets (with depth) of the views, by size.
    views: HashMap<(u32, u32), RenderTarget>,
    /// Low-resolution image of the post-process, by size.
    small: HashMap<(u32, u32), RenderTarget>,
}

impl FpGpu {
    pub fn new() -> FpGpu {
        let post_uniforms = vec![
            UniformDesc::new("srcSize", UniformType::Float2),
            UniformDesc::new("levels", UniformType::Float1),
            UniformDesc::new("dither", UniformType::Float1),
            UniformDesc::new("palCount", UniformType::Float1),
            UniformDesc::new("palette", UniformType::Float3).array(MAX_PALETTE),
        ];
        let plain = PipelineParams::default();
        FpGpu {
            world: material_3d(),
            post: load_material(ShaderSource::Glsl { vertex: VSH_2D, fragment: FSH_POST }, MaterialParams { pipeline_params: plain, uniforms: post_uniforms, textures: vec![] }).expect("postfx shader"),
            copy: load_material(ShaderSource::Glsl { vertex: VSH_2D, fragment: FSH_COPY }, MaterialParams { pipeline_params: plain, ..Default::default() }).expect("copy shader"),
            views: HashMap::new(),
            small: HashMap::new(),
        }
    }

    /// Replays a `FpView` or `PostFx` command. `scissor` = the clip rectangle in force, put
    /// back afterwards together with the canvas camera and the current material.
    pub fn apply(&mut self, c: &DrawCmd, textures: &[Texture2D], cv: &Canvas, fx: &mut GpuFx, scissor: Option<[f32; 4]>) {
        match c {
            DrawCmd::FpView { frame, x, y, w, h } => {
                self.render_view(frame, textures);
                restore(cv, fx, scissor);
                let rt = &self.views[&(frame.w, frame.h)];
                draw_texture_ex(&rt.texture, *x, *y, WHITE, DrawTextureParams { dest_size: Some(vec2(*w, *h)), ..Default::default() });
            }
            DrawCmd::PostFx { pixels_wide, levels, dither, palette } => {
                if !cv.on_frame() {
                    return;
                }
                let app = &cv.app.texture;
                let (aw, ah) = (app.width(), app.height());
                let pw = (*pixels_wide).clamp(1, aw.max(1.0) as u32);
                let ph = ((pw as f32 * ah / aw.max(1.0)).round() as u32).max(1);
                let small = self.small.entry((pw, ph)).or_insert_with(|| {
                    let rt = render_target(pw, ph);
                    rt.texture.set_filter(FilterMode::Nearest);
                    rt
                });
                set_camera(&camera_for(small, 0.0, 0.0, pw as f32, ph as f32));
                unsafe { get_internal_gl() }.quad_gl.scissor(None);
                let m = &self.post;
                m.set_uniform("srcSize", vec2(aw, ah));
                m.set_uniform("levels", *levels as f32);
                m.set_uniform("dither", *dither);
                m.set_uniform("palCount", palette.len().min(MAX_PALETTE) as f32);
                let mut pal = vec![0f32; MAX_PALETTE * 3];
                for (i, c) in palette.iter().take(MAX_PALETTE).enumerate() {
                    pal[i * 3] = ((c >> 16) & 255) as f32 / 255.0;
                    pal[i * 3 + 1] = ((c >> 8) & 255) as f32 / 255.0;
                    pal[i * 3 + 2] = (c & 255) as f32 / 255.0;
                }
                m.set_uniform_array("palette", &pal[..]);
                gl_use_material(m);
                draw_texture_ex(app, 0.0, 0.0, WHITE, DrawTextureParams { dest_size: Some(vec2(pw as f32, ph as f32)), ..Default::default() });
                // back over application_surface, blocky
                set_camera(&camera_for(&cv.app, 0.0, 0.0, aw, ah));
                gl_use_material(&self.copy);
                draw_texture_ex(&small.texture, 0.0, 0.0, WHITE, DrawTextureParams { dest_size: Some(vec2(aw, ah)), ..Default::default() });
                restore(cv, fx, scissor);
            }
            _ => {}
        }
    }

    fn render_view(&mut self, f: &FpFrame, textures: &[Texture2D]) {
        let rt = self.views.entry((f.w, f.h)).or_insert_with(|| {
            let rt = render_target_ex(f.w, f.h, RenderTargetParams { sample_count: 1, depth: true });
            rt.texture.set_filter(FilterMode::Nearest);
            rt
        });
        set_camera(&Cam3 { m: Mat4::from_cols_array(&f.mvp), pass: rt.render_pass.clone() });
        let gl = unsafe { get_internal_gl() }.quad_gl;
        gl.scissor(None);
        clear_background(Color::new(f.fog[0], f.fog[1], f.fog[2], 1.0));
        let m = &self.world;
        m.set_uniform("fogColor", vec4(f.fog[0], f.fog[1], f.fog[2], f.fog[3]));
        m.set_uniform("fogRange", vec2(f.fog_range[0], f.fog_range[1]));
        m.set_uniform("eye", vec3(f.eye[0], f.eye[1], f.eye[2]));
        gl_use_material(m);
        draw_faces(&f.faces, textures);
        draw_faces(&f.sprites, textures);
    }
}

impl Default for FpGpu {
    fn default() -> Self {
        Self::new()
    }
}

fn restore(cv: &Canvas, fx: &mut GpuFx, scissor: Option<[f32; 4]>) {
    cv.restore_camera();
    crate::scissor::set(scissor, cv);
    fx.use_current();
}

/// Quads in runs of the same texture, in chunks that fit macroquad's draw-call buffers.
fn draw_faces(faces: &[Face], textures: &[Texture2D]) {
    const QUADS: usize = 800;
    let gl = unsafe { get_internal_gl() }.quad_gl;
    gl.draw_mode(DrawMode::Triangles);
    let mut verts: Vec<Vertex> = Vec::with_capacity(QUADS * 4);
    let mut idx: Vec<u16> = Vec::with_capacity(QUADS * 6);
    let mut i = 0;
    while i < faces.len() {
        let tex = faces[i].tex;
        let t = tex.and_then(|t| textures.get(t as usize));
        // a tiny inset keeps nearest sampling inside the frame (no seams from the neighbours)
        let (eu, ev) = t.map(|t| (0.01 / t.width().max(1.0), 0.01 / t.height().max(1.0))).unwrap_or((0.0, 0.0));
        gl.texture(t);
        verts.clear();
        idx.clear();
        while i < faces.len() && faces[i].tex == tex && verts.len() < QUADS * 4 {
            let q = &faces[i];
            let c = Color::from_rgba(q.color[0], q.color[1], q.color[2], q.color[3]);
            let (u0, v0, u1, v1) = (q.uv[0] + eu, q.uv[1] + ev, q.uv[2] - eu, q.uv[3] - ev);
            let uvs = [(u0, v0), (u1, v0), (u1, v1), (u0, v1)];
            let b = verts.len() as u16;
            for (p, (u, v)) in q.p.iter().zip(uvs) {
                verts.push(Vertex::new(p[0], p[1], p[2], u, v, c));
            }
            idx.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
            i += 1;
        }
        gl.geometry(&verts, &idx);
    }
}

/// `DrawCmd::Mesh`: a triangle list in the current 2D coordinates.
pub fn draw_mesh(tex: Option<&Texture2D>, verts: &[MVert]) {
    const CHUNK: usize = 3 * 1000;
    let gl = unsafe { get_internal_gl() }.quad_gl;
    gl.texture(tex);
    gl.draw_mode(DrawMode::Triangles);
    let mut vs: Vec<Vertex> = Vec::with_capacity(CHUNK.min(verts.len()));
    for chunk in verts.chunks(CHUNK) {
        vs.clear();
        vs.extend(chunk.iter().map(|v| Vertex::new(v.x, v.y, 0.0, v.u, v.v, Color::from_rgba(v.color[0], v.color[1], v.color[2], v.color[3]))));
        let idx: Vec<u16> = (0..vs.len() as u16).collect();
        gl.geometry(&vs, &idx);
    }
}
