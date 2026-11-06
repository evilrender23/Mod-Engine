//! The game's render targets: application_surface (the frame the draw events paint), the
//! surfaces scripts create (`surface_*`), the final image of the `post_draw` pass, and the
//! opaque blit of all that to the window.

use crate::gpu_fx::GpuFx;
use macroquad::miniquad::PipelineParams;
use macroquad::prelude::*;
use modengine::draw::{DrawCmd, Rgba};
use modengine::Engine;
use std::collections::HashMap;

const BLIT_VSH: &str = "#version 100
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

const BLIT_FSH: &str = "#version 100
precision mediump float;
varying vec2 uv;
uniform sampler2D Texture;
void main() {
    gl_FragColor = vec4(texture2D(Texture, uv).rgb, 1.0);
}
";

/// Copies a texture ignoring its alpha (no blending): what reaches the window is opaque.
fn opaque_material() -> Material {
    load_material(ShaderSource::Glsl { vertex: BLIT_VSH, fragment: BLIT_FSH }, MaterialParams { pipeline_params: PipelineParams::default(), ..Default::default() })
        .expect("opaque blit shader")
}

fn col(c: Rgba) -> Color {
    Color::new(c.r, c.g, c.b, c.a)
}

pub fn camera_for(rt: &RenderTarget, x: f32, y: f32, w: f32, h: f32) -> Camera2D {
    Camera2D { target: vec2(x + w / 2.0, y + h / 2.0), zoom: vec2(2.0 / w, 2.0 / h), render_target: Some(rt.clone()), ..Default::default() }
}

fn new_target(w: u32, h: u32, filter: FilterMode) -> RenderTarget {
    let rt = render_target(w, h);
    rt.texture.set_filter(filter);
    rt
}

pub struct Canvas {
    /// application_surface: display width × height.
    pub app: RenderTarget,
    /// Final image when the frame has a `post_draw` pass.
    post: Option<RenderTarget>,
    surfaces: HashMap<u32, (u32, u32, RenderTarget)>,
    filter: FilterMode,
    /// Camera rectangle of the frame (last View / Gui command).
    view: (f32, f32, f32, f32),
    /// Surface being drawn to (None: the frame).
    target: Option<u32>,
    in_post: bool,
    /// The last rendered frame had a post_draw pass: show `post` instead of `app`.
    show_post: bool,
    opaque: Material,
}

impl Canvas {
    pub fn new(w: u32, h: u32, filter: FilterMode) -> Self {
        Canvas {
            app: new_target(w, h, filter),
            post: None,
            surfaces: HashMap::new(),
            filter,
            view: (0.0, 0.0, w as f32, h as f32),
            target: None,
            in_post: false,
            show_post: false,
            opaque: opaque_material(),
        }
    }

    fn frame(&self) -> &RenderTarget {
        match (&self.post, self.in_post) {
            (Some(p), true) => p,
            _ => &self.app,
        }
    }

    fn set_frame_camera(&self) {
        let (x, y, w, h) = self.view;
        set_camera(&camera_for(self.frame(), x, y, w, h));
    }

    /// Current coordinate rectangle and the pixel size of the target it maps to (for
    /// `gpu_set_scissor`, see scissor.rs).
    pub fn clip_space(&self) -> ((f32, f32, f32, f32), (f32, f32)) {
        match self.target.and_then(|id| self.surfaces.get(&id)) {
            Some((w, h, _)) => ((0.0, 0.0, *w as f32, *h as f32), (*w as f32, *h as f32)),
            None => {
                let t = &self.frame().texture;
                (self.view, (t.width(), t.height()))
            }
        }
    }

    /// The image the window shows (application_surface or the post_draw result).
    pub fn output(&self) -> &Texture2D {
        match (&self.post, self.show_post) {
            (Some(p), true) => &p.texture,
            _ => &self.app.texture,
        }
    }

    pub fn surface_texture(&self, id: u32) -> Option<&Texture2D> {
        if id == modengine::surface::APP {
            return Some(&self.app.texture);
        }
        self.surfaces.get(&id).map(|s| &s.2.texture)
    }

    pub fn begin(&mut self, e: &Engine) {
        self.target = None;
        self.in_post = false;
        self.view = (0.0, 0.0, e.cam.w as f32, e.cam.h as f32);
        self.set_frame_camera();
    }

    /// End of a frame: GPU targets of freed surfaces go away.
    pub fn end(&mut self, e: &Engine) {
        self.show_post = self.in_post;
        self.surfaces.retain(|id, _| e.surfaces.live.contains_key(id));
        if self.target.is_some() || self.in_post {
            self.target = None;
            self.in_post = false;
        }
    }

    /// Opaque copy of `tex` over `dest` (x, y, w, h) on the current target.
    fn blit(&self, tex: &Texture2D, x: f32, y: f32, w: f32, h: f32) {
        gl_use_material(&self.opaque);
        draw_texture_ex(tex, x, y, WHITE, DrawTextureParams { dest_size: Some(vec2(w, h)), ..Default::default() });
    }

