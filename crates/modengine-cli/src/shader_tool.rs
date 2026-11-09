//! `modengine shader <project> [name]`: compiles a project's shaders on the GPU and shows a
//! live preview that recompiles whenever the .vsh/.fsh files change.
//!
//! --check              compile every shader (or `name`), print the result and exit (exit code 1
//!                      when one fails)
//! --sprite S           preview on sprite S (default: a test image)
//! --screenshot F.png   save the preview after --frames N (default 30) and exit
//! --uniform n=a,b,...  value for a float uniform (repeatable)
//!
//! Float uniforms named `u_time`, `time` or `iTime` get the seconds since start,
//! `u_resolution` the preview size, samplers the preview image and other floats 1.

use crate::gpu_fx::compile;
use macroquad::prelude::*;
use modengine::shader::{load_assets, ShaderAsset, UType};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub struct Opts {
    pub dir: PathBuf,
    pub name: Option<String>,
    pub check: bool,
    pub sprite: Option<String>,
    pub screenshot: Option<String>,
    pub frames: u64,
    pub uniforms: Vec<(String, Vec<f32>)>,
}

fn test_image() -> (u16, u16, Vec<u8>) {
    let n = 128u16;
    let mut v = Vec::with_capacity(n as usize * n as usize * 4);
    for y in 0..n {
        for x in 0..n {
            let check = ((x / 16) + (y / 16)) % 2 == 0;
            let (r, g, b) = (x as f32 / n as f32, y as f32 / n as f32, 1.0 - x as f32 / n as f32);
            let k = if check { 1.0 } else { 0.75 };
            v.extend_from_slice(&[(r * 255.0 * k) as u8, (g * 255.0 * k) as u8, (b * 200.0 * k + 40.0) as u8, 255]);
        }
    }
    (n, n, v)
}

fn sprite_image(dir: &Path, name: &str) -> Option<(u16, u16, Vec<u8>)> {
    let p = modengine::project::Project::load(dir).ok()?;
    let s = p.sprites.into_iter().find(|s| s.name == name)?;
    // first frame only
    let f = s.frames[0];
    let mut v = Vec::with_capacity((f[2] * f[3] * 4) as usize);
    for y in 0..f[3] {
        let row = (((f[1] + y) * s.sheet_w + f[0]) * 4) as usize;
        v.extend_from_slice(&s.rgba[row..row + (f[2] * 4) as usize]);
    }
    Some((f[2] as u16, f[3] as u16, v))
}

fn stamp(s: &ShaderAsset) -> Vec<Option<SystemTime>> {
    [&s.vsh_path, &s.fsh_path].iter().map(|p| p.as_ref().and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())).collect()
}

/// Every shader compiled once; prints `ok name` / `error name` lines. Returns false on failure.
pub fn check_all(shaders: &[ShaderAsset], only: Option<&str>) -> bool {
    let mut ok = true;
    let mut any = false;
    for s in shaders.iter().filter(|s| only.is_none_or(|n| n == s.name)) {
        any = true;
        for p in s.problems().into_iter().chain(s.warnings()) {
            println!("warning {}: {p}", s.name);
        }
        match compile(Some(s), 0) {
            Ok(_) => println!("ok {}", s.name),
            Err(msg) => {
                ok = false;
                println!("error {}:", s.name);
                for l in msg.lines() {
                    println!("  {l}");
                }
            }
        }
    }
    if !any {
        println!("error: no shader{}", only.map(|n| format!(" named '{n}'")).unwrap_or_default());
        return false;
    }
    ok
}

/// `modengine shader ...` (arguments after `shader`).
pub fn main_cli(args: &[String]) {
    let mut o = Opts { dir: PathBuf::from("."), name: None, check: false, sprite: None, screenshot: None, frames: 30, uniforms: vec![] };
    let mut pos = vec![];
    let mut i = 0;
    while i < args.len() {
        let flag = args[i].clone();
        let mut val = || {
            i += 1;
            args.get(i).cloned().unwrap_or_default()
        };
        match flag.as_str() {
            "--check" => o.check = true,
            "--sprite" => o.sprite = Some(val()),
            "--screenshot" => o.screenshot = Some(val()),
            "--frames" => o.frames = val().parse().unwrap_or(30),
            "--uniform" => {
                let v = val();
                if let Some((n, list)) = v.split_once('=') {
                    o.uniforms.push((n.to_string(), list.split(',').filter_map(|x| x.trim().parse().ok()).collect()));
                }
            }
            other => pos.push(other.to_string()),
        }
        i += 1;
    }
    if let Some(d) = pos.first() {
        o.dir = PathBuf::from(d);
    }
    o.name = pos.get(1).cloned();
    let title = format!("Shader{}", o.name.as_ref().map(|n| format!(": {n}")).unwrap_or_default());
    let mut conf = macroquad::window::Conf {
        window_title: title,
        window_width: if o.check { 160 } else { 520 },
        window_height: if o.check { 90 } else { 420 },
        window_resizable: true,
        high_dpi: false,
        ..Default::default()
    };
    crate::display::prepare(&mut conf);
    macroquad::Window::from_config(conf, run(o));
}

