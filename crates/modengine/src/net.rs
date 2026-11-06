//! Networking: TCP and UDP sockets polled once per step on the engine thread
//! (GameMaker's `network_*` model), plus a small "session" layer (`net_host`, `net_join`,
//! `net_send`) that gives every player an id and sends ModScript values as JSON.
//!
//! Everything is non-blocking. The only helper thread is the one that resolves and
//! connects in `network_connect` (std has no non-blocking connect); its result comes back
//! as a `network_type_non_blocking_connect` event.
//!
//! Wire formats (documented in docs/red.md):
//! - framed TCP (`network_create_server`, `network_connect`): every packet is
//!   `u32 little-endian length` + payload, at most [`MAX_PACKET`] bytes.
//! - raw TCP (`*_raw`): bytes as they arrive, no framing.
//! - UDP: one datagram = one packet, at most [`MAX_UDP`] bytes.
//! - session layer: framed TCP whose payloads are JSON objects with an `op` field.

use std::collections::{BTreeMap, HashMap};
use std::io::{ErrorKind, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

/// Largest framed packet (payload) accepted or sent: bigger ones close the connection.
pub const MAX_PACKET: usize = 1024 * 1024;
/// Largest UDP datagram.
pub const MAX_UDP: usize = 65507;
/// Unsent data allowed per socket before sends start failing.
pub const MAX_OUTBUF: usize = 16 * 1024 * 1024;
/// Bytes read per socket and step at most (the rest waits for the next step).
const READ_BUDGET: usize = 8 * 1024 * 1024;
/// Session protocol version (bumped when the JSON messages change).
pub const SESSION_VERSION: u32 = 1;

// GameMaker constant values
pub const SOCKET_TCP: i64 = 0;
pub const SOCKET_UDP: i64 = 1;
pub const SOCKET_WS: i64 = 2;
pub const TYPE_CONNECT: i64 = 1;
pub const TYPE_DISCONNECT: i64 = 2;
pub const TYPE_DATA: i64 = 3;
pub const TYPE_NON_BLOCKING_CONNECT: i64 = 4;

pub const CONFIG_CONNECT_TIMEOUT: i64 = 0;
pub const CONFIG_USE_NON_BLOCKING_SOCKET: i64 = 1;
pub const CONFIG_ENABLE_RELIABLE_UDP: i64 = 2;
pub const CONFIG_DISABLE_RELIABLE_UDP: i64 = 3;
pub const CONFIG_AVOID_TIME_WAIT: i64 = 4;
pub const CONFIG_ENABLE_MULTICAST: i64 = 5;
pub const CONFIG_DISABLE_MULTICAST: i64 = 6;

/// Something that happened on the network since the last step.
#[derive(Clone, Debug, PartialEq)]
pub enum NetEvent {
    /// A client connected to server `server`; `socket` is the new socket to talk to it.
    Connect { server: u32, socket: u32, ip: String, port: u16 },
    /// A connection closed. `server` is the server socket for clients of a server, or
    /// the socket itself for sockets made with `network_create_socket`.
    Disconnect { server: u32, socket: u32, ip: String, port: u16 },
    /// A packet (framed TCP / UDP) or a chunk of bytes (raw TCP) arrived on `socket`.
    Data { socket: u32, ip: String, port: u16, data: Vec<u8> },
    /// Result of `network_connect`.
    NonBlockingConnect { socket: u32, ip: String, port: u16, succeeded: bool },
    /// Session layer (`net_*`): kind is "connect", "disconnect", "message" or "failed".
    Session { kind: &'static str, peer: u32, name: String, tipo: String, data: serde_json::Value, reason: String },
}

// ================================================================ framing

/// A framed packet: length (u32 LE) + payload.
pub fn frame(payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(payload.len() + 4);
    v.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    v.extend_from_slice(payload);
    v
}

/// Takes every complete packet out of `inbuf` (partial ones stay for later).
/// A length above `max` is an error: the stream can't be trusted any more.
pub fn unframe(inbuf: &mut Vec<u8>, max: usize) -> Result<Vec<Vec<u8>>, String> {
    let mut out = Vec::new();
    let mut at = 0;
    while inbuf.len() - at >= 4 {
        let len = u32::from_le_bytes([inbuf[at], inbuf[at + 1], inbuf[at + 2], inbuf[at + 3]]) as usize;
        if len > max {
            return Err(format!("packet of {len} bytes is bigger than the limit ({max}); wrong protocol? (a *_raw socket talking to a framed one?)"));
        }
        if inbuf.len() - at - 4 < len {
            break;
        }
        out.push(inbuf[at + 4..at + 4 + len].to_vec());
        at += 4 + len;
    }
    inbuf.drain(..at);
    Ok(out)
}

/// `::ffff:1.2.3.4` → `1.2.3.4`.
pub fn ip_text(ip: IpAddr) -> String {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None => v6.to_string(),
        },
        IpAddr::V4(v4) => v4.to_string(),
    }
}

fn would_block(e: &std::io::Error) -> bool {
    matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted | ErrorKind::TimedOut)
}

