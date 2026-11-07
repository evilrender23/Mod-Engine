//! Every asset kind of the editor in two tables: the **Crear** menu (GameMaker's "Create
//! asset" menu, with its Alt shortcuts) and the sections of the asset browser.
//!
//! Adding an asset kind is one row in [`CREATE`] (its `NewKind`, or `None` while it isn't
//! implemented: shown disabled as "próximamente") and one row in [`SECTIONS`].

use crate::app::{NewKind, Open};
use crate::motion::AnimKind;
use crate::project::Project;
use eframe::egui::{self, Key, Modifiers};

/// One entry of the Crear menu.
pub struct CreateRow {
    pub label: &'static str,
    pub icon: &'static str,
    /// Alt+key opens the New dialog of this kind.
    pub key: Option<Key>,
    /// `None`: not implemented yet (disabled, tooltip "próximamente").
    pub new: Option<NewKind>,
}

const fn row(label: &'static str, icon: &'static str, key: Option<Key>, new: Option<NewKind>) -> CreateRow {
    CreateRow { label, icon, key, new }
}

/// The Crear menu, alphabetical like GameMaker's. GameMaker uses Alt+S for both Sound and
/// Sprite; here Sprite is Alt+I (imagen) so every shortcut is unambiguous.
pub const CREATE: &[CreateRow] = &[
    row("Curva de animación", fa!(chart_line), Some(Key::J), Some(NewKind::Anim(AnimKind::Curve))),
    row("Extensión", fa!(puzzle_piece), Some(Key::E), Some(NewKind::Fx(crate::fx::FxKind::Extension))),
    row("Fuente", fa!(font), Some(Key::F), Some(NewKind::Font)),
    row("Idioma", fa!(globe), None, Some(NewKind::Fx(crate::fx::FxKind::Language))),
    row("Línea de tiempo", fa!(stopwatch), Some(Key::T), Some(NewKind::Anim(AnimKind::Timeline))),
    row("Nota", fa!(note_sticky), Some(Key::N), Some(NewKind::Note)),
    row("Objeto", fa!(cube), Some(Key::O), Some(NewKind::Object)),
    row("Room", fa!(map), Some(Key::R), Some(NewKind::Room)),
    row("Script", fa!(pen), Some(Key::C), Some(NewKind::Script)),
    row("Secuencia", fa!(film), Some(Key::G), Some(NewKind::Anim(AnimKind::Sequence))),
    row("Shader", fa!(circle_half_stroke), Some(Key::A), Some(NewKind::Fx(crate::fx::FxKind::Shader))),
    row("Sistema de partículas", fa!(wand_magic_sparkles), Some(Key::K), Some(NewKind::Fx(crate::fx::FxKind::Particles))),
    row("Sonido", fa!(music), Some(Key::S), Some(NewKind::Sound)),
    row("Sprite", fa!(image), Some(Key::I), Some(NewKind::PixelSprite)),
    row("Sprite desde un PNG", fa!(image), None, Some(NewKind::PngSprite)),
    row("Tabla de datos", fa!(table), None, Some(NewKind::Data)),
    row("Tile set", fa!(table_cells), Some(Key::B), Some(NewKind::TileSet)),
    row("Trayectoria", fa!(route), Some(Key::P), Some(NewKind::Anim(AnimKind::Path))),
];

pub fn shortcut_text(key: Key) -> String {
    format!("Alt+{}", key.symbol_or_name())
}

/// Draws the rows of the Crear menu (inside a menu or context menu). Returns the kind picked.
pub fn create_menu(ui: &mut egui::Ui) -> Option<NewKind> {
    let mut picked = None;
    for r in CREATE {
        let mut b = egui::Button::new(format!("{}  {}", r.icon, r.label));
        if let Some(k) = r.key {
            b = b.shortcut_text(shortcut_text(k));
        }
        let resp = ui.add_enabled(r.new.is_some(), b).on_disabled_hover_text("próximamente");
        if resp.clicked() {
            picked = r.new;
            ui.close();
        }
    }
    picked
}

