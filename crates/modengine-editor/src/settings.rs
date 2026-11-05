//! game.toml editor: name, display, room order and the shared palette.

use crate::app::Cx;
use crate::tex::rgb;
use crate::tomlw::{color_hex, key, num, parse_hex, q};
use eframe::egui::{self, RichText};
use modengine::project::GameToml;

pub struct SettingsEditor {
    game: GameToml,
    pub dirty: bool,
    new_char: String,
    /// [`crate::palette::generation`] seen last: a palette applied from the Paletas
    /// dialog reloads the form when it has no unsaved changes.
    pal_gen: u64,
}

pub fn write_game_toml(g: &GameToml) -> String {
    let mut s = String::from("[game]\n");
    s += &format!("name = {}\n", q(&g.game.name));
    if let Some(r) = &g.game.start_room {
        s += &format!("start_room = {}\n", q(r));
    }
    if !g.game.rooms.is_empty() {
        s += &format!("rooms = [{}]\n", g.game.rooms.iter().map(|r| q(r)).collect::<Vec<_>>().join(", "));
    }
    let d = &g.display;
    s += "\n[display]\n";
    s += &format!("width = {}\nheight = {}\nscale = {}\n", d.width, d.height, d.scale);
    s += &format!("pixel_perfect = {}\nstep_rate = {}\n", d.pixel_perfect, num(d.step_rate));
    s += &format!("letterbox = {}\nfont_size = {}\n", q(&d.letterbox), num(d.font_size as f64));
    if d.smooth {
        s += "smooth = true\n";
    }
    if d.fullscreen {
        s += "fullscreen = true\n";
    }
    if !g.palette.is_empty() {
        s += "\n[palette]\n";
        for (k, v) in &g.palette {
            s += &format!("{} = {}\n", key(k), q(v));
        }
    }
    s
}

impl SettingsEditor {
    pub fn new(p: &crate::project::Project) -> Self {
        SettingsEditor { game: p.game.clone(), dirty: false, new_char: String::new(), pal_gen: crate::palette::generation() }
    }

