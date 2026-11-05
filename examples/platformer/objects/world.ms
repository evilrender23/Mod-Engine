object Ground {
    sprite = ground
    anim_speed = 0

    // Once every block exists, pick grass (frame 0) or plain dirt (frame 1).
    on room_start {
        frame = collides_at(x, y - 16, Ground) ? 1 : 0
    }
}

object Spike {
    sprite = spike
}

object Coin {
    sprite = coin
    base_y = 0

    on create {
        base_y = y
        frame = irand(0, 3)
    }

    on step {
        y = base_y + dsin(steps() * 4 + x * 3) * 1.5
    }

    on collision(Player) {
        if other.dead exit
        global.score += 10
        global.coins += 1
        play_sound(snd_coin)
        burst(x, y, 6)
        destroy()
    }
}

object Goal {
    sprite = flag
    reached = false

    on collision(Player) {
        if reached or other.dead exit
        reached = true
        global.score += 100
        global.won = true
        play_sound(snd_win)
        repeat 4 burst(x + 4, y - 18, 5, choose(#ffcd75, #a7f070, #41a6f6, #f4f4f4))
        alarm[0] = 60
    }

    on alarm(0) {
        if !next_room() goto_room(ending)
    }
}

// A tiny particle. `tint` can be passed when spawning: spawn(Spark, x, y, -20, { tint: #ff0000 })
object Spark {
    depth = -50
    tint = #ffcd75
    life = 30

    on create {
        motion_set(rand(360), rand_range(1, 2.5))
        gravity = 0.12
        friction = 0.03
        life = irand(20, 35)
    }

    on step {
        life -= 1
        if life <= 0 destroy()
    }

    on draw {
        set_color(tint)
        set_alpha(min(1, life / 15))
        draw_rect(x - 1, y - 1, x + 1, y + 1)
        set_alpha(1)
    }
}
