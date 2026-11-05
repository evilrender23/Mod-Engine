//! Workspaces: every tab is an infinite canvas where each open asset editor is a floating
//! window, like the GameMaker IDE.
//!
//! What GameMaker ships (`Workspace.dll`, `IDE.dll`, `styles_workspace.json`,
//! `defaultProjectLayout.xml`) and what we keep from it:
//!
//! * a workspace area that pans and zooms (`min_zoom` 0.1, `max_zoom` 20 in GM; we use
//!   0.25..2 because text gets blurry beyond that),
//! * windows opened from another window are *chained*: placed to its right and joined by a
//!   link line (`WorkspaceChain`, `workspace_link_stack`, `ChainLinkColour`),
//! * several workspace tabs (`Workspace_TabTitle "Workspace 1"`, `+` to add one),
//! * docks around it (Inspector left, Asset Browser right, Output bottom) — those live in `app.rs`.
//!
//! The layout (workspaces, window rectangles, chains, docks) is saved per project in
//! `~/.config/modengine/layouts/`.

use crate::app::{Open, Tab};
use eframe::egui::{
    self, emath::TSTransform, Align2, Color32, CornerRadius, CursorIcon, FontId, Id, LayerId, Order, Pos2, Rect, Sense, Stroke, StrokeKind, Ui,
    UiBuilder, Vec2,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const TITLE_H: f32 = 24.0;
const MIN_SIZE: Vec2 = Vec2::new(260.0, 140.0);
const ZOOM: std::ops::RangeInclusive<f32> = 0.25..=2.0;
const GAP: f32 = 40.0;

pub struct Win {
    pub id: u64,
    pub tab: Tab,
    /// Top-left corner in canvas coordinates.
    pub pos: Pos2,
    pub size: Vec2,
    pub collapsed: bool,
    pub maximized: bool,
    /// The window this one was opened from (drawn as a chain link).
    pub parent: Option<u64>,
}

impl Win {
    pub fn rect(&self) -> Rect {
        let h = if self.collapsed { TITLE_H } else { self.size.y };
        Rect::from_min_size(self.pos, Vec2::new(self.size.x, h))
    }
}

pub struct Workspace {
    pub name: String,
    pub wins: Vec<Win>,
    pub pan: Vec2,
    pub zoom: f32,
    pub focus: Option<u64>,
}

impl Workspace {
    pub fn new(name: impl Into<String>) -> Self {
        Workspace { name: name.into(), wins: vec![], pan: Vec2::ZERO, zoom: 1.0, focus: None }
    }

    fn to_global(&self, canvas: Rect) -> TSTransform {
        TSTransform::new(canvas.min.to_vec2() + self.pan, self.zoom)
    }

    /// Where a new window goes: to the right of `after` (its chain), or at the top-left of
    /// the visible area, nudged so it never lands exactly on another window.
    fn place(&self, after: Option<u64>, size: Vec2, view: Rect) -> Pos2 {
        let mut p = match after.and_then(|id| self.wins.iter().find(|w| w.id == id)) {
            Some(w) => w.pos + Vec2::new(w.size.x + GAP, 0.0),
            None => {
                // first free spot along the visible top edge
                let mut p = view.min + Vec2::splat(30.0);
                let row: Vec<Rect> = self.wins.iter().map(|w| w.rect()).collect();
                while row.iter().any(|r| r.intersects(Rect::from_min_size(p, size))) && p.x < view.max.x {
                    let hit = row.iter().filter(|r| r.intersects(Rect::from_min_size(p, size))).map(|r| r.max.x).fold(p.x, f32::max);
                    p.x = hit + GAP;
                }
                if p.x >= view.max.x {
                    p = view.min + Vec2::splat(30.0);
                }
                p
            }
        };
        while self.wins.iter().any(|w| (w.pos - p).length() < 4.0) {
            p += Vec2::splat(24.0);
        }
        p
    }
}

/// Things the app has to handle after drawing.
pub enum WsAction {
    /// The user asked to close this window (the app checks for unsaved changes).
    Close(u64),
    /// A workspace tab's close button: close all its windows, then the tab.
    CloseWorkspace(usize),
    /// An asset from the browser was dropped on the empty canvas at this canvas position.
    DropAsset(Open, Pos2),
}

pub struct Workspaces {
    pub list: Vec<Workspace>,
    pub active: usize,
    next_id: u64,
    renaming: Option<(usize, String)>,
    /// Window to scroll into view and raise on the next frame.
    reveal: Option<u64>,
    /// Window to raise (without scrolling) on the next frame.
    raise: Option<u64>,
    canvas: Rect,
    /// Screen rectangles of the workspace tabs (last frame), to drop windows on them.
    tab_rects: Vec<Rect>,
    /// A window title is being dragged (last frame): tabs highlight as drop targets.
    title_drag: bool,
}

impl Default for Workspaces {
    fn default() -> Self {
        Workspaces {
            list: vec![Workspace::new("Workspace 1")],
            active: 0,
            next_id: 1,
            renaming: None,
            reveal: None,
            raise: None,
            canvas: Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 700.0)),
            tab_rects: vec![],
            title_drag: false,
        }
    }
}

