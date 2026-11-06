//! Real GPU rendering of the first-person view, primitives and the pixelate post-process,
//! read back pixel by pixel. Opens one small window (skipped without a display, or with
//! MODENGINE_NO_GPU_TESTS=1); macroquad allows one window per process, so everything runs
//! in a single test.

use macroquad::prelude::*;
use modengine::Engine;
use modengine_cli::canvas::{render_readback, Readback};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const SIZE: u32 = 64;

fn project(name: &str, obj: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("me_fpgpu_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let toml = format!(
        "[game]\nname = \"t\"\n[display]\nwidth = {SIZE}\nheight = {SIZE}\n[palette]\n\"r\" = \"#ff0000\"\n\"g\" = \"#00ff00\"\n\"w\" = \"#ffffff\"\n\"b\" = \"#0000ff\"\n"
    );
    let files: Vec<(&str, String)> = vec![
        ("game.toml", toml),
        ("rooms/r.toml", "[[layer]]\ntype = \"instances\"\n[[layer.instances]]\nobject = \"T\"\nx = 0\ny = 0\n".into()),
        ("objects/t.ms", obj.into()),
        ("sprites/green.toml", "pixels = ['''\ngggg\ngggg\ngggg\ngggg\n''']\n".into()),
        ("sprites/redgreen.toml", "pixels = ['''\nrrgg\nrrgg\nrrgg\nrrgg\n''']\n".into()),
        ("sprites/bluewall.toml", "pixels = ['''\nbb\nbb\n''']\n".into()),
    ];
    for (rel, text) in files {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    dir
}

fn render(name: &str, obj: &str) -> (Readback, Vec<String>) {
    let mut e = Engine::load(&project(name, obj)).unwrap_or_else(|m| panic!("{name}: {m}"));
    let r = render_readback(&mut e, 2);
    (r, e.errors.clone())
}

fn px(img: &Image, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * img.width as u32 + x) * 4) as usize;
    [img.bytes[i], img.bytes[i + 1], img.bytes[i + 2], img.bytes[i + 3]]
}

fn near(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (*x as i32 - y as i32).abs() <= 3)
}

/// A corridor 1 cell wide running east from (1,1) to (5,1), walls of type 1 all around.
const MAP: &str = "global.MAP = [[1,1,1,1,1,1,1],[1,0,0,0,0,0,1],[1,1,1,1,1,1,1]]\n";