    pub fn save(&mut self, cx: &mut Cx) -> Result<(), String> {
        let path = cx.project.dir.join("game.toml");
        std::fs::write(&path, write_game_toml(&self.game)).map_err(|e| format!("{}: {e}", path.display()))?;
        cx.project.game = self.game.clone();
        cx.project.game_error = None;
        cx.project.reload_sprites();
        self.dirty = false;
        Ok(())
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        if self.pal_gen != crate::palette::generation() {
            self.pal_gen = crate::palette::generation();
            if !self.dirty {
                self.game = cx.project.game.clone();
            }
        }
        let before = format!("{:?}", self.game);
        let rooms = cx.project.room_names();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.label(RichText::new("Guardar reescribe game.toml (se pierden los comentarios del fichero).").weak());
            ui.heading("Juego");
            egui::Grid::new("game_grid").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.label("Nombre");
                ui.text_edit_singleline(&mut self.game.game.name);
                ui.end_row();
                ui.label("Room inicial");
                let cur = self.game.game.start_room.clone().unwrap_or_else(|| "(la primera)".into());
                egui::ComboBox::from_id_salt("start_room").selected_text(cur).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.game.game.start_room, None, "(la primera)");
                    for r in &rooms {
                        ui.selectable_value(&mut self.game.game.start_room, Some(r.clone()), r);
                    }
                });
                ui.end_row();
            });

            ui.add_space(8.0);
            ui.heading("Orden de las rooms");
            ui.label(RichText::new("Es el orden de next_room() / prev_room().").weak());
            let order = &mut self.game.game.rooms;
            if order.is_empty() {
                if ui.button("Definir un orden (ahora es alfabético)").clicked() {
                    *order = rooms.clone();
                }
            } else {
                for r in &rooms {
                    if !order.contains(r) {
                        order.push(r.clone());
                    }
                }
                order.retain(|r| rooms.contains(r));
                let mut swap = None;
                for (i, r) in order.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("{}. {r}", i + 1));
                        if ui.small_button("▲").clicked() && i > 0 {
                            swap = Some((i, i - 1));
                        }
                        if ui.small_button("▼").clicked() && i + 1 < rooms.len() {
                            swap = Some((i, i + 1));
                        }
                    });
                }
                if let Some((a, b)) = swap {
                    if b < order.len() {
                        order.swap(a, b);
                    }
                }
            }

            ui.add_space(8.0);
            ui.heading("Pantalla");
            let d = &mut self.game.display;
            egui::Grid::new("display_grid").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.label("Resolución");
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut d.width).range(16..=4096));
                    ui.label("×");
                    ui.add(egui::DragValue::new(&mut d.height).range(16..=4096));
                });
                ui.end_row();
                ui.label("Escala de la ventana");
                ui.add(egui::DragValue::new(&mut d.scale).range(1..=12));
                ui.end_row();
                ui.label("Pasos por segundo");
                ui.add(egui::DragValue::new(&mut d.step_rate).range(1.0..=240.0));
                ui.end_row();
                ui.label("Tamaño de texto");
                ui.add(egui::DragValue::new(&mut d.font_size).range(4.0..=96.0));
                ui.end_row();
                ui.label("Píxeles enteros");
                ui.checkbox(&mut d.pixel_perfect, "");
                ui.end_row();
                ui.label("Filtrado suave");
                ui.checkbox(&mut d.smooth, "");
                ui.end_row();
                ui.label("Pantalla completa");
                ui.checkbox(&mut d.fullscreen, "");
                ui.end_row();
                ui.label("Color de bandas");
                let mut c = parse_hex(&d.letterbox).unwrap_or([0, 0, 0]);
                if ui.color_edit_button_srgb(&mut c).changed() {
                    d.letterbox = color_hex(c);
                }
                ui.end_row();
            });

            ui.add_space(8.0);
            ui.heading("Paleta compartida");
            ui.label(RichText::new("Caracteres que pueden usar todos los sprites de pixel art.").weak());
            ui.horizontal(|ui| {
                if ui.button("🎨 Paletas (Lospec)…").on_hover_text("Descargar o importar una paleta (Pear36 por defecto) y aplicarla al proyecto").clicked() {
                    crate::palette::open_dialog();
                }
                let cols: Vec<u32> = modengine::palette::ordered(&self.game.palette).into_iter().map(|(_, c)| c).collect();
                if !cols.is_empty() {
                    crate::palette::strip(ui, &cols, 240.0, 14.0);
                }
            });
            let mut remove = None;
            let keys: Vec<String> = self.game.palette.keys().cloned().collect();
            egui::Grid::new("palette_grid").num_columns(3).show(ui, |ui| {
                for k in keys {
                    let v = self.game.palette.get(&k).cloned().unwrap_or_default();
                    let mut c = parse_hex(&v).unwrap_or([255, 0, 255]);
                    ui.label(RichText::new(format!("'{k}'")).monospace().color(rgb(c)));
                    if ui.color_edit_button_srgb(&mut c).changed() {
                        self.game.palette.insert(k.clone(), color_hex(c));
                    }
                    if ui.small_button("quitar").clicked() {
                        remove = Some(k.clone());
                    }
                    ui.end_row();
                }
            });
            if let Some(k) = remove {
                self.game.palette.remove(&k);
            }
            ui.horizontal(|ui| {
                ui.label("Nuevo carácter:");
                ui.add(egui::TextEdit::singleline(&mut self.new_char).desired_width(30.0));
                let ch = self.new_char.chars().next();
                if ui.add_enabled(ch.is_some_and(|c| c != '.' && c != ' '), egui::Button::new("Añadir")).clicked() {
                    self.game.palette.insert(ch.unwrap().to_string(), "#ffffff".into());
                    self.new_char.clear();
                }
            });
        });
        if format!("{:?}", self.game) != before {
            self.dirty = true;
        }
        let _ = cx;
    }
}