/// Binds IPv6 (dual-stack where the OS allows it) and IPv4 on the same port.
fn bind_both<T>(port: u16, bind: impl Fn(SocketAddr) -> std::io::Result<T>, local: impl Fn(&T) -> Option<u16>) -> Result<(Vec<T>, u16), String> {
    let mut out = Vec::new();
    let mut port = port;
    let mut last_err = String::new();
    match bind(SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port)) {
        Ok(s) => {
            port = local(&s).unwrap_or(port);
            out.push(s);
        }
        Err(e) => last_err = e.to_string(),
    }
    // on Linux the IPv6 socket already takes IPv4 too and this fails: that is fine
    match bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port)) {
        Ok(s) => {
            port = local(&s).unwrap_or(port);
            out.push(s);
        }
        Err(e) => last_err = e.to_string(),
    }
    if out.is_empty() {
        return Err(format!("cannot open port {port}: {last_err}"));
    }
    Ok((out, port))
}

// ================================================================ sockets

enum ConnState {
    /// Made with `network_create_socket`, not connected yet (or failed / closed).
    Idle,
    Connecting { rx: Receiver<Result<TcpStream, String>>, host: String, port: u16 },
    Open(TcpStream),
}

struct TcpConn {
    state: ConnState,
    framed: bool,
    /// Accepted by this server socket.
    server: Option<u32>,
    inbuf: Vec<u8>,
    out: Vec<u8>,
    peer: Option<SocketAddr>,
    last_recv: Instant,
    out_since: Option<Instant>,
}

enum Sock {
    Server { listeners: Vec<TcpListener>, framed: bool, max_clients: usize, clients: Vec<u32>, port: u16 },
    Tcp(TcpConn),
    Udp { socks: Vec<UdpSocket>, port: u16 },
}

struct SockEntry {
    sock: Sock,
    /// `network_set_timeout`: (read, write) in milliseconds, 0 = never.
    timeout: (u64, u64),
}

/// The session (`net_*`) state.
enum Role {
    Host { server: u32, max_players: usize, next_peer: u32, clients: HashMap<u32, (u32, String)> },
    Client { socket: u32, welcomed: bool },
}

struct Session {
    role: Role,
    me: u32,
    name: String,
    game: String,
    /// Every peer in the game (including me): id → name.
    peers: BTreeMap<u32, String>,
}

pub struct Net {
    socks: HashMap<u32, SockEntry>,
    next_id: u32,
    pub connect_timeout: Duration,
    session: Option<Session>,
    /// Messages for the log (printed by the engine as `[red] ...`).
    pub log: Vec<String>,
}

impl Default for Net {
    fn default() -> Self {
        Net { socks: HashMap::new(), next_id: 0, connect_timeout: Duration::from_millis(5000), session: None, log: vec![] }
    }
}

impl Net {
    pub fn is_idle(&self) -> bool {
        self.socks.is_empty()
    }

    fn add(&mut self, sock: Sock) -> u32 {
        loop {
            self.next_id = self.next_id.wrapping_add(1);
            if self.next_id != 0 && !self.socks.contains_key(&self.next_id) {
                break;
            }
        }
        self.socks.insert(self.next_id, SockEntry { sock, timeout: (0, 0) });
        self.next_id
    }

    pub fn exists(&self, id: u32) -> bool {
        self.socks.contains_key(&id)
    }

    /// `network_create_server[_raw](type, port, max_clients)`.
    pub fn create_server(&mut self, udp: bool, port: u16, max_clients: usize, framed: bool) -> Result<u32, String> {
        if udp {
            return self.create_udp(port);
        }
        let (listeners, port) = bind_both(port, TcpListener::bind, |l: &TcpListener| l.local_addr().ok().map(|a| a.port()))?;
        for l in &listeners {
            l.set_nonblocking(true).map_err(|e| e.to_string())?;
        }
        Ok(self.add(Sock::Server { listeners, framed, max_clients: max_clients.max(1), clients: vec![], port }))
    }

    /// `network_create_socket(network_socket_udp)` / `network_create_socket_ext(udp, port)`.
    pub fn create_udp(&mut self, port: u16) -> Result<u32, String> {
        let (socks, port) = bind_both(port, UdpSocket::bind, |s: &UdpSocket| s.local_addr().ok().map(|a| a.port()))?;
        for s in &socks {
            s.set_nonblocking(true).map_err(|e| e.to_string())?;
        }
        Ok(self.add(Sock::Udp { socks, port }))
    }

    /// `network_create_socket(network_socket_tcp)`: a client socket, connect it later.
    pub fn create_tcp(&mut self) -> u32 {
        self.add(Sock::Tcp(TcpConn {
            state: ConnState::Idle,
            framed: true,
            server: None,
            inbuf: vec![],
            out: vec![],
            peer: None,
            last_recv: Instant::now(),
            out_since: None,
        }))
    }

    /// `network_connect[_raw](socket, host, port)`: starts connecting; the result arrives
    /// as a `NonBlockingConnect` event.
    pub fn connect(&mut self, id: u32, host: &str, port: u16, framed: bool) -> Result<(), String> {
        let timeout = self.connect_timeout;
        let Some(entry) = self.socks.get_mut(&id) else { return Err(format!("socket {id} does not exist")) };
        let Sock::Tcp(c) = &mut entry.sock else { return Err("network_connect needs a TCP socket from network_create_socket(network_socket_tcp)".into()) };
        if !matches!(c.state, ConnState::Idle) {
            return Err("this socket is already connected or connecting".into());
        }
        if host.trim().is_empty() {
            return Err("empty host".into());
        }
        let (tx, rx) = mpsc::channel();
        let (h, p) = (host.trim().to_string(), port);
        std::thread::Builder::new()
            .name("modengine-connect".into())
            .spawn(move || {
                let r = (|| {
                    let addrs: Vec<SocketAddr> = (h.as_str(), p).to_socket_addrs().map_err(|e| format!("cannot find '{h}': {e}"))?.collect();
                    let mut last = format!("'{h}' has no address");
                    for a in addrs {
                        match TcpStream::connect_timeout(&a, timeout) {
                            Ok(s) => return Ok(s),
                            Err(e) => last = format!("{a}: {e}"),
                        }
                    }
                    Err(last)
                })();
                let _ = tx.send(r);
            })
            .map_err(|e| e.to_string())?;
        c.framed = framed;
        c.inbuf.clear();
        c.state = ConnState::Connecting { rx, host: host.trim().to_string(), port };
        Ok(())
    }

