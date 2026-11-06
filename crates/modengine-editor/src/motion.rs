//! Motion assets in the editor: paths, timelines, animation curves and sequences.
//! Shared pieces: the asset kinds, their file lists, the editor tab that wraps the four
//! editors, an undo history, a pan/zoom view and the room backdrop.

use crate::app::Cx;
use crate::project::{FileEntry, Project};
use eframe::egui::{self, Color32, Pos2, Rect, Response, Sense, Stroke, Vec2};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AnimKind {
    Path,
    Timeline,
    Curve,
    Sequence,
}

impl AnimKind {
    pub const ALL: [AnimKind; 4] = [AnimKind::Path, AnimKind::Timeline, AnimKind::Curve, AnimKind::Sequence];

    /// Prefix used by `--open path:name` and saved layouts.
    pub fn key(self) -> &'static str {
        match self {
            AnimKind::Path => "path",
            AnimKind::Timeline => "timeline",
            AnimKind::Curve => "curve",
            AnimKind::Sequence => "sequence",
        }
    }
    pub fn parse(k: &str) -> Option<AnimKind> {
        Some(match k {
            "path" => AnimKind::Path,
            "timeline" => AnimKind::Timeline,
            "curve" | "animcurve" => AnimKind::Curve,
            "sequence" => AnimKind::Sequence,
            _ => return None,
        })
    }
    pub fn folder(self) -> &'static str {
        match self {
            AnimKind::Path => "paths",
            AnimKind::Timeline => "timelines",
            AnimKind::Curve => "curves",
            AnimKind::Sequence => "sequences",
        }
    }
    /// Singular label (Spanish UI).
    pub fn label(self) -> &'static str {
        match self {
            AnimKind::Path => "Trayectoria",
            AnimKind::Timeline => "Línea de tiempo",
            AnimKind::Curve => "Curva de animación",
            AnimKind::Sequence => "Secuencia",
        }
    }
    pub fn icon(self) -> &'static str {
        match self {
            AnimKind::Path => "〰",
            AnimKind::Timeline => "⏱",
            AnimKind::Curve => "📈",
            AnimKind::Sequence => "🎞",
        }
    }
    /// Asset browser section key (custom order).
    pub fn section(self) -> &'static str {
        self.folder()
    }
    /// Contents of a new asset file.
    pub fn template(self, p: &Project) -> String {
        match self {
            AnimKind::Path => {
                let (w, h) = (p.game.display.width as f64, p.game.display.height as f64);
                let t = modengine::path::PathToml {
                    smooth: true,
                    points: [(0.2, 0.3), (0.5, 0.2), (0.8, 0.5), (0.5, 0.8)].iter().map(|&(x, y)| modengine::path::PathPoint { x: (x * w).round(), y: (y * h).round(), speed: 100.0 }).collect(),
                    ..Default::default()
                };
                modengine::path::write_toml(&t)
            }
            AnimKind::Timeline => "[[moment]]\nstep = 0\ncode = \"// se ejecuta en el primer paso\"\n".into(),
            AnimKind::Curve => {
                let (kind, points) = modengine::animcurve::preset("ease_in_out").unwrap_or_default();
                let c = modengine::animcurve::AnimCurve { name: String::new(), channels: vec![modengine::animcurve::Channel::new("curve1", kind, points)] };
                modengine::animcurve::write_toml(&c.to_toml())
            }
            AnimKind::Sequence => {
                let t = modengine::sequence::SequenceToml { canvas: Some([p.game.display.width, p.game.display.height]), ..Default::default() };
                modengine::sequence::write_toml(&t).unwrap_or_default()
            }
        }
    }
}

// ------------------------------------------------------------------ file lists

/// The motion asset files of a project.
#[derive(Default)]
pub struct Lists {
    pub items: Vec<(AnimKind, FileEntry)>,
}