pub async fn run(o: Opts) {
    let shaders = match load_assets(&o.dir) {
        Ok(s) => s,
        Err(e) => {
            println!("error: {e}");
            std::process::exit(1)
        }
    };
    if o.check {
        let ok = check_all(&shaders, o.name.as_deref());
        std::process::exit(if ok { 0 } else { 1 });
    }
    let Some(mut cur) = shaders.into_iter().find(|s| o.name.as_ref().is_none_or(|n| *n == s.name)) else {
        println!("error: no shader{}", o.name.map(|n| format!(" named '{n}'")).unwrap_or_default());
        std::process::exit(1)
    };
    let (w, h, px) = o.sprite.as_deref().and_then(|s| sprite_image(&o.dir, s)).unwrap_or_else(test_image);
    let tex = Texture2D::from_rgba8(w, h, &px);
    tex.set_filter(FilterMode::Nearest);
    let mut mat = compile(Some(&cur), 0);
    let mut seen = stamp(&cur);
    let start = get_time();
    let mut frames = 0u64;
    loop {
        // live reload
        if frames % 20 == 0 {
            let now_stamp = stamp(&cur);
            if now_stamp != seen {
                seen = now_stamp;
                if let Some(s) = load_assets(&o.dir).ok().and_then(|l| l.into_iter().find(|s| s.name == cur.name)) {
                    cur = s;
                    mat = compile(Some(&cur), 0);
                }
            }
        }
        clear_background(Color::from_rgba(40, 42, 54, 255));
        let (sw, sh) = (screen_width(), screen_height() - 28.0);
        let k = ((sw - 40.0) / w as f32).min((sh - 40.0) / h as f32).max(0.1);
        let (dw, dh) = (w as f32 * k, h as f32 * k);
        let (x, y) = ((sw - dw) / 2.0, (sh - dh) / 2.0);
        match &mat {
            Ok(m) => {
                let t = (get_time() - start) as f32;
                for u in &cur.uniforms {
                    if let Some((_, v)) = o.uniforms.iter().find(|(n, _)| *n == u.name) {
                        if v.len() == u.floats() {
                            m.set_uniform_array(&u.name, &v[..]);
                        }
                    } else if u.ty == UType::Float(1) && matches!(u.name.as_str(), "u_time" | "time" | "iTime" | "u_Time") {
                        m.set_uniform(&u.name, t);
                    } else if u.ty == UType::Float(2) && matches!(u.name.as_str(), "u_resolution" | "iResolution") {
                        m.set_uniform(&u.name, (dw, dh));
                    } else if u.ty == UType::Sampler {
                        m.set_texture(&u.name, tex.clone());
                    } else if u.ty == UType::Float(1) {
                        // other float factors (u_amount, u_strength...) preview at 1
                        m.set_uniform(&u.name, 1.0f32);
                    }
                }
                gl_use_material(m);
                draw_texture_ex(&tex, x, y, WHITE, DrawTextureParams { dest_size: Some(vec2(dw, dh)), ..Default::default() });
                gl_use_default_material();
                draw_text(&format!("{}  -  compila OK  -  se recarga al guardar", cur.name), 8.0, screen_height() - 9.0, 20.0, Color::from_rgba(150, 230, 150, 255));
            }
            Err(msg) => {
                draw_texture_ex(&tex, x, y, Color::new(1.0, 1.0, 1.0, 0.25), DrawTextureParams { dest_size: Some(vec2(dw, dh)), ..Default::default() });
                for (i, l) in msg.lines().take(12).enumerate() {
                    draw_text(l, 8.0, 22.0 + i as f32 * 18.0, 18.0, Color::from_rgba(255, 140, 140, 255));
                }
                draw_text(&format!("{}  -  error de compilacion", cur.name), 8.0, screen_height() - 9.0, 20.0, Color::from_rgba(255, 140, 140, 255));
            }
        }
        frames += 1;
        if let Some(path) = &o.screenshot {
            if frames >= o.frames {
                get_screen_data().export_png(path);
                println!("saved {path}");
                if let Err(m) = &mat {
                    println!("error {}:\n{m}", cur.name);
                }
                break;
            }
        }
        if is_key_pressed(KeyCode::Escape) {
            break;
        }
        next_frame().await;
    }
}
