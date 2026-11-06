//! Real GPU rendering, read back pixel by pixel: canvas alpha, shader precision, surfaces
//! and the post_draw pass. Opens one small window (skipped without a display, or with
//! MODENGINE_NO_GPU_TESTS=1). Everything runs in a single test: macroquad allows one
//! window per process.

use macroquad::prelude::*;
use modengine::Engine;
use modengine_cli::canvas::{render_readback, Readback};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const SIZE: u32 = 64;

fn project(name: &str, shaders: &[(&str, &str)], obj: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("me_gpu_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let toml = format!("[game]\nname = \"t\"\n[display]\nwidth = {SIZE}\nheight = {SIZE}\nletterbox = \"#ff00ff\"\n");
    let mut files: Vec<(String, String)> = vec![
        ("game.toml".into(), toml),
        ("rooms/r.toml".into(), "[[layer]]\ntype = \"instances\"\n[[layer.instances]]\nobject = \"T\"\nx = 0\ny = 0\n".into()),
        ("objects/t.ms".into(), obj.into()),
    ];
    for (n, src) in shaders {
        files.push((format!("shaders/{n}.fsh"), src.to_string()));
    }
    for (rel, text) in files {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    dir
}

fn render(name: &str, shaders: &[(&str, &str)], obj: &str) -> (Readback, Vec<String>) {
    let mut e = Engine::load(&project(name, shaders, obj)).unwrap_or_else(|m| panic!("{name}: {m}"));
    let r = render_readback(&mut e, 2);
    (r, e.errors.clone())
}

fn px(img: &Image, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * img.width as u32 + x) * 4) as usize;
    [img.bytes[i], img.bytes[i + 1], img.bytes[i + 2], img.bytes[i + 3]]
}

/// Pixel of the window at the game coordinates (x, y) of a SIZE×SIZE game.
fn wpx(r: &Readback, x: u32, y: u32) -> [u8; 4] {
    let (w, h) = (r.window.width as u32, r.window.height as u32);
    px(&r.window, (x * w + w / 2) / SIZE, (y * h + h / 2) / SIZE)
}

fn near(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (*x as i32 - y as i32).abs() <= 3)
}

const PASS: &str = "varying vec2 v_vTexcoord;\nvarying vec4 v_vColour;\nvoid main() { gl_FragColor = v_vColour * texture2D(gm_BaseTexture, v_vTexcoord); }\n";

