//! Tile set editor (`tilesets/<name>.toml`), like GameMaker's: the image with its tile grid
//! (size, offset, separation), tile indices, animated tiles with a live preview and 16-tile
//! auto tiles. Rooms paint tile layers with these tiles (room.rs).

use crate::app::{Cx, Open};
use crate::project::Project;
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, TextureHandle, TextureOptions, Vec2};
use modengine::tileset::{from_toml, AutoTileToml, TileAnimToml, TileSetAsset, TileSetToml};
use std::path::{Path, PathBuf};

/// Creates `tilesets/<name>.toml` cut from `sprite`.
pub fn create(p: &mut Project, name: &str, sprite: &str) -> Result<Open, String> {
    let spr = p.sprite(sprite).ok_or_else(|| if sprite.is_empty() { "elige el sprite con la imagen de los tiles (impórtalo antes si hace falta)".to_string() } else { format!("el sprite '{sprite}' no existe") })?;
    let size = if spr.frames.len() > 1 { [spr.fw, spr.fh] } else { [16.min(spr.fw), 16.min(spr.fh)] };
    let t = TileSetToml { sprite: Some(sprite.into()), tile_width: Some(size[0]), tile_height: Some(size[1]), ..Default::default() };
    let path = p.dir.join("tilesets").join(format!("{name}.toml"));
    if path.exists() {
        return Err(format!("{} ya existe", path.display()));
    }
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&path, modengine::tileset::write_toml(&t)).map_err(|e| e.to_string())?;
    p.reload_lists();
    Ok(Open::TileSet(name.into()))
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Tiles,
    Animations,
    AutoTiles,
}

pub struct TileSetEditor {
    pub name: String,
    path: PathBuf,
    t: TileSetToml,
    saved: TileSetToml,
    pub dirty: bool,
    undo: Vec<TileSetToml>,
    redo: Vec<TileSetToml>,
    mode: Mode,
    /// Selected tiles (a rectangle in the grid: first..=last, row-major).
    sel: Option<(u32, u32)>,
    drag_from: Option<u32>,
    anim: usize,
    auto: usize,
    /// Auto tile slot (0..16) that the next tile click fills.
    slot: Option<usize>,
    zoom: f32,
    /// Tiles cut with the current settings (or why it failed).
    cut: Option<(String, Result<TileSetAsset, String>)>,
    tex: Option<(String, TextureHandle)>,
    start: std::time::Instant,
}

impl TileSetEditor {
    pub fn open(name: &str, path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let t: TileSetToml = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(TileSetEditor {
            name: name.into(),
            path: path.into(),
            saved: t.clone(),
            t,
            dirty: false,
            undo: vec![],
            redo: vec![],
            mode: Mode::Tiles,
            sel: Some((0, 0)),
            drag_from: None,
            anim: 0,
            auto: 0,
            slot: None,
            zoom: 0.0,
            cut: None,
            tex: None,
            start: std::time::Instant::now(),
        })
    }