    /// `network_destroy(socket)`. A server takes its clients with it.
    pub fn destroy(&mut self, id: u32) -> bool {
        let Some(entry) = self.socks.remove(&id) else { return false };
        match entry.sock {
            Sock::Server { clients, .. } => {
                for c in clients {
                    if let Some(SockEntry { sock: Sock::Tcp(mut conn), .. }) = self.socks.remove(&c) {
                        close_conn(&mut conn);
                    }
                }
            }
            Sock::Tcp(mut conn) => {
                close_conn(&mut conn);
                if let Some(s) = conn.server {
                    if let Some(SockEntry { sock: Sock::Server { clients, .. }, .. }) = self.socks.get_mut(&s) {
                        clients.retain(|c| *c != id);
                    }
                }
            }
            Sock::Udp { .. } => {}
        }
        if let Some(sess) = &self.session {
            let mine = match &sess.role {
                Role::Host { server, .. } => *server == id,
                Role::Client { socket, .. } => *socket == id,
            };
            if mine {
                self.session = None;
            }
        }
        true
    }

    /// Closes every socket (game end, restart).
    pub fn close_all(&mut self) {
        let ids: Vec<u32> = self.socks.keys().copied().collect();
        for id in ids {
            self.destroy(id);
        }
        self.session = None;
    }

    pub fn set_timeout(&mut self, id: u32, read_ms: u64, write_ms: u64) -> bool {
        match self.socks.get_mut(&id) {
            Some(e) => {
                e.timeout = (read_ms, write_ms);
                true
            }
            None => false,
        }
    }

    /// Port a server or UDP socket listens on (useful with port 0 = any free port).
    pub fn local_port(&self, id: u32) -> Option<u16> {
        match &self.socks.get(&id)?.sock {
            Sock::Server { port, .. } | Sock::Udp { port, .. } => Some(*port),
            Sock::Tcp(c) => match &c.state {
                ConnState::Open(s) => s.local_addr().ok().map(|a| a.port()),
                _ => None,
            },
        }
    }

