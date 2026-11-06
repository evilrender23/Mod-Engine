// Presentation actions of the combat, played one after another by global.q (see
// combat.ms). Each one is a coroutine: it may wait(steps). They only move what the player
// sees (shown_hp, card views, popups...): the model has already changed.

// Screen position over an enemy billboard (h = height in cells above the floor).
function enemy_screen(e, h) {
    var p = fpview_project(global.view_id, e.wx, e.wy, e.def.float + h)
    return p == null ? { x: 320, y: 120 } : p
}

function card_view_of(c) {
    for (v in global.views) {
        if exists(v) and v.card == c { return v }
    }
    return null
}

function relayout() {
    with CombatUI { layout() }
}

function anim_new_turn(turn, mana) {
    global.pv.turn = turn
    global.pv.mana = mana
    global.pv.block = 0
    global.pv.combo = 0
    with CombatUI { show_banner(loc("cb.turn", { n: turn })) }
    wait(12)
}

function anim_draw(c) {
    var v = spawn(CardView, 612, 300, -10, { card: c })
    push(global.views, v)
    relayout()
    play_sound(snd_card, { volume: 0.5, pitch: 0.9 + len(global.views) * 0.04 })
    wait(4)
}

function anim_reshuffle() {
    popup_text(580, 240, loc("cb.shuffle"), { gui: true, color: #4da6ff, font: fnt_ui })
    wait(6)
}

function anim_play(c, mana) {
    global.pv.mana = mana
    var v = card_view_of(c)
    if v != null {
        remove(global.views, index_of(global.views, v))
        v.played = true
        flash(v, #ffffeb, 0.15)
        pulse(v, 1.25, 0.18)
        tween_to(v, { x: 320, y: 150, alpha: 0 }, 0.3, "quad_in")
        relayout()
    }
    play_sound(snd_card)
    wait(8)
    if v != null and exists(v) { destroy(v) }
}

// Not enough mana: immediate feedback, outside the queue (nothing changed in the model).
function show_no_mana(c) {
    var v = card_view_of(c)
    if v != null {
        flash(v, #eb564b, 0.25)
        tween_to(v, { x: v.x + 5 }, 0.05, "linear", { yoyo: true, loops: 1 })
    }
    popup_text(320, 220, loc("cb.no_mana"), { gui: true, color: #eb564b, font: fnt_ui, scale: 1 })
    play_sound(snd_bump)
}

function anim_discard(c) {
    var v = card_view_of(c)
    if v != null {
        remove(global.views, index_of(global.views, v))
        v.played = true
        tween_to(v, { x: 36, y: 380, alpha: 0 }, 0.18, "quad_in")
    }
    wait(2)
}

function anim_combo(n) {
    global.pv.combo = n
    if n >= 2 {
        global.pv.combo_pop = 1.6
        tween_to(global.pv, { combo_pop: 1 }, 0.25, "back_out")
    }
}

function anim_hit(e, dmg, blocked, hp, blk, died) {
    var p = enemy_screen(e, e.def.scale * e.def.top * 0.5)
    e.flash = 1
    tween_to(e, { flash: 0 }, 0.3, "quad_out")
    e.pop = 1.18
    tween_to(e, { pop: 1 }, 0.2, "back_out")
    tween_to(e, { shown_hp: hp }, 0.25, "quad_out")
    e.shown_block = blk
    if blocked > 0 {
        popup_text(p.x + 22, p.y - 6, "-" + str(blocked), { gui: true, color: #4da6ff, font: fnt_ui, scale: 1 })
    }
    if dmg > 0 {
        popup_text(p.x, p.y, "-" + str(dmg), { gui: true, color: #ffe478, font: fnt_big, scale: 1, rise: 34, duration: 0.8 })
        hitstop(0.06)
        camera_shake(3, 8)
        play_sound(snd_hit, { pitch: 0.9 + min(dmg, 20) * 0.02 })
    } else {
        play_sound(snd_block)
    }
    wait(9)
    if died {
        play_sound(snd_lose, { volume: 0.4, pitch: 1.8 })
        tween_to(e, { fade: 0, pop: 0.6 }, 0.35, "quad_in")
        wait(10)
        e.gone = true
    }
}

function anim_player_block(total, amount) {
    global.pv.block = total
    popup_text(150, 52, "+" + str(amount), { gui: true, color: #4da6ff, font: fnt_big, scale: 1 })
    play_sound(snd_block)
    wait(6)
}

function anim_mana(m) {
    global.pv.mana = m
    popup_text(60, 76, "+", { gui: true, color: #66ffe3, font: fnt_big, scale: 1 })
    play_sound(snd_buff, { pitch: 1.5 })
    wait(6)
}

function anim_player_str(s) {
    global.pv.str = s
    popup_text(110, 76, loc("cb.strength", { n: s }), { gui: true, color: #ff9166, font: fnt_ui, scale: 1 })
    play_sound(snd_buff)
    wait(8)
}

// Shared by enemy attacks and self-damage (no waits: callable from other actions).
function show_player_hurt(dmg, blocked, hp, blk) {
    global.pv.block = blk
    tween_to(global.pv, { hp: hp }, 0.3, "quad_out")
    if dmg > 0 {
        global.pv.hurt = 1
        tween_to(global.pv, { hurt: 0 }, 0.4, "quad_out")
        popup_text(96, 30, "-" + str(dmg), { gui: true, color: #eb564b, font: fnt_big, scale: 1, rise: 30 })
        camera_shake(5, 10)
        hitstop(0.08)
        play_sound(snd_hurt)
    } else {
        play_sound(snd_block, { pitch: 0.8 })
    }
    if blocked > 0 {
        popup_text(150, 52, "-" + str(blocked), { gui: true, color: #4da6ff, font: fnt_ui, scale: 1 })
    }
}

function anim_player_hurt(dmg, blocked, hp, blk) {
    show_player_hurt(dmg, blocked, hp, blk)
    wait(10)
}

function anim_enemy_attack(e, dmg, blocked, hp, blk) {
    tween_to(e, { lunge: 1 }, 0.1, "quad_in")
    wait(6)
    show_player_hurt(dmg, blocked, hp, blk)
    tween_to(e, { lunge: 0 }, 0.18, "quad_out")
    wait(9)
}

function anim_enemy_block(e, blk, amount) {
    e.shown_block = blk
    var p = enemy_screen(e, e.def.scale * 0.6)
    popup_text(p.x, p.y, "+" + str(amount), { gui: true, color: #4da6ff, font: fnt_big, scale: 1 })
    play_sound(snd_block)
    wait(12)
}

function anim_enemy_buff(e, amount) {
    var p = enemy_screen(e, e.def.scale * 0.8)
    e.pop = 1.25
    tween_to(e, { pop: 1 }, 0.3, "back_out")
    popup_text(p.x, p.y, loc("cb.strength", { n: amount }), { gui: true, color: #ff9166, font: fnt_ui, scale: 1 })
    play_sound(snd_buff)
    wait(12)
}

function anim_victory() {
    wait(10)
    play_sound(snd_win)
    with CombatUI { show_banner(loc("cb.victory")) }
    wait(50)
    with Explorer { end_combat(true) }
}

function anim_defeat() {
    wait(10)
    play_sound(snd_lose)
    wait(40)
    with Explorer { end_combat(false) }
}