    pub fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.t, prev));
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.t, next));
        }
    }

    pub fn save(&mut self, cx: &mut Cx) -> Result<(), String> {
        std::fs::write(&self.path, modengine::tileset::write_toml(&self.t)).map_err(|e| format!("{}: {e}", self.path.display()))?;
        self.saved = self.t.clone();
        self.dirty = false;
        cx.project.reload_tilesets();
        Ok(())
    }

    /// Selects the tiles of the auto tile slots / animation frames (used by `--select`).
    pub fn auto_select(&mut self) {
        if !self.t.autotiles.is_empty() {
            self.mode = Mode::AutoTiles;
        } else if !self.t.animations.is_empty() {
            self.mode = Mode::Animations;
        }
    }

    fn recut(&mut self, ctx: &egui::Context, p: &Project) {
        let key = format!("{:?}", self.t);
        if self.cut.as_ref().is_none_or(|(k, _)| *k != key) {
            let folder = self.path.parent().map(Path::to_path_buf).unwrap_or_default();
            self.cut = Some((key, from_toml(&self.name, &self.t, &folder, &|n| p.sprite(n))));
        }
        // the texture is the whole source image: only reloaded when the source changes
        let src = format!("{:?}{:?}", self.t.sprite, self.t.image);
        if let Some((_, Ok(ts))) = &self.cut {
            if self.tex.as_ref().is_none_or(|(k, _)| *k != src) {
                let img = egui::ColorImage::from_rgba_unmultiplied([ts.sprite.sheet_w as usize, ts.sprite.sheet_h as usize], &ts.sprite.rgba);
                self.tex = Some((src, ctx.load_texture(format!("tileset_ed:{}", self.name), img, TextureOptions::NEAREST)));
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        self.recut(ui.ctx(), cx.project);
        let before = self.t.clone();
        egui::Panel::left(ui.scope_id().with("ts_props")).resizable(true).default_size(290.0).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.props(ui, cx.project));
        });
        egui::CentralPanel::default().show(ui, |ui| self.canvas(ui));
        if self.t != before {
            self.undo.push(before);
            self.redo.clear();
        }
        self.dirty = self.t != self.saved;
    }

    fn props(&mut self, ui: &mut egui::Ui, p: &Project) {
        ui.heading(&self.name);
        egui::Grid::new(ui.scope_id().with("ts_grid")).num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            ui.label("Imagen");
            let cur = self.t.sprite.clone().or(self.t.image.clone()).unwrap_or_else(|| "(elegir)".into());
            egui::ComboBox::from_id_salt(ui.scope_id().with("ts_sprite")).selected_text(cur).show_ui(ui, |ui| {
                for s in p.sprite_names() {
                    if ui.selectable_label(self.t.sprite.as_deref() == Some(s.as_str()), &s).clicked() {
                        self.t.sprite = Some(s);
                        self.t.image = None;
                    }
                }
            });
            ui.end_row();
            let mut tw = self.t.tile_width.unwrap_or(16);
            let mut th = self.t.tile_height.unwrap_or(16);
            ui.label("Tamaño de tile");
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut tw).range(1..=1024));
                ui.label("×");
                ui.add(egui::DragValue::new(&mut th).range(1..=1024));
            });
            ui.end_row();
            if Some(tw) != self.t.tile_width || Some(th) != self.t.tile_height {
                self.t.tile_width = Some(tw);
                self.t.tile_height = Some(th);
            }
            ui.label("Desplazamiento");
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut self.t.offset[0]).range(0..=1024));
                ui.add(egui::DragValue::new(&mut self.t.offset[1]).range(0..=1024));
            });
            ui.end_row();
            ui.label("Separación");
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut self.t.separation[0]).range(0..=256));
                ui.add(egui::DragValue::new(&mut self.t.separation[1]).range(0..=256));
            });
            ui.end_row();
            ui.label("Velocidad anim.");
            let mut fps = self.t.fps.unwrap_or(4.0);
            if ui.add(egui::DragValue::new(&mut fps).range(0.0..=60.0).speed(0.1).suffix(" fps")).changed() {
                self.t.fps = Some(fps);
            }
            ui.end_row();
        });
        match &self.cut {
            Some((_, Ok(ts))) => {
                ui.label(RichText::new(format!("{} tiles · {} columnas × {} filas", ts.count(), ts.columns, ts.rows)).weak());
            }
            Some((_, Err(e))) => {
                ui.colored_label(crate::theme::colors(ui.ctx()).error, e);
            }
            None => {}
        }
        ui.separator();
        ui.horizontal(|ui| {
            for (m, label) in [(Mode::Tiles, "Tiles"), (Mode::Animations, "Animaciones"), (Mode::AutoTiles, "Auto tiles")] {
                ui.selectable_value(&mut self.mode, m, label);
            }
        });
        match self.mode {
            Mode::Tiles => {
                ui.label(RichText::new("Clic: elegir tile · arrastrar: elegir varios. En una room, una capa de tiles con este tile set pinta con ellos.").weak());
                if let Some((a, b)) = self.sel {
                    ui.label(if a == b { format!("tile {a}") } else { format!("tiles {a}…{b}") });
                }
            }
            Mode::Animations => self.anim_props(ui),
            Mode::AutoTiles => self.auto_props(ui),
        }
    }

    fn selected_tiles(&self) -> Vec<u32> {
        let (Some((a, b)), Some((_, Ok(ts)))) = (self.sel, &self.cut) else { return vec![] };
        let cols = ts.columns.max(1);
        let (c0, r0, c1, r1) = (a % cols, a / cols, b % cols, b / cols);
        let mut out = vec![];
        for r in r0.min(r1)..=r0.max(r1) {
            for c in c0.min(c1)..=c0.max(c1) {
                out.push(r * cols + c);
            }
        }
        out
    }

    fn anim_props(&mut self, ui: &mut egui::Ui) {
        let mut remove = None;
        for (i, a) in self.t.animations.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                if ui.selectable_label(self.anim == i, "▶").clicked() {
                    self.anim = i;
                }
                ui.add(egui::TextEdit::singleline(&mut a.name).desired_width(90.0));
                let frames: Vec<String> = a.frames.iter().map(|f| f.to_string()).collect();
                ui.label(RichText::new(frames.join(" ")).monospace().weak());
                if ui.small_button("🗑").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            self.t.animations.remove(i);
        }
        let sel = self.selected_tiles();
        ui.horizontal(|ui| {
            if ui.add_enabled(sel.len() > 1, egui::Button::new("+ animación con la selección")).on_disabled_hover_text("arrastra sobre varios tiles: el primero es el que se pinta y el resto sus fotogramas").clicked() {
                self.t.animations.push(TileAnimToml { name: format!("anim{}", self.t.animations.len()), frames: sel.clone() });
                self.anim = self.t.animations.len() - 1;
            }
            if let Some(a) = self.t.animations.get_mut(self.anim) {
                if ui.add_enabled(!sel.is_empty(), egui::Button::new("+ fotogramas")).clicked() {
                    a.frames.extend(sel.iter().copied());
                }
            }
        });
        ui.label(RichText::new("El primer fotograma es el tile que se pinta en las rooms; el juego lo anima solo.").weak());
    }

    fn auto_props(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (i, a) in self.t.autotiles.iter().enumerate() {
                if ui.selectable_label(self.auto == i, &a.name).clicked() {
                    self.auto = i;
                }
            }
            if ui.button("+").on_hover_text("nuevo auto tile").clicked() {
                // the 16 tiles from the selection, or empty slots
                let sel = self.selected_tiles();
                let tiles = if sel.len() == 16 { sel.iter().map(|t| *t as i32).collect() } else { vec![-1; 16] };
                self.t.autotiles.push(AutoTileToml { name: format!("auto{}", self.t.autotiles.len()), tiles });
                self.auto = self.t.autotiles.len() - 1;
            }
        });
        let tex = self.tex.as_ref().map(|t| t.1.id());
        let ts = match &self.cut {
            Some((_, Ok(ts))) => Some(ts),
            _ => None,
        };
        let Some(a) = self.t.autotiles.get_mut(self.auto) else {
            ui.label(RichText::new("Un auto tile elige solo el tile de cada celda según sus vecinos (arriba, derecha, abajo, izquierda). Selecciona 16 tiles en orden y pulsa +.").weak());
            return;
        };
        let mut remove = false;
        ui.horizontal(|ui| {
            ui.label("Nombre");
            ui.add(egui::TextEdit::singleline(&mut a.name).desired_width(120.0));
            remove = ui.small_button("🗑").clicked();
        });
        ui.label(RichText::new("Clic en una casilla y luego en un tile. Las flechas indican con qué vecinos conecta.").weak());
        let cell = 44.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(cell * 4.0 + 6.0), Sense::hover());
        let pal = crate::theme::colors(ui.ctx());
        for m in 0..16usize {
            let r = Rect::from_min_size(rect.min + Vec2::new((m % 4) as f32 * (cell + 2.0), (m / 4) as f32 * (cell + 2.0)), Vec2::splat(cell));
            let resp = ui.interact(r, ui.scope_id().with(("slot", m)), Sense::click());
            crate::tex::checkerboard(ui.painter(), r, 6.0);
            if let (Some(tex), Some(ts), Some(&t)) = (tex, ts, a.tiles.get(m)) {
                if t >= 0 && (t as usize) < ts.sprite.frames.len() {
                    ui.painter().image(tex, r.shrink(3.0), frame_uv(&ts.sprite, t as usize), Color32::WHITE);
                }
            }
            // arrows: which neighbours this slot connects to
            let c = r.center();
            let k = cell * 0.42;
            for (bit, d, s) in [(1, Vec2::new(0.0, -k), "▴"), (2, Vec2::new(k, 0.0), "▸"), (4, Vec2::new(0.0, k), "▾"), (8, Vec2::new(-k, 0.0), "◂")] {
                if m & bit != 0 {
                    ui.painter().text(c + d, egui::Align2::CENTER_CENTER, s, egui::FontId::proportional(11.0), pal.selection);
                }
            }
            if self.slot == Some(m) {
                ui.painter().rect_stroke(r, 2.0, Stroke::new(2.0, pal.selection), StrokeKind::Inside);
            }
            if resp.on_hover_text(format!("vecinos = {m}")).clicked() {
                self.slot = Some(m);
            }
        }
        if remove {
            self.t.autotiles.remove(self.auto);
            self.auto = 0;
        }
    }

    fn canvas(&mut self, ui: &mut egui::Ui) {
        let Some((_, Ok(ts))) = &self.cut else {
            ui.label("Elige una imagen y un tamaño de tile válidos.");
            return;
        };
        let Some((_, tex)) = &self.tex else { return };
        let (sw, sh) = (ts.sprite.sheet_w as f32, ts.sprite.sheet_h as f32);
        let avail = ui.available_size();
        if self.zoom <= 0.0 {
            self.zoom = ((avail.x - 20.0) / sw).min((avail.y - 120.0) / sh).clamp(0.5, 8.0).floor().max(1.0);
        }
        ui.horizontal(|ui| {
            ui.label("Zoom");
            ui.add(egui::Slider::new(&mut self.zoom, 0.5..=8.0).step_by(0.5));
            if let Some(a) = self.t.animations.get(self.anim).filter(|_| self.mode == Mode::Animations) {
                // live preview of the animation, at the tile set's speed
                let t = self.start.elapsed().as_secs_f64();
                let f = ts.anim_of.get(a.frames.first().unwrap_or(&0)).map(|_| ts.frame_at(a.frames[0], t)).unwrap_or(0);
                ui.separator();
                ui.label(format!("{}:", a.name));
                let (r, _) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::hover());
                crate::tex::checkerboard(ui.painter(), r, 5.0);
                ui.painter().image(tex.id(), r, frame_uv(&ts.sprite, f as usize), Color32::WHITE);
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
            }
        });
        let z = self.zoom;
        let mut clicked: Option<u32> = None;
        let mut hovered: Option<u32> = None;
        let mut released = false;
        egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(sw, sh) * z, Sense::click_and_drag());
            let painter = ui.painter_at(rect);
            crate::tex::checkerboard(&painter, rect, 8.0);
            painter.image(tex.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            let pal = crate::theme::colors(ui.ctx());
            let grid = Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 90));
            let tile_rect = |i: usize| {
                let f = ts.sprite.frames[i];
                Rect::from_min_size(rect.min + Vec2::new(f[0] as f32, f[1] as f32) * z, Vec2::new(f[2] as f32, f[3] as f32) * z)
            };
            for i in 0..ts.sprite.frames.len() {
                painter.rect_stroke(tile_rect(i), 0.0, grid, StrokeKind::Inside);
            }
            // tiles that are part of the current animation / auto tile
            let marked: Vec<u32> = match self.mode {
                Mode::Animations => self.t.animations.get(self.anim).map(|a| a.frames.clone()).unwrap_or_default(),
                Mode::AutoTiles => self.t.autotiles.get(self.auto).map(|a| a.tiles.iter().filter(|t| **t >= 0).map(|t| *t as u32).collect()).unwrap_or_default(),
                Mode::Tiles => vec![],
            };
            for (k, t) in marked.iter().enumerate() {
                if (*t as usize) < ts.sprite.frames.len() {
                    let r = tile_rect(*t as usize);
                    painter.rect_filled(r, 0.0, pal.selection.gamma_multiply(0.25));
                    if self.mode == Mode::Animations {
                        painter.text(r.left_top() + Vec2::splat(2.0), egui::Align2::LEFT_TOP, k.to_string(), egui::FontId::proportional(10.0), Color32::WHITE);
                    }
                }
            }
            for t in self.selected_tiles() {
                if (t as usize) < ts.sprite.frames.len() {
                    painter.rect_stroke(tile_rect(t as usize), 0.0, Stroke::new(2.0, pal.selection), StrokeKind::Inside);
                }
            }
            if let Some(pos) = resp.hover_pos().or(resp.interact_pointer_pos()) {
                hovered = (0..ts.sprite.frames.len()).find(|&i| tile_rect(i).contains(pos)).map(|i| i as u32);
                if let Some(h) = hovered {
                    painter.rect_stroke(tile_rect(h as usize), 0.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Outside);
                }
            }
            if resp.drag_started() || resp.clicked() {
                clicked = hovered;
            }
            released = resp.drag_stopped() || resp.clicked();
        });
        if let Some(h) = hovered {
            ui.label(RichText::new(format!("tile {h}")).weak());
        }
        if let Some(c) = clicked {
            if let (Mode::AutoTiles, Some(slot)) = (self.mode, self.slot) {
                if let Some(a) = self.t.autotiles.get_mut(self.auto) {
                    a.tiles[slot] = c as i32;
                    self.slot = Some((slot + 1) % 16);
                }
            }
            self.drag_from = Some(c);
            self.sel = Some((c, c));
        }
        if let (Some(from), Some(h)) = (self.drag_from, hovered) {
            self.sel = Some((from, h));
        }
        if released {
            self.drag_from = None;
        }
    }
}