    /// Is this TCP socket connected (or connecting)?
    pub fn status(&self, id: u32) -> &'static str {
        match self.socks.get(&id).map(|e| &e.sock) {
            None => "none",
            Some(Sock::Server { .. }) => "server",
            Some(Sock::Udp { .. }) => "udp",
            Some(Sock::Tcp(c)) => match c.state {
                ConnState::Idle => "closed",
                ConnState::Connecting { .. } => "connecting",
                ConnState::Open(_) => "connected",
            },
        }
    }

    /// `network_send_packet` / `network_send_raw`: queues `data` on a TCP socket.
    /// The framing (or not) is the socket's: framed for `network_create_server` /
    /// `network_connect`, raw for the `_raw` versions.
    pub fn send(&mut self, id: u32, data: &[u8], raw: bool) -> Result<usize, String> {
        let Some(entry) = self.socks.get_mut(&id) else { return Err(format!("socket {id} does not exist")) };
        match &mut entry.sock {
            Sock::Tcp(c) => {
                if matches!(c.state, ConnState::Idle) {
                    return Err(format!("socket {id} is not connected"));
                }
                let framed = c.framed && !raw;
                if framed && data.len() > MAX_PACKET {
                    return Err(format!("packet of {} bytes is bigger than the limit ({MAX_PACKET})", data.len()));
                }
                if c.out.len() + data.len() + 4 > MAX_OUTBUF {
                    return Err(format!("socket {id}: too much unsent data (the other side does not read?)"));
                }
                if framed {
                    c.out.extend_from_slice(&frame(data));
                } else {
                    c.out.extend_from_slice(data);
                }
                if c.out_since.is_none() {
                    c.out_since = Some(Instant::now());
                }
                if flush(c).is_err() {
                    // the next poll notices the broken connection and reports it
                }
                Ok(data.len())
            }
            Sock::Server { .. } => Err("cannot send to a server socket: send to the client socket from the connect event (async_load.socket)".into()),
            Sock::Udp { .. } => Err("UDP sockets send with network_send_udp(socket, host, port, buffer, size)".into()),
        }
    }

    /// `network_send_udp`.
    pub fn send_udp(&mut self, id: u32, host: &str, port: u16, data: &[u8]) -> Result<usize, String> {
        if data.len() > MAX_UDP {
            return Err(format!("UDP packet of {} bytes is bigger than the limit ({MAX_UDP})", data.len()));
        }
        let addr = resolve(host, port)?;
        let Some(SockEntry { sock: Sock::Udp { socks, .. }, .. }) = self.socks.get(&id) else {
            return Err(format!("socket {id} is not a UDP socket"));
        };
        send_to_any(socks, addr, data).map_err(|e| format!("network_send_udp to {addr}: {e}"))
    }

    /// `network_send_broadcast(socket, port, ...)`: a UDP packet to every machine of the LAN.
    pub fn send_broadcast(&mut self, id: u32, port: u16, data: &[u8]) -> Result<usize, String> {
        if data.len() > MAX_UDP {
            return Err(format!("UDP packet of {} bytes is bigger than the limit ({MAX_UDP})", data.len()));
        }
        let Some(SockEntry { sock: Sock::Udp { socks, .. }, .. }) = self.socks.get_mut(&id) else {
            return Err(format!("socket {id} is not a UDP socket"));
        };
        if !socks.iter().any(|s| s.local_addr().map(|a| a.is_ipv4()).unwrap_or(false)) {
            // dual-stack systems only have the IPv6 socket: broadcasts need an IPv4 one
            let s = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).map_err(|e| e.to_string())?;
            s.set_nonblocking(true).map_err(|e| e.to_string())?;
            socks.push(s);
        }
        let s = socks.iter().find(|s| s.local_addr().map(|a| a.is_ipv4()).unwrap_or(false)).expect("ipv4 socket");
        s.set_broadcast(true).map_err(|e| e.to_string())?;
        s.send_to(data, (Ipv4Addr::BROADCAST, port)).map_err(|e| format!("broadcast: {e}"))
    }

    // ------------------------------------------------------------ polling

    /// Accepts, connects, reads and writes everything that is ready. Call once per step.
    pub fn poll(&mut self) -> Vec<NetEvent> {
        let mut out = Vec::new();
        if self.socks.is_empty() {
            return out;
        }
        let now = Instant::now();
        let mut ids: Vec<u32> = self.socks.keys().copied().collect();
        ids.sort();
        let mut closed: Vec<u32> = Vec::new();
        for id in ids {
            let Some(entry) = self.socks.get_mut(&id) else { continue };
            let timeout = entry.timeout;
            match &mut entry.sock {
                Sock::Server { listeners, framed, max_clients, clients, .. } => {
                    let mut accepted = Vec::new();
                    for l in listeners.iter() {
                        loop {
                            match l.accept() {
                                Ok((s, addr)) => accepted.push((s, addr)),
                                Err(e) if would_block(&e) => break,
                                Err(e) => {
                                    self.log.push(format!("server {id}: accept failed: {e}"));
                                    break;
                                }
                            }
                        }
                    }
                    let (framed, max) = (*framed, *max_clients);
                    let mut room = max.saturating_sub(clients.len());
                    let mut new_conns = Vec::new();
                    for (s, addr) in accepted {
                        if room == 0 {
                            self.log.push(format!("server {id}: full ({max} clients), refused {}", ip_text(addr.ip())));
                            let _ = s.shutdown(Shutdown::Both);
                            continue;
                        }
                        if s.set_nonblocking(true).is_err() {
                            continue;
                        }
                        let _ = s.set_nodelay(true);
                        room -= 1;
                        new_conns.push((s, addr));
                    }
                    for (s, addr) in new_conns {
                        let cid = self.add(Sock::Tcp(TcpConn {
                            state: ConnState::Open(s),
                            framed,
                            server: Some(id),
                            inbuf: vec![],
                            out: vec![],
                            peer: Some(addr),
                            last_recv: now,
                            out_since: None,
                        }));
                        if let Some(SockEntry { sock: Sock::Server { clients, .. }, .. }) = self.socks.get_mut(&id) {
                            clients.push(cid);
                        }
                        // the new socket gets polled from the next step on
                        out.push(NetEvent::Connect { server: id, socket: cid, ip: ip_text(addr.ip()), port: addr.port() });
                    }
                }
                Sock::Udp { socks, .. } => {
                    let mut buf = vec![0u8; 65536];
                    for s in socks.iter() {
                        for _ in 0..1024 {
                            match s.recv_from(&mut buf) {
                                Ok((n, addr)) => out.push(NetEvent::Data { socket: id, ip: ip_text(addr.ip()), port: addr.port(), data: buf[..n].to_vec() }),
                                Err(e) if would_block(&e) => break,
                                // Windows reports an ICMP "port unreachable" from an earlier send here
                                Err(e) if matches!(e.kind(), ErrorKind::ConnectionReset | ErrorKind::ConnectionRefused) => continue,
                                Err(e) => {
                                    self.log.push(format!("UDP socket {id}: {e}"));
                                    break;
                                }
                            }
                        }
                    }
                }
                Sock::Tcp(c) => {
                    if let ConnState::Connecting { rx, host, port } = &c.state {
                        let (host, port) = (host.clone(), *port);
                        match rx.try_recv() {
                            Err(TryRecvError::Empty) => continue,
                            Ok(Ok(s)) => {
                                let _ = s.set_nonblocking(true);
                                let _ = s.set_nodelay(true);
                                let peer = s.peer_addr().ok();
                                c.peer = peer;
                                c.last_recv = now;
                                c.state = ConnState::Open(s);
                                let ip = peer.map(|a| ip_text(a.ip())).unwrap_or(host);
                                out.push(NetEvent::NonBlockingConnect { socket: id, ip, port, succeeded: true });
                            }
                            Ok(Err(msg)) => {
                                self.log.push(format!("socket {id}: could not connect to {host}:{port}: {msg}"));
                                c.state = ConnState::Idle;
                                c.out.clear();
                                c.out_since = None;
                                out.push(NetEvent::NonBlockingConnect { socket: id, ip: host, port, succeeded: false });
                                continue;
                            }
                            Err(TryRecvError::Disconnected) => {
                                c.state = ConnState::Idle;
                                out.push(NetEvent::NonBlockingConnect { socket: id, ip: host, port, succeeded: false });
                                continue;
                            }
                        }
                    }
                    if !matches!(c.state, ConnState::Open(_)) {
                        continue;
                    }
                    let (ip, port) = c.peer.map(|a| (ip_text(a.ip()), a.port())).unwrap_or_default();
                    let mut broken: Option<String> = None;
                    if let Err(e) = flush(c) {
                        broken = Some(e);
                    }
                    // read what is there
                    let mut got = Vec::new();
                    if let ConnState::Open(s) = &mut c.state {
                        let mut buf = [0u8; 65536];
                        while got.len() < READ_BUDGET {
                            match s.read(&mut buf) {
                                Ok(0) => {
                                    broken.get_or_insert_with(|| "closed by the other side".into());
                                    break;
                                }
                                Ok(n) => got.extend_from_slice(&buf[..n]),
                                Err(e) if would_block(&e) => break,
                                Err(e) => {
                                    broken.get_or_insert_with(|| e.to_string());
                                    break;
                                }
                            }
                        }
                    }
                    if !got.is_empty() {
                        c.last_recv = now;
                        if c.framed {
                            c.inbuf.extend_from_slice(&got);
                            match unframe(&mut c.inbuf, MAX_PACKET) {
                                Ok(packets) => {
                                    for p in packets {
                                        out.push(NetEvent::Data { socket: id, ip: ip.clone(), port, data: p });
                                    }
                                }
                                Err(msg) => {
                                    c.inbuf.clear();
                                    broken = Some(msg);
                                }
                            }
                        } else {
                            out.push(NetEvent::Data { socket: id, ip: ip.clone(), port, data: got });
                        }
                    }
                    if broken.is_none() && timeout.0 > 0 && now.duration_since(c.last_recv) > Duration::from_millis(timeout.0) {
                        broken = Some(format!("nothing received in {} ms (network_set_timeout)", timeout.0));
                    }
                    if broken.is_none() && timeout.1 > 0 {
                        if let Some(t) = c.out_since {
                            if now.duration_since(t) > Duration::from_millis(timeout.1) {
                                broken = Some(format!("could not send for {} ms (network_set_timeout)", timeout.1));
                            }
                        }
                    }
                    if let Some(why) = broken {
                        if why != "closed by the other side" {
                            self.log.push(format!("socket {id} ({ip}:{port}) disconnected: {why}"));
                        }
                        close_conn(c);
                        let server = c.server.unwrap_or(id);
                        out.push(NetEvent::Disconnect { server, socket: id, ip, port });
                        if c.server.is_some() {
                            closed.push(id);
                        }
                    }
                }
            }
        }
        let out = self.session_filter(out);
        // sockets accepted by a server are gone once disconnected (like GameMaker)
        for id in closed {
            self.destroy(id);
        }
        out
    }

    // ------------------------------------------------------------ session layer (net_*)

    pub fn session_active(&self) -> bool {
        self.session.is_some()
    }
    pub fn session_is_host(&self) -> bool {
        matches!(self.session.as_ref().map(|s| &s.role), Some(Role::Host { .. }))
    }
    pub fn session_me(&self) -> u32 {
        self.session.as_ref().map(|s| s.me).unwrap_or(0)
    }
    /// "off", "connecting" or "online".
    pub fn session_status(&self) -> &'static str {
        match &self.session {
            None => "off",
            Some(Session { role: Role::Client { welcomed: false, .. }, .. }) => "connecting",
            Some(_) => "online",
        }
    }
    pub fn session_peers(&self) -> Vec<u32> {
        self.session.as_ref().map(|s| s.peers.keys().copied().collect()).unwrap_or_default()
    }
    pub fn session_peer_name(&self, id: u32) -> Option<String> {
        self.session.as_ref()?.peers.get(&id).cloned()
    }
    pub fn session_port(&self) -> Option<u16> {
        match &self.session.as_ref()?.role {
            Role::Host { server, .. } => self.local_port(*server),
            Role::Client { .. } => None,
        }
    }

    /// `net_host(port, max_players, name)`: this game becomes the host (peer 1).
    pub fn host(&mut self, port: u16, max_players: usize, name: &str, game: &str) -> Result<u16, String> {
        self.leave();
        let max_players = max_players.clamp(2, 64);
        // a few extra TCP slots so a full game can still answer "full" instead of hanging up
        let server = self.create_server(false, port, max_players + 4, true)?;
        let mut peers = BTreeMap::new();
        peers.insert(1, name.to_string());
        self.session = Some(Session {
            role: Role::Host { server, max_players, next_peer: 2, clients: HashMap::new() },
            me: 1,
            name: name.to_string(),
            game: game.to_string(),
            peers,
        });
        Ok(self.local_port(server).unwrap_or(port))
    }

    /// `net_join(host, port, name)`: starts joining; a "connect" (or "failed") event follows.
    pub fn join(&mut self, host: &str, port: u16, name: &str, game: &str) -> Result<(), String> {
        self.leave();
        let socket = self.create_tcp();
        if let Err(e) = self.connect(socket, host, port, true) {
            self.destroy(socket);
            return Err(e);
        }
        self.session = Some(Session { role: Role::Client { socket, welcomed: false }, me: 0, name: name.to_string(), game: game.to_string(), peers: BTreeMap::new() });
        Ok(())
    }

    /// `net_disconnect()`: leaves the game (a host closes it for everyone).
    pub fn leave(&mut self) {
        let Some(s) = self.session.take() else { return };
        match s.role {
            Role::Host { server, .. } => {
                self.destroy(server);
            }
            Role::Client { socket, .. } => {
                self.destroy(socket);
            }
        }
    }

    /// `net_send(to, tipo, data)`: `to` = a peer id, or None for everybody else.
    pub fn session_send(&mut self, to: Option<u32>, tipo: &str, data: serde_json::Value) -> Result<(), String> {
        let Some(s) = &self.session else { return Err("not in a game: call net_host() or net_join() first".into()) };
        let me = s.me;
        if to == Some(me) && me != 0 {
            return Err("net_send to yourself".into());
        }
        let msg = serde_json::json!({ "op": "msg", "from": me, "to": to.map(|t| t as i64).unwrap_or(-1), "k": tipo, "d": data });
        let bytes = serde_json::to_vec(&msg).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_PACKET {
            return Err(format!("message of {} bytes is bigger than the limit ({MAX_PACKET})", bytes.len()));
        }
        match &s.role {
            Role::Client { socket, welcomed } => {
                if !welcomed {
                    return Err("still connecting: wait for the \"connect\" event".into());
                }
                let socket = *socket;
                self.send(socket, &bytes, false).map(|_| ())
            }
            Role::Host { clients, .. } => {
                let targets: Vec<u32> = match to {
                    None => clients.iter().map(|(sock, _)| *sock).collect(),
                    Some(peer) => match clients.iter().find(|(_, (p, _))| *p == peer) {
                        Some((sock, _)) => vec![*sock],
                        None => return Err(format!("there is no player {peer}")),
                    },
                };
                for t in targets {
                    if let Err(e) = self.send(t, &bytes, false) {
                        self.log.push(format!("net_send: {e}"));
                    }
                }
                Ok(())
            }
        }
    }

    fn sess_json(&mut self, socket: u32, v: serde_json::Value) {
        if let Ok(b) = serde_json::to_vec(&v) {
            if let Err(e) = self.send(socket, &b, false) {
                self.log.push(format!("net: {e}"));
            }
        }
    }

    /// Turns the raw events of the session's sockets into session events.
    fn session_filter(&mut self, raw: Vec<NetEvent>) -> Vec<NetEvent> {
        let Some(mut s) = self.session.take() else { return raw };
        let mut out = Vec::new();
        let mut ended = false;
        for ev in raw {
            match &mut s.role {
                Role::Host { server, max_players, next_peer, clients } => {
                    let server = *server;
                    match ev {
                        NetEvent::Connect { server: sv, .. } if sv == server => {} // wait for its hello
                        NetEvent::Disconnect { server: sv, socket, .. } if sv == server => {
                            if let Some((peer, name)) = clients.remove(&socket) {
                                s.peers.remove(&peer);
                                let leave = serde_json::to_vec(&serde_json::json!({ "op": "leave", "id": peer })).unwrap_or_default();
                                for c in clients.keys() {
                                    let _ = self.send(*c, &leave, false);
                                }
                                out.push(sess_event("disconnect", peer, Some(&name)));
                            }
                        }
                        NetEvent::Data { socket, data, .. } if self.socks.get(&socket).is_some_and(|e| matches!(&e.sock, Sock::Tcp(c) if c.server == Some(server))) => {
                            let Ok(j) = serde_json::from_slice::<serde_json::Value>(&data) else {
                                self.log.push(format!("net: malformed message from socket {socket} (not JSON), ignored"));
                                continue;
                            };
                            match j.get("op").and_then(|o| o.as_str()) {
                                Some("hello") => {
                                    if clients.contains_key(&socket) {
                                        continue;
                                    }
                                    let name = j.get("name").and_then(|n| n.as_str()).unwrap_or("").chars().take(32).collect::<String>();
                                    let game = j.get("game").and_then(|n| n.as_str()).unwrap_or("");
                                    let v = j.get("v").and_then(|n| n.as_u64()).unwrap_or(0);
                                    let reason = if game != s.game {
                                        Some(format!("es otro juego ('{game}', este host juega a '{}')", s.game))
                                    } else if v != SESSION_VERSION as u64 {
                                        Some("versión distinta de Mod Engine".to_string())
                                    } else if s.peers.len() >= *max_players {
                                        Some(format!("la partida está llena ({max_players} jugadores)"))
                                    } else {
                                        None
                                    };
                                    if let Some(reason) = reason {
                                        self.sess_json(socket, serde_json::json!({ "op": "reject", "reason": reason }));
                                        continue;
                                    }
                                    let peer = *next_peer;
                                    *next_peer += 1;
                                    let list: Vec<serde_json::Value> = s.peers.iter().map(|(id, n)| serde_json::json!({ "id": id, "name": n })).collect();
                                    self.sess_json(socket, serde_json::json!({ "op": "welcome", "id": peer, "peers": list }));
                                    let join = serde_json::json!({ "op": "join", "id": peer, "name": name });
                                    for c in clients.keys().copied().collect::<Vec<_>>() {
                                        self.sess_json(c, join.clone());
                                    }
                                    out.push(sess_event("connect", peer, Some(&name)));
                                    clients.insert(socket, (peer, name.clone()));
                                    s.peers.insert(peer, name);
                                }
                                Some("msg") => {
                                    let Some((from, _)) = clients.get(&socket).cloned() else { continue };
                                    let to = j.get("to").and_then(|t| t.as_i64()).unwrap_or(-1);
                                    let tipo = j.get("k").and_then(|k| k.as_str()).unwrap_or("").to_string();
                                    let d = j.get("d").cloned().unwrap_or(serde_json::Value::Null);
                                    // relay with the real sender (no spoofing)
                                    let fwd = serde_json::json!({ "op": "msg", "from": from, "to": to, "k": tipo, "d": d });
                                    if to == -1 || to == 1 {
                                        let name = s.peers.get(&from).cloned().unwrap_or_default();
                                        out.push(NetEvent::Session { kind: "message", peer: from, name, tipo: tipo.clone(), data: d.clone(), reason: String::new() });
                                    }
                                    if to != 1 {
                                        let targets: Vec<u32> = clients.iter().filter(|(c, (p, _))| **c != socket && (to == -1 || *p as i64 == to)).map(|(c, _)| *c).collect();
                                        for t in targets {
                                            self.sess_json(t, fwd.clone());
                                        }
                                    }
                                }
                                Some("bye") => {}
                                _ => self.log.push(format!("net: unknown message from socket {socket}, ignored")),
                            }
                        }
                        other => out.push(other),
                    }
                }
                Role::Client { socket, welcomed } => {
                    let sock = *socket;
                    match ev {
                        NetEvent::NonBlockingConnect { socket: so, succeeded, ip, port } if so == sock => {
                            if succeeded {
                                let hello = serde_json::json!({ "op": "hello", "name": s.name, "game": s.game, "v": SESSION_VERSION });
                                self.sess_json(sock, hello);
                            } else {
                                out.push(sess_failed(format!("no se pudo conectar a {ip}:{port}")));
                                ended = true;
                            }
                        }
                        NetEvent::Disconnect { socket: so, .. } if so == sock => {
                            if *welcomed {
                                let me = s.me;
                                let others: Vec<u32> = s.peers.keys().copied().filter(|p| *p != me && *p != 1).collect();
                                for p in others {
                                    out.push(sess_event("disconnect", p, s.peers.get(&p)));
                                }
                                out.push(sess_event("disconnect", 1, s.peers.get(&1)));
                                s.peers.clear();
                            } else {
                                out.push(sess_failed("el host cerró la conexión".into()));
                            }
                            ended = true;
                        }
                        NetEvent::Data { socket: so, data, .. } if so == sock => {
                            let Ok(j) = serde_json::from_slice::<serde_json::Value>(&data) else {
                                self.log.push("net: malformed message from the host (not JSON), ignored".into());
                                continue;
                            };
                            let id_of = |k: &str| j.get(k).and_then(|v| v.as_u64()).map(|v| v as u32);
                            match j.get("op").and_then(|o| o.as_str()) {
                                Some("welcome") if !*welcomed => {
                                    let Some(me) = id_of("id") else { continue };
                                    *welcomed = true;
                                    s.me = me;
                                    s.peers.clear();
                                    if let Some(list) = j.get("peers").and_then(|p| p.as_array()) {
                                        for p in list {
                                            if let Some(id) = p.get("id").and_then(|v| v.as_u64()) {
                                                let n = p.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                                s.peers.insert(id as u32, n);
                                            }
                                        }
                                    }
                                    s.peers.insert(me, s.name.clone());
                                    let others: Vec<u32> = s.peers.keys().copied().filter(|p| *p != me).collect();
                                    for p in others {
                                        out.push(sess_event("connect", p, s.peers.get(&p)));
                                    }
                                }
                                Some("reject") => {
                                    let r = j.get("reason").and_then(|r| r.as_str()).unwrap_or("rechazado").to_string();
                                    out.push(sess_failed(r));
                                    ended = true;
                                }
                                Some("join") if *welcomed => {
                                    if let Some(id) = id_of("id") {
                                        let n = j.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                        out.push(sess_event("connect", id, Some(&n)));
                                        s.peers.insert(id, n);
                                    }
                                }
                                Some("leave") if *welcomed => {
                                    if let Some(id) = id_of("id") {
                                        if let Some(n) = s.peers.remove(&id) {
                                            out.push(sess_event("disconnect", id, Some(&n)));
                                        }
                                    }
                                }
                                Some("msg") if *welcomed => {
                                    let from = id_of("from").unwrap_or(0);
                                    let tipo = j.get("k").and_then(|k| k.as_str()).unwrap_or("").to_string();
                                    let d = j.get("d").cloned().unwrap_or(serde_json::Value::Null);
                                    let name = s.peers.get(&from).cloned().unwrap_or_default();
                                    out.push(NetEvent::Session { kind: "message", peer: from, name, tipo, data: d, reason: String::new() });
                                }
                                _ => self.log.push("net: unexpected message from the host, ignored".into()),
                            }
                        }
                        other => out.push(other),
                    }
                }
            }
            if ended {
                break;
            }
        }
        if ended {
            let sock = match s.role {
                Role::Client { socket, .. } => Some(socket),
                Role::Host { .. } => None,
            };
            if let Some(sock) = sock {
                self.destroy(sock);
            }
        } else {
            self.session = Some(s);
        }
        out
    }
}

