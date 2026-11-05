//! Particle systems, shaders and extensions, run headless on the showcase example and on
//! small projects written to temp folders.

use modengine::draw::DrawCmd;
use modengine::Engine;
use std::path::{Path, PathBuf};

fn showcase() -> Engine {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/showcase_assets_s3");
    Engine::load(&dir).unwrap_or_else(|e| panic!("showcase failed to load:\n{e}"))
}

fn run(e: &mut Engine, steps: u32) {
    for _ in 0..steps {
        e.tick();
        e.render();
        e.audio.clear();
    }
}

/// A minimal project in a temp folder: `files` are (relative path, contents).
fn project(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("me_effects_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut all: Vec<(&str, &str)> = vec![
        ("game.toml", "[game]\nname = \"t\"\n"),
        ("rooms/r.toml", "[[layer]]\ntype = \"instances\"\n[[layer.instances]]\nobject = \"T\"\nx = 0\ny = 0\n"),
    ];
    all.extend_from_slice(files);
    for (rel, text) in all {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    dir
}

fn global(e: &mut Engine, name: &str) -> modengine::modscript::Value {
    e.globals.get(&modengine::modscript::intern(name)).cloned().unwrap_or_default()
}

fn load(dir: &Path) -> Engine {
    Engine::load(dir).unwrap_or_else(|e| panic!("failed to load:\n{e}"))
}

#[test]
fn showcase_runs_and_draws_particles_with_shaders() {
    let mut e = showcase();
    e.start();
    run(&mut e, 200);
    assert!(e.errors.is_empty(), "runtime errors: {:?}", e.errors);
    let systems: Vec<usize> = e.parts.world.systems.iter().enumerate().filter(|(_, s)| s.is_some()).map(|(i, _)| i).collect();
    assert_eq!(systems.len(), 3, "fire, snow and the code-built trail");
    for s in &systems {
        assert!(e.parts.world.count(*s) > 0, "system {s} has particles");
    }
    // the burst asset fired into the trail system (bits are stars, mixed colours)
    let trail = e.parts.world.systems[2].as_ref().unwrap();
    let star = modengine::particles::Tex::Shape(4);
    assert!(trail.parts.iter().any(|p| e.parts.world.types[p.ty].as_ref().unwrap().tex == star), "burst bits are in the trail system");
    // draw list: shader on/off, uniforms, additive blend around the flames, shape sprites
    let has = |f: &dyn Fn(&DrawCmd) -> bool| e.cmds.iter().any(f);
    assert!(has(&|c| matches!(c, DrawCmd::Shader(Some(_)))));
    assert!(has(&|c| matches!(c, DrawCmd::Shader(None))));
    assert!(has(&|c| matches!(c, DrawCmd::Uniform { name, vals, .. } if name == "u_time" && vals.len() == 1)));
    assert!(has(&|c| matches!(c, DrawCmd::Blend(1))));
    let base = e.parts.shape_base;
    assert!(has(&|c| matches!(c, DrawCmd::Sprite { spr, .. } if *spr >= base)));
    assert_eq!(e.sprites.len() as u32, base + modengine::particles::SHAPE_NAMES.len() as u32);
    // extension: constants, init ran before the project's code
    assert_eq!(global(&mut e, "easing_ready").to_string(), "true");
}

#[test]
fn code_built_systems_follow_gamemaker_api() {
    let dir = project(
        "api",
        &[(
            "objects/t.ms",
            r#"
object T {
    on create {
        global.ps = part_system_create()
        global.pt = part_type_create()
        part_type_shape(global.pt, pt_shape_disk)
        part_type_life(global.pt, 10, 10)
        part_type_speed(global.pt, 2, 2, 0, 0)
        part_type_direction(global.pt, 0, 0, 0, 0)
        part_type_color2(global.pt, #ff0000, #0000ff)
        global.em = part_emitter_create(global.ps)
        part_emitter_region(global.ps, global.em, 100, 100, 50, 50, ps_shape_rectangle, ps_distr_linear)
        part_emitter_stream(global.ps, global.em, global.pt, 2)
        part_particles_create(global.ps, 0, 0, global.pt, 5)
        global.count0 = part_particles_count(global.ps)
    }
}
"#,
        )],
    );
    let mut e = load(&dir);
    e.start();
    assert_eq!(global(&mut e, "count0").to_string(), "5");
    run(&mut e, 1);
    // 5 created by hand + 2 streamed this step
    let ps = global(&mut e, "ps").as_num().unwrap() as usize;
    assert_eq!(e.parts.world.count(ps), 7);
    let p = e.parts.world.systems[ps].as_ref().unwrap().parts.iter().find(|p| p.x >= 100.0).unwrap().clone();
    assert_eq!(p.y, 50.0);
    run(&mut e, 3);
    // moving right at 2 px/step; the first 5 die after 10 steps
    run(&mut e, 10);
    assert_eq!(e.parts.world.count(ps), 20, "steady state: 2 per step × 10 steps of life");
    assert!(e.errors.is_empty(), "{:?}", e.errors);
    // destroying the system and type
    e.set_startup_code("").unwrap();
    let code = "part_emitter_destroy_all(global.ps)\npart_type_destroy(global.pt)\nglobal.after = part_particles_count(global.ps)\npart_system_destroy(global.ps)\nglobal.gone = !part_system_exists(global.ps)";
    let stmts = modengine::modscript::parse_stmts("<t>", code).unwrap();
    modengine::modscript::run_stmts(&mut e, &stmts, &std::rc::Rc::from("<t>"), modengine::modscript::CallCx::global()).unwrap();
    assert_eq!(global(&mut e, "after").to_string(), "0", "destroying a type removes its particles");
    assert_eq!(global(&mut e, "gone").to_string(), "true");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bad_assets_are_reported() {
    let dir = project(
        "bad",
        &[
            ("objects/t.ms", "object T {}\n"),
            ("particles/p.toml", "[[type]]\nname = \"a\"\nshape = \"blob\"\n[[emitter]]\ntype = \"zzz\"\n"),
        ],
    );
    let err = Engine::load(&dir).err().expect("should fail");
    assert!(err.contains("unknown shape 'blob'") && err.contains("type 'zzz' does not exist"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn shaders_load_check_and_set_uniforms() {
    let dir = project(
        "sh",
        &[
            ("shaders/sh_ok.fsh", "varying vec2 v_vTexcoord;\nvarying vec4 v_vColour;\nuniform vec2 u_off;\nuniform int u_mode;\nvoid main() { gl_FragColor = v_vColour * texture2D(gm_BaseTexture, v_vTexcoord + u_off); }\n"),
            ("shaders/sh_bad/sh_bad.fsh", "void main() {\n  gl_FragColor = vec4(1.0)\n}\n"),
            (
                "objects/t.ms",
                r#"
object T {
    on draw {
        shader_set(sh_ok)
        var u = shader_get_uniform(sh_ok, "u_off")
        shader_set_uniform_f(u, 0.5, 0.25)
        shader_set_uniform_i(shader_get_uniform(sh_ok, "u_mode"), 2)
        global.missing = shader_get_uniform(sh_ok, "nope")
        global.ok = shader_is_compiled(sh_ok)
        global.bad = shader_is_compiled(sh_bad)
        global.cur = shader_current() == sh_ok
        gpu_set_blendmode(bm_add)
        global.bm = gpu_get_blendmode()
        gpu_set_blendmode(bm_normal)
        shader_reset()
    }
}
"#,
            ),
        ],
    );
    let mut e = load(&dir);
    // the broken shader is reported (statically, without a GPU) but the game still loads
    assert!(e.errors.iter().any(|m| m.contains("sh_bad") && m.contains("fsh:2")), "{:?}", e.errors);
    e.errors.clear();
    e.start();
    run(&mut e, 1);
    assert!(e.errors.is_empty(), "{:?}", e.errors);
    for (k, v) in [("missing", "-1"), ("ok", "true"), ("bad", "false"), ("cur", "true"), ("bm", "1")] {
        assert_eq!(global(&mut e, k).to_string(), v, "{k}");
    }
    assert!(e.cmds.iter().any(|c| matches!(c, DrawCmd::Uniform { name, vals, .. } if name == "u_off" && *vals == vec![0.5, 0.25])));
    assert!(e.cmds.iter().any(|c| matches!(c, DrawCmd::Uniform { name, vals, .. } if name == "u_mode" && *vals == vec![2.0])));
    // wrong number of values is an error
    let stmts = modengine::modscript::parse_stmts("<t>", "shader_set_uniform_f(shader_get_uniform(sh_ok, \"u_off\"), 1)").unwrap();
    let r = modengine::modscript::run_stmts(&mut e, &stmts, &std::rc::Rc::from("<t>"), modengine::modscript::CallCx::global());
    assert!(r.unwrap_err().msg.contains("takes 2 value"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn extensions_load_first_with_constants_and_hooks() {
    let dir = project(
        "ext",
        &[
            ("extensions/lib/extension.toml", "name = \"lib\"\nversion = \"2.0\"\ninit = \"lib_init\"\nfiles = [\"b.ms\", \"a.ms\"]\n[constants]\nLIB_K = 7\n[[function]]\nname = \"lib_double\"\n"),
            ("extensions/lib/a.ms", "global.order = global.order + \"a\"\nfunction lib_double(x) { return x * 2 }\n"),
            ("extensions/lib/b.ms", "global.order = \"b\"\nfunction lib_init() { global.order = global.order + \"i\" }\n"),
            ("extensions/off/extension.toml", "enabled = false\n"),
            ("extensions/off/x.ms", "function lib_double(x) { return 0 }\n"),
            ("scripts/main.ms", "global.order = global.order + \"m\"\nglobal.v = lib_double(LIB_K)\nglobal.info = extension_get_info(\"lib\").version\nglobal.has_off = extension_exists(\"off\")\n"),
            ("objects/t.ms", "object T {}\n"),
        ],
    );
    let mut e = load(&dir);
    e.start();
    assert!(e.errors.is_empty(), "{:?}", e.errors);
    assert_eq!(global(&mut e, "order").to_string(), "baim", "extension files in `files` order, init right after them, then the project");
    assert_eq!(global(&mut e, "v").to_string(), "14");
    assert_eq!(global(&mut e, "info").to_string(), "2.0");
    assert_eq!(global(&mut e, "has_off").to_string(), "false");
    // a declared function that doesn't exist is a load error
    std::fs::write(dir.join("extensions/lib/extension.toml"), "[[function]]\nname = \"ghost\"\n").unwrap();
    assert!(Engine::load(&dir).err().unwrap().contains("'ghost'"));
    let _ = std::fs::remove_dir_all(&dir);
}
