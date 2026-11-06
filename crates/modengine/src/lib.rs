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
pub mod credits;
pub mod draw;
pub mod effects;
pub mod extension;
pub mod particles;
pub mod shader;
pub mod surface;
pub mod engine;
pub mod font;
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
