// Explorer: the first-person view, grid movement and the events of the cells. It also
// draws the enemies of a fight (same view) and the exploration HUD. The run's model is
// in scripts/run.ms; the fights in scripts/combat.ms.
//
// Logic vs presentation: (px, py, dir) change at once; the camera (cx, cy, ca) follows
// with an eased animation. Keys pressed during an animation are buffered.

global.view_id = -1
global.views = []

object Explorer {
    view = -1
    // logic
    px = 0
    py = 0
    dir = 0
    // camera (presentation)
    cx = 0
    cy = 0
    ca = 0
    from_x = 0
    from_y = 0
    from_a = 0
    to_x = 0
    to_y = 0
    to_a = 0
    t = 1
    dur = 1
    bump = false
    buffered = ""
    // combat framing: the camera steps back and the 3D view slides up over the card table
    cam_back = 0
    view_y = 0
    trans = 0
    msg = ""
    msg_t = 0
    music = null
    hp_shown = 0

    on create {
        view = fpview_create(640, 360)
        global.view_id = view
        fpview_set_fov(view, 75)
        fpview_set_shading(view, 0.72)
        fpview_set_wall(view, 4, door_arch, 0, #ffffff, false)
        if global.run == null { new_run(irand(1, 999999)) }
        global.run.state = "explore"
        music = audio_play_layers([mus_calm, mus_fight], [0.8, 0])
        start_floor()
        say(loc("msg.start"))
    }

    on room_start {
        // the 3D view becomes pixel art of the palette; the HUD (draw_gui) stays crisp
        postfx_pixelate(320, 0, 0.15, true)
    }

    on room_end {
        postfx_off()
        audio_stop_all()
    }

    function say(text) {
        msg = text
        msg_t = 240
    }

    // ------------------------------------------------------------ floors

    function start_floor() {
        var f = generate_floor(global.run.floor)
        fpview_set_map(view, f.map)
        fpview_set_wall(view, 1, f.def.wall, 0)
        fpview_set_wall(view, 2, f.def.wall, 1)
        fpview_set_wall(view, 3, f.def.wall, 2)
        fpview_set_floor(view, floor_tiles, 0)
        fpview_set_ceiling(view, ceiling_vault)
        fpview_set_fog(view, palette_color(f.def.fog), 1.4, f.def.fog_end)
        for (c in f.corridors) { fpview_set_cell_floor(view, c[0], c[1], floor_tiles, 1) }
        if f.def.boss == null { fpview_set_cell_floor(view, f.goal.x, f.goal.y, floor_tiles, 2) }
        // an arch on the room side of every door
        var k = 0
        for (d in f.doors) {
            for (k = 0; k < 4; k++) {
                if cell_open(d[0] + grid_dir_dx(k), d[1] + grid_dir_dy(k)) and cell_open(d[0] - grid_dir_dx(k), d[1] - grid_dir_dy(k)) {
                    fpview_set_edge(view, d[0], d[1], k, 4)
                    break
                }
            }
        }
        px = f.start.x
        py = f.start.y
        // look along the way to the exit
        var best = 0
        for (k = 0; k < 4; k++) {
            if px + grid_dir_dx(k) == f.towards[0] and py + grid_dir_dy(k) == f.towards[1] { best = k }
        }
        dir = best
        cx = px
        cy = py
        ca = dir * 90
        to_x = px
        to_y = py
        to_a = ca
        t = 1
        reveal()
        save_run()
    }

    // marks what can be seen from here as visited (minimap): straight lines up to 5 cells
    function reveal() {
        var f = global.floor
        f.visited[py][px] = 1
        for (var k = 0; k < 4; k++) {
            for (var n = 1; n <= 5; n++) {
                var x = px + grid_dir_dx(k) * n
                var y = py + grid_dir_dy(k) * n
                if !cell_open(x, y) { break }
                f.visited[y][x] = 1
            }
        }
    }

    // ------------------------------------------------------------ movement

    function animate(nx, ny, na, steps, is_bump) {
        from_x = cx
        from_y = cy
        from_a = ca
        to_x = nx
        to_y = ny
        to_a = na
        t = 0
        dur = steps
        bump = is_bump
    }

    function act(a) {
        if a == "turn_l" or a == "turn_r" {
            dir = (dir + (a == "turn_l" ? 1 : 3)) mod 4
            animate(px, py, to_a + (a == "turn_l" ? 90 : -90), 11, false)
            play_sound(snd_turn)
            exit
        }
        var d = dir
        if a == "back" { d = (dir + 2) mod 4 }
        if a == "left" { d = (dir + 1) mod 4 }
        if a == "right" { d = (dir + 3) mod 4 }
        var nx = px + grid_dir_dx(d)
        var ny = py + grid_dir_dy(d)
        var ev = event_at(nx, ny)
        if ev == null {
            // done props (open chests, dry fountains) still stand in their cell
            for (e in global.floor.events) {
                if e.done and e.x == nx and e.y == ny and (e.type == "chest" or e.type == "fountain") { ev = e }
            }
        }
        if ev != null and ev.type != "stairs" {
            touch(ev, d)
            return
        }
        if fpview_can_move(view, px, py, d) {
            px = nx
            py = ny
            animate(px, py, to_a, 13, false)
            play_sound(snd_step, { pitch: 0.9 + rand(0.2) })
        } else {
            animate(px + grid_dir_dx(d) * 0.18, py + grid_dir_dy(d) * 0.18, to_a, 10, true)
            play_sound(snd_bump)
        }
    }

    // walking into something that stands in the next cell
    function touch(ev, d) {
        if ev.type == "enemy" or ev.type == "boss" {
            // turn to face it (it may be beside us) and fight
            var turn = (d - dir + 4) mod 4
            if turn != 0 {
                dir = d
                animate(px, py, to_a + (turn == 1 ? 90 : (turn == 3 ? -90 : 180)), 10, false)
            }
            begin_combat(ev)
            return
        }
        animate(px + grid_dir_dx(d) * 0.12, py + grid_dir_dy(d) * 0.12, to_a, 10, true)
        if ev.done {
            if ev.type == "fountain" { say(loc("msg.fountain_dry")) }
            return
        }
        if ev.type == "chest" {
            ev.done = true
            play_sound(snd_chest)
            say(loc("msg.chest"))
            open_chest()
        } else if ev.type == "fountain" {
            ev.done = true
            var heal = min(global.run.max_hp - global.run.hp, 18)
            global.run.hp += heal
            play_sound(snd_heal)
            popup_text(96, 30, "+" + str(heal), { gui: true, color: #8fde5d, font: fnt_big, scale: 1 })
            say(loc("msg.fountain", { n: heal }))
        }
    }

    function open_chest() {
        global.run.state = "chest"
        spawn(ChestUI, 0, 0, -20, { offer: chest_offer() })
    }

    // called after entering a cell (when the step animation ends)
    function arrive() {
        reveal()
        var ev = event_at(px, py)
        if ev != null and ev.type == "stairs" { descend() }
    }

    function descend() {
        global.run.state = "descend"
        play_sound(snd_stairs)
        say(loc("msg.stairs"))
        coroutine(function() {
            var tw = tween_to(self, { trans: 1 }, 0.6, "quad_in")
            wait_until(function() { return tween_done(tw) })
            global.run.floor += 1
            start_floor()
            say(loc("msg.saved"))
            tw = tween_to(self, { trans: 0 }, 0.6, "quad_out")
            wait_until(function() { return tween_done(tw) })
            global.run.state = "explore"
        })
    }

    // ------------------------------------------------------------ combat

    function begin_combat(ev) {
        global.run.state = "combat"
        var name = loc(data_get("enemies", ev.group[0], "name"))
        say(ev.type == "boss" ? loc("msg.boss") : loc("msg.enemy", { name: name }))
        play_sound(snd_encounter)
        audio_layers_set(music, [0.25, 1], 600)
        tween_to(self, { cam_back: 0.38, view_y: -46 }, 0.5, "quad_inout")
        start_combat(ev.group, ev, ev.x, ev.y, dir)
        global.views = []
        spawn(CombatUI, 0, 0, 0)
    }

    function end_combat(won) {
        with CombatUI destroy()
        with CardView destroy()
        global.views = []
        var ev = global.cb.ev
        if !won {
            global.run.state = "lost"
            end_run()
            spawn(Summary, 0, 0, -30, { won: false })
            return
        }
        if ev != null { ev.done = true }
        if global.cb.boss {
            global.run.state = "won"
            end_run()
            spawn(Summary, 0, 0, -30, { won: true })
            return
        }
        global.run.state = "explore"
        global.cb = null
        audio_layers_set(music, [0.8, 0], 900)
        tween_to(self, { cam_back: 0, view_y: 0 }, 0.5, "quad_inout")
    }

    // ------------------------------------------------------------ debug jumps (see run.ms)

    function face_open() {
        for (var k = 0; k < 4; k++) {
            var d = (dir + k) mod 4
            if cell_open(px + grid_dir_dx(d), py + grid_dir_dy(d)) and event_at(px + grid_dir_dx(d), py + grid_dir_dy(d)) == null {
                dir = d
                ca = dir * 90
                to_a = ca
                return d
            }
        }
        return dir
    }

    // Debug: stands 1-3 cells away from the first event of that type, looking at it.
    function look_at(type) {
        for (e in global.floor.events) {
            if e.type != type or e.done { continue }
            for (var dist = 3; dist >= 1; dist--) {
                for (var k = 0; k < 4; k++) {
                    var ok = true
                    for (var n = 1; n <= dist; n++) {
                        var x = e.x - grid_dir_dx(k) * n
                        var y = e.y - grid_dir_dy(k) * n
                        if !cell_open(x, y) or event_at(x, y) != null { ok = false }
                    }
                    if ok {
                        px = e.x - grid_dir_dx(k) * dist
                        py = e.y - grid_dir_dy(k) * dist
                        dir = k
                        cx = px
                        cy = py
                        ca = dir * 90
                        to_x = px
                        to_y = py
                        to_a = ca
                        t = 1
                        reveal()
                        return true
                    }
                }
            }
        }
        return false
    }

    function debug_jump(what) {
        print("[debug] " + what)
        if global.run.state == "combat" and what != "win_combat" {
            actionq_clear(global.q)
            with CombatUI destroy()
            with CardView destroy()
            global.cb = null
            global.run.state = "explore"
            cam_back = 0
            view_y = 0
        }
        if starts_with(what, "floor:") {
            global.run.floor = clamp(num(substr(what, 6)) ?? 1, 1, global.MAX_FLOOR)
            start_floor()
        } else if starts_with(what, "combat") or what == "boss" {
            if what == "boss" and global.run.floor != global.MAX_FLOOR {
                global.run.floor = global.MAX_FLOOR
                start_floor()
            }
            var d = face_open()
            var def = global.floor.def
            // "combat:N" = group N of this floor (data/floors.toml)
            var gi = starts_with(what, "combat:") ? clamp(num(substr(what, 7)) ?? 0, 0, len(def.groups) - 1) : 0
            var group = what == "boss" ? [def.boss] : def.groups[gi]
            var ev = { type: what == "boss" ? "boss" : "enemy", x: px + grid_dir_dx(d), y: py + grid_dir_dy(d), done: false, t: 0, group: group }
            push(global.floor.events, ev)
            begin_combat(ev)
        } else if starts_with(what, "look:") {
            look_at(substr(what, 5))
        } else if what == "win_combat" {
            debug_win_combat()
        } else if what == "chest" {
            open_chest()
        } else if what == "fountain" {
            global.run.hp = max(1, global.run.hp - 20)
            var d = face_open()
            var ev = { type: "fountain", x: px + grid_dir_dx(d), y: py + grid_dir_dy(d), done: false, t: 0, group: [] }
            push(global.floor.events, ev)
            touch(ev, d)
        } else if what == "descend" {
            if global.run.floor < global.MAX_FLOOR { descend() }
        } else if what == "won" or what == "lost" {
            global.run.state = what
            spawn(Summary, 0, 0, -30, { won: what == "won" })
        }
    }

    // ------------------------------------------------------------ step

    on step {
        if global.debug != null {
            var what = global.debug
            global.debug = null
            debug_jump(what)
        }
        if msg_t > 0 { msg_t -= 1 }
        hp_shown = lerp(hp_shown, global.run.hp, 0.2)
        postfx_transition(trans)

        if global.run.state == "explore" {
            if key_pressed(key.w) or key_pressed(key.up) { buffered = "fwd" }
            if key_pressed(key.s) or key_pressed(key.down) { buffered = "back" }
            if key_pressed(key.a) { buffered = "left" }
            if key_pressed(key.d) { buffered = "right" }
            if key_pressed(key.q) or key_pressed(key.left) { buffered = "turn_l" }
            if key_pressed(key.e) or key_pressed(key.right) { buffered = "turn_r" }
            if key_pressed(key.escape) { goto_room(title) }
        } else {
            buffered = ""
        }

        if t < 1 {
            t = min(1, t + 1 / dur)
            var k = ease("smooth", t)
            if bump { k = sin(t * pi) }
            cx = lerp(from_x, to_x, k)
            cy = lerp(from_y, to_y, k)
            ca = lerp(from_a, to_a, k)
            if t >= 1 {
                if bump {
                    cx = from_x
                    cy = from_y
                } else if global.run.state == "explore" and cx == px and cy == py {
                    arrive()
                }
            }
        } else if buffered != "" {
            var a = buffered
            buffered = ""
            act(a)
        } else if global.run.state == "explore" and (key_down(key.w) or key_down(key.up)) {
            act("fwd")
        }

        // head bob while walking; in a fight the camera steps back to frame the enemies
        var h = 0.5
        if t < 1 and !bump and ca == to_a { h = 0.5 + sin(t * pi) * 0.025 }
        var back_x = -grid_dir_dx(dir) * cam_back
        var back_y = -grid_dir_dy(dir) * cam_back
        fpview_set_camera(view, cx + back_x, cy + back_y, ca, h - cam_back * 0.45, cam_back * 8)
        billboards()
    }

    function billboards() {
        fpview_clear_billboards(view)
        var anim = floor(steps() / 12) mod 2
        if global.run.state == "combat" and global.cb != null {
            for (e in global.cb.enemies) {
                if e.gone { continue }
                // the lunge moves it towards the camera; the flash tints it
                var lx = -grid_dir_dx(dir) * e.lunge * 0.35
                var ly = -grid_dir_dy(dir) * e.lunge * 0.35
                var col = color_mix(#ffffff, #ff5040, e.flash)
                var bob = sin(steps() * 0.08 + e.wx * 3) * 0.015
                e.bb = fpview_add_billboard(view, e.wx + lx, e.wy + ly, e.def.sprite, anim, e.def.scale * e.pop, e.def.float + bob, col, e.fade)
            }
            return
        }
        for (e in global.floor.events) {
            if abs(e.x - px) + abs(e.y - py) > 9 { continue }
            if e.type == "enemy" or e.type == "boss" {
                if e.done { continue }
                var n = len(e.group)
                var right = (dir + 3) mod 4
                for (i, id in e.group) {
                    var d = data_get("enemies", id)
                    var off = n == 1 ? 0 : (i - (n - 1) / 2) * 0.3
                    var bob = sin(steps() * 0.08 + i * 2) * 0.015
                    fpview_add_billboard(view, e.x + grid_dir_dx(right) * off, e.y + grid_dir_dy(right) * off, d.sprite, anim, d.scale, d.float + bob)
                }
            } else if e.type == "chest" {
                fpview_add_billboard(view, e.x, e.y, prop_chest, e.done ? 1 : 0, 0.42)
            } else if e.type == "fountain" {
                fpview_add_billboard(view, e.x, e.y, e.done ? prop_fountain_dry : prop_fountain, e.done ? 0 : anim, 0.55)
            } else if e.type == "stairs" {
                fpview_add_billboard(view, e.x, e.y, prop_exit_glow, 0, 0.18, 0.25 + sin(steps() * 0.06) * 0.04)
            }
        }
    }

    // ------------------------------------------------------------ drawing

    on draw {
        fpview_draw(view, 0, view_y, 640, 360)
    }

    on draw_gui {
        if global.run.state == "combat" or global.run.state == "won" or global.run.state == "lost" { exit }
        var f = global.floor
        // minimap with what has been seen
        var marks = []
        for (e in f.events) {
            if f.visited[e.y][e.x] == 0 { continue }
            var c = #ffe478
            if e.type == "enemy" or e.type == "boss" { c = e.done ? #606070 : #eb564b }
            if e.type == "fountain" { c = e.done ? #606070 : #4da6ff }
            if e.type == "chest" and e.done { c = #606070 }
            if e.type == "stairs" { c = #66ffe3 }
            push(marks, { x: e.x, y: e.y, color: c })
        }
        set_color(#272736)
        draw_rect(518, 6, 634, 122)
        set_color(#606070)
        draw_rect(518, 6, 634, 122, true)
        fpview_draw_minimap(view, 520, 8, 6, { width: 112, height: 112, visited: f.visited, background: #1c1726,
            wall: #7e7e8f, floor: #606070, edge: #c2c2d1, player: #ffffeb, markers: marks })
        // floor, HP and deck
        hud_player(8, 8, hp_shown, global.run.hp, global.run.max_hp, 0)
        set_font(fnt_ui)
        set_align(align.left, align.top)
        set_color(#c2c2d1)
        draw_text(8, 40, loc("hud.floor", { n: global.run.floor }))
        draw_sprite(ico_deck, 0, 96, 42)
        draw_text(112, 40, str(len(global.run.deck)))
        // message and controls
        if msg_t > 0 and msg != "" {
            set_align(align.center, align.middle)
            set_alpha(min(1, msg_t / 30))
            set_color(#272736)
            draw_roundrect(320 - text_width(msg) / 2 - 12, 300, 320 + text_width(msg) / 2 + 12, 324)
            set_color(#ffffeb)
            draw_text(320, 312, msg)
            set_alpha(1)
        }
        set_font(fnt_small)
        set_align(align.center, align.bottom)
        set_color(#272736)
        set_alpha(0.7)
        draw_rect(320 - text_width(loc("hud.controls")) / 2 - 8, 340, 320 + text_width(loc("hud.controls")) / 2 + 8, 360)
        set_alpha(1)
        set_color(#c2c2d1)
        draw_text(320, 356, loc("hud.controls"))
        set_align(align.left, align.top)
    }
}

// HP bar with a heart (and the block in a fight). Shared with CombatUI.
function hud_player(x, y, shown, hp, max_hp, block) {
    set_color(#272736)
    draw_roundrect(x, y, x + 160, y + 26)
    draw_sprite_ext(ico_heart, 0, x + 4, y + 2, 2, 2, 0, #ffffff, 1)
    draw_bar(x + 32, y + 7, x + 152, y + 19, clamp(shown / max_hp, 0, 1), #43434f, #eb564b)
    set_font(fnt_small)
    set_align(align.center, align.middle)
    set_color(#ffffeb)
    draw_text(x + 92, y + 13, str(round(shown)) + " / " + str(max_hp))
    if block > 0 {
        draw_sprite_ext(ico_block, 0, x + 164, y + 1, 2, 2, 0, #ffffff, 1)
        set_font(fnt_ui)
        set_color(#ffffeb)
        draw_text(x + 176, y + 13, str(block))
    }
    set_align(align.left, align.top)
}