impl Lists {
    pub fn load(dir: &Path) -> Lists {
        let mut items = vec![];
        for k in AnimKind::ALL {
            for (name, path, is_toml) in modengine::project::asset_files(&dir.join(k.folder()), &[]) {
                if is_toml {
                    items.push((k, FileEntry { name, path, is_toml }));
                }
            }
        }
        Lists { items }
    }
    pub fn names(&self, k: AnimKind) -> Vec<String> {
        self.items.iter().filter(|(kk, _)| *kk == k).map(|(_, f)| f.name.clone()).collect()
    }
    pub fn all_names(&self) -> impl Iterator<Item = String> + '_ {
        self.items.iter().map(|(_, f)| f.name.clone())
    }
    pub fn path(&self, k: AnimKind, name: &str) -> Option<PathBuf> {
        self.items.iter().find(|(kk, f)| *kk == k && f.name == name).map(|(_, f)| f.path.clone())
    }
}

// ------------------------------------------------------------------ asset browser / inspector

fn rows(p: &Project, k: AnimKind) -> Vec<crate::kinds::Row> {
    p.motion.names(k).into_iter().map(|n| crate::kinds::Row::plain(&n, crate::app::Open::Anim(k, n.clone()))).collect()
}
pub fn path_rows(p: &Project) -> Vec<crate::kinds::Row> {
    rows(p, AnimKind::Path)
}
pub fn timeline_rows(p: &Project) -> Vec<crate::kinds::Row> {
    rows(p, AnimKind::Timeline)
}
pub fn curve_rows(p: &Project) -> Vec<crate::kinds::Row> {
    rows(p, AnimKind::Curve)
}
pub fn sequence_rows(p: &Project) -> Vec<crate::kinds::Row> {
    rows(p, AnimKind::Sequence)
}

/// Inspector details of a motion asset.
pub fn inspector_rows(p: &Project, what: &crate::app::Open) -> Vec<(&'static str, String)> {
    let crate::app::Open::Anim(k, n) = what else { return vec![] };
    let Some(path) = p.motion.path(*k, n) else { return vec![] };
    let mut out = vec![("fichero", path.strip_prefix(&p.dir).unwrap_or(&path).display().to_string())];
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let bad = |e: String| vec![("error", e)];
    match k {
        AnimKind::Path => match toml::from_str::<modengine::path::PathToml>(&text) {
            Ok(t) => {
                let d = modengine::path::PathDef::new(n, t);
                out.push(("puntos", d.data.points.len().to_string()));
                out.push(("longitud", format!("{:.1} px", d.length())));
                out.push(("tipo", format!("{}{}", if d.data.smooth { "curva suave" } else { "rectas" }, if d.data.closed { ", cerrada" } else { "" })));
                if let Some(r) = &d.data.room {
                    out.push(("room de fondo", r.clone()));
                }
            }
            Err(e) => out.extend(bad(e.to_string())),
        },
        AnimKind::Timeline => match toml::from_str::<modengine::timeline::TimelineToml>(&text) {
            Ok(t) => {
                out.push(("momentos", t.moments.len().to_string()));
                out.push(("último paso", t.moments.iter().map(|m| m.step).max().unwrap_or(0).to_string()));
                let errs = t.moments.iter().filter(|m| crate::timeline::moment_error(&m.code).is_some()).count();
                if errs > 0 {
                    out.push(("errores", format!("{errs} momentos con errores")));
                }
            }
            Err(e) => out.extend(bad(e.to_string())),
        },
        AnimKind::Curve => match modengine::animcurve::AnimCurve::load(n, &path) {
            Ok(c) => {
                out.push(("canales", c.channels.iter().map(|c| format!("{} ({})", c.name, c.kind.name())).collect::<Vec<_>>().join(", ")));
            }
            Err(e) => out.extend(bad(e)),
        },
        AnimKind::Sequence => match toml::from_str::<modengine::sequence::SequenceToml>(&text) {
            Ok(t) => {
                out.push(("duración", format!("{} frames a {} fps", t.length, t.fps)));
                out.push(("reproducción", t.playback.name().into()));
                out.push(("pistas", t.tracks.iter().map(|t| format!("{} ({})", t.name, t.asset)).collect::<Vec<_>>().join(", ")));
                out.push(("momentos", t.moments.len().to_string()));
            }
            Err(e) => out.extend(bad(e.to_string())),
        },
    }
    out
}

