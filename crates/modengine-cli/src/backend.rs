//! Window, rendering, input and audio through macroquad.

use crate::script_input::ScriptedInput;
use macroquad::prelude::*;
use modengine::draw::{parse_color, DrawCmd, Rgba};
use modengine::input::{Input, KEY_NAMES};
use modengine::Engine;

#[derive(Default)]
pub struct RunOpts {
    /// Save a PNG of the window after this many rendered frames, then quit.
    pub screenshot: Option<(u64, String)>,
    pub input: Option<ScriptedInput>,
    /// On-screen touch controls from `[android]`: `Some(true)` shows them (the mouse acts
    /// as a finger on desktop), `None` only on Android.
    pub touch: Option<bool>,
    /// `--instance N` (several copies of the game for multiplayer tests): the window goes
    /// to its own spot, side by side with the others.
    pub instance: Option<u32>,
    /// Terminal test run: screenshots at logic steps, lockstep, hidden window (test_run.rs).
    pub test: Option<crate::test_run::WindowTest>,
}

fn col(c: Rgba) -> Color {
    Color::new(c.r, c.g, c.b, c.a)
}

fn key_codes(name: &str) -> Vec<KeyCode> {
    use KeyCode::*;
    let one = |k| vec![k];
    match name {
        "left" => one(Left),
        "right" => one(Right),
        "up" => one(Up),
        "down" => one(Down),
        "space" => one(Space),
        "enter" => vec![Enter, KpEnter],
        "escape" => one(Escape),
        "shift" => vec![LeftShift, RightShift],
        "ctrl" => vec![LeftControl, RightControl],
        "alt" => vec![LeftAlt, RightAlt],
        "tab" => one(Tab),
        "backspace" => one(Backspace),
        "delete" => one(Delete),
        "insert" => one(Insert),
        "home" => one(Home),
        "end" => one(End),
        "page_up" => one(PageUp),
        "page_down" => one(PageDown),
        "minus" => one(Minus),
        "equal" => one(Equal),
        "comma" => one(Comma),
        "period" => one(Period),
        "slash" => one(Slash),
        s if s.len() == 1 => {
            let c = s.chars().next().unwrap();
            let k = match c {
                'a' => A,
                'b' => B,
                'c' => C,
                'd' => D,
                'e' => E,
                'f' => F,
                'g' => G,
                'h' => H,
                'i' => I,
                'j' => J,
                'k' => K,
                'l' => L,
                'm' => M,
                'n' => N,
                'o' => O,
                'p' => P,
                'q' => Q,
                'r' => R,
                's' => S,
                't' => T,
                'u' => U,
                'v' => V,
                'w' => W,
                'x' => X,
                'y' => Y,
                'z' => Z,
                '0' => Key0,
                '1' => Key1,
                '2' => Key2,
                '3' => Key3,
                '4' => Key4,
                '5' => Key5,
                '6' => Key6,
                '7' => Key7,
                '8' => Key8,
                '9' => Key9,
                _ => return vec![],
            };
            vec![k]
        }
        s if s.starts_with('f') => match &s[1..] {
            "1" => one(F1),
            "2" => one(F2),
            "3" => one(F3),
            "4" => one(F4),
            "5" => one(F5),
            "6" => one(F6),
            "7" => one(F7),
            "8" => one(F8),
            "9" => one(F9),
            "10" => one(F10),
            "11" => one(F11),
            "12" => one(F12),
            _ => vec![],
        },
        _ => vec![],
    }
}

/// Collects keyboard/mouse edges between logic steps so no press is lost
/// (or seen twice) when frame rate and step rate differ.
struct InputCollector {
    codes: Vec<Vec<KeyCode>>,
    pressed: Vec<bool>,
    released: Vec<bool>,
    mpressed: [bool; 3],
    mreleased: [bool; 3],
    wheel: f32,
    pad: Option<crate::touch::VirtualPad>,
    /// Desktop testing of the touch pad: the mouse is a finger.
    mouse_finger: bool,
    /// Characters typed since the last step (`keyboard_string`).
    typed: String,
}

