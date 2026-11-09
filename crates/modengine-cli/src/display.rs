//! Picks how the window reaches the desktop on Linux.
//!
//! miniquad opens X11 by default and aborts ("XOpenDisplay() failed!") when `DISPLAY` is not
//! set. That happens on Wayland sessions that run XWayland without exporting `DISPLAY` to the
//! programs they start, and then to every game the editor launches. So when `DISPLAY` is
//! missing: use an X server that is listening anyway, and without one go through Wayland.
//! `MODENGINE_BACKEND=x11|wayland` forces one of the two.

use macroquad::window::Conf;

/// What [`prepare`] decided.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Choice {
    /// Leave everything as it is (X11 through the `DISPLAY` already set).
    Keep,
    /// X11 through this display number, found listening although `DISPLAY` was not set.
    X11(u32),
    /// The Wayland backend.
    Wayland,
}

/// The decision, from the environment: `forced` is `MODENGINE_BACKEND`, `display` is
/// `DISPLAY`, `x_servers` the display numbers that accept connections, `wayland` whether a
/// Wayland compositor is reachable.
pub fn choose(forced: Option<&str>, display: Option<&str>, x_servers: &[u32], wayland: bool) -> Choice {
    let has_display = display.is_some_and(|d| !d.trim().is_empty());
    let first_x = x_servers.iter().min().copied();
    match forced.map(|f| f.trim().to_ascii_lowercase()).as_deref() {
        Some("wayland") => return Choice::Wayland,
        Some("x11") => return if has_display { Choice::Keep } else { first_x.map_or(Choice::Keep, Choice::X11) },
        _ => {}
    }
    if has_display {
        return Choice::Keep;
    }
    match first_x {
        Some(n) => Choice::X11(n),
        None if wayland => Choice::Wayland,
        None => Choice::Keep,
    }
}

/// Applies the decision before a window is opened: sets `DISPLAY` or switches `conf` to the
/// Wayland backend. Does nothing outside Linux.
pub fn prepare(conf: &mut Conf) {
    #[cfg(target_os = "linux")]
    {
        let forced = std::env::var("MODENGINE_BACKEND").ok();
        let display = std::env::var("DISPLAY").ok();
        match choose(forced.as_deref(), display.as_deref(), &linux::x_servers(), linux::has_wayland()) {
            Choice::Keep => {}
            // called on the main thread right before the window exists; nothing else is
            // reading the environment at this point
            Choice::X11(n) => std::env::set_var("DISPLAY", format!(":{n}")),
            Choice::Wayland => conf.platform.linux_backend = macroquad::miniquad::conf::LinuxBackend::WaylandOnly,
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = conf;
}

/// False when no window can be opened at all (no X server and no Wayland compositor), so the
/// caller can report it instead of letting miniquad abort.
pub fn available() -> bool {
    #[cfg(target_os = "linux")]
    {
        std::env::var("DISPLAY").is_ok_and(|d| !d.trim().is_empty()) || !linux::x_servers().is_empty() || linux::has_wayland()
    }
    #[cfg(not(target_os = "linux"))]
    true
}

#[cfg(target_os = "linux")]
mod linux {
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;

    /// Display numbers of the X servers of this machine that accept a connection.
    pub fn x_servers() -> Vec<u32> {
        let Ok(dir) = std::fs::read_dir("/tmp/.X11-unix") else { return Vec::new() };
        let mut found: Vec<u32> = dir
            .flatten()
            .filter_map(|f| {
                let n: u32 = f.file_name().to_str()?.strip_prefix('X')?.parse().ok()?;
                UnixStream::connect(f.path()).ok().map(|_| n)
            })
            .collect();
        found.sort_unstable();
        found
    }

    /// Whether the Wayland socket named by the environment (`wayland-0` by default) is there.
    pub fn has_wayland() -> bool {
        let name = std::env::var("WAYLAND_DISPLAY").ok().filter(|n| !n.is_empty()).unwrap_or_else(|| "wayland-0".into());
        let path = PathBuf::from(&name);
        if path.is_absolute() {
            return path.exists();
        }
        std::env::var_os("XDG_RUNTIME_DIR").is_some_and(|d| PathBuf::from(d).join(name).exists())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_display_already_set_is_left_alone() {
        assert_eq!(choose(None, Some(":1"), &[0], true), Choice::Keep);
    }

    #[test]
    fn without_display_uses_the_x_server_that_is_listening() {
        assert_eq!(choose(None, None, &[3, 0], true), Choice::X11(0));
        assert_eq!(choose(None, Some(""), &[1], false), Choice::X11(1));
    }

    #[test]
    fn without_any_x_server_goes_through_wayland() {
        assert_eq!(choose(None, None, &[], true), Choice::Wayland);
        assert_eq!(choose(None, None, &[], false), Choice::Keep);
    }

    #[test]
    fn the_backend_can_be_forced() {
        assert_eq!(choose(Some("wayland"), Some(":0"), &[0], true), Choice::Wayland);
        assert_eq!(choose(Some("X11"), None, &[0], true), Choice::X11(0));
        assert_eq!(choose(Some("x11"), Some(":0"), &[], true), Choice::Keep);
        assert_eq!(choose(Some("other"), None, &[], true), Choice::Wayland);
    }
}
