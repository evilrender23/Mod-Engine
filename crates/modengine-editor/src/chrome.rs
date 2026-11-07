//! The editor's own window frame; the system one is off (`with_decorations(false)` in main.rs).
//! A title bar that moves the window (double click maximizes), minimize / maximize / close
//! buttons, an accent outline and invisible borders to resize it. Moving and resizing are
//! handed to the OS (`StartDrag`, `BeginResize`), so snapping to screen edges keeps working.

use eframe::egui::{self, Align2, Color32, CursorIcon, FontFamily, FontId, Rect, ResizeDirection, Sense, Stroke, Vec2, ViewportCommand};

pub const TITLE_HEIGHT: f32 = 32.0;
/// Width of the invisible resize borders (corners are twice as long along each side).
const EDGE: f32 = 5.0;
const BUTTON_WIDTH: f32 = 46.0;

fn maximized(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false))
}

/// The title bar, as the top panel of `ui`: draw it before every other panel. `subtitle` (the
/// open project) goes after the app name.
pub fn title_bar(ui: &mut egui::Ui, subtitle: Option<&str>) {
    let ctx = ui.ctx().clone();
    let max = maximized(&ctx);
    let frame = crate::theme::sidebar_frame(&ctx).inner_margin(egui::Margin { left: 10, ..Default::default() });
    egui::Panel::top("window_title").frame(frame).exact_size(TITLE_HEIGHT).resizable(false).show(ui, |ui| {
        let bar = ui.max_rect();
        // registered first: the buttons drawn on top of it take their own clicks
        let drag = ui.interact(bar, egui::Id::unique("window_drag"), Sense::click_and_drag());
        if drag.double_clicked() {
            ctx.send_viewport_cmd(ViewportCommand::Maximized(!max));
        } else if drag.drag_started_by(egui::PointerButton::Primary) {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
        ui.horizontal_centered(|ui| {
            ui.label(egui::RichText::new(fa!(cube)).color(crate::theme::accent(ui)));
            ui.label(egui::RichText::new("Mod Engine Editor").strong());
            if let Some(s) = subtitle {
                ui.label(egui::RichText::new(format!("·  {s}")).weak());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                if window_button(ui, fa!(xmark), "Cerrar", true).clicked() {
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
                let (icon, tip) = if max { (fa!(window_restore), "Restaurar") } else { (fa!(window_maximize), "Maximizar") };
                if window_button(ui, icon, tip, false).clicked() {
                    ctx.send_viewport_cmd(ViewportCommand::Maximized(!max));
                }
                if window_button(ui, fa!(window_minimize), "Minimizar", false).clicked() {
                    ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
                }
            });
        });
    });
}

/// A caption button the height of the bar: outlined icon, red behind the close one on hover.
fn window_button(ui: &mut egui::Ui, icon: &str, tip: &str, close: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(BUTTON_WIDTH, TITLE_HEIGHT), Sense::click());
    let mut color = ui.visuals().text_color();
    if resp.hovered() {
        let fill = if close { Color32::from_rgb(232, 17, 35) } else { ui.visuals().widgets.hovered.weak_bg_fill };
        ui.painter().rect_filled(rect, 0.0, fill);
        if close {
            color = Color32::WHITE;
        }
    }
    // the regular (outlined) family: thin window glyphs; xmark falls back to the solid one
    let font = FontId::new(12.0, FontFamily::Name(crate::icons::REGULAR.into()));
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, icon, font, color);
    resp.on_hover_text(tip)
}

/// The outline and the resize borders, over everything else: call it last in the frame.
pub fn frame_edges(ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    if maximized(&ctx) {
        return;
    }
    let r = ctx.content_rect();
    let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
    let color = if focused { crate::theme::accent(ui) } else { crate::theme::colors(&ctx).frame_border };
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("window_outline")));
    painter.rect_stroke(r, 0.0, Stroke::new(1.0, color), egui::StrokeKind::Inside);

    let c = EDGE * 2.0;
    let zones = [
        (Rect::from_min_max(r.left_top(), r.right_top() + Vec2::new(0.0, EDGE)), ResizeDirection::North, CursorIcon::ResizeNorth),
        (Rect::from_min_max(r.left_bottom() - Vec2::new(0.0, EDGE), r.right_bottom()), ResizeDirection::South, CursorIcon::ResizeSouth),
        (Rect::from_min_max(r.left_top(), r.left_bottom() + Vec2::new(EDGE, 0.0)), ResizeDirection::West, CursorIcon::ResizeWest),
        (Rect::from_min_max(r.right_top() - Vec2::new(EDGE, 0.0), r.right_bottom()), ResizeDirection::East, CursorIcon::ResizeEast),
        (Rect::from_min_max(r.left_top(), r.left_top() + Vec2::splat(c)), ResizeDirection::NorthWest, CursorIcon::ResizeNorthWest),
        (Rect::from_min_max(r.right_top() + Vec2::new(-c, 0.0), r.right_top() + Vec2::new(0.0, c)), ResizeDirection::NorthEast, CursorIcon::ResizeNorthEast),
        (Rect::from_min_max(r.left_bottom() + Vec2::new(0.0, -c), r.left_bottom() + Vec2::new(c, 0.0)), ResizeDirection::SouthWest, CursorIcon::ResizeSouthWest),
        (Rect::from_min_max(r.right_bottom() - Vec2::splat(c), r.right_bottom()), ResizeDirection::SouthEast, CursorIcon::ResizeSouthEast),
    ];
    let pressed = ctx.input(|i| i.pointer.primary_pressed());
    // registered after everything else (and corners after sides), so they win the hit test
    for (i, (rect, dir, cursor)) in zones.into_iter().enumerate() {
        let resp = ui.interact(rect, egui::Id::unique("window_edge").with(i), Sense::drag());
        if resp.hovered() {
            ctx.set_cursor_icon(cursor);
            if pressed {
                ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
            }
        }
    }
}