pub fn default_size(open: &Open) -> Vec2 {
    match open {
        Open::Room(_) => Vec2::new(940.0, 580.0),
        Open::Object(_) => Vec2::new(780.0, 500.0),
        Open::Sprite(_) => Vec2::new(780.0, 520.0),
        Open::Sound(_) => Vec2::new(440.0, 420.0),
        Open::Script(_) => Vec2::new(600.0, 400.0),
        Open::Settings => Vec2::new(560.0, 500.0),
    }
}

impl Workspaces {
    pub fn cur(&self) -> &Workspace {
        &self.list[self.active]
    }
    pub fn cur_mut(&mut self) -> &mut Workspace {
        &mut self.list[self.active]
    }

    pub fn wins(&self) -> impl Iterator<Item = &Win> {
        self.list.iter().flat_map(|w| w.wins.iter())
    }
    pub fn wins_mut(&mut self) -> impl Iterator<Item = &mut Win> {
        self.list.iter_mut().flat_map(|w| w.wins.iter_mut())
    }
    pub fn tabs_mut(&mut self) -> impl Iterator<Item = &mut Tab> {
        self.wins_mut().map(|w| &mut w.tab)
    }

    pub fn find(&self, open: &Open) -> Option<u64> {
        self.wins().find(|w| w.tab.open_key() == *open).map(|w| w.id)
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut Win> {
        self.wins_mut().find(|w| w.id == id)
    }

    /// The focused window of the visible workspace.
    pub fn focused(&self) -> Option<&Win> {
        let ws = self.cur();
        ws.focus.and_then(|id| ws.wins.iter().find(|w| w.id == id))
    }
    pub fn focused_mut(&mut self) -> Option<&mut Win> {
        let ws = self.cur_mut();
        let id = ws.focus?;
        ws.wins.iter_mut().find(|w| w.id == id)
    }

    /// Switches to the workspace holding `id`, focuses it and brings it into view.
    pub fn focus(&mut self, id: u64) {
        if let Some(i) = self.list.iter().position(|ws| ws.wins.iter().any(|w| w.id == id)) {
            self.active = i;
            self.list[i].focus = Some(id);
            self.reveal = Some(id);
        }
    }

    /// Adds an editor to the visible workspace, chained to `parent` (or to the focused window).
    pub fn add(&mut self, tab: Tab, parent: Option<u64>) -> u64 {
        let size = default_size(&tab.open_key());
        let id = self.next_id;
        self.next_id += 1;
        let canvas = self.canvas;
        let ws = self.cur_mut();
        let view = ws.to_global(canvas).inverse() * canvas;
        let parent = parent.filter(|p| ws.wins.iter().any(|w| w.id == *p));
        let pos = ws.place(parent, size, view);
        ws.wins.push(Win { id, tab, pos, size, collapsed: false, maximized: false, parent });
        self.focus(id);
        id
    }

    /// Adds a window with a known rectangle (restoring a saved layout).
    pub fn add_at(&mut self, ws: usize, tab: Tab, rect: Rect, collapsed: bool, maximized: bool) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.list[ws].wins.push(Win { id, tab, pos: rect.min, size: rect.size().max(MIN_SIZE), collapsed, maximized, parent: None });
        id
    }

    pub fn remove(&mut self, id: u64) -> Option<Win> {
        for ws in &mut self.list {
            if let Some(i) = ws.wins.iter().position(|w| w.id == id) {
                let w = ws.wins.remove(i);
                for o in &mut ws.wins {
                    if o.parent == Some(id) {
                        o.parent = w.parent;
                    }
                }
                if ws.focus == Some(id) {
                    ws.focus = w.parent.filter(|p| ws.wins.iter().any(|o| o.id == *p)).or(ws.wins.last().map(|o| o.id));
                }
                return Some(w);
            }
        }
        None
    }

    /// Moves a window (from any workspace) to `pos` in the visible workspace, focused and on top.
    pub fn place_at(&mut self, id: u64, pos: Pos2) {
        if !self.cur().wins.iter().any(|w| w.id == id) {
            let active = self.active;
            self.move_to_workspace(id, active);
        }
        if let Some(w) = self.cur_mut().wins.iter_mut().find(|w| w.id == id) {
            w.pos = pos;
            w.maximized = false;
            self.cur_mut().focus = Some(id);
            self.raise = Some(id);
            self.reveal = None; // it is where the user dropped it: don't scroll
        }
    }

    /// Moves a window to another workspace tab (unchained, at a free spot of its view) and
    /// shows that workspace.
    pub fn move_to_workspace(&mut self, id: u64, to: usize) -> bool {
        if to >= self.list.len() || self.list[to].wins.iter().any(|w| w.id == id) {
            return false;
        }
        let Some(mut w) = self.remove(id) else { return false };
        let canvas = self.canvas;
        let ws = &mut self.list[to];
        let view = ws.to_global(canvas).inverse() * canvas;
        w.parent = None;
        w.pos = ws.place(None, w.size, view);
        ws.wins.push(w);
        self.active = to;
        self.focus(id);
        true
    }

