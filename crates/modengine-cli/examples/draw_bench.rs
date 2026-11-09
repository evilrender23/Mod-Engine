//! Drawing benchmark: `cargo run --release -p modengine-cli --example draw_bench -- [project] [frames]`
//! (without a project: 5000 bullets of 4 sprites, interleaved, in a temporary project).
//!
//! 1. Engine only (no window): average time of `tick()` + `render()` per frame.
//! 2. Window with vsync off: frames per second of the whole loop, and how many sprites and
//!    texture changes (= draw calls) the last frame had. `MODENGINE_NO_ATLAS=1` measures
//!    the old path (one texture per sprite).

use modengine::Engine;
use modengine_cli::{backend, runner, sprite_draw};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = match args.first() {
        Some(d) => std::path::PathBuf::from(d),
        None => bullets_project(),
    };
    let frames: u64 = args.get(1).and_then(|f| f.parse().ok()).unwrap_or(600);

    let mut e = Engine::load(&dir).expect("load");
    e.start();
    for _ in 0..30 {
        e.tick();
        e.render();
    }
    let n = 300;
    let (mut tick, mut render) = (0.0, 0.0);
    for _ in 0..n {
        let t = Instant::now();
        e.tick();
        tick += t.elapsed().as_secs_f64();
        let t = Instant::now();
        e.render();
        render += t.elapsed().as_secs_f64();
    }
    let alive = e.insts.iter().filter(|i| i.alive).count();
    println!("engine: {alive} instances, {} draw commands; tick {:.3} ms, render {:.3} ms", e.cmds.len(), tick * 1000.0 / n as f64, render * 1000.0 / n as f64);

    let e = Engine::load(&dir).expect("load");
    let mut conf = runner::window_conf(&e);
    conf.platform.swap_interval = Some(0);
    modengine_cli::display::prepare(&mut conf);
    // lockstep: one logic step and one full render per frame, `frames` steps
    let test = modengine_cli::test_run::WindowTest::new(&[], None, None, Some(frames), false);
    let opts = backend::RunOpts { test: Some(test), ..Default::default() };
    macroquad::Window::from_config(conf, async move {
        let t = Instant::now();
        backend::run(e, opts).await;
        let s = t.elapsed().as_secs_f64();
        let (quads, switches) = sprite_draw::last_frame_stats();
        println!("window: {frames} frames in {s:.2} s = {:.0} fps ({:.2} ms/frame); last frame: {quads} sprites, {switches} texture changes", frames as f64 / s, s * 1000.0 / frames as f64);
    });
}

/// 5000 bullets of 4 different sprites created in turn (the worst case for batching by
/// texture: every sprite differs from the previous one).
fn bullets_project() -> std::path::PathBuf {
    let d = std::env::temp_dir().join("modengine_draw_bench");
    let w = |rel: &str, text: &str| {
        let p = d.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    };
    w("game.toml", "[game]\nname = \"Bench\"\nstart_room = \"main\"\n[display]\nwidth = 640\nheight = 360\nscale = 2\n");
    for (i, c) in ["#ff4040", "#40ff40", "#4080ff", "#ffff40"].iter().enumerate() {
        w(&format!("sprites/bala{i}.toml"), &format!("origin = \"center\"\npalette = {{ a = \"{c}\", w = \"#ffffff\" }}\npixels = ['''\n..aaaa..\n.aawwaa.\naawwwwaa\naawwwwaa\n.aawwaa.\n..aaaa..\n''']\n"));
    }
    w(
        "objects/bala.ms",
        "object Bala {\n    sprite = bala0\n    on create { speed = rand(1) + 0.5; direction = rand(360) }\n    on step {\n        if x < 0 x += room_width\n        if x > room_width x -= room_width\n        if y < 0 y += room_height\n        if y > room_height y -= room_height\n    }\n}\nobject Bala1 : Bala { sprite = bala1 }\nobject Bala2 : Bala { sprite = bala2 }\nobject Bala3 : Bala { sprite = bala3 }\n",
    );
    w(
        "rooms/main.toml",
        "width = 640\nheight = 360\nbackground = \"#101018\"\ncode = \"\"\"\nvar objs = [Bala, Bala1, Bala2, Bala3]\nfor (var i = 0; i < 5000; i++) spawn(objs[i % 4], rand(room_width), rand(room_height))\n\"\"\"\n[[layer]]\nname = \"Instances\"\ntype = \"instances\"\n",
    );
    d
}
