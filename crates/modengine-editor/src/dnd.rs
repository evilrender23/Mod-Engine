//! Drag and drop helpers shared by the editors, on top of egui's `DragAndDrop` payload.
//!
//! Two kinds of payload travel around:
//! * [`crate::app::Open`]: an asset dragged from the asset browser (or the room palette). It can
//!   be dropped on the workspace canvas (opens it there), on a room (places an instance), on
//!   the object editor's sprite/parent fields, or between rows of its own browser section.
//! * [`ListDrag`]: a row of a reorderable list (layers, events, frames, workspace tabs), only
//!   accepted by the list it came from.

use crate::app::Open;
use eframe::egui::{self, Align2, Color32, CornerRadius, DragAndDrop, FontId, Id, LayerId, Order, Pos2, Rect, Response, Stroke, Ui, Vec2};
use std::any::Any;
use std::collections::BTreeMap;

/// A row being dragged inside a reorderable list.
#[derive(Clone, Debug, PartialEq)]
pub struct ListDrag {
    /// Identifies the list (and the editor it belongs to), so rows never jump between lists.
    pub list: Id,
    pub from: usize,
    /// Text of the ghost shown next to the cursor.
    pub label: String,
}

/// Moves `v[from]` so it ends up before what was `v[to]` (`to == len` = at the end).
/// Returns the new index of the moved item.
pub fn move_item<T>(v: &mut Vec<T>, from: usize, to: usize) -> usize {
    if from >= v.len() {
        return from;
    }
    let to = to.min(v.len());
    let item = v.remove(from);
    let at = if to > from { to - 1 } else { to };
    v.insert(at, item);
    at
}

/// Where a row dropped at `p` goes: the number of rows whose centre is before `p`.
pub fn insert_index(rects: &[Rect], p: Pos2, vertical: bool) -> usize {
    rects.iter().filter(|r| if vertical { r.center().y < p.y } else { r.center().x < p.x }).count()
}

/// Dropping at `to` leaves the row where it is.
pub fn is_noop(from: usize, to: usize) -> bool {
    to == from || to == from + 1
}

/// `names` sorted by a saved custom order; names missing from it keep their order at the end.
pub fn apply_order(names: Vec<String>, order: &[String]) -> Vec<String> {
    let mut known: Vec<(usize, String)> = vec![];
    let mut rest = vec![];
    for n in names {
        match order.iter().position(|o| *o == n) {
            Some(i) => known.push((i, n)),
            None => rest.push(n),
        }
    }
    known.sort_by_key(|(i, _)| *i);
    known.into_iter().map(|(_, n)| n).chain(rest).collect()
}

/// Custom order of the asset browser sections (`"sprites" -> [names]`), saved with the layout.
pub type AssetOrder = BTreeMap<String, Vec<String>>;

/// The pointer, in `ui`'s layer coordinates, when it is over `rect` (and not covered by another
/// layer). Works inside the zoomed workspace windows.
pub fn pointer_in(ui: &Ui, rect: Rect) -> Option<Pos2> {
    if !ui.rect_contains_pointer(rect) {
        return None;
    }
    let p = ui.ctx().input(|i| i.pointer.interact_pos().or(i.pointer.latest_pos()))?;
    Some(match ui.ctx().layer_transform_from_global(ui.layer_id()) {
        Some(t) => t * p,
        None => p,
    })
}

/// Makes a clickable row draggable: sets `payload` once the drag really starts (past egui's
/// click distance, so clicks and double clicks still work) and shows the grab cursor.
pub fn source<P: Any + Send + Sync>(resp: &Response, payload: impl FnOnce() -> P) {
    if resp.drag_started() {
        DragAndDrop::set_payload(&resp.ctx, payload());
    }
    if resp.hovered() && !DragAndDrop::has_any_payload(&resp.ctx) {
        resp.ctx.set_cursor_icon(egui::CursorIcon::Grab);
    }
}

/// Is a row of this list being dragged right now?
pub fn dragging_from(ctx: &egui::Context, list: Id) -> Option<usize> {
    DragAndDrop::payload::<ListDrag>(ctx).filter(|d| d.list == list).map(|d| d.from)
}

/// Drop handling of a reorderable list. `rects` are its rows in order. While one of its rows
/// is dragged over it, draws the insertion line; on release returns `(from, to)` for
/// [`move_item`] (only when the order actually changes).
pub fn reorder(ui: &Ui, list: Id, rects: &[Rect], vertical: bool) -> Option<(usize, usize)> {
    let from = dragging_from(ui.ctx(), list)?;
    reorder_in(ui, list_area(ui, rects, vertical), rects, vertical, from).inspect(|_| DragAndDrop::clear_payload(ui.ctx()))
}

