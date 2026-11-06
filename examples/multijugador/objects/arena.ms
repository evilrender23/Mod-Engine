// The play room: our Player, one Remote per other player, chat and player list.
object Arena {
    chatting = false
    t = 0

    on create {
        // start at a spot that depends on our id, so players don't overlap
        var me = net_my_id()
        spawn(Player, 90 + ((me - 1) mod 4) * 100, 135)
    }

    on step {
        t += 1
        // one Remote per player we know about (players can join at any time)
        var ids = keys(global.players)
        for (var i = 0; i < len(ids); i++) {
            var p = get(global.players, ids[i])
            var found = false
            with Remote if peer == p.pid found = true
            if not found spawn(Remote, p.x, p.y, 0, { peer: p.pid })
        }

        // chat: T starts typing, Enter sends, Esc cancels
        if chatting {
            if key_pressed(key.enter) {
                var text = trim(keyboard_string())
                if text != "" {
                    net_send(all, "chat", text)
                    add_chat(global.my_name + ": " + text)
                }
                chatting = false
            } else if key_pressed(key.escape) {
                chatting = false
            }
        } else if key_pressed(key.t) {
            chatting = true
            keyboard_string_set("")
        } else if key_pressed(key.escape) {
            leave_game("")
        }
        global.chatting = chatting
    }

    on draw {
        // floor grid
        set_color(#3e4a66)
        for (var gx = 0; gx < room_width; gx += 30) draw_line(gx, 0, gx, room_height)
        for (var gy = 0; gy < room_height; gy += 30) draw_line(0, gy, room_width, gy)
    }

    on draw_gui {
        // who am I
        var me = net_my_id()
        var title = net_is_host() ? "HOST - puerto " + str(net_port()) : "CLIENTE - jugador " + str(me)
        draw_text_ext(6, 4, title, 1, #ffcd75)

        // player list (net_peers() includes us)
        set_align(align.right, align.top)
        var peers = net_peers()
        draw_text_ext(474, 4, "Jugadores: " + str(len(peers)), 1, #f4f4f4)
        for (var i = 0; i < len(peers); i++) {
            var pid = peers[i]
            var label = str(pid) + ". " + net_peer_name(pid) + (pid == me ? " (tú)" : "")
            draw_text_ext(474, 18 + i * 12, label, 1, player_color(pid))
        }
        set_align(align.left, align.top)

        // chat
        var y0 = 200 - len(global.chat) * 11
        for (var i = 0; i < len(global.chat); i++) {
            draw_text_ext(6, y0 + i * 11, global.chat[i], 1, #c2d3e8)
        }
        if chatting {
            var cursor = (t div 20) mod 2 == 0 ? "_" : " "
            draw_text_ext(6, 216, "> " + keyboard_string() + cursor, 1, #ffcd75)
        }
        draw_text_ext(6, 256, "Flechas/WASD: mover   T: chat   Esc: salir", 1, #94b0c2)
    }
}

// Our own square: moved by the keyboard, its position goes to everybody.
object Player {
    depth = -10
    last_x = -1
    last_y = -1
    t = 0

    on step {
        t += 1
        if not global.chatting {
            var sp = 2
            var dx = (key_down(key.right) or key_down(key.d)) - (key_down(key.left) or key_down(key.a))
            var dy = (key_down(key.down) or key_down(key.s)) - (key_down(key.up) or key_down(key.w))
            x = clamp(x + dx * sp, 8, room_width - 8)
            y = clamp(y + dy * sp, 8, room_height - 8)
        }
        // send when we moved (every 3 steps = 20 times/s), and twice a second anyway so
        // players that just joined see us too
        var moved = x != last_x or y != last_y
        if (moved and t mod 3 == 0) or t mod 30 == 0 {
            net_send(all, "pos", { x: x, y: y })
            last_x = x
            last_y = y
        }
    }

    on draw {
        draw_player(x, y, net_my_id(), global.my_name)
    }
}

// Another player. Its real position arrives ~20 times a second; drawing it there
// directly would look jumpy, so we glide towards it (interpolation).
object Remote {
    peer = 0

    on step {
        var p = get(global.players, str(peer), null)
        if p == null {
            destroy()     // that player left
            exit
        }
        if not p.seen exit
        if dist(x, y, p.x, p.y) > 120 {
            // too far (just joined, or a big lag spike): jump
            x = p.x
            y = p.y
        } else {
            x = lerp(x, p.x, 0.3)
            y = lerp(y, p.y, 0.3)
        }
    }

    on draw {
        var p = get(global.players, str(peer), null)
        if p != null and p.seen draw_player(x, y, peer, p.name)
    }
}

function draw_player(px, py, pid, name) {
    set_color(#1a1c2c)
    draw_rect(px - 9, py - 9, px + 9, py + 9)
    set_color(player_color(pid))
    draw_rect(px - 8, py - 8, px + 8, py + 8)
    set_align(align.center, align.bottom)
    draw_text_ext(px, py - 11, name, 1, #f4f4f4)
    set_align(align.left, align.top)
}
