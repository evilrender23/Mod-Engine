// Showcase of the S1 assets: fonts (TTF, pixel, sprite), tile sets and tile maps.

object Ball {
    sprite = spr_ball
    hsp = 0
    vsp = 0

    on step {
        hsp = input_axis(key.left, key.right) * 1.5
        vsp = min(vsp + 0.25, 5)
        if key_pressed(key.up) and tile_meeting(x, y + 1, "Mundo") vsp = -5
        // move pixel by pixel against the tiles of the "Mundo" layer
        repeat abs(round(hsp)) {
            if tile_meeting(x + sign(hsp), y, "Mundo") break
            x += sign(hsp)
        }
        repeat abs(round(vsp)) {
            if tile_meeting(x, y + sign(vsp), "Mundo") { vsp = 0; break }
            y += sign(vsp)
        }
        if y > room_height + 16 { x = 72; y = 40 }
    }
}

object Demo {
    tm = -1
    placed = 0

    on create {
        tm = layer_tilemap_get_id("Mundo")
    }

    on step {
        var cx = tilemap_get_cell_x_at_pixel(tm, mouse_x, mouse_y)
        var cy = tilemap_get_cell_y_at_pixel(tm, mouse_x, mouse_y)
        // paint / erase ground: the auto tile picks the right edges
        if mouse_down(mouse.left) {
            tilemap_autotile(tm, "suelo", cx, cy, true)
            placed += 1
        }
        if mouse_down(mouse.right) tilemap_autotile(tm, "suelo", cx, cy, false)
        // every second the arrow at (12, 7) of "Deco" turns around
        if steps() % 60 == 0 {
            var d = tilemap_get("Deco", 12, 7)
            tilemap_set("Deco", tile_set_mirror(d, !tile_get_mirror(d)), 12, 7)
        }
    }

    on draw_gui {
        draw_set_font(fnt_title)
        set_align(align.center, align.top)
        set_color(#ffcd75)
        draw_text(160, 4, "¡Fuentes y tile sets!")
        draw_set_font(fnt_small)
        set_align(align.left, align.top)
        set_color(#f4f4f4)
        var d = tilemap_get_at_pixel(tm, mouse_x, mouse_y)
        var info = "tile " + tile_get_index(d) + (tile_get_empty(d) ? " (vacío)" : "")
        draw_text_wrap(4, 30, info + " · clic: pintar suelo, derecho: borrar. Flechas: mover la bola.", -1, 150)
        draw_set_font(fnt_digits)
        set_align(align.right, align.top)
        draw_text(316, 30, str(placed))
        set_align(align.left, align.top)
    }
}
