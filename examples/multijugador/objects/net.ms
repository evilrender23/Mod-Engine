// ---------------------------------------------------------------------------
// Network side of the example. Uses the easy layer (net_host / net_join /
// net_send): every player has an id (the host is 1) and messages are a
// "tipo" (a name) plus any value (numbers, text, arrays, structs).
// ---------------------------------------------------------------------------

// shared state, set up once when the game starts
global.port = 6510          // TCP port of the game (open it in the router to play over the internet)
global.players = {}         // "id" -> { pid, name, x, y, seen }: everybody else in the game
global.chat = []            // last chat lines
global.status = ""          // message shown in the lobby (errors, "host closed the game"...)
global.my_name = ""

function player_color(pid) {
    var colors = [#ef7d57, #41a6f6, #a7f070, #ffcd75, #b13e53, #73eff7, #f4f4f4, #c48a54]
    return colors[(pid - 1) mod len(colors)]
}

function add_chat(text) {
    push(global.chat, text)
    if len(global.chat) > 6 remove(global.chat, 0)
    print("[chat] " + text)
}

function leave_game(reason) {
    net_disconnect()
    global.players = {}
    global.status = reason
    goto_room(lobby)
}

// Lives for the whole game and receives every network event.
object NetManager {
    persistent = true

    on create {
        // coming back to the lobby would create a second one
        if count(NetManager) > 1 {
            destroy()
            exit
        }
    }

    // "Async - Networking" in GameMaker: async_load says what happened
    on network {
        var kind = async_load.kind
        var pid = async_load.peer

        if kind == "connect" {
            // a player is in the game (when we join: one event per player already there)
            set(global.players, str(pid), { pid: pid, name: async_load.peer_name, x: 0, y: 0, seen: false })
            add_chat("» " + async_load.peer_name + " entró")
            if room == lobby goto_room(arena)
        } else if kind == "disconnect" {
            remove_key(global.players, str(pid))
            add_chat("« " + async_load.peer_name + " salió")
            // without the host there is no game
            if pid == 1 leave_game("El host cerró la partida")
        } else if kind == "message" {
            var p = get(global.players, str(pid), null)
            if async_load.tipo == "pos" and p != null {
                // only the target: Remote moves towards it smoothly
                p.x = async_load.data.x
                p.y = async_load.data.y
                p.seen = true
            } else if async_load.tipo == "chat" {
                add_chat(async_load.peer_name + ": " + str(async_load.data))
            }
        } else if kind == "failed" {
            global.status = "No se pudo unir: " + async_load.reason
        }
    }
}
