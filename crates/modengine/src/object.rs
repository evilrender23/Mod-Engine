//! Objects (the "classes" of the game) and their events.

use crate::input::{key_index, KEY_NAMES};
use modscript::{AssetKind, FuncDef, Stmt, Sym, Value};
use std::collections::HashMap;
use std::rc::Rc;

pub const ANY_KEY: u16 = u16::MAX;

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum Ev {
    Create,
    Destroy,
    StepBegin,
    Step,
    StepEnd,
    Alarm(u8),
    DrawBegin,
    Draw,
    DrawEnd,
    DrawGui,
    /// Before draw_begin, over the cleared frame (GUI coordinates).
    PreDraw,
    /// After draw_gui, over the finished frame (see `surface.rs`).
    PostDraw,
    Collision(u32),
    Key(u16),
    KeyPressed(u16),
    KeyReleased(u16),
    MouseDown(u8),
    MousePressed(u8),
    MouseReleased(u8),
    MouseEnter,
    MouseLeave,
    GlobalMouseDown(u8),
    GlobalMousePressed(u8),
    GlobalMouseReleased(u8),
    RoomStart,
    RoomEnd,
    GameStart,
    GameEnd,
    OutsideRoom,
    Boundary,
    AnimationEnd,
    PathEnded,
    User(u8),
    /// Async networking event (`async_load` holds what happened).
    Network,
}

impl Ev {
    pub fn is_input(self) -> bool {
        matches!(
            self,
            Ev::Key(_)
                | Ev::KeyPressed(_)
                | Ev::KeyReleased(_)
                | Ev::MouseDown(_)
                | Ev::MousePressed(_)
                | Ev::MouseReleased(_)
                | Ev::MouseEnter
                | Ev::MouseLeave
                | Ev::GlobalMouseDown(_)
                | Ev::GlobalMousePressed(_)
                | Ev::GlobalMouseReleased(_)
        )
    }
}

pub const EVENT_NAMES: &[&str] = &[
    "create",
    "destroy",
    "step_begin",
    "step",
    "step_end",
    "alarm(n)",
    "draw_begin",
    "draw",
    "draw_end",
    "draw_gui",
    "pre_draw",
    "post_draw",
    "collision(Object)",
    "key(k)",
    "key_pressed(k)",
    "key_released(k)",
    "mouse_down(b)",
    "mouse_pressed(b)",
    "mouse_released(b)",
    "mouse_enter",
    "mouse_leave",
    "global_mouse_down(b)",
    "global_mouse_pressed(b)",
    "global_mouse_released(b)",
    "room_start",
    "room_end",
    "game_start",
    "game_end",
    "outside_room",
    "boundary",
    "animation_end",
    "path_ended",
    "user(n)",
    "network",
];

fn small_int(arg: &Option<Value>, max: u32, what: &str) -> Result<u8, String> {
    match arg.as_ref().and_then(|v| v.as_num()) {
        Some(n) if n >= 0.0 && (n as u32) < max => Ok(n as u8),
        _ => Err(format!("{what} needs a number from 0 to {}", max - 1)),
    }
}

fn key_arg(arg: &Option<Value>) -> Result<u16, String> {
    match arg {
        None => Ok(ANY_KEY),
        Some(Value::Num(n)) if *n >= 0.0 && (*n as usize) < KEY_NAMES.len() => Ok(*n as u16),
        Some(Value::Str(s)) => key_index(&s.to_lowercase())
            .map(|k| k as u16)
            .ok_or_else(|| format!("unknown key \"{s}\"")),
        Some(other) => Err(format!("expected a key like key.space, got {other:?}")),
    }
}

fn mouse_arg(arg: &Option<Value>) -> Result<u8, String> {
    match arg {
        None => Ok(0),
        _ => small_int(arg, 3, "mouse events"),
    }
}

/// Turns `on <name>(<arg>)` into an [`Ev`].
pub fn parse_event(name: &str, arg: Option<Value>) -> Result<Ev, String> {
    Ok(match name {
        "create" => Ev::Create,
        "destroy" => Ev::Destroy,
        "step_begin" => Ev::StepBegin,
        "step" => Ev::Step,
        "step_end" => Ev::StepEnd,
        "alarm" => Ev::Alarm(small_int(&arg, crate::instance::ALARM_COUNT as u32, "alarm")?),
        "draw_begin" => Ev::DrawBegin,
        "draw" => Ev::Draw,
        "draw_end" => Ev::DrawEnd,
        "draw_gui" => Ev::DrawGui,
        "pre_draw" => Ev::PreDraw,
        "post_draw" => Ev::PostDraw,
        "collision" => match arg {
            Some(Value::Asset(AssetKind::Object, o)) => Ev::Collision(o),
            _ => return Err("collision needs an object, e.g. `on collision(Wall)`".into()),
        },
        "key" | "key_down" => Ev::Key(key_arg(&arg)?),
        "key_pressed" => Ev::KeyPressed(key_arg(&arg)?),
        "key_released" => Ev::KeyReleased(key_arg(&arg)?),
        "mouse_down" => Ev::MouseDown(mouse_arg(&arg)?),
        "mouse_pressed" => Ev::MousePressed(mouse_arg(&arg)?),
        "mouse_released" => Ev::MouseReleased(mouse_arg(&arg)?),
        "mouse_enter" => Ev::MouseEnter,
        "mouse_leave" => Ev::MouseLeave,
        "global_mouse_down" => Ev::GlobalMouseDown(mouse_arg(&arg)?),
        "global_mouse_pressed" => Ev::GlobalMousePressed(mouse_arg(&arg)?),
        "global_mouse_released" => Ev::GlobalMouseReleased(mouse_arg(&arg)?),
        "room_start" => Ev::RoomStart,
        "room_end" => Ev::RoomEnd,
        "game_start" => Ev::GameStart,
        "game_end" => Ev::GameEnd,
        "outside_room" => Ev::OutsideRoom,
        "boundary" => Ev::Boundary,
        "animation_end" => Ev::AnimationEnd,
        "path_ended" => Ev::PathEnded,
        "user" => Ev::User(small_int(&arg, 16, "user events")?),
        "network" | "async_network" => Ev::Network,
        other => {
            return Err(format!("unknown event '{other}'. Valid events: {}", EVENT_NAMES.join(", ")));
        }
    })
}

pub struct ObjectDef {
    pub name: String,
    pub sym: Sym,
    pub parent: Option<u32>,
    pub props: Rc<Vec<Stmt>>,
    /// Typed variables declared by this object itself.
    pub vars: Vec<modscript::VarDecl>,
    pub file: Rc<str>,
    pub own_events: HashMap<Ev, Rc<FuncDef>>,
    /// Effective events including inherited ones: event → (defining object, code).
    pub events: HashMap<Ev, (u32, Rc<FuncDef>)>,
    /// This object first, then its parent, grandparent...
    pub ancestors: Vec<u32>,
    pub collision_targets: Rc<Vec<u32>>,
    pub input_events: Rc<Vec<Ev>>,
    pub bounds_events: bool,
}

impl ObjectDef {
    pub fn has(&self, ev: Ev) -> bool {
        self.events.contains_key(&ev)
    }
}
