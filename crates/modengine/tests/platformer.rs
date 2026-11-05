//! Runs the example game without a window.

use modengine::input::key_index;
use modengine::Engine;
use std::path::PathBuf;

fn example() -> Engine {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/platformer");
    Engine::load(&dir).unwrap_or_else(|e| panic!("example failed to load:\n{e}"))
}

fn run(e: &mut Engine, steps: u32, hold: &[&str]) {
    for _ in 0..steps {
        e.input.down.iter_mut().for_each(|d| *d = false);
        for k in hold {
            e.input.down[key_index(k).unwrap()] = true;
        }
        e.tick();
        e.render();
        e.audio.clear();
    }
}

fn count(e: &Engine, object: &str) -> usize {
    e.insts.iter().filter(|i| i.alive && e.objects[i.object as usize].name == object).count()
}

#[test]
fn title_to_level_and_goal_to_next_level() {
    let mut e = example();
    e.start();
    assert_eq!(e.rooms[e.room].name, "title");

    e.input.pressed[key_index("enter").unwrap()] = true;
    run(&mut e, 2, &[]);
    assert_eq!(e.rooms[e.room].name, "level1");
    assert_eq!(count(&e, "Player"), 1);
    assert_eq!(count(&e, "Game"), 1, "the persistent Game object follows the player");

    // the player lands on the ground and stays there
    run(&mut e, 60, &[]);
    let p = e.insts.iter().find(|i| e.objects[i.object as usize].name == "Player").unwrap();
    let y = p.y;
    run(&mut e, 30, &[]);
    let p = e.insts.iter().find(|i| e.objects[i.object as usize].name == "Player").unwrap();
    assert_eq!(p.y, y, "player should rest on the ground");

    // teleport next to the flag and walk into it
    let ids = e.ids();
    for id in ids {
        let i = e.idx(id).unwrap();
        if e.objects[e.insts[i].object as usize].name == "Player" {
            e.insts[i].x = 860.0;
        }
    }
    run(&mut e, 120, &["right"]);
    assert_eq!(e.rooms[e.room].name, "level2");
    assert!(e.errors.is_empty(), "runtime errors: {:?}", e.errors);
}

#[test]
fn walking_into_a_slime_costs_a_life() {
    let mut e = example();
    e.start();
    e.input.pressed[key_index("enter").unwrap()] = true;
    run(&mut e, 2, &[]);
    // the first slime is a few tiles to the right of the start; the respawn takes 70 steps
    run(&mut e, 90, &["right"]);
    let lives = e.globals.get(&modengine::modscript::intern("lives")).unwrap().as_num().unwrap();
    assert_eq!(lives, 2.0);
    assert!(e.errors.is_empty(), "runtime errors: {:?}", e.errors);
}