fn sess_event(kind: &'static str, peer: u32, name: Option<&String>) -> NetEvent {
    NetEvent::Session { kind, peer, name: name.cloned().unwrap_or_default(), tipo: String::new(), data: serde_json::Value::Null, reason: String::new() }
}

fn sess_failed(reason: String) -> NetEvent {
    NetEvent::Session { kind: "failed", peer: 0, name: String::new(), tipo: String::new(), data: serde_json::Value::Null, reason }
}

fn flush(c: &mut TcpConn) -> Result<(), String> {
    let ConnState::Open(s) = &mut c.state else { return Ok(()) };
    let mut sent = 0;
    while sent < c.out.len() {
        match s.write(&c.out[sent..]) {
            Ok(0) => return Err("connection closed while sending".into()),
            Ok(n) => sent += n,
            Err(e) if would_block(&e) => break,
            Err(e) => {
                c.out.drain(..sent);
                return Err(e.to_string());
            }
        }
    }
    c.out.drain(..sent);
    if c.out.is_empty() {
        c.out_since = None;
    } else if sent > 0 {
        c.out_since = Some(Instant::now());
    }
    Ok(())
}

/// Best effort: send what is queued, then close.
fn close_conn(c: &mut TcpConn) {
    let _ = flush(c);
    if let ConnState::Open(s) = &c.state {
        let _ = s.shutdown(Shutdown::Both);
    }
    c.state = ConnState::Idle;
    c.out.clear();
    c.out_since = None;
    c.inbuf.clear();
}