// ------------------------------------------------------------------ painting rooms

use modengine::tileset::{autotile_apply, index, sample, EMPTY, FLIP, MIRROR, ROTATE};

/// What a room's tile layer paints with: a stamp of tiles (from a drag in the palette) with
/// mirror/flip/rotate bits, or an auto tile of the tile set.
#[derive(Clone, PartialEq, Debug)]
pub struct TileBrush {
    /// Rows of tile indices as picked in the palette.
    pub tiles: Vec<Vec<i32>>,
    /// MIRROR / FLIP / ROTATE bits applied to the whole stamp.
    pub flags: i32,
    /// Paint with this auto tile of the tile set instead.
    pub auto: Option<usize>,
}

impl Default for TileBrush {
    fn default() -> Self {
        TileBrush::single(0)
    }
}

impl TileBrush {
    pub fn single(t: i32) -> Self {
        TileBrush { tiles: vec![vec![t]], flags: 0, auto: None }
    }

    /// Size of the stamp once transformed (columns, rows).
    pub fn size(&self) -> (i32, i32) {
        let h = self.tiles.len() as i32;
        let w = self.tiles.iter().map(|r| r.len()).max().unwrap_or(0) as i32;
        if self.flags & ROTATE != 0 {
            (h, w)
        } else {
            (w, h)
        }
    }

