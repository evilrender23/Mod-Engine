//! Sprite flash / outline / overwrite drawn by the default sprite shader, read back from the
//! GPU pixel by pixel. Its own test binary (= its own process): macroquad allows one window
//! per process. Skipped without a display or with MODENGINE_NO_GPU_TESTS=1.

use macroquad::prelude::*;
use modengine::Engine;
use modengine_cli::canvas::render_readback;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const SIZE: u32 = 64;

fn project(name: &str, create: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("me_gpu_juice_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let files = [
        ("game.toml".to_string(), format!("[game]\nname = \"t\"\n[display]\nwidth = {SIZE}\nheight = {SIZE}\n[palette]\nr = \"#ff0000\"\n")),
        ("sprites/box.toml".into(), "pixels = ['''\nrrrrrrrr\nrrrrrrrr\nrrrrrrrr\nrrrrrrrr\nrrrrrrrr\nrrrrrrrr\nrrrrrrrr\nrrrrrrrr\n''']\n".into()),
        (
            "rooms/r.toml".into(),
            "background = \"#000000\"\n[[layer]]\ntype = \"instances\"\n[[layer.instances]]\nobject = \"A\"\nx = 8\ny = 8\n[[layer.instances]]\nobject = \"B\"\nx = 40\ny = 8\n[[layer.instances]]\nobject = \"C\"\nx = 8\ny = 40\n[[layer.instances]]\nobject = \"D\"\nx = 40\ny = 40\n".into(),
        ),
        ("objects/o.ms".into(), format!("object A {{ sprite = box }}\nobject B {{ sprite = box }}\nobject C {{ sprite = box }}\nobject D {{ sprite = box }}\nobject Ctl {{ }}\n{create}\n")),
    ];
    for (rel, text) in files {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    dir
}

fn px(img: &Image, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * img.width as u32 + x) * 4) as usize;
    [img.bytes[i], img.bytes[i + 1], img.bytes[i + 2], img.bytes[i + 3]]
}

fn near(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (*x as i32 - y as i32).abs() <= 3)
}

fn checks(fail: &mut Vec<String>) {
    let mut check = |ok: bool, what: String| {
        if !ok {
            fail.push(what);
        }
    };
    // A: flash white (full), B: green 1 px outline, C: overwrite blue at 50%, D: plain
    let code = "flash(A, #ffffff, 10)\nsprite_outline(B, #00ff00)\nsprite_overwrite(C, #0000ff, 0.5)\n";
    let mut e = Engine::load(&project("fx", "")).unwrap();
    // the loose statements run before the room: apply the effects from the room start instead
    e.set_startup_code(code).unwrap();
    let r = render_readback(&mut e, 2);
    check(e.errors.is_empty(), format!("errors: {:?}", e.errors));
    let img = &r.app;
    let red = [255, 0, 0, 255];
    let black = [0, 0, 0, 255];
    check(near(px(img, 12, 12), [255, 255, 255, 255]), format!("flash: white sprite = {:?}", px(img, 12, 12)));
    check(near(px(img, 7, 12), black), format!("flash: nothing outside = {:?}", px(img, 7, 12)));
    check(near(px(img, 44, 12), red), format!("outline: sprite keeps its colour = {:?}", px(img, 44, 12)));
    for (x, y) in [(39, 12), (48, 12), (44, 7), (44, 16)] {
        check(near(px(img, x, y), [0, 255, 0, 255]), format!("outline: green at ({x},{y}) = {:?}", px(img, x, y)));
    }
    check(near(px(img, 39, 7), black), format!("outline: 4-neighbour, corners stay empty = {:?}", px(img, 39, 7)));
    check(near(px(img, 37, 12), black), format!("outline: 1 px thick = {:?}", px(img, 37, 12)));
    check(near(px(img, 12, 44), [128, 0, 128, 255]), format!("overwrite: half blue = {:?}", px(img, 12, 44)));
    check(near(px(img, 44, 44), red), format!("plain sprite unchanged = {:?}", px(img, 44, 44)));
    check(near(px(img, 47, 47), red), format!("plain sprite unchanged (corner) = {:?}", px(img, 47, 47)));
}

#[test]
fn gpu_sprite_flash_outline_overwrite() {
    let display = !cfg!(target_os = "linux") || std::env::var_os("DISPLAY").is_some();
    if !display || std::env::var_os("MODENGINE_NO_GPU_TESTS").is_some() {
        eprintln!("skipped: no display (or MODENGINE_NO_GPU_TESTS)");
        return;
    }
    let fails = Arc::new(Mutex::new(None::<Vec<String>>));
    let out = fails.clone();
    let conf = Conf { window_title: "modengine gpu juice test".into(), window_width: SIZE as i32 * 2, window_height: SIZE as i32 * 2, ..Default::default() };
    macroquad::Window::from_config(conf, async move {
        let mut f = vec![];
        checks(&mut f);
        *out.lock().unwrap() = Some(f);
    });
    let f = fails.lock().unwrap().take().expect("the GPU checks ran");
    assert!(f.is_empty(), "{} GPU check(s) failed:\n{}", f.len(), f.join("\n"));
}
