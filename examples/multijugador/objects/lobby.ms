// Title screen: choose a name, then host (H) or join (J).
object Lobby {
    editing = ""        // "", "name" or "ip": which field is being typed
    ip = "127.0.0.1"
    before = ""         // value of the field before typing (Enter on an empty field keeps it)
    t = 0

    // a name for players that did not choose one (only when they host/join, so the
    // lobby itself stays the same on every run)
    function default_name(prefix) {
        if global.my_name != "" return
        randomize()
        global.my_name = prefix + str(irand(10, 99))
    }

    on step {
        t += 1
        if editing != "" {
            // typing: the field follows keyboard_string (Enter or Esc to finish)
            var text = substr(keyboard_string(), 0, 15)
            if editing == "name" global.my_name = text
            else ip = text
            if key_pressed(key.enter) or key_pressed(key.escape) {
                // an empty field keeps what it had before
                if global.my_name == "" global.my_name = before
                if ip == "" ip = before
                editing = ""
            }
            exit
        }
        if net_status() == "connecting" exit

        // N / I start typing a new name / IP (keyboard_string collects the keys)
        if key_pressed(key.n) {
            editing = "name"
            before = global.my_name
            keyboard_string_set("")
        } else if key_pressed(key.i) {
            editing = "ip"
            before = ip
            keyboard_string_set("")
        } else if key_pressed(key.h) {
            // become the host: our id is 1
            default_name("Host")
            if net_host(global.port, 4, global.my_name) {
                global.status = ""
                global.chat = []
                goto_room(arena)
            } else {
                global.status = "No se pudo abrir el puerto " + str(global.port) + " (¿ya hay un host en este PC?)"
            }
        } else if key_pressed(key.j) {
            // the answer arrives in NetManager's network event
            default_name("Jugador")
            global.chat = []
            global.status = ""
            net_join(ip, global.port, global.my_name)
        } else if key_pressed(key.escape) {
            quit()
        }
    }

    on draw_gui {
        set_align(align.center, align.top)
        var bob = dsin(t * 3) * 2
        draw_text_ext(241, 31 + bob, "MULTIJUGADOR", 3, #1a1c2c)
        draw_text_ext(240, 30 + bob, "MULTIJUGADOR", 3, #ffcd75)

        set_align(align.left, align.top)
        var c_name = editing == "name" ? #ffcd75 : #f4f4f4
        var c_ip = editing == "ip" ? #ffcd75 : #f4f4f4
        var cursor = (t div 20) mod 2 == 0 ? "_" : " "
        var shown = global.my_name == "" and editing != "name" ? "(sin elegir: pulsa N)" : global.my_name
        draw_text_ext(130, 96, "Nombre: " + shown + (editing == "name" ? cursor : ""), 1, c_name)
        draw_text_ext(130, 114, "IP del host: " + ip + (editing == "ip" ? cursor : ""), 1, c_ip)
        draw_text_ext(130, 132, "Puerto: " + str(global.port), 1, #94b0c2)

        draw_text_ext(130, 160, "H  crear partida (host)", 1, #a7f070)
        draw_text_ext(130, 176, "J  unirse a la IP de arriba", 1, #41a6f6)
        draw_text_ext(130, 192, "N  cambiar nombre    I  cambiar IP", 1, #94b0c2)
        draw_text_ext(130, 208, "Esc  salir", 1, #94b0c2)

        set_align(align.center, align.top)
        if net_status() == "connecting" {
            draw_text_ext(240, 236, "Conectando a " + ip + "...", 1, #ffcd75)
        } else if global.status != "" {
            draw_text_ext(240, 236, global.status, 1, #ef7d57)
        }
        set_align(align.left, align.top)
    }
}
