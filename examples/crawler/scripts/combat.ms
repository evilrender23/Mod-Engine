// Combat: the model resolves every play instantly (HP, block, deaths, draws) and pushes
// presentation actions to an action queue (global.q). The view (CombatUI, CardView and the
// billboards drawn by Explorer) only shows what those actions have revealed: shown_hp,
// the visible hand... So the speed (x1/x2/x4/instant) never changes the result, and the
// logic can be tested without a window (`modengine check`).
//
// Combat states (global.cb.state): player -> enemy -> player ... -> won | lost

global.cb = null
global.q = actionq_create()
global.SPEEDS = [1, 2, 4, infinity]
global.speed_i = 0
global.HAND_MAX = 7
// What the HUD shows of the player (moved by the actions, not by the model)
global.pv = { hp: 0, block: 0, mana: 0, combo: 0, combo_pop: 1, hurt: 0, str: 0, turn: 0 }

function set_speed(i) {
    global.speed_i = i mod len(global.SPEEDS)
    var s = global.SPEEDS[global.speed_i]
    actionq_set_speed(global.q, s)
    tween_set_speed(s == infinity ? 20 : s)
}

// ---------------------------------------------------------------- model

// group: enemy ids; (ex, ey): the cell they stand in; dir: the direction the player faces.
function start_combat(group, ev, ex, ey, dir) {
    var right = (dir + 3) mod 4
    var n = len(group)
    var enemies = []
    for (i, id in group) {
        var d = data_get("enemies", id)
        var off = n == 1 ? 0 : (i - (n - 1) / 2) * (n == 2 ? 0.44 : 0.34)
        var depth = d.boss ? 0.4 : 0.15        // the boss stands further back: it is big
        push(enemies, {
            id: id, def: d, hp: d.hp, max_hp: d.hp, block: 0, str: 0, intent: null, ii: 0, alive: true,
            wx: ex + grid_dir_dx(dir) * depth + grid_dir_dx(right) * off,
            wy: ey + grid_dir_dy(dir) * depth + grid_dir_dy(right) * off,
            // view state, moved by the actions
            shown_hp: d.hp, shown_block: 0, flash: 0, lunge: 0, fade: 1, pop: 1, gone: false, bb: -1,
        })
    }
    var cards = []
    for (c in global.run.deck) { push(cards, c) }
    var pile = deck_create(cards)
    deck_shuffle(pile, global.rng_cb)
    global.cb = {
        state: "player", enemies: enemies, ev: ev, turn: 0, mana: 0, max_mana: 3, block: 0, str: 0,
        combo: 0, boss: ev != null and ev.type == "boss",
        draw: pile, hand: deck_create(), discard: deck_create(), exhaust: deck_create(),
    }
    global.pv.hp = global.run.hp
    global.pv.block = 0
    global.pv.str = 0
    global.pv.combo = 0
    actionq_clear(global.q)
    start_turn()
}

function alive_enemies() {
    return filter(global.cb.enemies, function(e) { return e.alive })
}

// The first enemy still standing: target of "front" attacks.
function front_enemy() {
    for (e in global.cb.enemies) {
        if e.alive { return e }
    }
    return null
}

function choose_intents() {
    for (e in alive_enemies()) {
        var ints = e.def.intents
        if e.def.pattern {
            e.intent = ints[e.ii mod len(ints)]
            e.ii += 1
        } else {
            e.intent = rng_choose(global.rng_cb, ints)
        }
    }
}

function start_turn() {
    var cb = global.cb
    cb.turn += 1
    global.run.stats.turns += 1
    cb.mana = cb.max_mana
    cb.block = 0
    cb.combo = 0
    cb.state = "player"
    choose_intents()
    actionq_push(global.q, anim_new_turn, cb.turn, cb.mana)
    draw_cards(5)
}

