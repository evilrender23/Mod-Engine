// Draws every colour of the game palette with its pixel-art character,
// and a pear that changes colour with the arrow keys (palette_color).

object Swatches {
    sprite = pear
    pick = 4
    scale_x = 4
    scale_y = 4

    on step {
        if key_pressed(key.right) pick = (pick + 1) % palette_size()
        if key_pressed(key.left) pick = (pick - 1 + palette_size()) % palette_size()
        tint = palette_color(pick)
    }

    on draw_gui {
        var n = palette_size()
        for (var i = 0; i < n; i++) {
            var cx = 10 + (i % 12) * 25
            var cy = 8 + floor(i / 12) * 34
            set_color(palette_color(i))
            draw_rect(cx, cy, cx + 20, cy + 20)
            set_color(palette_color("H"))
            draw_rect(cx, cy, cx + 20, cy + 20, true)
            set_color(palette_color(palette_nearest(#ffffff)))
            draw_text(cx + 6, cy + 21, palette_char(i))
            if i == pick draw_rect(cx - 2, cy - 2, cx + 22, cy + 22, true)
        }
        set_color(palette_color("F"))
        draw_text(16, 164, "Pear36 · " + n + " colores · izq./der. elige: '" + palette_char(pick) + "'")
    }
}
