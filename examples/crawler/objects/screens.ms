// Title screen, chest choice and the end-of-run summary.

object Title {
    view = -1
    options = []
    t = 0

    on create {
        // a small fixed corridor behind the menu, slowly looking around
        view = fpview_create(640, 360)
        fpview_set_map(view, [
            [1,1,1,1,1,1,1,1,1],
            [1,0,0,0,0,0,0,0,1],
            [1,2,1,1,0,1,1,3,1],
            [1,1,1,1,0,1,1,1,1],
            [1,1,1,1,0,1,1,1,1],
            [1,1,1,1,1,1,1,1,1],
        ])
        fpview_set_wall(view, 1, wall_crypt, 0)
        fpview_set_wall(view, 2, wall_crypt, 1)
        fpview_set_wall(view, 3, wall_crypt, 2)
        fpview_set_floor(view, floor_tiles, 0)
        fpview_set_ceiling(view, ceiling_vault)
        fpview_set_fog(view, #272736, 1, 6)
        build_menu()
    }

    on room_start {
        postfx_pixelate(320, 0, 0.15, true)
    }

    on room_end {
        postfx_off()
    }

    function build_menu() {
        options = ["new"]
        if save_exists("partida") { push(options, "continue") }
        push(options, "lang")
        push(options, "quit")
        ui_focus_clear("menu")
        for (i, o in options) { ui_focus_add("menu", o, 220, 200 + i * 30, 200, 26) }
        ui_focus_wrap("menu", true)
    }

    function label(o) {
        if o == "continue" {
            var s = load_data("partida", { floor: 1 })
            return loc("menu.continue", { n: s.floor })
        }
        if o == "new" { return loc("menu.new") }
        if o == "lang" { return loc("menu.lang") }
        return loc("menu.quit")
    }

    function choose(o) {
        play_sound(snd_select)
        if o == "new" {
            end_run()
            new_run(irand(1, 999999))
            goto_room(crypt)
        } else if o == "continue" {
            if load_run() { goto_room(crypt) }
        } else if o == "lang" {
            global.settings.lang = loc_language() == "es" ? "en" : "es"
            loc_set_language(global.settings.lang)
            save_settings()
        } else if o == "quit" {
            quit()
        }
    }

    on step {
        t += 1
        fpview_set_camera(view, 4, 4.2, 90 + sin(t * 0.01) * 25, 0.5, 4)
        fpview_clear_billboards(view)
        fpview_add_billboard(view, 4, 1.3, en_skeleton, 0, 0.6)
        fpview_add_billboard(view, 3.4, 1.6, en_bat, floor(t / 10) mod 2, 0.4, 0.3 + sin(t * 0.07) * 0.03)
        var before = ui_focus_get("menu")
        var f = ui_focus_update()
        var h = ui_focus_at(gui_mouse_x(), gui_mouse_y(), "menu")
        if h != null and mouse_pressed(mouse.left) { choose(h) }
        else if ui_focus_confirm() { choose(f) }
        if f != before { play_sound(snd_select, { volume: 0.5 }) }
    }

    on draw {
        fpview_draw(view, 0, 0, 640, 360)
    }

    on draw_gui {
        set_color(#272736)
        set_alpha(0.55)
        draw_rect(0, 0, 640, 360)
        set_alpha(1)
        // title with a card on each side
        draw_sprite_ext(spr_card_back, 0, 150, 100, 2, 2, -12, #ffffff, 1)
        draw_sprite_ext(spr_card, 0, 490, 100, 2, 2, 12, #ffffff, 1)
        draw_sprite_ext(card_art, 6, 474, 88, 2, 2, 12, #ffffff, 1)
        set_font(fnt_big)
        set_align(align.center, align.middle)
        set_color(#272736)
        draw_text(322, 92, loc("title.name"))
        set_color(#ffe478)
        draw_text(320, 90, loc("title.name"))
        set_font(fnt_ui)
        set_color(#c2c2d1)
        draw_text(320, 130, loc("title.subtitle"))
        for (i, o in options) {
            var on = ui_focus_is("menu", o)
            set_color(on ? #ffe478 : #272736)
            draw_roundrect(220, 200 + i * 30, 420, 226 + i * 30)
            set_color(on ? #272736 : #c2c2d1)
            draw_text(320, 213 + i * 30, label(o))
        }
        set_font(fnt_small)
        set_color(#7e7e8f)
        draw_text(320, 336, loc("menu.hint"))
        set_align(align.right, align.bottom)
        draw_text(634, 356, loc("title.credits"))
        set_align(align.left, align.top)
    }
}

// Chest: three cards, pick one for the deck (or none).
object ChestUI {
    offer = []
    cards = []
    t = 0

    on create {
        ui_focus_push("chest")
        ui_focus_clear("chest")
        for (i, id in offer) {
            var x = 200 + i * 120
            var c = spawn(CardView, x, 420, -22, { card: { uid: -1 - i, id: id }, group: "chest", slot_y: 200 })
            tween_to(c, { y: 200 }, 0.35, "back_out", { delay: i * 0.08 })
            push(cards, c)
            ui_focus_add("chest", -1 - i, x - 40, 144, 80, 112)
        }
        ui_focus_wrap("chest", true)
    }

    function close() {
        for (c in cards) { if exists(c) { destroy(c) } }
        ui_focus_pop()
        global.run.state = "explore"
        destroy()
    }

    on step {
        t += 1
        if t < 20 { exit }
        var f = ui_focus_update()
        var h = ui_focus_at(gui_mouse_x(), gui_mouse_y(), "chest")
        if h != null and mouse_pressed(mouse.left) { f = h }
        if ui_focus_confirm() or (h != null and mouse_pressed(mouse.left)) {
            var i = -1 - f
            add_card(offer[i])
            play_sound(snd_chest)
            popup_text(320, 300, "+1", { gui: true, color: #8fde5d, font: fnt_big, scale: 1 })
            close()
        } else if ui_focus_back() {
            play_sound(snd_select)
            close()
        }
    }

    on draw_gui {
        set_color(#1c1726)
        set_alpha(0.75)
        draw_rect(0, 0, 640, 360)
        set_alpha(1)
        set_font(fnt_ui)
        set_align(align.center, align.middle)
        set_color(#ffe478)
        draw_text(320, 96, loc("chest.title"))
        set_font(fnt_small)
        set_color(#7e7e8f)
        draw_text(320, 300, loc("chest.skip"))
        set_align(align.left, align.top)
    }
}

// End of the run (victory or defeat).
object Summary {
    won = false
    t = 0

    on create {
        play_sound(won ? snd_win : snd_lose)
    }

    on step {
        t += 1
        if t > 30 and (key_pressed(key.enter) or key_pressed(key.space) or mouse_pressed(mouse.left)) {
            global.run = null
            goto_room(title)
        }
    }

    on draw_gui {
        set_color(#1c1726)
        set_alpha(min(0.85, t / 30))
        draw_rect(0, 0, 640, 360)
        set_alpha(min(1, t / 20))
        set_font(fnt_big)
        set_align(align.center, align.middle)
        set_color(won ? #ffe478 : #eb564b)
        draw_text(320, 100, won ? loc("end.won") : loc("end.lost", { n: global.run.floor }))
        draw_sprite_ext(won ? en_king : en_skeleton, 0, 320, 210, 2, 2, 0, won ? #606070 : #ffffff, 1)
        var s = global.run.stats
        set_font(fnt_ui)
        set_color(#c2c2d1)
        draw_text(320, 236, loc("end.kills", { n: s.kills }))
        draw_text(320, 256, loc("end.turns", { n: s.turns }))
        draw_text(320, 276, loc("end.cards", { n: s.cards }))
        draw_text(320, 296, loc("end.deck", { n: len(global.run.deck) }))
        set_color(#ffe478)
        if t > 30 { draw_text(320, 330, loc("end.back")) }
        set_alpha(1)
        set_align(align.left, align.top)
    }
}
