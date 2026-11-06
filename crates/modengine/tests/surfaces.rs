//! Surfaces, application_surface and the pre_draw / post_draw passes, run headless: the
//! engine records the commands a backend replays (no GPU needed, like `modengine check`).

use modengine::draw::DrawCmd;
use modengine::Engine;
use std::path::PathBuf;

fn project(name: &str, obj: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("me_surfaces_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let files: [(&str, &str); 4] = [
        ("game.toml", "[game]\nname = \"t\"\n[display]\nwidth = 320\nheight = 180\n"),
        ("rooms/r.toml", "[[layer]]\ntype = \"instances\"\n[[layer.instances]]\nobject = \"T\"\nx = 0\ny = 0\n"),
        ("shaders/crt.fsh", "varying vec2 v_vTexcoord;\nvarying vec4 v_vColour;\nvoid main() { gl_FragColor = v_vColour * texture2D(gm_BaseTexture, v_vTexcoord); }\n"),
        ("objects/t.ms", obj),
    ];
    for (rel, text) in files {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    dir
}

fn load(name: &str, obj: &str) -> Engine {
    let mut e = Engine::load(&project(name, obj)).unwrap_or_else(|m| panic!("failed to load:\n{m}"));
    e.start();
    e
}

fn frame(e: &mut Engine) {
    e.tick();
    e.render();
    e.audio.clear();
}

fn global(e: &Engine, name: &str) -> String {
    e.globals.get(&modengine::modscript::intern(name)).cloned().unwrap_or_default().to_string()
}

#[test]
fn surface_lifecycle_and_queries() {
    let mut e = load(
        "life",
        r#"
object T {
    on create {
        global.s = surface_create(64, 32)
        global.ex = surface_exists(global.s)
        global.w = surface_get_width(global.s)
        global.h = surface_get_height(global.s)
        global.aw = surface_get_width(application_surface)
        global.app = surface_exists(application_surface)
        global.nada = surface_exists(-1)
        surface_resize(global.s, 10, 20)
        global.w2 = surface_get_width(global.s)
        var t = surface_create(8, 8)
        surface_free(t)
        global.freed = surface_exists(t)
        global.ids_differ = surface_create(8, 8) != t
        global.drawon = application_surface_is_draw_enabled()
    }
}
"#,
    );
    frame(&mut e);
    assert!(e.errors.is_empty(), "{:?}", e.errors);
    for (k, v) in [("ex", "true"), ("w", "64"), ("h", "32"), ("aw", "320"), ("app", "true"), ("nada", "false"), ("w2", "10"), ("freed", "false"), ("ids_differ", "true"), ("drawon", "true")] {
        assert_eq!(global(&e, k), v, "{k}");
    }
}

#[test]
fn drawing_into_a_surface_records_target_blocks() {
    let mut e = load(
        "draw",
        r#"
object T {
    on create { s = surface_create(40, 30) }
    on draw {
        surface_set_target(s)
        draw_clear_alpha(#000000, 0)
        draw_rect(0, 0, 10, 10)
        global.tgt = surface_get_target() == s
        surface_reset_target()
        global.tgt2 = surface_get_target()
        draw_surface_ext(s, 5, 6, 2, -1, 90, #ff0000, 0.5)
        draw_surface_part(s, 1, 2, 3, 4, 7, 8)
        draw_surface_stretched(s, 0, 0, 100, 50)
    }
}
"#,
    );
    frame(&mut e);
    assert!(e.errors.is_empty(), "{:?}", e.errors);
    assert_eq!(global(&e, "tgt"), "true");
    assert_eq!(global(&e, "tgt2"), "-1");
    let c = &e.cmds;
    let open = c.iter().position(|c| matches!(c, DrawCmd::Target(Some((_, 40, 30))))).expect("set target");
    let close = c.iter().position(|c| matches!(c, DrawCmd::Target(None))).expect("reset target");
    assert!(open < close);
    assert!(matches!(c[open + 1], DrawCmd::Clear(col) if col.a == 0.0), "draw_clear_alpha inside the block");
    assert!(matches!(c[open + 2], DrawCmd::Rect { .. }));
    let surf: Vec<_> = c.iter().filter_map(|c| if let DrawCmd::Surface { src, x, y, w, h, angle, color, .. } = c { Some((*src, *x, *y, *w, *h, *angle, color.r, color.a)) } else { None }).collect();
    assert_eq!(surf.len(), 3);
    assert_eq!(surf[0], (None, 5.0, 6.0, 80.0, -30.0, 90.0, 1.0, 0.5), "draw_surface_ext scales the surface size");
    assert_eq!(surf[1].0, Some([1.0, 2.0, 3.0, 4.0]));
    assert_eq!((surf[1].3, surf[1].4), (3.0, 4.0));
    assert_eq!((surf[2].3, surf[2].4), (100.0, 50.0));
    // no post_draw pass unless asked for
    assert!(!c.iter().any(|c| matches!(c, DrawCmd::PostDraw { .. })));
}

#[test]
fn nested_targets_go_back_to_the_outer_surface() {
    let mut e = load(
        "nest",
        r#"
object T {
    on create { a = surface_create(16, 16); b = surface_create(8, 8) }
    on draw_gui {
        surface_set_target(a)
        surface_set_target(b)
        surface_reset_target()
        surface_reset_target()
    }
}
"#,
    );
    frame(&mut e);
    assert!(e.errors.is_empty(), "{:?}", e.errors);
    let t: Vec<_> = e.cmds.iter().filter_map(|c| if let DrawCmd::Target(t) = c { Some(t.map(|x| (x.1, x.2))) } else { None }).collect();
    assert_eq!(t, vec![Some((16, 16)), Some((8, 8)), Some((16, 16)), None]);
}

#[test]
fn surface_errors_are_reported() {
    let mut e = load(
        "err",
        r#"
object T {
    step = 0
    on draw {
        step += 1
        if step == 1 draw_surface(12345, 0, 0)
        if step == 2 surface_reset_target()
        if step == 3 surface_set_target(surface_create(4, 4))
        if step == 4 surface_create(0, 10)
    }
}
"#,
    );
    let mut seen = vec![];
    let mut last_target = vec![];
    for _ in 0..4 {
        e.errors.clear();
        frame(&mut e);
        seen.push(e.errors.join(" | "));
        last_target.push(e.cmds.iter().rev().find(|c| matches!(c, DrawCmd::Target(_))).cloned());
    }
    assert!(seen[0].contains("12345") && seen[0].contains("no existe"), "{}", seen[0]);
    assert!(seen[1].contains("sin surface_set_target"), "{}", seen[1]);
    // a target left open is closed at the end of the frame and reported
    assert!(seen[2].contains("sin surface_reset_target"), "{}", seen[2]);
    assert!(matches!(last_target[2], Some(DrawCmd::Target(None))), "{:?}", last_target[2]);
    assert!(seen[3].contains("no válido"), "{}", seen[3]);
}

#[test]
fn surfaces_drawn_outside_draw_events_reach_the_next_frame() {
    let mut e = load(
        "outside",
        r#"
object T {
    hecho = false
    on create {
        global.s = surface_create(32, 32)
        surface_set_target(global.s)
        shader_set(crt)
        draw_clear(#ff0000)
        surface_reset_target()
        draw_rect(0, 0, 1, 1)   // outside a target: dropped, like before
    }
    on step {
        if !hecho {
            hecho = true
            surface_set_target(global.s)
            draw_circle(4, 4, 2)
            surface_reset_target()
        }
    }
}
"#,
    );
    frame(&mut e);
    assert!(e.errors.is_empty(), "{:?}", e.errors);
    let c = &e.cmds;
    // create's block and step's block come first, before the frame's View
    let view = c.iter().position(|c| matches!(c, DrawCmd::View { .. })).unwrap();
    let pre = &c[..view];
    assert_eq!(pre.iter().filter(|c| matches!(c, DrawCmd::Target(Some(_)))).count(), 2, "{pre:?}");
    assert!(pre.iter().any(|c| matches!(c, DrawCmd::Clear(_))));
    assert!(pre.iter().any(|c| matches!(c, DrawCmd::Circle { .. })));
    assert!(!pre.iter().any(|c| matches!(c, DrawCmd::Rect { .. })), "draws outside a surface are not replayed");
    // the shader set inside the block does not leak into the frame
    assert!(matches!(pre[pre.len() - 2], DrawCmd::Shader(None)));
    // replayed once only
    frame(&mut e);
    let view = e.cmds.iter().position(|c| matches!(c, DrawCmd::View { .. })).unwrap();
    assert_eq!(view, 0, "{:?}", &e.cmds[..view]);
}

#[test]
fn pre_and_post_draw_passes() {
    let mut e = load(
        "post",
        r#"
object T {
    on pre_draw { draw_rect(0, 0, 2, 2) }
    on draw_gui { draw_circle(1, 1, 1) }
    on post_draw {
        shader_set(crt)
        draw_surface(application_surface, 0, 0)
        shader_reset()
    }
}
"#,
    );
    frame(&mut e);
    assert!(e.errors.is_empty(), "{:?}", e.errors);
    let c = &e.cmds;
    let kind = |c: &DrawCmd| match c {
        DrawCmd::View { .. } => "view",
        DrawCmd::Gui => "gui",
        DrawCmd::Clear(_) => "clear",
        DrawCmd::Rect { .. } => "rect",
        DrawCmd::Circle { .. } => "circle",
        DrawCmd::PostDraw { app: true } => "post",
        DrawCmd::Surface { id: 0, .. } => "app",
        DrawCmd::Shader(Some(_)) => "shader",
        DrawCmd::Shader(None) => "reset",
        _ => "?",
    };
    let seq: Vec<&str> = c.iter().map(kind).filter(|k| *k != "?").collect();
    assert_eq!(seq, ["view", "clear", "gui", "rect", "view", "gui", "circle", "post", "shader", "app", "reset"]);

    // draw disabled: post_draw is the only image, even without a post_draw event
    let mut e = load("nodraw", "object T {\n    on create { application_surface_draw_enable(false) }\n}\n");
    frame(&mut e);
    assert!(e.cmds.iter().any(|c| matches!(c, DrawCmd::PostDraw { app: false })));
    assert!(matches!(e.cmds.last(), Some(DrawCmd::PostDraw { app: false })), "nothing drawn after it");
}