function draw_cards(n) {
    var cb = global.cb
    repeat n {
        if deck_size(cb.hand) >= global.HAND_MAX { break }
        if deck_size(cb.draw) == 0 {
            if deck_size(cb.discard) == 0 { break }
            deck_move_all(cb.discard, cb.draw)
            deck_shuffle(cb.draw, global.rng_cb)
            actionq_push(global.q, anim_reshuffle)
        }
        var c = deck_move(cb.draw, cb.hand)
        actionq_push(global.q, anim_draw, c)
    }
}

function card_cost(c) {
    return data_get("cards", c.id, "cost")
}

function can_play(c) {
    return global.cb != null and global.cb.state == "player" and card_cost(c) <= global.cb.mana and deck_find(global.cb.hand, c) >= 0
}

// Plays a card from the hand. Returns false when it can't (not your turn, not enough mana).
function play_card(c) {
    var cb = global.cb
    if cb.state != "player" or deck_find(cb.hand, c) < 0 { return false }
    var def = data_get("cards", c.id)
    if def.cost > cb.mana { return false }
    cb.mana -= def.cost
    deck_move(cb.hand, def.exhaust ? cb.exhaust : cb.discard, c)
    global.run.stats.cards += 1
    actionq_push(global.q, anim_play, c, cb.mana)
    data_dispatch(def.effects, global.apply, { combo: cb.combo })
    cb.combo += 1
    actionq_push(global.q, anim_combo, cb.combo)
    check_end()
    return true
}

function check_end() {
    var cb = global.cb
    if cb.state == "won" or cb.state == "lost" { return }
    if global.run.hp <= 0 {
        cb.state = "lost"
        actionq_push(global.q, anim_defeat)
    } else if len(alive_enemies()) == 0 {
        cb.state = "won"
        actionq_push(global.q, anim_victory)
    }
}

// Damage to an enemy, through its block. Returns { dmg, blocked }.
function hit_enemy(e, amount) {
    var blocked = min(e.block, amount)
    e.block -= blocked
    var dmg = amount - blocked
    e.hp = max(0, e.hp - dmg)
    if e.hp <= 0 and e.alive {
        e.alive = false
        global.run.stats.kills += 1
    }
    return { dmg: dmg, blocked: blocked }
}

// Effect handlers (data_dispatch): one per effect type of data/cards.schema.toml.
global.apply = {
    damage: function(eff, ctx) {
        var cb = global.cb
        var amount = eff.amount + cb.str + ctx.combo + eff.per_combo * ctx.combo
        repeat eff.times {
            var targets = []
            if eff.target == "all" { targets = alive_enemies() }
            else if eff.target == "random" {
                var a = alive_enemies()
                if len(a) > 0 { targets = [rng_choose(global.rng_cb, a)] }
            } else {
                var f = front_enemy()
                if f != null { targets = [f] }
            }
            if len(targets) == 0 { break }
            var hits = []
            for (e in targets) {
                var r = hit_enemy(e, amount)
                push(hits, [anim_hit, e, r.dmg, r.blocked, e.hp, e.block, !e.alive])
            }
            if len(hits) == 1 { actionq_push(global.q, hits[0][0], hits[0][1], hits[0][2], hits[0][3], hits[0][4], hits[0][5], hits[0][6]) }
            else { actionq_push_parallel(global.q, hits) }
        }
    },
    block: function(eff, ctx) {
        global.cb.block += eff.amount
        actionq_push(global.q, anim_player_block, global.cb.block, eff.amount)
    },
    draw: function(eff, ctx) { draw_cards(eff.count) },
    mana: function(eff, ctx) {
        global.cb.mana += eff.amount
        actionq_push(global.q, anim_mana, global.cb.mana)
    },
    strength: function(eff, ctx) {
        global.cb.str += eff.amount
        actionq_push(global.q, anim_player_str, global.cb.str)
    },
    hurt_self: function(eff, ctx) {
        global.run.hp = max(0, global.run.hp - eff.amount)
        actionq_push(global.q, anim_player_hurt, eff.amount, 0, global.run.hp, global.cb.block)
    },
}

