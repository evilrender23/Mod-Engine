//! Art & world features without a window: scaled tile layers (drawing, collisions,
//! tilemap_*), compact `pixels_rle` sprites, rotation settings (game.toml default, sprite
//! TOML, sprite_set_rotation).

use modengine::draw::DrawCmd;
use modengine::modscript::{intern, Value};
use modengine::rotsprite::RotMode;
use modengine::Engine;
use std::path::{Path, PathBuf};

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("modengine_art_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn global(e: &Engine, name: &str) -> Value {
    e.globals.get(&intern(name)).cloned().unwrap_or_else(|| panic!("global.{name} not set"))
}

fn num(e: &Engine, name: &str) -> f64 {
    match global(e, name) {
        Value::Num(n) => n,
        Value::Bool(b) => b as i32 as f64,
        other => panic!("global.{name} = {other}"),
    }
}

fn project(dir: &Path, display_extra: &str) {
    write(dir, "game.toml", &format!("[game]\nname = \"t\"\nstart_room = \"r\"\n[display]\nwidth = 320\nheight = 180\n{display_extra}\n[palette]\nr = \"#ff0000\"\ng = \"#00ff00\"\n"));
    // 4 tiles of 16×16 in a 64×16 strip
    let row: String = "r".repeat(64);
    let rows = vec![row; 16].join("\n");
    write(dir, "sprites/tiles.toml", &format!("pixels = ['''\n{rows}\n''']\n"));
    write(dir, "tilesets/ts.toml", "sprite = \"tiles\"\ntile_width = 16\ntile_height = 16\n");
}