    /// Tile data at column `c`, row `r` of the transformed stamp (the stamp repeats).
    pub fn at(&self, c: i32, r: i32) -> i32 {
        let (w, h) = self.size();
        if w == 0 || h == 0 {
            return EMPTY;
        }
        let (c, r) = (c.rem_euclid(w), r.rem_euclid(h));
        // where (c, r) of the output comes from in the picked stamp (same mapping as a tile)
        let (u, v) = sample(self.flags, (c as f32 + 0.5) / w as f32, (r as f32 + 0.5) / h as f32);
        let (sw, sh) = (self.tiles.iter().map(|x| x.len()).max().unwrap_or(0) as f32, self.tiles.len() as f32);
        let (sc, sr) = ((u * sw).floor() as usize, (v * sh).floor() as usize);
        match self.tiles.get(sr).and_then(|row| row.get(sc)) {
            Some(&t) if t >= 0 => t | self.flags,
            _ => EMPTY,
        }
    }

    pub fn toggle(&mut self, bit: i32) {
        self.flags ^= bit;
    }
}

fn put(grid: &mut Vec<Vec<i32>>, c: i32, r: i32, v: i32) -> bool {
    if c < 0 || r < 0 || c > 4000 || r > 4000 {
        return false;
    }
    let (c, r) = (c as usize, r as usize);
    if grid.len() <= r {
        grid.resize(r + 1, vec![]);
    }
    if grid[r].len() <= c {
        grid[r].resize(c + 1, EMPTY);
    }
    let changed = grid[r][c] != v;
    grid[r][c] = v;
    changed
}