/// Creates `<folder>/<name>.toml` and reloads the lists.
pub fn create(p: &mut Project, k: AnimKind, name: &str) -> Result<(), String> {
    let path = p.dir.join(k.folder()).join(format!("{name}.toml"));
    if path.exists() {
        return Err(format!("{} ya existe", path.display()));
    }
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&path, k.template(p)).map_err(|e| e.to_string())?;
    p.motion = Lists::load(&p.dir);
    Ok(())
}

/// Moves the asset's file to `.trash` and reloads the lists.
pub fn delete(p: &mut Project, k: AnimKind, name: &str) -> Result<(), String> {
    if let Some(path) = p.motion.path(k, name) {
        p.trash(&path)?;
    }
    p.motion = Lists::load(&p.dir);
    Ok(())
}

// ------------------------------------------------------------------ the tab

pub enum AnimTab {
    Path(crate::path::PathEditor),
    Timeline(crate::timeline::TimelineEditor),
    Curve(crate::curve::CurveEditor),
    Sequence(crate::sequence::SequenceEditor),
}

impl AnimTab {
    pub fn open(p: &Project, k: AnimKind, name: &str) -> Result<AnimTab, Option<String>> {
        let path = p.motion.path(k, name).ok_or(None)?;
        Ok(match k {
            AnimKind::Path => AnimTab::Path(crate::path::PathEditor::open(name, &path).map_err(Some)?),
            AnimKind::Timeline => AnimTab::Timeline(crate::timeline::TimelineEditor::open(name, &path).map_err(Some)?),
            AnimKind::Curve => AnimTab::Curve(crate::curve::CurveEditor::open(name, &path).map_err(Some)?),
            AnimKind::Sequence => AnimTab::Sequence(crate::sequence::SequenceEditor::open(name, &path).map_err(Some)?),
        })
    }
    pub fn kind(&self) -> AnimKind {
        match self {
            AnimTab::Path(_) => AnimKind::Path,
            AnimTab::Timeline(_) => AnimKind::Timeline,
            AnimTab::Curve(_) => AnimKind::Curve,
            AnimTab::Sequence(_) => AnimKind::Sequence,
        }
    }
    pub fn name(&self) -> &str {
        match self {
            AnimTab::Path(e) => &e.name,
            AnimTab::Timeline(e) => &e.name,
            AnimTab::Curve(e) => &e.name,
            AnimTab::Sequence(e) => &e.name,
        }
    }
    pub fn title(&self) -> String {
        format!("{} {}: {}", self.kind().icon(), self.kind().label(), self.name())
    }
    pub fn dirty(&self) -> bool {
        match self {
            AnimTab::Path(e) => e.dirty(),
            AnimTab::Timeline(e) => e.dirty(),
            AnimTab::Curve(e) => e.dirty(),
            AnimTab::Sequence(e) => e.dirty(),
        }
    }
    pub fn save(&mut self, cx: &mut Cx) -> Result<(), String> {
        match self {
            AnimTab::Path(e) => e.save(cx),
            AnimTab::Timeline(e) => e.save(cx),
            AnimTab::Curve(e) => e.save(cx),
            AnimTab::Sequence(e) => e.save(cx),
        }
    }
    pub fn undo(&mut self) {
        match self {
            AnimTab::Path(e) => e.hist.undo(&mut e.data),
            AnimTab::Timeline(e) => e.hist.undo(&mut e.data),
            AnimTab::Curve(e) => e.hist.undo(&mut e.data),
            AnimTab::Sequence(e) => e.hist.undo(&mut e.data),
        }
    }
    pub fn redo(&mut self) {
        match self {
            AnimTab::Path(e) => e.hist.redo(&mut e.data),
            AnimTab::Timeline(e) => e.hist.redo(&mut e.data),
            AnimTab::Curve(e) => e.hist.redo(&mut e.data),
            AnimTab::Sequence(e) => e.hist.redo(&mut e.data),
        }
    }
    pub fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        match self {
            AnimTab::Path(e) => e.ui(ui, cx),
            AnimTab::Timeline(e) => e.ui(ui, cx),
            AnimTab::Curve(e) => e.ui(ui, cx),
            AnimTab::Sequence(e) => e.ui(ui, cx),
        }
    }
    /// `--select` (screenshots): show something selected, mid-animation.
    pub fn auto_select(&mut self) {
        match self {
            AnimTab::Path(e) => e.sel = Some(1).filter(|&i| i < e.data.points.len()),
            AnimTab::Timeline(e) => e.sel = e.data.moments.len().saturating_sub(1),
            AnimTab::Curve(e) => e.sel = Some(1).filter(|&i| e.data.channels.first().is_some_and(|c| i < c.points.len())),
            AnimTab::Sequence(e) => {
                e.head = (e.data.length * 0.45).round();
                e.track = Some(1).filter(|&t| t < e.data.tracks.len()).or(Some(0).filter(|_| !e.data.tracks.is_empty()));
                e.key = Some(1).filter(|&k| e.track.is_some_and(|t| k < e.data.tracks[t].keys.len()));
            }
        }
    }
    /// Default window size in the workspace.
    pub fn window_size(k: AnimKind) -> Vec2 {
        match k {
            AnimKind::Path => Vec2::new(900.0, 580.0),
            AnimKind::Timeline => Vec2::new(820.0, 520.0),
            AnimKind::Curve => Vec2::new(860.0, 540.0),
            AnimKind::Sequence => Vec2::new(1000.0, 660.0),
        }
    }
}

