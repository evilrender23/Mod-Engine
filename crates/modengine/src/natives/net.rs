//! Buffers (`buffer_*`), sockets (`network_*`, GameMaker style), the easy multiplayer layer
//! (`net_*`) and typed text (`keyboard_string`).

use super::core::{from_json, to_json};
use super::{Reg, A};
use crate::buffer::{base64_decode, base64_encode, BufKind, BufType, BufVal, Buffer};
use crate::engine::Engine;
use crate::net::{self, NetEvent};
use modscript::{intern, RtError, RtResult, Value};
use std::path::PathBuf;

fn err<T>(msg: impl Into<String>) -> RtResult<T> {
    Err(RtError::new(msg))
}

fn buf_id(a: &A, i: usize) -> RtResult<u32> {
    let n = a.num(i)?;
    if n < 0.0 || n.fract() != 0.0 || n > u32::MAX as f64 {
        return err(format!("{n} is not a buffer"));
    }
    Ok(n as u32)
}

fn buf<'e>(e: &'e mut Engine, a: &A, i: usize) -> RtResult<&'e mut Buffer> {
    let id = buf_id(a, i)?;
    e.buffers.get_mut(id).ok_or_else(|| RtError::new(format!("buffer {id} does not exist (deleted?)")))
}

fn buf_type(a: &A, i: usize) -> RtResult<BufType> {
    let n = a.int(i)?;
    BufType::from_num(n).ok_or_else(|| RtError::new(format!("{n} is not a buffer type (use buffer_u8, buffer_s32, buffer_string...)")))
}

fn to_bufval(v: &Value) -> BufVal {
    match v {
        Value::Bool(b) => BufVal::Bool(*b),
        Value::Num(n) => BufVal::Num(*n),
        Value::Null => BufVal::Num(0.0),
        other => BufVal::Str(other.to_string()),
    }
}

fn from_bufval(v: BufVal) -> Value {
    match v {
        BufVal::Num(n) => Value::Num(n),
        BufVal::Bool(b) => Value::Bool(b),
        BufVal::Str(s) => Value::string(s),
    }
}

/// A file name inside the save folder (no absolute paths, no `..`).
fn save_path(e: &Engine, name: &str) -> RtResult<PathBuf> {
    let p = std::path::Path::new(name);
    if name.is_empty() || p.is_absolute() || p.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
        return err(format!("'{name}': use a simple file name (it goes in the save folder)"));
    }
    Ok(e.save_dir.join(p))
}

fn sock_id(a: &A, i: usize) -> RtResult<u32> {
    let n = a.num(i)?;
    if n < 0.0 || n.fract() != 0.0 || n > u32::MAX as f64 {
        return err(format!("{n} is not a socket"));
    }
    Ok(n as u32)
}

fn port_arg(a: &A, i: usize) -> RtResult<u16> {
    let n = a.num(i)?;
    if !(0.0..=65535.0).contains(&n) || n.fract() != 0.0 {
        return err(format!("port must be a whole number from 0 to 65535, got {n}"));
    }
    Ok(n as u16)
}

/// Bytes `[0, size)` of a buffer (size defaults to the whole buffer).
fn buf_bytes(e: &mut Engine, a: &A, bi: usize, si: usize) -> RtResult<Vec<u8>> {
    let size = match a.opt(si) {
        Some(_) => a.num(si)?.max(0.0) as usize,
        None => usize::MAX,
    };
    let b = buf(e, a, bi)?;
    let size = if size == usize::MAX { b.size() } else { size };
    if size > b.size() {
        return err(format!("size {size} is bigger than the buffer ({} bytes)", b.size()));
    }
    Ok(b.slice(0, size).to_vec())
}

fn log_net(e: &mut Engine, msg: String) -> Value {
    e.net.log.push(msg);
    Value::Num(-1.0)
}

/// The game's name, so hosts only accept players of the same game.
fn game_id(e: &Engine) -> String {
    e.config.game.name.clone()
}

