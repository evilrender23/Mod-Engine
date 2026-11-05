//! `modengine import` / `modengine inspect-image` run as a process on generated art.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("modengine_import_cli_{tag}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn project(dir: &Path) -> PathBuf {
    let p = dir.join("game");
    modengine::scaffold::create_with(&p, &modengine::scaffold::NewProject { template: "empty".into(), ..Default::default() }).unwrap();
    p
}

/// `n` blobs of `cell`×`h` with transparent gutters, side by side.
fn strip(path: &Path, n: u32, cell: u32, h: u32) {
    let img = image::RgbaImage::from_fn(n * cell, h, |x, _| if x % cell >= 2 && x % cell < cell - 2 { image::Rgba([200, (x / cell * 50) as u8, 40, 255]) } else { image::Rgba([0, 0, 0, 0]) });
    img.save(path).unwrap();
}

fn opaque(path: &Path, w: u32, h: u32) {
    image::RgbaImage::from_fn(w, h, |x, y| image::Rgba([(x / 16 * 60) as u8, (y / 16 * 60) as u8, 90, 255])).save(path).unwrap();
}

fn wav(path: &Path) {
    let data = vec![0u8; 200];
    let mut b = b"RIFF".to_vec();
    b.extend((36 + data.len() as u32).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(8000u32.to_le_bytes());
    b.extend(16000u32.to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend((data.len() as u32).to_le_bytes());
    b.extend(data);
    fs::write(path, b).unwrap();
}

/// The art folder: a strip named as one, an unnamed strip, a grid sheet, a tile set, a sound,
/// a font, a licence and junk.
fn art(dir: &Path) -> PathBuf {
    let a = dir.join("Pack de arte");
    fs::create_dir_all(a.join("enemies")).unwrap();
    fs::create_dir_all(a.join("__MACOSX")).unwrap();
    strip(&a.join("hero_run_strip4.png"), 4, 16, 16);
    strip(&a.join("Slime Idle.png"), 3, 20, 14);
    // 2 rows of 4 cells of 16×16
    let grid = image::RgbaImage::from_fn(64, 32, |x, y| if x % 16 > 2 && x % 16 < 13 && y % 16 > 2 && y % 16 < 13 { image::Rgba([10, 200, 10, 255]) } else { image::Rgba([0, 0, 0, 0]) });
    grid.save(a.join("enemies/bat_16x16.png")).unwrap();
    opaque(&a.join("Cave Tiles.png"), 64, 48);
    wav(&a.join("Jump.WAV"));
    let mut font = vec![0u8, 1, 0, 0];
    font.extend([0u8; 60]);
    fs::write(a.join("Pixel Font.ttf"), font).unwrap();
    fs::write(a.join("LICENSE.txt"), "Art by Test Artist, CC-BY 4.0\n").unwrap();
    fs::write(a.join(".DS_Store"), "junk").unwrap();
    fs::write(a.join("Thumbs.db"), "junk").unwrap();
    fs::write(a.join("__MACOSX/._hero.png"), "junk").unwrap();
    fs::write(a.join("preview.jpg"), "not supported").unwrap();
    a
}

fn run(args: &[&str]) -> (i32, String, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_modengine")).args(args).output().unwrap();
    (o.status.code().unwrap_or(-1), String::from_utf8_lossy(&o.stdout).into_owned(), String::from_utf8_lossy(&o.stderr).into_owned())
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn imports_a_folder_of_art() {
    let dir = tmp("folder");
    let game = project(&dir);
    let art = art(&dir);

    // --dry-run: the plan, nothing written
    let (code, out, err) = run(&["import", s(&game), s(&art), "--dry-run", "--json"]);
    assert_eq!(code, 0, "{out}{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["dry_run"], true);
    assert_eq!(v["assets"].as_array().unwrap().len(), 6, "{out}");
    assert!(!game.join("sprites/hero_run.png").exists() && !game.join("CREDITS.md").exists());

    let (code, out, err) = run(&["import", s(&game), s(&art), "--json", "--credit", "Art by Test Artist, CC-BY 4.0"]);
    assert_eq!(code, 0, "{out}{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let assets = v["assets"].as_array().unwrap();
    let by = |name: &str| assets.iter().find(|a| a["name"] == name).unwrap_or_else(|| panic!("{name} in {out}")).clone();
    let hero = by("hero_run");
    assert_eq!((hero["kind"].as_str(), hero["frames"].as_u64(), hero["frame_size"].clone()), (Some("sprite"), Some(4), serde_json::json!([16, 16])));
    assert_eq!(hero["files"], serde_json::json!(["sprites/hero_run.png", "sprites/hero_run.toml"]));
    let bat = by("bat");
    assert_eq!((bat["frames"].as_u64(), bat["frame_size"].clone()), (Some(8), serde_json::json!([16, 16])));
    let slime = by("slime_idle");
    assert_eq!(slime["frames"], 1, "no name hint: one frame...");
    assert!(slime["hint"].as_str().unwrap().contains("--frames 3"), "...with the gutters as a hint: {slime}");
    let ts = by("ts_cave_tiles");
    assert_eq!((ts["kind"].as_str(), ts["sprite"].as_str(), ts["tile_size"].clone(), ts["frames"].as_u64()), (Some("tileset"), Some("spr_cave_tiles"), serde_json::json!([16, 16]), Some(12)));
    assert_eq!(by("jump")["files"], serde_json::json!(["sounds/jump.wav"]));
    assert_eq!(by("pixel_font")["files"], serde_json::json!(["fonts/pixel_font.ttf"]));
    let skipped: Vec<&str> = v["skipped"].as_array().unwrap().iter().map(|x| x["path"].as_str().unwrap()).collect();
    assert!(skipped.len() == 1 && skipped[0].ends_with("preview.jpg"), "junk is silent, unsupported files are listed: {skipped:?}");
    assert!(v["license_files"][0].as_str().unwrap().ends_with("LICENSE.txt"));
    assert_eq!(v["credits"], "CREDITS.md");
    let credits = fs::read_to_string(game.join("CREDITS.md")).unwrap();
    assert!(credits.contains("## Pack de arte") && credits.contains("CC-BY 4.0") && credits.contains("`sprites/hero_run.png` (de «hero_run_strip4.png»"), "{credits}");

    // the engine loads it all
    let p = modengine::project::Project::load(&game).unwrap();
    let spr = |n: &str| p.sprites.iter().find(|s| s.name == n).unwrap();
    assert_eq!((spr("hero_run").frames.len(), spr("hero_run").fw), (4, 16));
    assert_eq!(spr("bat").frames.len(), 8);
    assert_eq!(p.tilesets.iter().find(|t| t.name == "ts_cave_tiles").unwrap().count(), 12);
    assert!(p.sounds.iter().any(|s| s.name == "jump") && p.fonts.iter().any(|f| f.name == "pixel_font"));
    let (code, out, err) = run(&["check", s(&game), "--steps", "5"]);
    assert_eq!(code, 0, "{out}{err}");

    // again: new names, nothing replaced
    let (code, out, _) = run(&["import", s(&game), s(&art.join("hero_run_strip4.png"))]);
    assert_eq!(code, 0);
    assert!(out.contains("+ sprite hero_run_2") && game.join("sprites/hero_run_2.toml").exists(), "{out}");
    // an explicit name that is taken is an error...
    let (code, _, err) = run(&["import", s(&game), s(&art.join("hero_run_strip4.png")), "--name", "hero_run"]);
    assert_eq!(code, 1);
    assert!(err.contains("--overwrite"), "{err}");
    // ...unless replacing: the old files go to .trash
    let (code, out, err) = run(&["import", s(&game), s(&art.join("hero_run_strip4.png")), "--name", "hero_run", "--frames", "2", "--origin", "bottom-center", "--fps", "12", "--overwrite"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("~ sprite hero_run") && game.join(".trash/hero_run.toml").exists(), "{out}");
    assert_eq!(fs::read_to_string(game.join("sprites/hero_run.toml")).unwrap(), "image = \"hero_run.png\"\nframes = 2\norigin = \"bottom\"\nfps = 12\n");
    // a name used by an object can't be taken
    fs::create_dir_all(game.join("objects")).unwrap();
    fs::write(game.join("objects/game.ms"), "object Game {\n}\n").unwrap();
    let (code, _, err) = run(&["import", s(&game), s(&art.join("Slime Idle.png")), "--name", "Game"]);
    assert_eq!(code, 1, "{err}");
    // --name is for one file; bad options are usage errors; missing files are errors
    assert_eq!(run(&["import", s(&game), s(&art), "--name", "x"]).0, 1);
    assert_eq!(run(&["import", s(&game), s(&art), "--grid", "16"]).0, 2);
    assert_eq!(run(&["import", s(&game), s(&art), "--fps", "-1"]).0, 2);
    assert_eq!(run(&["import", s(&game), s(&dir.join("nope.png"))]).0, 1);
    assert_eq!(run(&["import", s(&dir), s(&art)]).0, 1, "not a project");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn tile_sets_prefixes_and_kinds() {
    let dir = tmp("tiles");
    let game = project(&dir);
    let art = art(&dir);
    // --tile makes a PNG a tile set, with offset and separation
    let sheet = dir.join("sheet.png");
    opaque(&sheet, 35, 18); // 1 px offset, 1 px separation: 2×1 tiles of 16×16
    let (code, out, err) = run(&["import", s(&game), s(&sheet), "--tile", "16x16", "--offset", "1,1", "--sep", "1,1", "--name", "ts_level"]);
    assert_eq!(code, 0, "{out}{err}");
    let t = fs::read_to_string(game.join("tilesets/ts_level.toml")).unwrap();
    assert_eq!(t, "sprite = \"spr_level\"\ntile_width = 16\ntile_height = 16\noffset = [1, 1]\nseparation = [1, 1]\n");
    let p = modengine::project::Project::load(&game).unwrap();
    assert_eq!(p.tilesets.iter().find(|t| t.name == "ts_level").unwrap().count(), 2);
    // --kind sound takes only sounds from a folder; --prefix names by subfolder
    let (code, out, _) = run(&["import", s(&game), s(&art), "--kind", "sound", "--json"]);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["assets"].as_array().unwrap().len(), 1);
    let (code, out, _) = run(&["import", s(&game), s(&art.join("enemies")), "--prefix", "--name-prefix", "spr_", "--dry-run"]);
    assert_eq!(code, 0);
    assert!(out.contains("sprite spr_bat"), "{out}");
    // a sheet with one animation per row: a sprite per row, empty cells dropped, "-" skips a row
    let sheet = image::RgbaImage::from_fn(48, 48, |x, y| {
        let filled = match y / 16 { 0 => x < 48, 1 => x < 32, _ => x < 16 };
        if filled && x % 16 > 1 && y % 16 > 1 { image::Rgba([90, 90, 250, 255]) } else { image::Rgba([0, 0, 0, 0]) }
    });
    sheet.save(dir.join("knight.png")).unwrap();
    let (code, out, err) = run(&["import", s(&game), s(&dir.join("knight.png")), "--grid", "16x16", "--row-names", "idle,-,jump", "--origin", "bottom-center", "--json"]);
    assert_eq!(code, 0, "{out}{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let rows: Vec<(String, u64)> = v["assets"].as_array().unwrap().iter().map(|a| (a["name"].as_str().unwrap().to_string(), a["frames"].as_u64().unwrap())).collect();
    assert_eq!(rows, [("knight_idle".to_string(), 3), ("knight_jump".to_string(), 1)]);
    let p = modengine::project::Project::load(&game).unwrap();
    let k = p.sprites.iter().find(|s| s.name == "knight_idle").unwrap();
    assert_eq!((k.frames.len(), k.sheet_w, k.sheet_h, k.origin), (3, 48, 16, (8.0, 16.0)));
    assert_eq!(run(&["import", s(&game), s(&dir.join("knight.png")), "--split-rows"]).0, 1, "needs a grid");
    // --crop: a sprite out of part of an atlas
    let (code, out, err) = run(&["import", s(&game), s(&dir.join("knight.png")), "--crop", "0,16,32,16", "--frames", "2", "--name", "knight_walk"]);
    assert_eq!(code, 0, "{out}{err}");
    let p = modengine::project::Project::load(&game).unwrap();
    let k = p.sprites.iter().find(|s| s.name == "knight_walk").unwrap();
    assert_eq!((k.frames.len(), k.fw, k.sheet_w, k.sheet_h), (2, 16, 32, 16));
    assert_eq!(run(&["import", s(&game), s(&dir.join("knight.png")), "--crop", "40,40,20,20"]).0, 1, "outside the image");
    let (code, out, _) = run(&["import", s(&game), s(&art), "--prefix", "--kind", "sprite", "--dry-run", "--json"]);
    assert_eq!(code, 0);
    assert!(out.contains("\"enemies_bat\""), "{out}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn inspect_image_guesses_layouts() {
    let dir = tmp("inspect");
    let art = art(&dir);
    let (code, out, err) = run(&["inspect-image", s(&art), "--json"]);
    assert_eq!(code, 0, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let imgs = v["images"].as_array().unwrap();
    assert_eq!(imgs.len(), 4, "the PNGs, not the junk");
    let of = |end: &str| imgs.iter().find(|i| i["path"].as_str().unwrap().ends_with(end)).unwrap().clone();
    let best = |end: &str| of(end)["layouts"][0].clone();
    // gutters, no name hint: 3 frames of 20×14
    let slime = best("Slime Idle.png");
    assert_eq!((slime["kind"].as_str(), slime["frames"].as_u64(), slime["frame_size"].clone(), slime["import_args"].as_str()), (Some("strip"), Some(3), serde_json::json!([20, 14]), Some("--frames 3")));
    let bat = best("bat_16x16.png");
    assert_eq!((bat["kind"].as_str(), bat["frame_size"].clone(), bat["import_args"].as_str()), (Some("grid"), serde_json::json!([16, 16]), Some("--grid 16x16")));
    let tiles = best("Cave Tiles.png");
    assert_eq!((tiles["kind"].as_str(), tiles["import_args"].as_str()), (Some("tileset"), Some("--kind tileset --tile 16x16")));
    assert_eq!(of("Cave Tiles.png")["transparency"], false);
    assert_eq!(of("Slime Idle.png")["blobs"], serde_json::json!([[2, 0, 16, 14], [22, 0, 16, 14], [42, 0, 16, 14]]));
    assert_eq!(of("hero_run_strip4.png")["size"], serde_json::json!([64, 16]));
    assert!(of("hero_run_strip4.png")["colors"].as_u64().unwrap() >= 4);
    // human output and errors
    let (code, out, _) = run(&["inspect-image", s(&art.join("hero_run_strip4.png"))]);
    assert!(code == 0 && out.contains("64×16 px") && out.contains("--frames 4"), "{out}");
    assert_eq!(run(&["inspect-image", s(&dir.join("nope.png"))]).0, 1);
    let _ = fs::remove_dir_all(&dir);
}
