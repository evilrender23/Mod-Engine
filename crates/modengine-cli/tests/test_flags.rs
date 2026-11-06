//! The terminal testing flags end to end through the `modengine` binary (docs/pruebas.md):
//! exit codes, mouse in --input, --test-script, --no-save / --save-dir, --profile, and
//! --screenshot-at in a hidden window (only where there is a display).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

static N: AtomicU32 = AtomicU32::new(0);

fn project() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("modengine-cli-testflags-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("rooms")).unwrap();
    std::fs::write(dir.join("game.toml"), "[game]\nname = \"flags\"\nstart_room = \"r\"\n[display]\nwidth = 64\nheight = 48\n").unwrap();
    // A gun: fires while the left button is held (every 5 steps), aims at the mouse.
    std::fs::write(
        dir.join("objects.ms"),
        r#"
object Gun {
    shots = 0
    cd = 0
    wheel = 0
    on step {
        cd = max(0, cd - 1)
        if mouse_down(mouse.left) && cd == 0 { shots += 1; cd = 5; spawn(Shot, x, y) }
        wheel += mouse_wheel()
        if steps() == 20 save_data("slot", { shots: shots })
    }
}
object Shot {
    on draw {
        set_color(#ff0000)
        draw_rect(mouse_x - 2, mouse_y - 2, mouse_x + 2, mouse_y + 2)
    }
}
"#,
    )
    .unwrap();
    std::fs::write(dir.join("rooms/r.toml"), "width = 64\nheight = 48\nbackground = \"#000000\"\n[[layer]]\ntype = \"instances\"\ninstances = [ { object = \"Gun\", x = 32, y = 24 } ]\n").unwrap();
    dir
}

fn run(args: &[&str]) -> (i32, String, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_modengine")).args(args).output().unwrap();
    (o.status.code().unwrap_or(-1), String::from_utf8_lossy(&o.stdout).into(), String::from_utf8_lossy(&o.stderr).into())
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn hold_to_fire_and_exit_codes() {
    let p = project();
    let saves = p.join("otra");
    let (code, out, err) = run(&[
        "check", s(&p), "--steps", "40", "--save-dir", s(&saves),
        "--input", "2:mouse=10,20, 5-26:mb_left, 30:wheel_up, 31-33:wheel_down",
        "--assert", "4:Gun.shots == 0",
        "--assert", "26:Gun.shots == 5",
        "--assert", "Gun.shots == 5",
        "--assert", "Gun.wheel == -1",
        "--assert", "*:mouse_x == 0 or steps() >= 2",
    ]);
    assert_eq!(code, 0, "{out}\n{err}");
    assert!(out.contains("asserts: 5 de 5 correctos"), "{out}");
    assert!(saves.join("slot.json").is_file() && !p.join("saves").exists());

    // a failing assert: exit code 3 and a clear message
    let (code, _, err) = run(&["check", s(&p), "--steps", "10", "--no-save", "--assert", "8:Gun.shots > 0"]);
    assert_eq!(code, 3, "{err}");
    assert!(err.contains("--assert fallido en el paso 8") && err.contains("Gun.shots > 0"), "{err}");
    assert!(!p.join("saves").exists(), "--no-save must not write the project's saves/");

    // runtime errors: exit code 1; bad flags: 2
    let (code, _, err) = run(&["check", s(&p), "--steps", "5", "--no-save", "--exec-at", "3:no_such_function()"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("1 runtime error(s)"), "{err}");
    assert_eq!(run(&["check", s(&p), "--assert", "3:("]).0, 2);
    assert_eq!(run(&["check", s(&p), "--input", "3:mouse=1"]).0, 2);
}

#[test]
fn test_scripts_and_profile() {
    let p = project();
    // inside the project folder (loaded once, not twice) and outside it
    let t = p.join("sonda.ms");
    let t2 = p.with_extension("fuera.ms");
    std::fs::write(&t2, "object Fuera {\n    on create { global.fuera = room_name() }\n}\n").unwrap();
    std::fs::write(&t, "global.vistas = 0\nobject Sonda {\n    persistent = true\n    on step { global.vistas += 1 }\n}\n").unwrap();
    let (code, out, err) = run(&["check", s(&p), "--steps", "12", "--no-save", "--test-script", s(&t), "--test-script", s(&t2), "--assert", "global.fuera == \"r\"", "--exec-at", "6:goto_room(r)", "--assert", "global.vistas == 12", "--assert", "count(Sonda) == 1", "--profile"]);
    assert_eq!(code, 0, "{out}\n{err}");
    assert!(out.contains("=== perfil (--profile) ===") && out.contains("Sonda") && out.contains("por objeto"), "{out}");
}

#[test]
fn screenshots_at_logic_steps_in_a_hidden_window() {
    if cfg!(target_os = "linux") && std::env::var_os("DISPLAY").is_none() {
        eprintln!("no DISPLAY: skipping the windowed screenshot test");
        return;
    }
    let p = project();
    let out = p.join("caps");
    let pattern = format!("{}/c_%03d.png", out.display());
    let take = || {
        run(&["run", s(&p), "--no-save", "--seed", "4", "--input", "2:mouse=10,20, 5-26:mb_left, 12:mouse=50,30", "--screenshot-at", "8,15", "--crop", "0,0,60,40", "--screenshot", &pattern])
    };
    let (code, so, se) = take();
    assert_eq!(code, 0, "{so}\n{se}");
    let a = image::open(out.join("c_008.png")).unwrap().to_rgba8();
    let b = image::open(out.join("c_015.png")).unwrap().to_rgba8();
    assert_eq!(a.dimensions(), (60, 40));
    // the red square follows the scripted mouse: (10,20) at step 8, (50,30) at step 15
    assert_eq!(a.get_pixel(10, 20).0, [255, 0, 0, 255]);
    assert_eq!(a.get_pixel(50, 30).0, [0, 0, 0, 255]);
    assert_eq!(b.get_pixel(50, 30).0, [255, 0, 0, 255]);
    let first = std::fs::read(out.join("c_015.png")).unwrap();
    assert_eq!(take().0, 0);
    assert!(first == std::fs::read(out.join("c_015.png")).unwrap(), "same flags, same bytes");
}
