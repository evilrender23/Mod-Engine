// CombatUI: input and HUD of a fight (hand focus, end turn, speed, intents, HP bars and
// the damage projection of the focused card). CardView: one card on screen (hand, chest).
// Both draw in draw_gui so they stay crisp over the pixelated 3D view.

object CombatUI {
    banner = ""
    banner_t = 0
    last_mx = -1
    last_my = -1
    want_end = false
    want_confirm = false

    on create {
        ui_focus_clear("hand")
        ui_focus_wrap("hand", true)
        set_speed(global.speed_i)
    }

    function show_banner(text) {
        banner = text
        banner_t = 50
    }

    // Slots of the visible hand (tweened) and their focus rectangles.
    function layout() {
        var keep = ui_focus_get("hand")
        ui_focus_clear("hand")
        var n = len(global.views)
        var gap = n > 5 ? 70 : 84
        var x0 = 262 - (n - 1) * gap / 2
        for (i, v in global.views) {
            if !exists(v) { continue }
            v.slot_x = x0 + i * gap
            v.slot_y = 302
            tween_cancel_all(v)
            tween_to(v, { x: v.slot_x, y: v.slot_y - (v.raised ? 20 : 0), alpha: 1 }, 0.18, "quad_out")
            ui_focus_add("hand", v.card.uid, v.slot_x - 36, v.slot_y - 56, 72, 112)
        }
        ui_focus_add("hand", "end", 516, 292, 118, 44)
        // keep the focus on the same card if it is still in the hand; else the first card
        var still = false
        for (v in global.views) {
            if exists(v) and v.card.uid == keep { still = true }
        }
        if still { ui_focus_set("hand", keep) }
        else { ui_focus_set("hand", n > 0 ? global.views[0].card.uid : "end") }
    }

    function focused_card() {
        var f = ui_focus_get("hand")
        for (v in global.views) {
            if exists(v) and v.card.uid == f { return v.card }
        }
        return null
    }

    function confirm(f) {
        if f == "end" {
            end_turn()
            play_sound(snd_select)
            return
        }
        var c = focused_card()
        if c != null and !play_card(c) { show_no_mana(c) }
    }

    on step {
        if banner_t > 0 { banner_t -= 1 }
        if key_pressed(key.f) {
            set_speed(global.speed_i + 1)
            play_sound(snd_select)
        }
        if key_pressed(key.tab) { actionq_finish(global.q) }
        var mx = gui_mouse_x()
        var my = gui_mouse_y()
        var moved = mx != last_mx or my != last_my
        last_mx = mx
        last_my = my
        if mouse_pressed(mouse.left) and mx > 560 and my < 40 {
            set_speed(global.speed_i + 1)
            play_sound(snd_select)
            exit
        }
        // E / Enter pressed while the presentation plays are kept and run as soon as it is idle
        if key_pressed(key.e) { want_end = true }
        if actionq_busy(global.q) and ui_focus_confirm() { want_confirm = true }
        if global.cb.state != "player" {
            want_end = false
            want_confirm = false
        }
        // the presentation catches up before the next decision
        if global.cb.state != "player" or actionq_busy(global.q) { exit }
        var before = ui_focus_get("hand")
        var f = ui_focus_update()
        if moved {
            var h = ui_focus_at(mx, my, "hand")
            if h != null {
                ui_focus_set("hand", h)
                f = h
            }
        }
        if f != before { play_sound(snd_select, { volume: 0.5 }) }
        if want_end {
            want_end = false
            end_turn()
            play_sound(snd_select)
        } else if ui_focus_confirm() or want_confirm {
            want_confirm = false
            confirm(f)
        } else if mouse_pressed(mouse.left) and ui_focus_at(mx, my, "hand") != null {
            confirm(ui_focus_at(mx, my, "hand"))
        }
    }

