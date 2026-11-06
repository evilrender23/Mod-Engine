//! Surfaces (GameMaker's `surface_*`) and the frame passes around the draw events
//! (`pre_draw`, `post_draw`, `application_surface`).
//!
//! The engine only keeps ids and sizes and records [`DrawCmd::Target`] / [`DrawCmd::Surface`]
//! commands; the backend owns the GPU render targets (see modengine-cli's `canvas.rs`), so
//! `modengine check` runs everything headless.
//!
//! Drawing into a surface outside the draw events (`create`, `step`...) works like in
//! GameMaker: those blocks are kept and replayed at the start of the next frame.

use crate::draw::{DrawCmd, DrawState};
use crate::engine::Engine;
use crate::object::Ev;
use std::collections::BTreeMap;

/// Id of `application_surface`: the game frame (display width × height).
pub const APP: u32 = 0;
/// Largest side of a surface.
pub const MAX_SIZE: u32 = 8192;

#[derive(Clone, Debug)]
pub struct Surfaces {
    /// Live surfaces: id → (width, height). Ids are never reused.
    pub live: BTreeMap<u32, (u32, u32)>,
    next: u32,
    /// `surface_set_target` stack (innermost last).
    pub stack: Vec<u32>,
    /// `application_surface_draw_enable`: false = the frame shown is only what `post_draw` draws.
    pub app_draw: bool,
    /// `cmds.len()` when the last frame finished: what comes after was recorded outside draw events.
    frame_end: usize,
}

impl Default for Surfaces {
    fn default() -> Self {
        Surfaces { live: BTreeMap::new(), next: 1, stack: vec![], app_draw: true, frame_end: 0 }
    }
}

impl Surfaces {
    pub fn create(&mut self, w: u32, h: u32) -> u32 {
        let id = self.next;
        self.next += 1;
        self.live.insert(id, (w, h));
        id
    }

    /// Size of a live surface; `app` is the size of application_surface.
    pub fn size(&self, id: i64, app: (u32, u32)) -> Option<(u32, u32)> {
        if id == APP as i64 {
            return Some(app);
        }
        u32::try_from(id).ok().and_then(|i| self.live.get(&i).copied())
    }
}

fn has_event(e: &Engine, ids: &[u32], ev: Ev) -> bool {
    ids.iter().any(|&id| e.idx(id).is_some_and(|i| e.insts[i].alive && e.insts[i].visible && e.objects[e.insts[i].object as usize].has(ev)))
}

/// Closes targets left open (GameMaker errors out; we report it and go back to the frame).
fn close_targets(e: &mut Engine, when: &str) {
    if e.surfaces.stack.is_empty() {
        return;
    }
    e.surfaces.stack.clear();
    e.cmds.push(DrawCmd::Target(None));
    e.report(modscript::RtError::new(format!("surface_set_target() sin surface_reset_target() {when}")));
}

/// Start of `Engine::render`: drops the previous frame but keeps surface drawing done
/// outside the draw events since then.
pub(crate) fn begin_render(e: &mut Engine) {
    close_targets(e, "al terminar el paso");
    let from = e.surfaces.frame_end.min(e.cmds.len());
    let outside: Vec<DrawCmd> = e.cmds.drain(from..).collect();
    e.cmds.clear();
    let mut inside = false;
    for c in outside {
        match c {
            DrawCmd::Target(t) => {
                inside = t.is_some();
                e.cmds.push(DrawCmd::Target(t));
            }
            c if inside => e.cmds.push(c),
            _ => {}
        }
    }
    if !e.cmds.is_empty() {
        // a shader or blend mode set inside those blocks must not leak into the frame
        e.cmds.push(DrawCmd::Shader(None));
        e.cmds.push(DrawCmd::Blend(0));
    }
}

/// `pre_draw` events: before `draw_begin`, over the cleared frame, in GUI coordinates.
pub(crate) fn pre_draw(e: &mut Engine, ids: &[u32], view: &DrawCmd) {
    if !has_event(e, ids, Ev::PreDraw) {
        return;
    }
    e.cmds.push(DrawCmd::Gui);
    e.dispatch_visible(ids, Ev::PreDraw);
    close_targets(e, "en pre_draw");
    e.cmds.push(view.clone());
    e.draw = DrawState::default();
}

/// After `draw_gui`: the `post_draw` pass over the finished frame (application_surface).
pub(crate) fn end_render(e: &mut Engine, ids: &[u32]) {
    close_targets(e, "al terminar el dibujo");
    if !e.surfaces.app_draw || has_event(e, ids, Ev::PostDraw) {
        e.cmds.push(DrawCmd::PostDraw { app: e.surfaces.app_draw });
        e.draw = DrawState::default();
        e.dispatch_visible(ids, Ev::PostDraw);
        close_targets(e, "en post_draw");
    }
    e.surfaces.frame_end = e.cmds.len();
}