/// Paints the brush with its top-left at cell (c, r). `autos` are the tile set's auto tiles.
pub fn stamp(grid: &mut Vec<Vec<i32>>, c: i32, r: i32, b: &TileBrush, autos: &[AutoTileToml]) {
    if let Some(a) = b.auto.and_then(|a| autos.get(a)) {
        autotile_apply(grid, &a.tiles, c, r, true);
        return;
    }
    let (w, h) = b.size();
    for dr in 0..h {
        for dc in 0..w {
            let t = b.at(dc, dr);
            if t >= 0 {
                put(grid, c + dc, r + dr, t);
            }
        }
    }
}

/// Fills cells c0..=c1 × r0..=r1 repeating the brush (or painting the auto tile).
pub fn fill_rect(grid: &mut Vec<Vec<i32>>, (c0, r0): (i32, i32), (c1, r1): (i32, i32), b: &TileBrush, autos: &[AutoTileToml]) {
    for r in r0.min(r1)..=r0.max(r1) {
        for c in c0.min(c1)..=c0.max(c1) {
            match b.auto.and_then(|a| autos.get(a)) {
                Some(a) => autotile_apply(grid, &a.tiles, c, r, true),
                None => {
                    put(grid, c, r, b.at(c - c0.min(c1), r - r0.min(r1)));
                }
            }
        }
    }
}

