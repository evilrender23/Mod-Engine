//! Fonts, tile sets and tile maps without a window: the showcase example and a tiny project
//! that calls every tilemap/font function from room code.

use modengine::draw::DrawCmd;
use modengine::modscript::{intern, Value};
use modengine::Engine;
use std::path::{Path, PathBuf};

fn showcase() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/showcase_assets_s1")
}

fn run(e: &mut Engine, steps: u32) {
    for _ in 0..steps {
        e.tick();
        e.render();
        e.audio.clear();
    }
}

fn global(e: &Engine, name: &str) -> Value {
    e.globals.get(&intern(name)).cloned().unwrap_or_else(|| panic!("global.{name} not set"))
}

/// Sprite commands drawn with the sprite of tile set `ts`.
fn tile_cmds(e: &Engine, ts: usize) -> Vec<(u32, f32, f32, f32, f32, f32)> {
    let spr = e.tilesets[ts].sprite_index;
    e.cmds
        .iter()
        .filter_map(|c| match c {
            DrawCmd::Sprite { spr: s, frame, x, y, sx, sy, angle, .. } if *s == spr => Some((*frame, *x, *y, *sx, *sy, *angle)),
            _ => None,
        })
        .collect()
}

#[test]
fn showcase_runs_and_draws_tiles_fonts() {
    let mut e = Engine::load(&showcase()).unwrap_or_else(|err| panic!("showcase failed to load:\n{err}"));
    e.start();
    run(&mut e, 2);
    let ts = &e.tilesets[0];
    assert_eq!((ts.name.as_str(), ts.count(), ts.columns), ("ts_world", 24, 8));
    let cmds = tile_cmds(&e, 0);
    // mirrored arrow (column 17, row 7): drawn flipped at the right edge of its cell
    assert!(cmds.iter().any(|c| c.0 == 22 && c.3 == -1.0 && c.1 == 17.0 * 16.0 + 16.0 && c.2 == 7.0 * 16.0), "{cmds:?}");
    // rotated arrow (column 18, row 6)
    assert!(cmds.iter().any(|c| c.0 == 22 && c.5 == -90.0), "rotated tile");
    // water is animated: frame 16 now, another frame a moment later
    let water = |e: &Engine| tile_cmds(e, 0).into_iter().filter(|c| (16..20).contains(&c.0) && c.2 == 9.0 * 16.0 && c.1 == 8.0 * 16.0).map(|c| c.0).next().unwrap();
    assert_eq!(water(&e), 16);
    run(&mut e, 15);
    assert_ne!(water(&e), 16, "animated tile advanced");
    // the GUI text uses the baked fonts
    let texts: Vec<Option<u32>> = e.cmds.iter().filter_map(|c| if let DrawCmd::Text { font, .. } = c { Some(*font) } else { None }).collect();
    assert!(texts.len() >= 3 && texts.iter().all(|f| f.is_some()), "{texts:?}");
    run(&mut e, 300);
    assert!(e.errors.is_empty(), "runtime errors: {:?}", e.errors);
}

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// A project with a 4×2 sheet of 8-px tiles, a tile set with an animation and an auto tile,
/// a sprite font and a room whose code calls the functions.
fn tiny(code: &str) -> (PathBuf, Engine) {
    let dir = std::env::temp_dir().join(format!("modengine_s1_{}_{}", std::process::id(), code.len()));
    let _ = std::fs::remove_dir_all(&dir);
    write(&dir, "game.toml", "[game]\nname = \"t\"\n[display]\nwidth = 64\nheight = 32\n[palette]\n\"y\" = \"#ffff00\"\n");
    std::fs::create_dir_all(dir.join("sprites")).unwrap();
    image::RgbaImage::from_pixel(32, 16, image::Rgba([200, 100, 50, 255])).save(dir.join("sprites/sheet.png")).unwrap();
    write(&dir, "sprites/digits.toml", "pixels = ['''\nyy\nyy\n''', '''\n.y\n.y\n''']\n");
    write(&dir, "fonts/fnt.toml", "sprite = \"digits\"\nchars = \"01\"\nspacing = 1\n");
    write(&dir, "tilesets/ts.toml", "sprite = \"sheet\"\ntile_width = 8\ntile_height = 8\nfps = 10\n[[animation]]\nname = \"a\"\nframes = [5, 6, 7]\n[[autotile]]\nname = \"g\"\ntiles = [0, 1, 2, 3, 4, 5, 6, 7, 0, 1, 2, 3, 4, 5, 6, 7]\n");
    write(&dir, "objects/o.ms", "object Probe {\n    sprite = digits\n}\n");
    // room code runs once its layers and instances exist
    let room = format!("code = '''\n{code}\n'''\n\n[[layer]]\nname = \"T\"\ntype = \"tiles\"\ntileset = \"ts\"\ndata = '''\n0 1 . 3\n.\n4\n'''\n\n[[layer]]\ntype = \"instances\"\ninstances = [{{ object = \"Probe\", x = 1, y = 1 }}]\n");
    write(&dir, "rooms/r.toml", &room);
    let mut e = Engine::load(&dir).unwrap_or_else(|err| panic!("{err}"));
    e.start();
    run(&mut e, 1);
    assert!(e.errors.is_empty(), "runtime errors: {:?}", e.errors);
    (dir, e)
}

fn num(v: Value) -> f64 {
    v.as_num().unwrap_or_else(|| panic!("not a number: {v:?}"))
}