#[test]
fn scaled_tile_layers_draw_and_collide_in_scaled_pixels() {
    let dir = temp("tiles");
    project(&dir, "");
    write(&dir, "objects/o.ms", "object Probe { sprite = probe }\n");
    write(&dir, "sprites/probe.toml", "pixels = ['''\ngggg\ngggg\n''']\n");
    write(
        &dir,
        "rooms/r.toml",
        r#"code = """
var tm = layer_tilemap_get_id("T")
global.tw = tilemap_get_tile_width(tm)
global.sx = tilemap_get_scale_x(tm)
global.at_100 = tilemap_get_at_pixel(tm, 100, 10)
global.cell_x = tilemap_get_cell_x_at_pixel(tm, 100, 10)
global.old = tile_at(60, 10)
var p = spawn(Probe, 0, 0)
global.hit = false
with p { global.hit = tile_meeting(150, 4, "T"); global.miss = tile_meeting(150, 60, "T") }
var nt = layer_tilemap_create("N", 0, 0, ts, 4, 4)
tilemap_set_scale(nt, 2, 4)
global.nw = tilemap_get_tile_width(nt)
global.nh = tilemap_get_tile_height(nt)
tilemap_set(nt, 1, 1, 1)
global.n_at = tilemap_get_at_pixel(nt, 40, 70)
global.bad = false
"""

[[layer]]
name = "T"
type = "tiles"
tileset = "ts"
scale = 3
data = '''
0 1 2 3 0 1 2 3
'''
"#,
    );
    let mut e = Engine::load(&dir).unwrap_or_else(|err| panic!("{err}"));
    e.start();
    e.tick();
    e.render();
    assert!(e.errors.is_empty(), "{:?}", e.errors);
    assert_eq!(num(&e, "tw"), 48.0, "cells are 16 × 3");
    assert_eq!(num(&e, "sx"), 3.0);
    assert_eq!(num(&e, "at_100"), 2.0, "x = 100 is in the third 48 px cell");
    assert_eq!(num(&e, "cell_x"), 2.0);
    assert_eq!(num(&e, "old"), 1.0, "tile_at uses scaled cells too");
    assert_eq!(num(&e, "hit"), 1.0, "a 4×2 box at x 150 touches the fourth tile (144..192)");
    assert_eq!(num(&e, "miss"), 0.0, "below the 48 px row there is nothing");
    assert_eq!((num(&e, "nw"), num(&e, "nh")), (32.0, 64.0), "tilemap_set_scale keeps the 16 px base cell");
    assert_eq!(num(&e, "n_at"), 1.0);
    // drawn scaled: tile 3 at x = 144 with scale 3
    let ts_spr = e.tilesets[0].sprite_index;
    let tiles: Vec<(u32, f32, f32, f32)> = e
        .cmds
        .iter()
        .filter_map(|c| match c {
            DrawCmd::Sprite { spr, frame, x, sx, sy, .. } if *spr == ts_spr => Some((*frame, *x, *sx, *sy)),
            _ => None,
        })
        .collect();
    assert!(tiles.contains(&(3, 144.0, 3.0, 3.0)), "{tiles:?}");
    // the room is as wide as the scaled layer (8 × 48)
    assert_eq!(e.room_w, 384.0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bad_tile_scale_is_a_load_error() {
    let dir = temp("badscale");
    project(&dir, "");
    write(&dir, "rooms/r.toml", "[[layer]]\nname = \"T\"\ntype = \"tiles\"\ntileset = \"ts\"\nscale = -2\ndata = '0'\n");
    let err = Engine::load(&dir).err().expect("negative scale must fail");
    assert!(err.contains("scale") && err.contains("'T'"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rle_sprites_and_rotation_settings() {
    let dir = temp("rle");
    project(&dir, "rotation = \"rotsprite\"\nrotation_steps = 32");
    // the same 6×3 image twice: plain and run-length (the digits of `3*2` are a count, the
    // lone `r` after a space is a colour)
    write(&dir, "sprites/plain.toml", "origin = \"center\"\npixels = ['''\nrrrrgg\n..rr..\n..rr..\n''']\n");
    write(&dir, "sprites/rle.toml", "origin = \"center\"\nrotation = \"nearest\"\npixels_rle = ['''\nr*4g*2\n.*2r*2.*2 *2\n''']\n");
    write(&dir, "sprites/smooth.toml", "rotation = \"smooth\"\nrotation_steps = 0\npixels = ['''\nr\n''']\n");
    write(&dir, "objects/o.ms", "object O {}\n");
    write(
        &dir,
        "rooms/r.toml",
        r#"code = """
global.before = sprite_get_rotation(plain)
sprite_set_rotation(rle, "rotsprite", 8)
global.after = sprite_get_rotation(rle)
global.steps = sprite_get_rotation_steps(rle)
"""
"#,
    );
    let mut e = Engine::load(&dir).unwrap_or_else(|err| panic!("{err}"));
    let s = |e: &Engine, n: &str| e.sprites.iter().position(|s| s.name == n).unwrap();
    let (p, r, sm) = (s(&e, "plain"), s(&e, "rle"), s(&e, "smooth"));
    assert!(e.sprites[p].rgba == e.sprites[r].rgba, "pixels_rle draws the same pixels");
    assert_eq!((e.sprites[r].fw, e.sprites[r].fh, e.sprites[r].origin), (6, 3, (3.0, 1.5)));
    assert_eq!(e.sprites[p].bbox, e.sprites[r].bbox);
    // game.toml default vs the sprite's own keys
    assert_eq!((e.sprites[p].rotation.mode, e.sprites[p].rotation.steps), (RotMode::Rotsprite, 32));
    assert_eq!((e.sprites[r].rotation.mode, e.sprites[r].rotation.steps), (RotMode::Nearest, 32));
    assert_eq!((e.sprites[sm].rotation.mode, e.sprites[sm].rotation.steps), (RotMode::Smooth, 0));
    e.start();
    e.tick();
    assert!(e.errors.is_empty(), "{:?}", e.errors);
    assert_eq!(global(&e, "before").to_string(), "rotsprite");
    assert_eq!(global(&e, "after").to_string(), "rotsprite");
    assert_eq!(num(&e, "steps"), 8.0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bad_rotation_names_fail_with_the_file() {
    let dir = temp("badrot");
    project(&dir, "");
    write(&dir, "sprites/x.toml", "rotation = \"bilinear\"\npixels = ['''\nr\n''']\n");
    write(&dir, "rooms/r.toml", "");
    let err = Engine::load(&dir).err().expect("unknown rotation must fail");
    assert!(err.contains("x.toml") && err.contains("bilinear"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}