/// Erases a cell (an auto tile brush also fixes the neighbours).
pub fn erase(grid: &mut Vec<Vec<i32>>, c: i32, r: i32, b: &TileBrush, autos: &[AutoTileToml]) {
    match b.auto.and_then(|a| autos.get(a)) {
        Some(a) => autotile_apply(grid, &a.tiles, c, r, false),
        None => {
            put(grid, c, r, EMPTY);
        }
    }
}

/// Draws tile data (index + mirror/flip/rotate) into `rect`, like the game does.
pub fn paint_tile(painter: &egui::Painter, tex: egui::TextureId, spr: &modengine::project::Sprite, data: i32, frame: usize, rect: Rect, tint: Color32) {
    if index(data) < 0 || spr.frames.is_empty() {
        return;
    }
    let uv = frame_uv(spr, frame);
    if data & (MIRROR | FLIP | ROTATE) == 0 {
        painter.image(tex, rect, uv, tint);
        return;
    }
    let mut mesh = egui::Mesh::with_texture(tex);
    let corners = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];
    let unit = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
    for (p, (u, v)) in corners.iter().zip(unit) {
        let (iu, iv) = sample(data, u, v);
        mesh.vertices.push(egui::epaint::Vertex { pos: *p, uv: Pos2::new(uv.min.x + iu * uv.width(), uv.min.y + iv * uv.height()), color: tint });
    }
    mesh.indices.extend([0, 1, 2, 0, 2, 3]);
    painter.add(egui::Shape::mesh(mesh));
}