    /// Reorders the workspace tabs (`to` is an insertion index), keeping the visible one.
    pub fn move_workspace(&mut self, from: usize, to: usize) {
        let mut idx: Vec<usize> = (0..self.list.len()).collect();
        crate::dnd::move_item(&mut idx, from, to);
        crate::dnd::move_item(&mut self.list, from, to);
        self.active = idx.iter().position(|i| *i == self.active).unwrap_or(0);
        self.renaming = None;
    }

    pub fn add_workspace(&mut self) {
        let mut n = self.list.len() + 1;
        while self.list.iter().any(|w| w.name == format!("Workspace {n}")) {
            n += 1;
        }
        self.list.push(Workspace::new(format!("Workspace {n}")));
        self.active = self.list.len() - 1;
    }

    /// Removes an (empty) workspace tab; there is always at least one.
    pub fn remove_workspace(&mut self, i: usize) {
        if self.list.len() > 1 && i < self.list.len() && self.list[i].wins.is_empty() {
            self.list.remove(i);
            if self.active >= self.list.len() || self.active > i {
                self.active = self.active.saturating_sub(1).min(self.list.len() - 1);
            }
        }
    }

    /// Pans and zooms the visible workspace so every window fits.
    pub fn fit_all(&mut self) {
        let canvas = self.canvas;
        let ws = self.cur_mut();
        let Some(bb) = ws.wins.iter().map(|w| w.rect()).reduce(|a, b| a.union(b)) else {
            ws.pan = Vec2::ZERO;
            ws.zoom = 1.0;
            return;
        };
        let bb = bb.expand(30.0);
        ws.zoom = (canvas.width() / bb.width()).min(canvas.height() / bb.height()).clamp(*ZOOM.start(), 1.0);
        ws.pan = (canvas.size() - bb.size() * ws.zoom) / 2.0 - bb.min.to_vec2() * ws.zoom;
    }

    pub fn reset_zoom(&mut self) {
        let canvas = self.canvas;
        let ws = self.cur_mut();
        let center = (canvas.size() / 2.0 - ws.pan) / ws.zoom;
        ws.zoom = 1.0;
        ws.pan = canvas.size() / 2.0 - center;
    }

    // ------------------------------------------------------------------ drawing