/// Writes a file and remembers its contents as saved.
pub fn write_file(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

// ------------------------------------------------------------------ undo

/// Undo/redo for an editor whose whole state is one value: every frame the editor calls
/// [`History::track`]; a change (finished when no drag is in progress) becomes one step.
pub struct History<T> {
    last: T,
    undo: Vec<T>,
    redo: Vec<T>,
}

impl<T: Clone + PartialEq> History<T> {
    pub fn new(v: &T) -> Self {
        History { last: v.clone(), undo: vec![], redo: vec![] }
    }
    pub fn track(&mut self, cur: &T, busy: bool) {
        if !busy && *cur != self.last {
            self.undo.push(std::mem::replace(&mut self.last, cur.clone()));
            self.redo.clear();
            if self.undo.len() > 200 {
                self.undo.remove(0);
            }
        }
    }
    pub fn undo(&mut self, cur: &mut T) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(cur, prev.clone()));
            self.last = prev;
        }
    }
    pub fn redo(&mut self, cur: &mut T) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(cur, next.clone()));
            self.last = next;
        }
    }
    #[cfg(test)]
    pub fn steps(&self) -> usize {
        self.undo.len()
    }
}

// ------------------------------------------------------------------ pan / zoom view

/// A pannable, zoomable view of world coordinates (middle drag or Space+drag pans,
/// the wheel zooms around the pointer).
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub pan: Vec2,
    pub zoom: f32,
    /// Fitted to the content; set to false to fit again ("encajar").
    pub fitted: bool,
    pub rect: Rect,
    /// The user zoomed or panned: stop re-fitting when the window is resized.
    touched: bool,
}

impl Default for View {
    fn default() -> Self {
        View { pan: Vec2::ZERO, zoom: 1.0, fitted: false, rect: Rect::NOTHING, touched: false }
    }
}

