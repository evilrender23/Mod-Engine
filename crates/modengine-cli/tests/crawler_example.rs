//! examples/crawler (Cripta de Naipes) end to end through the `modengine` binary, without a
//! window: the `crawler` template passes `modengine check`, a fight started with the debug
//! jump takes damage from a card played with the keyboard, a whole fight is won with keys
//! only, and the stairs lead to floor 2 (saving the run).

use std::path::{Path, PathBuf};
use std::process::Command;

fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/crawler")
}

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("modengine-crawler-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn run(args: &[&str]) -> (i32, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_modengine")).args(args).output().unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    (o.status.code().unwrap_or(-1), text)
}

fn check(extra: &[&str]) -> (i32, String) {
    let ex = example();
    let mut args = vec!["check", ex.to_str().unwrap(), "--room", "crypt", "--seed", "3"];
    args.extend_from_slice(extra);
    run(&args)
}

#[test]
fn template_creates_a_project_that_passes_check() {
    let dir = tmp("template");
    let (code, out) = run(&["new", "--template", "crawler", dir.to_str().unwrap()]);
    assert_eq!(code, 0, "{out}");
    assert!(dir.join("data/cards.toml").is_file() && dir.join("dungeons/piso1.toml").is_file() && dir.join("lang/en.toml").is_file());
    let (code, out) = run(&["check", dir.to_str().unwrap(), "--no-save", "--steps", "120"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("static analysis: no problems found") && out.contains("no runtime errors"), "{out}");
    // the run room too, through a fight and a chest
    let (code, out) = run(&[
        "check", dir.to_str().unwrap(), "--room", "crypt", "--no-save", "--steps", "300",
        "--exec-at", "10:global.debug = \"combat\"", "--exec-at", "200:global.debug = \"chest\"",
    ]);
    assert_eq!(code, 0, "{out}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_card_played_with_the_keyboard_hurts_the_enemy() {
    // debug jump to a fight with group 0 of floor 1 (one crypt rat), focus the first attack
    // card of the hand and press Enter
    let focus = "100:for (c in deck_to_array(global.cb.hand)) { if data_get(\"cards\", c.id, \"kind\") == \"attack\" { ui_focus_set(\"hand\", c.uid); break } }";
    let (code, out) = check(&[
        "--no-save", "--steps", "240",
        "--exec-at", "20:global.debug = \"combat:0\"",
        "--exec-at", "21:global.hp0 = global.cb.enemies[0].hp",
        "--exec-at", focus,
        "--input", "110:enter",
        "--assert", "105:global.cb.state == \"player\" and global.cb.mana == 3",
        "--assert", "200:global.cb.enemies[0].hp < global.hp0",
        "--assert", "200:global.run.stats.cards == 1",
        "--assert", "200:global.cb.combo == 1",
    ]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("asserts: 4 de 4 correctos"), "{out}");
}

#[test]
fn a_fight_can_be_won_with_keys_only() {
    // Enter plays the focused card; E ends the turn. The rat (9 HP) falls in a few turns.
    let mut keys = vec![];
    for i in 0..40 {
        let s = 120 + i * 30;
        keys.push(format!("{s}:enter"));
        if i % 4 == 3 {
            keys.push(format!("{}:e", s + 15));
        }
    }
    let input = keys.join(",");
    let (code, out) = check(&[
        "--no-save", "--steps", "1500",
        "--exec-at", "20:global.debug = \"combat:0\"",
        "--input", &input,
        "--assert", "1500:global.run.stats.kills == 1",
        "--assert", "1500:global.run.state == \"explore\" and global.cb == null",
        "--assert", "1500:global.run.hp > 0",
    ]);
    assert_eq!(code, 0, "{out}");
}

#[test]
fn the_stairs_lead_to_the_next_floor_and_save_the_run() {
    let saves = tmp("saves");
    let (code, out) = check(&[
        "--save-dir", saves.to_str().unwrap(), "--steps", "500",
        "--exec-at", "20:global.debug = \"look:stairs\"",
        "--input", "40:w,60:w,80:w",
        "--assert", "30:global.run.floor == 1",
        "--assert", "400:global.run.floor == 2 and global.run.state == \"explore\"",
    ]);
    assert_eq!(code, 0, "{out}");
    let save = std::fs::read_to_string(saves.join("partida.json")).expect("the run is saved on every floor");
    assert!(save.contains("\"floor\":2") || save.contains("\"floor\": 2"), "{save}");
    let _ = std::fs::remove_dir_all(&saves);
}
