//! `modengine new <folder>`: a tiny playable starting project.

use std::fs;
use std::path::Path;

const GAME_HEAD: &str = r##"[game]
name = "My Mod Engine game"
start_room = "main"

[display]
width = 320
height = 180
scale = 3
letterbox = "#272736"

"##;

/// game.toml of a new project: the default palette is Pear36 (`0`-`9`, `A`-`Z`).
pub fn game_toml() -> String {
    format!("{GAME_HEAD}{}", crate::palette::game_toml_section(&crate::palette::Palette::pear36()))
}

// Pear36 characters: K outline, 4 yellow, F white, C blue.
const HERO: &str = r#"origin = "center"
pixels = ['''
..KKKK..
.K4444K.
K4FKKF4K
K444444K
.KCCCCK.
KCCCCCCK
.KCKKCK.
.KK..KK.
''']
"#;

const HERO_MS: &str = r#"// Objects are declared with `object Name { ... }`.
// `name = value` lines set variables on every new instance;
// `on <event> { ... }` blocks run when that event happens.

object Hero {
    sprite = hero
    move_speed = 2

    on create {
        print("Hello from instance", id)
    }

    on step {
        var dx = input_axis([key.left, key.a], [key.right, key.d])
        var dy = input_axis([key.up, key.w], [key.down, key.s])
        x = clamp(x + dx * move_speed, 4, room_width - 4)
        y = clamp(y + dy * move_speed, 4, room_height - 4)
        if dx != 0 scale_x = dx
    }

    on draw_gui {
        draw_text(4, 4, "Arrow keys / WASD to move")
    }
}
"#;

const ROOM: &str = r##"background = "#323e4f"

[[layer]]
type = "instances"
cell = [16, 16]
legend = { "@" = "Hero" }
map = '''
....................
....................
....................
....................
.........@..........
'''
"##;

pub fn create(dir: &Path) -> Result<(), String> {
    if dir.join("game.toml").exists() {
        return Err(format!("{} already contains a project", dir.display()));
    }
    let game = game_toml();
    let files: [(&str, &str); 4] = [
        ("game.toml", &game),
        ("sprites/hero.toml", HERO),
        ("objects/hero.ms", HERO_MS),
        ("rooms/main.toml", ROOM),
    ];
    for (rel, content) in files {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::write(&p, content).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    Ok(())
}