fn resolve(host: &str, port: u16) -> Result<SocketAddr, String> {
    let host = host.trim();
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, port));
    }
    (host, port).to_socket_addrs().map_err(|e| format!("cannot find '{host}': {e}"))?.next().ok_or_else(|| format!("'{host}' has no address"))
}

/// Does this script (or room code) use networking? Decides the Android INTERNET permission.
pub fn source_uses_network(src: &str) -> bool {
    ["network_create_server", "network_create_socket", "network_connect", "net_host", "net_join", "network_send"].iter().any(|k| src.contains(k))
}

/// `network_resolve(host)`: the first IP address of a host name.
pub fn resolve_ip(host: &str) -> Option<String> {
    resolve(host, 0).ok().map(|a| ip_text(a.ip()))
}

fn send_to_any(socks: &[UdpSocket], addr: SocketAddr, data: &[u8]) -> std::io::Result<usize> {
    let fam = |s: &UdpSocket| s.local_addr().map(|a| a.is_ipv4()).unwrap_or(false);
    if let Some(s) = socks.iter().find(|s| fam(s) == addr.is_ipv4()) {
        return s.send_to(data, addr);
    }
    // only a dual-stack IPv6 socket: IPv4 addresses go as ::ffff:a.b.c.d
    match (addr, socks.first()) {
        (SocketAddr::V4(v4), Some(s)) => s.send_to(data, SocketAddr::new(IpAddr::V6(v4.ip().to_ipv6_mapped()), v4.port())),
        (_, Some(s)) => s.send_to(data, addr),
        _ => Err(std::io::Error::new(ErrorKind::NotConnected, "no socket")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framing_handles_partial_and_coalesced_packets() {
        let mut stream = Vec::new();
        stream.extend(frame(b"hello"));
        stream.extend(frame(b""));
        stream.extend(frame(&[7u8; 300]));
        // feed it one byte at a time: packets appear only when complete
        let mut inbuf = Vec::new();
        let mut got = Vec::new();
        for b in &stream {
            inbuf.push(*b);
            got.extend(unframe(&mut inbuf, MAX_PACKET).unwrap());
        }
        assert_eq!(got, vec![b"hello".to_vec(), vec![], vec![7u8; 300]]);
        assert!(inbuf.is_empty());
        // all at once (coalesced) plus half of another
        let mut inbuf = stream.clone();
        inbuf.extend(&frame(b"next")[..5]);
        let got = unframe(&mut inbuf, MAX_PACKET).unwrap();
        assert_eq!(got.len(), 3);
        assert_eq!(inbuf.len(), 5);
        inbuf.extend(&frame(b"next")[5..]);
        assert_eq!(unframe(&mut inbuf, MAX_PACKET).unwrap(), vec![b"next".to_vec()]);
    }

    #[test]
    fn oversized_or_garbage_lengths_are_errors_not_panics() {
        let mut inbuf = vec![0xff, 0xff, 0xff, 0xff, 1, 2, 3];
        assert!(unframe(&mut inbuf, MAX_PACKET).is_err());
        let mut inbuf = (2_000_000u32).to_le_bytes().to_vec();
        assert!(unframe(&mut inbuf, MAX_PACKET).is_err());
        let mut short = vec![1, 0];
        assert!(unframe(&mut short, MAX_PACKET).unwrap().is_empty());
    }

    #[test]
    fn detects_games_that_need_the_internet_permission() {
        assert!(source_uses_network("on create { net_host(6510) }"));
        assert!(source_uses_network("global.s = network_create_socket(network_socket_udp)"));
        assert!(source_uses_network("network_connect_raw(s, \"a\", 1)"));
        assert!(!source_uses_network("on step { x += 1 } // no network here"));
        assert!(!source_uses_network("var buffer = buffer_create(4, buffer_grow, 1)"));
    }

    #[test]
    fn mapped_ipv6_addresses_read_as_ipv4() {
        assert_eq!(ip_text("::ffff:192.168.1.5".parse().unwrap()), "192.168.1.5");
        assert_eq!(ip_text("::1".parse().unwrap()), "::1");
        assert_eq!(ip_text("10.0.0.1".parse().unwrap()), "10.0.0.1");
    }
}
