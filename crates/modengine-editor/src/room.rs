//! Room editor: layers, instance placement (grid maps and free instances),
//! tile painting, room and camera settings.

use crate::app::{Cx, Open};
use crate::project::Project;
use crate::tex::{checkerboard, rgb};
use crate::tomlw::{block, color_hex, key, num, parse_hex, q};
use eframe::egui::{self, Color32, PointerButton, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};
use modengine::project::{CameraToml, InstToml, LayerToml, RoomToml};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

// ------------------------------------------------------------------ model

#[derive(Clone, PartialEq)]
struct LayerM {
    t: LayerToml,
    cell: [u32; 2],
    /// Instance map: object names per cell (usually one).
    objs: Vec<Vec<Vec<String>>>,
    /// Tile map: frame index per cell, -1 = empty.
    tiles: Vec<Vec<i32>>,
    /// Characters already used for each legend value, reused when saving.
    legend: Vec<(String, char)>,
}

#[derive(Clone, PartialEq)]
struct RoomModel {
    width: Option<u32>,
    height: Option<u32>,
    background: Option<String>,
    speed: Option<f64>,
    persistent: bool,
    code: String,
    camera: CameraToml,
    layers: Vec<LayerM>,
}

fn map_rows(map: &str) -> Vec<String> {
    let lines: Vec<&str> = map.lines().collect();
    let start = lines.iter().position(|l| !l.trim().is_empty()).unwrap_or(lines.len());
    let end = lines.iter().rposition(|l| !l.trim().is_empty()).map(|e| e + 1).unwrap_or(start);
    lines[start..end].iter().map(|l| l.trim_end().to_string()).collect()
}

impl LayerM {
    fn kind(&self) -> &str {
        &self.t.kind
    }

    fn get_obj(&self, c: i32, r: i32) -> &[String] {
        if c < 0 || r < 0 {
            return &[];
        }
        self.objs.get(r as usize).and_then(|row| row.get(c as usize)).map(|v| v.as_slice()).unwrap_or(&[])
    }

    fn set_obj(&mut self, c: i32, r: i32, v: Vec<String>) -> bool {
        if c < 0 || r < 0 || c > 4000 || r > 4000 {
            return false;
        }
        let (c, r) = (c as usize, r as usize);
        if self.objs.len() <= r {
            self.objs.resize(r + 1, vec![]);
        }
        if self.objs[r].len() <= c {
            self.objs[r].resize(c + 1, vec![]);
        }
        if self.objs[r][c] == v {
            return false;
        }
        self.objs[r][c] = v;
        true
    }

    fn get_tile(&self, c: i32, r: i32) -> i32 {
        if c < 0 || r < 0 {
            return -1;
        }
        self.tiles.get(r as usize).and_then(|row| row.get(c as usize)).copied().unwrap_or(-1)
    }

    fn set_tile(&mut self, c: i32, r: i32, t: i32) -> bool {
        if c < 0 || r < 0 || c > 4000 || r > 4000 {
            return false;
        }
        let (c, r) = (c as usize, r as usize);
        if self.tiles.len() <= r {
            self.tiles.resize(r + 1, vec![]);
        }
        if self.tiles[r].len() <= c {
            self.tiles[r].resize(c + 1, -1);
        }
        if self.tiles[r][c] == t {
            return false;
        }
        self.tiles[r][c] = t;
        true
    }

    /// Drops empty cells at the end of rows and empty rows at the bottom.
    fn normalize(&mut self) {
        for row in &mut self.objs {
            while row.last().is_some_and(|v| v.is_empty()) {
                row.pop();
            }
        }
        while self.objs.last().is_some_and(|r| r.is_empty()) {
            self.objs.pop();
        }
        for row in &mut self.tiles {
            while row.last().is_some_and(|t| *t < 0) {
                row.pop();
            }
        }
        while self.tiles.last().is_some_and(|r| r.is_empty()) {
            self.tiles.pop();
        }
    }

    /// Size of the map content in cells.
    fn extent(&self) -> (usize, usize) {
        let mut w = 0;
        let mut h = 0;
        if self.kind() == "tiles" {
            for (r, row) in self.tiles.iter().enumerate() {
                if let Some(c) = row.iter().rposition(|t| *t >= 0) {
                    w = w.max(c + 1);
                    h = r + 1;
                }
            }
        } else {
            for (r, row) in self.objs.iter().enumerate() {
                if let Some(c) = row.iter().rposition(|v| !v.is_empty()) {
                    w = w.max(c + 1);
                    h = r + 1;
                }
            }
        }
        (w, h)
    }
}

fn load_model(rt: RoomToml, project: &Project) -> RoomModel {
    let mut layers = Vec::new();
    for l in rt.layers {
        let mut m = LayerM { cell: [16, 16], objs: vec![], tiles: vec![], legend: vec![], t: l.clone() };
        match l.kind.as_str() {
            "tiles" => {
                let ts = l.tileset.as_deref().and_then(|t| project.sprite(t));
                m.cell = l.cell.unwrap_or(ts.map(|s| [s.fw, s.fh]).unwrap_or([16, 16]));
                let mut lookup: HashMap<char, i32> = HashMap::new();
                for (k, v) in &l.legend {
                    if let (Some(c), Some(i)) = (k.chars().next(), v.as_integer()) {
                        lookup.insert(c, i as i32);
                        m.legend.push((i.to_string(), c));
                    }
                }
                if let Some(map) = &l.map {
                    m.tiles = map_rows(map).iter().map(|row| row.chars().map(|c| *lookup.get(&c).unwrap_or(&-1)).collect()).collect();
                }
            }
            _ => {
                m.cell = l.cell.unwrap_or([16, 16]);
                let mut lookup: HashMap<char, Vec<String>> = HashMap::new();
                for (k, v) in &l.legend {
                    let names: Vec<String> = match v {
                        toml::Value::String(s) => vec![s.clone()],
                        toml::Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
                        _ => continue,
                    };
                    if let Some(c) = k.chars().next() {
                        m.legend.push((names.join("+"), c));
                        lookup.insert(c, names);
                    }
                }
                if let Some(map) = &l.map {
                    m.objs = map_rows(map).iter().map(|row| row.chars().map(|c| lookup.get(&c).cloned().unwrap_or_default()).collect()).collect();
                }
            }
        }
        m.t.map = None;
        m.t.legend = BTreeMap::new();
        m.normalize();
        layers.push(m);
    }
    RoomModel {
        width: rt.width,
        height: rt.height,
        background: rt.background,
        speed: rt.speed,
        persistent: rt.persistent,
        code: rt.code.unwrap_or_default(),
        camera: rt.camera,
        layers,
    }
}

const CHAR_POOL: &str = "#@%&*+=-~$?!^<>:;/|abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

fn pick_char(keyname: &str, used: &[char]) -> char {
    let first = keyname.chars().next().unwrap_or('x');
    for c in [first.to_ascii_lowercase(), first.to_ascii_uppercase()] {
        if c.is_ascii_alphanumeric() && !used.contains(&c) {
            return c;
        }
    }
    CHAR_POOL.chars().find(|c| !used.contains(c)).unwrap_or('?')
}

/// Builds `legend` + `map` text for a layer. Returns None when the map is empty.
fn map_text(l: &LayerM) -> Option<(String, String)> {
    let (w, h) = l.extent();
    if w == 0 || h == 0 {
        return None;
    }
    let tiles = l.kind() == "tiles";
    let key_of = |c: usize, r: usize| -> Option<String> {
        if tiles {
            let t = l.get_tile(c as i32, r as i32);
            (t >= 0).then(|| t.to_string())
        } else {
            let v = l.get_obj(c as i32, r as i32);
            (!v.is_empty()).then(|| v.join("+"))
        }
    };
    let mut chars: Vec<(String, char)> = Vec::new();
    let mut rows = Vec::new();
    for r in 0..h {
        let mut row = String::new();
        for c in 0..w {
            match key_of(c, r) {
                None => row.push('.'),
                Some(k) => {
                    let ch = match chars.iter().find(|(kk, _)| *kk == k) {
                        Some((_, ch)) => *ch,
                        None => {
                            let used: Vec<char> = chars.iter().map(|x| x.1).chain(['.', ' ', '\'', '"', '\\']).collect();
                            let ch = match l.legend.iter().find(|(kk, ch)| *kk == k && !used.contains(ch)) {
                                Some((_, ch)) => *ch,
                                None => {
                                    let reserved: Vec<char> = used.iter().copied().chain(l.legend.iter().filter(|(kk, _)| *kk != k).map(|x| x.1)).collect();
                                    if tiles {
                                        "0123456789".chars().chain(CHAR_POOL.chars()).find(|c| !reserved.contains(c)).unwrap_or('?')
                                    } else {
                                        pick_char(&k, &reserved)
                                    }
                                }
                            };
                            chars.push((k.clone(), ch));
                            ch
                        }
                    };
                    row.push(ch);
                }
            }
        }
        rows.push(row);
    }
    let entries: Vec<String> = chars
        .iter()
        .map(|(k, ch)| {
            let v = if tiles {
                k.clone()
            } else {
                let names: Vec<&str> = k.split('+').collect();
                if names.len() == 1 {
                    q(names[0])
                } else {
                    format!("[{}]", names.iter().map(|n| q(n)).collect::<Vec<_>>().join(", "))
                }
            };
            format!("{} = {}", q(&ch.to_string()), v)
        })
        .collect();
    Some((format!("{{ {} }}", entries.join(", ")), rows.join("\n")))
}