impl View {
    /// Fits `world` (a rectangle in world units) the first time, then handles pan and zoom.
    pub fn update(&mut self, ui: &egui::Ui, resp: &Response, world: Rect) {
        let area = resp.rect;
        if !self.touched && (area.size() - self.rect.size()).length() > 1.0 {
            self.fitted = false; // the window was resized: fit again
        }
        if !self.fitted {
            self.touched = false;
        }
        self.rect = area;
        if !self.fitted && area.width() > 50.0 && world.width() > 0.0 && world.height() > 0.0 {
            self.zoom = ((area.width() - 40.0) / world.width()).min((area.height() - 40.0) / world.height()).clamp(0.05, 16.0);
            self.pan = area.center() - area.min - world.center().to_vec2() * self.zoom;
            self.fitted = true;
        }
        if let Some(hp) = resp.hover_pos() {
            let factor = ui.input(|i| crate::zoom::factor(i.smooth_scroll_delta.y, i.zoom_delta()));
            if factor != 1.0 {
                let w = self.to_w(hp);
                self.zoom = (self.zoom * factor).clamp(0.05, 16.0);
                self.pan = hp - area.min - w.to_vec2() * self.zoom;
                self.touched = true;
            }
        }
        let space = ui.input(|i| i.key_down(egui::Key::Space));
        if resp.dragged_by(egui::PointerButton::Middle) || (space && resp.dragged_by(egui::PointerButton::Primary)) {
            self.pan += resp.drag_delta();
            self.touched = true;
        }
    }
    pub fn to_s(&self, w: Pos2) -> Pos2 {
        self.rect.min + self.pan + w.to_vec2() * self.zoom
    }
    pub fn to_w(&self, s: Pos2) -> Pos2 {
        Pos2::ZERO + (s - self.rect.min - self.pan) / self.zoom
    }
    /// Space is held: primary drags pan instead of editing.
    pub fn panning(ui: &egui::Ui) -> bool {
        ui.input(|i| i.key_down(egui::Key::Space))
    }
}

/// Grid lines every `step` world units inside the visible part of `area`.
pub fn grid(painter: &egui::Painter, view: &View, step: f32, color: Color32) {
    if step * view.zoom < 6.0 {
        return;
    }
    let a = view.to_w(view.rect.min);
    let b = view.to_w(view.rect.max);
    let st = Stroke::new(1.0, color);
    let mut x = (a.x / step).floor() * step;
    while x <= b.x {
        painter.line_segment([view.to_s(Pos2::new(x, a.y)), view.to_s(Pos2::new(x, b.y))], st);
        x += step;
    }
    let mut y = (a.y / step).floor() * step;
    while y <= b.y {
        painter.line_segment([view.to_s(Pos2::new(a.x, y)), view.to_s(Pos2::new(b.x, y))], st);
        y += step;
    }
}

// ------------------------------------------------------------------ room backdrop

/// Size of a room (explicit, or the screen) and its parsed file.
pub fn room_toml(p: &Project, room: &str) -> Option<modengine::project::RoomToml> {
    let r = p.rooms.iter().find(|r| r.name == room)?;
    toml::from_str(&std::fs::read_to_string(&r.path).ok()?).ok()
}

