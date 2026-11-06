//! `modengine dungeon`: exit codes, JSON output, PNG previews, data-table configs.

use std::path::PathBuf;
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_modengine"))
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("modengine-dungeon-cli-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

const CONFIG: &str = r#"
width = 48
height = 40
rooms = [6, 8]
extra_edges = 0.3

[[templates]]
name = "sala"
rows = ['.....', '..E..', '.....']

[[events]]
type = "enemy"
count = [2, 4]
where = "room"
marker = "E"
"#;

#[test]
fn ascii_summary_and_png() {
    let d = tmp("png");
    std::fs::write(d.join("cfg.toml"), CONFIG).unwrap();
    let png = d.join("out/m.png");
    let out = bin().args(["dungeon", "-"]).arg(d.join("cfg.toml")).args(["--seed", "5", "--scale", "4", "--png"]).arg(&png).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{text}\n{}", String::from_utf8_lossy(&out.stderr));
    assert!(text.contains("semilla 5") && text.contains("salas"), "{text}");
    // the ASCII map: one line per row
    assert_eq!(text.lines().filter(|l| l.len() == 48 && l.starts_with('#')).count(), 40);
    let img = image::open(&png).unwrap();
    assert_eq!((img.width(), img.height()), (48 * 4, 40 * 4));
}

#[test]
fn json_is_deterministic_and_complete() {
    let d = tmp("json");
    std::fs::write(d.join("cfg.json"), serde_json::to_string(&toml::from_str::<serde_json::Value>(CONFIG).unwrap()).unwrap()).unwrap();
    let run = || {
        let out = bin().args(["dungeon", "-"]).arg(d.join("cfg.json")).args(["--seed", "9", "--json", "--no-ascii"]).output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()
    };
    let (a, b) = (run(), run());
    assert_eq!(a["hash"], b["hash"]);
    let rooms = a["rooms"].as_array().unwrap();
    assert!(rooms.len() >= 6);
    assert_eq!(a["mst"].as_array().unwrap().len(), rooms.len() - 1);
    assert!(a["events"].as_array().unwrap().iter().all(|e| e["type"] == "enemy"));
    assert!(a.get("ascii").is_none());
    // several seeds: an array
    let out = bin().args(["dungeon", "-"]).arg(d.join("cfg.json")).args(["--seed", "1,2,abc", "--json"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 3);
    assert_eq!(v[2]["seed_text"], "abc");
}

#[test]
fn exit_codes() {
    let d = tmp("codes");
    // bad arguments
    assert_eq!(bin().args(["dungeon"]).output().unwrap().status.code(), Some(2));
    assert_eq!(bin().args(["dungeon", "-", "x.toml", "--nope"]).output().unwrap().status.code(), Some(2));
    // missing file / bad config
    assert_eq!(bin().args(["dungeon", "-", "no-existe.toml"]).output().unwrap().status.code(), Some(1));
    std::fs::write(d.join("bad.toml"), "[[events]]\ntype = \"x\"\nwhere = \"cielo\"\n").unwrap();
    let out = bin().args(["dungeon", "-"]).arg(d.join("bad.toml")).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("cielo"));
    // warnings: 0, or 3 with --strict
    std::fs::write(d.join("tight.toml"), "width = 16\nheight = 16\nrooms = [12, 12]\nattempts = 2\n").unwrap();
    assert_eq!(bin().args(["dungeon", "-"]).arg(d.join("tight.toml")).output().unwrap().status.code(), Some(0));
    assert_eq!(bin().args(["dungeon", "-"]).arg(d.join("tight.toml")).arg("--strict").output().unwrap().status.code(), Some(3));
    // help
    let out = bin().args(["dungeon", "--help"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("table:id"));
}

#[test]
fn config_from_a_project_file_and_a_data_table() {
    let p = tmp("proj");
    for dir in ["rooms", "data", "dungeon"] {
        std::fs::create_dir_all(p.join(dir)).unwrap();
    }
    std::fs::write(p.join("game.toml"), "[game]\nname = \"p\"\nrooms = [\"r\"]\n").unwrap();
    std::fs::write(p.join("rooms/r.toml"), "width = 320\nheight = 240\n").unwrap();
    std::fs::write(p.join("dungeon/piso1.toml"), CONFIG).unwrap();
    std::fs::write(p.join("data/dungeons.toml"), "[[dungeon]]\nid = \"piso2\"\nwidth = 40\nheight = 40\nrooms = [5, 5]\n").unwrap();
    // relative to the project
    let out = bin().arg("dungeon").arg(&p).arg("dungeon/piso1.toml").args(["--json", "--no-ascii"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    // a record of a data table
    let out = bin().arg("dungeon").arg(&p).arg("dungeons:piso2").args(["--json", "--no-ascii"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["width"], 40);
    assert_eq!(v["rooms"].as_array().unwrap().len(), 5);
    let out = bin().arg("dungeon").arg(&p).arg("dungeons:nada").output().unwrap();
    assert_eq!(out.status.code(), Some(1));
}
