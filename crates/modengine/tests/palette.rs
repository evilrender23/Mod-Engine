//! Pear36 default palette: new projects use it, sprites drawn with its characters load,
//! and scripts can read the palette.

use modengine::palette::{Palette, PEAR36};
use modengine::Engine;
use std::path::PathBuf;

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("modengine-palette-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn pixel(e: &Engine, sprite: &str, x: usize, y: usize) -> [u8; 4] {
    let s = e.sprites.iter().find(|s| s.name == sprite).unwrap();
    let i = (y * s.sheet_w as usize + x) * 4;
    s.rgba[i..i + 4].try_into().unwrap()
}

#[test]
fn new_project_uses_pear36_and_runs() {
    let dir = temp("new");
    modengine::scaffold::create(&dir).unwrap();
    let text = std::fs::read_to_string(dir.join("game.toml")).unwrap();
    assert!(text.contains("Pear36"), "{text}");
    let mut e = Engine::load(&dir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(e.config.palette, Palette::pear36().char_map());
    // hero.toml row 1 is ".K4444K.": K = #272736, 4 = #ffe478
    assert_eq!(pixel(&e, "hero", 1, 1), [0x27, 0x27, 0x36, 255]);
    assert_eq!(pixel(&e, "hero", 2, 1), [0xff, 0xe4, 0x78, 255]);
    assert_eq!(pixel(&e, "hero", 0, 0)[3], 0, "'.' is transparent");
    e.start();
    for _ in 0..30 {
        e.tick();
        e.render();
    }
    assert!(e.errors.is_empty(), "{:?}", e.errors);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn scripts_read_the_palette() {
    let dir = temp("script");
    modengine::scaffold::create(&dir).unwrap();
    std::fs::write(
        dir.join("objects/probe.ms"),
        r#"object Probe {
    on create {
        global.n = palette_size()
        global.first = palette_color(0)
        global.white = palette_color("F")
        global.last = palette_color(35)
        global.ch = palette_char(15)
        global.near = palette_nearest(#ff0000)
        global.miss = palette_char(99)
    }
}
"#,
    )
    .unwrap();
    let room = std::fs::read_to_string(dir.join("rooms/main.toml")).unwrap().replace("{ \"@\" = \"Hero\" }", "{ \"@\" = \"Hero\", \"P\" = \"Probe\" }").replace(".........@", "P........@");
    std::fs::write(dir.join("rooms/main.toml"), room).unwrap();
    let mut e = Engine::load(&dir).unwrap_or_else(|e| panic!("{e}"));
    e.start();
    e.tick();
    assert!(e.errors.is_empty(), "{:?}", e.errors);
    let g = |n: &str| e.globals.get(&modscript::intern(n)).cloned().unwrap_or_else(|| panic!("global {n} not set"));
    assert_eq!(g("n").as_num(), Some(36.0));
    assert_eq!(g("first").as_num(), Some(PEAR36[0] as f64));
    assert_eq!(g("white").as_num(), Some(0xffffeb as f64));
    assert_eq!(g("last").as_num(), Some(0xffb5b5 as f64));
    assert_eq!(g("ch").to_string(), "F");
    assert_eq!(g("near").as_num(), Some(27.0));
    assert_eq!(g("miss").to_string(), "");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn old_palettes_still_load() {
    // the example keeps its own letters: nothing about Pear36 is forced on it
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/platformer");
    let e = Engine::load(&dir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(e.config.palette.get("k").map(String::as_str), Some("#1a1c2c"));
}
