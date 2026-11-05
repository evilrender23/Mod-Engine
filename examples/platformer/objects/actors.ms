// Anything that walks on Ground: gravity + pixel-precise collision.
// Player and Slime inherit from it with `object Child : Actor`.
object Actor {
    hsp = 0
    vsp = 0
    grav = 0.3
    max_fall = 6
    on_ground = false
    hit_wall = false

    function move_and_collide() {
        hit_wall = false

        if collides_at(x + hsp, y, Ground) {
            var s = sign(hsp)
            while !collides_at(x + s, y, Ground) { x += s }
            hsp = 0
            hit_wall = true
        } else {
            x += hsp
        }

        vsp = min(vsp + grav, max_fall)
        if collides_at(x, y + vsp, Ground) {
            var s = sign(vsp)
            while !collides_at(x, y + s, Ground) { y += s }
            vsp = 0
        } else {
            y += vsp
        }

        on_ground = collides_at(x, y + 1, Ground)
    }
}

object Player : Actor {
    sprite = player_idle
    mask = player_idle      // same collision box whatever the animation
    depth = -10
    run_speed = 2
    accel = 0.35
    jump_power = 5.4
    coyote = 0              // steps left where a jump is still allowed after leaving a ledge
    jump_buffer = 0         // remembers a jump press made just before landing
    dead = false

    on step {
        if dead exit

        var dir = input_axis([key.left, key.a], [key.right, key.d])
        hsp = approach(hsp, dir * run_speed, on_ground ? accel : accel * 0.6)

        if jump_pressed() jump_buffer = 8
        coyote = on_ground ? 6 : coyote - 1
        if jump_buffer > 0 and coyote > 0 {
            vsp = -jump_power
            coyote = 0
            jump_buffer = 0
            play_sound(snd_jump)
        }
        jump_buffer -= 1
        // letting go of jump early gives a short hop
        if vsp < -2 and !jump_held() vsp = -2

        move_and_collide()

        if dir != 0 scale_x = dir
        if !on_ground sprite = player_jump
        else if abs(hsp) > 0.2 sprite = player_run
        else sprite = player_idle

        if y > room_height + 32 die()
    }

    on collision(Slime) {
        if dead exit
        // falling onto a slime squashes it; touching it any other way hurts
        if vsp > 0 and bbox_bottom - vsp <= other.bbox_top + 4 {
            other.squash()
            vsp = -4
            global.score += 50
            play_sound(snd_stomp)
        } else {
            die()
        }
    }

    on collision(Spike) {
        die()
    }

    function die() {
        if dead return
        dead = true
        visible = false
        global.lives -= 1
        play_sound(snd_hurt)
        camera_shake(3, 15)
        burst(x, y, 14, #b13e53)
        alarm[0] = 70
    }

    on alarm(0) {
        if global.lives <= 0 {
            global.won = false
            goto_room(ending)
        } else {
            restart_room()
        }
    }
}

object Slime : Actor {
    sprite = slime
    walk_speed: num(0, 2) = 0.5      // editable per instance in the room editor
    dir: int(-1, 1) = -1

    on step {
        hsp = dir * walk_speed
        move_and_collide()
        // turn around at walls and at the edge of a platform
        if hit_wall or (on_ground and inst_at(x + dir * 7, bbox_bottom + 1, Ground) == none) {
            dir = -dir
        }
        scale_x = -dir
    }

    function squash() {
        spawn(Squish, x, y, depth, { scale_x: scale_x })
        burst(x, y + 4, 6, #a7f070)
        destroy()
    }
}

// The flattened slime left behind after a stomp: just fades away.
object Squish {
    sprite = slime
    frame = 1
    anim_speed = 0
    life = 24

    on step {
        life -= 1
        alpha = life / 24
        if life <= 0 destroy()
    }
}
