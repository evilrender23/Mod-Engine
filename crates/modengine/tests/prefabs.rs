//! Prefabs: groups of instances placed from rooms and spawned from scripts.

use modengine::Engine;
use modscript::{intern, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static N: AtomicU32 = AtomicU32::new(0);

fn project(files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("modengine-prefab-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("game.toml"), "[game]\nname = \"t\"\n\n[palette]\n\"w\" = \"#ffffff\"\n").unwrap();
    for (rel, text) in files {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    dir
}

const OBJS: &str = r#"
object Turret {
    hp: int(1, 50) = 10
    on create {
        global.created = 1
        // the whole group exists before any create event runs
        mates = len(prefab_members())
    }
}
object Guard {
    side = 0
    on create { mates = len(prefab_members()) }
}
object Lamp {
    power = 1
}
object Spawner {
    what: prefab = pf_tower
    on create { made = prefab_spawn(what, 200, 120, 0, { side: 9 }) }
}
"#;

const TOWER: &str = r#"
description = "Torreta con dos escoltas"

[[instance]]
object = "Turret"
x = 0
y = 0
vars = { hp = 20 }

[[instance]]
object = "Guard"
x = -16
y = 0
vars = { side = -1 }

[[instance]]
object = "Guard"
x = 16
y = 0
vars = { side = 1 }

[[instance]]
object = "pf_lamp"
x = 0
y = -16
"#;

const LAMP: &str = "[[instance]]\nobject = \"Lamp\"\nx = 0\ny = -8\nvars = { power = 3 }\n";

fn objs_named<'a>(e: &'a Engine, name: &str) -> Vec<&'a modengine::instance::Instance> {
    e.insts.iter().filter(|i| i.alive && e.objects[i.object as usize].name == name).collect()
}

fn var(i: &modengine::instance::Instance, n: &str) -> Value {
    i.vars.get(&intern(n)).cloned().unwrap_or(Value::Null)
}

#[test]
fn rooms_place_prefabs_with_offsets_nesting_and_vars() {
    let room = r#"
[[layer]]
type = "instances"
cell = [16, 16]
legend = { "T" = "pf_tower" }
map = '''
....
...T
'''
instances = [ { object = "pf_tower", x = 100, y = 50, vars = { hp = 3 } } ]
"#;
    let dir = project(&[("objects.ms", OBJS), ("prefabs/pf_tower.toml", TOWER), ("prefabs/pf_lamp.toml", LAMP), ("rooms/r.toml", room)]);
    let mut e = Engine::load(&dir).unwrap_or_else(|e| panic!("{e}"));
    e.start();
    assert!(e.errors.is_empty(), "{:?}", e.errors);
    // two towers: the free one and the one in the map
    let turrets = objs_named(&e, "Turret");
    assert_eq!(turrets.len(), 2);
    assert_eq!(objs_named(&e, "Guard").len(), 4);
    assert_eq!(objs_named(&e, "Lamp").len(), 2);

    // free instance: its vars replace the prefab's (hp 20 → 3)
    let free = turrets.iter().find(|t| t.x == 100.0).expect("free tower at 100, 50");
    assert_eq!(free.y, 50.0);
    assert_eq!(var(free, "hp").as_num(), Some(3.0));
    assert_eq!(var(free, "mates").as_num(), Some(4.0), "turret, two guards and the nested lamp");
    // map instance: the prefab's origin at the cell's top-left corner (col 3, row 1)
    let mapped = turrets.iter().find(|t| t.x == 48.0).expect("tower in the map at 48, 16");
    assert_eq!(mapped.y, 16.0);
    assert_eq!(var(mapped, "hp").as_num(), Some(20.0));

    let guards = objs_named(&e, "Guard");
    assert!(guards.iter().any(|g| g.x == 84.0 && var(g, "side").as_num() == Some(-1.0)));
    assert!(guards.iter().any(|g| g.x == 116.0 && var(g, "side").as_num() == Some(1.0)));
    // the nested lamp: (0, -16) in the tower + (0, -8) in the lamp
    let lamps = objs_named(&e, "Lamp");
    assert!(lamps.iter().any(|l| l.x == 100.0 && l.y == 26.0 && var(l, "power").as_num() == Some(3.0)));

    // every member knows its prefab and its group; groups don't mix
    let Value::Asset(k, pi) = var(free, "prefab") else { panic!("prefab must be an asset") };
    assert_eq!(e.asset_name(k, pi), "pf_tower");
    let Value::Array(g) = var(free, "prefab_group") else { panic!("prefab_group must be an array") };
    assert_eq!(g.borrow().len(), 4);
    assert!(g.borrow().iter().all(|v| matches!(v, Value::Inst(id) if e.insts[e.idx(*id).unwrap()].y <= 50.0 && e.insts[e.idx(*id).unwrap()].x >= 84.0)));
}