fn write_room(m: &RoomModel) -> String {
    let mut s = String::new();
    if let Some(w) = m.width {
        s += &format!("width = {w}\n");
    }
    if let Some(h) = m.height {
        s += &format!("height = {h}\n");
    }
    if let Some(b) = &m.background {
        s += &format!("background = {}\n", q(b));
    }
    if let Some(sp) = m.speed {
        s += &format!("speed = {}\n", num(sp));
    }
    if m.persistent {
        s += "persistent = true\n";
    }
    if !m.code.trim().is_empty() {
        s += &format!("code = {}\n", block(&m.code));
    }
    let c = &m.camera;
    if c.follow.is_some() || c.smooth != 1.0 || c.x != 0.0 || c.y != 0.0 {
        s += "\n[camera]\n";
        if let Some(f) = &c.follow {
            s += &format!("follow = {}\n", q(f));
        }
        if c.smooth != 1.0 {
            s += &format!("smooth = {}\n", num(c.smooth));
        }
        if c.x != 0.0 {
            s += &format!("x = {}\n", num(c.x));
        }
        if c.y != 0.0 {
            s += &format!("y = {}\n", num(c.y));
        }
    }
    for l in &m.layers {
        let t = &l.t;
        s += "\n[[layer]]\n";
        if !t.name.is_empty() {
            s += &format!("name = {}\n", q(&t.name));
        }
        s += &format!("type = {}\n", q(&t.kind));
        if t.depth != 0.0 {
            s += &format!("depth = {}\n", num(t.depth));
        }
        if !t.visible {
            s += "visible = false\n";
        }
        if t.alpha != 1.0 {
            s += &format!("alpha = {}\n", num(t.alpha));
        }
        match t.kind.as_str() {
            "background" => {
                if let Some(sp) = &t.sprite {
                    s += &format!("sprite = {}\n", q(sp));
                }
                if let Some(c) = &t.color {
                    s += &format!("color = {}\n", q(c));
                }
                for (flag, name) in [(t.tile_x, "tile_x"), (t.tile_y, "tile_y"), (t.stretch, "stretch")] {
                    if flag {
                        s += &format!("{name} = true\n");
                    }
                }
                if t.hspeed != 0.0 {
                    s += &format!("hspeed = {}\n", num(t.hspeed));
                }
                if t.vspeed != 0.0 {
                    s += &format!("vspeed = {}\n", num(t.vspeed));
                }
                if let Some(p) = &t.parallax {
                    s += &format!("parallax = {p}\n");
                }
            }
            "tiles" => {
                if let Some(ts) = &t.tileset {
                    s += &format!("tileset = {}\n", q(ts));
                }
            }
            _ => {}
        }
        if t.x != 0.0 {
            s += &format!("x = {}\n", num(t.x));
        }
        if t.y != 0.0 {
            s += &format!("y = {}\n", num(t.y));
        }
        if t.kind != "background" {
            s += &format!("cell = [{}, {}]\n", l.cell[0], l.cell[1]);
            if let Some((legend, map)) = map_text(l) {
                s += &format!("legend = {legend}\nmap = '''\n{map}\n'''\n");
            }
        }
        if t.kind == "instances" && !t.instances.is_empty() {
            s += "instances = [\n";
            for i in &t.instances {
                let vars = if i.vars.is_empty() {
                    String::new()
                } else {
                    let v: Vec<String> = i.vars.iter().map(|(k, v)| format!("{} = {v}", key(k))).collect();
                    format!(", vars = {{ {} }}", v.join(", "))
                };
                s += &format!("    {{ object = {}, x = {}, y = {}{vars} }},\n", q(&i.object), num(i.x), num(i.y));
            }
            s += "]\n";
        }
    }
    s
}

// ------------------------------------------------------------------ editor

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Tool {
    Paint,
    Erase,
    Select,
    Rect,
    Fill,
    Pick,
}

pub struct RoomEditor {
    pub name: String,
    path: PathBuf,
    m: RoomModel,
    undo: Vec<RoomModel>,
    redo: Vec<RoomModel>,
    pub dirty: bool,
    loaded: Option<RoomToml>,
    layer: usize,
    pub(crate) tool: Tool,
    pub(crate) brush: Option<String>,
    brush_tile: i32,
    snap: bool,
    show_grid: bool,
    show_view: bool,
    zoom: f32,
    pan: Vec2,
    fitted: bool,
    sel: Option<usize>,
    drag_offset: Vec2,
    rect_from: Option<(i32, i32)>,
    stroke: Option<RoomModel>,
    drafts: HashMap<String, String>,
    new_var: String,
    want_select: bool,
    palette_filter: String,
    /// Screen rectangle of the canvas in the last frame.
    pub(crate) canvas_rect: Rect,
}

/// Where the engine places a map instance inside its cell (see engine docs).
fn cell_anchor(cell: Rect, spr: Option<(Vec2, Vec2)>) -> Pos2 {
    match spr {
        Some((size, origin)) => {
            let x = cell.left() + (cell.width() - size.x) / 2.0 + origin.x;
            let y = if size.y > cell.height() { cell.top() + cell.height() - size.y + origin.y } else { cell.top() + (cell.height() - size.y) / 2.0 + origin.y };
            Pos2::new(x, y)
        }
        None => cell.min,
    }
}

impl RoomEditor {
    pub fn open(name: &str, path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let rt: RoomToml = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(RoomEditor {
            name: name.into(),
            path: path.into(),
            m: RoomModel { width: None, height: None, background: None, speed: None, persistent: false, code: String::new(), camera: CameraToml::default(), layers: vec![] },
            undo: vec![],
            redo: vec![],
            dirty: false,
            loaded: Some(rt),
            layer: 0,
            tool: Tool::Paint,
            brush: None,
            brush_tile: 0,
            snap: true,
            show_grid: true,
            show_view: true,
            zoom: 1.0,
            pan: Vec2::ZERO,
            fitted: false,
            sel: None,
            drag_offset: Vec2::ZERO,
            rect_from: None,
            stroke: None,
            drafts: HashMap::new(),
            new_var: String::new(),
            want_select: false,
            palette_filter: String::new(),
            canvas_rect: Rect::NOTHING,
        })
    }

