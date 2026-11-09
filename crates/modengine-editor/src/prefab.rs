//! Prefab editor (`prefabs/<name>.toml`): a group of instances placed together, built on a
//! canvas around the prefab's origin. Rooms place a prefab like an object (room.rs draws it,
//! its Agrupar tool makes one from a part of a room and Desagrupar turns one back into
//! instances); scripts spawn it with `prefab_spawn()`.

use crate::app::{Cx, Open};
use crate::motion::{self, History, View};
use crate::project::Project;
use eframe::egui::{self, Color32, PointerButton, Pos2, Rect, RichText, Stroke, StrokeKind, Vec2};
use modengine::prefab::{write_toml, FlatItem, PrefabToml};
use modengine::project::InstToml;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Creates `prefabs/<name>.toml` with these instances (none from the New dialog).
pub fn create(p: &mut Project, name: &str, instances: Vec<InstToml>, grid: [u32; 2]) -> Result<Open, String> {
    if p.taken_names().contains(name) {
        return Err(format!("ya hay un asset llamado '{name}'"));
    }
    let path = p.dir.join("prefabs").join(format!("{name}.toml"));
    let data = PrefabToml { description: String::new(), grid: Some(grid), instances };
    motion::write_file(&path, &write_toml(&data))?;
    p.reload_lists();
    Ok(Open::Prefab(name.to_string()))
}

/// Moves the prefab to `.trash`.
pub fn delete(p: &mut Project, name: &str) -> Result<(), String> {
    if let Some(e) = p.prefabs.iter().find(|e| e.name == name) {
        let path = e.path.clone();
        p.trash(&path)?;
    }
    p.reload_lists();
    Ok(())
}

/// Rows of the asset browser: the icon is the sprite of the prefab's first object.
pub fn rows(p: &Project) -> Vec<crate::kinds::Row> {
    p.prefabs
        .iter()
        .map(|e| {
            let sprite = p.prefab_items(&e.name).ok().and_then(|items| items.iter().find_map(|i| p.object_sprite(&i.object)));
            crate::kinds::Row { sprite, ..crate::kinds::Row::plain(&e.name, Open::Prefab(e.name.clone())) }
        })
        .collect()
}

/// Inspector lines of a prefab.
pub fn inspector_rows(p: &Project, name: &str) -> Vec<(&'static str, String)> {
    let Some(e) = p.prefabs.iter().find(|e| e.name == name) else { return vec![] };
    let mut out = vec![("fichero", e.path.strip_prefix(&p.dir).unwrap_or(&e.path).display().to_string())];
    match &e.data {
        Ok(t) => {
            if !t.description.is_empty() {
                out.push(("descripción", t.description.clone()));
            }
            match p.prefab_items(name) {
                Ok(items) => {
                    out.push(("instancias", items.len().to_string()));
                    let mut objs: Vec<String> = items.iter().map(|i| i.object.clone()).collect();
                    objs.sort();
                    objs.dedup();
                    out.push(("objetos", objs.join(", ")));
                }
                Err(m) => out.push(("error", m)),
            }
            let used = rooms_using(p, name);
            out.push(("usado en", if used.is_empty() { "—".into() } else { used.join(", ") }));
        }
        Err(m) => out.push(("error", m.clone())),
    }
    out
}

/// Rooms that place prefab `name` (in a map legend or as a free instance).
pub fn rooms_using(p: &Project, name: &str) -> Vec<String> {
    p.rooms
        .iter()
        .filter(|r| {
            let Ok(rt) = std::fs::read_to_string(&r.path).map_err(|_| ()).and_then(|t| toml::from_str::<modengine::project::RoomToml>(&t).map_err(|_| ())) else { return false };
            rt.layers.iter().any(|l| {
                l.instances.iter().any(|i| i.object == name)
                    || l.legend.values().any(|v| match v {
                        toml::Value::String(s) => s == name,
                        toml::Value::Array(a) => a.iter().any(|x| x.as_str() == Some(name)),
                        _ => false,
                    })
            })
        })
        .map(|r| r.name.clone())
        .collect()
}

// ------------------------------------------------------------------ drawing (shared with rooms)

/// Box of an object instance whose origin is at `at` (world units).
pub fn object_box(p: &Project, object: &str, at: Pos2) -> Rect {
    match p.object_sprite(object).and_then(|s| p.sprite(&s).map(|s| (s.fw as f32, s.fh as f32, s.origin))) {
        Some((fw, fh, (ox, oy))) => Rect::from_min_size(at - Vec2::new(ox as f32, oy as f32), Vec2::new(fw, fh)),
        None => Rect::from_min_size(at, Vec2::splat(16.0)),
    }
}