/// The tile set palette of the room editor: the whole image with its grid, drag to pick a
/// stamp of several tiles, mirror/flip/rotate and the auto tiles. Returns true on change.
pub fn palette_ui(ui: &mut egui::Ui, tex: egui::TextureId, ts: &TileSetAsset, brush: &mut TileBrush, drag: &mut Option<u32>) -> bool {
    let before = brush.clone();
    let pal = crate::theme::colors(ui.ctx());
    ui.horizontal_wrapped(|ui| {
        for (bit, label, tip) in [(MIRROR, "↔", "espejo horizontal (X)"), (FLIP, "↕", "voltear vertical (Y)"), (ROTATE, "⟳", "girar 90° (Z)")] {
            if ui.selectable_label(brush.flags & bit != 0, label).on_hover_text(tip).clicked() {
                brush.toggle(bit);
            }
        }
        let (w, h) = brush.size();
        ui.label(RichText::new(if brush.auto.is_some() { "auto tile".to_string() } else { format!("pincel {w}×{h}") }).weak());
    });
    if !ts.autotiles.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.label("Auto:");
            for (i, a) in ts.autotiles.iter().enumerate() {
                if ui.selectable_label(brush.auto == Some(i), &a.name).on_hover_text("pinta eligiendo el tile según los vecinos").clicked() {
                    brush.auto = if brush.auto == Some(i) { None } else { Some(i) };
                }
            }
        });
    }
    let (sw, sh) = (ts.sprite.sheet_w as f32, ts.sprite.sheet_h as f32);
    let z = ((ui.available_width() - 8.0) / sw).clamp(0.5, 4.0);
    egui::ScrollArea::both().id_salt("ts_palette").auto_shrink([false, false]).show(ui, |ui| {
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(sw, sh) * z, Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        crate::tex::checkerboard(&painter, rect, 6.0);
        painter.image(tex, rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        let tile_rect = |i: usize| {
            let f = ts.sprite.frames[i];
            Rect::from_min_size(rect.min + Vec2::new(f[0] as f32, f[1] as f32) * z, Vec2::new(f[2] as f32, f[3] as f32) * z)
        };
        let hovered = resp.hover_pos().or(resp.interact_pointer_pos()).and_then(|p| (0..ts.sprite.frames.len()).find(|&i| tile_rect(i).contains(p))).map(|i| i as u32);
        if let Some(h) = hovered {
            painter.rect_stroke(tile_rect(h as usize), 0.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Inside);
            if resp.drag_started() || resp.clicked() {
                *drag = Some(h);
            }
            if let Some(from) = *drag {
                // the rectangle from where the drag started to here becomes the stamp
                let cols = ts.columns.max(1);
                let (c0, r0, c1, r1) = (from % cols, from / cols, h % cols, h / cols);
                brush.tiles = (r0.min(r1)..=r0.max(r1)).map(|r| (c0.min(c1)..=c0.max(c1)).map(|c| (r * cols + c) as i32).collect()).collect();
                brush.auto = None;
            }
        }
        if resp.drag_stopped() || resp.clicked() {
            *drag = None;
        }
        if brush.auto.is_none() {
            for row in &brush.tiles {
                for &t in row {
                    if t >= 0 && (t as usize) < ts.sprite.frames.len() {
                        painter.rect_stroke(tile_rect(t as usize), 0.0, Stroke::new(2.0, pal.selection), StrokeKind::Inside);
                    }
                }
            }
        }
        resp.on_hover_text(hovered.map(|h| format!("tile {h} · arrastra para un pincel de varios tiles")).unwrap_or_default());
    });
    *brush != before
}