    pub fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.m, prev));
            self.dirty = true;
            self.sel = None;
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.m, next));
            self.dirty = true;
            self.sel = None;
        }
    }

    pub fn save(&mut self, cx: &mut Cx) -> Result<(), String> {
        std::fs::write(&self.path, write_room(&self.m)).map_err(|e| format!("{}: {e}", self.path.display()))?;
        self.dirty = false;
        let _ = cx;
        Ok(())
    }

    fn room_size(&self, p: &Project) -> (f32, f32) {
        let d = &p.game.display;
        let (mut w, mut h) = (d.width as f32, d.height as f32);
        for l in &self.m.layers {
            let (cw, ch) = l.extent();
            w = w.max(cw as f32 * l.cell[0] as f32 + l.t.x as f32);
            h = h.max(ch as f32 * l.cell[1] as f32 + l.t.y as f32);
        }
        (self.m.width.map(|v| v as f32).unwrap_or(w), self.m.height.map(|v| v as f32).unwrap_or(h))
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        if let Some(rt) = self.loaded.take() {
            self.m = load_model(rt, cx.project);
            self.layer = self.m.layers.iter().position(|l| l.kind() == "instances").unwrap_or(0);
            if std::mem::take(&mut self.want_select) {
                self.auto_select();
            }
        }
        let before = self.m.clone();
        egui::Panel::left("room_side").resizable(true).default_size(270.0).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.side(ui, cx));
        });
        egui::Panel::right("room_palette").resizable(true).default_size(210.0).show(ui, |ui| self.palette(ui, cx));
        egui::CentralPanel::default().show(ui, |ui| {
            self.toolbar(ui);
            self.canvas(ui, cx);
        });
        // property edits (not canvas strokes) become one undo step each frame they change
        if self.stroke.is_none() && self.m != before {
            self.undo.push(before);
            self.redo.clear();
            self.dirty = true;
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (t, label, tip) in [
                (Tool::Paint, "✏ Pintar", "coloca el objeto o tile elegido (clic derecho borra)"),
                (Tool::Erase, "⌫ Borrar", "quita lo que hay bajo el cursor"),
                (Tool::Select, "⬚ Mover", "selecciona y mueve instancias libres"),
                (Tool::Rect, "▭ Rectángulo", "arrastra para rellenar un rectángulo"),
                (Tool::Fill, "🪣 Rellenar", "rellena la zona conectada"),
                (Tool::Pick, "💧 Cuentagotas", "toma el objeto o tile bajo el cursor"),
            ] {
                ui.selectable_value(&mut self.tool, t, label).on_hover_text(tip);
            }
            ui.separator();
            ui.checkbox(&mut self.snap, "ajustar a rejilla").on_hover_text("sin ajustar, Pintar crea instancias libres en cualquier posición");
            ui.checkbox(&mut self.show_grid, "rejilla");
            ui.checkbox(&mut self.show_view, "cámara");
            if ui.button("⤢ encajar").clicked() {
                self.fitted = false;
            }
            ui.label(RichText::new(format!("{}%", (self.zoom * 100.0).round())).weak());
        });
    }

    fn side(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        ui.heading(&self.name);
        ui.horizontal(|ui| {
            ui.strong("Capas");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button("+", |ui| {
                    for (kind, label) in [("instances", "Instancias"), ("tiles", "Tiles"), ("background", "Fondo")] {
                        if ui.button(label).clicked() {
                            let mut t = LayerToml { kind: kind.into(), name: format!("{label}{}", self.m.layers.len()), ..LayerToml::default() };
                            if kind == "background" {
                                t.depth = 1000.0;
                                t.color = Some("#29366f".into());
                            }
                            if kind == "tiles" {
                                t.depth = 100.0;
                                t.tileset = cx.project.sprite_names().first().cloned();
                            }
                            self.m.layers.push(LayerM { t, cell: [16, 16], objs: vec![], tiles: vec![], legend: vec![] });
                            self.layer = self.m.layers.len() - 1;
                            ui.close();
                        }
                    }
                });
            });
        });
        let mut remove = None;
        let mut swap = None;
        let n = self.m.layers.len();
        let list_id = ui.scope_id().with("room_layers");
        let dragging = crate::dnd::dragging_from(ui.ctx(), list_id);
        let mut rects = vec![];
        for i in 0..n {
            let l = &mut self.m.layers[i];
            let row = ui.horizontal(|ui| {
                let eye = if l.t.visible { "👁" } else { "–" };
                if ui.small_button(eye).on_hover_text("mostrar/ocultar").clicked() {
                    l.t.visible = !l.t.visible;
                }
                let icon = match l.t.kind.as_str() {
                    "tiles" => "▦",
                    "background" => "🖼",
                    _ => "◆",
                };
                let label = format!("{icon} {}", if l.t.name.is_empty() { &l.t.kind } else { &l.t.name });
                let r = ui.add(egui::Button::selectable(self.layer == i || dragging == Some(i), label.clone()).sense(Sense::click_and_drag())).on_hover_text("arrastra para cambiar el orden");
                crate::dnd::source(&r, || crate::dnd::ListDrag { list: list_id, from: i, label });
                if r.clicked() || r.drag_started() {
                    self.layer = i;
                    self.sel = None;
                }
                r.context_menu(|ui| {
                    if ui.button("Eliminar capa").clicked() {
                        remove = Some(i);
                        ui.close();
                    }
                });
                ui.label(RichText::new(format!("{}", num(l.t.depth))).weak());
                if ui.small_button("▲").clicked() && i > 0 {
                    swap = Some((i, i - 1));
                }
                if ui.small_button("▼").clicked() && i + 1 < n {
                    swap = Some((i, i + 1));
                }
            });
            rects.push(row.response.rect);
        }
        // drag a layer between two others (one undo step, like the arrows)
        if let Some((from, to)) = crate::dnd::reorder(ui, list_id, &rects, true) {
            let mut idx: Vec<usize> = (0..n).collect();
            crate::dnd::move_item(&mut idx, from, to);
            crate::dnd::move_item(&mut self.m.layers, from, to);
            self.layer = idx.iter().position(|k| *k == self.layer).unwrap_or(0);
        }
        if let Some((a, b)) = swap {
            self.m.layers.swap(a, b);
            self.layer = b;
        }
        if let Some(i) = remove {
            self.m.layers.remove(i);
            self.layer = self.layer.min(self.m.layers.len().saturating_sub(1));
        }
        ui.label(RichText::new("Se dibujan de mayor a menor profundidad.").weak());

        if let Some(l) = self.m.layers.get_mut(self.layer) {
            ui.separator();
            ui.strong("Capa seleccionada");
            let sprites = cx.project.sprite_names();
            egui::Grid::new("layer_props").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                ui.label("Nombre");
                ui.text_edit_singleline(&mut l.t.name);
                ui.end_row();
                ui.label("Profundidad");
                ui.add(egui::DragValue::new(&mut l.t.depth));
                ui.end_row();
                ui.label("Opacidad");
                ui.add(egui::Slider::new(&mut l.t.alpha, 0.0..=1.0));
                ui.end_row();
                ui.label("Desplazamiento");
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut l.t.x));
                    ui.add(egui::DragValue::new(&mut l.t.y));
                });
                ui.end_row();
                if l.t.kind != "background" {
                    ui.label("Celda");
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut l.cell[0]).range(1..=512));
                        ui.add(egui::DragValue::new(&mut l.cell[1]).range(1..=512));
                    });
                    ui.end_row();
                }
                if l.t.kind == "tiles" {
                    ui.label("Tileset");
                    sprite_combo(ui, "tileset", &mut l.t.tileset, &sprites, false);
                    ui.end_row();
                }
                if l.t.kind == "background" {
                    ui.label("Sprite");
                    sprite_combo(ui, "bg_sprite", &mut l.t.sprite, &sprites, true);
                    ui.end_row();
                    ui.label("Color");
                    color_opt(ui, &mut l.t.color);
                    ui.end_row();
                    ui.label("Repetir");
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut l.t.tile_x, "x");
                        ui.checkbox(&mut l.t.tile_y, "y");
                        ui.checkbox(&mut l.t.stretch, "estirar");
                    });
                    ui.end_row();
                    ui.label("Velocidad");
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut l.t.hspeed).speed(0.05));
                        ui.add(egui::DragValue::new(&mut l.t.vspeed).speed(0.05));
                    });
                    ui.end_row();
                    ui.label("Parallax");
                    let (mut px, mut py) = parallax(&l.t.parallax);
                    ui.horizontal(|ui| {
                        let a = ui.add(egui::DragValue::new(&mut px).speed(0.01).range(0.0..=2.0));
                        let b = ui.add(egui::DragValue::new(&mut py).speed(0.01).range(0.0..=2.0));
                        if a.changed() || b.changed() {
                            l.t.parallax = if px == 1.0 && py == 1.0 {
                                None
                            } else if px == py {
                                Some(toml::Value::Float(px))
                            } else {
                                Some(toml::Value::Array(vec![toml::Value::Float(px), toml::Value::Float(py)]))
                            };
                        }
                    });
                    ui.end_row();
                }
            });
            if l.t.kind == "instances" {
                ui.label(RichText::new(format!("{} instancias libres", l.t.instances.len())).weak());
            }
        }

        ui.separator();
        egui::CollapsingHeader::new(RichText::new("Room").strong()).default_open(true).show(ui, |ui| {
            let (aw, ah) = self.room_size(cx.project);
            egui::Grid::new("room_props").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                for (label, val, auto) in [("Ancho", &mut self.m.width, aw), ("Alto", &mut self.m.height, ah)] {
                    ui.label(label);
                    ui.horizontal(|ui| {
                        let mut fixed = val.is_some();
                        if ui.checkbox(&mut fixed, "fijo").changed() {
                            *val = if fixed { Some(auto as u32) } else { None };
                        }
                        match val {
                            Some(v) => {
                                ui.add(egui::DragValue::new(v).range(16..=100_000));
                            }
                            None => {
                                ui.label(RichText::new(format!("{auto} (auto)")).weak());
                            }
                        }
                    });
                    ui.end_row();
                }
                ui.label("Fondo");
                color_opt(ui, &mut self.m.background);
                ui.end_row();
                ui.label("Pasos/s");
                ui.horizontal(|ui| {
                    let mut custom = self.m.speed.is_some();
                    if ui.checkbox(&mut custom, "propio").changed() {
                        self.m.speed = if custom { Some(cx.project.game.display.step_rate) } else { None };
                    }
                    if let Some(s) = &mut self.m.speed {
                        ui.add(egui::DragValue::new(s).range(1.0..=240.0));
                    }
                });
                ui.end_row();
                ui.label("Persistente");
                ui.checkbox(&mut self.m.persistent, "");
                ui.end_row();
                ui.label("Cámara sigue a");
                let objs = cx.project.object_names();
                let cam = &mut self.m.camera;
                egui::ComboBox::from_id_salt("cam_follow").selected_text(cam.follow.clone().unwrap_or_else(|| "(nadie)".into())).show_ui(ui, |ui| {
                    ui.selectable_value(&mut cam.follow, None, "(nadie)");
                    for o in &objs {
                        ui.selectable_value(&mut cam.follow, Some(o.clone()), o);
                    }
                });
                ui.end_row();
                ui.label("Suavizado");
                ui.add(egui::Slider::new(&mut cam.smooth, 0.01..=1.0));
                ui.end_row();
                ui.label("Cámara inicial");
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut cam.x));
                    ui.add(egui::DragValue::new(&mut cam.y));
                });
                ui.end_row();
            });
            ui.label("Código de creación (se ejecuta tras crear las instancias):");
            crate::highlight::code_editor(ui, &mut self.m.code, ("roomcode", &self.name), 4);
        });
    }

    fn palette(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        let kind = self.m.layers.get(self.layer).map(|l| l.t.kind.clone()).unwrap_or_default();
        match kind.as_str() {
            "instances" => {
                ui.strong("Objetos");
                ui.add(egui::TextEdit::singleline(&mut self.palette_filter).hint_text("filtrar"));
                let names = cx.project.object_names();
                let h = (ui.available_height() * 0.6).max(160.0);
                egui::ScrollArea::vertical().id_salt("objpal").max_height(h).show(ui, |ui| {
                    for n in names {
                        if !self.palette_filter.is_empty() && !n.to_lowercase().contains(&self.palette_filter.to_lowercase()) {
                            continue;
                        }
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
                            if let Some(s) = cx.project.object_sprite(&n) {
                                if let Some(t) = cx.tex.get(ui.ctx(), cx.project, &s) {
                                    t.draw_fit(ui.painter(), 0, r);
                                }
                            } else {
                                ui.painter().rect_stroke(r.shrink(3.0), 2.0, Stroke::new(1.0, Color32::GRAY), StrokeKind::Inside);
                            }
                            let r = ui.add(egui::Button::selectable(self.brush.as_deref() == Some(n.as_str()), &n).sense(Sense::click_and_drag())).on_hover_text("clic: pincel · arrastrar a la room: colocar una instancia");
                            crate::dnd::source(&r, || Open::Object(n.clone()));
                            if r.clicked() {
                                self.brush = Some(n.clone());
                                if matches!(self.tool, Tool::Erase | Tool::Pick | Tool::Select) {
                                    self.tool = Tool::Paint;
                                }
                            }
                        });
                    }
                });
                ui.separator();
                self.inspector(ui, cx);
            }
            "tiles" => {
                ui.strong("Tiles");
                let ts = self.m.layers[self.layer].t.tileset.clone();
                let Some(ts) = ts else {
                    ui.label("Elige un tileset en las propiedades de la capa.");
                    return;
                };
                let Some(t) = cx.tex.get(ui.ctx(), cx.project, &ts) else {
                    ui.label(format!("El sprite '{ts}' no se pudo cargar."));
                    return;
                };
                ui.label(RichText::new(format!("{} tiles de {ts}", t.spr.frames.len())).weak());
                egui::ScrollArea::vertical().id_salt("tilepal").show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for f in 0..t.spr.frames.len() {
                            let (r, resp) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::click());
                            checkerboard(ui.painter(), r, 6.0);
                            t.draw_fit(ui.painter(), f, r.shrink(2.0));
                            if self.brush_tile == f as i32 {
                                ui.painter().rect_stroke(r, 2.0, Stroke::new(2.0, crate::theme::colors(ui.ctx()).selection), StrokeKind::Inside);
                            }
                            if resp.on_hover_text(format!("tile {f}")).clicked() {
                                self.brush_tile = f as i32;
                                if matches!(self.tool, Tool::Erase | Tool::Pick | Tool::Select) {
                                    self.tool = Tool::Paint;
                                }
                            }
                        }
                    });
                });
            }
            "background" => {
                ui.label("Las capas de fondo no se pintan: ajusta sus propiedades a la izquierda.");
            }
            _ => {
                ui.label("Añade una capa con el botón + de la izquierda.");
            }
        }
    }

    /// The selected free instance as the room file writes it (attached to Agente messages).
    pub fn selection_toml(&self) -> Option<String> {
        let l = self.m.layers.get(self.layer)?;
        let i = l.t.instances.get(self.sel?)?;
        let vars = if i.vars.is_empty() {
            String::new()
        } else {
            let v: Vec<String> = i.vars.iter().map(|(k, v)| format!("{} = {v}", key(k))).collect();
            format!(", vars = {{ {} }}", v.join(", "))
        };
        Some(format!("# layer {}, instance {} of its instances list\n{{ object = {}, x = {}, y = {}{vars} }}", q(&l.t.name), self.sel? + 1, q(&i.object), num(i.x), num(i.y)))
    }

    /// Selects the first free instance with the move tool (used by `--select`).
    pub fn auto_select(&mut self) {
        if self.loaded.is_some() {
            self.want_select = true;
            return;
        }
        if let Some(li) = self.m.layers.iter().position(|l| !l.t.instances.is_empty()) {
            self.layer = li;
            self.tool = Tool::Select;
            self.sel = Some(0);
        }
    }

    fn inspector(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        let Some(si) = self.sel else {
            ui.label(RichText::new("Con ⬚ Mover selecciona una instancia libre para editar su posición y variables.").weak());
            return;
        };
        let objs = cx.project.object_names();
        let Some(inst) = self.m.layers.get_mut(self.layer).and_then(|l| l.t.instances.get_mut(si)) else {
            self.sel = None;
            return;
        };
        ui.strong("Instancia seleccionada");
        egui::Grid::new("inst").num_columns(2).show(ui, |ui| {
            ui.label("Objeto");
            egui::ComboBox::from_id_salt("inst_obj").selected_text(inst.object.clone()).show_ui(ui, |ui| {
                for o in &objs {
                    ui.selectable_value(&mut inst.object, o.clone(), o);
                }
            });
            ui.end_row();
            ui.label("x, y");
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut inst.x));
                ui.add(egui::DragValue::new(&mut inst.y));
            });
            ui.end_row();
        });
        if ui.small_button("abrir objeto").clicked() {
            cx.open.push(Open::Object(inst.object.clone()));
        }
        // typed variables declared by the object (and its parents)
        let typed = crate::typedvar::object_vars(cx.project, &inst.object);
        if !typed.is_empty() {
            ui.add_space(4.0);
            ui.strong("Variables del objeto");
            let weak = crate::theme::colors(ui.ctx()).weak;
            let mut changes: Vec<(String, Option<toml::Value>)> = Vec::new();
            egui::Grid::new("inst_typed").num_columns(3).spacing([8.0, 4.0]).show(ui, |ui| {
                for v in &typed {
                    let overridden = inst.vars.get(&v.name);
                    ui.label(RichText::new(&v.name).monospace().strong()).on_hover_text(format!("{}  (declarada en {})", v.ty.to_source(), v.owner));
                    let cur = overridden
                        .and_then(|tv| crate::typedvar::from_toml(&v.ty, tv))
                        .or_else(|| crate::typedvar::from_source(&v.ty, &v.value));
                    match cur {
                        Some(mut plain) => {
                            ui.horizontal(|ui| {
                                if crate::typedvar::widget(ui, ("iv", si, &v.name), &v.ty, &mut plain, cx.project) {
                                    changes.push((v.name.clone(), Some(crate::typedvar::to_toml(&v.ty, &plain))));
                                }
                            });
                        }
                        None => {
                            ui.label(RichText::new(&v.value).monospace().color(weak)).on_hover_text("valor calculado al crear la instancia");
                        }
                    }
                    if overridden.is_some() {
                        if ui.small_button("↺").on_hover_text("volver al valor del objeto").clicked() {
                            changes.push((v.name.clone(), None));
                        }
                    } else {
                        ui.label(RichText::new("objeto").small().color(weak));
                    }
                    ui.end_row();
                }
            });
            for (k, val) in changes {
                self.drafts.remove(&k);
                match val {
                    Some(val) => {
                        inst.vars.insert(k, val);
                    }
                    None => {
                        inst.vars.remove(&k);
                    }
                }
            }
            ui.add_space(4.0);
        }
        ui.label("Otras variables (valores TOML: 3, \"texto\", true…)");
        let keys: Vec<String> = inst.vars.keys().filter(|k| !typed.iter().any(|v| v.name == **k)).cloned().collect();
        let mut remove = None;
        for k in keys {
            let shown = inst.vars.get(&k).map(|v| v.to_string()).unwrap_or_default();
            let draft = self.drafts.entry(k.clone()).or_insert(shown);
            ui.horizontal(|ui| {
                ui.label(RichText::new(&k).monospace());
                let r = ui.add(egui::TextEdit::singleline(draft).desired_width(90.0).font(egui::TextStyle::Monospace));
                if r.changed() {
                    if let Ok(t) = toml::from_str::<toml::Table>(&format!("v = {draft}")) {
                        if let Some(v) = t.get("v") {
                            inst.vars.insert(k.clone(), v.clone());
                        }
                    }
                }
                if ui.small_button("🗑").clicked() {
                    remove = Some(k.clone());
                }
            });
        }
        if let Some(k) = remove {
            inst.vars.remove(&k);
            self.drafts.remove(&k);
        }
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.new_var).hint_text("nueva").desired_width(80.0));
            if ui.add_enabled(crate::tomlw::valid_name(self.new_var.trim()), egui::Button::new("+")).clicked() {
                inst.vars.insert(self.new_var.trim().to_string(), toml::Value::Integer(0));
                self.new_var.clear();
            }
        });
        if ui.button("Eliminar instancia").clicked() {
            self.m.layers[self.layer].t.instances.remove(si);
            self.sel = None;
        }
    }

    fn canvas(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let area = resp.rect;
        self.canvas_rect = area;
        let painter = painter.with_clip_rect(area);
        let project: &Project = cx.project;
        let (rw, rh) = self.room_size(project);
        if !self.fitted && area.width() > 50.0 {
            self.zoom = ((area.width() - 40.0) / rw).min((area.height() - 40.0) / rh).clamp(0.1, 8.0);
            self.pan = (area.size() - Vec2::new(rw, rh) * self.zoom) / 2.0;
            self.fitted = true;
        }
        let space = ui.input(|i| i.key_down(egui::Key::Space));
        if let Some(hp) = resp.hover_pos() {
            let factor = ui.input(|i| crate::zoom::factor(i.smooth_scroll_delta.y, i.zoom_delta()));
            if factor != 1.0 {
                let world = (hp - area.min - self.pan) / self.zoom;
                self.zoom = (self.zoom * factor).clamp(0.1, 16.0);
                self.pan = hp - area.min - world * self.zoom;
            }
        }
        if resp.dragged_by(PointerButton::Middle) || (space && resp.dragged_by(PointerButton::Primary)) {
            self.pan += resp.drag_delta();
        }
        let z = self.zoom;
        let origin = area.min + self.pan;
        let to_s = |p: Vec2| origin + p * z;
        let to_w = |p: Pos2| Pos2::ZERO + (p - origin) / z;

        // ---- room background
        let pal = crate::theme::colors(ui.ctx());
        painter.rect_filled(area, 0.0, pal.canvas);
        let room_rect = Rect::from_min_size(to_s(Vec2::ZERO), Vec2::new(rw, rh) * z);
        let bg = self.m.background.as_deref().and_then(parse_hex).map(rgb).unwrap_or(Color32::BLACK);
        painter.rect_filled(room_rect, 0.0, bg);

        // object → sprite lookup, once per frame
        let mut spr_of: HashMap<String, Option<String>> = HashMap::new();
        for o in project.object_names() {
            let s = project.object_sprite(&o);
            spr_of.insert(o, s);
        }

        // ---- layers, back to front
        let mut order: Vec<usize> = (0..self.m.layers.len()).collect();
        order.sort_by(|a, b| self.m.layers[*b].t.depth.partial_cmp(&self.m.layers[*a].t.depth).unwrap_or(std::cmp::Ordering::Equal));
        for li in order {
            let l = &self.m.layers[li];
            if !l.t.visible {
                continue;
            }
            let a = (l.t.alpha.clamp(0.0, 1.0) * 255.0) as u8;
            let tint = Color32::from_white_alpha(a);
            let off = Vec2::new(l.t.x as f32, l.t.y as f32);
            let (cw, ch) = (l.cell[0] as f32, l.cell[1] as f32);
            match l.t.kind.as_str() {
                "background" => {
                    if let Some(c) = l.t.color.as_deref().and_then(parse_hex) {
                        painter.rect_filled(room_rect, 0.0, rgb(c).gamma_multiply(l.t.alpha as f32));
                    }
                    if let Some(t) = l.t.sprite.as_deref().and_then(|s| cx.tex.get(ui.ctx(), project, s)) {
                        let fs = t.frame_size();
                        if l.t.stretch {
                            painter.image(t.id, room_rect, t.uv(0), tint);
                        } else {
                            let nx = if l.t.tile_x { (rw / fs.x).ceil() as i32 + 1 } else { 1 };
                            let ny = if l.t.tile_y { (rh / fs.y).ceil() as i32 + 1 } else { 1 };
                            for iy in 0..ny {
                                for ix in 0..nx {
                                    let p = off + Vec2::new(ix as f32 * fs.x, iy as f32 * fs.y);
                                    painter.image(t.id, Rect::from_min_size(to_s(p), fs * z), t.uv(0), tint);
                                }
                            }
                        }
                    }
                }
                "tiles" => {
                    if let Some(t) = l.t.tileset.as_deref().and_then(|s| cx.tex.get(ui.ctx(), project, s)) {
                        for (r, row) in l.tiles.iter().enumerate() {
                            for (c, &tile) in row.iter().enumerate() {
                                if tile >= 0 && (tile as usize) < t.spr.frames.len() {
                                    let p = off + Vec2::new(c as f32 * cw, r as f32 * ch);
                                    painter.image(t.id, Rect::from_min_size(to_s(p), Vec2::new(cw, ch) * z), t.uv(tile as usize), tint);
                                }
                            }
                        }
                    }
                }
                _ => {
                    for (r, row) in l.objs.iter().enumerate() {
                        for (c, names) in row.iter().enumerate() {
                            for n in names {
                                let cell = Rect::from_min_size(Pos2::ZERO + off + Vec2::new(c as f32 * cw, r as f32 * ch), Vec2::new(cw, ch));
                                draw_object(&painter, cx.tex, ui.ctx(), project, &spr_of, n, None, cell, &to_s, z, tint);
                            }
                        }
                    }
                    for (i, inst) in l.t.instances.iter().enumerate() {
                        let at = Pos2::new(inst.x as f32, inst.y as f32);
                        let rect = draw_object(&painter, cx.tex, ui.ctx(), project, &spr_of, &inst.object, Some(at), Rect::NOTHING, &to_s, z, tint);
                        if li == self.layer && self.sel == Some(i) {
                            painter.rect_stroke(rect.expand(1.0), 0.0, Stroke::new(2.0, crate::theme::colors(ui.ctx()).selection), StrokeKind::Outside);
                        }
                    }
                }
            }
        }

        // ---- grid, room border, camera
        let active = self.m.layers.get(self.layer).cloned();
        if let Some(l) = &active {
            if self.show_grid && l.t.kind != "background" {
                let (cw, ch) = (l.cell[0] as f32, l.cell[1] as f32);
                if cw * z >= 5.0 && ch * z >= 5.0 {
                    let st = Stroke::new(1.0, Color32::from_white_alpha(22));
                    let mut x = l.t.x as f32;
                    while x <= rw {
                        painter.line_segment([to_s(Vec2::new(x, 0.0)), to_s(Vec2::new(x, rh))], st);
                        x += cw;
                    }
                    let mut y = l.t.y as f32;
                    while y <= rh {
                        painter.line_segment([to_s(Vec2::new(0.0, y)), to_s(Vec2::new(rw, y))], st);
                        y += ch;
                    }
                }
            }
        }
        painter.rect_stroke(room_rect, 0.0, Stroke::new(1.0, pal.canvas_border), StrokeKind::Outside);
        if self.show_view {
            let d = &project.game.display;
            let v = Rect::from_min_size(to_s(Vec2::new(self.m.camera.x as f32, self.m.camera.y as f32)), Vec2::new(d.width as f32, d.height as f32) * z);
            painter.rect_stroke(v, 0.0, Stroke::new(1.0, pal.camera), StrokeKind::Middle);
            painter.text(v.left_top() + Vec2::new(3.0, 2.0), egui::Align2::LEFT_TOP, "cámara", egui::FontId::proportional(11.0), pal.camera);
        }

        // ---- an object dragged from the asset browser or the palette: place one instance
        if let Some(Open::Object(name)) = resp.dnd_hover_payload::<Open>().map(|o| (*o).clone()) {
            let target = match self.m.layers.get(self.layer) {
                Some(l) if l.kind() == "instances" => Some(self.layer),
                _ => self.m.layers.iter().position(|l| l.kind() == "instances"),
            };
            if let (Some(li), Some(p)) = (target, crate::dnd::pointer_in(ui, area)) {
                let w = to_w(p);
                let l = &self.m.layers[li];
                let (cw, ch) = (l.cell[0] as f32, l.cell[1] as f32);
                let off = Vec2::new(l.t.x as f32, l.t.y as f32);
                let cell = (((w.x - off.x) / cw).floor() as i32, ((w.y - off.y) / ch).floor() as i32);
                let cell_rect = Rect::from_min_size(Pos2::ZERO + off + Vec2::new(cell.0 as f32 * cw, cell.1 as f32 * ch), Vec2::new(cw, ch));
                let at = if self.snap { None } else { Some(Pos2::new(w.x.round(), w.y.round())) };
                draw_object(&painter, cx.tex, ui.ctx(), project, &spr_of, &name, at, cell_rect, &to_s, z, Color32::from_white_alpha(170));
                if self.snap {
                    painter.rect_stroke(Rect::from_min_max(to_s(cell_rect.min.to_vec2()), to_s(cell_rect.max.to_vec2())), 0.0, Stroke::new(2.0, pal.selection), StrokeKind::Inside);
                }
                if resp.dnd_release_payload::<Open>().is_some() {
                    // the end-of-frame comparison in `ui` turns this into one undo step
                    let layer = &mut self.m.layers[li];
                    if self.snap {
                        layer.set_obj(cell.0, cell.1, vec![name.clone()]);
                    } else {
                        layer.t.instances.push(InstToml { object: name.clone(), x: w.x.round() as f64, y: w.y.round() as f64, vars: toml::Table::new() });
                    }
                    self.layer = li;
                    self.brush = Some(name);
                    self.sel = None;
                }
            }
        }

        // ---- interaction
        let Some(l) = active else { return };
        if space || l.t.kind == "background" {
            return;
        }
        let pointer = resp.hover_pos().or(resp.interact_pointer_pos());
        let (primary, secondary, pressed, released) =
            ui.input(|i| (i.pointer.primary_down(), i.pointer.secondary_down(), i.pointer.button_pressed(PointerButton::Primary) || i.pointer.button_pressed(PointerButton::Secondary), i.pointer.any_released()));
        let (cw, ch) = (l.cell[0] as f32, l.cell[1] as f32);
        let off = Vec2::new(l.t.x as f32, l.t.y as f32);
        let tiles = l.t.kind == "tiles";

        if let Some(p) = pointer {
            let w = to_w(p);
            let cell = (((w.x - off.x) / cw).floor() as i32, ((w.y - off.y) / ch).floor() as i32);
            let cell_rect = Rect::from_min_size(Pos2::ZERO + off + Vec2::new(cell.0 as f32 * cw, cell.1 as f32 * ch), Vec2::new(cw, ch));
            if resp.hovered() {
                // brush preview
                if self.tool == Tool::Paint && !secondary {
                    if tiles {
                        if let Some(t) = l.t.tileset.as_deref().and_then(|s| cx.tex.get(ui.ctx(), project, s)) {
                            painter.image(t.id, Rect::from_min_max(to_s(cell_rect.min.to_vec2()), to_s(cell_rect.max.to_vec2())), t.uv(self.brush_tile.max(0) as usize), Color32::from_white_alpha(150));
                        }
                    } else if let Some(b) = &self.brush {
                        let at = if self.snap { None } else { Some(Pos2::new(w.x.round(), w.y.round())) };
                        draw_object(&painter, cx.tex, ui.ctx(), project, &spr_of, b, at, cell_rect, &to_s, z, Color32::from_white_alpha(140));
                    }
                }
                if self.snap || tiles || self.tool != Tool::Paint {
                    painter.rect_stroke(Rect::from_min_max(to_s(cell_rect.min.to_vec2()), to_s(cell_rect.max.to_vec2())), 0.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Inside);
                }
                if let Some(a) = self.rect_from {
                    let (c0, c1) = ((a.0.min(cell.0), a.1.min(cell.1)), (a.0.max(cell.0), a.1.max(cell.1)));
                    let r = Rect::from_min_max(
                        to_s(off + Vec2::new(c0.0 as f32 * cw, c0.1 as f32 * ch)),
                        to_s(off + Vec2::new((c1.0 + 1) as f32 * cw, (c1.1 + 1) as f32 * ch)),
                    );
                    painter.rect_stroke(r, 0.0, Stroke::new(2.0, crate::theme::colors(ui.ctx()).selection), StrokeKind::Inside);
                }
                painter.text(area.left_bottom() + Vec2::new(6.0, -6.0), egui::Align2::LEFT_BOTTOM, format!("x {:.0}  y {:.0}   celda {}, {}", w.x, w.y, cell.0, cell.1), egui::FontId::proportional(12.0), pal.canvas_text);
            }

            let on_canvas = resp.is_pointer_button_down_on() || resp.dragged();
            if pressed && resp.hovered() && self.stroke.is_none() {
                self.stroke = Some(self.m.clone());
                if self.tool == Tool::Rect && primary {
                    self.rect_from = Some(cell);
                }
                if self.tool == Tool::Select && primary && !tiles {
                    self.sel = self.hit_free(project, w);
                    if let Some(i) = self.sel {
                        let inst = &self.m.layers[self.layer].t.instances[i];
                        self.drag_offset = Vec2::new(inst.x as f32, inst.y as f32) - w.to_vec2();
                        self.drafts.clear();
                    }
                }
                if self.tool == Tool::Paint && primary && !tiles && !self.snap {
                    if let Some(b) = self.brush.clone() {
                        self.m.layers[self.layer].t.instances.push(InstToml { object: b, x: w.x.round() as f64, y: w.y.round() as f64, vars: toml::Table::new() });
                    }
                }
                if self.tool == Tool::Fill && primary {
                    self.fill(cell);
                }
                if self.tool == Tool::Pick && primary {
                    self.pick(cell, project, w);
                }
            }
            if on_canvas && self.stroke.is_some() {
                let layer = &mut self.m.layers[self.layer];
                if secondary {
                    if tiles {
                        layer.set_tile(cell.0, cell.1, -1);
                    } else {
                        layer.set_obj(cell.0, cell.1, vec![]);
                        let hit = hit_free_in(layer, project, w);
                        if let Some(i) = hit {
                            layer.t.instances.remove(i);
                            self.sel = None;
                        }
                    }
                } else if primary {
                    match self.tool {
                        Tool::Paint if tiles => {
                            layer.set_tile(cell.0, cell.1, self.brush_tile);
                        }
                        Tool::Paint if self.snap => {
                            if let Some(b) = &self.brush {
                                layer.set_obj(cell.0, cell.1, vec![b.clone()]);
                            }
                        }
                        Tool::Erase => {
                            if tiles {
                                layer.set_tile(cell.0, cell.1, -1);
                            } else {
                                layer.set_obj(cell.0, cell.1, vec![]);
                                if let Some(i) = hit_free_in(layer, project, w) {
                                    layer.t.instances.remove(i);
                                    self.sel = None;
                                }
                            }
                        }
                        Tool::Select => {
                            if let Some(i) = self.sel {
                                if let Some(inst) = layer.t.instances.get_mut(i) {
                                    let mut np = w + self.drag_offset;
                                    if self.snap {
                                        np = Pos2::new(((np.x - off.x) / cw).round() * cw + off.x, ((np.y - off.y) / ch).round() * ch + off.y);
                                    }
                                    inst.x = np.x.round() as f64;
                                    inst.y = np.y.round() as f64;
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            if released {
                if let Some(a) = self.rect_from.take() {
                    let layer = &mut self.m.layers[self.layer];
                    for r in a.1.min(cell.1)..=a.1.max(cell.1) {
                        for c in a.0.min(cell.0)..=a.0.max(cell.0) {
                            if tiles {
                                layer.set_tile(c, r, self.brush_tile);
                            } else if let Some(b) = &self.brush {
                                layer.set_obj(c, r, vec![b.clone()]);
                            }
                        }
                    }
                }
            }
        }
        if released || !(primary || secondary) {
            self.rect_from = None;
            if let Some(before) = self.stroke.take() {
                if before != self.m {
                    self.undo.push(before);
                    self.redo.clear();
                    self.dirty = true;
                    if self.undo.len() > 200 {
                        self.undo.remove(0);
                    }
                }
            }
        }
        if cx.active && ui.input(|i| i.key_pressed(egui::Key::Delete)) && ui.memory(|m| m.focused().is_none()) {
            if let Some(i) = self.sel.take() {
                let before = self.m.clone();
                if i < self.m.layers[self.layer].t.instances.len() {
                    self.m.layers[self.layer].t.instances.remove(i);
                    self.undo.push(before);
                    self.dirty = true;
                }
            }
        }
    }

    /// Screen position of a room coordinate (as of the last frame).
    #[cfg(test)]
    #[cfg(test)]
    pub(crate) fn cell_objects(&self, x: i32, y: i32) -> Vec<String> {
        self.m.layers[self.layer].get_obj(x, y).to_vec()
    }

    #[cfg(test)]
    pub(crate) fn screen_of(&self, x: f32, y: f32) -> Pos2 {
        self.canvas_rect.min + self.pan + Vec2::new(x, y) * self.zoom
    }

    fn hit_free(&self, p: &Project, w: Pos2) -> Option<usize> {
        hit_free_in(&self.m.layers[self.layer], p, w)
    }

    fn fill(&mut self, cell: (i32, i32)) {
        let brush_tile = self.brush_tile;
        let brush = self.brush.clone();
        let layer = &mut self.m.layers[self.layer];
        let tiles = layer.kind() == "tiles";
        let (ew, eh) = layer.extent();
        let maxc = (ew as i32).max(cell.0 + 1).max(64);
        let maxr = (eh as i32).max(cell.1 + 1).max(64);
        if cell.0 < 0 || cell.1 < 0 {
            return;
        }
        let target_tile = layer.get_tile(cell.0, cell.1);
        let target_obj = layer.get_obj(cell.0, cell.1).to_vec();
        let new_obj: Vec<String> = brush.into_iter().collect();
        if (tiles && target_tile == brush_tile) || (!tiles && target_obj == new_obj) {
            return;
        }
        let mut stack = vec![cell];
        let mut seen = std::collections::HashSet::new();
        while let Some((c, r)) = stack.pop() {
            if c < 0 || r < 0 || c >= maxc || r >= maxr || !seen.insert((c, r)) {
                continue;
            }
            let same = if tiles { layer.get_tile(c, r) == target_tile } else { layer.get_obj(c, r) == target_obj.as_slice() };
            if !same {
                continue;
            }
            if tiles {
                layer.set_tile(c, r, brush_tile);
            } else {
                layer.set_obj(c, r, new_obj.clone());
            }
            stack.extend([(c + 1, r), (c - 1, r), (c, r + 1), (c, r - 1)]);
        }
    }

    fn pick(&mut self, cell: (i32, i32), p: &Project, w: Pos2) {
        let layer = &self.m.layers[self.layer];
        if layer.kind() == "tiles" {
            let t = layer.get_tile(cell.0, cell.1);
            if t >= 0 {
                self.brush_tile = t;
                self.tool = Tool::Paint;
            }
        } else if let Some(i) = hit_free_in(layer, p, w) {
            self.brush = Some(layer.t.instances[i].object.clone());
            self.tool = Tool::Paint;
        } else if let Some(n) = layer.get_obj(cell.0, cell.1).first() {
            self.brush = Some(n.clone());
            self.tool = Tool::Paint;
        }
    }
}

fn object_box(p: &Project, name: &str, at: Pos2) -> Rect {
    match p.object_sprite(name).and_then(|s| p.sprite(&s).map(|s| (s.fw as f32, s.fh as f32, s.origin))) {
        Some((fw, fh, (ox, oy))) => Rect::from_min_size(at - Vec2::new(ox as f32, oy as f32), Vec2::new(fw, fh)),
        None => Rect::from_min_size(at, Vec2::splat(16.0)),
    }
}

fn hit_free_in(l: &LayerM, p: &Project, w: Pos2) -> Option<usize> {
    l.t.instances.iter().enumerate().rev().find(|(_, i)| object_box(p, &i.object, Pos2::new(i.x as f32, i.y as f32)).contains(w)).map(|(i, _)| i)
}

/// Draws an object either at a free position (`at`) or placed inside `cell`.
/// Returns the screen rectangle it covers.
#[allow(clippy::too_many_arguments)]
fn draw_object(
    painter: &egui::Painter,
    tex: &mut crate::tex::TexCache,
    ctx: &egui::Context,
    project: &Project,
    spr_of: &HashMap<String, Option<String>>,
    name: &str,
    at: Option<Pos2>,
    cell: Rect,
    to_s: &dyn Fn(Vec2) -> Pos2,
    z: f32,
    tint: Color32,
) -> Rect {
    let spr = spr_of.get(name).cloned().flatten();
    let t = spr.as_deref().and_then(|s| tex.get(ctx, project, s));
    match t {
        Some(t) => {
            let pos = at.unwrap_or_else(|| cell_anchor(cell, Some((t.frame_size(), t.origin()))));
            let screen = to_s(pos.to_vec2());
            t.draw(painter, 0, screen, z, tint);
            Rect::from_min_size(screen - t.origin() * z, t.frame_size() * z)
        }
        None => {
            let min = at.unwrap_or(cell.min);
            let size = if at.is_some() { Vec2::splat(16.0) } else { cell.size() };
            let r = Rect::from_min_size(to_s(min.to_vec2()), size * z);
            painter.rect_filled(r.shrink(1.0), 2.0, Color32::from_rgba_unmultiplied(160, 90, 220, tint.a() / 2));
            painter.text(r.center(), egui::Align2::CENTER_CENTER, name.chars().next().unwrap_or('?'), egui::FontId::proportional((10.0 * z).clamp(8.0, 22.0)), Color32::WHITE);
            r
        }
    }
}

fn parallax(v: &Option<toml::Value>) -> (f64, f64) {
    let f = |v: &toml::Value| v.as_float().or_else(|| v.as_integer().map(|i| i as f64));
    match v {
        Some(toml::Value::Array(a)) if a.len() == 2 => (f(&a[0]).unwrap_or(1.0), f(&a[1]).unwrap_or(1.0)),
        Some(other) => {
            let p = f(other).unwrap_or(1.0);
            (p, p)
        }
        None => (1.0, 1.0),
    }
}

fn sprite_combo(ui: &mut egui::Ui, id: &str, v: &mut Option<String>, sprites: &[String], allow_none: bool) {
    egui::ComboBox::from_id_salt(id).selected_text(v.clone().unwrap_or_else(|| "(ninguno)".into())).show_ui(ui, |ui| {
        if allow_none {
            ui.selectable_value(v, None, "(ninguno)");
        }
        for s in sprites {
            ui.selectable_value(v, Some(s.clone()), s);
        }
    });
}

fn color_opt(ui: &mut egui::Ui, v: &mut Option<String>) {
    ui.horizontal(|ui| {
        let mut on = v.is_some();
        if ui.checkbox(&mut on, "").changed() {
            *v = if on { Some("#29366f".into()) } else { None };
        }
        if let Some(s) = v {
            let mut c = parse_hex(s).unwrap_or([0, 0, 0]);
            if ui.color_edit_button_srgb(&mut c).changed() {
                *s = color_hex(c);
            }
        }
    });
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::app::LogLine;
    use crate::tex::TexCache;
    use eframe::egui::{Event, Modifiers};
    use egui_kittest::Harness;

    pub(crate) fn temp_project(tag: &str) -> PathBuf {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/platformer");
        let dst = std::env::temp_dir().join(format!("modeditor_test_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dst);
        copy_dir(&src, &dst);
        dst
    }

    fn copy_dir(src: &Path, dst: &Path) {
        std::fs::create_dir_all(dst).unwrap();
        for e in std::fs::read_dir(src).unwrap() {
            let e = e.unwrap();
            let to = dst.join(e.file_name());
            if e.path().is_dir() {
                if e.file_name() != "saves" {
                    copy_dir(&e.path(), &to);
                }
            } else {
                std::fs::copy(e.path(), to).unwrap();
            }
        }
    }

    struct St {
        ed: RoomEditor,
        project: Project,
        tex: TexCache,
        log: Vec<LogLine>,
        open: Vec<Open>,
    }

    fn press(h: &Harness<St>, p: Pos2, button: PointerButton, pressed: bool) {
        h.event(Event::PointerMoved(p));
        h.event(Event::PointerButton { pos: p, button, pressed, modifiers: Modifiers::NONE });
    }

    #[test]
    fn paint_erase_undo_and_save() {
        let dir = temp_project("room");
        let project = Project::open(&dir).unwrap();
        let path = project.rooms.iter().find(|r| r.name == "title").unwrap().path.clone();
        let ed = RoomEditor::open("title", &path).unwrap();
        let st = St { ed, project, tex: TexCache::default(), log: vec![], open: vec![] };
        let mut h = Harness::builder().with_size([1300.0, 820.0]).allow_missing_glyphs().with_accessibility_check(false).build_ui_state(
            |ui, s: &mut St| {
                let mut cx = Cx { project: &mut s.project, tex: &mut s.tex, log: &mut s.log, open: &mut s.open, active: true };
                s.ed.ui(ui, &mut cx);
            },
            st,
        );
        h.run_steps(3);
        {
            let s = h.state_mut();
            assert_eq!(s.ed.m.layers[s.ed.layer].kind(), "instances");
            s.ed.brush = Some("Coin".into());
            s.ed.tool = Tool::Paint;
        }
        // drag across two empty cells of row 3
        let a = h.state().ed.screen_of(5.0 * 16.0 + 8.0, 3.0 * 16.0 + 8.0);
        let b = h.state().ed.screen_of(6.0 * 16.0 + 8.0, 3.0 * 16.0 + 8.0);
        press(&h, a, PointerButton::Primary, true);
        h.run_steps(2);
        h.event(Event::PointerMoved(b));
        h.run_steps(2);
        press(&h, b, PointerButton::Primary, false);
        h.run_steps(2);
        {
            let l = &h.state().ed.m.layers[h.state().ed.layer];
            assert_eq!(l.get_obj(5, 3), ["Coin".to_string()]);
            assert_eq!(l.get_obj(6, 3), ["Coin".to_string()]);
            assert_eq!(h.state().ed.undo.len(), 1, "one drag = one undo step");
        }
        // right click erases
        press(&h, a, PointerButton::Secondary, true);
        h.run_steps(2);
        press(&h, a, PointerButton::Secondary, false);
        h.run_steps(2);
        assert!(h.state().ed.m.layers[h.state().ed.layer].get_obj(5, 3).is_empty());
        // undo the erase, then save and check the file and the engine
        h.state_mut().ed.undo();
        assert_eq!(h.state().ed.m.layers[h.state().ed.layer].get_obj(5, 3), ["Coin".to_string()]);
        {
            let s = h.state_mut();
            let mut cx = Cx { project: &mut s.project, tex: &mut s.tex, log: &mut s.log, open: &mut s.open, active: true };
            s.ed.save(&mut cx).unwrap();
        }
        let rt: RoomToml = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let m = load_model(rt, &h.state().project);
        let l = m.layers.iter().find(|l| l.kind() == "instances").unwrap();
        assert_eq!(l.get_obj(6, 3), ["Coin".to_string()]);
        let e = modengine::Engine::load(&dir).expect("edited project must load in the engine");
        assert!(e.rooms.iter().any(|r| r.name == "title"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A real mouse drag: press, move in steps (past the click distance), release.
    pub(crate) fn drag<S>(h: &mut Harness<S>, from: Pos2, to: Pos2) {
        h.event(Event::PointerMoved(from));
        h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(1);
        for k in 1..=8 {
            h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 8.0)));
            h.run_steps(1);
        }
        h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(3);
    }

    fn room_harness(tag: &str) -> (PathBuf, Harness<'static, St>) {
        let dir = temp_project(tag);
        let project = Project::open(&dir).unwrap();
        let path = project.rooms.iter().find(|r| r.name == "title").unwrap().path.clone();
        let ed = RoomEditor::open("title", &path).unwrap();
        let st = St { ed, project, tex: TexCache::default(), log: vec![], open: vec![] };
        let mut h = Harness::builder().with_size([1300.0, 820.0]).allow_missing_glyphs().with_accessibility_check(false).build_ui_state(
            |ui, s: &mut St| {
                let mut cx = Cx { project: &mut s.project, tex: &mut s.tex, log: &mut s.log, open: &mut s.open, active: true };
                s.ed.ui(ui, &mut cx);
            },
            st,
        );
        h.run_steps(3);
        (dir, h)
    }

    #[test]
    fn drag_object_from_palette_places_one_instance() {
        use egui_kittest::kittest::Queryable;
        let (dir, mut h) = room_harness("dnd_place");
        assert!(h.state().ed.brush.is_none());
        let from = h.get_by_label("Coin").rect().center();
        let to = h.state().ed.screen_of(8.0 * 16.0 + 8.0, 5.0 * 16.0 + 8.0);
        drag(&mut h, from, to);
        let ed = &h.state().ed;
        assert_eq!(ed.m.layers[ed.layer].get_obj(8, 5), ["Coin".to_string()]);
        assert_eq!(ed.undo.len(), 1, "one drop = one undo step");
        assert_eq!(ed.brush.as_deref(), Some("Coin"), "the dropped object becomes the brush");
        h.state_mut().ed.undo();
        assert!(h.state().ed.m.layers[h.state().ed.layer].get_obj(8, 5).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn drag_layer_to_the_top() {
        use egui_kittest::kittest::Queryable;
        let (dir, mut h) = room_harness("dnd_layers");
        let kinds = |h: &Harness<St>| h.state().ed.m.layers.iter().map(|l| l.kind().to_string()).collect::<Vec<_>>();
        assert_eq!(kinds(&h), ["background", "background", "instances"]);
        let from = h.get_by_label("◆ instances").rect().center();
        let first = h.get_all_by_label("🖼 background").next().unwrap().rect();
        drag(&mut h, from, first.center() - Vec2::new(0.0, 5.0));
        assert_eq!(kinds(&h), ["instances", "background", "background"]);
        let ed = &h.state().ed;
        assert_eq!(ed.layer, 0, "the selected layer follows the move");
        assert_eq!(ed.undo.len(), 1);
        h.state_mut().ed.undo();
        assert_eq!(kinds(&h), ["background", "background", "instances"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn example_rooms_roundtrip() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/platformer");
        let project = Project::open(&dir).unwrap();
        for r in &project.rooms {
            let text = std::fs::read_to_string(&r.path).unwrap();
            let rt: RoomToml = toml::from_str(&text).unwrap();
            let m = load_model(rt.clone(), &project);
            let out = write_room(&m);
            let back: RoomToml = toml::from_str(&out).unwrap_or_else(|e| panic!("{}: {e}\n{out}", r.name));
            let m2 = load_model(back, &project);
            assert!(m == m2, "room {} changed after save/load", r.name);
            // same instances in the same cells
            for (a, b) in m.layers.iter().zip(&m2.layers) {
                assert_eq!(a.extent(), b.extent());
            }
        }
    }
}