const BUTTONS: [MouseButton; 3] = [MouseButton::Left, MouseButton::Right, MouseButton::Middle];

impl InputCollector {
    fn new() -> Self {
        let n = KEY_NAMES.len();
        InputCollector {
            codes: KEY_NAMES.iter().map(|k| key_codes(k)).collect(),
            pressed: vec![false; n],
            released: vec![false; n],
            mpressed: [false; 3],
            mreleased: [false; 3],
            wheel: 0.0,
            pad: None,
            mouse_finger: false,
            typed: String::new(),
        }
    }

    /// Window-pixel positions of the fingers on the screen this frame. A finger lifted this
    /// frame still counts once, so a quick tap (down and up between two frames) is not lost.
    fn fingers(&self) -> Vec<(f32, f32)> {
        let mut v: Vec<(f32, f32)> = touches().iter().filter(|t| t.phase != TouchPhase::Cancelled).map(|t| (t.position.x, t.position.y)).collect();
        if self.mouse_finger && is_mouse_button_down(MouseButton::Left) {
            v.push(mouse_position());
        }
        v
    }

    fn collect_frame(&mut self) {
        let p = get_keys_pressed();
        let r = get_keys_released();
        for (i, cs) in self.codes.iter().enumerate() {
            if cs.iter().any(|c| p.contains(c)) {
                self.pressed[i] = true;
            }
            if cs.iter().any(|c| r.contains(c)) {
                self.released[i] = true;
            }
        }
        for (i, b) in BUTTONS.iter().enumerate() {
            self.mpressed[i] |= is_mouse_button_pressed(*b);
            self.mreleased[i] |= is_mouse_button_released(*b);
        }
        self.wheel += mouse_wheel().1;
        while let Some(c) = get_char_pressed() {
            if self.typed.len() < 256 {
                self.typed.push(c);
            }
        }
        let fingers = self.fingers();
        if let Some(pad) = &mut self.pad {
            pad.layout(screen_width(), screen_height());
            let (p, r) = pad.update(&fingers);
            p.into_iter().for_each(|i| self.pressed[i] = true);
            r.into_iter().for_each(|i| self.released[i] = true);
        }
    }

    fn apply(&mut self, inp: &mut Input, lay: &Layout) {
        for (i, cs) in self.codes.iter().enumerate() {
            inp.down[i] = cs.iter().any(|c| is_key_down(*c)) || self.pad.as_ref().is_some_and(|p| p.held[i]);
            inp.pressed[i] = std::mem::take(&mut self.pressed[i]);
            inp.released[i] = std::mem::take(&mut self.released[i]);
        }
        for (i, b) in BUTTONS.iter().enumerate() {
            inp.mouse_down[i] = is_mouse_button_down(*b);
            inp.mouse_pressed[i] = std::mem::take(&mut self.mpressed[i]);
            inp.mouse_released[i] = std::mem::take(&mut self.mreleased[i]);
        }
        inp.wheel = std::mem::take(&mut self.wheel) as f64;
        inp.typed.push_str(&std::mem::take(&mut self.typed));
        let (mx, my) = mouse_position();
        inp.mouse_x = ((mx - lay.ox) / lay.scale) as f64;
        inp.mouse_y = ((my - lay.oy) / lay.scale) as f64;
        inp.touches = self.fingers().into_iter().map(|(x, y)| (((x - lay.ox) / lay.scale) as f64, ((y - lay.oy) / lay.scale) as f64)).collect();
    }
}

struct Layout {
    scale: f32,
    ox: f32,
    oy: f32,
    w: f32,
    h: f32,
}