/// Where a list accepts its rows: the rows plus a margin, and the whole width of `ui` for
/// vertical lists (rows are often only as wide as their label).
pub fn list_area(ui: &Ui, rects: &[Rect], vertical: bool) -> Rect {
    let area = rects.iter().fold(Rect::NOTHING, |a, r| a.union(*r));
    if vertical {
        let x = ui.max_rect().x_range();
        Rect::from_x_y_ranges(egui::Rangef::new(x.min.min(area.min.x), x.max.max(area.max.x)), area.y_range()).expand2(Vec2::new(0.0, 8.0))
    } else {
        area.expand2(Vec2::new(8.0, 0.0))
    }
}

/// Shared by [`reorder`] and the asset browser: line + release for a payload already accepted.
pub fn reorder_in(ui: &Ui, area: Rect, rects: &[Rect], vertical: bool, from: usize) -> Option<(usize, usize)> {
    if rects.is_empty() {
        return None;
    }
    let p = pointer_in(ui, area)?;
    let to = insert_index(rects, p, vertical);
    if !is_noop(from, to) {
        insert_line(ui, rects, to, vertical, if vertical { area.x_range() } else { area.y_range() });
    }
    let released = ui.input(|i| i.pointer.any_released());
    (released && !is_noop(from, to)).then_some((from, to))
}

/// Accent line between rows where a dragged row would land.
/// `span`: extent of the line across the list (x range for vertical lists).
pub fn insert_line(ui: &Ui, rects: &[Rect], to: usize, vertical: bool, span: egui::Rangef) {
    let color = ui.visuals().selection.stroke.color;
    let stroke = Stroke::new(2.5, color);
    // same layer as the rows (drawn after them), so it follows zoomed workspace windows
    let painter = ui.painter().clone().with_clip_rect(ui.clip_rect().expand(4.0));
    if vertical {
        let y = match (rects.get(to.wrapping_sub(1)), rects.get(to)) {
            (Some(a), Some(b)) => (a.max.y + b.min.y) / 2.0,
            (None, Some(b)) => b.min.y - 1.0,
            (Some(a), None) => a.max.y + 1.0,
            (None, None) => return,
        };
        let x = egui::Rangef::new(span.min + 4.0, span.max - 4.0);
        painter.hline(x, y, stroke);
        painter.circle_filled(Pos2::new(x.min, y), 3.5, color);
    } else {
        let x = match (rects.get(to.wrapping_sub(1)), rects.get(to)) {
            (Some(a), Some(b)) => (a.max.x + b.min.x) / 2.0,
            (None, Some(b)) => b.min.x - 1.0,
            (Some(a), None) => a.max.x + 1.0,
            (None, None) => return,
        };
        let y = span;
        painter.vline(x, y, stroke);
        painter.circle_filled(Pos2::new(x, y.min), 3.5, color);
    }
}

/// Icon used for an asset in ghosts and drop hints.
pub fn icon(o: &Open) -> &'static str {
    match o {
        Open::Room(_) => "▦",
        Open::Object(_) => "◆",
        Open::Sprite(_) => "▣",
        Open::Sound(_) => "♪",
        Open::Script(_) => "✎",
        Open::Settings => "⚙",
    }
}

pub fn asset_label(o: &Open) -> String {
    match o {
        Open::Room(n) | Open::Object(n) | Open::Sprite(n) | Open::Sound(n) | Open::Script(n) => format!("{} {n}", icon(o)),
        Open::Settings => "⚙ Ajustes".into(),
    }
}

/// Draws the label of whatever is being dragged next to the cursor (on top of everything).
pub fn ghost(ctx: &egui::Context) {
    let text = if let Some(o) = DragAndDrop::payload::<Open>(ctx) {
        asset_label(&o)
    } else if let Some(d) = DragAndDrop::payload::<ListDrag>(ctx) {
        d.label.clone()
    } else {
        return;
    };
    let Some(p) = ctx.pointer_latest_pos() else { return };
    let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::unique("modeditor_dnd_ghost")));
    let font = FontId::proportional(13.0);
    let visuals = ctx.global_style().visuals.clone();
    let galley = painter.layout_no_wrap(text, font, visuals.text_color());
    let rect = Rect::from_min_size(p + Vec2::new(14.0, 10.0), galley.size() + Vec2::new(16.0, 8.0));
    painter.add(egui::epaint::Shadow { offset: [0, 2], blur: 8, spread: 0, color: Color32::from_black_alpha(40) }.as_shape(rect, CornerRadius::same(5)));
    painter.rect_filled(rect, 5.0, visuals.window_fill);
    painter.rect_stroke(rect, 5.0, Stroke::new(1.5, visuals.selection.stroke.color), egui::StrokeKind::Inside);
    painter.galley(rect.min + Vec2::new(8.0, 4.0), galley, visuals.text_color());
}