    /// Workspace tabs bar.
    pub fn tab_bar(&mut self, ui: &mut Ui, dirty: &HashSet<u64>) -> Vec<WsAction> {
        let mut out = vec![];
        let mut add = false;
        let list_id = Id::unique("modeditor_ws_tabs");
        let mut rects = vec![];
        let pointer = ui.ctx().pointer_latest_pos();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for i in 0..self.list.len() {
                if let Some((ri, text)) = &mut self.renaming {
                    if *ri == i {
                        let r = ui.add(egui::TextEdit::singleline(text).desired_width(110.0));
                        r.request_focus();
                        if r.lost_focus() || ui.input(|k| k.key_pressed(egui::Key::Enter)) {
                            let t = text.trim().to_string();
                            if !t.is_empty() {
                                self.list[i].name = t;
                            }
                            self.renaming = None;
                        }
                        continue;
                    }
                }
                let ws = &self.list[i];
                let unsaved = ws.wins.iter().any(|w| dirty.contains(&w.id));
                let label = format!("{}{}", ws.name, if unsaved { " ●" } else { "" });
                let dragged = crate::dnd::dragging_from(ui.ctx(), list_id) == Some(i);
                let r = ui.add(egui::Button::selectable(i == self.active || dragged, label.clone()).min_size(Vec2::new(90.0, 22.0)).sense(Sense::click_and_drag()));
                crate::dnd::source(&r, || crate::dnd::ListDrag { list: list_id, from: i, label: format!("▤ {label}") });
                rects.push(r.rect);
                if r.clicked() {
                    self.active = i;
                }
                // a window title dragged over another tab: drop it there
                if self.title_drag && i != self.active && pointer.is_some_and(|p| r.rect.contains(p)) {
                    crate::dnd::drop_hint(ui, r.rect, "");
                }
                if r.double_clicked() {
                    self.renaming = Some((i, self.list[i].name.clone()));
                }
                r.context_menu(|ui| {
                    if ui.button("Renombrar").clicked() {
                        self.renaming = Some((i, self.list[i].name.clone()));
                        ui.close();
                    }
                    if ui.add_enabled(self.list.len() > 1, egui::Button::new("Cerrar workspace")).clicked() {
                        out.push(WsAction::CloseWorkspace(i));
                        ui.close();
                    }
                });
                if self.list.len() > 1 && ui.small_button("×").on_hover_text("cerrar workspace").clicked() {
                    out.push(WsAction::CloseWorkspace(i));
                }
                ui.add_space(4.0);
            }
            if ui.small_button("+").on_hover_text("nuevo workspace").clicked() {
                add = true;
            }
            if let Some((from, to)) = crate::dnd::reorder(ui, list_id, &rects, false) {
                self.move_workspace(from, to);
            }
        });
        self.tab_rects = rects;
        if add {
            self.add_workspace();
        }
        out
    }

    /// The canvas of the visible workspace with its windows. `content` draws one editor.
    pub fn canvas(&mut self, ui: &mut Ui, dirty: &HashSet<u64>, mut content: impl FnMut(&mut Ui, &mut Win)) -> Vec<WsAction> {
        let mut out = vec![];
        let (canvas, bg) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        self.canvas = canvas;
        let ctx = ui.ctx().clone();
        let colors = crate::theme::colors(&ctx);
        let accent = ui.visuals().selection.stroke.color;

        // ---- pan & zoom on the empty background
        {
            let ws = self.cur_mut();
            if bg.dragged_by(egui::PointerButton::Middle) || bg.dragged_by(egui::PointerButton::Primary) {
                ws.pan += bg.drag_delta();
            }
            if bg.hovered() {
                if let Some(hp) = bg.hover_pos() {
                    // Plain wheel pans; Ctrl+wheel and pinch arrive as `zoom_delta`
                    // (egui converts Ctrl+wheel, made gentle by `zoom::install`).
                    let (zoom_delta, scroll) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta));
                    let factor = crate::zoom::factor(0.0, zoom_delta);
                    if factor != 1.0 {
                        let at = (hp - canvas.min - ws.pan) / ws.zoom;
                        ws.zoom = (ws.zoom * factor).clamp(*ZOOM.start(), *ZOOM.end());
                        ws.pan = hp - canvas.min - at * ws.zoom;
                    } else if scroll != Vec2::ZERO {
                        ws.pan += scroll;
                    }
                }
            }
            if bg.double_clicked() {
                ws.focus = None;
            }
        }
        // ---- an asset from the browser dropped on the empty canvas opens there
        let drop_hover = bg.dnd_hover_payload::<Open>();
        if let Some(what) = bg.dnd_release_payload::<Open>() {
            if let Some(p) = ctx.pointer_latest_pos() {
                let at = self.cur().to_global(canvas).inverse() * p;
                out.push(WsAction::DropAsset((*what).clone(), at - Vec2::new(60.0, TITLE_H / 2.0)));
            }
        }
        bg.context_menu(|ui| {
            if ui.button("Ver todas las ventanas").clicked() {
                self.fit_all();
                ui.close();
            }
            if ui.button("Zoom al 100 %").clicked() {
                self.reset_zoom();
                ui.close();
            }
            if ui.button("Cerrar todas las ventanas").clicked() {
                out.extend(self.cur().wins.iter().map(|w| WsAction::Close(w.id)));
                ui.close();
            }
        });

        // ---- reveal a window that was just opened or re-focused
        if let Some(id) = self.reveal.take() {
            let ws = self.cur_mut();
            if let Some(w) = ws.wins.iter().find(|w| w.id == id) {
                let t = ws.to_global(canvas);
                let r = t * w.rect();
                // try to show the window together with the one it is chained to
                let parent = w.parent.and_then(|p| ws.wins.iter().find(|o| o.id == p)).map(|p| t * p.rect());
                let r = match parent {
                    Some(pr) if pr.union(r).width() < canvas.width() - 40.0 && pr.union(r).height() < canvas.height() - 40.0 => pr.union(r),
                    _ => r,
                };
                if !canvas.contains_rect(r) {
                    let fits = r.width() < canvas.width() - 40.0;
                    let target = if fits && r.min.x > canvas.min.x && r.max.x > canvas.max.x {
                        // slide left just enough to show the new window
                        Vec2::new(canvas.max.x - 20.0 - r.max.x, (canvas.min.y + 20.0 - r.min.y).max(0.0).min(canvas.max.y - r.max.y))
                    } else {
                        canvas.min + Vec2::splat(20.0) - r.min
                    };
                    ws.pan += target;
                }
            }
            ctx.move_to_top(LayerId::new(Order::Middle, win_id(id)));
        }
        if let Some(id) = self.raise.take() {
            ctx.move_to_top(LayerId::new(Order::Middle, win_id(id)));
        }
        let tab_rects = std::mem::take(&mut self.tab_rects);
        let mut to_tab: Vec<(u64, usize)> = vec![];
        let mut title_drag = false;

        let ws = &mut self.list[self.active];
        let t = ws.to_global(canvas);
        let painter = ui.painter_at(canvas);

        // ---- background: soft dot grid
        painter.rect_filled(canvas, 0.0, colors.workspace);
        let step = 32.0 * ws.zoom;
        if step >= 10.0 {
            let start = canvas.min + Vec2::new(ws.pan.x.rem_euclid(step), ws.pan.y.rem_euclid(step));
            let mut y = start.y;
            while y < canvas.max.y {
                let mut x = start.x;
                while x < canvas.max.x {
                    painter.circle_filled(Pos2::new(x, y), 1.0, colors.workspace_dot);
                    x += step;
                }
                y += step;
            }
        }
        if ws.wins.is_empty() {
            painter.text(
                canvas.center(),
                Align2::CENTER_CENTER,
                "Haz doble clic en un asset del navegador, o arrástralo aquí, para abrirlo\narrastra el fondo para moverte · Ctrl + rueda para el zoom",
                FontId::proportional(14.0),
                colors.weak,
            );
        }

        // ---- chain links (below the windows)
        for w in ws.wins.iter().filter(|w| !w.maximized) {
            let Some(p) = w.parent.and_then(|p| ws.wins.iter().find(|o| o.id == p)) else { continue };
            if p.maximized {
                continue;
            }
            let (a, b) = (p.rect(), w.rect());
            let (from, to) = if b.center().x >= a.center().x {
                (Pos2::new(a.max.x, a.min.y + TITLE_H / 2.0), Pos2::new(b.min.x, b.min.y + TITLE_H / 2.0))
            } else {
                (Pos2::new(a.min.x, a.min.y + TITLE_H / 2.0), Pos2::new(b.max.x, b.min.y + TITLE_H / 2.0))
            };
            let (from, to) = (t * from, t * to);
            let dx = ((to.x - from.x) / 2.0).abs().max(30.0) * if to.x >= from.x { 1.0 } else { -1.0 };
            let curve = egui::epaint::CubicBezierShape::from_points_stroke(
                [from, from + Vec2::new(dx, 0.0), to - Vec2::new(dx, 0.0), to],
                false,
                Color32::TRANSPARENT,
                Stroke::new(3.0 * ws.zoom, colors.chain),
            );
            painter.add(curve);
            painter.circle_filled(from, 4.0 * ws.zoom, colors.chain);
            painter.circle_filled(to, 4.0 * ws.zoom, colors.chain);
        }

        // ---- zoom badge
        painter.text(
            canvas.right_bottom() - Vec2::new(10.0, 8.0),
            Align2::RIGHT_BOTTOM,
            format!("{:.0} %   ·   {} ventanas", ws.zoom * 100.0, ws.wins.len()),
            FontId::proportional(11.0),
            colors.weak,
        );

        if let Some(what) = &drop_hover {
            crate::dnd::drop_hint(ui, canvas.shrink(4.0), &format!("Suelta para abrir {} aquí", crate::dnd::asset_label(what)));
        }

        // ---- windows
        let clip = t.inverse() * canvas;
        let mut focus = None;
        for w in &mut ws.wins {
            let id = win_id(w.id);
            ctx.set_transform_layer(LayerId::new(Order::Middle, id), t);
            let max_rect = w.maximized.then(|| clip.shrink(8.0 / ws.zoom));
            let pos = max_rect.map(|r| r.min).unwrap_or(w.pos);
            let is_focused = ws.focus == Some(w.id);
            let resp = egui::Area::new(id).order(Order::Middle).fixed_pos(pos).constrain_to(clip).constrain(false).show(&ctx, |ui| {
                window_ui(ui, w, dirty.contains(&w.id), is_focused, max_rect, accent, &mut content)
            });
            let r = resp.inner;
            if r.focus {
                focus = Some(w.id);
            }
            if r.close {
                out.push(WsAction::Close(w.id));
            }
            title_drag |= r.title_drag;
            if let Some(p) = r.title_drop {
                if let Some(i) = tab_rects.iter().position(|t| t.contains(p)) {
                    to_tab.push((w.id, i));
                }
            }
        }
        if let Some(f) = focus {
            ws.focus = Some(f);
        }
        self.title_drag = title_drag;
        self.tab_rects = tab_rects;
        for (id, i) in to_tab {
            self.move_to_workspace(id, i);
        }
        out
    }
}