/// Alt+letter pressed this frame (consumed): the kind whose New dialog to open.
pub fn shortcut(ctx: &egui::Context) -> Option<NewKind> {
    ctx.input_mut(|i| CREATE.iter().find_map(|r| match (r.key, r.new) {
        (Some(k), Some(n)) if i.consume_key(Modifiers::ALT, k) => Some(n),
        _ => None,
    }))
}

/// `--new <kind>` names: `font`, `note`, `tileset`, `sprite`, `sound`, `object`, `room`, `script`.
pub fn parse_kind(s: &str) -> Option<NewKind> {
    Some(match s {
        "font" => NewKind::Font,
        "note" => NewKind::Note,
        "tileset" => NewKind::TileSet,
        "sprite" => NewKind::PixelSprite,
        "sound" => NewKind::Sound,
        "object" => NewKind::Object,
        "room" => NewKind::Room,
        "script" => NewKind::Script,
        "data" => NewKind::Data,
        _ => return None,
    })
}

/// Title of the New dialog of a kind.
pub fn new_title(k: NewKind) -> String {
    match k {
        NewKind::PngSprite => "Importar PNG como sprite".into(),
        NewKind::PixelSprite => "Nuevo sprite (pixel art)".into(),
        _ => match CREATE.iter().find(|r| r.new == Some(k)) {
            Some(r) => format!("{} {}: nuevo asset", r.icon, r.label),
            None => "Nuevo asset".into(),
        },
    }
}

/// Extra fields of the New dialog for a kind (`extra` is the dialog's free text value).
pub fn new_extra(ui: &mut egui::Ui, kind: NewKind, extra: &mut String, p: &Project) {
    match kind {
        NewKind::TileSet => {
            ui.horizontal(|ui| {
                ui.label("Imagen (sprite):");
                egui::ComboBox::from_id_salt("new_ts_sprite").selected_text(if extra.is_empty() { "(elegir)" } else { extra.as_str() }).show_ui(ui, |ui| {
                    for s in p.sprite_names() {
                        ui.selectable_value(extra, s.clone(), s);
                    }
                });
            });
            ui.label(egui::RichText::new("Tamaño de tile, márgenes y separación se ajustan en el editor.").weak());
        }
        NewKind::Font => {
            ui.horizontal(|ui| {
                ui.label("Fichero TTF/OTF:");
                ui.add(egui::TextEdit::singleline(extra).hint_text("opcional: ruta de una fuente").desired_width(260.0));
            });
            ui.label(egui::RichText::new("Vacío: fuente integrada. En el editor eliges una del sistema o del proyecto.").weak());
        }
        _ => {}
    }
}