fn layout(rw: f32, rh: f32, pixel_perfect: bool) -> Layout {
    let (sw, sh) = (screen_width(), screen_height());
    let mut s = (sw / rw).min(sh / rh);
    if pixel_perfect && s >= 1.0 {
        s = s.floor();
    }
    let (w, h) = (rw * s, rh * s);
    Layout { scale: s, ox: ((sw - w) / 2.0).floor(), oy: ((sh - h) / 2.0).floor(), w, h }
}

pub(crate) struct Gpu {
    /// Shaders and blend modes (see gpu_fx.rs).
    fx: std::cell::RefCell<crate::gpu_fx::GpuFx>,
    textures: Vec<Texture2D>,
    /// Baked fonts (`fonts/`): glyph page texture + metrics, drawn quad by quad.
    atlases: Vec<Option<(Texture2D, std::sync::Arc<modengine::font::GlyphAtlas>)>>,
    fonts: Vec<Option<Font>>,
    font_sizes: Vec<f32>,
    default_size: f32,
    /// Pixel-art game (`smooth = false`): text from the built-in face is placed glyph by glyph
    /// on whole pixels.
    pixel_text: bool,
    /// Sprite atlases and cached RotSprite images (see sprite_draw.rs).
    sprites: std::cell::RefCell<crate::sprite_draw::SpriteDraw>,
    /// First-person views and the pixelate post-process (see fpview.rs).
    fp: std::cell::RefCell<crate::fpview::FpGpu>,
}

impl Gpu {
    /// Uploads sprites and fonts and installs the text measuring of the backend.
    pub(crate) fn new(e: &mut Engine) -> Gpu {
        let smooth = e.config.display.smooth;
        let filter = if smooth { FilterMode::Linear } else { FilterMode::Nearest };
        let textures: Vec<Texture2D> = e
            .sprites
            .iter()
            .map(|s| {
                let t = Texture2D::from_rgba8(s.sheet_w as u16, s.sheet_h as u16, &s.rgba);
                t.set_filter(filter);
                t
            })
            .collect();
        let atlases: Vec<_> = e
            .fonts
            .iter()
            .map(|f| {
                f.atlas.as_ref().map(|a| {
                    let t = Texture2D::from_rgba8(a.w as u16, a.h as u16, &a.rgba);
                    // pixel fonts stay crisp; antialiased ones follow the game's filter
                    t.set_filter(if f.antialias { filter } else { FilterMode::Nearest });
                    (t, a.clone())
                })
            })
            .collect();
        // macroquad's built-in face (ProggyClean) has a linear-filtered glyph atlas: in a pixel-art
        // game glyphs at half-pixel advances come out blurred (an `o` read as `c`). Follow the game.
        get_default_font().set_filter(filter);
        let mut fonts: Vec<Option<Font>> = Vec::new();
        for f in &e.fonts {
            if let Some(err) = &f.error {
                eprintln!("[error] font '{}': {err} (using the default font)", f.name);
            }
            if f.atlas.is_some() || f.data.is_empty() || f.error.is_some() {
                // baked (drawn from its atlas) or the built-in font at another size
                fonts.push(None);
                continue;
            }
            match load_ttf_font_from_bytes(&f.data) {
                Ok(mut font) => {
                    font.set_filter(filter);
                    fonts.push(Some(font));
                }
                Err(err) => {
                    eprintln!("[error] font '{}': {err:?} (using the default font)", f.name);
                    fonts.push(None);
                }
            }
        }
        // particle shapes are smooth textures even in pixel-perfect games
        for k in 0..modengine::particles::SHAPE_NAMES.len() {
            if let Some(t) = textures.get(e.parts.shape_base as usize + k) {
                t.set_filter(FilterMode::Linear);
            }
        }
        let measure_atlases: Vec<_> = atlases.iter().map(|a| a.as_ref().map(|(_, at)| at.clone())).collect();
        let shapes = e.parts.shape_base as usize..e.parts.shape_base as usize + modengine::particles::SHAPE_NAMES.len();
        let sprites = std::cell::RefCell::new(crate::sprite_draw::SpriteDraw::new(&e.sprites, &textures, smooth, shapes));
        let gpu = Gpu {
            sprites,
            fp: std::cell::RefCell::new(crate::fpview::FpGpu::new()),
            fx: std::cell::RefCell::new(crate::gpu_fx::GpuFx::new(e)),
            textures,
            atlases,
            font_sizes: e.fonts.iter().map(|f| f.size).collect(),
            fonts: fonts.clone(),
            default_size: e.config.display.font_size,
            pixel_text: !smooth,
        };
        let (sizes, default_size) = (gpu.font_sizes.clone(), gpu.default_size);
        e.measure = Box::new(move |font, text, scale| {
            if let Some(Some(a)) = font.and_then(|i| measure_atlases.get(i as usize)) {
                return a.measure(text, scale);
            }
            let f = font.and_then(|i| fonts.get(i as usize)).and_then(|f| f.as_ref());
            let size = font_size(&sizes, default_size, font);
            let lines: Vec<&str> = text.split('\n').collect();
            let w = lines.iter().map(|l| measure_text(l, f, size.round() as u16, scale).width).fold(0.0, f32::max);
            (w, size * scale * lines.len() as f32)
        });
        gpu
    }
}

