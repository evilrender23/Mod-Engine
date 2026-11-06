// Showcase de las funciones F1. Todo se mueve solo (sirve para capturas sin teclado);
// con teclado: flechas mueven al jugador y lo que escribas sale en el cuadro de texto.

// Fosos: capa de colisión 2. Las balas (máscara 1) pasan por encima; los enemigos los rodean
// porque están marcados en la rejilla de navegación.
object Foso {
    sprite = spr_foso
    collision_layer = 2
    depth = 40
}

object Jugador {
    sprite = spr_jugador
    collision_mask = 1 | 2
    t = 0
    on step {
        t += 1
        var dx = input_axis([key.left, key.a], [key.right, key.d])
        var dy = input_axis([key.up, key.w], [key.down, key.s])
        if dx == 0 and dy == 0 {
            // paseo automático por la habitación de la derecha
            dx = cos(t / 40) * 0.8
            dy = sin(t / 55) * 0.6
        }
        var nx = x + dx * 1.5
        var ny = y + dy * 1.5
        if mp_grid_cell_at(global.nav, nx, ny) == 0 {
            x = nx
            y = ny
        }
        // el campo de flujo sigue al jugador (recalcular cada 20 pasos basta)
        if t % 20 == 0 mp_flow_update(global.flujo, x, y)
    }
}

object Enemigo {
    sprite = spr_enemigo
    collision_mask = 1 | 2
    on step {
        var d = mp_flow_dir(global.flujo, x, y)
        if d >= 0 motion_set(d, 0.9) else speed = 0
    }
}

object Bala {
    sprite = spr_bala
    collision_mask = 1      // no ve los fosos (capa 2)
    on step {
        if tile_meeting(x, y, "Muros") destroy()
        if place_meeting(x, y, Foso) global.bala_en_foso = true
    }
    on collision(Enemigo) { destroy() }
}

// Torreta: 32 rayos contra muros (tiles) y enemigos; dispara a quien ve.
object Torreta {
    sprite = spr_jugador
    tint = #ffe478
    giro = 0
    on step {
        giro += 0.6
        if steps() % 25 == 0 {
            var e = nearest(Enemigo)
            if e != null and line_of_sight(x, y, e.x, e.y, "Muros") {
                var b = spawn(Bala, x, y)
                b.direction = angle_to(x, y, e.x, e.y)
                b.speed = 3
            }
        }
    }
    on draw {
        for (var i = 0; i < 32; i++) {
            var a = giro + i * 360 / 32
            var h = raycast_dir(x, y, a, 300, ["Muros", Enemigo])
            if h == null continue
            set_alpha(0.55)
            set_color(h.inst != null ? #ff6b97 : #66ffe3)
            draw_line(x, y, h.x, h.y)
            set_alpha(1)
            // normal de la cara golpeada
            set_color(#ffe478)
            draw_line(h.x, h.y, h.x + h.nx * 6, h.y + h.ny * 6)
        }
        set_color(#ffffff)
        draw_self()
    }
}

object Demo {
    depth = -100
    items = []
    on create {
        cursor_sprite = spr_mira
        keyboard_string = ""
        for (var i = 0; i < 14; i++) items[i] = "Elemento " + str(i + 1)
    }
    on draw {
        // rejilla (depuración) y camino A* de la esquina al jugador
        // la rejilla solo en la mitad izquierda de la room (recorte en coordenadas de room)
        gpu_set_scissor(0, 0, 240, room_height)
        mp_grid_draw(global.nav)
        gpu_reset_scissor()
        var j = nearest(Jugador)
        var pts = mp_grid_path_points(global.nav, 24, 24, j.x, j.y)
        set_color(#cfff70)
        for (var i = 1; i < len(pts); i++) draw_line(pts[i - 1][0], pts[i - 1][1], pts[i][0], pts[i][1], 2)
        // la flecha del campo de flujo en algunas celdas
        set_color(#ffffeb)
        for (var cy = 1; cy < 16; cy += 2) {
            for (var cx = 1; cx < 29; cx += 2) {
                var c = mp_grid_cell_center(global.nav, cx, cy)
                var d = mp_flow_dir(global.flujo, c[0], c[1])
                if d >= 0 draw_line(c[0], c[1], c[0] + len_x(5, d), c[1] + len_y(5, d))
            }
        }
        set_color(#ffffff)
    }
    on draw_gui {
        // panel redondeado con una lista recortada que se desplaza
        set_color(#272736)
        set_alpha(0.9)
        draw_roundrect_ext(330, 104, 470, 200, 10, 10)
        set_alpha(1)
        set_color(#4da6ff)
        draw_roundrect_ext(330, 104, 470, 200, 10, 10, true, 2)
        gpu_push_scissor(338, 112, 124, 80)
        var scroll = (steps() * 0.4) % (len(items) * 14)
        for (var i = 0; i < len(items); i++) {
            set_color(i % 2 == 0 ? #ffffeb : #c2c2d1)
            draw_text(342, 114 + i * 14 - scroll, items[i])
        }
        gpu_pop_scissor()
        // título con texto enriquecido
        set_color(#ffffff)
        draw_text_rich(334, 8, "[c=#ffe478][wave=3]Funciones F1[/wave][/c]\n[b]A*[/b], [c=#cfff70]flujo[/c] y [c=#ff6b97][shake]rayos[/][/]", 140)
        // texto por glifos: arcoíris ondulante
        draw_text_glyphs(334, 214, "¡Glifos al vuelo!", function(i, ch, gx, gy) {
            return { dy: sin(steps() / 6 + i / 2) * 3, color: hsv(i * 18 + steps() * 3, 0.6, 1) }
        })
        // caja de texto con keyboard_string
        set_color(#43434f)
        draw_roundrect(334, 236, 474, 258, false, 6)
        set_color(#ffffeb)
        var caret = (steps() / 30) % 2 < 1 ? "_" : ""
        gpu_set_scissor(338, 236, 132, 22)
        draw_text(340, 239, "Nombre: " + keyboard_string + caret)
        gpu_reset_scissor()
    }
}