pub fn win_id(id: u64) -> Id {
    Id::unique(("modeditor_ws_window", id))
}

#[derive(Default)]
struct WinOut {
    focus: bool,
    close: bool,
    /// The title bar is being dragged.
    title_drag: bool,
    /// The title bar drag ended here (screen coordinates).
    title_drop: Option<Pos2>,
}

fn window_ui(ui: &mut Ui, w: &mut Win, dirty: bool, focused: bool, max_rect: Option<Rect>, accent: Color32, content: &mut impl FnMut(&mut Ui, &mut Win)) -> WinOut {
    let mut out = WinOut::default();
    let colors = crate::theme::colors(ui.ctx());
    let size = match max_rect {
        Some(r) => r.size(),
        None if w.collapsed => Vec2::new(w.size.x, TITLE_H),
        None => w.size,
    };
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let r6 = CornerRadius::same(6);
    let fill = ui.visuals().window_fill;
    let border = if focused { accent } else { ui.visuals().window_stroke.color };

    let p = ui.painter();
    p.add(egui::epaint::Shadow { offset: [0, 4], blur: 18, spread: 0, color: Color32::from_black_alpha(if focused { 46 } else { 26 }) }.as_shape(rect, r6));
    p.rect_filled(rect, r6, fill);

    // ---- title bar
    let title_rect = Rect::from_min_size(rect.min, Vec2::new(rect.width(), TITLE_H));
    let tr = CornerRadius { nw: 6, ne: 6, sw: if w.collapsed { 6 } else { 0 }, se: if w.collapsed { 6 } else { 0 } };
    p.rect_filled(title_rect, tr, if focused { colors.title_active } else { colors.title });
    let drag = ui.interact(title_rect, ui.scope_id().with("title"), Sense::click_and_drag());
    if max_rect.is_none() && drag.dragged() {
        w.pos += drag.drag_delta();
        out.title_drag = true;
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    }
    if max_rect.is_none() && drag.drag_stopped() {
        out.title_drop = ui.ctx().pointer_latest_pos();
    }
    if drag.double_clicked() {
        w.maximized = !w.maximized;
        w.collapsed = false;
    }
    if drag.hovered() && max_rect.is_none() && !drag.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::Grab);
    }

    let btn = |ui: &mut Ui, x: f32, text: &str, tip: &str| -> bool {
        let r = Rect::from_min_size(Pos2::new(x, rect.min.y + 2.0), Vec2::new(20.0, TITLE_H - 4.0));
        ui.put(r, egui::Button::new(egui::RichText::new(text).size(12.0)).frame(false)).on_hover_text(tip).clicked()
    };
    if max_rect.is_none() && btn(ui, rect.min.x + 3.0, if w.collapsed { "▸" } else { "▾" }, "plegar") {
        w.collapsed = !w.collapsed;
    }
    if btn(ui, rect.max.x - 23.0, "✕", "cerrar") {
        out.close = true;
    }
    if btn(ui, rect.max.x - 45.0, if w.maximized { "🗗" } else { "🗖" }, "maximizar (doble clic en el título)") {
        w.maximized = !w.maximized;
        w.collapsed = false;
    }
    let title = format!("{}{}", w.tab.window_title(), if dirty { "  ●" } else { "" });
    let text_rect = Rect::from_min_max(Pos2::new(rect.min.x + 26.0, rect.min.y), Pos2::new(rect.max.x - 50.0, title_rect.max.y));
    ui.painter().with_clip_rect(text_rect.intersect(ui.clip_rect())).text(
        Pos2::new(text_rect.min.x, title_rect.center().y),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(12.5),
        ui.visuals().text_color(),
    );

    // ---- content
    if !w.collapsed || max_rect.is_some() {
        let content_rect = Rect::from_min_max(Pos2::new(rect.min.x + 1.0, title_rect.max.y), rect.max - Vec2::splat(1.0));
        ui.painter().hline(content_rect.x_range(), content_rect.min.y, Stroke::new(1.0, ui.visuals().window_stroke.color));
        let mut child = ui.new_child(UiBuilder::new().id_salt("content").max_rect(content_rect.shrink(4.0)).layout(egui::Layout::top_down(egui::Align::Min)));
        child.set_clip_rect(content_rect.intersect(ui.clip_rect()));
        content(&mut child, w);

        // resize from the right edge, bottom edge and corner
        if max_rect.is_none() {
            let edges = [
                (Rect::from_min_max(Pos2::new(rect.max.x - 4.0, rect.min.y + TITLE_H), Pos2::new(rect.max.x + 3.0, rect.max.y - 12.0)), Vec2::X, CursorIcon::ResizeHorizontal),
                (Rect::from_min_max(Pos2::new(rect.min.x, rect.max.y - 4.0), Pos2::new(rect.max.x - 12.0, rect.max.y + 3.0)), Vec2::Y, CursorIcon::ResizeVertical),
                (Rect::from_min_max(rect.max - Vec2::splat(14.0), rect.max + Vec2::splat(3.0)), Vec2::splat(1.0), CursorIcon::ResizeNwSe),
            ];
            for (i, (r, axis, cursor)) in edges.into_iter().enumerate() {
                let resp = ui.interact(r, ui.scope_id().with(("resize", i)), Sense::drag());
                if resp.hovered() || resp.dragged() {
                    ui.ctx().set_cursor_icon(cursor);
                }
                if resp.dragged() {
                    w.size = (w.size + resp.drag_delta() * axis).max(MIN_SIZE);
                }
            }
            let g = ui.visuals().weak_text_color();
            for k in [4.0, 8.0] {
                ui.painter().line_segment([rect.max - Vec2::new(k, 2.0), rect.max - Vec2::new(2.0, k)], Stroke::new(1.0, g));
            }
        }
    }
    ui.painter().rect_stroke(rect, r6, Stroke::new(if focused { 1.5 } else { 1.0 }, border), StrokeKind::Inside);

    if ui.input(|i| i.pointer.any_pressed()) && ui.rect_contains_pointer(rect) {
        out.focus = true;
    }
    out
}