#[test]
fn scripts_spawn_query_and_destroy_groups() {
    let room = "[[layer]]\ntype = \"instances\"\ninstances = [ { object = \"Spawner\", x = 0, y = 0 } ]\n";
    let dir = project(&[("objects.ms", OBJS), ("prefabs/pf_tower.toml", TOWER), ("prefabs/pf_lamp.toml", LAMP), ("rooms/r.toml", room)]);
    let mut e = Engine::load(&dir).unwrap_or_else(|e| panic!("{e}"));
    e.start();
    assert!(e.errors.is_empty(), "{:?}", e.errors);
    assert_eq!(objs_named(&e, "Turret").len(), 1);
    let guards = objs_named(&e, "Guard");
    assert!(guards.iter().all(|g| var(g, "side").as_num() == Some(9.0)), "prefab_spawn's struct applies to every member");
    assert!(guards.iter().any(|g| g.x == 184.0 && g.y == 120.0));

    e.run_code(
        r#"
        global.n = prefab_count(pf_tower)
        global.d = prefab_description(pf_tower)
        global.objs = map(prefab_objects(pf_tower), function(o) { return asset_name(o) })
        global.of = asset_name(prefab_of(find(Guard)))
        global.killed = prefab_destroy(find(Lamp))
    "#,
    )
    .unwrap();
    let g = |n: &str| e.globals.get(&intern(n)).cloned().unwrap();
    assert_eq!(g("n").as_num(), Some(4.0));
    assert_eq!(g("d").to_string(), "Torreta con dos escoltas");
    assert_eq!(g("objs").to_string(), "[\"Turret\", \"Guard\", \"Guard\", \"Lamp\"]", "nested prefabs are expanded");
    assert_eq!(g("of").to_string(), "pf_tower");
    assert_eq!(g("killed").as_num(), Some(4.0), "destroying from any member takes the whole group");
    e.tick();
    assert_eq!(objs_named(&e, "Turret").len(), 0);
    assert_eq!(objs_named(&e, "Guard").len(), 0);
}

#[test]
fn bad_prefabs_fail_to_load_with_the_file_named() {
    let room = "[[layer]]\ntype = \"instances\"\n";
    // a cycle
    let dir = project(&[("objects.ms", OBJS.replace("what: prefab = pf_tower", "what = 0").as_str()), ("prefabs/a.toml", "[[instance]]\nobject = \"b\"\nx = 0\ny = 0\n"), ("prefabs/b.toml", "[[instance]]\nobject = \"a\"\nx = 0\ny = 0\n"), ("rooms/r.toml", room)]);
    let err = Engine::load(&dir).err().expect("cycle must fail");
    assert!(err.contains("prefabs/a.toml") && err.contains("a → b → a"), "{err}");
    // a typed variable out of range inside the prefab
    let dir = project(&[("objects.ms", OBJS), ("prefabs/pf_tower.toml", &TOWER.replace("hp = 20", "hp = 99")), ("prefabs/pf_lamp.toml", LAMP), ("rooms/r.toml", room)]);
    let err = Engine::load(&dir).err().expect("hp 99 is outside int(1, 50)");
    assert!(err.contains("prefabs/pf_tower.toml") && err.contains("hp"), "{err}");
    // an unknown object
    let dir = project(&[("objects.ms", OBJS), ("prefabs/pf_tower.toml", &TOWER.replace("\"Guard\"", "\"Gaurd\"")), ("prefabs/pf_lamp.toml", LAMP), ("rooms/r.toml", room)]);
    let err = Engine::load(&dir).err().unwrap();
    assert!(err.contains("Gaurd"), "{err}");
    // prefabs share the asset namespace
    let dir = project(&[("objects.ms", OBJS), ("prefabs/pf_tower.toml", TOWER), ("prefabs/pf_lamp.toml", LAMP), ("prefabs/Guard.toml", LAMP), ("rooms/r.toml", room)]);
    let err = Engine::load(&dir).err().unwrap();
    assert!(err.contains("duplicado 'Guard'") && err.contains("prefabs/Guard.toml"), "{err}");
}