fn text_metrics(gpu: &Gpu, font: Option<u32>) -> (Option<&Font>, f32) {
    let f = font.and_then(|f| gpu.fonts.get(f as usize)).and_then(|f| f.as_ref());
    (f, font_size(&gpu.font_sizes, gpu.default_size, font))
}

/// Pixel size of a font asset (`fonts/<name>.toml` `size`), also for the ones drawn with the
/// built-in face (no `file`, or a face that failed to load); `[display] font_size` without
/// a font.
fn font_size(sizes: &[f32], default_size: f32, font: Option<u32>) -> f32 {
    font.and_then(|i| sizes.get(i as usize)).copied().unwrap_or(default_size)
}

#[allow(clippy::too_many_arguments)]
fn draw_text_block(gpu: &Gpu, text: &str, x: f32, y: f32, font: Option<u32>, scale: f32, color: Color, halign: u8, valign: u8) {
    if let Some(Some((tex, atlas))) = font.and_then(|f| gpu.atlases.get(f as usize)) {
        return draw_atlas_text(tex, atlas, text, x, y, scale, color, halign, valign);
    }
    let (f, size) = text_metrics(gpu, font);
    let size16 = size.round().max(1.0) as u16;
    let lines: Vec<&str> = text.split('\n').collect();
    let line_h = size * scale;
    let total = line_h * lines.len() as f32;
    let ascent = measure_text("Ag", f, size16, scale).offset_y;
    let mut top = y - match valign {
        1 => total / 2.0,
        2 => total,
        _ => 0.0,
    };
    for line in lines {
        let w = measure_text(line, f, size16, scale).width;
        let lx = x - match halign {
            1 => w / 2.0,
            2 => w,
            _ => 0.0,
        };
        let ly = (top + ascent).round();
        if gpu.pixel_text {
            // glyph by glyph on whole pixels: the face's advances are fractional, and a glyph
            // that lands on a half pixel loses or doubles a column (`Diablillo` read `Diablillc`)
            let mut pen = 0.0f32;
            let mut buf = [0u8; 4];
            for ch in line.chars() {
                let g: &str = ch.encode_utf8(&mut buf);
                draw_text_ex(g, (lx + pen).round(), ly, TextParams { font: f, font_size: size16, font_scale: scale, color, ..Default::default() });
                pen += measure_text(g, f, size16, scale).width;
            }
        } else {
            draw_text_ex(line, lx.round(), ly, TextParams { font: f, font_size: size16, font_scale: scale, color, ..Default::default() });
        }
        top += line_h;
    }
}