    /// Handles the target/camera commands: View, Gui, Target, Surface, PostDraw.
    pub fn apply(&mut self, c: &DrawCmd, e: &Engine, fx: &mut GpuFx) {
        match c {
            DrawCmd::View { x, y, w, h } => {
                self.view = (*x, *y, *w, *h);
                if self.target.is_none() {
                    self.set_frame_camera();
                }
            }
            DrawCmd::Gui => {
                self.view = (0.0, 0.0, e.cam.w as f32, e.cam.h as f32);
                if self.target.is_none() {
                    self.set_frame_camera();
                }
            }
            DrawCmd::Target(None) => {
                self.target = None;
                self.set_frame_camera();
            }
            DrawCmd::Target(Some((id, w, h))) => {
                let fresh = !matches!(self.surfaces.get(id), Some((sw, sh, _)) if sw == w && sh == h);
                if fresh {
                    self.surfaces.insert(*id, (*w, *h, new_target(*w, *h, self.filter)));
                }
                let rt = &self.surfaces[id].2;
                set_camera(&camera_for(rt, 0.0, 0.0, *w as f32, *h as f32));
                if fresh {
                    clear_background(Color::new(0.0, 0.0, 0.0, 0.0));
                }
                self.target = Some(*id);
            }
            DrawCmd::Surface { id, src, x, y, w, h, angle, color } => {
                // drawing a surface into itself is undefined on the GPU: skipped
                let into_app = self.target.is_none() && !self.in_post;
                if Some(*id) == self.target || (*id == modengine::surface::APP && into_app) {
                    return;
                }
                let Some(tex) = self.surface_texture(*id) else { return };
                let left = x.min(x + w);
                let top = y.min(y + h);
                draw_texture_ex(
                    tex,
                    left,
                    top,
                    col(*color),
                    DrawTextureParams {
                        dest_size: Some(vec2(w.abs(), h.abs())),
                        source: src.map(|s| Rect::new(s[0], s[1], s[2], s[3])),
                        rotation: -angle.to_radians(),
                        flip_x: *w < 0.0,
                        flip_y: *h < 0.0,
                        pivot: Some(vec2(*x, *y)),
                    },
                );
            }
            DrawCmd::PostDraw { app } => {
                let (aw, ah) = (self.app.texture.width(), self.app.texture.height());
                if self.post.as_ref().is_none_or(|p| p.texture.width() != aw || p.texture.height() != ah) {
                    self.post = Some(new_target(aw as u32, ah as u32, self.filter));
                }
                self.target = None;
                self.in_post = true;
                self.view = (0.0, 0.0, e.cam.w as f32, e.cam.h as f32);
                self.set_frame_camera();
                clear_background(BLACK);
                if *app {
                    self.blit(&self.app.texture, 0.0, 0.0, self.view.2, self.view.3);
                    fx.use_current();
                }
            }
            _ => {}
        }
    }

    /// Draws the frame to the window (current camera = the window), always opaque.
    pub fn present(&self, x: f32, y: f32, w: f32, h: f32) {
        self.blit(self.output(), x, y, w, h);
        gl_use_default_material();
    }
}

/// A frame read back from the GPU, rows top to bottom (tests and tools).
pub struct Readback {
    /// application_surface (what the draw events painted).
    pub app: Image,
    /// What the window shows before scaling (application_surface or the post_draw image).
    pub output: Image,
    /// The window itself after the final blit (`screen_width × screen_height`).
    pub window: Image,
    /// Every live surface: (id, contents).
    pub surfaces: Vec<(u32, Image)>,
}

fn flip_rows(mut img: Image) -> Image {
    let row = img.width as usize * 4;
    let mut out = Vec::with_capacity(img.bytes.len());
    for r in img.bytes.chunks(row).rev() {
        out.extend_from_slice(r);
    }
    img.bytes = out;
    img
}

/// Runs `steps` logic steps of a started-or-not game, rendering each one like the window
/// loop does, then reads every target back. Must run inside a macroquad window.
pub fn render_readback(e: &mut Engine, steps: u64) -> Readback {
    let gpu = crate::backend::Gpu::new(e);
    let filter = if e.config.display.smooth { FilterMode::Linear } else { FilterMode::Nearest };
    let mut cv = Canvas::new(e.config.display.width, e.config.display.height, filter);
    e.start();
    for _ in 0..steps.max(1) {
        e.tick();
        e.render();
        e.audio.clear();
        crate::backend::exec(e, &gpu, &mut cv);
    }
    set_default_camera();
    clear_background(Color::new(1.0, 0.0, 1.0, 0.0));
    cv.present(0.0, 0.0, screen_width(), screen_height());
    let window = flip_rows(get_screen_data());
    // render targets are stored top row first (see camera_for)
    let surfaces = cv.surfaces.iter().map(|(id, s)| (*id, s.2.texture.get_texture_data())).collect();
    Readback { app: cv.app.texture.get_texture_data(), output: cv.output().get_texture_data(), window, surfaces }
}
