//! `modengine store` run as a process against canned OpenGameArt pages
//! (`MODENGINE_STORE_FIXTURES`): never the network.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("modengine_store_cli_{tag}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn me(dir: &Path, interval_ms: u64) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_modengine"));
    c.env("MODENGINE_STORE_FIXTURES", dir.join("site")).env("XDG_CACHE_HOME", dir.join("cache")).env("MODENGINE_STORE_INTERVAL_MS", interval_ms.to_string());
    c
}

fn run(dir: &Path, args: &[&str]) -> Output {
    me(dir, 0).arg("store").args(args).output().unwrap()
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

fn requests(dir: &Path) -> Vec<(u64, String)> {
    fs::read_to_string(dir.join("site/requests.log"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once(' ').map(|(t, u)| (t.parse().unwrap(), u.to_string())))
        .collect()
}

#[test]
fn search_show_get_and_check() {
    let dir = tmp("flow");
    modengine_store::testing::fixture_site(&dir.join("site"));
    let project = dir.join("game");
    modengine::scaffold::create_with(&project, &modengine::scaffold::NewProject { template: "empty".into(), ..Default::default() }).unwrap();
    let p = project.to_str().unwrap();

    let o = run(&dir, &["search", "slime", "--licencia", "cc0", "--json"]);
    assert!(o.status.success(), "{}", text(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["results"].as_array().unwrap().len(), 24);
    assert!(requests(&dir)[0].1.contains("field_art_licenses_tid%5B%5D=4&"), "the CC0 filter reaches the URL");

    let o = run(&dir, &["show", "pixel-art-animated-slime", "--zip"]);
    assert!(o.status.success(), "{}", text(&o));
    let t = text(&o);
    assert!(t.contains("Licencia: CC0") && t.contains("Slime/slime_idle_strip4.png") && t.contains("--zip-entry"), "{t}");

    // zip entry + import options, credits written
    let o = run(&dir, &["get", "pixel-art-animated-slime", p, "--zip-entry", "*idle*", "--name", "slime", "--origin", "bottom-center", "--json"]);
    assert!(o.status.success(), "{}", text(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!((v["assets"][0]["name"].as_str(), v["assets"][0]["frames"].as_u64()), (Some("slime"), Some(4)));
    let md = fs::read_to_string(project.join("CREDITS.md")).unwrap();
    assert!(md.contains("## Pixel art animated Slime — rvros") && md.contains("<!-- oga:pixel-art-animated-slime -->"), "{md}");

    // a share-alike asset: exit code 4 until the licence is accepted
    let html = fs::read_to_string(dir.join("site/animated-slime.html")).unwrap().replace("CC-BY 3.0", "CC-BY-SA 3.0");
    fs::write(dir.join("site/animated-slime.html"), html).unwrap();
    let o = run(&dir, &["get", "animated-slime", p, "--frames", "4"]);
    assert_eq!(o.status.code(), Some(4), "{}", text(&o));
    assert!(text(&o).contains("--acepto-licencia"));
    let o = run(&dir, &["get", "animated-slime", p, "--frames", "4", "--name", "slime_sa", "--acepto-licencia"]);
    assert!(o.status.success(), "{}", text(&o));
    // the second time everything comes from the cache
    let before = requests(&dir).len();
    let o = run(&dir, &["get", "pixel-art-animated-slime", p, "--zip-entry", "*idle*", "--name", "slime", "--overwrite"]);
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(requests(&dir).len(), before, "cached for 24 h");
    assert_eq!(fs::read_to_string(project.join("CREDITS.md")).unwrap().matches("<!-- oga:pixel-art-animated-slime -->").count(), 1);

    // bad input
    assert_eq!(run(&dir, &["get", "https://opengameart.org/users/foo", p]).status.code(), Some(1), "not allowed by robots.txt");
    assert_eq!(run(&dir, &["search", "--tipo", "3d"]).status.code(), Some(2));
    assert_eq!(run(&dir, &["get", "animated-slime"]).status.code(), Some(2));

    let check = Command::new(env!("CARGO_BIN_EXE_modengine")).args(["check", p, "--steps", "5"]).output().unwrap();
    assert!(check.status.success(), "{}", text(&check));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn processes_share_the_rate_limit() {
    let dir = tmp("limit");
    modengine_store::testing::fixture_site(&dir.join("site"));
    // three processes at once, one request each (different pages, empty cache)
    let kids: Vec<_> = ["animated-slime", "tiny-creatures", "pixel-art-animated-slime"]
        .iter()
        .map(|slug| me(&dir, 600).args(["store", "show", slug, "--json"]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().unwrap())
        .collect();
    for mut k in kids {
        assert!(k.wait().unwrap().success());
    }
    let mut t: Vec<u64> = requests(&dir).iter().map(|r| r.0).collect();
    t.sort();
    assert_eq!(t.len(), 3);
    for w in t.windows(2) {
        assert!(w[1] - w[0] >= 590, "requests {} ms apart", w[1] - w[0]);
    }
    assert!(dir.join("cache/modengine/store/rate-limit.lock").is_file());
    let _ = fs::remove_dir_all(&dir);
}