/// Text with a baked font: one textured quad per glyph (the same layout the editor previews).
#[allow(clippy::too_many_arguments)]
fn draw_atlas_text(tex: &Texture2D, atlas: &modengine::font::GlyphAtlas, text: &str, x: f32, y: f32, scale: f32, color: Color, halign: u8, valign: u8) {
    let lines: Vec<&str> = text.split('\n').collect();
    let line_h = atlas.line_height * scale;
    let total = line_h * lines.len() as f32;
    let mut top = y - match valign {
        1 => total / 2.0,
        2 => total,
        _ => 0.0,
    };
    for line in lines {
        let w = atlas.line_width(line) * scale;
        let lx = x - match halign {
            1 => w / 2.0,
            2 => w,
            _ => 0.0,
        };
        for q in atlas.layout_line(line) {
            let g = q.glyph;
            draw_texture_ex(
                tex,
                (lx + q.x * scale).round(),
                (top + q.y * scale).round(),
                color,
                DrawTextureParams { dest_size: Some(vec2(g.w as f32 * scale, g.h as f32 * scale)), source: Some(Rect::new(g.x as f32, g.y as f32, g.w as f32, g.h as f32)), ..Default::default() },
            );
        }
        top += line_h;
    }
}

/// Replays the engine's draw list into the canvas (application_surface, surfaces, post_draw).
pub(crate) fn exec(e: &Engine, gpu: &Gpu, cv: &mut crate::canvas::Canvas) {
    gpu.fx.borrow_mut().begin_frame();
    gpu.sprites.borrow_mut().begin_frame();
    cv.begin(e);
    // clip rectangle in force (fpview.rs puts it back after drawing into its own targets)
    let mut scissor: Option<[f32; 4]> = None;
    for c in &e.cmds {
        match c {
            DrawCmd::Shader(_) | DrawCmd::Uniform { .. } | DrawCmd::Sampler { .. } | DrawCmd::Blend(_) => gpu.fx.borrow_mut().apply(c, &gpu.textures),
            DrawCmd::View { .. } | DrawCmd::Gui | DrawCmd::Target(_) | DrawCmd::PostDraw { .. } => {
                cv.apply(c, e, &mut gpu.fx.borrow_mut());
                // a new camera / target starts unclipped
                scissor = None;
                crate::scissor::set(None, cv);
            }
            DrawCmd::Surface { .. } => cv.apply(c, e, &mut gpu.fx.borrow_mut()),
            DrawCmd::SpriteFx(fx) => gpu.sprites.borrow_mut().set_fx(*fx),
            DrawCmd::Scissor(r) => {
                scissor = *r;
                crate::scissor::set(*r, cv)
            }
            DrawCmd::Mesh { tex, verts } => crate::fpview::draw_mesh(tex.and_then(|t| gpu.textures.get(t as usize)), verts),
            DrawCmd::FpView { .. } | DrawCmd::PostFx { .. } => gpu.fp.borrow_mut().apply(c, &gpu.textures, cv, &mut gpu.fx.borrow_mut(), scissor),
            DrawCmd::Clear(c) => clear_background(col(*c)),
            DrawCmd::Sprite { spr, frame, x, y, sx, sy, angle, color } => {
                let shader = gpu.fx.borrow().shader_active();
                gpu.sprites.borrow_mut().draw(&e.sprites, &gpu.textures[*spr as usize], *spr, *frame, *x, *y, *sx, *sy, *angle, col(*color), shader);
            }
            DrawCmd::Stretched { spr, frame, x, y, w, h, color } => {
                let s = &e.sprites[*spr as usize];
                if let Some(fr) = s.frames.get(*frame as usize) {
                    draw_texture_ex(
                        &gpu.textures[*spr as usize],
                        *x,
                        *y,
                        col(*color),
                        DrawTextureParams {
                            dest_size: Some(vec2(*w, *h)),
                            source: Some(Rect::new(fr[0] as f32, fr[1] as f32, fr[2] as f32, fr[3] as f32)),
                            ..Default::default()
                        },
                    );
                }
            }
            DrawCmd::Rect { x, y, w, h, color, outline } => {
                if *outline {
                    draw_rectangle_lines(*x, *y, *w, *h, 1.0, col(*color));
                } else {
                    draw_rectangle(*x, *y, *w, *h, col(*color));
                }
            }
            DrawCmd::Circle { x, y, r, color, outline } => {
                if *outline {
                    draw_circle_lines(*x, *y, *r, 1.0, col(*color));
                } else {
                    draw_circle(*x, *y, *r, col(*color));
                }
            }
            DrawCmd::Line { x1, y1, x2, y2, width, color } => draw_line(*x1, *y1, *x2, *y2, *width, col(*color)),
            DrawCmd::Tri { pts, color, outline } => {
                let (a, b, c) = (vec2(pts[0], pts[1]), vec2(pts[2], pts[3]), vec2(pts[4], pts[5]));
                if *outline {
                    draw_triangle_lines(a, b, c, 1.0, col(*color));
                } else {
                    draw_triangle(a, b, c, col(*color));
                }
            }
            DrawCmd::Text { x, y, text, font, scale, color, halign, valign } => {
                draw_text_block(gpu, text, *x, *y, *font, *scale, col(*color), *halign, *valign)
            }
        }
    }
    crate::scissor::set(None, cv);
    gpu.fx.borrow_mut().end_frame();
    cv.end(e);
}

