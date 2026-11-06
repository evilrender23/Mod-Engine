//! ModScript features that need the engine: closures with the array builtins,
//! `super` / `inherited()` in methods, for-in over instances, static analysis
//! (`modengine::lint`) and asset name clash messages.

use modengine::lint::{self, Severity};
use modengine::Engine;
use modscript::intern;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static N: AtomicU32 = AtomicU32::new(0);

fn project(script: &str, room: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("modengine-lang-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("rooms")).unwrap();
    std::fs::create_dir_all(dir.join("sprites")).unwrap();
    std::fs::write(dir.join("game.toml"), "[game]\nname = \"t\"\n\n[palette]\n\"w\" = \"#ffffff\"\n").unwrap();
    std::fs::write(dir.join("sprites/dot.toml"), "pixels = ['''\nw\n''']\n").unwrap();
    std::fs::write(dir.join("main.ms"), script).unwrap();
    std::fs::write(dir.join("rooms/r.toml"), room).unwrap();
    dir
}

fn room(objects: &[&str]) -> String {
    let insts: Vec<String> = objects.iter().enumerate().map(|(i, o)| format!("{{ object = \"{o}\", x = {}, y = 8 }}", i * 16)).collect();
    format!("width = 64\nheight = 64\n[[layer]]\ntype = \"instances\"\ninstances = [ {} ]\n", insts.join(", "))
}

fn run(script: &str, objects: &[&str], steps: u32) -> Engine {
    let mut e = Engine::load(&project(script, &room(objects))).unwrap_or_else(|e| panic!("{e}"));
    e.start();
    for _ in 0..steps {
        e.tick();
    }
    assert!(e.errors.is_empty(), "runtime errors: {:?}", e.errors);
    e
}

fn global(e: &Engine, n: &str) -> String {
    e.globals.get(&intern(n)).map(|v| format!("{v:?}")).unwrap_or_else(|| "<unset>".into())
}

#[test]
fn closures_see_locals_in_builtins() {
    let src = r#"
object Main {
    on create {
        var min_hp = 3
        var factor = 10
        var hps = [5, 1, 4, 2]
        global.big = filter(hps, function(h) { return h >= min_hp })
        global.scaled = map(hps, function(h) { return h * factor })
        var target = 3
        var sorted = copy(hps)
        sort(sorted, function(a, b) { return abs(a - target) - abs(b - target) })
        global.closest = sorted
        var total = 0
        map(hps, function(h) { total += h })
        global.total = total
        // a method stored on the instance keeps the local it captured
        var bonus = 7
        boost = function(n) { return n + bonus + x }
    }
    on step { global.boosted = boost(1) }
}
"#;
    let e = run(src, &["Main"], 1);
    assert_eq!(global(&e, "big"), "[5, 4]");
    assert_eq!(global(&e, "scaled"), "[50, 10, 40, 20]");
    assert_eq!(global(&e, "closest"), "[4, 2, 5, 1]", "stable sort by distance to the local `target`");
    assert_eq!(global(&e, "total"), "12");
    assert_eq!(global(&e, "boosted"), "8", "1 + bonus 7 + x 0");
}

const HIERARCHY: &str = r#"
object A {
    function hit(n) { global.log += "A" + n + ";"; return n }
    function name() { return "A" }
    on create { global.log = "" }
}
object B : A {
    function hit(n) { global.log += "B;"; return inherited() + 1 }
    function name() { return "B<" + super.name() + ">" }
}
object C : B {
    function hit(n) { global.log += "C;"; return super.hit(n * 2) + 100 }
    function name() { return "C<" + super.name() + ">" }
    on step {
        global.r = hit(5)
        global.n = name()
    }
}
object D : B {
    // no hit of its own: B's version runs, and its inherited() still reaches A
    on step { global.d = hit(1) }
}
"#;

#[test]
fn super_and_inherited_in_methods_multi_level() {
    let e = run(HIERARCHY, &["C"], 1);
    assert_eq!(global(&e, "log"), "\"C;B;A10;\"");
    assert_eq!(global(&e, "r"), "111", "A returns 10, B adds 1, C adds 100");
    assert_eq!(global(&e, "n"), "\"C<B<A>>\"");
    let e = run(HIERARCHY, &["D"], 1);
    assert_eq!(global(&e, "d"), "2");
}

#[test]
fn inherited_in_events_still_works_and_methods_called_from_events() {
    let src = r#"
object P {
    on step { global.ev += "P" }
    function f() { global.ev += "pf" }
}
object K : P {
    function f() { inherited(); global.ev += "kf" }
    on create { global.ev = "" }
    on step { f(); inherited(); global.ev += "K" }
}
"#;
    let e = run(src, &["K"], 1);
    // inherited() inside the method f runs P.f, not P's step event
    assert_eq!(global(&e, "ev"), "\"pfkfPK\"");
}

#[test]
fn inherited_without_parent_method_returns_null() {
    let src = "object A {}\nobject B : A { function f(n) { return inherited() ?? \"nada\" }\n on step { global.r = f(1) } }\n";
    let e = run(src, &["B"], 1);
    assert_eq!(global(&e, "r"), "\"nada\"");
}

#[test]
fn lint_increment_alone_does_not_define_a_variable() {
    let src = "object A {\n on step { contador += 1 }\n}\n";
    let e = Engine::load(&project(src, &room(&["A"]))).unwrap();
    let found = lint::check(&e);
    assert!(found.iter().any(|f| f.line == 2 && f.msg.contains("'contador'")), "{found:?}");
    let src = "object A {\n contador = 0\n on step { contador += 1 }\n}\n";
    let e = Engine::load(&project(src, &room(&["A"]))).unwrap();
    assert!(lint::check(&e).is_empty());
}

#[test]
fn super_without_parent_method_is_an_error() {
    let src = "object A { function f() { return super.g() }\n on step { f() } }\n";
    let mut e = Engine::load(&project(src, &room(&["A"]))).unwrap();
    e.start();
    e.tick();
    assert!(e.errors.iter().any(|m| m.contains("super.g")), "{:?}", e.errors);
    let found = lint::check(&e);
    assert!(found.iter().any(|f| f.severity == Severity::Error && f.msg.contains("super.g") && f.line == 1), "{found:?}");
}

#[test]
fn for_in_over_instances_and_struct() {
    let src = r#"
object Coin { value = 2 }
object Main {
    on step {
        var sum = 0
        for (c in Coin) { sum += c.value; if sum >= 4 destroy(c) }
        global.sum = sum
        var n = 0
        for (c in Coin) n += 1
        global.left = n
        var s = { a: 1, b: 2 }
        var ks = ""
        for (k, v in s) ks += k + v
        global.ks = ks
    }
}
"#;
    let e = run(src, &["Main", "Coin", "Coin", "Coin"], 1);
    assert_eq!(global(&e, "sum"), "6");
    assert_eq!(global(&e, "left"), "1", "destroyed instances are skipped");
    assert_eq!(global(&e, "ks"), "\"a1b2\"");
}

// ------------------------------------------------------------------ static analysis

const LINT_SRC: &str = r#"
object Enemy {
    hp = 3
    function hurt(n) { hp -= n; if hp <= 0 destroy() }
}
object Player {
    speed = 4
    target = null
    on create {
        var lista = [1, 2, 3]
        var umbral = 2
        var grandes = filter(lista, function(v) { return v > umbral })
        for (k, v in { a: 1 }) show_debug_message(k + str(v))
    }
    on step {
        x += sped
        Enemy.hurt(1)
        Enemy.hurtt(1)
        spwan(0, 0, Enemy)
        sprite_index = dto
        target = nearest(Enemy)
        var d = target.hp
        var e2 = nearest(Enemy)
        if exists(e2) { d = e2.hp }
        var victim = find(Enemy, 0)
        destroy(victim)
        d = victim.hp
        global.score += 1
        show_debug_message(global.scroe)
        with (Enemy) { hurt(1) }
    }
    on draw {
        if exists(target) draw_text(0, 0, str(target.hp))
        var p = nearest(Enemy)
        draw_text(0, 10, str(p.hp))
        var q = nearest(Enemy)
        if q != null draw_text(0, 20, str(q.hp))
    }
}
global.score = 0
"#;

#[test]
fn lint_finds_unknown_names_with_suggestions() {
    let e = Engine::load(&project(LINT_SRC, &room(&["Player"]))).unwrap_or_else(|e| panic!("{e}"));
    let found = lint::check(&e);
    let text: Vec<String> = found.iter().map(|f| f.to_string()).collect();
    let has = |line: u32, sev: Severity, needle: &str| found.iter().any(|f| f.line == line && f.severity == sev && f.msg.contains(needle));
    assert!(has(16, Severity::Warning, "'sped'") && has(16, Severity::Warning, "'speed'"), "{text:#?}");
    assert!(has(18, Severity::Error, "no tiene el método 'hurtt'") && has(18, Severity::Error, "'hurt'"), "{text:#?}");
    assert!(has(19, Severity::Error, "'spwan'") && has(19, Severity::Error, "'spawn'"), "{text:#?}");
    assert!(has(20, Severity::Warning, "asset o variable desconocida 'dto'") && has(20, Severity::Warning, "'dot'"), "{text:#?}");
    assert!(has(22, Severity::Warning, "'target.hp'"), "unguarded stored query: {text:#?}");
    assert!(has(27, Severity::Warning, "después de destroy(victim)"), "{text:#?}");
    assert!(has(29, Severity::Warning, "'global.scroe'") && has(29, Severity::Warning, "'score'"), "{text:#?}");
    // no false positives: closures, for-in vars, guarded refs, methods, with blocks, known globals
    assert!(has(35, Severity::Warning, "'p.hp'"), "unguarded local query: {text:#?}");
    let allowed = [16, 18, 19, 20, 22, 27, 29, 35];
    let extra: Vec<&String> = found.iter().zip(&text).filter(|(f, _)| !allowed.contains(&f.line)).map(|(_, t)| t).collect();
    assert!(extra.is_empty(), "unexpected findings: {extra:#?}");
    assert!(text[0].starts_with("main.ms:16: aviso: "), "{}", text[0]);
}

#[test]
fn lint_is_clean_on_correct_code() {
    let e = run(HIERARCHY, &["C"], 0);
    let found = lint::check(&e);
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn asset_clash_names_both_files() {
    let dir = project("object Main {}\n", &room(&["Main"]));
    std::fs::write(dir.join("rooms/dot.toml"), "width = 8\nheight = 8\n").unwrap();
    let err = Engine::load(&dir).err().expect("clash must fail");
    assert!(err.contains("'dot'") && err.contains("sprites/dot.toml") && err.contains("rooms/dot.toml"), "{err}");
    let dir = project("object dot {}\n", &room(&[]));
    let err = Engine::load(&dir).err().expect("clash must fail");
    assert!(err.contains("sprites/dot.toml") && err.contains("main.ms:1"), "{err}");
}
