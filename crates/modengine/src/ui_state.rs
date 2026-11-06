//! Small pieces of engine-wide state used by the UI helpers: the mouse cursor
//! (`window_set_cursor`, `cursor_sprite`), the last typed character, the scissor
//! stack (`gpu_set_scissor`) and the backspace auto-repeat of `keyboard_string`.

/// GameMaker's cursor constants (`cr_*`) and their values.
pub const CURSORS: &[(&str, i32)] = &[
    ("cr_default", 0),
    ("cr_none", -1),
    ("cr_arrow", -2),
    ("cr_cross", -3),
    ("cr_beam", -4),
    ("cr_size_nesw", -6),
    ("cr_size_ns", -7),
    ("cr_size_nwse", -8),
    ("cr_size_we", -9),
    ("cr_uparrow", -10),
    ("cr_hourglass", -11),
    ("cr_drag", -12),
    ("cr_appstart", -19),
    ("cr_handpoint", -21),
    ("cr_size_all", -22),
];

pub const CR_DEFAULT: i32 = 0;
pub const CR_NONE: i32 = -1;

#[derive(Clone, Debug)]
pub struct UiState {
    /// `window_set_cursor` value (`cr_*`).
    pub cursor: i32,
    /// Sprite drawn at the mouse on top of the GUI (`cursor_sprite`); hides the system cursor.
    pub cursor_sprite: Option<u32>,
    /// Frame of `cursor_sprite` (animated with the step counter).
    pub cursor_frame: f64,
    /// Last character typed (`keyboard_lastchar`).
    pub lastchar: String,
    /// Steps backspace has been held (for auto-repeat).
    pub backspace_held: u32,
    /// Scissor rectangles pushed during this frame (current coordinates: room or GUI).
    pub clip: Vec<[f32; 4]>,
}

impl Default for UiState {
    fn default() -> Self {
        UiState { cursor: CR_DEFAULT, cursor_sprite: None, cursor_frame: 0.0, lastchar: String::new(), backspace_held: 0, clip: vec![] }
    }
}

impl UiState {
    /// Should the system cursor be visible?
    pub fn system_cursor_visible(&self) -> bool {
        self.cursor != CR_NONE && self.cursor_sprite.is_none()
    }

    /// Steps after which a held backspace starts repeating, and the repeat interval.
    pub const REPEAT_DELAY: u32 = 30;
    pub const REPEAT_EVERY: u32 = 3;

    /// Advances the backspace auto-repeat; true when this step should delete a character
    /// (besides the first press, which the caller handles).
    pub fn backspace_repeat(&mut self, down: bool) -> bool {
        if !down {
            self.backspace_held = 0;
            return false;
        }
        self.backspace_held += 1;
        let h = self.backspace_held;
        h > Self::REPEAT_DELAY && (h - Self::REPEAT_DELAY) % Self::REPEAT_EVERY == 0
    }
}

/// Intersection of two rectangles `[x, y, w, h]` (empty = zero size).
pub fn intersect(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let x1 = a[0].max(b[0]);
    let y1 = a[1].max(b[1]);
    let x2 = (a[0] + a[2]).min(b[0] + b[2]);
    let y2 = (a[1] + a[3]).min(b[1] + b[3]);
    [x1, y1, (x2 - x1).max(0.0), (y2 - y1).max(0.0)]
}

/// Variables that read/write engine state (or the collision layers) instead of a plain
/// instance field. They also work from global code (room code, scripts).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ExtVar {
    CollisionLayer,
    CollisionMask,
    KeyboardString,
    KeyboardLastchar,
    CursorSprite,
}

impl ExtVar {
    /// The engine-wide ones (usable without an instance).
    pub fn is_global(self) -> bool {
        !matches!(self, ExtVar::CollisionLayer | ExtVar::CollisionMask)
    }
}
