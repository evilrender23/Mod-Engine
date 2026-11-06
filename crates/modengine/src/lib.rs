//! # Mod Engine core
//!
//! A 2D game engine built around *objects* with *events*, placed as
//! *instances* inside *rooms*. Game logic is written in ModScript.
//!
//! This crate is renderer-agnostic: [`Engine::tick`] advances the game one
//! step and [`Engine::render`] fills a list of [`draw::DrawCmd`]s that a
//! backend (see `modengine-cli`) draws on screen.

pub mod anim_assets;
pub mod audio;
pub mod coroutine;
pub mod pool;
pub mod buffer;
pub mod animcurve;
pub mod actionq;
pub mod datatable;
pub mod credits;
pub mod draw;
pub mod dsgrid;
pub mod dungeon;
pub mod effects;
pub mod extension;
pub mod feel;
pub mod loc;
pub mod uifocus;
pub mod particles;
pub mod shader;
pub mod surface;
pub mod engine;
pub mod font;
pub mod fpview;
pub mod import;
pub mod input;
pub mod lint;
pub mod methods;
pub mod instance;
pub mod natives;
pub mod net;
pub mod object;
pub mod pack;
pub mod pixelart;
pub mod pixelcheck;
pub mod path;
pub mod palette;
pub mod project;
pub mod rotsprite;
pub mod scaffold;
pub mod sequence;
pub mod synth;
pub mod timeline;
pub mod tileset;
pub mod navgrid;
pub mod ui_state;
pub mod testing;
pub mod vfs;

pub use engine::{AudioCmd, Engine};
pub use modscript;

/// The user's home folder: `HOME`, or `USERPROFILE` on Windows (where `HOME` is usually
/// unset). Settings live under `home_dir()/.config/modengine` on every desktop.
pub fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).filter(|h| !h.is_empty()).map(std::path::PathBuf::from)
}

/// `p.canonicalize()` without Windows' `\\?\` verbatim prefix, which other programs never
/// report, users should not see and batch files can't take as their folder.
pub fn canonical(p: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    p.canonicalize().map(|c| plain_path(&c))
}

/// `p` without a Windows `\\?\` verbatim prefix: `\\?\C:\x` → `C:\x`, `\\?\UNC\srv\share` →
/// `\\srv\share`. Anything else (and every path elsewhere) is returned as is.
pub fn plain_path(p: &std::path::Path) -> std::path::PathBuf {
    if let Some(rest) = p.to_str().and_then(|s| s.strip_prefix(r"\\?\")).filter(|_| cfg!(windows)) {
        if let [d, b':', ..] = rest.as_bytes() {
            if d.is_ascii_alphabetic() {
                return std::path::PathBuf::from(rest);
            }
        }
        if let Some(unc) = rest.strip_prefix(r"UNC\") {
            return std::path::PathBuf::from(format!(r"\\{unc}"));
        }
    }
    p.to_path_buf()
}