    on draw_gui {
        var cb = global.cb
        if cb == null { exit }
        // red vignette when the player is hit
        if global.pv.hurt > 0 {
            set_alpha(global.pv.hurt * 0.35)
            set_color(#eb564b)
            draw_rect(0, 0, 640, 360)
            set_alpha(1)
        }
        // top strip behind the turn and the hint
        set_color(#1c1726)
        set_alpha(0.6)
        draw_rect(0, 0, 640, 36)
        set_alpha(1)
        // card table
        set_color(#1c1726)
        draw_rect(0, 314, 640, 360)
        set_color(#43434f)
        draw_line(0, 314, 640, 314)

        var focus = focused_card()
        var proj = null
        if focus != null and cb.state == "player" and !actionq_busy(global.q) { proj = project_card(focus) }

        // enemies, above the head: name, intent, HP bar (and the projection of the focused card)
        for (i, e in cb.enemies) {
            if e.gone { continue }
            var top = enemy_screen(e, e.def.scale * e.def.top + 0.03)
            var bx = top.x
            var by = max(top.y - 18, 90)
            set_alpha(e.fade)
            // HP bar with the numbers on it
            draw_bar(bx - 36, by, bx + 36, by + 10, clamp(e.shown_hp / e.max_hp, 0, 1), #43434f, #b0305c)
            if proj != null and proj.dmg[i] > 0 {
                // the part of the bar this card would take, blinking
                var a = clamp((e.shown_hp - proj.dmg[i]) / e.max_hp, 0, 1)
                set_color(steps() mod 40 < 20 ? #ffe478 : #f2a65e)
                draw_rect(bx - 36 + 72 * a, by, bx - 36 + 72 * clamp(e.shown_hp / e.max_hp, 0, 1), by + 10)
            }
            set_color(#272736)
            draw_rect(bx - 37, by - 1, bx + 37, by + 11, true)
            set_font(fnt_small)
            set_align(align.center, align.middle)
            set_color(#ffffeb)
            draw_text(bx, by + 5, str(round(e.shown_hp)) + "/" + str(e.max_hp))
            if e.shown_block > 0 {
                draw_sprite_ext(ico_block, 0, bx + 40, by - 2, 1, 1, 0, #ffffff, 1)
                set_align(align.left, align.middle)
                set_color(#4da6ff)
                draw_text(bx + 54, by + 5, str(e.shown_block))
            }
            // intent badge
            if e.alive and e.intent != null and cb.state != "won" {
                var it = e.intent
                var ico = it.type == "attack" ? ico_attack : (it.type == "block" ? ico_block : ico_buff)
                var label = it.type == "attack" ? str(it.amount + e.str) + (it.times > 1 ? "x" + str(it.times) : "") : (it.type == "buff" ? "+" + str(it.amount) : str(it.amount))
                set_font(fnt_ui)
                var w = 32 + text_width(label)
                set_color(#272736)
                draw_roundrect(bx - w / 2, by - 30, bx + w / 2, by - 4)
                draw_sprite_ext(ico, 0, bx - w / 2 + 4, by - 29, 2, 2, 0, #ffffff, 1)
                set_color(it.type == "attack" ? #ff9166 : (it.type == "block" ? #4da6ff : #ffe478))
                set_align(align.left, align.middle)
                draw_text(bx - w / 2 + 30, by - 17, label)
            }
            set_font(fnt_small)
            set_align(align.center, align.bottom)
            set_color(#c2c2d1)
            draw_text(bx, by - 32, loc(e.def.name))
            if proj != null and proj.dmg[i] > 0 {
                // what the focused card would do, under the bar
                var t = "-" + str(proj.dmg[i]) + (proj.hp[i] <= 0 ? " " + loc("cb.kills") : "")
                set_font(fnt_ui)
                var tw = text_width(t)
                set_color(#272736)
                draw_roundrect(bx - tw / 2 - 5, by + 13, bx + tw / 2 + 5, by + 31)
                set_align(align.center, align.middle)
                set_color(proj.hp[i] <= 0 ? #eb564b : #ffe478)
                draw_text(bx, by + 22, t)
            }
            set_alpha(1)
        }

        // player: HP, block, mana, strength
        hud_player(8, 8, global.pv.hp, global.run.hp, global.run.max_hp, global.pv.block)
        for (var k = 0; k < max(cb.max_mana, global.pv.mana); k++) {
            draw_sprite_ext(ico_mana, 0, 10 + k * 22, 40, 2, 2, 0, k < global.pv.mana ? #ffffff : #43434f, 1)
        }
        if global.pv.str > 0 {
            draw_sprite_ext(ico_buff, 0, 10, 68, 2, 2, 0, #ffffff, 1)
            set_font(fnt_ui)
            set_color(#ff9166)
            set_align(align.left, align.middle)
            draw_text(38, 80, "+" + str(global.pv.str))
        }
        // turn, combo and speed
        set_font(fnt_ui)
        set_align(align.center, align.top)
        set_color(#c2c2d1)
        draw_text(320, 6, loc("cb.turn", { n: global.pv.turn }))
        if global.pv.combo >= 1 {
            set_font(fnt_big)
            set_align(align.left, align.middle)
            set_color(global.pv.combo >= 3 ? #ff9166 : #ffe478)
            draw_text_ext(12, 112, loc("cb.combo", { n: global.pv.combo }), 0.6 * global.pv.combo_pop)
        }
        var sp = global.SPEEDS[global.speed_i]
        set_font(fnt_small)
        set_align(align.right, align.top)
        set_color(#272736)
        draw_roundrect(548, 6, 634, 30)
        set_color(#ffe478)
        draw_text(628, 12, loc("cb.speed", { s: sp == infinity ? loc("cb.instant") : "x" + str(sp) }))
        // piles
        set_align(align.right, align.top)
        set_color(#c2c2d1)
        draw_text(632, 258, loc("cb.draw", { n: deck_size(cb.draw) }))
        draw_text(632, 272, loc("cb.discard", { n: deck_size(cb.discard) }))
        // end turn button
        var on = ui_focus_is("hand", "end") and cb.state == "player"
        set_color(on ? #ffe478 : #43434f)
        draw_roundrect(516, 292, 634, 336)
        set_color(on ? #272736 : #c2c2d1)
        set_font(fnt_ui)
        set_align(align.center, align.middle)
        draw_text(575, 314, loc("cb.end"))
        // hint and banner
        set_font(fnt_small)
        set_align(align.center, align.top)
        set_color(#7e7e8f)
        draw_text(320, 24, loc("cb.hint"))
        if banner_t > 0 {
            set_alpha(min(1, banner_t / 12))
            set_font(fnt_big)
            set_align(align.center, align.middle)
            set_color(#272736)
            draw_text(322, 132, banner)
            set_color(#ffffeb)
            draw_text(320, 130, banner)
            set_alpha(1)
        }
        set_align(align.left, align.top)
    }
}

object CardView {
    sprite = spr_card
    card = null
    slot_x = 0
    slot_y = 0
    raised = false
    played = false
    group = "hand"
    kind_frame = 0

    on create {
        scale_x = 2
        scale_y = 2
        anim_speed = 0
        var def = data_get("cards", card.id)
        kind_frame = def.kind == "attack" ? 0 : (def.kind == "skill" ? 1 : 2)
        frame = kind_frame
    }

    // drawn in draw_gui (crisp, over the pixelated view), not in the room
    on draw {}

    on step {
        if played { exit }
        var want = ui_focus_is(group, card.uid) and ui_focus_group() == group
        if group == "hand" and (global.cb == null or global.cb.state != "player") { want = false }
        if want != raised {
            raised = want
            tween_to(id, { y: slot_y - (want ? 20 : 0) }, 0.15, want ? "back_out" : "quad_out")
            if want { pulse(id, 1.06, 0.12) }
        }
        sprite_outline(id, raised ? #ffe478 : null, 1)
        // chest cards go above the chest overlay
        depth = (group == "hand" ? -10 : -30) - (raised ? 2 : 0)
        var playable = group != "hand" or (global.cb != null and global.cb.state == "player" and card_cost(card) <= global.cb.mana)
        tint = playable ? #ffffff : #8c8c9c
    }

    on draw_gui {
        draw_self()
        var k = scale_x / 2
        var x0 = x - 40 * k
        var y0 = y - 56 * k
        var def = data_get("cards", card.id)
        set_alpha(alpha)
        // cost on the gem, name on the band
        set_font(fnt_ui)
        set_align(align.center, align.middle)
        set_color(#ffffeb)
        draw_text(x0 + 13 * k, y0 + 13 * k, str(def.cost))
        set_font(fnt_small)
        set_color(#272736)
        draw_text(x + 5 * k + 1, y0 + 14 * k + 1, loc(def.name))
        set_color(#ffffeb)
        draw_text(x + 5 * k, y0 + 14 * k, loc(def.name))
        draw_sprite_ext(card_art, def.art, x - 16 * k, y0 + 28 * k, 2 * k, 2 * k, 0, tint, alpha)
        set_color(#c2c2d1)
        set_align(align.center, align.top)
        draw_text_wrap(x, y0 + 70 * k, card_text(card.id), 13, 70)
        set_alpha(1)
        set_align(align.left, align.top)
    }
}
