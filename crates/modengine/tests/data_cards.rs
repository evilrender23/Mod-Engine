//! Data tables (`data/*.toml|json|csv` + schemas), card piles (`deck_*`), action queues
//! (`actionq_*`) and `data_dispatch`, run headless.

use modengine::Engine;
use modscript::{intern, Value};

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

static N: AtomicU32 = AtomicU32::new(0);

fn project(files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("modengine-data-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&dir);
    for d in ["rooms", "sprites", "sounds", "data"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    std::fs::write(dir.join("game.toml"), "[game]\nname = \"data\"\n\n[palette]\n\"w\" = \"#ffffff\"\n").unwrap();
    std::fs::write(dir.join("sprites/spr_slash.toml"), "pixels = ['''\nw\n''']\n").unwrap();
    std::fs::write(dir.join("sprites/spr_bat.toml"), "pixels = ['''\nw\n''']\n").unwrap();
    std::fs::write(dir.join("rooms/r.toml"), "width = 320\nheight = 240\n").unwrap();
    for (name, data) in files {
        let p = dir.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, data).unwrap();
    }
    dir
}

fn start(dir: &Path) -> Engine {
    let mut e = Engine::load(dir).unwrap_or_else(|err| panic!("{err}"));
    e.save_dir = dir.join("saves");
    e.start();
    e
}

fn load_err(files: &[(&str, &str)]) -> String {
    let dir = project(files);
    let r = Engine::load(&dir).err().expect("the project should not load");
    let _ = std::fs::remove_dir_all(&dir);
    r
}

fn g(e: &Engine, name: &str) -> Value {
    e.globals.get(&intern(name)).cloned().unwrap_or(Value::Null)
}

fn s(e: &Engine, name: &str) -> String {
    format!("{:?}", g(e, name))
}

fn num(e: &Engine, name: &str) -> f64 {
    g(e, name).as_num().unwrap_or_else(|| panic!("global.{name} = {}", g(e, name)))
}

fn run(e: &mut Engine, code: &str) {
    e.run_code(code).unwrap_or_else(|err| panic!("{err}"));
}

const CARDS: &str = r#"# The starting cards
[[card]]
id = "slash"
name = "Tajo"
cost = 1
kind = "attack"
sprite = "spr_slash"
effects = [{ type = "damage", amount = 6 }]

[[card]]
id = "guard"      # a defence card
name = "Guardia"
kind = "skill"
effects = [{ type = "block", amount = 5 }, { type = "draw", count = 1 }]
upgrade = "slash"
"#;

const SCHEMA: &str = r#"
[fields]
name = { type = "string", required = true }
cost = { type = "int", default = 1, min = 0, max = 9 }
kind = { type = "enum", values = ["attack", "skill", "power"] }
sprite = { type = "sprite" }
upgrade = { type = "ref", table = "cards" }
effects = { type = "list", of = { type = "struct", tag = "type", variants = { damage = { amount = "int" }, block = { amount = "int" }, draw = { count = "int" } } } }
"#;

const ENEMIES_JSON: &str = r#"[
  {"id": "bat", "hp": 12, "sprite": "spr_bat", "intents": ["bite", "bite", "screech"]},
  {"id": "slime", "hp": 30, "sprite": "spr_bat", "intents": ["slam"]}
]"#;

const RELICS_CSV: &str = "id,name,price,rare,tags\nring,\"Anillo, de oro\",50,false,\"[\"\"gold\"\"]\"\nskull,Calavera,120,true,[]\n";

