//! Input state for one logic step. Backends fill it before every `tick()`.

/// Names of the keys scripts can use (`key.left`, `key.a`, `key["1"]`...).
/// A key's value in scripts is its index in this table.
pub const KEY_NAMES: &[&str] = &[
    "left", "right", "up", "down", "space", "enter", "escape", "shift", "ctrl", "alt", "tab", "backspace", "delete",
    "insert", "home", "end", "page_up", "page_down", "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m",
    "n", "o", "p", "q", "r", "s", "t", "u", "v", "w", "x", "y", "z", "0", "1", "2", "3", "4", "5", "6", "7", "8", "9",
    "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9", "f10", "f11", "f12", "minus", "equal", "comma", "period",
    "slash",
];

pub fn key_index(name: &str) -> Option<usize> {
    KEY_NAMES.iter().position(|k| *k == name)
}

pub const MOUSE_BUTTONS: &[&str] = &["left", "right", "middle"];

#[derive(Clone, Debug)]
pub struct Input {
    pub down: Vec<bool>,
    pub pressed: Vec<bool>,
    pub released: Vec<bool>,
    pub mouse_down: [bool; 3],
    pub mouse_pressed: [bool; 3],
    pub mouse_released: [bool; 3],
    /// Mouse position in game-screen (GUI) coordinates.
    pub mouse_x: f64,
    pub mouse_y: f64,
    pub wheel: f64,
    /// Fingers on a touch screen, in game-screen (GUI) coordinates.
    pub touches: Vec<(f64, f64)>,
}

impl Default for Input {
    fn default() -> Self {
        let n = KEY_NAMES.len();
        Input {
            down: vec![false; n],
            pressed: vec![false; n],
            released: vec![false; n],
            mouse_down: [false; 3],
            mouse_pressed: [false; 3],
            mouse_released: [false; 3],
            mouse_x: 0.0,
            mouse_y: 0.0,
            wheel: 0.0,
            touches: vec![],
        }
    }
}

impl Input {
    pub fn any_down(&self) -> bool {
        self.down.iter().any(|b| *b)
    }
    pub fn any_pressed(&self) -> bool {
        self.pressed.iter().any(|b| *b)
    }
    pub fn any_released(&self) -> bool {
        self.released.iter().any(|b| *b)
    }
    /// Clears the one-step edges (pressed / released / wheel).
    pub fn clear_edges(&mut self) {
        self.pressed.iter_mut().for_each(|b| *b = false);
        self.released.iter_mut().for_each(|b| *b = false);
        self.mouse_pressed = [false; 3];
        self.mouse_released = [false; 3];
        self.wheel = 0.0;
    }
}
