//! Font Awesome Free 6.7.2 icons (assets/fonts, SIL OFL 1.1; icons CC BY 4.0, see
//! assets/fonts/LICENSE-FontAwesome.txt). `theme::install_fonts` puts the solid font in every
//! text family, so an icon is just its private-use character inside any string:
//!
//! ```ignore
//! ui.button(concat!(fa!(folder_open), " Elegir…"));   // still a &'static str
//! ```
//!
//! Text that leaves the editor (prompts for the agent CLIs, exported Markdown, files, stdout)
//! must not carry these characters: they are blank outside the editor.
//!
//! [`regular`] draws the outlined style (☆ vs ★, ☐ vs ☑) of the icons that have one.

use eframe::egui::{FontFamily, RichText};

/// The font family of the outlined (regular) style; letters fall back to the UI font.
pub const REGULAR: &str = "fa-regular";

/// `text` drawn with the outlined icon style (for icons marked "regular" below).
pub fn regular(text: impl Into<String>) -> RichText {
    RichText::new(text).family(FontFamily::Name(REGULAR.into()))
}

/// `fa!(name)`: the icon as a string literal (usable in `concat!` and consts). Available in
/// every module through `#[macro_use] mod icons` in main.rs.
macro_rules! fa {
    (folder_open) => { "\u{f07c}" };  // regular too
    (folder) => { "\u{f07b}" };  // regular too
    (pen) => { "\u{f304}" };
    (pencil) => { "\u{f303}" };
    (play) => { "\u{f04b}" };
    (stop) => { "\u{f04d}" };
    (pause) => { "\u{f04c}" };
    (backward_step) => { "\u{f048}" };
    (forward_step) => { "\u{f051}" };
    (caret_left) => { "\u{f0d9}" };
    (caret_right) => { "\u{f0da}" };
    (caret_up) => { "\u{f0d8}" };
    (caret_down) => { "\u{f0d7}" };
    (trash_can) => { "\u{f2ed}" };  // regular too
    (cube) => { "\u{f1b2}" };
    (image) => { "\u{f03e}" };  // regular too
    (map) => { "\u{f279}" };  // regular too
    (table_cells) => { "\u{f00a}" };
    (table) => { "\u{f0ce}" };
    (font) => { "\u{f031}" };
    (note_sticky) => { "\u{f249}" };  // regular too
    // U+F001/U+F002 are fi/fl ligatures in egui's Ubuntu font, which comes first: Font Awesome's
    // emoji-codepoint alias instead (in no text font).
    (music) => { "\u{1f3b5}" };
    (wand_magic_sparkles) => { "\u{e2ca}" };
    (circle_half_stroke) => { "\u{f042}" };
    (puzzle_piece) => { "\u{f12e}" };
    (film) => { "\u{f008}" };
    (route) => { "\u{f4d7}" };
    (chart_line) => { "\u{f201}" };
    (stopwatch) => { "\u{f2f2}" };
    (globe) => { "\u{f0ac}" };
    (check) => { "\u{f00c}" };
    (xmark) => { "\u{f00d}" };
    (gear) => { "\u{f013}" };
    (triangle_exclamation) => { "\u{f071}" };
    (clone) => { "\u{f24d}" };  // regular too
    (file) => { "\u{f15b}" };  // regular too
    (magnifying_glass) => { "\u{1f50d}" };
    (download) => { "\u{f019}" };
    (upload) => { "\u{f093}" };
    (arrow_up) => { "\u{f062}" };
    (arrow_down) => { "\u{f063}" };
    (arrow_left) => { "\u{f060}" };
    (arrow_right) => { "\u{f061}" };
    (gamepad) => { "\u{f11b}" };
    (plus) => { "\u{f067}" };
    (minus) => { "\u{f068}" };
    (eye) => { "\u{f06e}" };  // regular too
    (eye_slash) => { "\u{f070}" };  // regular too
    (terminal) => { "\u{f120}" };
    (bars) => { "\u{f0c9}" };
    (vector_square) => { "\u{f5cb}" };
    (star) => { "\u{f005}" };  // regular too
    (paperclip) => { "\u{f0c6}" };
    (rotate_left) => { "\u{f2ea}" };
    (rotate_right) => { "\u{f2f9}" };
    (arrows_rotate) => { "\u{f021}" };
    (cart_shopping) => { "\u{f07a}" };
    (clipboard) => { "\u{f328}" };  // regular too
    (pen_to_square) => { "\u{f044}" };  // regular too
    (palette) => { "\u{f53f}" };
    (circle) => { "\u{f111}" };  // regular too
    (expand) => { "\u{f065}" };
    (compress) => { "\u{f066}" };
    (window_restore) => { "\u{f2d2}" };  // regular too
    (window_maximize) => { "\u{f2d0}" };  // regular too
    (window_minimize) => { "\u{f2d1}" };  // regular too
    (hard_drive) => { "\u{f0a0}" };  // regular too
    (floppy_disk) => { "\u{f0c7}" };  // regular too
    (clock) => { "\u{f017}" };  // regular too
    (camera) => { "\u{f030}" };
    (scissors) => { "\u{f0c4}" };
    (box) => { "\u{f466}" };
    (circle_info) => { "\u{f05a}" };
    (thumbtack) => { "\u{f08d}" };
    (link) => { "\u{f0c1}" };
    (desktop) => { "\u{f390}" };
    (ban) => { "\u{f05e}" };
    (square_check) => { "\u{f14a}" };  // regular too
    (square) => { "\u{f0c8}" };  // regular too
    (hourglass_half) => { "\u{f252}" };  // regular too
    (burst) => { "\u{e4dc}" };
    (fire) => { "\u{f06d}" };
    (wind) => { "\u{f72e}" };
    (snowflake) => { "\u{f2dc}" };  // regular too
    (cloud_rain) => { "\u{f73d}" };
    (wand_sparkles) => { "\u{f72b}" };
    (crosshairs) => { "\u{f05b}" };
    (circle_dot) => { "\u{f192}" };  // regular too
    (magnet) => { "\u{f076}" };
    (eraser) => { "\u{f12d}" };
    (fill_drip) => { "\u{f576}" };
    (eye_dropper) => { "\u{f1fb}" };
    (bolt) => { "\u{f0e7}" };
    (flag) => { "\u{f024}" };  // regular too
    (file_zipper) => { "\u{f1c6}" };  // regular too
    (house) => { "\u{f015}" };
    (arrows_left_right) => { "\u{f07e}" };
    (arrows_up_down) => { "\u{f07d}" };
    (table_columns) => { "\u{f0db}" };
    (ellipsis) => { "\u{f141}" };
    (ellipsis_vertical) => { "\u{f142}" };
    (right_left) => { "\u{f362}" };
    (reply) => { "\u{f3e5}" };
    (code) => { "\u{f121}" };
    (list) => { "\u{f03a}" };
    (sliders) => { "\u{f1de}" };
    (layer_group) => { "\u{f5fd}" };
    (shapes) => { "\u{f61f}" };
    (keyboard) => { "\u{f11c}" };  // regular too
    (comment) => { "\u{f075}" };  // regular too
    (robot) => { "\u{f544}" };
    (spinner) => { "\u{f110}" };
    (circle_check) => { "\u{f058}" };  // regular too
    (circle_xmark) => { "\u{f057}" };  // regular too
    (lightbulb) => { "\u{f0eb}" };  // regular too
    (sun) => { "\u{f185}" };  // regular too
    (moon) => { "\u{f186}" };  // regular too
    (volume_high) => { "\u{f028}" };
    (images) => { "\u{f302}" };  // regular too
    (photo_film) => { "\u{f87c}" };
    (clapperboard) => { "\u{e131}" };
    (heart) => { "\u{f004}" };  // regular too
    (filter) => { "\u{f0b0}" };
    (sort) => { "\u{f0dc}" };
    (grip_vertical) => { "\u{f58e}" };
    (arrow_down_long) => { "\u{f175}" };
    (arrow_up_long) => { "\u{f176}" };
    (angles_down) => { "\u{f103}" };
    (angles_up) => { "\u{f102}" };
    (angle_down) => { "\u{f107}" };
    (angle_up) => { "\u{f106}" };
    (angle_left) => { "\u{f104}" };
    (angle_right) => { "\u{f105}" };
    (square_plus) => { "\u{f0fe}" };  // regular too
    (square_minus) => { "\u{f146}" };  // regular too
    (up_right_from_square) => { "\u{f35d}" };
    (copy) => { "\u{f0c5}" };  // regular too
    (paste) => { "\u{f0ea}" };  // regular too
    (box_open) => { "\u{f49e}" };
    (toolbox) => { "\u{f552}" };
    (hammer) => { "\u{f6e3}" };
    (sitemap) => { "\u{f0e8}" };
    (diagram_project) => { "\u{f542}" };
    (bezier_curve) => { "\u{f55b}" };
    (object_group) => { "\u{f247}" };  // regular too
    (cubes) => { "\u{f1b3}" };
    (user) => { "\u{f007}" };  // regular too
    (users) => { "\u{f0c0}" };
    (lock) => { "\u{f023}" };
    (unlock) => { "\u{f09c}" };
    (language) => { "\u{f1ab}" };
    (dice) => { "\u{f522}" };
    (shuffle) => { "\u{f074}" };
    (repeat) => { "\u{f363}" };
    (list_check) => { "\u{f0ae}" };
    (rectangle_list) => { "\u{f022}" };  // regular too
    (border_all) => { "\u{f84c}" };
    (draw_polygon) => { "\u{f5ee}" };
    (location_dot) => { "\u{f3c5}" };
    (location_pin) => { "\u{f041}" };
    (tag) => { "\u{f02b}" };
    (tags) => { "\u{f02c}" };
    (bug) => { "\u{f188}" };
    (question) => { "\u{f128}" };
    (circle_question) => { "\u{f059}" };  // regular too
}