// ------------------------------------------------------------------ saved layout

#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
pub struct LayoutFile {
    pub dir: String,
    pub active: usize,
    /// Inspector, asset browser, output.
    pub docks: [bool; 3],
    /// The Agente dock (absent in layouts saved before it existed).
    #[serde(default)]
    pub agent: bool,
    #[serde(default, rename = "workspace")]
    pub workspaces: Vec<WsFile>,
    /// Custom order of the asset browser sections (absent in older layouts).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub order: crate::dnd::AssetOrder,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct WsFile {
    pub name: String,
    pub pan: [f32; 2],
    pub zoom: f32,
    #[serde(default, rename = "window")]
    pub windows: Vec<WinFile>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct WinFile {
    /// `room:level1`, `object:Player`...
    pub open: String,
    pub rect: [f32; 4],
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default)]
    pub maximized: bool,
    #[serde(default)]
    pub focused: bool,
    /// Index (in this workspace) of the window this one is chained to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<usize>,
}

impl Workspaces {
    /// `docks`: inspector, asset browser, output, agent.
    pub fn to_file(&self, dir: &str, docks: [bool; 4]) -> LayoutFile {
        LayoutFile {
            dir: dir.into(),
            active: self.active,
            docks: [docks[0], docks[1], docks[2]],
            agent: docks[3],
            workspaces: self
                .list
                .iter()
                .map(|ws| WsFile {
                    name: ws.name.clone(),
                    pan: [ws.pan.x, ws.pan.y],
                    zoom: ws.zoom,
                    windows: ws
                        .wins
                        .iter()
                        .map(|w| WinFile {
                            open: w.tab.open_key().key(),
                            rect: [w.pos.x, w.pos.y, w.size.x, w.size.y],
                            collapsed: w.collapsed,
                            maximized: w.maximized,
                            focused: ws.focus == Some(w.id),
                            parent: w.parent.and_then(|p| ws.wins.iter().position(|o| o.id == p)),
                        })
                        .collect(),
                })
                .collect(),
            order: Default::default(),
        }
    }