fn checks(fail: &mut Vec<String>) {
    let mut check = |ok: bool, what: String| {
        if !ok {
            fail.push(what);
        }
    };

    // 1. alpha: half-transparent rect, then a shader draw on top (RogueShot's bug: the
    // shader inherited the lowered canvas alpha and the window showed through it)
    let (r, errs) = render(
        "alpha",
        &[("paso", PASS)],
        r#"
object T {
    on draw {
        set_color(#00ff00)
        set_alpha(0.5)
        draw_rect(0, 0, 64, 64)
        gpu_set_blendmode(bm_add)
        draw_rect(0, 48, 64, 64)
        gpu_set_blendmode(bm_normal)
        set_alpha(1)
        set_color(#ffffff)
        shader_set(paso)
        draw_rect(16, 16, 32, 32)
        shader_reset()
        set_alpha(0.25)
        draw_rect(40, 16, 56, 32)
    }
}
"#,
    );
    check(errs.is_empty(), format!("alpha: errors {errs:?}"));
    for (x, y) in [(2, 2), (20, 20), (60, 60), (45, 20), (10, 50)] {
        check(px(&r.app, x, y)[3] == 255, format!("alpha: canvas alpha at ({x},{y}) = {:?}", px(&r.app, x, y)));
    }
    check(near(px(&r.app, 2, 2), [0, 128, 0, 255]), format!("alpha: half green over black = {:?}", px(&r.app, 2, 2)));
    check(near(px(&r.app, 20, 20), [255, 255, 255, 255]), format!("alpha: shader-drawn white = {:?}", px(&r.app, 20, 20)));
    check(near(wpx(&r, 20, 20), [255, 255, 255, 255]), format!("alpha: shader draw on the window = {:?}", wpx(&r, 20, 20)));
    check(near(wpx(&r, 2, 2), [0, 128, 0, 255]), format!("alpha: window shows the canvas, not the letterbox = {:?}", wpx(&r, 2, 2)));
    check(near(px(&r.app, 45, 20), [64, 160, 64, 255]), format!("alpha: quarter white over half green = {:?}", px(&r.app, 45, 20)));

    // 2. precision: pixel-sized length()/dot() in a fragment shader
    let prec = "varying vec2 v_vTexcoord;\nvarying vec4 v_vColour;\nvoid main() {\n    vec2 p = gl_FragCoord.xy + vec2(1280.0, 720.0);\n    float d = length(p);\n    float sq = dot(p, p);\n    bool ok = d == d && d > 1460.0 && d < 1560.0 && sq > 2.1e6 && sq < 2.45e6;\n    gl_FragColor = ok ? vec4(0.0, 1.0, 0.0, 1.0) : vec4(1.0, 0.0, 0.0, 1.0);\n}\n";
    let (r, errs) = render("prec", &[("dist", prec)], "object T {\n    on draw {\n        shader_set(dist)\n        draw_rect(0, 0, 64, 64)\n        shader_reset()\n    }\n}\n");
    check(errs.is_empty(), format!("precision: errors {errs:?}"));
    for (x, y) in [(1, 1), (32, 32), (62, 62)] {
        check(near(px(&r.app, x, y), [0, 255, 0, 255]), format!("precision: length() of 1280-ish coordinates at ({x},{y}) = {:?}", px(&r.app, x, y)));
    }

    // 3. surfaces: contents, orientation, parts, flips, nesting
    let (r, errs) = render(
        "surf",
        &[],
        r#"
object T {
    on create { a = surface_create(16, 16); b = surface_create(4, 4) }
    on draw {
        surface_set_target(a)
        draw_clear(#ff0000)
        set_color(#0000ff)
        draw_rect(0, 0, 8, 8)
        surface_set_target(b)
        draw_clear(#ffff00)
        surface_reset_target()
        draw_surface(b, 12, 12)
        surface_reset_target()
        set_color(#ffffff)
        draw_surface(a, 0, 0)
        draw_surface_ext(a, 64, 0, -1, 1, 0, #ffffff, 1)
        draw_surface_part(a, 8, 8, 8, 8, 0, 32)
        draw_surface_stretched(a, 32, 32, 32, 32)
    }
}
"#,
    );
    check(errs.is_empty(), format!("surfaces: errors {errs:?}"));
    for (x, y, want, what) in [
        (2, 2, [0, 0, 255, 255], "top-left of the surface is its (0,0)"),
        (12, 4, [255, 0, 0, 255], "cleared red"),
        (13, 13, [255, 255, 0, 255], "nested surface drawn inside the outer one"),
        (61, 2, [0, 0, 255, 255], "negative xscale mirrors around x"),
        (50, 2, [255, 0, 0, 255], "mirrored red half"),
        (1, 33, [255, 0, 0, 255], "draw_surface_part takes the bottom-right part"),
        (6, 38, [255, 255, 0, 255], "draw_surface_part includes the nested yellow"),
        (34, 34, [0, 0, 255, 255], "stretched ×2 top-left"),
        (50, 40, [255, 0, 0, 255], "stretched ×2 red"),
        (40, 20, [0, 0, 0, 255], "background untouched"),
    ] {
        check(near(px(&r.app, x, y), want), format!("surfaces: {what} at ({x},{y}) = {:?}", px(&r.app, x, y)));
    }
    check(r.surfaces.len() == 2, format!("surfaces: {} GPU targets", r.surfaces.len()));

    // 4. post_draw: a shader over the whole finished frame
    let invert = "varying vec2 v_vTexcoord;\nvarying vec4 v_vColour;\nvoid main() {\n    vec4 c = texture2D(gm_BaseTexture, v_vTexcoord);\n    gl_FragColor = vec4(1.0 - c.rgb, 1.0);\n}\n";
    let obj = r#"
object T {
    on draw {
        set_color(#ff0000)
        draw_rect(0, 0, 32, 64)
    }
    on post_draw {
        shader_set(inv)
        draw_surface(application_surface, 0, 0)
        shader_reset()
    }
}
"#;
    let (r, errs) = render("post", &[("inv", invert)], obj);
    check(errs.is_empty(), format!("post: errors {errs:?}"));
    check(near(px(&r.app, 4, 4), [255, 0, 0, 255]), format!("post: application_surface keeps the game = {:?}", px(&r.app, 4, 4)));
    check(near(px(&r.output, 4, 4), [0, 255, 255, 255]), format!("post: output inverted red = {:?}", px(&r.output, 4, 4)));
    check(near(px(&r.output, 50, 4), [255, 255, 255, 255]), format!("post: output inverted black = {:?}", px(&r.output, 50, 4)));
    check(near(wpx(&r, 4, 4), [0, 255, 255, 255]), format!("post: the window shows the post_draw image = {:?}", wpx(&r, 4, 4)));
    check(near(wpx(&r, 4, 60), [0, 255, 255, 255]), format!("post: window bottom-left (orientation) = {:?}", wpx(&r, 4, 60)));
    // draw disabled and nothing in post_draw: black frame
    let (r, _) = render("nodraw", &[], "object T {\n    on create { application_surface_draw_enable(false) }\n    on draw { draw_rect(0, 0, 64, 64) }\n}\n");
    check(near(px(&r.output, 10, 10), [0, 0, 0, 255]), format!("post: draw disabled shows black = {:?}", px(&r.output, 10, 10)));
    // fonts without `file` use the built-in face at their own size (it used to be [display] font_size)
    let obj = "object T {\n    on draw {\n        draw_rect(0, 0, 64, 64)\n        set_color(#000000)\n        draw_rect(0, 0, 64, 64)\n        set_color(#ffffff)\n        set_font(fnt_s)\n        draw_rect(0, 0, text_width(\"MMMM\"), 8)\n        set_font(fnt_b)\n        draw_rect(0, 20, text_width(\"MMMM\"), 28)\n    }\n}\n";
    let dir = project("fontsize", &[], obj);
    std::fs::create_dir_all(dir.join("fonts")).unwrap();
    std::fs::write(dir.join("fonts/fnt_s.toml"), "size = 6\n").unwrap();
    std::fs::write(dir.join("fonts/fnt_b.toml"), "size = 24\n").unwrap();
    let mut e = Engine::load(&dir).unwrap();
    let r = render_readback(&mut e, 2);
    let width = |y: u32| (0..SIZE).filter(|&x| px(&r.output, x, y)[0] > 128).count();
    let (ws, wb) = (width(4), width(24));
    check(ws > 0 && wb >= ws * 3, format!("fonts: built-in face at size 6 ({ws} px) vs 24 ({wb} px) for \"MMMM\""));
    // pixel-art games draw the built-in face on whole pixels: the same glyph keeps all its
    // pixels after any number of advances (5.6875 px at size 13: 8 of them land on x.5)
    let mut obj = String::from("object T {\n    on draw {\n        set_color(#000000)\n        draw_rect(0, 0, 256, 64)\n        set_color(#ffffff)\n        set_font(fnt_s)\n");
    for (k, n) in [0usize, 8, 24, 40].iter().enumerate() {
        obj += &format!("        draw_text(0, {}, \"{}o\")\n", k * 16, " ".repeat(*n));
    }
    obj += "    }\n}\n";
    let dir = project("fontpx", &[], &obj);
    std::fs::write(dir.join("game.toml"), "[game]\nname = \"t\"\n[display]\nwidth = 256\nheight = 64\n").unwrap();
    std::fs::create_dir_all(dir.join("fonts")).unwrap();
    std::fs::write(dir.join("fonts/fnt_s.toml"), "size = 13\n").unwrap();
    let mut e = Engine::load(&dir).unwrap();
    let r = render_readback(&mut e, 2);
    let lit = |y0: u32| (y0..y0 + 16).flat_map(|y| (0..256).map(move |x| (x, y))).filter(|&(x, y)| px(&r.output, x, y)[0] > 128).count();
    let counts: Vec<usize> = (0..4u32).map(|k| lit(k * 16)).collect();
    check(counts[0] > 0 && counts.iter().all(|&c| c == counts[0]), format!("fonts: the same glyph after 0, 8, 24 and 40 advances has {counts:?} lit pixels"));
}

#[test]
fn gpu_render_readback() {
    // miniquad opens an X11 window on Linux (XWayland under Wayland)
    let display = !cfg!(target_os = "linux") || std::env::var_os("DISPLAY").is_some();
    if !display || std::env::var_os("MODENGINE_NO_GPU_TESTS").is_some() {
        eprintln!("skipped: no display (or MODENGINE_NO_GPU_TESTS)");
        return;
    }
    let fails = Arc::new(Mutex::new(None::<Vec<String>>));
    let out = fails.clone();
    let conf = Conf { window_title: "modengine gpu test".into(), window_width: SIZE as i32 * 2, window_height: SIZE as i32 * 2, ..Default::default() };
    macroquad::Window::from_config(conf, async move {
        let mut f = vec![];
        checks(&mut f);
        *out.lock().unwrap() = Some(f);
    });
    let f = fails.lock().unwrap().take().expect("the GPU checks ran");
    assert!(f.is_empty(), "{} GPU check(s) failed:\n{}", f.len(), f.join("\n"));
}