pub async fn run(mut e: Engine, mut opts: RunOpts) {
    // the engine's software mixer (pitch, pan, fades, groups...) plays through the audio
    // thread of macroquad's quad-snd (see vendor/quad-snd)
    modengine::audio::decode_all(&mut e);
    let mixer = modengine::audio::handle(&mut e);
    mixer.lock().unwrap_or_else(|p| p.into_inner()).realtime = true;
    quad_snd::set_mix_hook(Some(Box::new(move |buf: &mut [f32]| {
        if let Ok(mut m) = mixer.lock() {
            m.render(buf);
        }
    })));
    let gpu = Gpu::new(&mut e);
    let filter = if e.config.display.smooth { FilterMode::Linear } else { FilterMode::Nearest };


    let (rw, rh) = (e.config.display.width, e.config.display.height);
    let mut canvas = crate::canvas::Canvas::new(rw, rh, filter);
    let letterbox = parse_color(&e.config.display.letterbox).unwrap_or(0);
    let pixel_perfect = e.config.display.pixel_perfect;

    prevent_quit();
    if let Some(n) = opts.instance {
        let d = &e.config.display;
        let w = d.width * d.scale.max(1);
        let x = 20 + (n.max(1) - 1) % 3 * (w + 16);
        let y = 40 + (n.max(1) - 1) / 3 * (d.height * d.scale.max(1) + 48);
        macroquad::miniquad::window::set_window_position(x, y);
    }
    if opts.test.as_ref().is_some_and(|t| t.hidden) {
        crate::test_run::hide_window();
    }
    e.test_starting();
    e.start();
    e.test_started();
    let mut collector = InputCollector::new();
    let mut cursor = crate::scissor::CursorState::new();
    if opts.touch.unwrap_or(cfg!(target_os = "android")) {
        collector.pad = crate::touch::VirtualPad::from_config(&e.config.android);
        collector.mouse_finger = !cfg!(target_os = "android");
    }
    let mut acc = 0.0f64;
    let mut frames: u64 = 0;
    let mut shot_n = 0;
    let mut fullscreen = e.config.display.fullscreen;

    loop {
        if e.quit || is_quit_requested() {
            e.shutdown();
            break;
        }
        if is_key_pressed(KeyCode::F3) {
            e.debug = !e.debug;
        }
        if is_key_pressed(KeyCode::F4) {
            fullscreen = !fullscreen;
            set_fullscreen(fullscreen);
        }
        let lay = layout(rw as f32, rh as f32, pixel_perfect);
        collector.collect_frame();

        let dt = 1.0 / e.step_rate();
        acc += (get_frame_time() as f64).min(0.25);
        if opts.test.as_ref().is_some_and(|t| t.lockstep) {
            // one logic step per frame, whatever the real time
            acc = dt;
        }
        let mut ticks = 0;
        while acc >= dt && ticks < 5 {
            collector.apply(&mut e.input, &lay);
            if let Some(si) = &opts.input {
                si.apply_to(e.steps + 1, &mut e);
            }
            e.test_tick();
            acc -= dt;
            ticks += 1;
            if e.quit {
                break;
            }
        }
        if ticks == 5 {
            acc = 0.0;
        }
        e.fps = get_fps() as f64;
        cursor.update(&e);

        // audio already went to the mixer; e.audio is only a log (tests)
        e.audio.clear();

        if ticks > 0 || frames == 0 {
            e.test_render();
            exec(&e, &gpu, &mut canvas);
        }

        set_default_camera();
        if let Some(t) = &mut opts.test {
            // the canvas is flushed: screenshots of this step, end of a test run
            if t.after_frame(&e, canvas.output()) {
                e.shutdown();
                break;
            }
        }
        clear_background(col(Rgba::from_rgb(letterbox, 1.0)));
        canvas.present(lay.ox, lay.oy, lay.w, lay.h);
        if let Some(pad) = &collector.pad {
            pad.draw();
        }
        overlay(&e);

        frames += 1;
        if is_key_pressed(KeyCode::F12) {
            shot_n += 1;
            // exported games can't write inside their pack: next to the saves instead
            let dir = if modengine::vfs::is_mounted(&e.dir) { e.save_dir.parent().unwrap_or(&e.save_dir).to_path_buf() } else { e.dir.clone() };
            let _ = std::fs::create_dir_all(&dir);
            let path = dir.join(format!("screenshot_{shot_n}.png"));
            get_screen_data().export_png(&path.to_string_lossy());
            println!("saved {}", path.display());
        }
        if let Some((at, path)) = &opts.screenshot {
            if frames >= *at {
                get_screen_data().export_png(path);
                println!("saved {path}");
                opts.screenshot = None;
                e.shutdown();
                break;
            }
        }
        next_frame().await;
    }
    crate::test_run::finish_window(&mut e);
}

