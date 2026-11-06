//! Networking between real engines in one process: TCP (framed), UDP, the `net_*` session
//! layer and bad input. Everything runs on 127.0.0.1 with ports picked by the OS.

use modengine::Engine;
use modscript::{intern, Value};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

static N: AtomicU32 = AtomicU32::new(0);

/// A one-room project whose room holds one `Net` instance.
fn project(objects: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("modengine-net-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("rooms")).unwrap();
    std::fs::write(dir.join("game.toml"), "[game]\nname = \"nettest\"\n").unwrap();
    std::fs::write(dir.join("objects.ms"), objects).unwrap();
    std::fs::write(dir.join("rooms/r.toml"), "width = 64\nheight = 64\n[[layer]]\ntype = \"instances\"\ninstances = [ { object = \"Net\", x = 0, y = 0 } ]\n").unwrap();
    dir
}

fn engine(objects: &str, exec: &str) -> Engine {
    let mut e = Engine::load(&project(objects)).unwrap_or_else(|m| panic!("{m}"));
    if !exec.is_empty() {
        e.set_startup_code(exec).unwrap();
    }
    e.start();
    e
}

fn global(e: &Engine, name: &str) -> Value {
    e.globals.get(&intern(name)).cloned().unwrap_or(Value::Null)
}

fn log(e: &Engine) -> Vec<String> {
    match global(e, "log") {
        Value::Array(a) => a.borrow().iter().map(|v| v.to_string()).collect(),
        _ => vec![],
    }
}

/// Ticks every engine until `done` holds (or fails after 5 s).
fn run_until(engines: &mut [&mut Engine], what: &str, done: impl Fn(&[&mut Engine]) -> bool) {
    let start = Instant::now();
    while !done(engines) {
        assert!(start.elapsed() < Duration::from_secs(5), "timed out waiting for: {what}\n{}", engines.iter().map(|e| format!("{:?}", log(e))).collect::<Vec<_>>().join("\n"));
        for e in engines.iter_mut() {
            e.tick();
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn no_errors(e: &Engine) {
    assert!(e.errors.is_empty(), "runtime errors: {:?}", e.errors);
}

const SERVER: &str = r#"
global.log = []
object Net {
    on create {
        global.srv = network_create_server(network_socket_tcp, 0, 4)
        global.port = network_get_port(global.srv)
    }
    on network {
        var t = async_load.type
        if t == network_type_connect {
            push(global.log, format("connect id={} ip={} socket={}", async_load.id == global.srv, async_load.ip, async_load.socket > 0))
        } else if t == network_type_data {
            var b = async_load.buffer
            var msg = buffer_read(b, buffer_string)
            var n = buffer_read(b, buffer_u16)
            var f = buffer_read(b, buffer_f64)
            push(global.log, format("data {} {} {} size={}", msg, n, f, async_load.size))
            // answer on the same socket
            var out = buffer_create(16, buffer_grow, 1)
            buffer_write(out, buffer_string, "pong:" + msg)
            buffer_write(out, buffer_s32, -n)
            network_send_packet(async_load.id, out, buffer_tell(out))
            buffer_delete(out)
        } else if t == network_type_disconnect {
            push(global.log, "disconnect")
        }
    }
}
"#;

const CLIENT: &str = r#"
global.log = []
object Net {
    on network {
        var t = async_load.type
        if t == network_type_non_blocking_connect {
            push(global.log, "connected " + str(async_load.succeeded))
            if async_load.succeeded {
                var b = buffer_create(1, buffer_grow, 1)
                buffer_write(b, buffer_string, "hola")
                buffer_write(b, buffer_u16, 1234)
                buffer_write(b, buffer_f64, 2.5)
                network_send_packet(global.sock, b, buffer_tell(b))
                buffer_delete(b)
            }
        } else if t == network_type_data {
            push(global.log, format("reply {} {}", buffer_read(async_load.buffer, buffer_string), buffer_read(async_load.buffer, buffer_s32)))
        } else if t == network_type_disconnect {
            push(global.log, "server gone")
        }
    }
}
"#;

#[test]
fn tcp_server_and_client_talk_with_framed_packets() {
    let mut server = engine(SERVER, "");
    let port = global(&server, "port").as_num().unwrap();
    assert!(port > 0.0, "server opened a port: {port}");
    let mut client = engine(CLIENT, &format!("global.sock = network_create_socket(network_socket_tcp)\nnetwork_connect(global.sock, \"127.0.0.1\", {port})"));

    run_until(&mut [&mut server, &mut client], "the reply", |e| log(e[1]).iter().any(|l| l.starts_with("reply")));
    assert_eq!(log(&client), vec!["connected true", "reply pong:hola -1234"]);
    let slog = log(&server);
    assert_eq!(slog[0], "connect id=true ip=127.0.0.1 socket=true");
    assert_eq!(slog[1], "data hola 1234 2.5 size=15");

    // the client hangs up: the server sees it
    let sock = global(&client, "sock").as_num().unwrap();
    client.run_code(&format!("network_destroy({sock})")).unwrap();
    run_until(&mut [&mut server, &mut client], "the disconnect", |e| log(e[0]).last().map(|l| l == "disconnect").unwrap_or(false));
    // the temporary buffers of the data events were freed
    assert!(server.buffers.list.is_empty() && client.buffers.list.is_empty());
    no_errors(&server);
    no_errors(&client);

    // the server going away is seen by a client too
    let mut client2 = engine(CLIENT, &format!("global.sock = network_create_socket(network_socket_tcp)\nnetwork_connect(global.sock, \"localhost\", {port})"));
    run_until(&mut [&mut server, &mut client2], "second client", |e| log(e[1]).iter().any(|l| l.starts_with("reply")));
    server.shutdown();
    run_until(&mut [&mut client2], "server gone", |e| log(e[0]).last().map(|l| l == "server gone").unwrap_or(false));
    no_errors(&client2);
}

#[test]
fn connecting_to_a_closed_port_fails_in_the_event() {
    // grab a free port and close it again
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut client = engine(CLIENT, &format!("global.sock = network_create_socket(network_socket_tcp)\nnetwork_connect(global.sock, \"127.0.0.1\", {port})"));
    run_until(&mut [&mut client], "the failed connect", |e| !log(e[0]).is_empty());
    assert_eq!(log(&client), vec!["connected false"]);
    no_errors(&client);
}

const UDP: &str = r#"
global.log = []
object Net {
    on create {
        global.udp = network_create_socket(network_socket_udp)
        global.port = network_get_port(global.udp)
    }
    on network {
        if async_load.type == network_type_data {
            var b = async_load.buffer
            push(global.log, format("{} from {} size={}", buffer_read(b, buffer_text), async_load.ip, async_load.size))
            if async_load.port > 0 and async_load.size < 10 {
                var r = buffer_create(32, buffer_fixed, 1)
                buffer_write(r, buffer_text, "eco de udp")
                network_send_udp(global.udp, async_load.ip, async_load.port, r, buffer_tell(r))
                buffer_delete(r)
            }
        }
    }
}
"#;

#[test]
fn udp_datagrams_both_ways() {
    let mut a = engine(UDP, "");
    let pa = global(&a, "port").as_num().unwrap();
    let mut b = engine(UDP, &format!("var m = buffer_create(8, buffer_grow, 1)\nbuffer_write(m, buffer_text, \"ping\")\nnetwork_send_udp(global.udp, \"127.0.0.1\", {pa}, m, buffer_tell(m))"));
    run_until(&mut [&mut a, &mut b], "the udp echo", |e| !log(e[1]).is_empty());
    assert_eq!(log(&a), vec!["ping from 127.0.0.1 size=4"]);
    assert_eq!(log(&b), vec!["eco de udp from 127.0.0.1 size=10"]);
    no_errors(&a);
    no_errors(&b);
}

const SESSION: &str = r#"
global.log = []
object Net {
    on network {
        var k = async_load.kind
        if k == "message" {
            push(global.log, format("{} {} from {} ({}): {}", k, async_load.tipo, async_load.peer, async_load.peer_name, async_load.data))
        } else {
            push(global.log, format("{} {} {}{}", k, async_load.peer, async_load.peer_name, async_load.reason))
        }
    }
}
"#;

#[test]
fn session_layer_with_a_host_and_two_players() {
    let mut host = engine(SESSION, "global.port = net_host(0, 3, \"Ana\")");
    let port = global(&host, "port").as_num().unwrap();
    assert!(port > 0.0);
    assert!(host.net.session_is_host());
    assert_eq!(host.net.session_me(), 1);

    let mut bea = engine(SESSION, &format!("net_join(\"127.0.0.1\", {port}, \"Bea\")"));
    run_until(&mut [&mut host, &mut bea], "Bea joins", |e| log(e[1]).len() == 1 && log(e[0]).len() == 1);
    assert_eq!(log(&host), vec!["connect 2 Bea"]);
    assert_eq!(log(&bea), vec!["connect 1 Ana"]);
    assert_eq!(bea.net.session_me(), 2);
    assert_eq!(bea.net.session_status(), "online");

    let mut cris = engine(SESSION, &format!("net_join(\"127.0.0.1\", {port}, \"Cris\")"));
    run_until(&mut [&mut host, &mut bea, &mut cris], "Cris joins", |e| log(e[2]).len() == 2 && log(e[1]).len() == 2);
    assert_eq!(log(&cris), vec!["connect 1 Ana", "connect 2 Bea"]);
    assert_eq!(log(&bea)[1], "connect 3 Cris");
    assert_eq!(cris.net.session_peers(), vec![1, 2, 3]);
    assert_eq!(host.net.session_peers(), vec![1, 2, 3]);

    // a fourth player does not fit (max 3)
    let mut dani = engine(SESSION, &format!("net_join(\"127.0.0.1\", {port}, \"Dani\")"));
    run_until(&mut [&mut host, &mut bea, &mut cris, &mut dani], "Dani is rejected", |e| !log(e[3]).is_empty());
    assert!(log(&dani)[0].starts_with("failed 0 la partida está llena"), "{:?}", log(&dani));
    assert_eq!(dani.net.session_status(), "off");

    // Cris talks to everybody (relayed by the host), Bea only to Cris
    cris.run_code("net_send(all, \"pos\", {x: 5, y: 6.5, tags: [\"a\", true, null]})").unwrap();
    bea.run_code("net_send(3, \"secreto\", \"hola Cris\")").unwrap();
    run_until(&mut [&mut host, &mut bea, &mut cris], "the messages", |e| log(e[0]).len() == 3 && log(e[1]).len() == 3 && log(e[2]).len() == 3);
    // (JSON objects come back with their keys sorted)
    assert_eq!(log(&host)[2], "message pos from 3 (Cris): {tags: [\"a\", true, null], x: 5, y: 6.5}");
    assert_eq!(log(&bea)[2], "message pos from 3 (Cris): {tags: [\"a\", true, null], x: 5, y: 6.5}");
    assert_eq!(log(&cris)[2], "message secreto from 2 (Bea): hola Cris");
    // the host did not see the private message
    assert!(!log(&host).iter().any(|l| l.contains("secreto")));

    // Cris leaves: the others see it
    cris.run_code("net_disconnect()").unwrap();
    run_until(&mut [&mut host, &mut bea], "Cris leaves", |e| log(e[0]).len() == 4 && log(e[1]).len() == 4);
    assert_eq!(log(&host)[3], "disconnect 3 Cris");
    assert_eq!(log(&bea)[3], "disconnect 3 Cris");

    // the host closes the game: Bea is told
    host.run_code("net_disconnect()").unwrap();
    run_until(&mut [&mut bea], "host leaves", |e| log(e[0]).len() == 5);
    assert_eq!(log(&bea)[4], "disconnect 1 Ana");
    assert_eq!(bea.net.session_status(), "off");
    for e in [&host, &bea, &cris, &dani] {
        no_errors(e);
    }
}

#[test]
fn bad_input_never_panics() {
    let mut server = engine(SERVER, "");
    let port = global(&server, "port").as_num().unwrap() as u16;
    // a header announcing 4 GB, then junk
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.write_all(&[0xff, 0xff, 0xff, 0xff, 1, 2, 3]).unwrap();
    run_until(&mut [&mut server], "the bad client is dropped", |e| log(e[0]).iter().any(|l| l == "disconnect"));
    no_errors(&server);

    // a well-framed packet whose content is not what the script expects: a script error,
    // reported like any other, and the game goes on
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.write_all(&[2, 0, 0, 0, b'h', b'i']).unwrap();
    run_until(&mut [&mut server], "the bad packet", |e| !e[0].errors.is_empty());
    assert!(server.errors[0].contains("buffer"), "{:?}", server.errors);

    // the session host ignores junk and keeps working
    let mut host = engine(SESSION, "global.port = net_host(0, 4, \"Ana\")");
    let hp = global(&host, "port").as_num().unwrap() as u16;
    let mut junk = std::net::TcpStream::connect(("127.0.0.1", hp)).unwrap();
    junk.write_all(&[3, 0, 0, 0, b'{', b'x', b'!']).unwrap();
    junk.write_all(&[2, 0, 0, 0, b'{', b'}']).unwrap();
    junk.write_all(b"\x20\x00\x00\x00{\"op\":\"msg\",\"to\":-1,\"k\":\"x\"}  ").unwrap();
    for _ in 0..50 {
        host.tick();
        std::thread::sleep(Duration::from_millis(1));
    }
    let mut bea = engine(SESSION, &format!("net_join(\"127.0.0.1\", {hp}, \"Bea\")"));
    run_until(&mut [&mut host, &mut bea], "a real player still joins", |e| log(e[1]).len() == 1);
    assert_eq!(log(&host), vec!["connect 2 Bea"]);
    no_errors(&host);
    no_errors(&bea);
}

#[test]
fn network_event_and_async_load() {
    // the event parses, is listed, and async_load is null outside of it
    assert!(modengine::object::EVENT_NAMES.contains(&"network"));
    assert_eq!(modengine::object::parse_event("network", None).unwrap(), modengine::object::Ev::Network);
    let mut e = engine("object Net {\n on network { }\n on create { global.outside = async_load }\n}\n", "");
    assert!(global(&e, "outside").is_null());
    e.tick();
    no_errors(&e);
    // net_send outside of a game just answers false
    e.run_code("global.r = net_send(all, \"x\", 1)").unwrap();
    assert_eq!(global(&e, "r").to_string(), "false");
}