pub(crate) fn register(r: &mut Reg) {
    // ---------------------------------------------------------------- buffers
    r.f("buffer_create", |e, _, a| {
        let a = A(a);
        let size = a.num(0)?;
        if size < 0.0 {
            return err("buffer size can't be negative");
        }
        let kind = BufKind::from_num(a.int(1)?).ok_or_else(|| RtError::new("type must be buffer_fixed, buffer_grow, buffer_wrap or buffer_fast"))?;
        let align = a.num_or(2, 1.0)?.max(1.0) as usize;
        let b = Buffer::new(size as usize, kind, align).map_err(RtError::new)?;
        Ok(Value::Num(e.buffers.add(b) as f64))
    });
    r.f("buffer_delete", |e, _, a| Ok(Value::Bool(e.buffers.remove(buf_id(&A(a), 0)?))));
    r.f("buffer_exists", |e, _, a| {
        let a = A(a);
        Ok(Value::Bool(match a.val(0)?.as_num() {
            Some(n) if n >= 0.0 && n.fract() == 0.0 => e.buffers.get(n as u32).is_some(),
            _ => false,
        }))
    });
    r.f("buffer_write", |e, _, a| {
        let a = A(a);
        let t = buf_type(&a, 1)?;
        let v = to_bufval(a.val(2)?);
        buf(e, &a, 0)?.write(t, &v).map_err(RtError::new)?;
        Ok(Value::Num(0.0))
    });
    r.f("buffer_read", |e, _, a| {
        let a = A(a);
        let t = buf_type(&a, 1)?;
        Ok(from_bufval(buf(e, &a, 0)?.read(t).map_err(RtError::new)?))
    });
    r.f("buffer_peek", |e, _, a| {
        let a = A(a);
        let off = a.num(1)?.max(0.0) as usize;
        let t = buf_type(&a, 2)?;
        Ok(from_bufval(buf(e, &a, 0)?.peek(off, t).map_err(RtError::new)?))
    });
    r.f("buffer_poke", |e, _, a| {
        let a = A(a);
        let off = a.num(1)?.max(0.0) as usize;
        let t = buf_type(&a, 2)?;
        let v = to_bufval(a.val(3)?);
        buf(e, &a, 0)?.poke(off, t, &v).map_err(RtError::new)?;
        Ok(Value::Num(0.0))
    });
    r.f("buffer_fill", |e, _, a| {
        // buffer_fill(buf, offset, type, value, size)
        let a = A(a);
        let off = a.num(1)?.max(0.0) as usize;
        let t = buf_type(&a, 2)?;
        let v = to_bufval(a.val(3)?);
        let size = a.num(4)?.max(0.0) as usize;
        let bytes = Buffer::encode(t, &v).map_err(RtError::new)?;
        let b = buf(e, &a, 0)?;
        if bytes.is_empty() {
            return Ok(Value::Num(0.0));
        }
        let end = off.saturating_add(size).min(b.size());
        let mut p = off;
        while p + bytes.len() <= end {
            b.data[p..p + bytes.len()].copy_from_slice(&bytes);
            p += bytes.len();
        }
        b.used = b.used.max(p.min(b.size()));
        Ok(Value::Num(0.0))
    });
    r.f("buffer_seek", |e, _, a| {
        let a = A(a);
        let base = a.int(1)?;
        let off = a.int(2)?;
        buf(e, &a, 0)?.seek(base, off).map_err(RtError::new)?;
        Ok(Value::Null)
    });
    r.f("buffer_tell", |e, _, a| Ok(Value::Num(buf(e, &A(a), 0)?.pos as f64)));
    r.f("buffer_get_size", |e, _, a| Ok(Value::Num(buf(e, &A(a), 0)?.size() as f64)));
    r.f("buffer_get_used_size", |e, _, a| Ok(Value::Num(buf(e, &A(a), 0)?.used as f64)));
    r.f("buffer_get_type", |e, _, a| Ok(Value::Num(buf(e, &A(a), 0)?.kind.num() as f64)));
    r.f("buffer_get_alignment", |e, _, a| Ok(Value::Num(buf(e, &A(a), 0)?.align as f64)));
    r.f("buffer_resize", |e, _, a| {
        let a = A(a);
        let size = a.num(1)?.max(0.0) as usize;
        buf(e, &a, 0)?.resize(size).map_err(RtError::new)?;
        Ok(Value::Null)
    });
    r.f("buffer_sizeof", |_, _, a| Ok(Value::Num(buf_type(&A(a), 0)?.size() as f64)));
    r.f("buffer_copy", |e, _, a| {
        // buffer_copy(src, src_offset, size, dest, dest_offset)
        let a = A(a);
        let (so, size, dof) = (a.num(1)?.max(0.0) as usize, a.num(2)?.max(0.0) as usize, a.num(4)?.max(0.0) as usize);
        let bytes = buf(e, &a, 0)?.slice(so, size).to_vec();
        let d = buf(e, &a, 3)?;
        let need = dof + bytes.len();
        if need > d.size() {
            if d.kind == BufKind::Grow {
                d.resize(need).map_err(RtError::new)?;
            } else {
                return err(format!("buffer_copy: {} bytes at {dof} do not fit in the destination ({} bytes)", bytes.len(), d.size()));
            }
        }
        d.data[dof..need].copy_from_slice(&bytes);
        d.used = d.used.max(need);
        Ok(Value::Null)
    });
    r.f("buffer_base64_encode", |e, _, a| {
        let a = A(a);
        let b = buf(e, &a, 0)?;
        let off = a.num_or(1, 0.0)?.max(0.0) as usize;
        let size = match a.opt(2) {
            Some(_) => a.num(2)?.max(0.0) as usize,
            None => b.size(),
        };
        Ok(Value::string(base64_encode(b.slice(off, size))))
    });
    r.f("buffer_base64_decode", |e, _, a| {
        let bytes = base64_decode(&A(a).str(0)?).map_err(RtError::new)?;
        Ok(Value::Num(e.buffers.add(Buffer::from_bytes(bytes, BufKind::Grow)) as f64))
    });
    r.f("buffer_save", |e, _, a| {
        let a = A(a);
        let path = save_path(e, &a.str(1)?)?;
        let bytes = buf(e, &a, 0)?.data.clone();
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        Ok(Value::Bool(std::fs::write(path, bytes).is_ok()))
    });
    r.f("buffer_save_ext", |e, _, a| {
        // buffer_save_ext(buf, file, offset, size)
        let a = A(a);
        let path = save_path(e, &a.str(1)?)?;
        let (off, size) = (a.num(2)?.max(0.0) as usize, a.num(3)?.max(0.0) as usize);
        let bytes = buf(e, &a, 0)?.slice(off, size).to_vec();
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        Ok(Value::Bool(std::fs::write(path, bytes).is_ok()))
    });
    r.f("buffer_load", |e, _, a| {
        let path = save_path(e, &A(a).str(0)?)?;
        match std::fs::read(path) {
            Ok(bytes) if bytes.len() <= crate::buffer::MAX_BUFFER_SIZE => Ok(Value::Num(e.buffers.add(Buffer::from_bytes(bytes, BufKind::Grow)) as f64)),
            _ => Ok(Value::Num(-1.0)),
        }
    });

    // ---------------------------------------------------------------- sockets (GameMaker style)
    for (name, framed) in [("network_create_server", true), ("network_create_server_raw", false)] {
        let f: super::NativeFn = if framed {
            |e, _, a| create_server(e, &A(a), true)
        } else {
            |e, _, a| create_server(e, &A(a), false)
        };
        r.f(name, f);
    }
    r.f("network_create_socket", |e, _, a| create_socket(e, A(a).int(0)?, 0));
    r.f("network_create_socket_ext", |e, _, a| {
        let a = A(a);
        create_socket(e, a.int(0)?, port_arg(&a, 1)?)
    });
    for name in ["network_connect", "network_connect_async"] {
        r.f(name, |e, _, a| connect(e, &A(a), true));
    }
    for name in ["network_connect_raw", "network_connect_raw_async"] {
        r.f(name, |e, _, a| connect(e, &A(a), false));
    }
    r.f("network_send_packet", |e, _, a| send(e, &A(a), false));
    r.f("network_send_raw", |e, _, a| send(e, &A(a), true));
    for name in ["network_send_udp", "network_send_udp_raw"] {
        r.f(name, |e, _, a| {
            // network_send_udp(socket, host, port, buffer, size)
            let a = A(a);
            let (id, host, port) = (sock_id(&a, 0)?, a.text(1)?, port_arg(&a, 2)?);
            let bytes = buf_bytes(e, &a, 3, 4)?;
            Ok(match e.net.send_udp(id, &host, port, &bytes) {
                Ok(n) => Value::Num(n as f64),
                Err(m) => log_net(e, m),
            })
        });
    }
    r.f("network_send_broadcast", |e, _, a| {
        // network_send_broadcast(socket, port, buffer, size)
        let a = A(a);
        let (id, port) = (sock_id(&a, 0)?, port_arg(&a, 1)?);
        let bytes = buf_bytes(e, &a, 2, 3)?;
        Ok(match e.net.send_broadcast(id, port, &bytes) {
            Ok(n) => Value::Num(n as f64),
            Err(m) => log_net(e, m),
        })
    });
    r.f("network_destroy", |e, _, a| Ok(Value::Bool(e.net.destroy(sock_id(&A(a), 0)?))));
    r.f("network_set_timeout", |e, _, a| {
        let a = A(a);
        let id = sock_id(&a, 0)?;
        let (rd, wr) = (a.num(1)?.max(0.0) as u64, a.num_or(2, 0.0)?.max(0.0) as u64);
        Ok(Value::Bool(e.net.set_timeout(id, rd, wr)))
    });
    r.f("network_set_config", |e, _, a| {
        let a = A(a);
        Ok(Value::Num(match a.int(0)? {
            net::CONFIG_CONNECT_TIMEOUT => {
                e.net.connect_timeout = std::time::Duration::from_millis(a.num(1)?.clamp(100.0, 120_000.0) as u64);
                0.0
            }
            // always non-blocking; the rest has no meaning here
            net::CONFIG_USE_NON_BLOCKING_SOCKET
            | net::CONFIG_ENABLE_RELIABLE_UDP
            | net::CONFIG_DISABLE_RELIABLE_UDP
            | net::CONFIG_AVOID_TIME_WAIT
            | net::CONFIG_ENABLE_MULTICAST
            | net::CONFIG_DISABLE_MULTICAST => 0.0,
            _ => -1.0,
        }))
    });
    r.f("network_resolve", |_, _, a| Ok(Value::string(net::resolve_ip(&A(a).text(0)?).unwrap_or_default())));
    r.f("network_get_port", |e, _, a| Ok(Value::Num(e.net.local_port(sock_id(&A(a), 0)?).map(|p| p as f64).unwrap_or(-1.0))));
    r.f("network_get_status", |e, _, a| Ok(Value::str(e.net.status(sock_id(&A(a), 0)?))));
    r.f("network_exists", |e, _, a| Ok(Value::Bool(A(a).val(0)?.as_num().is_some_and(|n| n >= 0.0 && e.net.exists(n as u32)))));

    // ---------------------------------------------------------------- easy multiplayer (net_*)
    r.f("net_host", |e, _, a| {
        let a = A(a);
        let port = port_arg(&a, 0)?;
        let max = a.num_or(1, 4.0)?.max(2.0) as usize;
        let name = match a.opt(2) {
            Some(_) => a.text(2)?,
            None => "Host".into(),
        };
        let game = game_id(e);
        Ok(match e.net.host(port, max, &name, &game) {
            Ok(p) => Value::Num(p as f64),
            Err(m) => {
                e.net.log.push(format!("net_host({port}): {m}"));
                Value::Bool(false)
            }
        })
    });
    r.f("net_join", |e, _, a| {
        let a = A(a);
        let host = a.text(0)?;
        let port = port_arg(&a, 1)?;
        let name = match a.opt(2) {
            Some(_) => a.text(2)?,
            None => "Jugador".into(),
        };
        let game = game_id(e);
        Ok(Value::Bool(match e.net.join(&host, port, &name, &game) {
            Ok(()) => true,
            Err(m) => {
                e.net.log.push(format!("net_join({host}, {port}): {m}"));
                false
            }
        }))
    });
    r.f("net_send", |e, _, a| {
        let a = A(a);
        let to = match a.val(0)? {
            Value::All => None,
            Value::Num(n) if *n < 0.0 => None,
            Value::Num(n) if n.fract() == 0.0 => Some(*n as u32),
            other => return err(format!("net_send(): send to a player id or all, got {}", other.type_name())),
        };
        let tipo = a.text(1)?;
        let data = match a.opt(2) {
            Some(v) => to_json(v)?,
            None => serde_json::Value::Null,
        };
        if !e.net.session_active() {
            return Ok(Value::Bool(false));
        }
        match e.net.session_send(to, &tipo, data) {
            Ok(()) => Ok(Value::Bool(true)),
            Err(m) => {
                e.net.log.push(format!("net_send: {m}"));
                Ok(Value::Bool(false))
            }
        }
    });
    r.f("net_peers", |e, _, _| Ok(Value::array(e.net.session_peers().into_iter().map(|p| Value::Num(p as f64)).collect())));
    r.f("net_peer_name", |e, _, a| {
        let id = A(a).num(0)?;
        Ok(Value::string(if id >= 0.0 { e.net.session_peer_name(id as u32).unwrap_or_default() } else { String::new() }))
    });
    r.f("net_my_id", |e, _, _| Ok(Value::Num(e.net.session_me() as f64)));
    r.f("net_is_host", |e, _, _| Ok(Value::Bool(e.net.session_is_host())));
    r.f("net_status", |e, _, _| Ok(Value::str(e.net.session_status())));
    r.f("net_is_online", |e, _, _| Ok(Value::Bool(e.net.session_status() == "online")));
    r.f("net_port", |e, _, _| Ok(Value::Num(e.net.session_port().map(|p| p as f64).unwrap_or(-1.0))));
    r.f("net_disconnect", |e, _, _| {
        e.net.leave();
        Ok(Value::Null)
    });

    // ---------------------------------------------------------------- typed text
    r.f("keyboard_string", |e, _, _| Ok(Value::string(e.keyboard_string.clone())));
    r.f("keyboard_string_set", |e, _, a| {
        e.keyboard_string = A(a).text(0)?.chars().take(KEYBOARD_MAX).collect();
        Ok(Value::Null)
    });
}

