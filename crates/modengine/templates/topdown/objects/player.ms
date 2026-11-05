// Top-down movement: 8 directions, blocked by Wall.
object Player {
    sprite = player
    depth = -10
    move_speed = 1.5

    on step {
        var dx = input_axis([key.left, key.a], [key.right, key.d])
        var dy = input_axis([key.up, key.w], [key.down, key.s])
        // move each axis on its own so the player slides along walls
        if !collides_at(x + dx * move_speed, y, Wall) x += dx * move_speed
        if !collides_at(x, y + dy * move_speed, Wall) y += dy * move_speed
        if dx != 0 scale_x = dx
    }

    on draw_gui {
        draw_text(4, 4, format("Gemas: {}  ·  Flechas / WASD", global.gems))
        if count(Gem) == 0 draw_text(4, 14, "¡Todas recogidas!")
    }
}

object Wall {
    sprite = wall
}

object Gem {
    sprite = gem

    on collision(Player) {
        global.gems += 1
        destroy()
    }
}