/// Draws a room as a dimmed backdrop: background colours, sprite backgrounds, tile maps
/// and instances. Returns the room size.
pub fn draw_room(painter: &egui::Painter, cx: &mut Cx, ctx: &egui::Context, view: &View, room: &str, dim: f32) -> Option<Vec2> {
    let p: &Project = cx.project;
    let rt = room_toml(p, room)?;
    let mut w = rt.width.unwrap_or(p.game.display.width) as f32;
    let mut h = rt.height.unwrap_or(p.game.display.height) as f32;
    let alpha = |a: f64| Color32::from_white_alpha((a.clamp(0.0, 1.0) as f32 * dim * 255.0) as u8);
    let bg = rt.background.as_deref().and_then(crate::tomlw::parse_hex).map(crate::tex::rgb).unwrap_or(Color32::BLACK);
    let mut layers: Vec<&modengine::project::LayerToml> = rt.layers.iter().filter(|l| l.visible).collect();
    layers.sort_by(|a, b| b.depth.partial_cmp(&a.depth).unwrap_or(std::cmp::Ordering::Equal));
    // maps may make the room bigger than its explicit size
    for l in &layers {
        if let Some(m) = &l.map {
            let [cw, ch] = l.cell.unwrap_or([16, 16]);
            let rows: Vec<&str> = m.lines().filter(|r| !r.trim().is_empty()).collect();
            let cols = rows.iter().map(|r| r.trim_end().chars().count()).max().unwrap_or(0);
            if rt.width.is_none() {
                w = w.max(cols as f32 * cw as f32 + l.x as f32);
            }
            if rt.height.is_none() {
                h = h.max(rows.len() as f32 * ch as f32 + l.y as f32);
            }
        }
    }
    let room_rect = Rect::from_min_max(view.to_s(Pos2::ZERO), view.to_s(Pos2::new(w, h)));
    painter.rect_filled(room_rect, 0.0, bg.gamma_multiply(dim));
    let z = view.zoom;
    for l in layers {
        let tint = alpha(l.alpha);
        let off = Vec2::new(l.x as f32, l.y as f32);
        match l.kind.as_str() {
            "background" => {
                if let Some(c) = l.color.as_deref().and_then(crate::tomlw::parse_hex) {
                    painter.rect_filled(room_rect, 0.0, crate::tex::rgb(c).gamma_multiply(dim * l.alpha as f32));
                }
                if let Some(t) = l.sprite.as_deref().and_then(|s| cx.tex.get(ctx, p, s)) {
                    if l.stretch {
                        painter.image(t.id, room_rect, t.uv(0), tint);
                    } else {
                        let fs = t.frame_size();
                        let nx = if l.tile_x { (w / fs.x).ceil() as i32 } else { 1 };
                        let ny = if l.tile_y { (h / fs.y).ceil() as i32 } else { 1 };
                        for iy in 0..ny {
                            for ix in 0..nx {
                                let at = Pos2::ZERO + off + Vec2::new(ix as f32 * fs.x, iy as f32 * fs.y);
                                painter.image(t.id, Rect::from_min_size(view.to_s(at), fs * z), t.uv(0), tint);
                            }
                        }
                    }
                }
            }
            "tiles" => {
                let (Some(ts), Some(map)) = (l.tileset.as_deref(), l.map.as_deref()) else { continue };
                let Some(t) = cx.tex.get(ctx, p, ts) else { continue };
                let [cw, ch] = l.cell.unwrap_or([t.spr.fw, t.spr.fh]);
                let (sx, sy) = l.tile_scale().unwrap_or((1.0, 1.0));
                let (cw, ch) = ((cw as f64 * sx).round() as u32, (ch as f64 * sy).round() as u32);
                for (r, row) in map.lines().filter(|r| !r.trim().is_empty()).enumerate() {
                    for (c, chr) in row.chars().enumerate() {
                        let Some(tile) = l.legend.get(&chr.to_string()).and_then(|v| v.as_integer()) else { continue };
                        if tile >= 0 && (tile as usize) < t.spr.frames.len() {
                            let at = Pos2::ZERO + off + Vec2::new(c as f32 * cw as f32, r as f32 * ch as f32);
                            painter.image(t.id, Rect::from_min_size(view.to_s(at), Vec2::new(cw as f32, ch as f32) * z), t.uv(tile as usize), tint);
                        }
                    }
                }
            }
            _ => {
                if let Some(map) = l.map.as_deref() {
                    let [cw, ch] = l.cell.unwrap_or([16, 16]);
                    for (r, row) in map.lines().filter(|r| !r.trim().is_empty()).enumerate() {
                        for (c, chr) in row.chars().enumerate() {
                            let names: Vec<String> = match l.legend.get(&chr.to_string()) {
                                Some(toml::Value::String(s)) => vec![s.clone()],
                                Some(toml::Value::Array(a)) => a.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
                                _ => continue,
                            };
                            let cell = Rect::from_min_size(Pos2::ZERO + off + Vec2::new(c as f32 * cw as f32, r as f32 * ch as f32), Vec2::new(cw as f32, ch as f32));
                            for n in names {
                                if let Some(t) = p.object_sprite(&n).and_then(|s| cx.tex.get(ctx, p, &s)) {
                                    let fs = t.frame_size();
                                    let min = cell.center() - fs / 2.0;
                                    painter.image(t.id, Rect::from_min_size(view.to_s(min), fs * z), t.uv(0), tint);
                                }
                            }
                        }
                    }
                }
                for inst in &l.instances {
                    if let Some(t) = p.object_sprite(&inst.object).and_then(|s| cx.tex.get(ctx, p, &s)) {
                        t.draw(painter, 0, view.to_s(Pos2::new(inst.x as f32, inst.y as f32)), z, tint);
                    }
                }
            }
        }
    }
    painter.rect_stroke(room_rect, 0.0, Stroke::new(1.0, Color32::from_white_alpha(60)), egui::StrokeKind::Outside);
    Some(Vec2::new(w, h))
}