fn overlay(e: &Engine) {
    if e.debug {
        let text = format!(
            "{} fps  |  room {}  |  {} instances  |  step {}",
            e.fps,
            e.rooms[e.room].name,
            e.insts.iter().filter(|i| i.alive).count(),
            e.steps
        );
        draw_rectangle(0.0, 0.0, screen_width(), 22.0, Color::new(0.0, 0.0, 0.0, 0.6));
        draw_text(&text, 6.0, 16.0, 18.0, WHITE);
    }
    if !e.errors.is_empty() {
        let shown: Vec<&String> = e.errors.iter().rev().take(3).collect();
        let lines: Vec<String> = shown.iter().flat_map(|m| m.lines().take(2).map(String::from)).collect();
        let h = 8.0 + lines.len() as f32 * 18.0;
        let y0 = screen_height() - h;
        draw_rectangle(0.0, y0, screen_width(), h, Color::new(0.25, 0.0, 0.0, 0.85));
        for (i, l) in lines.iter().enumerate() {
            draw_text(l, 6.0, y0 + 18.0 + i as f32 * 18.0, 18.0, Color::new(1.0, 0.75, 0.75, 1.0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::font_size;

    #[test]
    fn built_in_face_fonts_keep_their_size() {
        // a font asset without `file` (built-in face) used to fall back to [display] font_size
        let sizes = [8.0, 14.0];
        assert_eq!(font_size(&sizes, 16.0, Some(0)), 8.0);
        assert_eq!(font_size(&sizes, 16.0, Some(1)), 14.0);
        assert_eq!(font_size(&sizes, 16.0, None), 16.0);
        assert_eq!(font_size(&sizes, 16.0, Some(9)), 16.0);
    }
}
