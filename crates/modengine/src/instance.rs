//! Runtime instances of objects and their built-in variables.

use modscript::{ArrayRef, Sym, Value};
use std::collections::HashMap;

pub const ALARM_COUNT: usize = 12;

#[derive(Clone)]
pub struct Instance {
    pub id: u32,
    pub object: u32,
    pub alive: bool,
    pub x: f64,
    pub y: f64,
    pub xprev: f64,
    pub yprev: f64,
    pub xstart: f64,
    pub ystart: f64,
    pub hspeed: f64,
    pub vspeed: f64,
    /// Direction of motion in degrees (0 = right, 90 = up). Kept even when speed is 0.
    pub dir: f64,
    pub gravity: f64,
    pub gravity_dir: f64,
    pub friction: f64,
    pub sprite: Option<u32>,
    pub mask: Option<u32>,
    pub frame: f64,
    pub anim_speed: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    pub angle: f64,
    pub alpha: f64,
    pub tint: u32,
    pub depth: f64,
    pub visible: bool,
    pub solid: bool,
    pub persistent: bool,
    pub alarms: ArrayRef,
    pub vars: HashMap<Sym, Value>,
    pub outside: bool,
    pub at_boundary: bool,
    pub mouse_over: bool,
    /// path_start state (path_index, path_position...).
    pub path: crate::path::PathFollow,
    /// timeline_index, timeline_position...
    pub timeline: crate::timeline::TimelineRun,
    /// Collision layers this instance is on (bit mask, `collision_layer`).
    pub col_layer: u32,
    /// Layers it collides with (`collision_mask`).
    pub col_mask: u32,
}

impl Instance {
    pub fn new(id: u32, object: u32, x: f64, y: f64, depth: f64) -> Self {
        let alarms = match Value::array(vec![Value::Num(-1.0); ALARM_COUNT]) {
            Value::Array(a) => a,
            _ => unreachable!(),
        };
        Instance {
            id,
            object,
            alive: true,
            x,
            y,
            xprev: x,
            yprev: y,
            xstart: x,
            ystart: y,
            hspeed: 0.0,
            vspeed: 0.0,
            dir: 0.0,
            gravity: 0.0,
            gravity_dir: 270.0,
            friction: 0.0,
            sprite: None,
            mask: None,
            frame: 0.0,
            anim_speed: 1.0,
            scale_x: 1.0,
            scale_y: 1.0,
            angle: 0.0,
            alpha: 1.0,
            tint: 0xFFFFFF,
            depth,
            visible: true,
            solid: false,
            persistent: false,
            alarms,
            vars: HashMap::new(),
            outside: false,
            at_boundary: false,
            mouse_over: false,
            path: Default::default(),
            timeline: Default::default(),
            col_layer: 1,
            col_mask: u32::MAX,
        }
    }

    pub fn speed(&self) -> f64 {
        self.hspeed.hypot(self.vspeed)
    }

    pub fn set_hv(&mut self, h: f64, v: f64) {
        self.hspeed = snap(h);
        self.vspeed = snap(v);
        if self.hspeed != 0.0 || self.vspeed != 0.0 {
            self.dir = (-self.vspeed).atan2(self.hspeed).to_degrees().rem_euclid(360.0);
        }
    }

    pub fn set_speed(&mut self, s: f64) {
        let r = self.dir.to_radians();
        self.hspeed = snap(s * r.cos());
        self.vspeed = snap(-s * r.sin());
    }

    pub fn set_dir(&mut self, d: f64) {
        let s = self.speed();
        self.dir = d.rem_euclid(360.0);
        self.set_speed(s);
    }
}

fn snap(v: f64) -> f64 {
    if v.abs() < 1e-9 {
        0.0
    } else {
        v
    }
}