#[test]
fn tables_load_with_schema_defaults_assets_and_refs() {
    let dir = project(&[("data/cards.toml", CARDS), ("data/cards.schema.toml", SCHEMA), ("data/enemies.json", ENEMIES_JSON), ("data/relics.csv", RELICS_CSV)]);
    let mut e = start(&dir);
    run(
        &mut e,
        r#"
        tables = data_tables()
        c = data_get("cards", "slash")
        g = data_get("cards", "guard")
        guard_cost = g.cost
        guard_eff = g.effects
        spr_ok = c.sprite == spr_slash
        ids = data_ids("cards")
        bat_hp = data_get("enemies", "bat", "hp")
        intents = data_get("enemies", "bat").intents
        ring = data_get("relics", "ring")
        skull_rare = data_get("relics", "skull", "rare")
        attacks = data_find("cards", "kind", "attack")
        cheap = data_find("enemies", function(r) { return r.hp < 20 })
        ex = [data_exists("cards", "slash"), data_exists("cards", "nope"), data_exists("nope"), data_exists("cards")]
        n = data_count("enemies")
        // a copy: changing it does not change the table
        c.cost = 99
        again = data_get("cards", "slash").cost
        first_key = keys(data_get("cards", "slash"))[0]
    "#,
    );
    assert_eq!(s(&e, "tables"), r#"["cards", "enemies", "relics"]"#);
    assert_eq!(num(&e, "guard_cost"), 1.0, "default from the schema");
    assert_eq!(s(&e, "guard_eff"), r#"[{type: "block", amount: 5}, {type: "draw", count: 1}]"#);
    assert_eq!(s(&e, "spr_ok"), "true", "sprite fields become the sprite asset");
    assert_eq!(s(&e, "ids"), r#"["slash", "guard"]"#);
    assert_eq!(num(&e, "bat_hp"), 12.0);
    assert_eq!(s(&e, "intents"), r#"["bite", "bite", "screech"]"#);
    assert_eq!(s(&e, "ring"), r#"{id: "ring", name: "Anillo, de oro", price: 50, rare: false, tags: ["gold"]}"#);
    assert_eq!(s(&e, "skull_rare"), "true");
    assert_eq!(s(&e, "attacks").matches("id: \"slash\"").count(), 1);
    assert_eq!(s(&e, "cheap").matches("id: ").count(), 1);
    assert_eq!(s(&e, "ex"), "[true, false, false, true]");
    assert_eq!(num(&e, "n"), 2.0);
    assert_eq!(num(&e, "again"), 1.0, "data_get returns copies");
    assert_eq!(s(&e, "first_key"), "\"id\"");
    // a typo in an id or a table name explains itself
    let err = e.run_code("x = data_get(\"cards\", \"slsh\")").unwrap_err();
    assert!(err.contains("'slsh' no es un id de la tabla 'cards'") && err.contains("¿quizás 'slash'?"), "{err}");
    let err = e.run_code("x = data_get(\"crads\", \"slash\")").unwrap_err();
    assert!(err.contains("no hay ninguna tabla 'crads'") && err.contains("cards"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn each_error_has_file_and_line() {
    let cases: Vec<(Vec<(&str, &str)>, &str)> = vec![
        (vec![("data/cards.toml", "[[card]]\nid = \"a\"\ncost = \n")], "data/cards.toml:3: TOML no válido"),
        (vec![("data/cards.toml", "[[card]]\nid = \"a\"\n\n[[card]]\nname = \"x\"\n")], "data/cards.toml:4: registro sin id"),
        (vec![("data/cards.toml", "[[card]]\nid = \"a\"\n\n[[card]]\nid = \"a\"\n")], "data/cards.toml:4: 'a': id repetido (ya está en la línea 1)"),
        (vec![("data/cards.toml", CARDS.replace("cost = 1", "cost = \"uno\"").leak()), ("data/cards.schema.toml", SCHEMA)], "data/cards.toml:5: 'slash': cost debe ser un entero, no texto"),
        (vec![("data/cards.toml", CARDS.replace("kind = \"attack\"", "kind = \"atack\"").leak()), ("data/cards.schema.toml", SCHEMA)], "data/cards.toml:6: 'slash': kind: 'atack' no es un valor válido"),
        (vec![("data/cards.toml", CARDS.replace("cost = 1", "cots = 1").leak()), ("data/cards.schema.toml", SCHEMA)], "data/cards.toml:5: 'slash': campo 'cots' no está en data/cards.schema.toml (¿quizás 'cost'?)"),
        (vec![("data/cards.toml", CARDS.replace("upgrade = \"slash\"", "upgrade = \"slahs\"").leak()), ("data/cards.schema.toml", SCHEMA)], "data/cards.toml:15: 'guard': upgrade: 'slahs' no es un id de la tabla 'cards' (¿quizás 'slash'?)"),
        (vec![("data/cards.toml", CARDS.replace("sprite = \"spr_slash\"", "sprite = \"spr_nope\"").leak()), ("data/cards.schema.toml", SCHEMA)], "data/cards.toml:7: 'slash': sprite: no hay ningún sprite llamado 'spr_nope'"),
        (vec![("data/cards.toml", CARDS.replace("name = \"Guardia\"\n", "").leak()), ("data/cards.schema.toml", SCHEMA)], "data/cards.toml:10: 'guard': falta el campo obligatorio 'name'"),
        (vec![("data/cards.toml", CARDS.replace("cost = 1", "cost = 12").leak()), ("data/cards.schema.toml", SCHEMA)], "data/cards.toml:5: 'slash': cost = 12 es mayor que el máximo 9"),
        (vec![("data/cards.toml", CARDS.replace("type = \"damage\"", "type = \"dmg\"").leak()), ("data/cards.schema.toml", SCHEMA)], "data/cards.toml:8: 'slash': effects[0]: type = 'dmg' no es uno de: block, damage, draw"),
        (vec![("data/cards.toml", CARDS.replace("amount = 6", "amount = 6, extra = 1").leak()), ("data/cards.schema.toml", SCHEMA)], "effects[0]: campo desconocido 'extra'"),
        (vec![("data/cards.toml", CARDS), ("data/cards.schema.toml", "[fields]\ncost = \"integr\"\n")], "data/cards.schema.toml:2: campo 'cost': tipo desconocido 'integr'"),
        (vec![("data/cards.toml", CARDS), ("data/cards.schema.toml", "[fields]\nup = { type = \"ref\", table = \"nope\" }\n"), ("data/x.toml", "[[x]]\nid = \"q\"\n")], ""),
        (vec![("data/enemies.json", "[\n {\"id\": \"a\"},\n {\"hp\": 3}\n]")], "data/enemies.json:3: registro sin id"),
        (vec![("data/enemies.json", "[\n {\"id\": \"a\",}\n]")], "data/enemies.json:2: JSON no válido"),
        (vec![("data/relics.csv", "id,price\na,10\nb,\"x\n"), ("data/relics.schema.toml", "[fields]\nprice = \"int\"\n")], "data/relics.csv:3: comillas sin cerrar"),
        (vec![("data/relics.csv", "id,price\na,10\nb,lots\n"), ("data/relics.schema.toml", "[fields]\nprice = \"int\"\n")], "data/relics.csv:3: campo 'price': 'lots' no es un entero"),
        (vec![("data/cards.toml", CARDS), ("data/cards.json", "[]")], "data/cards.toml:1: la tabla 'cards' ya está en data/cards.json"),
        (vec![("data/ghost.schema.toml", "[fields]\n")], "data/ghost.schema.toml:1: esquema sin tabla"),
        (vec![("data/cards.toml", "title = \"x\"\n[[card]]\nid = \"a\"\n")], "data/cards.toml:1: 'title' no es una lista de registros"),
    ];
    for (files, want) in cases {
        if want.is_empty() {
            // a ref to a table that does not exist: reported on the record using it
            let dir = project(&files);
            std::fs::write(dir.join("data/cards.toml"), "[[card]]\nid = \"a\"\nup = \"q\"\n").unwrap();
            let err = Engine::load(&dir).err().unwrap();
            assert!(err.contains("data/cards.toml:3: 'a': up: la tabla 'nope' no existe"), "{err}");
            let _ = std::fs::remove_dir_all(&dir);
            continue;
        }
        let err = load_err(&files);
        assert!(err.contains(want), "expected `{want}` in:\n{err}");
    }
}

#[test]
fn hot_reload_picks_up_changes_and_keeps_old_data_on_errors() {
    let dir = project(&[("data/cards.toml", CARDS), ("data/cards.schema.toml", SCHEMA)]);
    let mut e = start(&dir);
    run(&mut e, "v0 = data_version()\nc0 = data_get(\"cards\", \"slash\").cost");
    assert_eq!(num(&e, "c0"), 1.0);
    std::fs::write(dir.join("data/cards.toml"), CARDS.replace("cost = 1", "cost = 3")).unwrap();
    for _ in 0..31 {
        e.tick();
    }
    run(&mut e, "v1 = data_version()\nc1 = data_get(\"cards\", \"slash\").cost");
    assert_eq!(num(&e, "c1"), 3.0, "the run reloads changed tables");
    assert_eq!(num(&e, "v1"), 1.0);
    // a broken edit: old data stays, the error is reported once
    std::fs::write(dir.join("data/cards.toml"), CARDS.replace("cost = 1", "cost = \"x\"")).unwrap();
    for _ in 0..31 {
        e.tick();
    }
    run(&mut e, "c2 = data_get(\"cards\", \"slash\").cost");
    assert_eq!(num(&e, "c2"), 3.0);
    assert!(e.errors.iter().any(|m| m.contains("data/cards.toml:5") && m.contains("se siguen usando los datos anteriores")), "{:?}", e.errors);
    // data_reload() forces it
    std::fs::write(dir.join("data/cards.toml"), CARDS.replace("cost = 1", "cost = 7")).unwrap();
    run(&mut e, "ok = data_reload()\nc3 = data_get(\"cards\", \"slash\").cost");
    assert_eq!(s(&e, "ok"), "true");
    assert_eq!(num(&e, "c3"), 7.0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn decks_draw_move_shuffle_and_save() {
    let dir = project(&[]);
    let mut e = start(&dir);
    run(
        &mut e,
        r#"
        d = deck_create(["a", "b", "c"])
        deck_add(d, "z")                 // bottom
        deck_add(d, "top", deck_top)
        deck_add(d, "mid", 2)
        order = deck_to_array(d)
        first = deck_draw(d)
        two = deck_draw(d, 2)
        size = deck_size(d)
        peek = [deck_peek(d), deck_peek(d, -1), deck_peek(d, 99)]
        hand = deck_create()
        moved = deck_move(d, hand)
        moved2 = deck_move(d, hand, "z")
        nothing = deck_move(d, hand, "nothing")
        found = [deck_find(hand, "z"), deck_find(hand, "q")]
        removed = deck_remove(hand, "b")
        at = deck_remove_at(hand, 0)
        left = [deck_to_array(d), deck_to_array(hand)]
        empty_draw = deck_draw(deck_create())
        // move_all keeps the order
        disc = deck_create([1, 2, 3])
        pile = deck_create([0])
        n = deck_move_all(disc, pile)
        all_cards = [deck_to_array(pile), deck_size(disc)]
        deck_destroy(disc)
        alive = [deck_exists(disc), deck_exists(pile)]
    "#,
    );
    assert_eq!(s(&e, "order"), r#"["top", "a", "mid", "b", "c", "z"]"#);
    assert_eq!(s(&e, "first"), "\"top\"");
    assert_eq!(s(&e, "two"), r#"["a", "mid"]"#);
    assert_eq!(num(&e, "size"), 3.0);
    assert_eq!(s(&e, "peek"), r#"["b", "z", null]"#);
    assert_eq!(s(&e, "moved"), "\"b\"");
    assert_eq!(s(&e, "moved2"), "\"z\"");
    assert_eq!(s(&e, "nothing"), "null");
    assert_eq!(s(&e, "found"), "[1, -1]");
    assert_eq!(s(&e, "removed"), "true");
    assert_eq!(s(&e, "at"), "\"z\"");
    assert_eq!(s(&e, "left"), r#"[["c"], []]"#);
    assert_eq!(s(&e, "empty_draw"), "null");
    assert_eq!(num(&e, "n"), 3.0);
    assert_eq!(s(&e, "all_cards"), "[[0, 1, 2, 3], 0]");
    assert_eq!(s(&e, "alive"), "[false, true]");

    // seeded shuffles are reproducible and match rng_shuffle; save/load round trip
    run(
        &mut e,
        r#"
        cards = []
        for (var i = 0; i < 20; i++) { push(cards, {id: "c" + str(i), lvl: i % 3}) }
        r1 = rng_create(42)
        a = deck_create(cards)
        deck_shuffle(a, r1)
        r2 = rng_create(42)
        b = deck_create(cards)
        deck_shuffle(b, r2)
        r3 = rng_create(42)
        arr = copy(cards)
        rng_shuffle(r3, arr)
        same = to_json(deck_to_array(a)) == to_json(deck_to_array(b))
        same_as_rng = to_json(deck_to_array(a)) == to_json(arr)
        moved_any = to_json(deck_to_array(a)) != to_json(cards)
        // deck_random uses the stream too
        deck_add(a, {id: "curse"}, deck_random, r1)
        curse_at = -1
        for (var i = 0; i < deck_size(a); i++) { if deck_peek(a, i).id == "curse" { curse_at = i } }
        text = to_json({draw: deck_save(a), rng: rng_get_state(r1)})
        back = from_json(text)
        c = deck_load(back.draw)
        r4 = rng_create(1)
        rng_set_state(r4, back.rng)
        round = to_json(deck_to_array(c)) == to_json(deck_to_array(a))
        next_same = rng_random(r4) == rng_random(r1)
        // deck_load into an existing pile replaces its cards
        deck_load([{id: "x"}], b)
        b_cards = deck_to_array(b)
    "#,
    );
    assert_eq!(s(&e, "same"), "true");
    assert_eq!(s(&e, "same_as_rng"), "true");
    assert_eq!(s(&e, "moved_any"), "true");
    assert!(num(&e, "curse_at") >= 0.0);
    assert_eq!(s(&e, "round"), "true", "save/load keeps the order");
    assert_eq!(s(&e, "next_same"), "true", "the rng state travels with the save");
    assert_eq!(s(&e, "b_cards"), r#"[{id: "x"}]"#);
    let err = e.run_code("deck_size(999)").unwrap_err();
    assert!(err.contains("no es un mazo"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Runs `steps` game steps and returns global `log` as text.
fn steps(e: &mut Engine, n: u32) {
    for _ in 0..n {
        e.tick();
    }
}

const ACTIONS: &str = r#"
global.log = []
function act(name, n) {
    push(global.log, name + "+" + str(global.t))
    wait(n)
    push(global.log, name + "-" + str(global.t))
}
"#;

#[test]
fn action_queue_order_parallel_speed_and_instant() {
    let dir = project(&[("scripts/a.ms", ACTIONS), ("objects.ms", "object Clock { on step { global.t += 1 } }\n"), ("rooms/r.toml", "width = 320\nheight = 240\n[[layer]]\nname = \"i\"\ninstances = [{ object = \"Clock\", x = 0, y = 0 }]\n")]);
    let mut e = start(&dir);
    run(
        &mut e,
        r#"
        global.t = 0
        q = actionq_create()
        actionq_push(q, act, "a", 3)
        actionq_push_wait(q, 2 / 60)  // 2 steps
        actionq_push_parallel(q, [[act, "p", 2], [act, "q", 4]])
        actionq_push(q, act, "b", 1)
        busy0 = actionq_busy(q)
        size0 = actionq_size(q)
    "#,
    );
    assert_eq!(s(&e, "busy0"), "true");
    assert_eq!(num(&e, "size0"), 4.0);
    steps(&mut e, 20);
    // first update at t = 1: a 1..4, wait 2 (4..6), p and q together from 6 (p ends 8, q ends 10), b 10..11
    assert_eq!(s(&e, "log"), r#"["a+1", "a-4", "p+6", "q+6", "p-8", "q-10", "b+10", "b-11"]"#);
    run(&mut e, "idle = actionq_busy(q)");
    assert_eq!(s(&e, "idle"), "false");

    // ×2: the same actions in half the steps
    run(&mut e, "global.log = []\nglobal.t = 0\nactionq_set_speed(q, 2)\nactionq_push(q, act, \"a\", 6)\nactionq_push(q, act, \"b\", 4)\nsp = actionq_get_speed(q)");
    assert_eq!(num(&e, "sp"), 2.0);
    steps(&mut e, 4);
    run(&mut e, "mid = actionq_busy(q)");
    assert_eq!(s(&e, "mid"), "true");
    steps(&mut e, 2);
    assert_eq!(s(&e, "log"), r#"["a+1", "a-4", "b+4", "b-6"]"#);
    run(&mut e, "done2 = actionq_busy(q)");
    assert_eq!(s(&e, "done2"), "false");

    // ×0.5 takes twice as long; 0 pauses
    run(&mut e, "global.log = []\nglobal.t = 0\nactionq_set_speed(q, 0.5)\nactionq_push(q, act, \"s\", 2)");
    steps(&mut e, 6);
    assert_eq!(s(&e, "log"), r#"["s+2", "s-6"]"#);
    run(&mut e, "global.log = []\nactionq_set_speed(q, 0)\nactionq_push(q, act, \"z\", 1)");
    steps(&mut e, 10);
    assert_eq!(s(&e, "log"), "[]", "paused");

    // instant: everything right now, in order, waits included
    run(&mut e, "global.log = []\nglobal.t = 100\nactionq_set_speed(q, infinity)");
    steps(&mut e, 1);
    assert_eq!(s(&e, "log"), r#"["z+101", "z-101"]"#);
    run(&mut e, "global.log = []\nactionq_set_speed(q, 1)\nactionq_push(q, act, \"k\", 50)\nactionq_push(q, act, \"l\", 50)\nactionq_finish(q)\nafter = actionq_busy(q)");
    assert_eq!(s(&e, "log"), r#"["k+101", "k-101", "l+101", "l-101"]"#);
    assert_eq!(s(&e, "after"), "false");

    // push_next from inside an action: reactions play right after it, in order
    run(
        &mut e,
        r#"
        global.log = []
        actionq_push(q, function() { push(global.log, "hit"); actionq_push_next(q, act, "r1", 0); actionq_push_next(q, act, "r2", 0) })
        actionq_push(q, act, "after", 0)
        actionq_finish(q)
        "#,
    );
    assert_eq!(s(&e, "log"), r#"["hit", "r1+101", "r1-101", "r2+101", "r2-101", "after+101", "after-101"]"#);

    // clear drops pending and playing actions; manual queues only move with actionq_update
    run(&mut e, "global.log = []\nactionq_push(q, act, \"c\", 5)\nactionq_push(q, act, \"d\", 5)");
    steps(&mut e, 2);
    run(&mut e, "actionq_clear(q)\ncleared = actionq_busy(q)\nm = actionq_create(false)\nactionq_push(m, act, \"m\", 2)");
    steps(&mut e, 10);
    assert_eq!(s(&e, "log"), r#"["c+102"]"#, "cleared mid-action: c never ends, d never starts; m waits for actionq_update");
    assert_eq!(s(&e, "cleared"), "false");
    run(&mut e, "actionq_update(m)\nactionq_update(m)\nactionq_update(m)");
    assert_eq!(s(&e, "log"), r#"["c+102", "m+113", "m-113"]"#);
    let err = e.run_code("actionq_set_speed(q, -1)").unwrap_err();
    assert!(err.contains("velocidad"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn action_owned_by_a_destroyed_instance_is_dropped() {
    let dir = project(&[(
        "objects.ms",
        r#"
        object Card {
            on create {
                global.q = actionq_create()
                actionq_push(global.q, function() { global.ran += 1; wait(5); global.ran += 10 })
                actionq_push(global.q, function() { global.ran += 100 })
            }
        }
        "#,
    )]);
    let mut e = start(&dir);
    run(&mut e, "global.ran = 0\nc = spawn(Card, 0, 0)");
    steps(&mut e, 2);
    run(&mut e, "destroy(c)");
    steps(&mut e, 10);
    run(&mut e, "busy = actionq_busy(global.q)");
    assert_eq!(num(&e, "ran"), 1.0, "the dead card's actions never finish");
    assert_eq!(s(&e, "busy"), "false");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dispatch_runs_effect_handlers_with_preview() {
    let dir = project(&[("data/cards.toml", CARDS), ("data/cards.schema.toml", SCHEMA)]);
    let mut e = start(&dir);
    run(
        &mut e,
        r#"
        global.enemy = {hp: 20, block: 0}
        global.drawn = 0
        apply = {
            damage: function(eff, target) { target.hp -= eff.amount; return eff.amount },
            block: function(eff, target) { return 0 },
            draw: function(eff, target) { global.drawn += eff.count; return 0 },
        }
        // the same effects, only computed: a projection for the UI
        preview = {
            damage: function(eff, target) { return "-" + str(eff.amount) },
            fallback: function(eff, target) { return "" },
        }
        shown = data_dispatch(data_get("cards", "slash").effects, preview, global.enemy)
        hp_before = global.enemy.hp
        res = data_dispatch(data_get("cards", "slash").effects, apply, global.enemy)
        one = data_dispatch(data_get("cards", "guard").effects[1], apply, global.enemy)
        hp_after = global.enemy.hp
    "#,
    );
    assert_eq!(s(&e, "shown"), r#"["-6"]"#);
    assert_eq!(num(&e, "hp_before"), 20.0, "previews change nothing");
    assert_eq!(s(&e, "res"), "[6]");
    assert_eq!(num(&e, "hp_after"), 14.0);
    assert_eq!(num(&e, "drawn"), 1.0);
    let err = e.run_code("data_dispatch({type: \"damag\"}, apply)").unwrap_err();
    assert!(err.contains("no hay handler para el efecto 'damag'") && err.contains("¿quizás 'damage'?"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn validation_api_for_tools() {
    // the editor validates unsaved rows with the same functions
    let schema = modengine::datatable::parse_schema("data/cards.schema.toml", SCHEMA).unwrap();
    assert_eq!(schema.root.fields.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(), vec!["name", "cost", "kind", "sprite", "upgrade", "effects"], "file order");
    let mut t = modengine::datatable::parse_table("cards", "data/cards.toml", CARDS, modengine::datatable::Format::Toml, Some(&schema)).unwrap();
    t.schema = Some(schema);
    assert_eq!(t.record_key, "card");
    assert_eq!(t.records[1].line, 10);
    t.records[0].fields.retain(|f| f.0 != "name");
    let d = modengine::datatable::validate(&[t], &|_, n| n == "spr_slash");
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!((d[0].record, d[0].field.as_deref()), (Some(0), Some("name")));
}