/// Characters `keyboard_string` keeps (GameMaker keeps 1024).
pub const KEYBOARD_MAX: usize = 1024;

fn create_server(e: &mut Engine, a: &A, framed: bool) -> RtResult<Value> {
    let kind = a.int(0)?;
    let port = port_arg(a, 1)?;
    let max = a.num_or(2, 32.0)?.max(1.0) as usize;
    let udp = match kind {
        net::SOCKET_TCP => false,
        net::SOCKET_UDP => true,
        net::SOCKET_WS => return Ok(log_net(e, "network_socket_ws (WebSocket) is not supported yet: use network_socket_tcp".into())),
        _ => return err("type must be network_socket_tcp or network_socket_udp"),
    };
    Ok(match e.net.create_server(udp, port, max, framed) {
        Ok(id) => Value::Num(id as f64),
        Err(m) => log_net(e, format!("network_create_server: {m}")),
    })
}

fn create_socket(e: &mut Engine, kind: i64, port: u16) -> RtResult<Value> {
    Ok(match kind {
        net::SOCKET_TCP => Value::Num(e.net.create_tcp() as f64),
        net::SOCKET_UDP => match e.net.create_udp(port) {
            Ok(id) => Value::Num(id as f64),
            Err(m) => log_net(e, format!("network_create_socket: {m}")),
        },
        net::SOCKET_WS => log_net(e, "network_socket_ws (WebSocket) is not supported yet: use network_socket_tcp".into()),
        _ => return err("type must be network_socket_tcp or network_socket_udp"),
    })
}

