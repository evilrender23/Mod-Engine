//! Text input, cursor, navigation grids, raycasts, collision layers, scissor, rounded
//! rectangles and glyph / rich text, run headless on tiny generated projects.

use modengine::draw::DrawCmd;
use modengine::modscript::{intern, Value};
use modengine::Engine;
use std::path::{Path, PathBuf};

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn run(e: &mut Engine, steps: u32) {
    for _ in 0..steps {
        e.tick();
        e.render();
        e.audio.clear();
    }
}

fn g(e: &Engine, name: &str) -> Value {
    e.globals.get(&intern(name)).cloned().unwrap_or_else(|| panic!("global.{name} not set; errors: {:?}", e.errors))
}

fn num(v: Value) -> f64 {
    v.as_num().unwrap_or_else(|| panic!("not a number: {v:?}"))
}

fn arr(v: Value) -> Vec<Value> {
    match v {
        Value::Array(a) => a.borrow().clone(),
        other => panic!("not an array: {other:?}"),
    }
}

/// Values have no `==`: compare their debug text.
fn d<T: std::fmt::Debug>(v: T) -> String {
    format!("{v:?}")
}

fn field(v: &Value, k: &str) -> Value {
    match v {
        Value::Struct(s) => s.borrow().fields.get(&intern(k)).cloned().unwrap_or(Value::Null),
        other => panic!("not a struct: {other:?}"),
    }
}

/// A 16×16 wall, an 8×8 bullet, a 16×16 pit, a sprite font with only "AB" (no space,
/// no lowercase), a tile layer "Muros" of 8-px tiles (columns 8–9, rows 0–3: x 64..80,
/// y 0..32) and the given objects + room code.
fn project(tag: &str, objects: &str, room_code: &str) -> (PathBuf, Engine) {
    let dir = std::env::temp_dir().join(format!("modengine_f1_{}_{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    write(&dir, "game.toml", "[game]\nname = \"f1\"\n[display]\nwidth = 160\nheight = 120\n[palette]\n\"y\" = \"#ffff00\"\n");
    std::fs::create_dir_all(dir.join("sprites")).unwrap();
    image::RgbaImage::from_pixel(16, 16, image::Rgba([90, 90, 120, 255])).save(dir.join("sprites/wall.png")).unwrap();
    image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 255, 0, 255])).save(dir.join("sprites/bullet.png")).unwrap();
    image::RgbaImage::from_pixel(16, 16, image::Rgba([20, 20, 20, 255])).save(dir.join("sprites/pit.png")).unwrap();
    image::RgbaImage::from_pixel(8, 8, image::Rgba([200, 100, 50, 255])).save(dir.join("sprites/sheet.png")).unwrap();
    write(&dir, "sprites/letters.toml", "pixels = ['''\nyy\nyy\n''', '''\ny.\nyy\n''']\n");
    write(&dir, "fonts/caps.toml", "sprite = \"letters\"\nchars = \"AB\"\nspacing = 1\n");
    write(&dir, "objects/o.ms", objects);
    let room = format!(
        "code = '''\n{room_code}\n'''\n\n[[layer]]\nname = \"Muros\"\ntype = \"tiles\"\ntileset = \"sheet\"\ndata = '''\n. . . . . . . . 0 0\n. . . . . . . . 0 0\n. . . . . . . . 0 0\n. . . . . . . . 0 0\n'''\n\n[[layer]]\ntype = \"instances\"\n"
    );
    write(&dir, "rooms/r.toml", &room);
    let mut e = Engine::load(&dir).unwrap_or_else(|err| panic!("{err}"));
    e.start();
    run(&mut e, 1);
    (dir, e)
}

const BASE_OBJECTS: &str = r#"
object Wall { sprite = wall  solid = true }
object Pit { sprite = pit  collision_layer = 2 }
object Bullet { sprite = bullet }
object Probe { sprite = bullet }
"#;

// ------------------------------------------------------------------ text input