/// Inspector details of the kinds defined in this session (fonts, notes, tile sets).
pub fn inspector_rows(p: &Project, what: &Open) -> Vec<(&'static str, String)> {
    let rel = |path: &std::path::Path| path.strip_prefix(&p.dir).unwrap_or(path).display().to_string();
    let mut out = vec![];
    match what {
        Open::Font(n) => {
            if let Some(f) = p.fonts.iter().find(|f| &f.name == n) {
                out.push(("fichero", rel(&f.path)));
                if let Ok(t) = std::fs::read_to_string(&f.path).map_err(|_| ()).and_then(|t| if f.is_toml { toml::from_str::<modengine::font::FontToml>(&t).map_err(|_| ()) } else { Err(()) }) {
                    out.push(("tamaño", modscript::fmt_num(t.size.unwrap_or(16.0) as f64)));
                    let style = [(t.bold, "negrita"), (t.italic, "cursiva"), (t.antialias == Some(false), "sin antialias")].iter().filter(|x| x.0).map(|x| x.1).collect::<Vec<_>>().join(", ");
                    out.push(("estilo", if style.is_empty() { "normal".into() } else { style }));
                    out.push(("origen", t.sprite.or(t.file).unwrap_or_else(|| "fuente integrada".into())));
                }
            }
        }
        Open::Note(n) => {
            if let Some(f) = p.notes.iter().find(|f| &f.name == n) {
                out.push(("fichero", rel(&f.path)));
                let text = std::fs::read_to_string(&f.path).unwrap_or_default();
                out.push(("líneas", text.lines().count().to_string()));
                if let Some(first) = text.lines().find(|l| !l.trim().is_empty()) {
                    out.push(("empieza", first.trim_start_matches('#').trim().chars().take(40).collect()));
                }
            }
        }
        Open::TileSet(n) => {
            if let Some(t) = p.tilesets.iter().find(|f| &f.name == n) {
                out.push(("fichero", rel(&t.path)));
                match &t.data {
                    Ok(ts) => {
                        out.push(("imagen", ts.source.clone()));
                        out.push(("tile", format!("{} × {} px", ts.tile_w, ts.tile_h)));
                        out.push(("tiles", format!("{} ({} × {})", ts.count(), ts.columns, ts.rows)));
                        out.push(("animaciones", ts.anims.len().to_string()));
                        out.push(("auto tiles", ts.autotiles.len().to_string()));
                    }
                    Err(e) => out.push(("error", e.clone())),
                }
            }
        }
        _ => {}
    }
    out
}

// ------------------------------------------------------------------ asset browser

/// One row of the asset browser.
pub struct Row {
    /// Name used for the custom order of the section.
    pub name: String,
    pub label: String,
    /// Sprite drawn as its icon.
    pub sprite: Option<String>,
    pub what: Open,
}

impl Row {
    pub fn plain(name: &str, what: Open) -> Row {
        Row { name: name.to_string(), label: name.to_string(), sprite: None, what }
    }
}

/// One section of the asset browser.
pub struct Section {
    /// Key of the section's fold state and custom order (also the folder of import.rs kinds).
    pub key: &'static str,
    pub title: &'static str,
    /// What its "+" button creates.
    pub new: Option<NewKind>,
    pub rows: fn(&Project) -> Vec<Row>,
}

pub const SECTIONS: &[Section] = &[
    Section { key: "sprites", title: "Sprites", new: Some(NewKind::PixelSprite), rows: sprite_rows },
    Section { key: "tilesets", title: "Tile sets", new: Some(NewKind::TileSet), rows: tileset_rows },
    Section { key: "objects", title: "Objetos", new: Some(NewKind::Object), rows: object_rows },
    Section { key: "rooms", title: "Rooms", new: Some(NewKind::Room), rows: room_rows },
    Section { key: "sounds", title: "Sonidos", new: Some(NewKind::Sound), rows: sound_rows },
    Section { key: "fonts", title: "Fuentes", new: Some(NewKind::Font), rows: font_rows },
    Section { key: "scripts", title: "Scripts", new: Some(NewKind::Script), rows: script_rows },
    Section { key: "notes", title: "Notas", new: Some(NewKind::Note), rows: note_rows },
    Section { key: "paths", title: "Trayectorias", new: Some(NewKind::Anim(AnimKind::Path)), rows: crate::motion::path_rows },
    Section { key: "timelines", title: "Líneas de tiempo", new: Some(NewKind::Anim(AnimKind::Timeline)), rows: crate::motion::timeline_rows },
    Section { key: "curves", title: "Curvas de animación", new: Some(NewKind::Anim(AnimKind::Curve)), rows: crate::motion::curve_rows },
    Section { key: "sequences", title: "Secuencias", new: Some(NewKind::Anim(AnimKind::Sequence)), rows: crate::motion::sequence_rows },
    Section { key: "particles", title: "Sistemas de partículas", new: Some(NewKind::Fx(crate::fx::FxKind::Particles)), rows: crate::fx::particle_rows },
    Section { key: "shaders", title: "Shaders", new: Some(NewKind::Fx(crate::fx::FxKind::Shader)), rows: crate::fx::shader_rows },
    Section { key: "extensions", title: "Extensiones", new: Some(NewKind::Fx(crate::fx::FxKind::Extension)), rows: crate::fx::extension_rows },
    Section { key: "lang", title: "Idiomas", new: Some(NewKind::Fx(crate::fx::FxKind::Language)), rows: crate::fx::language_rows },
    Section { key: "data", title: "Tablas de datos", new: Some(NewKind::Data), rows: crate::datatable::rows },
];