/// Dashed-looking accent frame and a hint, painted over a drop target while it accepts.
pub fn drop_hint(ui: &Ui, rect: Rect, text: &str) {
    let c = ui.visuals().selection.stroke.color;
    let painter = ui.painter().clone().with_clip_rect(rect.expand(2.0));
    painter.rect_filled(rect, 4.0, c.gamma_multiply(0.08));
    painter.rect_stroke(rect.shrink(1.0), 4.0, Stroke::new(2.0, c), egui::StrokeKind::Inside);
    if !text.is_empty() {
        painter.text(rect.center_top() + Vec2::new(0.0, 10.0), Align2::CENTER_TOP, text, FontId::proportional(13.0), c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_item_down_up_and_to_the_end() {
        let base = vec!['a', 'b', 'c', 'd'];
        let mut v = base.clone();
        assert_eq!(move_item(&mut v, 0, 2), 1); // a before c
        assert_eq!(v, ['b', 'a', 'c', 'd']);
        let mut v = base.clone();
        assert_eq!(move_item(&mut v, 3, 0), 0);
        assert_eq!(v, ['d', 'a', 'b', 'c']);
        let mut v = base.clone();
        assert_eq!(move_item(&mut v, 1, 4), 3); // to the end
        assert_eq!(v, ['a', 'c', 'd', 'b']);
        let mut v = base.clone();
        move_item(&mut v, 2, 2); // no-op
        move_item(&mut v, 2, 3); // no-op too (right after itself)
        assert_eq!(v, base);
        assert!(is_noop(2, 2) && is_noop(2, 3) && !is_noop(2, 1));
        let mut v = base.clone();
        move_item(&mut v, 9, 0); // out of range: untouched
        move_item(&mut v, 0, 99); // clamped to the end
        assert_eq!(v, ['b', 'c', 'd', 'a']);
    }

    #[test]
    fn insert_index_uses_row_centres() {
        let rows: Vec<Rect> = (0..3).map(|i| Rect::from_min_size(Pos2::new(0.0, i as f32 * 20.0), Vec2::new(100.0, 18.0))).collect();
        assert_eq!(insert_index(&rows, Pos2::new(5.0, -3.0), true), 0);
        assert_eq!(insert_index(&rows, Pos2::new(5.0, 12.0), true), 1);
        assert_eq!(insert_index(&rows, Pos2::new(5.0, 45.0), true), 2);
        assert_eq!(insert_index(&rows, Pos2::new(5.0, 99.0), true), 3);
    }

    #[test]
    fn saved_order_puts_new_assets_last() {
        let names = vec!["a".to_string(), "b".into(), "c".into(), "new".into()];
        let order = vec!["c".to_string(), "gone".into(), "a".into(), "b".into()];
        assert_eq!(apply_order(names.clone(), &order), ["c", "a", "b", "new"]);
        assert_eq!(apply_order(names.clone(), &[]), names);
    }

    #[test]
    fn order_is_saved_with_the_layout() {
        use crate::workspace::{LayoutFile, Workspaces};
        let mut f = Workspaces::default().to_file("/p", [true; 4]);
        let plain = toml::to_string(&f).unwrap();
        assert!(!plain.contains("order"), "no custom order: nothing written\n{plain}");
        f.order.insert("sprites".into(), vec!["spr_b".into(), "spr_a".into()]);
        f.order.insert("objects".into(), vec!["Player".into()]);
        let text = toml::to_string(&f).unwrap();
        let back: LayoutFile = toml::from_str(&text).unwrap();
        assert_eq!(back, f);
        assert_eq!(apply_order(vec!["spr_a".into(), "spr_b".into(), "spr_c".into()], &back.order["sprites"]), ["spr_b", "spr_a", "spr_c"]);
        // layouts saved before the order existed still load
        let old: LayoutFile = toml::from_str(&plain).unwrap();
        assert!(old.order.is_empty());
    }
}