function end_turn() {
    var cb = global.cb
    if cb.state != "player" { return false }
    cb.state = "enemy"
    for (c in deck_to_array(cb.hand)) { actionq_push(global.q, anim_discard, c) }
    deck_move_all(cb.hand, cb.discard)
    for (e in alive_enemies()) {
        e.block = 0
        var it = e.intent
        if it.type == "attack" {
            repeat it.times {
                var amount = it.amount + e.str
                var blocked = min(cb.block, amount)
                cb.block -= blocked
                var dmg = amount - blocked
                global.run.hp = max(0, global.run.hp - dmg)
                actionq_push(global.q, anim_enemy_attack, e, dmg, blocked, global.run.hp, cb.block)
                if global.run.hp <= 0 { break }
            }
        } else if it.type == "block" {
            e.block += it.amount
            actionq_push(global.q, anim_enemy_block, e, e.block, it.amount)
        } else {
            e.str += it.amount
            actionq_push(global.q, anim_enemy_buff, e, it.amount)
        }
        if global.run.hp <= 0 { break }
    }
    check_end()
    if cb.state == "enemy" { start_turn() }
    return true
}

// Debug: wins the current fight at once.
function debug_win_combat() {
    if global.cb == null { return }
    for (e in alive_enemies()) {
        var r = hit_enemy(e, e.hp + e.block)
        actionq_push(global.q, anim_hit, e, r.dmg, r.blocked, 0, 0, true)
    }
    check_end()
}

// ---------------------------------------------------------------- projection and text

// What the card would do right now (same rules as global.apply, nothing changes).
// p = { dmg: [per enemy], hp: [...], blk: [...], block: n, random: bool }
global.preview = {
    damage: function(eff, p) {
        var cb = global.cb
        var amount = eff.amount + cb.str + cb.combo + eff.per_combo * cb.combo
        if eff.target == "random" {
            p.random = true
            return null
        }
        repeat eff.times {
            // the front: the first enemy the projection still sees alive (before this hit)
            var first = -1
            for (j, e2 in cb.enemies) {
                if p.hp[j] > 0 and first < 0 { first = j }
            }
            for (i, e in cb.enemies) {
                if p.hp[i] <= 0 { continue }
                if eff.target == "all" or i == first {
                    var b = min(p.blk[i], amount)
                    p.blk[i] -= b
                    p.dmg[i] += min(p.hp[i], amount - b)
                    p.hp[i] -= amount - b
                }
            }
        }
        return null
    },
    block: function(eff, p) { p.block += eff.amount },
    fallback: function(eff, p) {},
}

function project_card(c) {
    var cb = global.cb
    var p = { dmg: [], hp: [], blk: [], block: 0, random: false }
    for (e in cb.enemies) {
        push(p.dmg, 0)
        push(p.hp, e.alive ? e.hp : 0)
        push(p.blk, e.block)
    }
    data_dispatch(data_get("cards", c.id, "effects"), global.preview, p)
    return p
}

// Card text generated from its effects (never out of sync with the data).
global.describe = {
    damage: function(eff) {
        var t = eff.target == "all" ? loc("fx.damage_all", { n: eff.amount }) : (eff.target == "random" ? loc("fx.damage_random", { n: eff.amount, t: eff.times }) : loc("fx.damage", { n: eff.amount }))
        if eff.per_combo > 0 { t += "\n" + loc("fx.per_combo", { n: eff.per_combo }) }
        return t
    },
    block: function(eff) { return loc("fx.block", { n: eff.amount }) },
    draw: function(eff) { return loc("fx.draw", { n: eff.count }) },
    mana: function(eff) { return loc("fx.mana", { n: eff.amount }) },
    strength: function(eff) { return loc("fx.strength", { n: eff.amount }) },
    hurt_self: function(eff) { return loc("fx.hurt_self", { n: eff.amount }) },
}

function card_text(id) {
    var def = data_get("cards", id)
    var lines = data_dispatch(def.effects, global.describe)
    if def.exhaust { push(lines, loc("fx.exhaust")) }
    return join(lines, "\n")
}