    /// Rebuilds workspaces from a saved layout. `make` creates the editor for an asset (or
    /// `None` when it no longer exists, so the window is skipped).
    pub fn from_file(f: &LayoutFile, mut make: impl FnMut(&Open) -> Option<Tab>) -> Workspaces {
        let mut s = Workspaces { list: vec![], ..Default::default() };
        for wf in &f.workspaces {
            s.list.push(Workspace { name: wf.name.clone(), wins: vec![], pan: Vec2::new(wf.pan[0], wf.pan[1]), zoom: wf.zoom.clamp(*ZOOM.start(), *ZOOM.end()), focus: None });
            let wi = s.list.len() - 1;
            let mut ids: Vec<Option<u64>> = vec![];
            for w in &wf.windows {
                let id = Open::parse(&w.open).and_then(|o| make(&o)).map(|tab| {
                    let r = Rect::from_min_size(Pos2::new(w.rect[0], w.rect[1]), Vec2::new(w.rect[2], w.rect[3]));
                    s.add_at(wi, tab, r, w.collapsed, w.maximized)
                });
                ids.push(id);
            }
            for (k, w) in wf.windows.iter().enumerate() {
                let Some(id) = ids[k] else { continue };
                let parent = w.parent.and_then(|p| ids.get(p).copied().flatten());
                if let Some(win) = s.list[wi].wins.iter_mut().find(|x| x.id == id) {
                    win.parent = parent;
                }
                if w.focused {
                    s.list[wi].focus = Some(id);
                }
            }
        }
        if s.list.is_empty() {
            s.list.push(Workspace::new("Workspace 1"));
        }
        s.active = f.active.min(s.list.len() - 1);
        s
    }
}