fn view_obj(setup: &str, draw_extra: &str) -> String {
    format!(
        "{MAP}object T {{
    on create {{
        global.v = fpview_create(64, 64)
        fpview_set_map(global.v, global.MAP)
        fpview_set_wall(global.v, 1, null, 0, #ff0000)
        fpview_set_floor(global.v, #0000ff)
        fpview_set_ceiling(global.v, #00ff00)
        fpview_set_fog(global.v, #000000, 20, 30)
        fpview_set_fov(global.v, 90)
        fpview_set_camera(global.v, 1, 1, 0)
        {setup}
    }}
    on draw {{
        fpview_draw(global.v, 0, 0, 64, 64)
        {draw_extra}
    }}
}}
"
    )
}

fn checks(fail: &mut Vec<String>) {
    let mut check = |ok: bool, what: String| {
        if !ok {
            fail.push(what);
        }
    };

    // 1. the wall at the end of the corridor fills the centre; floor below, ceiling above
    let (r, errs) = render("front", &view_obj("fpview_set_camera(global.v, 4, 1, 0)", ""));
    check(errs.is_empty(), format!("front: errors {errs:?}"));
    let o = &r.output;
    check(near(px(o, 32, 32), [255, 0, 0, 255]), format!("front: wall in front = {:?}", px(o, 32, 32)));
    check(near(px(o, 32, 62), [0, 0, 255, 255]), format!("front: floor at the bottom = {:?}", px(o, 32, 62)));
    check(near(px(o, 32, 1), [0, 255, 0, 255]), format!("front: ceiling at the top = {:?}", px(o, 32, 1)));
    // side walls (north/south faces) are shaded
    let side = px(o, 1, 32);
    check(side[0] > 150 && side[0] < 240 && side[1] < 10, format!("front: shaded side wall = {side:?}"));

    // 2. fog: the same wall from further away, with fog from 0.5 to 6 cells, is darker
    let (r, _) = render("fog", &view_obj("fpview_set_fog(global.v, #000000, 0.5, 6)", ""));
    let far = px(&r.output, 32, 32);
    check(far[0] > 20 && far[0] < 128 && far[1] < 10, format!("fog: far wall darkened = {far:?}"));
    // near the camera the floor is still almost unfogged... and the fog colour is used
    let (r, _) = render("fogcol", &view_obj("fpview_set_fog(global.v, #ffffff, 0, 0.01)", ""));
    check(near(px(&r.output, 32, 32), [255, 255, 255, 255]), format!("fog: full fog = the fog colour {:?}", px(&r.output, 32, 32)));

    // 3. billboards: one in the corridor is drawn; one behind the end wall is hidden
    let (r, errs) = render("bb", &view_obj("fpview_add_billboard(global.v, 3, 1, green, 0, 0.6)", ""));
    check(errs.is_empty(), format!("bb: errors {errs:?}"));
    check(near(px(&r.output, 32, 36), [0, 255, 0, 255]), format!("bb: billboard in front = {:?}", px(&r.output, 32, 36)));
    let (r, _) = render("bbhidden", &view_obj("fpview_add_billboard(global.v, 6, 1, green, 0, 1)\n        fpview_set_camera(global.v, 4, 1, 0)", ""));
    check(near(px(&r.output, 32, 32), [255, 0, 0, 255]), format!("bb: billboard behind the wall hidden = {:?}", px(&r.output, 32, 32)));
    // a billboard behind another one (depth-sorted): the near one wins
    let (r, _) = render(
        "bbsort",
        &view_obj("fpview_add_billboard(global.v, 2, 1, green, 0, 0.6)\n        fpview_add_billboard(global.v, 4, 1, redgreen, 0, 0.6)", ""),
    );
    check(near(px(&r.output, 30, 40), [0, 255, 0, 255]), format!("bb: nearer billboard on top = {:?}", px(&r.output, 30, 40)));
    // textured walls
    let (r, _) = render("tex", &view_obj("fpview_set_wall(global.v, 1, bluewall)", ""));
    check(near(px(&r.output, 32, 30), [0, 0, 255, 255]), format!("tex: textured wall = {:?}", px(&r.output, 32, 30)));
    // a view drawn into a surface, then that surface drawn small at (32, 0)
    let (r, errs) = render(
        "surface",
        &view_obj(
            "global.s = surface_create(64, 64)\n        fpview_set_camera(global.v, 4, 1, 0)",
            "surface_set_target(global.s)\n        draw_clear(#000000)\n        fpview_draw(global.v, 0, 0)\n        surface_reset_target()\n        draw_clear(#ffffff)\n        draw_surface_stretched(global.s, 32, 0, 32, 32)",
        ),
    );
    check(errs.is_empty(), format!("surface: errors {errs:?}"));
    check(near(px(&r.output, 48, 16), [255, 0, 0, 255]), format!("surface: view inside a surface = {:?}", px(&r.output, 48, 16)));
    check(near(px(&r.output, 10, 50), [255, 255, 255, 255]), format!("surface: the rest untouched = {:?}", px(&r.output, 10, 50)));

    // 4. primitives: a red fan and draw_sprite_pos of a red|green sprite
    let (r, errs) = render(
        "prim",
        "object T {
    on draw {
        draw_clear(#000000)
        draw_primitive_begin(pr_trianglefan)
        draw_vertex_color(0, 0, #ff0000, 1)
        draw_vertex_color(30, 0, #ff0000, 1)
        draw_vertex_color(30, 30, #ff0000, 1)
        draw_vertex_color(0, 30, #ff0000, 1)
        draw_primitive_end()
        draw_sprite_pos(redgreen, 0, 32, 32, 64, 36, 64, 60, 32, 64, 1, true)
        draw_primitive_begin(pr_linelist)
        draw_vertex_color(0, 40.5, #ffffff, 1)
        draw_vertex_color(20, 40.5, #ffffff, 1)
        draw_primitive_end()
    }
}
",
    );
    check(errs.is_empty(), format!("prim: errors {errs:?}"));
    let o = &r.output;
    check(near(px(o, 15, 15), [255, 0, 0, 255]), format!("prim: fan = {:?}", px(o, 15, 15)));
    check(near(px(o, 40, 48), [255, 0, 0, 255]), format!("prim: sprite_pos left = {:?}", px(o, 40, 48)));
    check(near(px(o, 58, 48), [0, 255, 0, 255]), format!("prim: sprite_pos right = {:?}", px(o, 58, 48)));
    check(near(px(o, 10, 40), [255, 255, 255, 255]), format!("prim: line = {:?}", px(o, 10, 40)));
    check(near(px(o, 10, 44), [0, 0, 0, 255]), format!("prim: below the line = {:?}", px(o, 10, 44)));

    // 5. pixelate: a 64-step gradient becomes 8 blocks of 8 px; posterize to 2 levels
    let grad = "object T {
    on create { postfx_pixelate(8, LEVELS, 0) }
    on draw {
        for (var i = 0; i < 64; i++) {
            set_color(i * 4 * 65793)
            draw_rect(i, 0, i + 1, 64)
        }
    }
    on draw_gui { set_color(#ff0000); draw_rect(0, 60, 2, 64) }
}
";
    let (r, errs) = render("pix", &grad.replace("LEVELS", "0"));
    check(errs.is_empty(), format!("pix: errors {errs:?}"));
    let o = &r.output;
    let mut blocky = true;
    for bx in 0..8 {
        let c = px(o, bx * 8, 10);
        for dx in 0..8 {
            blocky &= near(px(o, bx * 8 + dx, 10), c) && near(px(o, bx * 8 + dx, 50), c);
        }
    }
    check(blocky, format!("pix: 8-px blocks, row = {:?}", (0..64).step_by(4).map(|x| px(o, x, 10)[0]).collect::<Vec<_>>()));
    check(px(o, 60, 10)[0] > px(o, 4, 10)[0] + 100, "pix: the gradient is kept".into());
    check(near(px(o, 1, 62), [255, 0, 0, 255]), format!("pix: the GUI is not pixelated = {:?}", px(o, 1, 62)));
    let (r, _) = render("poster", &grad.replace("LEVELS", "2"));
    let vals: Vec<u8> = (0..64).map(|x| px(&r.output, x, 10)[0]).collect();
    check(vals.iter().all(|v| *v < 3 || *v > 252), format!("poster: 2 levels only = {vals:?}"));
    // palette snapping: every pixel is one of the two colours
    let (r, _) = render("pal", &grad.replace("postfx_pixelate(8, LEVELS, 0)", "postfx_pixelate(16, 0, 0, [#ff0000, #0000ff])"));
    let ok = (0..64).all(|x| {
        let c = px(&r.output, x, 20);
        near(c, [255, 0, 0, 255]) || near(c, [0, 0, 255, 255])
    });
    check(ok, format!("pal: snapped to the palette = {:?}", px(&r.output, 40, 20)));
}

#[test]
fn gpu_fpview_readback() {
    let display = !cfg!(target_os = "linux") || std::env::var_os("DISPLAY").is_some();
    if !display || std::env::var_os("MODENGINE_NO_GPU_TESTS").is_some() {
        eprintln!("skipped: no display (or MODENGINE_NO_GPU_TESTS)");
        return;
    }
    let fails = Arc::new(Mutex::new(None::<Vec<String>>));
    let out = fails.clone();
    let conf = Conf { window_title: "modengine fpview test".into(), window_width: SIZE as i32 * 2, window_height: SIZE as i32 * 2, ..Default::default() };
    macroquad::Window::from_config(conf, async move {
        let mut f = vec![];
        checks(&mut f);
        *out.lock().unwrap() = Some(f);
    });
    let f = fails.lock().unwrap().take().expect("the GPU checks ran");
    assert!(f.is_empty(), "{} GPU check(s) failed:\n{}", f.len(), f.join("\n"));
}