fn connect(e: &mut Engine, a: &A, framed: bool) -> RtResult<Value> {
    let id = sock_id(a, 0)?;
    let host = a.text(1)?;
    let port = port_arg(a, 2)?;
    Ok(match e.net.connect(id, &host, port, framed) {
        Ok(()) => Value::Num(0.0),
        Err(m) => log_net(e, format!("network_connect({host}, {port}): {m}")),
    })
}

fn send(e: &mut Engine, a: &A, raw: bool) -> RtResult<Value> {
    // network_send_packet(socket, buffer, size)
    let id = sock_id(a, 0)?;
    let bytes = buf_bytes(e, a, 1, 2)?;
    Ok(match e.net.send(id, &bytes, raw) {
        Ok(n) => Value::Num(n as f64),
        Err(m) => log_net(e, format!("network_send: {m}")),
    })
}

/// Network events of this step, as (`async_load` value, temporary buffer to delete after).
pub(crate) fn poll(e: &mut Engine) -> Vec<(Value, Option<u32>)> {
    let events = e.net.poll();
    for m in e.net.log.drain(..) {
        println!("[red] {m}");
    }
    let mut out = Vec::new();
    for ev in events {
        let n = |x: f64| Value::Num(x);
        // every field is always there, so `async_load.buffer` never fails
        let mut kind = "socket";
        let ty: i64;
        let (mut id, mut socket, mut ip, mut port) = (-1.0, -1.0, String::new(), 0u16);
        let (mut buffer, mut size, mut succeeded) = (-1.0, 0.0, true);
        let (mut peer, mut peer_name, mut tipo, mut data, mut reason) = (0u32, String::new(), String::new(), Value::Null, String::new());
        let mut tmp = None;
        match ev {
            NetEvent::Connect { server, socket: s, ip: i, port: p } => {
                (ty, id, socket, ip, port) = (net::TYPE_CONNECT, server as f64, s as f64, i, p);
            }
            NetEvent::Disconnect { server, socket: s, ip: i, port: p } => {
                (ty, id, socket, ip, port) = (net::TYPE_DISCONNECT, server as f64, s as f64, i, p);
            }
            NetEvent::Data { socket: s, ip: i, port: p, data: bytes } => {
                (ty, id, socket, ip, port) = (net::TYPE_DATA, s as f64, s as f64, i, p);
                size = bytes.len() as f64;
                let b = e.buffers.add(Buffer::from_bytes(bytes, BufKind::Fixed));
                buffer = b as f64;
                tmp = Some(b);
            }
            NetEvent::NonBlockingConnect { socket: s, ip: i, port: p, succeeded: ok } => {
                (ty, id, socket, ip, port, succeeded) = (net::TYPE_NON_BLOCKING_CONNECT, s as f64, s as f64, i, p, ok);
            }
            NetEvent::Session { kind: k, peer: pe, name: nm, tipo: t, data: d, reason: r } => {
                kind = k;
                peer = pe;
                peer_name = nm;
                tipo = t;
                data = from_json(&d);
                reason = r;
                ty = match k {
                    "connect" => net::TYPE_CONNECT,
                    "disconnect" => net::TYPE_DISCONNECT,
                    "message" => net::TYPE_DATA,
                    _ => net::TYPE_NON_BLOCKING_CONNECT,
                };
                succeeded = k != "failed";
            }
        }
        let fields = [
            ("type", n(ty as f64)),
            ("id", n(id)),
            ("socket", n(socket)),
            ("ip", Value::string(ip)),
            ("port", n(port as f64)),
            ("buffer", n(buffer)),
            ("size", n(size)),
            ("succeeded", Value::Bool(succeeded)),
            ("kind", Value::str(kind)),
            ("peer", n(peer as f64)),
            ("peer_name", Value::string(peer_name)),
            ("tipo", Value::string(tipo)),
            ("data", data),
            ("reason", Value::string(reason)),
        ];
        out.push((Value::new_struct(fields.into_iter().map(|(k, v)| (intern(k), v))), tmp));
    }
    out
}