/// A combo box listing `options` (plus "—" for none when `none` is set).
pub fn combo(ui: &mut egui::Ui, id: impl std::hash::Hash + std::fmt::Debug, value: &mut Option<String>, options: &[String], none: &str) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(id).selected_text(value.clone().unwrap_or_else(|| none.to_string())).show_ui(ui, |ui| {
        if !none.is_empty() && ui.selectable_label(value.is_none(), none).clicked() {
            *value = None;
            changed = true;
        }
        for o in options {
            if ui.selectable_label(value.as_deref() == Some(o.as_str()), o).clicked() {
                *value = Some(o.clone());
                changed = true;
            }
        }
    });
    changed
}

/// Animation clock for previews: seconds since the editor started previewing.
pub fn now(ui: &egui::Ui) -> f64 {
    ui.input(|i| i.time)
}

/// Allocates a canvas filling the available space.
pub fn canvas(ui: &mut egui::Ui) -> (Response, egui::Painter) {
    let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    let painter = painter.with_clip_rect(resp.rect);
    (resp, painter)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn history_groups_changes() {
        let mut v = 1;
        let mut h = History::new(&v);
        v = 2;
        h.track(&v, true);
        v = 3;
        h.track(&v, true);
        h.track(&v, false);
        assert_eq!(h.steps(), 1, "a drag is one step");
        h.undo(&mut v);
        assert_eq!(v, 1);
        h.redo(&mut v);
        assert_eq!(v, 3);
    }

    #[test]
    fn templates_load_in_the_engine() {
        let dir = crate::room::tests::temp_project("motion_templates");
        let mut p = Project::open(&dir).unwrap();
        for k in AnimKind::ALL {
            create(&mut p, k, &format!("new_{}", k.key())).unwrap();
            assert!(p.motion.path(k, &format!("new_{}", k.key())).is_some());
        }
        assert!(create(&mut p, AnimKind::Path, "new_path").is_err());
        let e = modengine::Engine::load(&dir).expect("new motion assets must load");
        assert_eq!(e.paths.len(), 1);
        assert_eq!(e.timelines.len(), 1);
        assert_eq!(e.curves.len(), 1);
        assert_eq!(e.sequences.len(), 1);
        delete(&mut p, AnimKind::Curve, "new_curve").unwrap();
        assert!(p.motion.path(AnimKind::Curve, "new_curve").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