/// Box of an object or prefab placed at `at`.
pub fn thing_box(p: &Project, name: &str, at: Pos2) -> Rect {
    if p.is_prefab(name) {
        let items = p.prefab_items(name).unwrap_or_default();
        let r = items.iter().map(|i| object_box(p, &i.object, at + Vec2::new(i.x as f32, i.y as f32))).reduce(|a, b| a.union(b));
        return r.unwrap_or(Rect::from_min_size(at, Vec2::splat(16.0)));
    }
    object_box(p, name, at)
}

/// Draws one object instance (sprite frame 0, or a box with its initial) with its origin at
/// `at` (world). Returns the screen rectangle it covers.
#[allow(clippy::too_many_arguments)]
pub fn draw_one(painter: &egui::Painter, tex: &mut crate::tex::TexCache, ctx: &egui::Context, p: &Project, object: &str, at: Pos2, to_s: &dyn Fn(Vec2) -> Pos2, z: f32, tint: Color32) -> Rect {
    match p.object_sprite(object).and_then(|s| tex.get(ctx, p, &s)) {
        Some(t) => {
            let screen = to_s(at.to_vec2());
            t.draw(painter, 0, screen, z, tint);
            Rect::from_min_size(screen - t.origin() * z, t.frame_size() * z)
        }
        None => {
            let r = Rect::from_min_size(to_s(at.to_vec2()), Vec2::splat(16.0) * z);
            painter.rect_filled(r.shrink(1.0), 2.0, Color32::from_rgba_unmultiplied(160, 90, 220, tint.a() / 2));
            painter.text(r.center(), egui::Align2::CENTER_CENTER, object.chars().next().unwrap_or('?'), egui::FontId::proportional((10.0 * z).clamp(8.0, 22.0)), Color32::WHITE);
            r
        }
    }
}

/// Draws a prefab with its origin at `at` (world) and a dashed frame around it. Returns the
/// screen rectangle it covers.
#[allow(clippy::too_many_arguments)]
pub fn draw_prefab(painter: &egui::Painter, tex: &mut crate::tex::TexCache, ctx: &egui::Context, p: &Project, name: &str, at: Pos2, to_s: &dyn Fn(Vec2) -> Pos2, z: f32, tint: Color32) -> Rect {
    let items = match p.prefab_items(name) {
        Ok(items) => items,
        Err(_) => {
            // broken prefab (cycle, bad file): a red box so it can still be found and removed
            let r = Rect::from_min_size(to_s(at.to_vec2()), Vec2::splat(16.0) * z);
            painter.rect_stroke(r, 2.0, Stroke::new(2.0, Color32::from_rgb(230, 70, 70)), StrokeKind::Inside);
            return r;
        }
    };
    let mut r: Option<Rect> = None;
    for it in &items {
        let one = draw_one(painter, tex, ctx, p, &it.object, at + Vec2::new(it.x as f32, it.y as f32), to_s, z, tint);
        r = Some(r.map_or(one, |a| a.union(one)));
    }
    let r = r.unwrap_or_else(|| Rect::from_min_size(to_s(at.to_vec2()), Vec2::splat(16.0) * z));
    // violet: stands out on sky blues, greens and dark backgrounds alike
    let c = Color32::from_rgba_unmultiplied(200, 120, 255, tint.a().min(220));
    dashed_rect(painter, r.expand(1.0), Stroke::new(1.5, c));
    // origin marker
    let o = to_s(at.to_vec2());
    painter.line_segment([o - Vec2::new(3.0, 0.0), o + Vec2::new(3.0, 0.0)], Stroke::new(1.0, c));
    painter.line_segment([o - Vec2::new(0.0, 3.0), o + Vec2::new(0.0, 3.0)], Stroke::new(1.0, c));
    r
}

fn dashed_rect(painter: &egui::Painter, r: Rect, st: Stroke) {
    let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    for w in pts.windows(2) {
        painter.add(egui::Shape::dashed_line(&[w[0], w[1]], st, 4.0, 3.0));
    }
}

/// Where a map places an instance inside its cell: centred, standing on the cell's bottom
/// when the sprite is taller (the engine's rule).
pub fn cell_anchor(p: &Project, object: &str, cell: Rect) -> Pos2 {
    if p.is_prefab(object) {
        return cell.min; // a prefab's origin goes on the cell's top-left corner
    }
    match p.object_sprite(object).and_then(|s| p.sprite(&s).map(|s| (Vec2::new(s.fw as f32, s.fh as f32), Vec2::new(s.origin.0 as f32, s.origin.1 as f32)))) {
        Some((size, origin)) => {
            let x = cell.left() + (cell.width() - size.x) / 2.0 + origin.x;
            let y = if size.y > cell.height() { cell.top() + cell.height() - size.y + origin.y } else { cell.top() + (cell.height() - size.y) / 2.0 + origin.y };
            Pos2::new(x, y)
        }
        None => cell.min,
    }
}