pub fn layout_path(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let canon = dir.canonicalize().unwrap_or(dir.to_path_buf());
    // FNV-1a: stable across runs and Rust versions
    let mut h: u64 = 0xcbf29ce484222325;
    for b in canon.to_string_lossy().bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    let name = canon.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(format!(".config/modengine/layouts/{name}-{h:016x}.toml")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::ScriptEditor;

    fn tab(rel: &str) -> Tab {
        Tab::Script(ScriptEditor::new(rel))
    }

    #[test]
    fn chained_windows_go_to_the_right() {
        let mut s = Workspaces::default();
        let a = s.add(tab("a.ms"), None);
        let b = s.add(tab("b.ms"), Some(a)); // opened from a: chained to it
        let (wa, wb) = (s.cur().wins[0].rect(), s.cur().wins[1].rect());
        assert_eq!(s.cur().wins[1].parent, Some(a));
        assert!(wb.min.x >= wa.max.x, "{wa:?} {wb:?}");
        assert_eq!(wa.min.y, wb.min.y);
        assert_eq!(s.focused().map(|w| w.id), Some(b));
        // closing the middle of a chain re-links it
        let c = s.add(tab("c.ms"), Some(b));
        s.remove(b);
        assert_eq!(s.get_mut(c).unwrap().parent, Some(a));
    }

    #[test]
    fn find_and_focus_switch_workspace() {
        let mut s = Workspaces::default();
        let a = s.add(tab("a.ms"), None);
        s.add_workspace();
        assert_eq!(s.active, 1);
        assert_eq!(s.find(&Open::Script("a.ms".into())), Some(a));
        s.focus(a);
        assert_eq!(s.active, 0);
        s.remove_workspace(1);
        assert_eq!(s.list.len(), 1);
    }

    #[test]
    fn layout_roundtrip() {
        let mut s = Workspaces::default();
        let a = s.add(tab("a.ms"), None);
        s.add(tab("b.ms"), Some(a));
        s.add_workspace();
        s.add(tab("c.ms"), None);
        s.list[0].zoom = 0.75;
        let f = s.to_file("/p", [true, false, true, true]);
        let text = toml::to_string(&f).unwrap();
        let back: LayoutFile = toml::from_str(&text).unwrap();
        assert_eq!(back, f);
        assert!(back.agent);
        // Layouts saved before the Agente dock still load, with it hidden.
        let old: LayoutFile = toml::from_str(&text.replace("agent = true\n", "")).unwrap();
        assert!(!old.agent && old.docks == [true, false, true]);
        let r = Workspaces::from_file(&back, |o| match o {
            Open::Script(rel) if rel != "c.ms" => Some(tab(rel)),
            _ => None, // c.ms was deleted: its window is skipped
        });
        assert_eq!(r.list.len(), 2);
        assert_eq!(r.list[0].wins.len(), 2);
        assert_eq!(r.list[0].wins[1].parent, Some(r.list[0].wins[0].id));
        assert_eq!(r.list[0].zoom, 0.75);
        assert!(r.list[1].wins.is_empty());
        assert_eq!(r.active, 1);
    }

    #[test]
    fn drag_tabs_to_reorder_and_windows_onto_tabs() {
        use egui_kittest::{kittest::Queryable, Harness};
        let mut ws = Workspaces::default();
        let a = ws.add(tab("a.ms"), None);
        ws.add_workspace();
        ws.add_workspace();
        ws.active = 0;
        let mut h = Harness::builder().with_size([1200.0, 700.0]).allow_missing_glyphs().with_accessibility_check(false).build_ui_state(
            |ui, ws: &mut Workspaces| {
                ws.tab_bar(ui, &HashSet::new());
                ws.canvas(ui, &HashSet::new(), |_, _| {});
            },
            ws,
        );
        h.run_steps(3);
        let names = |h: &Harness<Workspaces>| h.state().list.iter().map(|w| w.name.clone()).collect::<Vec<_>>();
        // drag "Workspace 1" (visible) after "Workspace 3"
        let from = h.get_by_label("Workspace 1").rect().center();
        let last = h.get_by_label("Workspace 3").rect();
        crate::room::tests::drag(&mut h, from, last.right_center() + Vec2::new(8.0, 0.0));
        assert_eq!(names(&h), ["Workspace 2", "Workspace 3", "Workspace 1"]);
        assert_eq!(h.state().active, 2, "the visible workspace stays visible");
        // drag the window's title bar onto the "Workspace 2" tab: it moves there
        let title = {
            let s = h.state();
            let w = &s.cur().wins[0];
            s.cur().to_global(s.canvas) * (w.pos + Vec2::new(120.0, TITLE_H / 2.0))
        };
        let target = h.get_by_label("Workspace 2").rect().center();
        crate::room::tests::drag(&mut h, title, target);
        let s = h.state();
        assert_eq!(s.list[0].name, "Workspace 2");
        assert_eq!(s.list[0].wins.iter().map(|w| w.id).collect::<Vec<_>>(), [a]);
        assert!(s.list[2].wins.is_empty());
        assert_eq!(s.active, 0);
    }

    #[test]
    fn painting_inside_a_zoomed_window_hits_the_right_cell() {
        use crate::app::{Cx, LogLine};
        use crate::project::Project;
        use crate::room::RoomEditor;
        use crate::tex::TexCache;
        use eframe::egui::{Event, Modifiers, PointerButton};
        use egui_kittest::Harness;

        struct St {
            ws: Workspaces,
            project: Project,
            tex: TexCache,
            log: Vec<LogLine>,
        }
        let dir = crate::room::tests::temp_project("ws");
        let project = Project::open(&dir).unwrap();
        let path = project.rooms.iter().find(|r| r.name == "title").unwrap().path.clone();
        let mut ws = Workspaces::default();
        ws.add(tab("pad.ms"), None); // a first window, so the room is chained to its right
        let first = ws.cur().wins[0].id;
        let id = ws.add(Tab::Room(RoomEditor::open("title", &path).unwrap()), Some(first));
        ws.cur_mut().zoom = 0.8;
        let st = St { ws, project, tex: TexCache::default(), log: vec![] };
        let mut h = Harness::builder().with_size([1600.0, 900.0]).allow_missing_glyphs().with_accessibility_check(false).build_ui_state(
            |ui, s: &mut St| {
                let focused = s.ws.cur().focus;
                let (project, tex, log) = (&mut s.project, &mut s.tex, &mut s.log);
                s.ws.canvas(ui, &HashSet::new(), |ui, w| {
                    let mut open = vec![];
                    let mut cx = Cx { project, tex, log, open: &mut open, active: focused == Some(w.id) };
                    w.tab.ui(ui, &mut cx);
                });
            },
            st,
        );
        h.run_steps(4);
        let (target, transform) = {
            let s = h.state_mut();
            let canvas = s.ws.canvas;
            let t = s.ws.cur().to_global(canvas);
            let Some(Tab::Room(ed)) = s.ws.get_mut(id).map(|w| &mut w.tab) else { panic!() };
            ed.brush = Some("Coin".into());
            ed.tool = crate::room::Tool::Paint;
            (ed.screen_of(5.0 * 16.0 + 8.0, 3.0 * 16.0 + 8.0), t)
        };
        h.run_steps(2);
        let p = transform * target; // canvas (layer) coordinates -> screen
        h.event(Event::PointerMoved(p));
        h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(2);
        h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(2);
        let s = h.state_mut();
        let Some(Tab::Room(ed)) = s.ws.get_mut(id).map(|w| &mut w.tab) else { panic!() };
        assert_eq!(ed.cell_objects(5, 3), vec!["Coin".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
