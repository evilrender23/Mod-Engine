// Showcase of paths, timelines, animation curves and sequences.

global.dings = 0

// Follows the smooth closed path `loop`, facing where it goes.
object Ship {
    sprite = ship
    on create { path_start(loop, 1.5, path_action.restart, true) }
    on step { angle = direction - 90 }
}

// Walks the zigzag relative to where it was placed, back and forth.
object Zig {
    sprite = ball
    on create { path_start(zigzag, 1, path_action.reverse, false) }
}

// Blinks and drops coins with the timeline `blink`.
object Lamp {
    sprite = lamp
    anim_speed = 0
    on create {
        timeline_index = blink
        timeline_running = true
        timeline_loop = true
    }
}

object Drop {
    sprite = coin
    on create { vspeed = 1 }
    on outside_room { destroy() }
}

// Moved by the curve `bounce`: channel "y" for the height, "scale" for the size.
object Bouncer {
    sprite = ball
    t = 0
    base = 0
    on create { base = y }
    on step {
        t = (t + 1 / 120) mod 1
        y = base - 40 * animcurve_evaluate(bounce, "y", t)
        var s = animcurve_channel_evaluate(animcurve_get_channel(bounce, "scale"), t)
        scale_x = s
        scale_y = s
    }
}

// Coins created by the sequence's object track.
object Coin {
    sprite = coin
}

// Plays the sequence and draws the paths and some labels.
object Director {
    seq = -1
    on create { seq = layer_sequence_create("Instances", 0, 0, intro) }
    on key_pressed(key.space) {
        if layer_sequence_is_paused(seq) layer_sequence_play(seq)
        else layer_sequence_pause(seq)
    }
    on draw {
        set_color(#566c86)
        draw_path(loop, 0, 0, true)
        draw_path(zigzag, 200, 120, false)
    }
    on draw_gui {
        set_color(#f4f4f4)
        draw_text(4, 4, "frame " + str(floor(layer_sequence_get_headpos(seq))) + "  dings " + str(global.dings))
        draw_text(4, 166, "espacio: pausa la secuencia")
    }
}
