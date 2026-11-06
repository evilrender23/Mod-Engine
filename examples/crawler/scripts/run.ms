// The run: everything a save needs (deck, HP, floor, rng states) and the floor model.
// Presentation (camera, HUD, animations) lives in the objects; this file is pure model.
//
// State machine of a run (global.run.state):
//   explore -> combat -> explore ... -> descend -> explore (next floor) ... -> won | lost
//   explore -> chest -> explore
//
// Debug jumps (the wiki's lesson: jump straight to any state while testing). From the
// terminal:  modengine run examples/crawler --room crypt --exec-at '10:global.debug = "combat"'
//   "combat"   a fight with the first group of this floor, right in front of you
//   "combat:N" a fight with group N of this floor (data/floors.toml)
//   "boss"     floor 3 and the boss fight
//   "floor:N"  generate floor N (1-3) and stand on its start
//   "look:T"   stand in front of the first event of type T (enemy, chest, fountain, stairs, boss)
//   "chest"    open a chest choice   "fountain" heal   "descend" take the stairs
//   "won" / "lost"  the end screens   "win_combat" kills every enemy of the current fight

global.run = null
global.floor = null
global.debug = null
global.MAX_FLOOR = 3

// Separate random streams: the dungeon only depends on the seed; fights and loot never
// shift it (a run can be reproduced and saved exactly).
global.rng_gen = rng_create(1)
global.rng_cb = rng_create(2)
global.rng_loot = rng_create(3)

global.settings = load_data("ajustes", { lang: "es" })
loc_set_language(global.settings.lang)

function new_run(seed) {
    rng_seed(global.rng_gen, seed)
    rng_seed(global.rng_cb, str(seed) + "-combate")
    rng_seed(global.rng_loot, str(seed) + "-botin")
    global.run = {
        seed: seed,
        floor: 1,
        hp: 50,
        max_hp: 50,
        uid: 0,
        deck: [],
        state: "explore",
        gen_state: "",
        resume: false,
        stats: { kills: 0, turns: 0, cards: 0 },
    }
    for (id in ["tajo", "tajo", "tajo", "tajo", "tajo", "guardia", "guardia", "guardia", "guardia", "remate"]) {
        add_card(id)
    }
}

function add_card(id) {
    global.run.uid += 1
    push(global.run.deck, { uid: global.run.uid, id: id })
}

// Saved at the start of every floor (before generating it, so continuing rebuilds the same map).
function save_run() {
    var r = global.run
    return save_data("partida", {
        seed: r.seed, floor: r.floor, hp: r.hp, max_hp: r.max_hp, uid: r.uid, deck: r.deck,
        stats: r.stats, gen_state: r.gen_state,
        cb_state: rng_get_state(global.rng_cb), loot_state: rng_get_state(global.rng_loot),
    })
}

function load_run() {
    var s = load_data("partida", null)
    if s == null { return false }
    global.run = {
        seed: s.seed, floor: s.floor, hp: s.hp, max_hp: s.max_hp, uid: s.uid, deck: s.deck,
        stats: s.stats, state: "explore", gen_state: s.gen_state, resume: true,
    }
    rng_set_state(global.rng_gen, s.gen_state)
    rng_set_state(global.rng_cb, s.cb_state)
    rng_set_state(global.rng_loot, s.loot_state)
    return true
}

function end_run() {
    save_delete("partida")
}

// ---------------------------------------------------------------- floors

// Generates floor `n` with the dungeon stream. Returns the floor model:
// { n, def, gen, w, h, map (array [y][x] for fpview: 0 open, 1 wall, 2 torch wall, 3 niche wall),
//   events: [{ type, x, y, group, done, t }], start, exit, visited, doors, corridors }
function generate_floor(n) {
    var def = data_get("floors", "f" + str(n))
    if global.run.resume {
        global.run.resume = false          // the saved state already points before this floor
    } else {
        global.run.gen_state = rng_get_state(global.rng_gen)
    }
    var gen = dungeon_create(def.dungeon)
    dungeon_generate(gen, null, global.rng_gen)
    var g = dungeon_grid(gen)
    var w = dungeon_width(gen)
    var h = dungeon_height(gen)
    var map = []
    var visited = []
    var corridors = []
    var doors = []
    for (var y = 0; y < h; y++) {
        var row = []
        for (var x = 0; x < w; x++) {
            var c = g[# x, y]
            if c == 1 {
                // wall variants: mostly plain, some torches and skull niches
                var r = rng_random(global.rng_gen)
                push(row, r < 0.07 ? 2 : (r < 0.11 ? 3 : 1))
            } else {
                push(row, 0)
                if c == 3 { push(corridors, [x, y]) }
                if c == 2 { push(doors, [x, y]) }
            }
        }
        push(map, row)
        push(visited, array(w, 0))
    }
    var events = []
    for (ev in dungeon_events(gen)) {
        var e = { type: ev.type, x: ev.x, y: ev.y, done: false, t: ev.t, group: [] }
        if ev.type == "enemy" {
            // deeper events pick from later groups
            var k = min(len(def.groups) - 1, floor(ev.t * len(def.groups) * 0.7 + rng_irandom(global.rng_gen, 2)))
            e.group = def.groups[k]
        }
        if ev.type == "boss" { e.group = [def.boss] }
        push(events, e)
    }
    var ex = dungeon_exit(gen)
    if def.boss == null {
        push(events, { type: "stairs", x: ex.x, y: ex.y, done: false, t: 1, group: [] })
    }
    var s = dungeon_start(gen)
    var path = dungeon_main_path(gen)
    global.floor = {
        n: n, def: def, gen: gen, w: w, h: h, map: map, events: events, visited: visited,
        start: s, goal: ex, doors: doors, corridors: corridors,
        // the first step of the way to the exit: the player starts looking there
        towards: len(path) > 1 ? path[1] : [s.x + 1, s.y],
    }
    dungeon_destroy(gen)
    return global.floor
}

function event_at(x, y) {
    if global.floor == null { return null }
    for (e in global.floor.events) {
        if !e.done and e.x == x and e.y == y { return e }
    }
    return null
}

// Is the cell open (no wall) in the current floor?
function cell_open(x, y) {
    var f = global.floor
    if x < 0 or y < 0 or x >= f.w or y >= f.h { return false }
    return f.map[y][x] == 0
}

// Three different cards for a chest (loot stream: never shifts the dungeon or the fights).
function chest_offer() {
    var pool = []
    for (id in data_ids("cards")) {
        var r = data_get("cards", id, "rarity")
        if r == "common" { push(pool, id); push(pool, id) }
        if r == "rare" { push(pool, id) }
    }
    var out = []
    while len(out) < 3 {
        var id = rng_choose(global.rng_loot, pool)
        if !contains(out, id) { push(out, id) }
    }
    return out
}

// The settings (language) live in their own save, separate from the run.
function save_settings() {
    save_data("ajustes", global.settings)
}
