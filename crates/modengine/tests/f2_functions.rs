//! Functions part 2, run headless: coroutines, call_later, instance pools, RNG streams,
//! atomic saves with defaults, and the audio mixer driven from scripts.

use modengine::Engine;
use modscript::{intern, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

static N: AtomicU32 = AtomicU32::new(0);

fn project(files: &[(&str, &[u8])]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("modengine-f2-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&dir);
    for d in ["rooms", "sprites", "sounds", "objects"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    std::fs::write(dir.join("game.toml"), "[game]\nname = \"f2\"\nrooms = [\"r\", \"r2\"]\n\n[display]\nwidth = 320\nheight = 240\n\n[palette]\n\"w\" = \"#ffffff\"\n").unwrap();
    std::fs::write(dir.join("sprites/dot.toml"), "pixels = ['''\nw\n''']\n").unwrap();
    std::fs::write(dir.join("rooms/r.toml"), "width = 320\nheight = 240\n").unwrap();
    std::fs::write(dir.join("rooms/r2.toml"), "width = 320\nheight = 240\n").unwrap();
    for (name, data) in files {
        let p = dir.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, data).unwrap();
    }
    dir
}

fn start(dir: &Path) -> Engine {
    let mut e = Engine::load(dir).unwrap_or_else(|err| panic!("{err}"));
    e.save_dir = dir.join("saves");
    e.start();
    e
}

fn g(e: &Engine, name: &str) -> Value {
    e.globals.get(&intern(name)).cloned().unwrap_or(Value::Null)
}

fn num(e: &Engine, name: &str) -> f64 {
    g(e, name).as_num().unwrap_or_else(|| panic!("global.{name} = {}", g(e, name)))
}

fn run(e: &mut Engine, code: &str) {
    e.run_code(code).unwrap_or_else(|err| panic!("{err}"));
}

fn count(e: &Engine, obj: &str) -> usize {
    e.insts.iter().filter(|i| i.alive && e.objects[i.object as usize].name == obj).count()
}

/// 16-bit mono WAV at 44100 Hz with `frames` samples of `v`.
fn wav(frames: u32, v: i16) -> Vec<u8> {
    let mut w = Vec::new();
    w.extend(b"RIFF");
    w.extend((36 + frames * 2).to_le_bytes());
    w.extend(b"WAVEfmt ");
    w.extend(16u32.to_le_bytes());
    w.extend(1u16.to_le_bytes());
    w.extend(1u16.to_le_bytes());
    w.extend(44100u32.to_le_bytes());
    w.extend((44100u32 * 2).to_le_bytes());
    w.extend(2u16.to_le_bytes());
    w.extend(16u16.to_le_bytes());
    w.extend(b"data");
    w.extend((frames * 2).to_le_bytes());
    for _ in 0..frames {
        w.extend(v.to_le_bytes());
    }
    w
}

// ------------------------------------------------------------------ coroutines

const BOSS: &str = r#"
object Bala {
    sprite = dot
    on create { global.creadas += 1 }
}
object Jefe {
    sprite = dot
    on create {
        // a bullet pattern: 3 bursts of 4 bullets, 10 steps apart, then a spiral
        coroutine(function() {
            repeat (3) {
                for (var i = 0; i < 4; i++) spawn(Bala, x, y, 0, { direction: i * 90 })
                global.rafagas += 1
                wait(10)
            }
            var a = 0
            while (a < 360) {
                spawn(Bala, x, y, 0, { direction: a })
                a += 45
                yield()
            }
            global.fin_patron = steps()
        })
    }
}
"#;

#[test]
fn coroutines_drive_a_bullet_pattern_step_by_step() {
    let dir = project(&[("objects/o.ms", BOSS.as_bytes()), ("scripts/g.ms", b"global.creadas = 0\nglobal.rafagas = 0\nglobal.fin_patron = -1\n")]);
    let mut e = start(&dir);
    run(&mut e, "global.jefe = spawn(Jefe, 100, 100)");
    // the first burst runs at once, inside spawn()
    assert_eq!(num(&e, "rafagas"), 1.0);
    assert_eq!(count(&e, "Bala"), 4);
    for _ in 0..9 {
        e.tick();
    }
    assert_eq!(num(&e, "rafagas"), 1.0, "wait(10) still waiting after 9 steps");
    e.tick();
    assert_eq!(num(&e, "rafagas"), 2.0);
    assert_eq!(count(&e, "Bala"), 8);
    for _ in 0..10 {
        e.tick();
    }
    assert_eq!(num(&e, "rafagas"), 3.0);
    // 10 more steps to leave the repeat, then one bullet per step for the spiral (8)
    for _ in 0..30 {
        e.tick();
    }
    assert_eq!(count(&e, "Bala"), 12 + 8);
    assert!(num(&e, "fin_patron") > 0.0);
    assert!(e.errors.is_empty(), "{:?}", e.errors);
    run(&mut e, "global.quedan = coroutine_count()");
    assert_eq!(num(&e, "quedan"), 0.0, "a finished coroutine is gone");
}

#[test]
fn coroutines_die_with_their_instance_and_global_ones_survive_rooms() {
    let src = r#"
object Tic {
    on create { global.co = coroutine(function() { while (true) { global.tics += 1; yield() } }) }
}
"#;
    let dir = project(&[("objects/o.ms", src.as_bytes()), ("scripts/g.ms", b"global.tics = 0\nglobal.g = 0\n")]);
    let mut e = start(&dir);
    run(&mut e, "global.t = spawn(Tic, 0, 0)\nglobal.gco = coroutine(function(paso) { while (true) { global.g += paso; wait(2) } }, 5)");
    assert_eq!((num(&e, "tics"), num(&e, "g")), (1.0, 5.0));
    for _ in 0..4 {
        e.tick();
    }
    assert_eq!(num(&e, "tics"), 5.0);
    assert_eq!(num(&e, "g"), 15.0, "wait(2): steps 2 and 4");
    run(&mut e, "destroy(global.t)\nglobal.vivo = coroutine_running(global.co)");
    e.tick();
    run(&mut e, "global.vivo = coroutine_running(global.co)");
    assert_eq!(g(&e, "vivo").truthy(), false);
    assert_eq!(num(&e, "tics"), 5.0);
    // the global one goes on in another room
    run(&mut e, "goto_room(r2)");
    for _ in 0..4 {
        e.tick();
    }
    assert!(num(&e, "g") >= 25.0);
    run(&mut e, "coroutine_pause(global.gco)");
    let before = num(&e, "g");
    for _ in 0..6 {
        e.tick();
    }
    assert_eq!(num(&e, "g"), before, "paused");
    run(&mut e, "coroutine_resume(global.gco)\ncoroutine_stop(global.gco)");
    e.tick();
    run(&mut e, "global.n = coroutine_count()");
    assert_eq!(num(&e, "n"), 0.0);
}

#[test]
fn wait_until_wait_seconds_and_call_later() {
    let dir = project(&[("scripts/g.ms", b"global.listo = false\nglobal.paso_listo = -1\nglobal.seg = -1\nglobal.tarde = []\n")]);
    let mut e = start(&dir);
    run(
        &mut e,
        r#"
coroutine(function() { wait_until(function() { return global.listo }); global.paso_listo = steps() })
coroutine(function() { wait_seconds(0.5); global.seg = steps() })
call_later(3, time_source_units_frames, function() { push(global.tarde, steps()) })
global.rep = call_later(2, time_source_units_frames, function() { push(global.tarde, -steps()) }, true)
"#,
    );
    for _ in 0..5 {
        e.tick();
    }
    run(&mut e, "global.listo = true\ncall_cancel(global.rep)");
    e.tick();
    for _ in 0..30 {
        e.tick();
    }
    assert_eq!(num(&e, "paso_listo"), 6.0);
    assert_eq!(num(&e, "seg"), 30.0, "0.5 s at 60 steps per second");
    let Value::Array(t) = g(&e, "tarde") else { panic!() };
    let t: Vec<f64> = t.borrow().iter().map(|v| v.as_num().unwrap()).collect();
    assert_eq!(t, vec![-2.0, 3.0, -4.0]);
    assert!(e.errors.is_empty(), "{:?}", e.errors);
}

#[test]
fn coroutine_errors_are_reported_with_line_and_stop_it() {
    let dir = project(&[("scripts/g.ms", b"global.x = 0\n")]);
    let mut e = start(&dir);
    run(&mut e, "global.c = coroutine(function() {\n wait(1)\n global.x = no_existe + 1\n})");
    e.tick();
    assert!(e.errors.iter().any(|m| m.contains("no_existe") && m.contains("coroutine")), "{:?}", e.errors);
    run(&mut e, "global.vivo = coroutine_running(global.c)");
    assert!(!g(&e, "vivo").truthy());
    // wait() outside a coroutine explains itself
    let err = e.run_code("wait(3)").unwrap_err();
    assert!(err.contains("corrutina"), "{err}");
}

// ------------------------------------------------------------------ pools

#[test]
fn pools_reuse_storage_and_rerun_create_with_new_ids() {
    let src = r#"
object Bala {
    sprite = dot
    vida = 3
    on create { global.creates += 1; vida += 1 }
    on destroy { global.destroys += 1 }
}
"#;
    let dir = project(&[("objects/o.ms", src.as_bytes()), ("scripts/g.ms", b"global.creates = 0\nglobal.destroys = 0\n")]);
    let mut e = start(&dir);
    run(&mut e, "instance_pool_create(Bala, 2)\nglobal.libres = pool_free_count(Bala)\nglobal.a = pool_spawn(Bala, 5, 6, 0, { vida: 10 })");
    assert_eq!(num(&e, "libres"), 2.0);
    run(&mut e, "global.va = global.a.vida\npool_release(global.a)\nglobal.existe = exists(global.a)");
    assert_eq!(num(&e, "va"), 11.0, "vars applied before create, like spawn");
    assert!(!g(&e, "existe").truthy(), "released = gone from the room");
    assert_eq!(num(&e, "destroys"), 0.0, "no destroy event by default");
    e.tick();
    run(&mut e, "global.b = pool_spawn(Bala, 1, 2)\nglobal.vb = global.b.vida\nglobal.s = pool_stats()");
    assert_ne!(g(&e, "a").to_string(), g(&e, "b").to_string(), "a new id: old references stay dead");
    assert_eq!(num(&e, "vb"), 4.0, "property initializers ran again");
    assert_eq!(num(&e, "creates"), 2.0);
    let Value::Struct(s) = g(&e, "s") else { panic!() };
    assert_eq!(s.borrow().fields[&intern("reused")].as_num(), Some(2.0));
    run(&mut e, "pool_release(global.b, true)");
    assert_eq!(num(&e, "destroys"), 1.0);
}

#[test]
fn pool_vs_spawn_destroy_benchmark() {
    // not a pass/fail speed test: prints both timings (cargo test -- --nocapture)
    let src = r#"
object Bala {
    sprite = dot
    dano = 1
    vida = 60
    rastro = []
    on create { direction = rand(360); speed = 4 }
}
"#;
    let dir = project(&[("objects/o.ms", src.as_bytes())]);
    let mut e = start(&dir);
    let n = 20_000;
    let t0 = std::time::Instant::now();
    run(&mut e, &format!("repeat ({}) {{ var b = spawn(Bala, 10, 10)\n destroy(b, false) }}", n));
    e.tick();
    let plain = t0.elapsed();
    run(&mut e, "instance_pool_create(Bala, 64)");
    let t1 = std::time::Instant::now();
    run(&mut e, &format!("repeat ({}) {{ var b = pool_spawn(Bala, 10, 10)\n pool_release(b) }}", n));
    e.tick();
    let pooled = t1.elapsed();
    println!("spawn+destroy x{n}: {plain:?}   pool_spawn+pool_release x{n}: {pooled:?}");
    assert_eq!(count(&e, "Bala"), 0);
    // steady state: 2000 live bullets, 100 replaced every step, 200 steps
    for (label, spawn, kill) in [("spawn/destroy", "spawn", "destroy(b, false)"), ("pool", "pool_spawn", "pool_release(b)")] {
        run(&mut e, &format!("global.vivas = []\nrepeat (2000) push(global.vivas, {spawn}(Bala, 10, 10))"));
        let code = format!("repeat (100) {{ var b = global.vivas[0]\n remove(global.vivas, 0)\n {kill}\n push(global.vivas, {spawn}(Bala, 10, 10)) }}");
        let t = std::time::Instant::now();
        for _ in 0..200 {
            run(&mut e, &code);
            e.tick();
        }
        println!("{label}: 2000 live, 100 replaced per step: {:?} per step", t.elapsed() / 200);
        run(&mut e, &format!("for (b in global.vivas) {kill}"));
        e.tick();
    }
}

// ------------------------------------------------------------------ RNG streams

#[test]
fn rng_streams_are_independent_and_reproducible() {
    let dir = project(&[]);
    let mut e = start(&dir);
    run(
        &mut e,
        r#"
var g1 = rng_create(1234)
global.a = [rng_irandom(g1, 100), rng_irandom(g1, 100), rng_irandom(g1, 100)]
var g2 = rng_create(1234)
rand(); rand(); irand(1, 6)   // gameplay randomness in between does not matter
global.b = [rng_irandom(g2, 100), rng_irandom(g2, 100), rng_irandom(g2, 100)]
var g3 = rng_create("mazmorra")
var st = rng_get_state(g3)
global.c1 = rng_random(g3)
rng_set_state(g3, st)
global.c2 = rng_random(g3)
var arr = [1, 2, 3, 4, 5, 6]
rng_shuffle(rng_create(7), arr)
global.sh = arr
var arr2 = [1, 2, 3, 4, 5, 6]
rng_shuffle(rng_create(7), arr2)
global.sh2 = arr2
global.ch = rng_choose(g1, "a", "b", "c")
global.r = rng_random_range(g1, 5, 6)
global.ir = rng_irandom_range(g1, -2, 2)
"#,
    );
    assert_eq!(g(&e, "a").to_string(), g(&e, "b").to_string());
    assert_eq!(num(&e, "c1"), num(&e, "c2"));
    assert_eq!(g(&e, "sh").to_string(), g(&e, "sh2").to_string());
    assert_ne!(g(&e, "sh").to_string(), "[1, 2, 3, 4, 5, 6]");
    assert!(["a", "b", "c"].contains(&g(&e, "ch").to_string().as_str()));
    assert!((5.0..6.0).contains(&num(&e, "r")));
    assert!((-2.0..=2.0).contains(&num(&e, "ir")));
    // rng_create() without a seed follows seed() / --seed
    run(&mut e, "seed(9)\nglobal.u1 = rng_irandom(rng_create(), 1000000)\nseed(9)\nglobal.u2 = rng_irandom(rng_create(), 1000000)");
    assert_eq!(num(&e, "u1"), num(&e, "u2"));
}

// ------------------------------------------------------------------ saves

#[test]
fn saves_are_atomic_with_backup_and_defaults_merge() {
    let dir = project(&[]);
    let mut e = start(&dir);
    run(&mut e, "save_data(\"prog\", { nivel: 3, opciones: { musica: 0.2 } })\nsave_data(\"prog\", { nivel: 4, opciones: { musica: 0.3 } })");
    let saves = dir.join("saves");
    assert!(saves.join("prog.json").exists() && saves.join("prog.json.bak").exists());
    assert!(!saves.join("prog.json.tmp").exists());
    run(&mut e, "global.p = load_data(\"prog\", { nivel: 1, monedas: 0, opciones: { musica: 1, efectos: 0.9 } })");
    run(&mut e, "global.n = global.p.nivel\nglobal.m = global.p.monedas\nglobal.mu = global.p.opciones.musica\nglobal.ef = global.p.opciones.efectos");
    assert_eq!((num(&e, "n"), num(&e, "m"), num(&e, "mu"), num(&e, "ef")), (4.0, 0.0, 0.3, 0.9));
    // corrupt the main file: the backup (previous save) is loaded
    std::fs::write(saves.join("prog.json"), "{ \"nivel\": 5, ").unwrap();
    run(&mut e, "global.p = load_data(\"prog\", { nivel: 1 })\nglobal.n = global.p.nivel\nglobal.ex = save_exists(\"prog\")");
    assert_eq!(num(&e, "n"), 3.0);
    assert!(g(&e, "ex").truthy());
    run(&mut e, "save_delete(\"prog\")\nglobal.ex = save_exists(\"prog\")\nglobal.d = load_data(\"prog\", { nivel: 1 }).nivel");
    assert!(!g(&e, "ex").truthy());
    assert_eq!(num(&e, "d"), 1.0);
}

// ------------------------------------------------------------------ audio from scripts

#[test]
fn audio_voices_pitch_fades_groups_and_intro_loop_headless() {
    let intro = wav(4410, 3000); // 0.1 s
    let main = wav(8820, 6000); // 0.2 s
    let dir = project(&[
        ("sounds/intro.wav", &intro),
        ("sounds/bucle.wav", &main),
        ("sounds/musica.toml", b"file = \"bucle.wav\"\nintro = \"intro.wav\"\ngroup = \"musica\"\nvolume = 0.8\n"),
        ("sounds/tiro.toml", b"group = \"efectos\"\n[synth]\nwave = \"square\"\nfreq = 440\nduration = 0.25\n"),
    ]);
    let mut e = start(&dir);
    run(
        &mut e,
        r#"
global.mus = play_sound(musica, { loop: true })
global.t1 = play_sound(tiro, { pitch: 2, pan: -1, volume: 0.5 })
global.t2 = audio_play_sound(tiro, 1, false)
global.len = audio_sound_length(musica)
global.grupo = audio_sound_get_audio_group(musica)
audio_group_set_gain("efectos", 0, 1000)
"#,
    );
    assert!((num(&e, "len") - 0.3).abs() < 0.001, "intro + loop = 0.3 s: {}", num(&e, "len"));
    assert_eq!(g(&e, "grupo").to_string(), "musica");
    // 9 steps = 0.15 s: the pitch-2 voice is at 0.30 of its 0.25 s → finished
    for _ in 0..9 {
        e.tick();
    }
    run(&mut e, "global.p1 = audio_is_playing(global.t1)\nglobal.p2 = audio_is_playing(global.t2)\nglobal.pos2 = audio_sound_get_track_position(global.t2)\nglobal.gg = audio_group_get_gain(\"efectos\")");
    assert!(!g(&e, "p1").truthy(), "pitch 2 halves the length");
    assert!(g(&e, "p2").truthy());
    assert!((num(&e, "pos2") - 0.15).abs() < 0.01, "{}", num(&e, "pos2"));
    assert!((num(&e, "gg") - 0.85).abs() < 0.02, "group fade in progress: {}", num(&e, "gg"));
    // the music went past the intro and loops inside the main part (0.1 .. 0.3 s)
    for _ in 0..60 {
        e.tick();
    }
    run(&mut e, "global.mp = audio_sound_get_track_position(global.mus)\nglobal.mon = audio_is_playing(global.mus)");
    assert!(g(&e, "mon").truthy());
    let mp = num(&e, "mp");
    assert!((0.1..0.3).contains(&mp), "{mp}");
    // fade out and stop
    run(&mut e, "audio_stop_sound(global.mus, 100)");
    for _ in 0..8 {
        e.tick();
    }
    run(&mut e, "global.mon = audio_is_playing(musica)\nglobal.v = len(audio_voices())");
    assert!(!g(&e, "mon").truthy());
    assert!(e.errors.is_empty(), "{:?}", e.errors);
}

#[test]
fn intro_loop_from_two_sounds_and_positional_listener() {
    let dir = project(&[("sounds/a.wav", &wav(441, 1000)), ("sounds/b.wav", &wav(882, 2000))]);
    let mut e = start(&dir);
    run(&mut e, "global.v = audio_play_sound_intro_loop(a, b)");
    for _ in 0..3 {
        e.tick();
    }
    run(&mut e, "var info = audio_voices()[0]\nglobal.snd = info.sound == b\nglobal.lp = info.loop\nglobal.same = info.id == global.v");
    assert!(g(&e, "snd").truthy(), "now playing the loop sound");
    assert!(g(&e, "lp").truthy() && g(&e, "same").truthy());
    // the listener follows the view centre (160, 120) unless placed
    run(&mut e, "audio_stop_all()\nglobal.em = audio_emitter_create()\naudio_emitter_position(global.em, 1000, 120)\naudio_emitter_falloff(global.em, 50, 400, 1)\nglobal.s = audio_play_sound_on(global.em, b, true, 0)");
    e.tick();
    {
        let m = modengine::audio::handle(&mut e);
        let m = m.lock().unwrap();
        assert_eq!(m.listener, (160.0, 120.0));
    }
    run(&mut e, "audio_listener_position(1000, 120)");
    e.tick();
    let m = modengine::audio::handle(&mut e);
    assert_eq!(m.lock().unwrap().listener, (1000.0, 120.0));
}