/// Built-in instance variables.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Bv {
    X,
    Y,
    Xprev,
    Yprev,
    Xstart,
    Ystart,
    Hspeed,
    Vspeed,
    Speed,
    Direction,
    Gravity,
    GravityDir,
    Friction,
    Sprite,
    Mask,
    Frame,
    FrameCount,
    AnimSpeed,
    ScaleX,
    ScaleY,
    Angle,
    Alpha,
    Tint,
    Depth,
    Visible,
    Solid,
    Persistent,
    Alarm,
    Id,
    Object,
    BboxLeft,
    BboxRight,
    BboxTop,
    BboxBottom,
    SpriteWidth,
    SpriteHeight,
    Path(crate::path::PathVar),
    Timeline(crate::timeline::TlVar),
    /// Engine-wide or collision-layer variables (see `ui_state::ExtVar`).
    Ext(crate::ui_state::ExtVar),
}

pub const BUILTIN_VARS: &[(&str, Bv)] = &[
    ("x", Bv::X),
    ("y", Bv::Y),
    ("xprev", Bv::Xprev),
    ("yprev", Bv::Yprev),
    ("xstart", Bv::Xstart),
    ("ystart", Bv::Ystart),
    ("hspeed", Bv::Hspeed),
    ("vspeed", Bv::Vspeed),
    ("speed", Bv::Speed),
    ("direction", Bv::Direction),
    ("gravity", Bv::Gravity),
    ("gravity_dir", Bv::GravityDir),
    ("friction", Bv::Friction),
    ("sprite", Bv::Sprite),
    ("mask", Bv::Mask),
    ("frame", Bv::Frame),
    ("frame_count", Bv::FrameCount),
    ("anim_speed", Bv::AnimSpeed),
    ("scale_x", Bv::ScaleX),
    ("scale_y", Bv::ScaleY),
    ("angle", Bv::Angle),
    ("alpha", Bv::Alpha),
    ("tint", Bv::Tint),
    ("depth", Bv::Depth),
    ("visible", Bv::Visible),
    ("solid", Bv::Solid),
    ("persistent", Bv::Persistent),
    ("alarm", Bv::Alarm),
    ("id", Bv::Id),
    ("object", Bv::Object),
    ("bbox_left", Bv::BboxLeft),
    ("bbox_right", Bv::BboxRight),
    ("bbox_top", Bv::BboxTop),
    ("bbox_bottom", Bv::BboxBottom),
    ("sprite_width", Bv::SpriteWidth),
    ("sprite_height", Bv::SpriteHeight),
    ("path_index", Bv::Path(crate::path::PathVar::Index)),
    ("path_position", Bv::Path(crate::path::PathVar::Position)),
    ("path_positionprevious", Bv::Path(crate::path::PathVar::PositionPrevious)),
    ("path_speed", Bv::Path(crate::path::PathVar::Speed)),
    ("path_scale", Bv::Path(crate::path::PathVar::Scale)),
    ("path_orientation", Bv::Path(crate::path::PathVar::Orientation)),
    ("path_endaction", Bv::Path(crate::path::PathVar::EndAction)),
    ("timeline_index", Bv::Timeline(crate::timeline::TlVar::Index)),
    ("timeline_position", Bv::Timeline(crate::timeline::TlVar::Position)),
    ("timeline_speed", Bv::Timeline(crate::timeline::TlVar::Speed)),
    ("timeline_running", Bv::Timeline(crate::timeline::TlVar::Running)),
    ("timeline_loop", Bv::Timeline(crate::timeline::TlVar::Loop)),
    ("collision_layer", Bv::Ext(crate::ui_state::ExtVar::CollisionLayer)),
    ("collision_mask", Bv::Ext(crate::ui_state::ExtVar::CollisionMask)),
    ("keyboard_string", Bv::Ext(crate::ui_state::ExtVar::KeyboardString)),
    ("keyboard_lastchar", Bv::Ext(crate::ui_state::ExtVar::KeyboardLastchar)),
    ("cursor_sprite", Bv::Ext(crate::ui_state::ExtVar::CursorSprite)),
];
