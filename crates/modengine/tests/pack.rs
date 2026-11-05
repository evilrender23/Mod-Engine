//! Exported games load their project from a pack (`game.dat` or appended to the runner):
//! every example must load and play exactly the same from a pack as from its folder.

use modengine::project::Project;
use modengine::{pack, vfs, Engine};
use std::path::PathBuf;

fn examples() -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut v: Vec<PathBuf> = std::fs::read_dir(&root).unwrap().flatten().map(|e| e.path()).filter(|p| p.join("game.toml").is_file()).collect();
    v.sort();
    assert!(v.len() >= 5, "expected the example projects in {}", root.display());
    v
}

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("me_pack_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn same_project(a: &Project, b: &Project, what: &str) {
    assert_eq!(format!("{:?}", a.config), format!("{:?}", b.config), "{what}: game.toml");
    assert_eq!(a.scripts, b.scripts, "{what}: scripts");
    let rooms = |p: &Project| p.rooms.iter().map(|(n, _, r)| (n.clone(), r.clone())).collect::<Vec<_>>();
    assert_eq!(rooms(a), rooms(b), "{what}: rooms");
    assert_eq!(a.sprites.len(), b.sprites.len(), "{what}: sprites");
    for (x, y) in a.sprites.iter().zip(&b.sprites) {
        assert_eq!((&x.name, &x.frames, &x.rgba, x.origin, x.bbox, x.fps), (&y.name, &y.frames, &y.rgba, y.origin, y.bbox, y.fps), "{what}: sprite {}", x.name);
    }
    let sounds = |p: &Project| p.sounds.iter().map(|s| (s.name.clone(), s.data.clone(), s.volume)).collect::<Vec<_>>();
    assert_eq!(sounds(a), sounds(b), "{what}: sounds");
    let fonts = |p: &Project| p.fonts.iter().map(|f| (f.name.clone(), f.data.clone(), f.size, f.atlas.as_ref().map(|a| a.rgba.clone()))).collect::<Vec<_>>();
    assert_eq!(fonts(a), fonts(b), "{what}: fonts");
    let ts = |p: &Project| p.tilesets.iter().map(|t| (t.name.clone(), t.sprite.rgba.clone(), t.tile_w, t.columns)).collect::<Vec<_>>();
    assert_eq!(ts(a), ts(b), "{what}: tilesets");
}

fn play(e: &mut Engine, steps: u32) -> Vec<String> {
    e.start();
    let mut frames = vec![];
    for i in 0..steps {
        e.tick();
        e.render();
        e.audio.clear();
        if i % 40 == 0 {
            frames.push(format!("{:?}", e.cmds));
        }
    }
    frames
}

#[test]
fn every_example_loads_the_same_from_a_pack() {
    let out = temp("examples");
    for dir in examples() {
        let name = dir.file_name().unwrap().to_string_lossy().to_string();
        let bytes = pack::pack_dir(&dir).unwrap();
        assert_eq!(bytes, pack::pack_dir(&dir).unwrap(), "{name}: packing is deterministic");
        let file = out.join(format!("{name}.dat"));
        std::fs::write(&file, &bytes).unwrap();
        let p = pack::Pack::open(&file).unwrap();
        assert!(p.file("game.toml").is_some());
        assert!(p.files().all(|(rel, _)| pack::include(rel)), "{name}: only runtime files");

        let src = vfs::mount(&file, p);
        let from_dir = Project::load(&dir).unwrap_or_else(|e| panic!("{name} (folder): {e}"));
        let from_pack = Project::load(src.root()).unwrap_or_else(|e| panic!("{name} (pack): {e}"));
        same_project(&from_dir, &from_pack, &name);

        let mut a = Engine::load(&dir).unwrap();
        let mut b = Engine::load_source(&src).unwrap();
        assert_eq!(a.shaders.list.len(), b.shaders.list.len(), "{name}: shaders");
        assert_eq!(a.extensions.len(), b.extensions.len(), "{name}: extensions");
        assert_eq!((a.paths.len(), a.timelines.len(), a.curves.len(), a.sequences.len()), (b.paths.len(), b.timelines.len(), b.curves.len(), b.sequences.len()), "{name}: motion assets");
        assert_eq!(a.save_dir, dir.join("saves"));
        assert_ne!(b.save_dir, a.save_dir, "{name}: exported games save in the user folder");
        // same (empty) save folder for both, so saved high scores don't differ
        let saves = out.join(format!("{name}_saves"));
        a.save_dir = saves.clone();
        b.save_dir = saves;
        let (fa, fb) = (play(&mut a, 241), play(&mut b, 241));
        assert!(fa == fb, "{name}: the pack plays different frames than the folder");
        vfs::unmount(&file);
    }
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn appended_pack_runs_from_an_executable_and_saves_in_the_user_folder() {
    let out = temp("exe");
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/platformer");
    let exe = out.join("Coin Climber");
    std::fs::write(&exe, pack::append_to(b"\x7fELF not really a runner", &pack::pack_dir(&dir).unwrap())).unwrap();
    let mut e = Engine::load_pack(&exe).unwrap();
    assert_eq!(e.config.game.name, "Coin Climber");
    assert!(e.save_dir.ends_with("coin_climber/saves") || e.save_dir.ends_with("coin_climber\\saves"), "{}", e.save_dir.display());
    // save_data() writes there, never next to the executable
    let custom = out.join("userdata");
    e.save_dir = custom.clone();
    e.set_startup_code("save_data(\"slot\", 3)").unwrap();
    e.start();
    e.tick();
    assert!(custom.join("slot.json").is_file());
    assert!(!out.join("saves").exists());
    vfs::unmount(&exe);
    let _ = std::fs::remove_dir_all(&out);
}
