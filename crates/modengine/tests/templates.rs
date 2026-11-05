//! Every `modengine new --template` project loads with the real engine and runs without
//! script errors, also with a custom name, display size and palette.

use modengine::palette::Palette;
use modengine::scaffold::{create_with, NewProject, TEMPLATES};
use modengine::Engine;
use std::path::PathBuf;

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("modengine-templates-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn run(dir: &std::path::Path) -> Engine {
    let mut e = Engine::load(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    e.start();
    for _ in 0..120 {
        e.tick();
        e.render();
        e.audio.clear();
    }
    assert!(e.errors.is_empty(), "{}: {:?}", dir.display(), e.errors);
    e
}

#[test]
fn every_template_loads_and_runs() {
    for t in TEMPLATES {
        let dir = temp(t.id);
        create_with(&dir, &NewProject { template: t.id.into(), ..Default::default() }).unwrap();
        let e = run(&dir);
        assert!(!e.rooms.is_empty(), "{}", t.id);
        assert!(!dir.join("saves").exists());
        // a second time it refuses and writes nothing
        assert!(create_with(&dir, &NewProject { template: t.id.into(), ..Default::default() }).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn options_set_name_display_and_palette() {
    // a 4-colour palette: every sprite gets remapped to it
    let pal = Palette { name: "Mini".into(), author: "test".into(), colors: vec![0x000000, 0xffffff, 0xff0000, 0x00ff00] };
    for id in ["basic", "topdown", "platformer"] {
        let dir = temp(&format!("opts-{id}"));
        let opts = NewProject { template: id.into(), name: Some("Mi \"Juego\" ñ".into()), display: Some((256, 144, 4)), palette: Some(pal.clone()) };
        create_with(&dir, &opts).unwrap();
        let e = run(&dir);
        assert_eq!(e.config.game.name, "Mi \"Juego\" ñ");
        assert_eq!((e.config.display.width, e.config.display.height, e.config.display.scale), (256, 144, 4));
        assert_eq!(e.config.palette, pal.char_map(), "{id}");
        // every opaque pixel of every sprite is one of the palette's colours
        for s in &e.sprites {
            for px in s.rgba.chunks(4).filter(|p| p[3] == 255) {
                let c = (px[0] as u32) << 16 | (px[1] as u32) << 8 | px[2] as u32;
                assert!(pal.colors.contains(&c), "{id}/{}: #{c:06x}", s.name);
            }
        }
        let game = std::fs::read_to_string(dir.join("game.toml")).unwrap();
        if id == "platformer" {
            assert!(game.contains("package = \"com.modengine.mijuego\""), "{game}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn unknown_template_is_an_error() {
    let dir = temp("unknown");
    let err = create_with(&dir, &NewProject { template: "nope".into(), ..Default::default() }).unwrap_err();
    assert!(err.contains("platformer"), "{err}");
    assert!(!dir.exists());
}