fn sprite_rows(p: &Project) -> Vec<Row> {
    p.sprite_names().into_iter().map(|s| Row { sprite: Some(s.clone()), ..Row::plain(&s, Open::Sprite(s.clone())) }).collect()
}

fn object_rows(p: &Project) -> Vec<Row> {
    p.object_names()
        .into_iter()
        .map(|o| {
            let label = match p.object(&o).and_then(|e| e.parent.clone()) {
                Some(pa) => format!("{o}  ‹ {pa}"),
                None => o.clone(),
            };
            Row { label, sprite: p.object_sprite(&o), what: Open::Object(o.clone()), name: o }
        })
        .collect()
}

fn room_rows(p: &Project) -> Vec<Row> {
    let start = p.game.game.start_room.clone();
    p.room_names()
        .into_iter()
        .map(|r| Row { label: if start.as_deref() == Some(r.as_str()) { format!("{r}  {}", fa!(star)) } else { r.clone() }, ..Row::plain(&r, Open::Room(r.clone())) })
        .collect()
}

fn sound_rows(p: &Project) -> Vec<Row> {
    p.sounds.iter().map(|s| Row::plain(&s.name, Open::Sound(s.name.clone()))).collect()
}

fn script_rows(p: &Project) -> Vec<Row> {
    p.scripts.iter().map(|s| Row::plain(&s.rel, Open::Script(s.rel.clone()))).collect()
}

fn font_rows(p: &Project) -> Vec<Row> {
    p.fonts.iter().map(|s| Row::plain(&s.name, Open::Font(s.name.clone()))).collect()
}

fn tileset_rows(p: &Project) -> Vec<Row> {
    p.tilesets.iter().map(|s| Row::plain(&s.name, Open::TileSet(s.name.clone()))).collect()
}

fn note_rows(p: &Project) -> Vec<Row> {
    p.notes.iter().map(|s| Row::plain(&s.name, Open::Note(s.name.clone()))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_gamemaker_kind_has_a_unique_shortcut() {
        let keys: Vec<Key> = CREATE.iter().filter_map(|r| r.key).collect();
        let mut dedup = keys.clone();
        dedup.sort_by_key(|k| k.name());
        dedup.dedup();
        assert_eq!(keys.len(), dedup.len(), "two kinds share an Alt shortcut");
        // the 15 kinds of GameMaker's Create asset menu
        assert_eq!(keys.len(), 15);
        for (label, key) in [("Fuente", Key::F), ("Nota", Key::N), ("Tile set", Key::B), ("Objeto", Key::O), ("Room", Key::R), ("Sprite", Key::I), ("Sonido", Key::S)] {
            let r = CREATE.iter().find(|r| r.label == label).unwrap();
            assert_eq!(r.key, Some(key), "{label}");
            assert!(r.new.is_some(), "{label} is implemented");
        }
        assert_eq!(shortcut_text(Key::F), "Alt+F");
    }

    #[test]
    fn alt_shortcut_picks_the_kind() {
        let ctx = egui::Context::default();
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Key { key: Key::N, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::ALT });
        let mut got = None;
        ctx.run_ui(input, |ui| got = shortcut(ui.ctx())).textures_delta.clear();
        assert_eq!(got, Some(NewKind::Note));
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Key { key: Key::N, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
        ctx.run_ui(input, |ui| got = shortcut(ui.ctx())).textures_delta.clear();
        assert_eq!(got, None, "without Alt nothing happens");
    }
}