#[test]
fn tilemap_functions() {
    let (dir, e) = tiny(
        r#"
var tm = layer_tilemap_get_id("T")
global.id = tm
global.exists = layer_tilemap_exists("T")
global.a = tilemap_get(tm, 1, 0)
global.empty = tilemap_get("T", 2, 0)
global.px = tilemap_get_at_pixel(tm, 25, 3)
global.cx = tilemap_get_cell_x_at_pixel(tm, 25, 3)
global.w = tilemap_get_width(tm)
global.h = tilemap_get_height(tm)
global.tw = tilemap_get_tile_width(tm)
tilemap_set(tm, tile_set_rotate(tile_set_mirror(2)), 5, 1)
global.d = tilemap_get(tm, 5, 1)
global.idx = tile_get_index(global.d)
global.mir = tile_get_mirror(global.d)
global.flip = tile_get_flip(global.d)
global.rot = tile_get_rotate(global.d)
global.unmir = tile_get_mirror(tile_set_mirror(global.d, false))
global.isempty = tile_get_empty(tilemap_get(tm, 9, 9))
tilemap_set_at_pixel(tm, 7, 1, 17)
global.set_px = tilemap_get(tm, 0, 2)
tilemap_autotile(tm, "g", 0, 3, true)
tilemap_autotile(tm, "g", 1, 3, true)
global.auto_l = tilemap_get(tm, 0, 3)
global.auto_r = tilemap_get(tm, 1, 3)
global.ts = tilemap_get_tileset(tm)
global.info = tileset_get_info(ts)
tilemap_x(tm, 4)
global.x = tilemap_get_x(tm)
global.after_move = tilemap_get_at_pixel(tm, 13, 3)
var nt = layer_tilemap_create("Nueva", 0, 0, ts, 3, 2)
global.new_w = tilemap_get_width(nt)
global.new_t = tilemap_get(nt, 2, 1)
with Probe {
    global.meet = tile_meeting(5, 1, "T")
    global.no_meet = tile_meeting(5, 9, "T")
}
global.sw = string_width("0101")
global.sh = string_height("0")
draw_set_font(fnt)
global.sw2 = string_width("0101")
global.sh2 = string_height("0\n1")
global.wrap = text_wrap("01 01 01", 9)
global.fsize = font_get_size(fnt)
global.has1 = font_has_char(fnt, "1")
global.has2 = font_has_char(fnt, "2")
global.first = font_get_first(fnt)
"#,
    );
    let g = |n: &str| global(&e, n);
    assert_eq!(num(g("id")), 0.0);
    assert!(g("exists").truthy());
    assert_eq!(num(g("a")), 1.0);
    assert_eq!(num(g("empty")), -1.0);
    assert_eq!((num(g("px")), num(g("cx"))), (3.0, 3.0));
    assert_eq!((num(g("w")), num(g("h")), num(g("tw"))), (4.0, 3.0, 8.0));
    assert_eq!(num(g("d")), (2 | 1 << 28 | 1 << 30) as f64);
    assert_eq!(num(g("idx")), 2.0);
    assert!(g("mir").truthy() && !g("flip").truthy() && g("rot").truthy() && !g("unmir").truthy());
    assert!(g("isempty").truthy());
    assert_eq!(num(g("set_px")), 7.0);
    // two auto tile cells side by side under tile 4 (also one of the auto tile's tiles): the
    // left one connects up and right (1 + 2 = 3), the right one left (8 → its tile is 0)
    assert_eq!((num(g("auto_l")), num(g("auto_r"))), (3.0, 0.0));
    let Value::Struct(info) = g("info") else { panic!() };
    let info = info.borrow();
    assert_eq!(info.fields.get(&intern("tile_count")).and_then(|v| v.as_num()), Some(8.0));
    assert_eq!(info.fields.get(&intern("columns")).and_then(|v| v.as_num()), Some(4.0));
    drop(info);
    assert_eq!(num(g("x")), 4.0);
    assert_eq!(num(g("after_move")), 1.0, "pixel 13 is now in cell 1");
    assert_eq!((num(g("new_w")), num(g("new_t"))), (3.0, -1.0));
    assert!(g("meet").truthy() && !g("no_meet").truthy());
    // headless built-in font: 8 px per char; the sprite font: 2 + 1 + 1(+1 spacing)...
    assert_eq!(num(g("sw")), 32.0);
    assert_eq!(num(g("sh")), 16.0);
    // "0" is 2 px + 1, "1" is 1 px + 1 (proportional), plus 1 px of spacing between characters
    assert_eq!(num(g("sw2")), 3.0 + 2.0 + 3.0 + 2.0 + 3.0);
    assert_eq!(num(g("sh2")), 4.0);
    let Value::Array(w) = g("wrap") else { panic!() };
    assert_eq!(w.borrow().len(), 3, "each '01' is 6 px wide: one per line");
    assert_eq!(num(g("fsize")), 2.0);
    assert!(g("has1").truthy() && !g("has2").truthy());
    assert_eq!(num(g("first")), 32.0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn older_rooms_with_sprite_tilesets_still_load() {
    // the platformer has no tile sets: it must load and draw as before
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/platformer");
    let mut e = Engine::load(&dir).unwrap();
    assert!(e.tilesets.is_empty());
    e.start();
    run(&mut e, 30);
    assert!(e.errors.is_empty(), "{:?}", e.errors);
}