/// The free instances a placed prefab stands for (Desagrupar): nested prefabs stay nested,
/// the placement's `vars` apply to all of them.
pub fn ungroup(p: &Project, inst: &InstToml) -> Vec<InstToml> {
    let Some(t) = p.prefab(&inst.object) else { return vec![] };
    t.instances
        .iter()
        .map(|i| {
            let mut vars = i.vars.clone();
            vars.extend(inst.vars.iter().map(|(k, v)| (k.clone(), v.clone())));
            InstToml { object: i.object.clone(), x: inst.x + i.x, y: inst.y + i.y, vars }
        })
        .collect()
}

// ------------------------------------------------------------------ editor

pub struct PrefabEditor {
    pub name: String,
    file: PathBuf,
    pub data: PrefabToml,
    saved: PrefabToml,
    pub hist: History<PrefabToml>,
    pub sel: Option<usize>,
    /// Instance being dragged and the pointer's offset from its position.
    drag: Option<(usize, Vec2)>,
    view: View,
    pub snap: bool,
    /// Object or prefab placed by clicking on the canvas.
    pub brush: Option<String>,
    filter: String,
    new_var: String,
    drafts: HashMap<String, String>,
}

impl PrefabEditor {
    pub fn open(name: &str, file: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
        let data: PrefabToml = toml::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;
        Ok(PrefabEditor {
            name: name.into(),
            file: file.to_path_buf(),
            hist: History::new(&data),
            saved: data.clone(),
            data,
            sel: None,
            drag: None,
            view: View::default(),
            snap: true,
            brush: None,
            filter: String::new(),
            new_var: String::new(),
            drafts: HashMap::new(),
        })
    }

    pub fn dirty(&self) -> bool {
        self.data != self.saved
    }

    pub fn save(&mut self, cx: &mut Cx) -> Result<(), String> {
        motion::write_file(&self.file, &write_toml(&self.data))?;
        self.saved = self.data.clone();
        cx.project.reload_lists();
        Ok(())
    }

    pub fn undo(&mut self) {
        self.hist.undo(&mut self.data);
        self.sel = None;
    }

    pub fn redo(&mut self) {
        self.hist.redo(&mut self.data);
        self.sel = None;
    }

    fn grid(&self) -> Vec2 {
        let [w, h] = self.data.grid.unwrap_or([16, 16]);
        Vec2::new(w.max(1) as f32, h.max(1) as f32)
    }

