// Lives for the whole game (persistent) and draws the HUD on top of every level.
object Game {
    persistent = true

    on create {
        // coming back to the title room would create a second one
        if count(Game) > 1 {
            destroy()
            exit
        }
    }

    on key_pressed(key.escape) {
        if room == title quit()
        else goto_room(title)
    }

    on draw_gui {
        if room == title or room == ending exit
        for (var i = 0; i < 3; i++) {
            draw_sprite(heart, i < global.lives ? 0 : 1, 6 + i * 10, 6)
        }
        draw_sprite(coin, 0, 48, 10)
        shadow_text(56, 3, "x" + str(global.coins))
        set_align(align.right)
        shadow_text(314, 3, pad(global.score, 6, "0"))
        set_align(align.left)
    }
}

object Title {
    t = 0
    best = 0

    on create {
        best = load_data("best", 0)
    }

    on step {
        t += 1
        if key_pressed(key.enter) or key_pressed(key.space) {
            reset_run()
            play_sound(snd_select)
            goto_room(level1)
        }
    }

    on draw_gui {
        set_align(align.center, align.middle)
        var bob = dsin(t * 3) * 3
        draw_text_ext(162, 42 + bob, "COIN CLIMBER", 3, #1a1c2c)
        draw_text_ext(160, 40 + bob, "COIN CLIMBER", 3, #ffcd75)
        if (t div 30) mod 2 == 0 shadow_text(160, 80, "press ENTER")
        shadow_text(160, 100, "arrows / WASD: move    SPACE: jump")
        shadow_text(160, 116, "best score: " + pad(best, 6, "0"), 1, #ffcd75)
        set_align(align.left, align.top)
    }
}

object Ending {
    best = 0
    record = false

    on create {
        best = load_data("best", 0)
        if global.score > best {
            best = global.score
            record = true
            save_data("best", best)
        }
    }

    on key_pressed(key.enter) {
        play_sound(snd_select)
        goto_room(title)
    }

    on draw_gui {
        set_align(align.center, align.middle)
        if global.won shadow_text(160, 40, "YOU WIN!", 3, #a7f070)
        else shadow_text(160, 40, "GAME OVER", 3, #b13e53)
        shadow_text(160, 84, "score  " + pad(global.score, 6, "0"))
        shadow_text(160, 100, "coins  " + str(global.coins))
        if record shadow_text(160, 118, "new best score!", 1, #ffcd75)
        else shadow_text(160, 118, "best   " + pad(best, 6, "0"))
        shadow_text(160, 150, "press ENTER", 1, #c2d3e8)
        set_align(align.left, align.top)
    }
}