#[test]
fn keyboard_string_and_lastchar_from_typed_text() {
    let objects = r#"
object Typer {
    on step {
        global.ks = keyboard_string
        global.ksf = keyboard_string()
        global.lc = keyboard_lastchar
        global.lcf = keyboard_lastchar()
    }
}
"#;
    let (dir, mut e) = project("kb", objects, "spawn(Typer, 0, 0)");
    e.input.typed = "Hola Ñandú, ¿qué?".into();
    run(&mut e, 1);
    assert_eq!(g(&e, "ks").to_string(), "Hola Ñandú, ¿qué?");
    assert_eq!(g(&e, "ksf").to_string(), "Hola Ñandú, ¿qué?");
    assert_eq!(g(&e, "lc").to_string(), "?");
    assert_eq!(g(&e, "lcf").to_string(), "?");
    // backspace deletes one character (a whole ú, not a byte)
    e.input.typed = "ú".into();
    run(&mut e, 1);
    e.input.pressed[modengine::input::BACKSPACE] = true;
    e.input.down[modengine::input::BACKSPACE] = true;
    run(&mut e, 1);
    assert_eq!(g(&e, "ks").to_string(), "Hola Ñandú, ¿qué?");
    // held: repeats after a delay
    for _ in 0..40 {
        e.input.down[modengine::input::BACKSPACE] = true;
        run(&mut e, 1);
    }
    let left = g(&e, "ks").to_string().chars().count();
    assert!((10..17).contains(&left), "auto-repeat deleted a few: {left}");
    e.input.down[modengine::input::BACKSPACE] = false;
    // settable from an instance and from global code, GameMaker style
    e.run_code("with Typer { keyboard_string = \"abc\" }").unwrap();
    run(&mut e, 1);
    assert_eq!(g(&e, "ks").to_string(), "abc");
    e.run_code("keyboard_string = \"\"\nglobal.after = keyboard_string").unwrap();
    assert_eq!(g(&e, "after").to_string(), "");
    assert!(e.errors.is_empty(), "{:?}", e.errors);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cursor_and_cursor_sprite() {
    let (dir, mut e) = project("cur", BASE_OBJECTS, "global.c0 = window_get_cursor()\nwindow_set_cursor(cr_none)\nglobal.c1 = window_get_cursor()");
    assert_eq!(num(g(&e, "c0")), 0.0);
    assert_eq!(num(g(&e, "c1")), -1.0);
    assert!(!e.ui.system_cursor_visible());
    e.run_code("window_set_cursor(cr_cross)\ncursor_sprite = bullet\nglobal.cs = cursor_sprite").unwrap();
    assert!(matches!(g(&e, "cs"), Value::Asset(..)));
    assert!(!e.ui.system_cursor_visible(), "a cursor sprite hides the system cursor");
    e.input.mouse_x = 40.0;
    e.input.mouse_y = 30.0;
    run(&mut e, 1);
    // drawn last, after the GUI, at the mouse
    match e.cmds.last() {
        Some(DrawCmd::Sprite { x, y, .. }) => assert_eq!((*x, *y), (40.0, 30.0)),
        other => panic!("cursor sprite not drawn last: {other:?}"),
    }
    e.run_code("cursor_sprite = null\nwindow_set_cursor(cr_default)").unwrap();
    assert!(e.ui.system_cursor_visible());
    assert!(e.run_code("window_set_cursor(12345)").is_err());
    e.errors.clear();
    let _ = std::fs::remove_dir_all(&dir);
}

// ------------------------------------------------------------------ navigation grids

#[test]
fn mp_grid_paths_distances_and_flow() {
    let code = r#"
// 10×6 cells of 16 px; a wall column at cx = 4 from row 0 to 4 (gap at the bottom row)
var gr = mp_grid_create(0, 0, 10, 6, 16, 16)
global.gr = gr
mp_grid_add_rectangle(gr, 64, 0, 79, 79)
global.blocked = mp_grid_get_cell(gr, 4, 2)
global.free = mp_grid_get_cell(gr, 3, 2)
global.outside = mp_grid_get_cell(gr, -1, 0)
global.at_px = mp_grid_cell_at(gr, 70, 10)
global.pts = mp_grid_path_points(gr, 8, 8, 150, 8, false)
global.nopath = mp_grid_path_points(gr, 8, 8, 70, 10)
var p = path_add()
global.found = mp_grid_path(gr, p, 8, 8, 150, 8, true)
global.plen = path_get_number(p)
global.p_end_x = path_get_point_x(p, path_get_number(p) - 1)
global.dmap = mp_grid_distance_map(gr, 8, 8)
var f = mp_flow_create(gr, 150, 8, false)
global.fdist = mp_flow_distance(f, 8, 8)
global.fdir = mp_flow_dir(f, 8, 8)
global.fdir_at_goal = mp_flow_dir(f, 150, 8)
global.fnext = mp_flow_next(f, 8, 8)
global.fnext_d = mp_flow_distance(f, global.fnext[0], global.fnext[1])
// open the wall at the top and recompute
mp_grid_clear_cell(gr, 4, 0)
mp_flow_update(f)
global.fdist2 = mp_flow_distance(f, 8, 8)
// instances and tiles block cells too
spawn(Wall, 0, 80)
mp_grid_add_instances(gr, Wall)
global.inst_cell = mp_grid_get_cell(gr, 0, 5)
global.inst_next = mp_grid_get_cell(gr, 1, 5)
var g2 = mp_grid_create(0, 0, 10, 2, 16, 16)
global.ntiles = mp_grid_add_tilemap(g2, "Muros")
global.tile_cell = mp_grid_get_cell(g2, 4, 1)
global.rect_free = mp_grid_rect_free(gr, 0, 0, 60, 60)
global.rect_blocked = mp_grid_rect_free(gr, 60, 20, 70, 30)
global.ray = mp_grid_raycast(gr, 8, 40, 150, 40)
mp_grid_destroy(g2)
global.g2_exists = mp_grid_exists(g2)
"#;
    let (dir, e) = project("nav", BASE_OBJECTS, code);
    assert!(e.errors.is_empty(), "{:?}", e.errors);
    assert_eq!((num(g(&e, "blocked")), num(g(&e, "free")), num(g(&e, "outside")), num(g(&e, "at_px"))), (-1.0, 0.0, -1.0, -1.0));
    let pts = arr(g(&e, "pts"));
    // down to the gap (row 5), through it, back up: 9 moves right + 10 vertical = 20 cells
    assert_eq!(pts.len(), 20, "{pts:?}");
    assert_eq!(arr(pts[0].clone()).iter().map(|v| num(v.clone())).collect::<Vec<_>>(), vec![8.0, 8.0]);
    assert_eq!(arr(pts[pts.len() - 1].clone()).iter().map(|v| num(v.clone())).collect::<Vec<_>>(), vec![150.0, 8.0]);
    // every point is on a free cell, and the path goes through the gap
    let ys: Vec<f64> = pts.iter().map(|p| num(arr(p.clone())[1].clone())).collect();
    assert!(ys.iter().any(|y| *y > 80.0), "went through the bottom gap");
    assert!(arr(g(&e, "nopath")).is_empty(), "goal inside the wall: no path");
    assert!(g(&e, "found").truthy());
    assert!(num(g(&e, "plen")) >= 3.0);
    assert_eq!(num(g(&e, "p_end_x")), 150.0);
    let dmap = arr(g(&e, "dmap"));
    assert_eq!(dmap.len(), 6);
    assert_eq!(num(arr(dmap[0].clone())[0].clone()), 0.0);
    assert_eq!(num(arr(dmap[2].clone())[4].clone()), -1.0, "walls are unreachable");
    assert_eq!(num(arr(dmap[0].clone())[5].clone()), 4.0 + 5.0 + 1.0 + 5.0, "around the wall");
    assert_eq!(num(g(&e, "fdist")), 19.0);
    // right and down are equally short: either is fine, and the next cell is one closer
    assert!([0.0, 270.0].contains(&num(g(&e, "fdir"))), "{:?}", g(&e, "fdir"));
    assert_eq!(num(g(&e, "fnext_d")), 18.0);
    assert_eq!(num(g(&e, "fdir_at_goal")), -1.0);
    assert_eq!(num(g(&e, "fdist2")), 9.0, "straight along the top after opening it");
    assert_eq!((num(g(&e, "inst_cell")), num(g(&e, "inst_next"))), (-1.0, 0.0));
    assert_eq!(num(g(&e, "ntiles")), 2.0);
    assert_eq!(num(g(&e, "tile_cell")), -1.0);
    assert!(g(&e, "rect_free").truthy() && !g(&e, "rect_blocked").truthy());
    let ray = g(&e, "ray");
    assert_eq!((num(field(&ray, "x")), num(field(&ray, "nx")), num(field(&ray, "distance"))), (64.0, -1.0, 56.0));
    assert!(!g(&e, "g2_exists").truthy());
    let _ = std::fs::remove_dir_all(&dir);
}

// ------------------------------------------------------------------ rays and line of sight

#[test]
fn raycasts_line_of_sight_and_collision_lists() {
    let code = r#"
var w1 = spawn(Wall, 40, 0)
var w2 = spawn(Wall, 80, 0)
var pit = spawn(Pit, 20, 0)
global.w1 = w1
global.w2 = w2
global.pit = pit
global.hit = raycast(0, 8, 150, 8, Wall)
global.hits_all = raycast_all(0, 8, 150, 8, all, -1, false)
global.list = collision_line_list(0, 8, 150, 8, Wall)
global.cl = collision_line(150, 8, 0, 8, Wall)
global.los = line_of_sight(0, 40, 150, 40, Wall)
global.no_los = line_of_sight(0, 8, 150, 8, Wall)
global.up = raycast(48, 60, 48, 0, Wall)
global.dir = raycast_dir(0, 8, 0, 150, Wall)
global.miss = raycast(0, 100, 150, 100, Wall)
// tile layer "Muros": columns 8–9 (x 64..80)
global.tile = raycast(0, 4, 159, 4, "Muros")
// walls stop the ray: instances behind the tiles are dropped
global.mixed = raycast_all(0, 8, 159, 8, [Wall, "Muros"])
global.pit_mask = raycast(0, 8, 150, 8, all, 1)
global.circle = collision_circle(48, 8, 4, Wall, false, true)
global.rect = collision_rectangle_list(0, 0, 100, 4, Wall, false, true)
global.point = collision_point(85, 5, Wall, false, true)
"#;
    let (dir, e) = project("ray", BASE_OBJECTS, code);
    assert!(e.errors.is_empty(), "{:?}", e.errors);
    let hit = g(&e, "hit");
    assert_eq!(d(field(&hit, "inst")), d(g(&e, "w1")));
    assert_eq!((num(field(&hit, "x")), num(field(&hit, "y")), num(field(&hit, "distance"))), (40.0, 8.0, 40.0));
    assert_eq!((num(field(&hit, "nx")), num(field(&hit, "ny"))), (-1.0, 0.0));
    let all: Vec<Value> = arr(g(&e, "hits_all")).iter().map(|h| field(h, "inst")).collect();
    assert_eq!(d(all), d(vec![g(&e, "pit"), g(&e, "w1"), g(&e, "w2")]), "every hit, nearest first");
    assert_eq!(d(arr(g(&e, "list"))), d(vec![g(&e, "w1"), g(&e, "w2")]));
    assert_eq!(d(g(&e, "cl")), d(g(&e, "w2")), "collision_line returns the nearest to the start");
    assert!(g(&e, "los").truthy() && !g(&e, "no_los").truthy());
    let up = g(&e, "up");
    assert_eq!((num(field(&up, "y")), num(field(&up, "ny"))), (16.0, 1.0));
    assert_eq!(num(field(&g(&e, "dir"), "distance")), 40.0);
    assert!(g(&e, "miss").is_null());
    let tile = g(&e, "tile");
    assert_eq!((num(field(&tile, "x")), num(field(&tile, "cell_x")), field(&tile, "source").to_string()), (64.0, 8.0, "Muros".to_string()));
    let mixed: Vec<f64> = arr(g(&e, "mixed")).iter().map(|h| num(field(h, "distance"))).collect();
    assert_eq!(mixed, vec![40.0, 64.0], "w2 at 80 is behind the tiles");
    // with mask 1 the pit (layer 2) is invisible to the ray
    assert_eq!(d(field(&g(&e, "pit_mask"), "inst")), d(g(&e, "w1")));
    assert_eq!(d(g(&e, "circle")), d(g(&e, "w1")));
    assert_eq!(arr(g(&e, "rect")).len(), 2);
    assert_eq!(d(g(&e, "point")), d(g(&e, "w2")));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn collision_layers_let_bullets_cross_pits() {
    let objects = format!(
        "{BASE_OBJECTS}\nobject Shot {{\n    sprite = bullet\n    collision_mask = 1\n    on collision(Pit) {{ global.shot_pit = true }}\n}}\nobject Walker {{\n    sprite = bullet\n    on collision(Pit) {{ global.walker_pit = true }}\n}}\nobject Crate {{\n    sprite = wall\n    solid = true\n    collision_layer = 4\n}}\n"
    );
    let code = r#"
global.shot_pit = false
global.walker_pit = false
spawn(Pit, 0, 0)
spawn(Wall, 40, 0)
spawn(Crate, 80, 0)
var s = spawn(Shot, 4, 4)
var w = spawn(Walker, 4, 4)
global.s = s
global.w = w
with s {
    global.s_pit = place_meeting(4, 4, Pit)
    global.s_cpit = collides_at(4, 4, Pit)
    global.s_wall = place_meeting(40, 4, Wall)
    global.s_free = place_free(40, 4)
    global.s_crate_free = place_free(84, 4)
    global.s_layer = collision_layer
    global.s_mask = collision_mask
    global.s_list = instance_place_list(4, 4, all)
}
with w {
    global.w_pit = place_meeting(4, 4, Pit)
    global.w_list = instance_place_list(4, 4, all)
    global.w_mask = collision_mask
    global.w_crate_free = place_free(84, 4)
}
"#;
    let (dir, mut e) = project("layers", &objects, code);
    assert!(e.errors.is_empty(), "{:?}", e.errors);
    assert!(!g(&e, "s_pit").truthy() && !g(&e, "s_cpit").truthy(), "the shot's mask ignores layer 2");
    assert!(g(&e, "s_wall").truthy() && !g(&e, "s_free").truthy());
    assert!(g(&e, "s_crate_free").truthy(), "solids on other layers don't block it");
    assert!(!g(&e, "w_crate_free").truthy());
    assert_eq!((num(g(&e, "s_layer")), num(g(&e, "s_mask"))), (1.0, 1.0));
    assert_eq!(d(arr(g(&e, "s_list"))), d(vec![g(&e, "w")]), "only layer-1 things");
    assert!(g(&e, "w_pit").truthy());
    assert_eq!(arr(g(&e, "w_list")).len(), 2, "the pit and the shot");
    assert_eq!(num(g(&e, "w_mask")), u32::MAX as f64);
    run(&mut e, 2);
    assert!(g(&e, "walker_pit").truthy(), "collision event with the pit");
    assert!(!g(&e, "shot_pit").truthy(), "no collision event across layers");
    // -1 = every layer; negative other values are rejected
    e.run_code("with global.s { collision_mask = -1 }").unwrap();
    run(&mut e, 1);
    assert!(g(&e, "shot_pit").truthy());
    assert!(e.run_code("with global.s { collision_mask = \"x\" }").is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

// ------------------------------------------------------------------ drawing

fn gui_cmds(e: &Engine) -> Vec<DrawCmd> {
    let k = e.cmds.iter().position(|c| matches!(c, DrawCmd::Gui)).expect("gui");
    e.cmds[k + 1..].to_vec()
}

#[test]
fn scissor_stack_and_rounded_rects() {
    let objects = r#"
object Ui {
    on draw_gui {
        gpu_push_scissor(10, 10, 100, 50)
        gpu_push_scissor(50, 0, 100, 30)
        global.sc = gpu_get_scissor()
        gpu_pop_scissor()
        global.sc2 = gpu_get_scissor()
        gpu_reset_scissor()
        global.sc3 = gpu_get_scissor()
        gpu_set_scissor({ x: 1, y: 2, w: 3, h: 4 })
        draw_roundrect_ext(20, 20, 60, 40, 8, 8, false)
        draw_roundrect(70, 20, 110, 40, true, 6)
        draw_roundrect_colour(0, 0, 10, 10, #ff0000, #00ff00, false)
    }
}
"#;
    let (dir, mut e) = project("draw", objects, "spawn(Ui, 0, 0)");
    run(&mut e, 1);
    assert!(e.errors.is_empty(), "{:?}", e.errors);
    let sc = g(&e, "sc");
    assert_eq!([field(&sc, "x"), field(&sc, "y"), field(&sc, "w"), field(&sc, "h")].map(num), [50.0, 10.0, 60.0, 20.0], "nested = intersection");
    assert_eq!(num(field(&g(&e, "sc2"), "w")), 100.0);
    assert!(g(&e, "sc3").is_null());
    let cmds = gui_cmds(&e);
    let clips: Vec<Option<[f32; 4]>> = cmds.iter().filter_map(|c| if let DrawCmd::Scissor(r) = c { Some(*r) } else { None }).collect();
    assert_eq!(clips, vec![Some([10.0, 10.0, 100.0, 50.0]), Some([50.0, 10.0, 60.0, 20.0]), Some([10.0, 10.0, 100.0, 50.0]), None, Some([1.0, 2.0, 3.0, 4.0])]);
    // filled: a fan of triangles inside the rectangle, corners cut
    let tris: Vec<[f32; 6]> = cmds.iter().filter_map(|c| if let DrawCmd::Tri { pts, .. } = c { Some(*pts) } else { None }).collect();
    assert!(tris.len() >= 16, "{}", tris.len());
    let first_fill: Vec<&[f32; 6]> = tris.iter().filter(|p| p[0] == 40.0 && p[1] == 30.0).collect();
    assert!(first_fill.iter().all(|p| p.chunks(2).all(|q| (20.0..=60.0).contains(&q[0]) && (20.0..=40.0).contains(&q[1]))));
    assert!(!first_fill.iter().any(|p| p.chunks(2).any(|q| q[0] == 20.0 && q[1] == 20.0)), "the corner is rounded off");
    // outline: line segments, inset half a pixel
    let lines = cmds.iter().filter(|c| matches!(c, DrawCmd::Line { x1, .. } if *x1 >= 70.0)).count();
    assert!(lines >= 8, "{lines}");
    // the frame starts unclipped: render resets the stack
    run(&mut e, 1);
    let k = e.cmds.iter().position(|c| matches!(c, DrawCmd::Gui)).unwrap();
    assert!(!e.cmds[..k].iter().any(|c| matches!(c, DrawCmd::Scissor(_))));
    let _ = std::fs::remove_dir_all(&dir);
}

fn texts(cmds: &[DrawCmd]) -> Vec<(String, f32, f32, f32, (f32, f32, f32, f32))> {
    cmds.iter()
        .filter_map(|c| match c {
            DrawCmd::Text { x, y, text, scale, color, .. } => Some((text.clone(), *x, *y, *scale, (color.r, color.g, color.b, color.a))),
            _ => None,
        })
        .collect()
}

#[test]
fn glyph_callbacks_and_rich_text() {
    let objects = r#"
object Ui {
    on draw_gui {
        var amp = 3
        global.size = draw_text_glyphs(10, 10, "ab c", function(i, ch, x, y) {
            if ch == "c" return { visible: false }
            return { dy: i * 2, color: #ff0000, alpha: 0.5, scale: 2 }
        })
        draw_text_rich(10, 50, "[c=#00ff00]G[/c]x[b]B[/b][wave=0]w[/wave][[")
        global.rich = draw_text_rich(0, 80, "uno dos tres", 30)
        global.plain = text_rich_strip("[c=red]hola[/c] [shake]mundo[/] [[x]")
        global.rsize = text_rich_size("[s=2]A[/s]B")
        set_font(caps)
        global.sw = string_width("AB AB")
        global.sw_lower = string_width("ab")
        global.lines = text_wrap("AB AB AB", 12)
        draw_text_wrap(0, 100, "AB AB", -1, 12)
    }
}
"#;
    let (dir, mut e) = project("text", objects, "spawn(Ui, 0, 0)");
    run(&mut e, 1);
    assert!(e.errors.is_empty(), "{:?}", e.errors);
    let t = texts(&gui_cmds(&e));
    // glyphs: a, b and the space are drawn (space skipped as it is invisible), c hidden by the callback
    let a = t.iter().find(|x| x.0 == "a").unwrap();
    let b = t.iter().find(|x| x.0 == "b").unwrap();
    assert!(!t.iter().any(|x| x.0 == "c"));
    assert_eq!((a.3, a.4), (2.0, (1.0, 0.0, 0.0, 0.5)));
    assert!(b.1 > a.1, "pen advances");
    assert_eq!(b.2 - a.2, 2.0, "dy = i * 2");
    assert_eq!(num(field(&g(&e, "size"), "lines")), 1.0);
    // rich: colour of G, bold B drawn twice, `[[` is a bracket
    let gg = t.iter().find(|x| x.0 == "G").unwrap();
    assert_eq!(gg.4, (0.0, 1.0, 0.0, 1.0));
    assert_eq!(t.iter().filter(|x| x.0 == "B").count(), 2);
    assert!(t.iter().any(|x| x.0 == "["));
    // wrapped at 30 px with the built-in font (8 px per char headless): one word per line
    assert_eq!(num(field(&g(&e, "rich"), "lines")), 3.0);
    let ys: Vec<f32> = t.iter().filter(|x| x.2 >= 80.0 && x.2 < 100.0).map(|x| x.2).collect();
    assert!(ys.windows(2).all(|w| w[0] <= w[1]) && ys.first() != ys.last());
    assert_eq!(g(&e, "plain").to_string(), "hola mundo [x]");
    assert_eq!(num(field(&g(&e, "rsize"), "w")), 8.0 * 2.0 + 8.0, "scaled characters count double");
    // sprite font without a space or lowercase: spaces get a width, lowercase uses the capitals
    // "A" = 2 + 1, "B" = 2 + 1 (proportional), spacing 1, space = advance of "0"/"n" or half a line
    let sw = num(g(&e, "sw"));
    assert!(sw > 3.0 + 1.0 + 3.0 + 1.0 + 3.0 + 1.0 + 3.0, "space counted: {sw}");
    assert_eq!(num(g(&e, "sw_lower")), 7.0);
    assert_eq!(arr(g(&e, "lines")).len(), 3);
    let wrapped = t.iter().filter(|x| x.2 >= 100.0 && x.0.starts_with("AB")).count();
    assert_eq!(wrapped, 2, "draw_text_wrap with a sprite font: two lines");
    let _ = std::fs::remove_dir_all(&dir);
}
