//! Tweens, sprite effects, popups, hit-stop, UI focus, layered music and localization,
//! run headless on small projects written to temp folders (fixed step = deterministic).

use modengine::draw::DrawCmd;
use modengine::modscript::{intern, Value};
use modengine::Engine;
use std::path::PathBuf;

fn project(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("me_feel_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut all: Vec<(&str, &str)> = vec![
        ("game.toml", "[game]\nname = \"t\"\n[display]\nstep_rate = 60\n[palette]\nr = \"#ff0000\"\n"),
        ("sprites/box.toml", "pixels = ['''\nrrrr\nrrrr\nrrrr\nrrrr\n''']\n"),
        ("rooms/r.toml", "[[layer]]\ntype = \"instances\"\n[[layer.instances]]\nobject = \"T\"\nx = 0\ny = 0\n"),
    ];
    for f in files {
        if let Some(i) = all.iter().position(|a| a.0 == f.0) {
            all.remove(i);
        }
    }
    all.extend_from_slice(files);
    for (rel, text) in all {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    dir
}

fn start(dir: &PathBuf) -> Engine {
    let mut e = Engine::load(dir).unwrap_or_else(|m| panic!("load failed:\n{m}"));
    e.start();
    e
}

fn obj(code: &str) -> String {
    format!("object T {{\n    sprite = box\n{code}\n}}\n")
}

fn g(e: &Engine, n: &str) -> Value {
    e.globals.get(&intern(n)).cloned().unwrap_or_default()
}

fn num(e: &Engine, n: &str) -> f64 {
    g(e, n).as_num().unwrap_or_else(|| panic!("global.{n} is not a number: {}", g(e, n)))
}

fn run(e: &mut Engine, code: &str) {
    e.run_code(code).unwrap_or_else(|m| panic!("{m}\n{code}"));
}

fn ticks(e: &mut Engine, n: u32) {
    for _ in 0..n {
        e.tick();
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

#[test]
fn easing_values_at_known_points() {
    use modengine::feel::ease;
    let e = |n: &str, t: f64| ease(n, t).unwrap_or_else(|| panic!("{n}"));
    assert!(close(e("linear", 0.3), 0.3));
    assert!(close(e("quad_in", 0.5), 0.25));
    assert!(close(e("quad_out", 0.5), 0.75));
    assert!(close(e("cubic_in", 0.5), 0.125));
    assert!(close(e("cubic_inout", 0.5), 0.5));
    assert!(close(e("quart_out", 0.5), 1.0 - 0.0625));
    assert!(close(e("quint_in", 0.5), 0.03125));
    assert!(close(e("sine_in", 0.5), 1.0 - (std::f64::consts::PI / 4.0).cos()));
    assert!(close(e("expo_in", 0.0), 0.0) && close(e("expo_out", 1.0), 1.0));
    assert!(close(e("circ_out", 1.0), 1.0));
    assert!((0..100).any(|i| e("back_out", i as f64 / 100.0) > 1.0), "back_out overshoots");
    assert!((0..100).any(|i| e("back_in", i as f64 / 100.0) < 0.0), "back_in anticipates");
    assert!(close(e("elastic_out", 0.0), 0.0) && close(e("elastic_out", 1.0), 1.0));
    assert!(close(e("bounce_out", 1.0), 1.0) && close(e("bounce_in", 0.0), 0.0));
    assert!(close(e("bounce_out", 1.0 / 2.75), 1.0), "first bounce lands at 1/2.75");
    assert!(close(e("back", 0.5), e("back_out", 0.5)), "a bare family name is its out version");
    for n in modengine::feel::ease_names() {
        assert!(close(e(&n, 0.0), 0.0) && close(e(&n, 1.0), 1.0), "{n} goes 0 → 1");
    }
    assert!(ease("nope_out", 0.5).is_none());
    // the same through ModScript
    let mut eng = start(&project("ease", &[("objects/t.ms", &obj(""))]));
    run(&mut eng, "global.a = ease(\"quad_in\", 0.5)\nglobal.n = len(ease_names())");
    assert!(close(num(&eng, "a"), 0.25));
    assert_eq!(num(&eng, "n"), 32.0);
    assert!(eng.run_code("ease(\"wobble\", 0.5)").is_err());
}

#[test]
fn tween_progresses_with_fixed_steps() {
    let mut e = start(&project("tw", &[("objects/t.ms", &obj(""))]));
    run(&mut e, "with (T) { x = 0; global.t = tween_to(self, { x: 100, alpha: 0 }, 0.5, \"linear\") }");
    ticks(&mut e, 15);
    run(&mut e, "global.x = T.x\nglobal.a = T.alpha\nglobal.d = tween_done(global.t)\nglobal.left = tween_remaining(global.t)");
    assert!(close(num(&e, "x"), 50.0), "half way after 15 of 30 steps: {}", num(&e, "x"));
    assert!(close(num(&e, "a"), 0.5));
    assert!(!g(&e, "d").truthy());
    assert!(close(num(&e, "left"), 0.25));
    ticks(&mut e, 15);
    run(&mut e, "global.x = T.x\nglobal.d = tween_done(global.t)\nglobal.c = tween_count()");
    assert!(close(num(&e, "x"), 100.0));
    assert!(g(&e, "d").truthy());
    assert_eq!(num(&e, "c"), 0.0);
    // tween(target, field, to, secs, ease, delay) + delay
    run(&mut e, "with (T) { y = 0; global.t = tween(self, \"y\", 60, 0.25, \"linear\", 0.25) }");
    ticks(&mut e, 15);
    run(&mut e, "global.y = T.y");
    assert!(close(num(&e, "y"), 0.0), "still in its delay");
    ticks(&mut e, 6);
    run(&mut e, "global.y = T.y");
    assert!(close(num(&e, "y"), 24.0), "6 of 15 steps after the delay: {}", num(&e, "y"));
    // structs, colours and tween_from
    run(&mut e, "global.s = { v: 0, color: #000000 }\ntween_to(global.s, { v: 10, color: #ff8000 }, 0.1, \"linear\")\nwith (T) tween_from(self, { scale_x: 3 }, 0.1, \"linear\")\nglobal.sx0 = T.scale_x");
    assert!(close(num(&e, "sx0"), 3.0), "tween_from sets the start value at once");
    ticks(&mut e, 3);
    run(&mut e, "global.v = global.s.v\nglobal.col = global.s.color");
    assert!(close(num(&e, "v"), 5.0));
    assert_eq!(num(&e, "col") as u32, 0x804000, "per channel: {:06x}", num(&e, "col") as u32);
    ticks(&mut e, 3);
    run(&mut e, "global.sx = T.scale_x");
    assert!(close(num(&e, "sx"), 1.0));
    // errors: bad target, unknown variable, bad option
    assert!(e.run_code("tween_to(3, { x: 1 }, 1)").is_err());
    assert!(e.run_code("with (T) tween_to(self, { nada: 1 }, 1)").is_err());
    assert!(e.run_code("with (T) tween_to(self, { x: 1 }, 1, \"linear\", { rebote: true })").is_err());
}

#[test]
fn tween_cancel_complete_chain_yoyo_loops() {
    let src = obj("    hits = 0\n    function done(id) { hits += 1; global.last = id }");
    let mut e = start(&project("tw2", &[("objects/t.ms", &src)]));
    // cancel (stays where it is) and cancel with jump to the end
    run(&mut e, "with (T) { x = 0; global.a = tween_to(self, { x: 100 }, 1, \"linear\") }");
    ticks(&mut e, 6);
    run(&mut e, "global.ok = tween_cancel(global.a)\nglobal.x = T.x\nglobal.again = tween_cancel(global.a)");
    assert!(g(&e, "ok").truthy() && !g(&e, "again").truthy());
    assert!(close(num(&e, "x"), 10.0));
    run(&mut e, "with (T) { global.a = tween_to(self, { x: 50 }, 1) }\ntween_cancel(global.a, true)\nglobal.x = T.x");
    assert!(close(num(&e, "x"), 50.0));
    // on_complete: option and tween_on_complete, called once with the id
    run(&mut e, "with (T) { global.a = tween_to(self, { y: 10 }, 0.05, \"linear\", { on_complete: done }) }");
    ticks(&mut e, 5);
    run(&mut e, "global.h = T.hits");
    assert_eq!(num(&e, "h"), 1.0);
    assert_eq!(num(&e, "last"), num(&e, "a"));
    // chain: b waits for a, starts from where a left the value
    run(&mut e, "with (T) { x = 0; global.a = tween_to(self, { x: 30 }, 0.1, \"linear\"); global.b = tween_to(self, { x: 0 }, 0.1, \"linear\"); tween_on_complete(global.b, done) }\ntween_chain(global.a, global.b)");
    ticks(&mut e, 6);
    run(&mut e, "global.x = T.x\nglobal.bd = tween_done(global.b)");
    assert!(close(num(&e, "x"), 30.0), "a finished, b not started: {}", num(&e, "x"));
    assert!(!g(&e, "bd").truthy());
    ticks(&mut e, 3);
    run(&mut e, "global.x = T.x");
    assert!(close(num(&e, "x"), 15.0), "b half way back: {}", num(&e, "x"));
    ticks(&mut e, 3);
    run(&mut e, "global.x = T.x\nglobal.h = T.hits");
    assert!(close(num(&e, "x"), 0.0));
    assert_eq!(num(&e, "h"), 2.0);
    // yoyo + 1 loop: 0 → 10 → 0 → 10 → 0, then done at 0
    run(&mut e, "with (T) { x = 0; global.a = tween_to(self, { x: 10 }, 0.1, \"linear\", { yoyo: true, loops: 1 }) }");
    let mut xs = vec![];
    for _ in 0..24 {
        e.tick();
        run(&mut e, "global.x = T.x");
        xs.push(num(&e, "x"));
    }
    assert!(close(xs[5], 10.0) && close(xs[11], 0.0) && close(xs[17], 10.0) && close(xs[23], 0.0), "{xs:?}");
    run(&mut e, "global.d = tween_done(global.a)");
    assert!(g(&e, "d").truthy());
    // endless loop until cancelled; cancel_all by target; destroyed targets drop their tweens
    run(&mut e, "with (T) { tween_to(self, { x: 5 }, 0.1, \"linear\", { loops: -1 }); tween_to(self, { y: 5 }, 0.1) }\nglobal.n = tween_count(T.id)");
    assert_eq!(num(&e, "n"), 2.0);
    ticks(&mut e, 100);
    run(&mut e, "global.n = tween_count()\nwith (T) global.k = tween_cancel_all(self)");
    assert_eq!(num(&e, "n"), 1.0, "only the endless one is left");
    assert_eq!(num(&e, "k"), 1.0);
    run(&mut e, "with (T) tween_to(self, { x: 5 }, 1)\nwith (T) destroy()");
    ticks(&mut e, 2);
    assert!(e.feel.tweens.list.is_empty());
}

#[test]
fn speed_hitstop_and_unscaled_tweens() {
    let src = obj("    steps = 0\n    on step { steps += 1 }");
    let mut e = start(&project("hs", &[("objects/t.ms", &src)]));
    run(&mut e, "tween_set_speed(2)\nglobal.s = { a: 0, b: 0 }\ntween_to(global.s, { a: 10 }, 0.5, \"linear\")\nglobal.u = tween_to(global.s, { b: 10 }, 0.5, \"linear\")\ntween_set_unscaled(global.u)");
    ticks(&mut e, 15);
    run(&mut e, "global.a = global.s.a\nglobal.b = global.s.b\nglobal.sp = tween_get_speed()");
    assert!(close(num(&e, "a"), 10.0), "double speed: done in 15 steps");
    assert!(close(num(&e, "b"), 5.0), "unscaled ignores the speed");
    assert_eq!(num(&e, "sp"), 2.0);
    // hit-stop: 0.1 s = 6 steps without step events; unscaled tweens go on
    run(&mut e, "tween_set_speed(1)\nglobal.s = { a: 0, b: 0 }\ntween_to(global.s, { a: 60 }, 1, \"linear\")\nglobal.u = tween_to(global.s, { b: 60 }, 1, \"linear\", { unscaled: true })\nhitstop(0.1)\nglobal.st = T.steps\nglobal.on = hitstop_active()");
    assert!(g(&e, "on").truthy());
    let steps0 = num(&e, "st");
    let frame0 = e.steps;
    ticks(&mut e, 6);
    run(&mut e, "global.st = T.steps\nglobal.a = global.s.a\nglobal.b = global.s.b\nglobal.on = hitstop_active()");
    assert_eq!(num(&e, "st"), steps0, "no step events during the hit-stop");
    assert_eq!(e.steps, frame0 + 6, "the frame counter keeps going");
    assert!(close(num(&e, "a"), 0.0), "scaled tweens are frozen");
    assert!(close(num(&e, "b"), 6.0), "unscaled tweens move");
    assert!(!g(&e, "on").truthy());
    ticks(&mut e, 1);
    run(&mut e, "global.st = T.steps\nglobal.a = global.s.a");
    assert_eq!(num(&e, "st"), steps0 + 1.0);
    assert!(close(num(&e, "a"), 1.0));
    // a longer hit-stop wins over a shorter one; cancel ends it
    run(&mut e, "hitstop(0.5)\nhitstop(0.1)");
    assert_eq!(e.feel.hitstop, 30);
    run(&mut e, "hitstop_cancel()\nglobal.on = hitstop_active()");
    assert!(!g(&e, "on").truthy());
}

fn fx_cmds(e: &mut Engine) -> Vec<DrawCmd> {
    e.render();
    e.cmds.iter().filter(|c| matches!(c, DrawCmd::SpriteFx(_) | DrawCmd::Sprite { .. })).cloned().collect()
}

#[test]
fn flash_outline_overwrite_and_pulse_draw_commands() {
    let mut e = start(&project("fx", &[("objects/t.ms", &obj("    x = 20\n    y = 20"))]));
    let plain = fx_cmds(&mut e);
    assert_eq!(plain.len(), 1, "no effect: one plain sprite");
    run(&mut e, "flash(T, #ffffff, 0.1)");
    let c = fx_cmds(&mut e);
    assert!(matches!(c[0], DrawCmd::SpriteFx(Some(f)) if f == [1.0, 1.0, 1.0, 1.0]), "{:?}", c[0]);
    assert!(matches!(c.last(), Some(DrawCmd::SpriteFx(None))));
    ticks(&mut e, 3);
    let c = fx_cmds(&mut e);
    assert!(matches!(c[0], DrawCmd::SpriteFx(Some(f)) if (f[3] - 0.5).abs() < 1e-6), "fades linearly: {:?}", c[0]);
    // a hit-stop holds the flash
    run(&mut e, "hitstop(0.05)");
    ticks(&mut e, 3);
    let c = fx_cmds(&mut e);
    assert!(matches!(c[0], DrawCmd::SpriteFx(Some(f)) if (f[3] - 0.5).abs() < 1e-6), "held during the hit-stop: {:?}", c[0]);
    ticks(&mut e, 3);
    assert_eq!(fx_cmds(&mut e).len(), 1, "flash over");
    // outline: 4 neighbours fully overwritten + the sprite
    run(&mut e, "sprite_outline(T, #00ff00)");
    let c = fx_cmds(&mut e);
    let sprites: Vec<(f32, f32)> = c.iter().filter_map(|c| if let DrawCmd::Sprite { x, y, .. } = c { Some((*x, *y)) } else { None }).collect();
    assert_eq!(sprites.len(), 5);
    assert!(sprites.contains(&(19.0, 20.0)) && sprites.contains(&(21.0, 20.0)) && sprites.contains(&(20.0, 19.0)) && sprites.contains(&(20.0, 21.0)));
    assert!(matches!(c[0], DrawCmd::SpriteFx(Some(f)) if f == [0.0, 1.0, 0.0, 1.0]));
    run(&mut e, "sprite_outline(T, #00ff00, 2)");
    assert_eq!(fx_cmds(&mut e).iter().filter(|c| matches!(c, DrawCmd::Sprite { .. })).count(), 13, "thickness 2: 12 around + 1");
    run(&mut e, "sprite_outline(T, null)\nsprite_overwrite(T, #4080ff, 0.5)\nglobal.fx = sprite_fx(T.id)");
    let c = fx_cmds(&mut e);
    assert_eq!(c.len(), 3);
    assert!(matches!(c[0], DrawCmd::SpriteFx(Some(f)) if (f[3] - 0.5).abs() < 1e-6 && (f[2] - 1.0).abs() < 1e-6));
    // reduced effects cap the flash
    run(&mut e, "sprite_fx_clear(T)\njuice_set_reduced(true)\nflash(T)");
    let c = fx_cmds(&mut e);
    assert!(matches!(c[0], DrawCmd::SpriteFx(Some(f)) if (f[3] as f64 - modengine::feel::REDUCED_FLASH).abs() < 1e-6), "{:?}", c[0]);
    run(&mut e, "juice_set_reduced(false)\nsprite_fx_clear(T)");
    ticks(&mut e, 10);
    // pulse: scale up and back to the base, restarting keeps the base
    run(&mut e, "pulse(T, 1.5, 0.2)");
    ticks(&mut e, 6);
    run(&mut e, "global.sx = T.scale_x\npulse(T, 1.5, 0.2)\nglobal.sx2 = T.scale_x");
    assert!(close(num(&e, "sx"), 1.5), "peak at half the time: {}", num(&e, "sx"));
    assert!(close(num(&e, "sx2"), 1.0), "a new pulse restarts from the base");
    ticks(&mut e, 12);
    run(&mut e, "global.sx = T.scale_x");
    assert!(close(num(&e, "sx"), 1.0));
    assert!(e.feel.fx.is_empty(), "finished effects are forgotten");
}

#[test]
fn popups_are_pooled_and_drawn_on_top() {
    let mut e = start(&project("pop", &[("objects/t.ms", &obj(""))]));
    run(&mut e, "popup_text(10, 10, \"12\", { color: #ff0000, duration: 0.5, rise: 30 })\npopup_text(5, 5, \"GUI\", { gui: true, outline: false })\nglobal.n = popup_count()");
    assert_eq!(num(&e, "n"), 2.0);
    e.render();
    let gui_at = e.cmds.iter().position(|c| matches!(c, DrawCmd::Gui)).unwrap();
    let texts: Vec<(usize, String, f32)> = e.cmds.iter().enumerate().filter_map(|(i, c)| if let DrawCmd::Text { text, scale, .. } = c { Some((i, text.clone(), *scale)) } else { None }).collect();
    assert_eq!(texts.iter().filter(|t| t.1 == "12").count(), 9, "8 outline copies + the text");
    assert!(texts.iter().filter(|t| t.1 == "12").all(|t| t.0 < gui_at), "world popup before the GUI");
    assert_eq!(texts.iter().filter(|t| t.1 == "GUI").count(), 1);
    assert!(texts.iter().any(|t| t.1 == "GUI" && t.0 > gui_at));
    assert!(texts.iter().all(|t| (t.2 - 1.6).abs() < 1e-6), "starts popped");
    ticks(&mut e, 31);
    run(&mut e, "global.n = popup_count()");
    assert_eq!(num(&e, "n"), 1.0, "the 0.5 s one ended, the 0.8 s one is still there");
    // pool: never more than POPUP_POOL slots, finished slots are reused
    run(&mut e, "for (var i = 0; i < 300; i++) popup_text(i, 0, str(i))\nglobal.n = popup_count()");
    assert_eq!(num(&e, "n"), modengine::feel::POPUP_POOL as f64);
    assert_eq!(e.feel.popups.len(), modengine::feel::POPUP_POOL);
    ticks(&mut e, 60);
    run(&mut e, "popup_text(0, 0, \"x\")\nglobal.n = popup_count()");
    assert_eq!(num(&e, "n"), 1.0);
    assert_eq!(e.feel.popups.len(), modengine::feel::POPUP_POOL, "no new slot");
    // reduced effects: no pop
    run(&mut e, "popup_clear()\njuice_set_reduced(true)\npopup_text(0, 0, \"r\")");
    e.render();
    assert!(e.cmds.iter().any(|c| matches!(c, DrawCmd::Text { text, scale, .. } if text == "r" && (*scale - 1.0).abs() < 1e-6)));
}

#[test]
fn reduced_effects_scale_the_camera_shake() {
    let mut e = start(&project("shake", &[("objects/t.ms", &obj("")), ("rooms/r.toml", "width = 2000\nheight = 2000\n[[layer]]\ntype = \"instances\"\n[[layer.instances]]\nobject = \"T\"\nx = 0\ny = 0\n")]));
    let view = |e: &mut Engine| {
        e.render();
        match e.cmds[0] {
            DrawCmd::View { x, y, .. } => (x, y),
            _ => panic!(),
        }
    };
    run(&mut e, "camera_set(500, 500)\ncamera_shake(40, 1000)");
    let mut full = 0.0f32;
    let mut red = 0.0f32;
    for _ in 0..30 {
        let (x, y) = view(&mut e);
        full = full.max((x - 500.0).abs().max((y - 500.0).abs()));
    }
    run(&mut e, "juice_set_reduced(true)\nglobal.r = juice_reduced()");
    assert!(g(&e, "r").truthy());
    for _ in 0..30 {
        let (x, y) = view(&mut e);
        red = red.max((x - 500.0).abs().max((y - 500.0).abs()));
    }
    assert!(full > 20.0 && red <= 40.0 * modengine::feel::REDUCED_SHAKE as f32 + 0.01, "full {full}, reduced {red}");
}

#[test]
fn focus_navigation_with_keys_and_stack() {
    let src = r#"object T {
    on create {
        for (var i = 0; i < 4; i++) ui_focus_add("hand", i, 10 + i * 40, 100, 32, 48)
        ui_focus_add("hand", "end", 140, 170, 40, 16)
    }
    on step {
        global.f = ui_focus_update()
        global.ok = ui_focus_confirm()
        if (ui_focus_back()) ui_focus_pop()
    }
}
"#;
    let mut e = start(&project("focus", &[("objects/t.ms", src)]));
    let press = |e: &mut Engine, k: &str| {
        let i = modengine::input::key_index(k).unwrap();
        e.input.pressed[i] = true;
        e.tick();
    };
    e.tick();
    assert_eq!(num(&e, "f"), 0.0);
    press(&mut e, "right");
    press(&mut e, "d");
    assert_eq!(num(&e, "f"), 2.0, "arrow and WASD");
    press(&mut e, "down");
    assert_eq!(g(&e, "f").to_string(), "end", "nearest below");
    press(&mut e, "up");
    assert_eq!(g(&e, "f").to_string(), "3", "back up to the closest card: {:?}", e.feel.focus.groups);
    press(&mut e, "right");
    assert_eq!(num(&e, "f"), 3.0, "no wrap");
    run(&mut e, "ui_focus_wrap(\"hand\", true)");
    press(&mut e, "right");
    assert_eq!(num(&e, "f"), 0.0, "wraps");
    press(&mut e, "enter");
    assert!(g(&e, "ok").truthy());
    e.tick();
    assert!(!g(&e, "ok").truthy(), "confirm is one step");
    // a nested panel takes the keys; back pops it and the hand keeps its focus
    run(&mut e, "ui_focus_push(\"dialog\")\nui_focus_add(\"dialog\", \"yes\", 0, 0, 10, 10)\nui_focus_add(\"dialog\", \"no\", 20, 0, 10, 10)");
    press(&mut e, "right");
    assert_eq!(g(&e, "f").to_string(), "no");
    run(&mut e, "global.grp = ui_focus_group()\nglobal.r = ui_focus_rect()");
    assert_eq!(g(&e, "grp").to_string(), "dialog");
    press(&mut e, "escape");
    run(&mut e, "global.grp = ui_focus_group()\nglobal.hf = ui_focus_get()\nglobal.at = ui_focus_at(95, 120)\nglobal.is = ui_focus_is(\"hand\", 0)");
    assert_eq!(g(&e, "grp").to_string(), "hand");
    assert_eq!(num(&e, "hf"), 0.0);
    assert_eq!(num(&e, "at"), 2.0);
    assert!(g(&e, "is").truthy());
    // custom keys
    run(&mut e, "ui_focus_keys({ right: key.l, confirm: [key.k] })");
    press(&mut e, "right");
    assert_eq!(num(&e, "f"), 0.0, "right arrow no longer mapped");
    press(&mut e, "l");
    assert_eq!(num(&e, "f"), 1.0);
    assert!(e.run_code("ui_focus_move(\"sideways\")").is_err());
    run(&mut e, "global.m = ui_focus_move(\"left\", \"hand\")");
    assert_eq!(num(&e, "m"), 0.0);
}

#[test]
fn layered_music_starts_together_and_crossfades() {
    let dir = project(
        "layers",
        &[
            ("objects/t.ms", &obj("")),
            ("sounds/calma.toml", "[synth]\nwave = \"sine\"\nfreq = 220\nduration = 1.0\n"),
            ("sounds/combate.toml", "[synth]\nwave = \"square\"\nfreq = 220\nduration = 1.0\n"),
        ],
    );
    let mut e = start(&dir);
    run(&mut e, "global.v = audio_play_layers([calma, combate], [1, 0])");
    ticks(&mut e, 12);
    run(&mut e, "var vs = audio_voices()\nglobal.p0 = vs[0].position\nglobal.p1 = vs[1].position\nglobal.g1 = vs[1].gain\naudio_layers_set(global.v, [0, 1], 100)");
    assert!(close(num(&e, "p0"), num(&e, "p1")), "same position: {} {}", num(&e, "p0"), num(&e, "p1"));
    assert!(num(&e, "p0") > 0.15);
    assert_eq!(num(&e, "g1"), 0.0);
    ticks(&mut e, 12);
    run(&mut e, "var vs = audio_voices()\nglobal.g0 = vs[0].gain\nglobal.g1 = vs[1].gain\nglobal.p0 = vs[0].position\nglobal.p1 = vs[1].position");
    assert!(close(num(&e, "g0"), 0.0) && close(num(&e, "g1"), 1.0), "crossfaded");
    assert!(close(num(&e, "p0"), num(&e, "p1")), "still in sync");
}

// ------------------------------------------------------------------ localization

const ES: &str = "title = \"Mazmorra\"\nonly_es = \"Solo aquí\"\n[menu]\nplay = \"Jugar\"\ngreet = \"Hola, {name}\"\n[coins]\nzero = \"Sin monedas\"\none = \"{n} moneda\"\nother = \"{n} monedas\"\n";
const EN: &str = "title = \"Dungeon\"\nonly_en = \"Only here\"\n[menu]\nplay = \"Play\"\ngreet = \"Hi, {name}\"\n[coins]\none = \"{n} coin\"\nother = \"{n} coins\"\n";

fn loc_project(name: &str, code: &str) -> PathBuf {
    project(
        name,
        &[
            ("game.toml", "[game]\nname = \"t\"\n[palette]\nr = \"#ff0000\"\n[localization]\ndefault = \"es\"\nfallback = \"en\"\n"),
            ("lang/es.toml", ES),
            ("lang/en.toml", EN),
            ("objects/t.ms", &obj(code)),
        ],
    )
}

fn loc(e: &mut Engine, expr: &str) -> String {
    run(e, &format!("global.r = {expr}"));
    g(e, "r").to_string()
}

#[test]
fn loc_lookup_placeholders_plurals_fallback_pseudo() {
    let mut e = start(&loc_project("loc", ""));
    assert_eq!(loc(&mut e, "loc_language()"), "es");
    assert_eq!(loc(&mut e, "loc(\"menu.play\")"), "Jugar");
    assert_eq!(loc(&mut e, "loc(\"menu.greet\", { name: \"Ana\" })"), "Hola, Ana");
    assert_eq!(loc(&mut e, "loc(\"coins\", { n: 1 })"), "1 moneda");
    assert_eq!(loc(&mut e, "loc(\"coins\", { n: 5 })"), "5 monedas");
    assert_eq!(loc(&mut e, "loc(\"coins\", { n: 0 })"), "Sin monedas");
    assert_eq!(loc(&mut e, "loc(\"only_en\")"), "Only here", "falls back to en");
    assert_eq!(loc(&mut e, "loc(\"nope.key\")"), "nope.key", "missing: the key itself");
    assert_eq!(loc(&mut e, "loc_missing()[0]"), "nope.key");
    assert_eq!(loc(&mut e, "loc_exists(\"title\")"), "true");
    assert_eq!(loc(&mut e, "loc_exists(\"nope.key\")"), "false");
    assert_eq!(loc(&mut e, "loc_languages()"), "[\"en\", \"es\", \"pseudo\"]");
    assert_eq!(loc(&mut e, "loc_set_language(\"en\")"), "true");
    assert_eq!(loc(&mut e, "loc(\"title\")"), "Dungeon");
    assert_eq!(loc(&mut e, "loc(\"coins\", { n: 0 })"), "0 coins", "no zero form in English");
    assert_eq!(loc(&mut e, "loc(\"only_es\")"), "Solo aquí", "then the default language");
    assert_eq!(loc(&mut e, "loc_set_language(\"xx\")"), "false");
    assert_eq!(loc(&mut e, "loc_language()"), "en");
    // pseudo language: the default language, accented, longer, in brackets
    loc(&mut e, "loc_set_language(\"pseudo\")");
    let p = loc(&mut e, "loc(\"menu.greet\", { name: \"Ana\" })");
    assert!(p.starts_with("[Hölá, Ana") && p.ends_with(']') && p.contains('~'), "{p}");
    assert_eq!(loc(&mut e, "loc_pseudo()"), "true");
    // pseudo on top of a real language
    loc(&mut e, "loc_set_language(\"en\")\nloc_set_pseudo(true)");
    let p = loc(&mut e, "loc(\"menu.play\")");
    assert!(p.starts_with("[Pláý") && p.chars().count() >= 7, "{p}");
    loc(&mut e, "loc_set_pseudo(false)");
    assert_eq!(loc(&mut e, "loc_plural(3)"), "other");
    assert_eq!(loc(&mut e, "loc_fallback_language()"), "en");
    assert_eq!(loc(&mut e, "len(loc_keys(\"es\"))"), "5");
    assert!(e.errors.is_empty(), "{:?}", e.errors);
}

#[test]
fn loc_without_config_uses_first_language_and_reports_bad_tables() {
    let mut e = start(&project("loc2", &[("objects/t.ms", &obj("")), ("lang/fr.toml", "a = \"A\"\nb = 3\n"), ("lang/de.toml", "a = \"A-de\"\n")]));
    assert_eq!(loc(&mut e, "loc_language()"), "de", "alphabetical first");
    assert_eq!(loc(&mut e, "loc(\"a\")"), "A-de");
    assert!(e.errors.iter().any(|m| m.contains("lang/fr.toml") && m.contains("'b'")), "{:?}", e.errors);
    // no lang/ at all: loc() returns the key
    let mut e = start(&project("loc3", &[("objects/t.ms", &obj(""))]));
    assert_eq!(loc(&mut e, "loc(\"hello\")"), "hello");
    assert_eq!(loc(&mut e, "loc_languages()"), "[]");
}

#[test]
fn check_reports_missing_and_unused_keys_with_lines() {
    let code = "    on create {\n        show_debug_message(loc(\"menu.play\"))\n        var t = loc(\"only_es\")\n        var u = loc(\"ghost.key\")\n    }";
    let e = Engine::load(&loc_project("check", code)).unwrap();
    let found = modengine::lint::check(&e);
    let text: Vec<String> = found.iter().map(|f| f.to_string()).collect();
    let file = "objects/t.ms";
    let only_es = found.iter().find(|f| f.msg.contains("'only_es'")).unwrap_or_else(|| panic!("{text:#?}"));
    assert!(only_es.file.ends_with(file) && only_es.line == 5 && only_es.msg.contains("lang/en.toml") && !only_es.msg.contains("lang/es.toml"), "{only_es}");
    assert_eq!(only_es.severity, modengine::lint::Severity::Warning);
    let ghost = found.iter().find(|f| f.msg.contains("'ghost.key'")).unwrap();
    assert_eq!(ghost.severity, modengine::lint::Severity::Error, "missing everywhere");
    assert_eq!(ghost.line, 6);
    assert!(!found.iter().any(|f| f.msg.contains("'menu.play'")), "present in both");
    let unused_es = found.iter().find(|f| f.file == "lang/es.toml").unwrap_or_else(|| panic!("{text:#?}"));
    assert!(unused_es.msg.contains("title") && unused_es.msg.contains("coins") && !unused_es.msg.contains("menu.play"), "{unused_es}");
}