    /// Screen position of a world point (tests).
    #[cfg(test)]
    pub(crate) fn screen_of(&self, x: f32, y: f32) -> Pos2 {
        self.view.to_s(Pos2::new(x, y))
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        egui::Panel::left(("prefab_side", &self.name)).resizable(true).default_size(270.0).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.side(ui, cx));
        });
        egui::Panel::right(("prefab_palette", &self.name)).resizable(true).default_size(200.0).show(ui, |ui| self.palette(ui, cx));
        egui::CentralPanel::default().show(ui, |ui| {
            self.toolbar(ui);
            self.canvas(ui, cx);
        });
        if self.sel.is_some_and(|i| i >= self.data.instances.len()) {
            self.sel = None;
        }
        self.hist.track(&self.data, self.drag.is_some());
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.snap, "ajustar a rejilla").on_hover_text("las instancias se colocan en las celdas como en un mapa de room");
            if ui.button(concat!(fa!(expand), " encajar")).clicked() {
                self.view.fitted = false;
            }
            ui.label(RichText::new(format!("{}%", (self.view.zoom * 100.0).round())).weak());
            ui.label(RichText::new(fa!(circle_info)).weak()).on_hover_text(
                "elige un objeto o prefab a la derecha y haz clic para colocarlo\narrastrar: mover · clic derecho o Supr: quitar\nla cruz azul es el origen: el punto que se coloca en la room\nrueda: zoom · botón central o Espacio+arrastrar: desplazar",
            );
        });
    }

    fn side(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        ui.heading(format!("{} {}", fa!(object_group), self.name));
        ui.label("Descripción");
        ui.add(egui::TextEdit::multiline(&mut self.data.description).desired_rows(2).desired_width(f32::INFINITY).hint_text("qué es y para qué sirve"));
        ui.horizontal(|ui| {
            ui.label("Rejilla");
            let mut g = self.data.grid.unwrap_or([16, 16]);
            let a = ui.add(egui::DragValue::new(&mut g[0]).range(1..=512)).changed();
            let b = ui.add(egui::DragValue::new(&mut g[1]).range(1..=512)).changed();
            if a || b {
                self.data.grid = Some(g);
            }
        });
        // validation with the project's own expansion (also catches cycles)
        let p: &Project = cx.project;
        let get = |n: &str| if n == self.name { Some(&self.data) } else { p.prefab(n) };
        match modengine::prefab::flatten(&self.name, &get) {
            Ok(flat) => {
                let nested = self.data.instances.iter().filter(|i| p.is_prefab(&i.object)).count();
                let mut info = format!("{} instancias", flat.len());
                if nested > 0 {
                    info += &format!(" ({nested} prefabs anidados)");
                }
                ui.label(RichText::new(info).weak());
                let unknown: Vec<&str> = flat.iter().map(|f| f.object.as_str()).filter(|o| p.object(o).is_none()).collect();
                if !unknown.is_empty() {
                    ui.colored_label(crate::theme::colors(ui.ctx()).error, format!("{} objetos que no existen: {}", fa!(triangle_exclamation), unknown.join(", ")));
                }
            }
            Err(m) => {
                ui.colored_label(crate::theme::colors(ui.ctx()).error, format!("{} {m}", fa!(triangle_exclamation)));
            }
        }
        let used = rooms_using(p, &self.name);
        if !used.is_empty() {
            ui.label(RichText::new(format!("Usado en: {}", used.join(", "))).weak());
        }
        ui.horizontal_wrapped(|ui| {
            if ui.button(concat!(fa!(crosshairs), " Origen al centro")).on_hover_text("mueve las instancias para que el origen quede en el centro del grupo").clicked() {
                self.recenter(p, false);
            }
            if ui.button(concat!(fa!(arrow_up), " Origen arriba-izquierda")).on_hover_text("mueve las instancias para que el origen sea la esquina de arriba a la izquierda (el de las celdas de un mapa)").clicked() {
                self.recenter(p, true);
            }
        });
        ui.separator();
        ui.strong("Instancias");
        let mut remove = None;
        egui::Grid::new(("prefab_list", &self.name)).num_columns(3).spacing([6.0, 2.0]).striped(true).show(ui, |ui| {
            for (i, it) in self.data.instances.iter().enumerate() {
                let icon = if p.is_prefab(&it.object) { fa!(object_group) } else { fa!(cube) };
                if ui.selectable_label(self.sel == Some(i), format!("{icon} {}", it.object)).clicked() {
                    self.sel = Some(i);
                    self.drafts.clear();
                }
                ui.label(RichText::new(format!("{}, {}", crate::tomlw::num(it.x), crate::tomlw::num(it.y))).weak().monospace());
                if ui.small_button(fa!(trash_can)).on_hover_text("quitar").clicked() {
                    remove = Some(i);
                }
                ui.end_row();
            }
        });
        if let Some(i) = remove {
            self.data.instances.remove(i);
            self.sel = None;
        }
        if self.data.instances.is_empty() {
            ui.label(RichText::new("Vacío: elige un objeto a la derecha y haz clic en el lienzo.").weak());
        }
        ui.separator();
        self.inspector(ui, cx);
    }

    /// Moves every instance so the origin is the group's centre (or its top-left corner).
    fn recenter(&mut self, p: &Project, top_left: bool) {
        let Some(b) = self.data.instances.iter().map(|i| thing_box(p, &i.object, Pos2::new(i.x as f32, i.y as f32))).reduce(|a, b| a.union(b)) else { return };
        let g = self.grid();
        let target = if top_left { b.min } else { b.center() };
        // keep the instances on the grid: shift by whole cells when snapping
        let (dx, dy) = if self.snap { ((target.x / g.x).round() * g.x, (target.y / g.y).round() * g.y) } else { (target.x.round(), target.y.round()) };
        for i in &mut self.data.instances {
            i.x -= dx as f64;
            i.y -= dy as f64;
        }
    }

    fn inspector(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        let Some(si) = self.sel else {
            ui.label(RichText::new("Haz clic en una instancia para editar su posición y variables.").weak());
            return;
        };
        let p: &Project = cx.project;
        let mut names = p.object_names();
        names.extend(p.prefab_names().into_iter().filter(|n| *n != self.name));
        let Some(inst) = self.data.instances.get_mut(si) else { return };
        ui.strong("Instancia seleccionada");
        egui::Grid::new(("prefab_inst", si)).num_columns(2).show(ui, |ui| {
            ui.label("Objeto");
            egui::ComboBox::from_id_salt(("prefab_inst_obj", si)).selected_text(inst.object.clone()).show_ui(ui, |ui| {
                for o in &names {
                    let icon = if p.is_prefab(o) { fa!(object_group) } else { fa!(cube) };
                    ui.selectable_value(&mut inst.object, o.clone(), format!("{icon} {o}"));
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
        if ui.small_button(concat!(fa!(up_right_from_square), " abrir")).clicked() {
            cx.open.push(if p.is_prefab(&inst.object) { Open::Prefab(inst.object.clone()) } else { Open::Object(inst.object.clone()) });
        }
        let typed = crate::typedvar::object_vars(p, &inst.object);
        let mut changes: Vec<(String, Option<toml::Value>)> = vec![];
        if !typed.is_empty() {
            ui.add_space(4.0);
            ui.strong("Variables del objeto");
            egui::Grid::new(("prefab_typed", si)).num_columns(3).spacing([8.0, 4.0]).show(ui, |ui| {
                for v in &typed {
                    let over = inst.vars.get(&v.name);
                    ui.label(RichText::new(&v.name).monospace().strong()).on_hover_text(format!("{}  (declarada en {})", v.ty.to_source(), v.owner));
                    let cur = over.and_then(|tv| crate::typedvar::from_toml(&v.ty, tv)).or_else(|| crate::typedvar::from_source(&v.ty, &v.value));
                    match cur {
                        Some(mut plain) => {
                            ui.horizontal(|ui| {
                                if crate::typedvar::widget(ui, ("pv", si, &v.name), &v.ty, &mut plain, p) {
                                    changes.push((v.name.clone(), Some(crate::typedvar::to_toml(&v.ty, &plain))));
                                }
                            });
                        }
                        None => {
                            ui.label(RichText::new(&v.value).monospace().weak());
                        }
                    }
                    if over.is_some() {
                        if ui.small_button(fa!(rotate_left)).on_hover_text("volver al valor del objeto").clicked() {
                            changes.push((v.name.clone(), None));
                        }
                    } else {
                        ui.label(RichText::new("objeto").small().weak());
                    }
                    ui.end_row();
                }
            });
        }
        ui.label("Otras variables (valores TOML: 3, \"texto\", true…)");
        let keys: Vec<String> = inst.vars.keys().filter(|k| !typed.iter().any(|v| v.name == **k)).cloned().collect();
        for k in keys {
            let shown = inst.vars.get(&k).map(|v| v.to_string()).unwrap_or_default();
            let draft = self.drafts.entry(k.clone()).or_insert(shown);
            ui.horizontal(|ui| {
                ui.label(RichText::new(&k).monospace());
                if ui.add(egui::TextEdit::singleline(draft).desired_width(90.0).font(egui::TextStyle::Monospace)).changed() {
                    if let Some(v) = toml::from_str::<toml::Table>(&format!("v = {draft}")).ok().and_then(|t| t.get("v").cloned()) {
                        changes.push((k.clone(), Some(v)));
                    }
                }
                if ui.small_button(fa!(trash_can)).clicked() {
                    changes.push((k.clone(), None));
                }
            });
        }
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.new_var).hint_text("nueva").desired_width(80.0));
            if ui.add_enabled(crate::tomlw::valid_name(self.new_var.trim()), egui::Button::new("+")).clicked() {
                changes.push((self.new_var.trim().to_string(), Some(toml::Value::Integer(0))));
                self.new_var.clear();
            }
        });
        for (k, v) in changes {
            match v {
                Some(v) => {
                    inst.vars.insert(k, v);
                }
                None => {
                    inst.vars.remove(&k);
                    self.drafts.remove(&k);
                }
            }
        }
        if ui.button(concat!(fa!(trash_can), " Quitar instancia")).clicked() {
            self.data.instances.remove(si);
            self.sel = None;
        }
    }

    fn palette(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        ui.strong("Colocar");
        ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("filtrar"));
        if ui.selectable_label(self.brush.is_none(), concat!(fa!(vector_square), " (solo seleccionar)")).clicked() {
            self.brush = None;
        }
        let p: &Project = cx.project;
        let f = self.filter.to_lowercase();
        let keep = |n: &str| f.is_empty() || n.to_lowercase().contains(&f);
        egui::ScrollArea::vertical().id_salt(("prefab_pal", &self.name)).show(ui, |ui| {
            ui.label(RichText::new("Objetos").weak());
            for n in p.object_names().into_iter().filter(|n| keep(n)) {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(20.0), egui::Sense::hover());
                    if let Some(t) = p.object_sprite(&n).and_then(|s| cx.tex.get(ui.ctx(), p, &s)) {
                        t.draw_fit(ui.painter(), 0, r);
                    }
                    if ui.selectable_label(self.brush.as_deref() == Some(n.as_str()), &n).clicked() {
                        self.brush = Some(n.clone());
                    }
                });
            }
            let others: Vec<String> = p.prefab_names().into_iter().filter(|n| *n != self.name && keep(n)).collect();
            if !others.is_empty() {
                ui.add_space(4.0);
                ui.label(RichText::new("Prefabs (anidados)").weak());
                for n in others {
                    if ui.selectable_label(self.brush.as_deref() == Some(n.as_str()), format!("{} {n}", fa!(object_group))).clicked() {
                        self.brush = Some(n.clone());
                    }
                }
            }
        });
    }

    /// World position for `name` with the pointer at `w`: in the cell under it (as a map
    /// would place it) when snapping, else the pointer rounded.
    fn place_at(&self, p: &Project, name: &str, w: Pos2) -> Pos2 {
        if !self.snap {
            return Pos2::new(w.x.round(), w.y.round());
        }
        let g = self.grid();
        let cell = Rect::from_min_size(Pos2::new((w.x / g.x).floor() * g.x, (w.y / g.y).floor() * g.y), g);
        let a = cell_anchor(p, name, cell);
        Pos2::new(a.x.round(), a.y.round())
    }

    fn hit(&self, p: &Project, w: Pos2) -> Option<usize> {
        self.data.instances.iter().enumerate().rev().find(|(_, i)| thing_box(p, &i.object, Pos2::new(i.x as f32, i.y as f32)).contains(w)).map(|(i, _)| i)
    }

    fn canvas(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        let (resp, painter) = motion::canvas(ui);
        let p: &Project = cx.project;
        let g = self.grid();
        let content = self.data.instances.iter().map(|i| thing_box(p, &i.object, Pos2::new(i.x as f32, i.y as f32))).fold(Rect::from_center_size(Pos2::ZERO, g * 6.0), |a, b| a.union(b));
        self.view.update(ui, &resp, content.expand(g.x));
        let pal = crate::theme::colors(ui.ctx());
        painter.rect_filled(resp.rect, 0.0, pal.canvas);
        motion::grid(&painter, &self.view, g.x, Color32::from_white_alpha(18));
        // the origin: the point that lands on the room position
        let o = self.view.to_s(Pos2::ZERO);
        let axis = Stroke::new(1.0, pal.camera.gamma_multiply(0.6));
        painter.line_segment([Pos2::new(resp.rect.left(), o.y), Pos2::new(resp.rect.right(), o.y)], axis);
        painter.line_segment([Pos2::new(o.x, resp.rect.top()), Pos2::new(o.x, resp.rect.bottom())], axis);
        painter.circle_stroke(o, 4.0, Stroke::new(2.0, pal.camera));
        painter.text(o + Vec2::new(6.0, 4.0), egui::Align2::LEFT_TOP, "origen", egui::FontId::proportional(11.0), pal.camera);

        let view = self.view;
        let z = view.zoom;
        let to_s = |v: Vec2| view.to_s(Pos2::ZERO + v);
        for (i, it) in self.data.instances.iter().enumerate() {
            let at = Pos2::new(it.x as f32, it.y as f32);
            let r = if p.is_prefab(&it.object) {
                draw_prefab(&painter, cx.tex, ui.ctx(), p, &it.object, at, &to_s, z, Color32::WHITE)
            } else {
                draw_one(&painter, cx.tex, ui.ctx(), p, &it.object, at, &to_s, z, Color32::WHITE)
            };
            if self.sel == Some(i) {
                painter.rect_stroke(r.expand(1.0), 0.0, Stroke::new(2.0, pal.selection), StrokeKind::Outside);
            }
        }

        // an object or prefab dragged from the asset browser
        let dropped = match resp.dnd_hover_payload::<Open>().map(|o| (*o).clone()) {
            Some(Open::Object(n)) => Some(n),
            Some(Open::Prefab(n)) if n != self.name => Some(n),
            _ => None,
        };
        if let (Some(name), Some(sp)) = (dropped, crate::dnd::pointer_in(ui, resp.rect)) {
            let at = self.place_at(p, &name, view.to_w(sp));
            self.preview(&painter, cx, ui.ctx(), &name, at, &to_s, z);
            if resp.dnd_release_payload::<Open>().is_some() {
                self.data.instances.push(InstToml { object: name, x: at.x as f64, y: at.y as f64, vars: toml::Table::new() });
                self.sel = Some(self.data.instances.len() - 1);
            }
            return;
        }

        if motion::View::panning(ui) {
            return;
        }
        let p: &Project = cx.project;
        let Some(sp) = resp.hover_pos().or(resp.interact_pointer_pos()) else { return };
        let w = view.to_w(sp);
        let hit = self.hit(p, w);
        if resp.hovered() && hit.is_none() && self.drag.is_none() {
            if let Some(b) = self.brush.clone() {
                let at = self.place_at(p, &b, w);
                self.preview(&painter, cx, ui.ctx(), &b, at, &to_s, z);
            }
        }
        let p: &Project = cx.project;
        painter.text(resp.rect.left_bottom() + Vec2::new(6.0, -6.0), egui::Align2::LEFT_BOTTOM, format!("x {:.0}  y {:.0}", w.x, w.y), egui::FontId::proportional(12.0), pal.canvas_text);
        let (pressed, secondary) = ui.input(|i| (i.pointer.button_pressed(PointerButton::Primary), i.pointer.button_pressed(PointerButton::Secondary)));
        if resp.hovered() && pressed {
            match (hit, self.brush.clone()) {
                (Some(i), _) => {
                    let it = &self.data.instances[i];
                    self.drag = Some((i, Vec2::new(it.x as f32, it.y as f32) - w.to_vec2()));
                    if self.sel != Some(i) {
                        self.drafts.clear();
                    }
                    self.sel = Some(i);
                }
                (None, Some(b)) => {
                    let at = self.place_at(p, &b, w);
                    self.data.instances.push(InstToml { object: b, x: at.x as f64, y: at.y as f64, vars: toml::Table::new() });
                    self.sel = Some(self.data.instances.len() - 1);
                    self.drafts.clear();
                }
                (None, None) => self.sel = None,
            }
        }
        if resp.hovered() && secondary {
            if let Some(i) = hit {
                self.data.instances.remove(i);
                self.sel = None;
            }
        }
        if let Some((i, off)) = self.drag {
            if ui.input(|i| i.pointer.primary_down()) {
                let np = w + off;
                let snapped = if self.snap {
                    let name = self.data.instances[i].object.clone();
                    // the cell under the instance's anchor point, then where a map would put it
                    self.place_at(p, &name, np)
                } else {
                    Pos2::new(np.x.round(), np.y.round())
                };
                if let Some(it) = self.data.instances.get_mut(i) {
                    it.x = snapped.x as f64;
                    it.y = snapped.y as f64;
                }
            } else {
                self.drag = None;
            }
        }
        if cx.active && ui.input(|i| i.key_pressed(egui::Key::Delete)) && ui.memory(|m| m.focused().is_none()) {
            if let Some(i) = self.sel.take() {
                if i < self.data.instances.len() {
                    self.data.instances.remove(i);
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn preview(&self, painter: &egui::Painter, cx: &mut Cx, ctx: &egui::Context, name: &str, at: Pos2, to_s: &dyn Fn(Vec2) -> Pos2, z: f32) {
        let tint = Color32::from_white_alpha(140);
        if cx.project.is_prefab(name) {
            draw_prefab(painter, cx.tex, ctx, cx.project, name, at, to_s, z, tint);
        } else {
            draw_one(painter, cx.tex, ctx, cx.project, name, at, to_s, z, tint);
        }
    }
}

/// Flattened items of a prefab (re-exported for room.rs).
pub fn items(p: &Project, name: &str) -> Vec<FlatItem> {
    p.prefab_items(name).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::LogLine;
    use crate::tex::TexCache;
    use eframe::egui::{Event, Modifiers};
    use egui_kittest::Harness;

    struct St {
        ed: PrefabEditor,
        project: Project,
        tex: TexCache,
        log: Vec<LogLine>,
        open: Vec<Open>,
    }

    fn click(h: &mut Harness<St>, p: Pos2, button: PointerButton) {
        h.event(Event::PointerMoved(p));
        h.event(Event::PointerButton { pos: p, button, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(2);
        h.event(Event::PointerButton { pos: p, button, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(2);
    }

    #[test]
    fn build_a_prefab_on_the_canvas_and_spawn_it() {
        let dir = crate::room::tests::temp_project("prefab_ed");
        let mut project = Project::open(&dir).unwrap();
        create(&mut project, "pf_test", vec![], [16, 16]).unwrap();
        assert!(create(&mut project, "Coin", vec![], [16, 16]).is_err(), "names are shared with every asset");
        let path = project.prefabs.iter().find(|e| e.name == "pf_test").unwrap().path.clone();
        let ed = PrefabEditor::open("pf_test", &path).unwrap();
        let st = St { ed, project, tex: TexCache::default(), log: vec![], open: vec![] };
        let mut h = Harness::builder().with_size([1200.0, 760.0]).allow_missing_glyphs().with_accessibility_check(false).build_ui_state(
            |ui, s: &mut St| {
                let mut cx = Cx { project: &mut s.project, tex: &mut s.tex, log: &mut s.log, open: &mut s.open, active: true };
                s.ed.ui(ui, &mut cx);
            },
            st,
        );
        h.run_steps(3);
        // a ground block in the origin cell and a coin in the cell above it
        h.state_mut().ed.brush = Some("Ground".into());
        let a = h.state().ed.screen_of(8.0, 8.0);
        click(&mut h, a, PointerButton::Primary);
        h.state_mut().ed.brush = Some("Coin".into());
        let b = h.state().ed.screen_of(8.0, -8.0);
        click(&mut h, b, PointerButton::Primary);
        {
            let d = &h.state().ed.data;
            assert_eq!(d.instances.len(), 2);
            assert_eq!((d.instances[0].object.as_str(), d.instances[0].x, d.instances[0].y), ("Ground", 0.0, 0.0), "top-left origin sprite fills the cell");
            assert_eq!((d.instances[1].object.as_str(), d.instances[1].x, d.instances[1].y), ("Coin", 8.0, -8.0), "centred sprite in the middle of its cell");
            assert_eq!(h.state().ed.hist.steps(), 2);
        }
        // right click removes the coin, undo brings it back
        click(&mut h, b, PointerButton::Secondary);
        assert_eq!(h.state().ed.data.instances.len(), 1);
        h.state_mut().ed.undo();
        assert_eq!(h.state().ed.data.instances.len(), 2);
        {
            let s = h.state_mut();
            let mut cx = Cx { project: &mut s.project, tex: &mut s.tex, log: &mut s.log, open: &mut s.open, active: true };
            s.ed.save(&mut cx).unwrap();
            assert!(!s.ed.dirty());
        }
        // a room places it and the engine creates both instances at the right place
        std::fs::write(dir.join("rooms/zz_test.toml"), "[[layer]]\ntype = \"instances\"\ninstances = [ { object = \"pf_test\", x = 64, y = 96 } ]\n").unwrap();
        let mut e = modengine::Engine::load(&dir).unwrap_or_else(|e| panic!("{e}"));
        e.start_room = e.rooms.iter().position(|r| r.name == "zz_test").unwrap();
        e.start();
        let at = |n: &str| e.insts.iter().filter(|i| e.objects[i.object as usize].name == n).map(|i| (i.x, i.y)).collect::<Vec<_>>();
        assert_eq!(at("Ground"), [(64.0, 96.0)]);
        assert_eq!(at("Coin").len(), 1);
        assert_eq!(at("Coin")[0].0, 72.0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ungroup_offsets_and_overrides() {
        let dir = crate::room::tests::temp_project("prefab_ungroup");
        let mut project = Project::open(&dir).unwrap();
        let mut vars = toml::Table::new();
        vars.insert("walk_speed".into(), toml::Value::Float(1.0));
        create(&mut project, "pf_two", vec![InstToml { object: "Slime".into(), x: 0.0, y: 0.0, vars: vars.clone() }, InstToml { object: "Coin".into(), x: 16.0, y: -8.0, vars: toml::Table::new() }], [16, 16]).unwrap();
        let mut over = toml::Table::new();
        over.insert("walk_speed".into(), toml::Value::Float(2.0));
        let out = ungroup(&project, &InstToml { object: "pf_two".into(), x: 100.0, y: 50.0, vars: over });
        assert_eq!(out.len(), 2);
        assert_eq!((out[0].x, out[0].y), (100.0, 50.0));
        assert_eq!(out[0].vars.get("walk_speed").and_then(|v| v.as_float()), Some(2.0));
        assert_eq!((out[1].object.as_str(), out[1].x, out[1].y), ("Coin", 116.0, 42.0));
        let row = rows(&project).into_iter().find(|r| r.name == "pf_two").unwrap();
        assert_eq!(row.sprite.as_deref(), Some("slime"), "the icon is the first object's sprite");
        delete(&mut project, "pf_two").unwrap();
        assert!(!project.is_prefab("pf_two"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