/// UV rectangle of frame `i` of a sprite sheet.
pub fn frame_uv(spr: &modengine::project::Sprite, i: usize) -> Rect {
    let f = spr.frames[i % spr.frames.len().max(1)];
    let (w, h) = (spr.sheet_w as f32, spr.sheet_h as f32);
    Rect::from_min_max(Pos2::new(f[0] as f32 / w, f[1] as f32 / h), Pos2::new((f[0] + f[2]) as f32 / w, (f[1] + f[3]) as f32 / h))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use egui_kittest::Harness;

    /// A project with a 64×32 tile sheet sprite (8 tiles of 16 px).
    pub(crate) fn project_with_sheet(tag: &str) -> (PathBuf, Project) {
        let dir = crate::room::tests::temp_project(tag);
        let mut img = image::RgbaImage::new(64, 32);
        for (x, y, px) in img.enumerate_pixels_mut() {
            let t = (y / 16) * 4 + x / 16;
            *px = image::Rgba([(t * 30) as u8, 120, 200 - (t * 20) as u8, 255]);
        }
        img.save(dir.join("sprites/spr_sheet.png")).unwrap();
        let p = Project::open(&dir).unwrap();
        (dir, p)
    }

    #[test]
    fn brush_transforms_reorder_the_stamp() {
        let b = TileBrush { tiles: vec![vec![1, 2]], flags: 0, auto: None };
        assert_eq!(b.size(), (2, 1));
        assert_eq!((b.at(0, 0), b.at(1, 0), b.at(2, 0)), (1, 2, 1), "the stamp repeats");
        let m = TileBrush { flags: MIRROR, ..b.clone() };
        assert_eq!((m.at(0, 0), m.at(1, 0)), (2 | MIRROR, 1 | MIRROR));
        let r = TileBrush { flags: ROTATE, ..b.clone() };
        assert_eq!(r.size(), (1, 2));
        assert_eq!((r.at(0, 0), r.at(0, 1)), (1 | ROTATE, 2 | ROTATE), "rotated clockwise: left goes on top");
        let mut g = vec![];
        stamp(&mut g, 1, 1, &m, &[]);
        assert_eq!(g[1], vec![EMPTY, 2 | MIRROR, 1 | MIRROR]);
        fill_rect(&mut g, (0, 3), (2, 3), &b, &[]);
        assert_eq!(g[3], vec![1, 2, 1]);
        erase(&mut g, 1, 3, &b, &[]);
        assert_eq!(g[3], vec![1, EMPTY, 1]);
        let auto = vec![AutoTileToml { name: "a".into(), tiles: (10..26).collect() }];
        let ab = TileBrush { auto: Some(0), ..b };
        stamp(&mut g, 5, 5, &ab, &auto);
        stamp(&mut g, 6, 5, &ab, &auto);
        assert_eq!((g[5][5], g[5][6]), (12, 18), "neighbours: right (2) and left (8)");
    }

    struct St {
        ed: TileSetEditor,
        p: Project,
        tex: crate::tex::TexCache,
    }

    #[test]
    fn create_cut_animate_and_save() {
        let (dir, mut p) = project_with_sheet("tsedit");
        assert!(create(&mut p, "ts_x", "").is_err());
        assert_eq!(create(&mut p, "ts_x", "spr_sheet").unwrap(), Open::TileSet("ts_x".into()));
        assert_eq!(p.tileset("ts_x").unwrap().count(), 8);
        let path = p.tilesets[0].path.clone();
        let ed = TileSetEditor::open("ts_x", &path).unwrap();
        let mut h = Harness::builder().with_size([1100.0, 700.0]).allow_missing_glyphs().with_accessibility_check(false).build_ui_state(
            |ui, s: &mut St| {
                let (mut log, mut open) = (vec![], vec![]);
                let mut cx = Cx { project: &mut s.p, tex: &mut s.tex, log: &mut log, open: &mut open, active: true };
                s.ed.ui(ui, &mut cx);
            },
            St { ed, p, tex: Default::default() },
        );
        h.run_steps(3);
        {
            let s = h.state_mut();
            s.ed.sel = Some((1, 3));
            assert_eq!(s.ed.selected_tiles(), vec![1, 2, 3]);
            s.ed.mode = Mode::Animations;
        }
        h.run_steps(2);
        use egui_kittest::kittest::Queryable;
        h.get_by_label("+ animación con la selección").click();
        h.run_steps(2);
        {
            let s = h.state_mut();
            assert_eq!(s.ed.t.animations[0].frames, vec![1, 2, 3]);
            assert!(s.ed.dirty);
            let before = s.ed.t.clone();
            s.ed.undo.push(before);
            s.ed.t.tile_width = Some(64);
        }
        h.run_steps(2);
        {
            let s = h.state_mut();
            // 64-px wide tiles: only 2 tiles, the animation frames 2 and 3 are gone → reported
            assert!(matches!(&s.ed.cut, Some((_, Err(e))) if e.contains("does not exist")));
            s.ed.undo();
            assert_eq!(s.ed.t.tile_width, Some(16));
            let (mut log, mut open) = (vec![], vec![]);
            let mut cx = Cx { project: &mut s.p, tex: &mut s.tex, log: &mut log, open: &mut open, active: true };
            s.ed.save(&mut cx).unwrap();
            assert_eq!(s.p.tileset("ts_x").unwrap().anims.len(), 1);
        }
        let e = modengine::Engine::load(&dir).unwrap();
        assert_eq!(e.tilesets[0].count(), 8);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
