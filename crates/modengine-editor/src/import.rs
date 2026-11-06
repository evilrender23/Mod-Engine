//! Importing files from anywhere on disk into the open project: PNG images become sprites
//! (`sprites/`), WAV/OGG files sounds (`sounds/`) and TTF/OTF files fonts (`fonts/`).
//!
//! Importing is copying the file into the project as `<name>.<ext>` and, only for a sprite with
//! options that differ from the engine's defaults (strip or grid of frames, origin, fps),
//! writing the companion `<name>.toml`. Nothing here changes what the engine understands.
//!
//! The dialog has a path field (paste a file or a folder; `~` and `file://` work), a small file
//! browser with multi-select, and per-file options. Files dropped from the OS onto the editor
//! window (anywhere but the Agente panel) open it prefilled; see [`route_drop`].

use crate::project::Project;
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Stroke, TextureOptions, Vec2};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

// The files and names themselves are the engine's (shared with `modengine import`, which the
// coding agent runs): both write the same files.
pub use modengine::import::{expand, human_size, import, sanitize, strip_suffix, unique, valid_name, Frames, Imported, Kind, Origin, SpriteOpts, BIG_FILE, DEFAULT_FPS};
use modengine::import::header_problem;
#[cfg(test)]
use modengine::import::sprite_toml;

/// Every name in use in the project and what has it (`None`: an object or a room). Assets
/// share one namespace.
pub fn owners(p: &Project) -> BTreeMap<String, Option<Kind>> {
    let mut m = BTreeMap::new();
    for n in p.taken_names() {
        m.insert(n, None);
    }
    for s in &p.sprites {
        m.insert(s.name.clone(), Some(Kind::Sprite));
    }
    for s in &p.sounds {
        m.insert(s.name.clone(), Some(Kind::Sound));
    }
    for s in &p.fonts {
        m.insert(s.name.clone(), Some(Kind::Font));
    }
    m
}

/// The browser/editor key of an imported asset (fonts have no editor).
pub trait OpenImported {
    fn open(&self) -> Option<crate::app::Open>;
}

impl OpenImported for Imported {
    fn open(&self) -> Option<crate::app::Open> {
        match self.kind {
            Kind::Sprite => Some(crate::app::Open::Sprite(self.name.clone())),
            Kind::Sound => Some(crate::app::Open::Sound(self.name.clone())),
            Kind::Font => Some(crate::app::Open::Font(self.name.clone())),
        }
    }
}

// ------------------------------------------------------------------ editor hooks

/// What the asset browser section `key` imports (`sprites`, `sounds`, `fonts`).
pub fn section_kind(key: &str) -> Option<Kind> {
    Kind::ALL.into_iter().find(|k| k.folder() == key)
}

pub fn menu_label(k: Kind) -> &'static str {
    match k {
        Kind::Sprite => "Importar imágenes PNG…",
        Kind::Sound => "Importar sonidos WAV/OGG…",
        Kind::Font => "Importar fuentes TTF/OTF…",
    }
}

/// Screen rectangle (last frame) of the side panel `Panel::…(id_salt)` shown in `ui`, read
/// from egui's memory so the panel's own code needs no changes.
pub fn panel_rect(ui: &egui::Ui, id_salt: &str) -> Option<Rect> {
    egui::PanelState::load(ui.ctx(), ui.scope_id().with(id_salt)).map(|s| s.outer_rect)
}

// ------------------------------------------------------------------ OS drops

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DropTarget {
    /// Over the Agente panel: it attaches the files, the import dialog leaves them alone.
    Agent,
    Import,
    /// No project open.
    Ignore,
}

/// Who gets files dropped from the OS at `pointer` (unknown on some platforms while dragging
/// files: then they are imported). `agent` is the Agente dock's rectangle when it is shown.
pub fn route_drop(pointer: Option<Pos2>, agent: Option<Rect>, has_project: bool) -> DropTarget {
    if !has_project {
        DropTarget::Ignore
    } else if matches!((pointer, agent), (Some(p), Some(r)) if r.contains(p)) {
        DropTarget::Agent
    } else {
        DropTarget::Import
    }
}

/// "Drop to import" hint over the whole window while files are dragged over it.
pub fn paint_drop_hint(ctx: &egui::Context, n: usize) {
    let rect = ctx.content_rect();
    let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("import_drop_hint")));
    let accent = ctx.global_style().visuals.selection.bg_fill;
    p.rect_filled(rect, 0.0, Color32::from_black_alpha(110));
    p.rect_stroke(rect.shrink(6.0), 8.0, Stroke::new(3.0, accent), egui::StrokeKind::Inside);
    let text = if n == 1 { "Suelta para importar el fichero".to_string() } else { format!("Suelta para importar {n} ficheros") };
    p.text(rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(26.0), Color32::WHITE);
}

// ------------------------------------------------------------------ dialog state

/// A file waiting in the dialog.
pub struct Item {
    pub src: PathBuf,
    pub kind: Option<Kind>,
    pub name: String,
    pub bytes: u64,
    /// Image size, or why it can't be read (sprites only).
    pub image: Option<Result<(u32, u32), String>>,
    /// Problem with the file itself (unreadable, wrong header...).
    pub file_error: Option<String>,
    /// 0: one frame, 1: horizontal strip, 2: grid.
    pub mode: u8,
    pub strip: u32,
    pub grid: [u32; 2],
    pub origin: Origin,
    pub fps: f64,
    pub include: bool,
    pub overwrite: bool,
    /// Error of the last import attempt.
    pub failed: Option<String>,
    thumb: Option<([usize; 2], Vec<u8>)>,
    tex: Option<egui::TextureHandle>,
}

/// What [`Item::status`] found.
#[derive(Default, Debug, PartialEq)]
pub struct Status {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    /// The name belongs to an asset of the same kind: "Sobrescribir" can fix the error.
    pub can_overwrite: bool,
    /// `(frames, frame w, frame h)` for images.
    pub slice: Option<(u32, u32, u32)>,
}

impl Item {
    /// Reads what's needed to show and validate `src` (an image is decoded here).
    pub fn new(src: &Path, name: String) -> Item {
        let kind = Kind::of(src);
        let mut it = Item {
            src: src.to_path_buf(),
            kind,
            name,
            bytes: 0,
            image: None,
            file_error: None,
            mode: 0,
            strip: 1,
            grid: [16, 16],
            origin: Origin::TopLeft,
            fps: DEFAULT_FPS,
            include: kind.is_some(),
            overwrite: false,
            failed: None,
            thumb: None,
            tex: None,
        };
        match fs::metadata(src) {
            Ok(m) if m.is_file() => it.bytes = m.len(),
            Ok(_) => it.file_error = Some("no es un fichero".into()),
            Err(e) => it.file_error = Some(format!("no se puede leer: {e}")),
        }
        if it.file_error.is_some() {
            it.include = false;
            return it;
        }
        match kind {
            Some(Kind::Sprite) => {
                it.image = Some(match fs::read(src).map_err(|e| e.to_string()).and_then(|b| image::load_from_memory(&b).map_err(|e| e.to_string())) {
                    Ok(img) => {
                        let img = img.to_rgba8();
                        let (w, h) = img.dimensions();
                        // a sensible start: square frames side by side, cells as tall as the image
                        it.strip = if h > 0 && w > h && w % h == 0 { w / h } else { 1 };
                        let cell = if h > 0 && w % h == 0 { h } else { w.min(h) };
                        it.grid = [cell.max(1), cell.max(1)];
                        let small = if w.max(h) > 512 {
                            let k = 512.0 / w.max(h) as f32;
                            image::imageops::thumbnail(&img, ((w as f32 * k) as u32).max(1), ((h as f32 * k) as u32).max(1))
                        } else {
                            img
                        };
                        let (tw, th) = small.dimensions();
                        it.thumb = Some(([tw as usize, th as usize], small.into_raw()));
                        Ok((w, h))
                    }
                    Err(e) => Err(format!("no se puede leer la imagen: {e}")),
                });
            }
            Some(k) => it.file_error = header_problem(k, src),
            None => {}
        }
        if it.file_error.is_some() || matches!(it.image, Some(Err(_))) {
            it.include = false;
        }
        it
    }

    pub fn opts(&self) -> SpriteOpts {
        let frames = match self.mode {
            1 => Frames::Strip(self.strip),
            2 => Frames::Grid(self.grid[0], self.grid[1]),
            _ => Frames::Single,
        };
        SpriteOpts { frames, origin: self.origin, fps: self.fps }
    }

    pub fn file_name(&self) -> String {
        self.src.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default()
    }

    /// Problems importing it as it is set now. `others`: names of the items before it.
    pub fn status(&self, owners: &BTreeMap<String, Option<Kind>>, others: &BTreeSet<String>) -> Status {
        let mut st = Status::default();
        let Some(kind) = self.kind else {
            let ext = self.src.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_else(|| "sin extensión".into());
            st.errors.push(format!("formato no soportado ({ext}): usa PNG, WAV, OGG, TTF u OTF"));
            return st;
        };
        if let Some(e) = &self.file_error {
            st.errors.push(e.clone());
        }
        if let Some(Err(e)) = &self.image {
            st.errors.push(e.clone());
        }
        if self.bytes > BIG_FILE {
            st.warnings.push(format!("fichero grande ({}): el juego tardará más en cargar", human_size(self.bytes)));
        }
        if let Some(Ok((w, h))) = self.image {
            let o = self.opts();
            match o.slice(w, h) {
                Ok(s) => st.slice = Some(s),
                Err(e) => st.errors.push(e),
            }
            if let Some(w) = o.leftover(w, h) {
                st.warnings.push(w);
            }
        }
        if !valid_name(&self.name) {
            st.errors.push("nombre no válido: letras, números y _, sin empezar por número".into());
        } else if others.contains(&self.name) {
            st.errors.push(format!("otro fichero de la lista ya se llama '{}'", self.name));
        } else if let Some(owner) = owners.get(&self.name) {
            if *owner == Some(kind) {
                st.can_overwrite = true;
                if self.overwrite {
                    st.warnings.push(format!("sustituye al {} '{}' (el actual va a .trash)", kind.label(), self.name));
                } else {
                    st.errors.push(format!("ya existe un {} '{}'", kind.label(), self.name));
                }
            } else {
                let what = owner.map(|k| format!("un {}", k.label())).unwrap_or_else(|| "un objeto o una room".into());
                st.errors.push(format!("el nombre '{}' ya lo usa {what} (todos los assets comparten nombres)", self.name));
            }
        }
        st
    }
}

/// One line of the file browser.
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub kind: Option<Kind>,
}

/// Folders (not hidden) and importable files of `dir`, folders first, by name.
pub fn list_folder(dir: &Path, filter: Option<Kind>) -> Result<Vec<Entry>, String> {
    let rd = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut out = vec![];
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let path = e.path();
        if path.is_dir() {
            out.push(Entry { name, path, is_dir: true, kind: None });
        } else if let Some(k) = Kind::of(&path).filter(|k| filter.is_none_or(|f| f == *k)) {
            out.push(Entry { name, path, is_dir: false, kind: Some(k) });
        }
    }
    out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(out)
}

/// `~/.config/modengine/import.txt`: the folder the browser opens in.
pub fn last_dir_file() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/modengine/import.txt"))
}

/// What the user did with the dialog this frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    Open,
    Cancel,
    Import,
}

pub struct ImportDialog {
    pub items: Vec<Item>,
    /// Folder shown in the browser.
    pub dir: PathBuf,
    /// The path field.
    pub path: String,
    /// Only files of this kind in the browser (opened from a browser section).
    pub filter: Option<Kind>,
    /// Problem with the path field or the folder.
    pub message: Option<String>,
    entries: Vec<Entry>,
    listed: Option<(PathBuf, Option<Kind>)>,
    picked: BTreeSet<PathBuf>,
    /// Where the last folder is remembered (`None` in tests).
    config: Option<PathBuf>,
}

impl ImportDialog {
    pub fn new(config: Option<PathBuf>) -> ImportDialog {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
        let dir = config.as_ref().and_then(|f| fs::read_to_string(f).ok()).map(|s| PathBuf::from(s.trim())).filter(|d| d.is_dir()).unwrap_or(home);
        ImportDialog {
            items: vec![],
            path: String::new(),
            dir,
            filter: None,
            message: None,
            entries: vec![],
            listed: None,
            picked: BTreeSet::new(),
            config,
        }
    }

    /// Shows `dir` in the browser and remembers it for next time.
    pub fn set_dir(&mut self, dir: &Path) {
        self.dir = dir.to_path_buf();
        self.picked.clear();
        self.listed = None;
        if let Some(f) = &self.config {
            if let Some(d) = f.parent() {
                let _ = fs::create_dir_all(d);
            }
            let _ = fs::write(f, dir.to_string_lossy().as_bytes());
        }
    }

    /// Adds a file to the list (a folder is opened in the browser instead). Its name is the
    /// file's, cleaned up and made unique.
    pub fn add(&mut self, path: &Path, owners: &BTreeMap<String, Option<Kind>>) {
        self.message = None;
        if path.is_dir() {
            self.set_dir(path);
            return;
        }
        if !path.exists() {
            self.message = Some(format!("no existe: {}", path.display()));
            return;
        }
        if self.items.iter().any(|i| i.src == path) {
            return;
        }
        let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let strip = strip_suffix(&stem).filter(|_| Kind::of(path) == Some(Kind::Sprite));
        let base = sanitize(strip.map(|s| s.0).unwrap_or(&stem));
        let name = unique(&base, |n| owners.contains_key(n) || self.items.iter().any(|i| i.name == n));
        let mut it = Item::new(path, name);
        if let (Some((_, n)), Some(Ok((w, _)))) = (strip, &it.image) {
            if n <= *w {
                it.mode = 1;
                it.strip = n;
            }
        }
        self.items.push(it);
        if let Some(parent) = path.parent().filter(|p| p.is_dir()) {
            if self.items.len() == 1 && self.dir != parent {
                self.set_dir(parent);
            }
        }
    }

    /// Statuses of every item, in order (duplicate names count against the later item).
    pub fn statuses(&self, owners: &BTreeMap<String, Option<Kind>>) -> Vec<Status> {
        let mut seen = BTreeSet::new();
        self.items
            .iter()
            .map(|it| {
                let st = it.status(owners, &seen);
                if it.include {
                    seen.insert(it.name.clone());
                }
                st
            })
            .collect()
    }

    /// Included items, ready to import (no errors), and how many are included.
    pub fn ready(&self, owners: &BTreeMap<String, Option<Kind>>) -> (bool, usize) {
        let st = self.statuses(owners);
        let n = self.items.iter().filter(|i| i.include).count();
        let ok = n > 0 && self.items.iter().zip(&st).all(|(i, s)| !i.include || s.errors.is_empty());
        (ok, n)
    }

    /// Imports the included items into `project`. Imported items leave the list; failures stay
    /// with their error.
    pub fn run(&mut self, project: &Path) -> Vec<Imported> {
        let mut done = vec![];
        let mut keep = vec![];
        for mut it in std::mem::take(&mut self.items) {
            let Some(kind) = it.kind.filter(|_| it.include) else {
                keep.push(it);
                continue;
            };
            match import(project, kind, &it.src, &it.name, &it.opts(), it.overwrite) {
                Ok(imp) => done.push(imp),
                Err(e) => {
                    it.failed = Some(e);
                    keep.push(it);
                }
            }
        }
        // Something failed: it stays (with what was left out) to fix and retry. Otherwise the
        // list empties and the dialog closes.
        if keep.iter().any(|i| i.failed.is_some()) {
            self.items = keep;
        }
        done
    }

    fn refresh(&mut self) {
        let key = (self.dir.clone(), self.filter);
        if self.listed.as_ref() == Some(&key) {
            return;
        }
        match list_folder(&self.dir, self.filter) {
            Ok(e) => {
                self.entries = e;
                if self.message.as_deref().is_some_and(|m| m.starts_with("carpeta:")) {
                    self.message = None;
                }
            }
            Err(e) => {
                self.entries.clear();
                self.message = Some(format!("carpeta: {e}"));
            }
        }
        self.listed = Some(key);
    }

    // ------------------------------------------------------------ ui

    pub fn show(&mut self, ctx: &egui::Context, owners: &BTreeMap<String, Option<Kind>>) -> Outcome {
        self.refresh();
        let mut out = Outcome::Open;
        let screen = ctx.content_rect().size();
        let width = (screen.x - 80.0).clamp(560.0, 1040.0);
        // the whole modal (title, columns, buttons) has to fit in the window
        let height = (screen.y - 230.0).clamp(200.0, 560.0);
        egui::Modal::new(egui::Id::unique("import_assets")).show(ctx, |ui| {
            ui.set_width(width);
            ui.heading("Importar assets");
            ui.label(RichText::new("PNG → sprites/   ·   WAV, OGG → sounds/   ·   TTF, OTF → fonts/   (se copian al proyecto)").weak());
            ui.add_space(4.0);
            ui.horizontal_top(|ui| {
                let left = (width * 0.36).max(260.0);
                ui.vertical(|ui| {
                    ui.set_width(left);
                    self.browser_ui(ui, owners, height);
                });
                // (a Separator here would grow to the modal's unbounded height)
                let (line, _) = ui.allocate_exact_size(Vec2::new(9.0, height), egui::Sense::hover());
                ui.painter().vline(line.center().x, line.y_range(), ui.visuals().widgets.noninteractive.bg_stroke);
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width());
                    self.items_ui(ui, owners, height);
                });
            });
            ui.separator();
            let (ok, n) = self.ready(owners);
            ui.horizontal(|ui| {
                let label = if n == 1 { "Importar 1 fichero".to_string() } else { format!("Importar {n} ficheros") };
                if ui.add_enabled(ok, crate::theme::primary_button(ui, &label)).clicked() {
                    out = Outcome::Import;
                }
                if ui.button("Cancelar").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    out = Outcome::Cancel;
                }
                if !ok && n > 0 {
                    ui.label(RichText::new("corrige los errores marcados en rojo").weak());
                }
            });
        });
        out
    }

    fn browser_ui(&mut self, ui: &mut egui::Ui, owners: &BTreeMap<String, Option<Kind>>, height: f32) {
        ui.horizontal(|ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut self.path).hint_text("ruta a un fichero o carpeta (~/…)").desired_width(ui.available_width() - 64.0));
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("Añadir").on_hover_text("un fichero va a la lista; una carpeta se abre abajo").clicked() || enter {
                let text = std::mem::take(&mut self.path);
                for line in text.lines().filter(|l| !l.trim().is_empty()) {
                    self.add(&expand(line), owners);
                }
            }
        });
        if let Some(m) = &self.message {
            ui.colored_label(crate::theme::colors(ui.ctx()).error, m);
        }
        ui.horizontal(|ui| {
            if ui.add_enabled(self.dir.parent().is_some(), egui::Button::new("⬆")).on_hover_text("carpeta de arriba").clicked() {
                if let Some(p) = self.dir.parent().map(Path::to_path_buf) {
                    self.set_dir(&p);
                }
            }
            if ui.button("⌂").on_hover_text("carpeta personal").clicked() {
                self.set_dir(&expand("~"));
            }
            let shown = self.filter.map(Kind::plural).unwrap_or("Todo lo importable");
            egui::ComboBox::from_id_salt("import_filter").selected_text(shown).width(ui.available_width() - 8.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut self.filter, None, "Todo lo importable");
                for k in Kind::ALL {
                    ui.selectable_value(&mut self.filter, Some(k), k.plural());
                }
            });
        });
        ui.label(RichText::new(self.dir.display().to_string()).small().weak());
        let mut enter = None;
        let mut add_now = vec![];
        egui::Frame::group(ui.style()).inner_margin(2).show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("import_browser").max_height(height - 130.0).min_scrolled_height(height - 130.0).auto_shrink([false, false]).show(ui, |ui| {
                if self.entries.is_empty() {
                    ui.label(RichText::new("(no hay carpetas ni ficheros importables)").weak());
                }
                for e in &self.entries {
                    let picked = self.picked.contains(&e.path);
                    let text = match e.kind {
                        None => format!("📁 {}", e.name),
                        Some(k) => format!("{} {}", k.icon(), e.name),
                    };
                    let r = ui.add(egui::Button::selectable(picked, text).frame(false));
                    if e.is_dir {
                        if r.clicked() {
                            enter = Some(e.path.clone());
                        }
                    } else {
                        if r.clicked() {
                            if !picked {
                                self.picked.insert(e.path.clone());
                            } else {
                                self.picked.remove(&e.path);
                            }
                        }
                        if r.double_clicked() {
                            add_now.push(e.path.clone());
                        }
                    }
                }
            });
        });
        if let Some(d) = enter {
            self.set_dir(&d);
        }
        ui.horizontal(|ui| {
            let n = self.picked.len();
            if ui.add_enabled(n > 0, egui::Button::new(format!("Añadir seleccionados ({n})"))).clicked() {
                add_now.extend(std::mem::take(&mut self.picked));
            }
            let files = self.entries.iter().filter(|e| !e.is_dir).count();
            if ui.add_enabled(files > 0, egui::Button::new(format!("Añadir todos ({files})"))).clicked() {
                add_now.extend(self.entries.iter().filter(|e| !e.is_dir).map(|e| e.path.clone()));
            }
        });
        ui.label(RichText::new("clic: seleccionar varios · doble clic: añadir").small().weak());
        for p in add_now {
            self.picked.remove(&p);
            self.add(&p, owners);
        }
    }

    fn items_ui(&mut self, ui: &mut egui::Ui, owners: &BTreeMap<String, Option<Kind>>, height: f32) {
        let statuses = self.statuses(owners);
        let pal = crate::theme::colors(ui.ctx());
        let warn = ui.visuals().warn_fg_color;
        let mut remove = None;
        egui::ScrollArea::vertical().id_salt("import_items").max_height(height).min_scrolled_height(height).auto_shrink([false, false]).show(ui, |ui| {
            if self.items.is_empty() {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Elige ficheros a la izquierda, pega una ruta\no arrastra ficheros a la ventana del editor.").weak());
                });
            }
            for (i, (it, st)) in self.items.iter_mut().zip(&statuses).enumerate() {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.add_enabled(it.kind.is_some(), egui::Checkbox::without_text(&mut it.include)).on_hover_text("importar este fichero");
                        let kind = it.kind.map(|k| format!("{} {}", k.icon(), k.label())).unwrap_or_else(|| "?".into());
                        ui.label(RichText::new(kind).strong());
                        ui.label("nombre:");
                        ui.add(egui::TextEdit::singleline(&mut it.name).desired_width(150.0).id_salt(("import_name", i)));
                        let mut info = format!("{} · {}", it.file_name(), human_size(it.bytes));
                        if let Some(Ok((w, h))) = it.image {
                            info += &format!(" · {w}×{h} px");
                        }
                        ui.label(RichText::new(info).weak()).on_hover_text(it.src.display().to_string());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("✕").on_hover_text("quitar de la lista").clicked() {
                                remove = Some(i);
                            }
                        });
                    });
                    if let Some(Ok((w, h))) = it.image {
                        sprite_options(ui, it, i, (w, h), st);
                    }
                    if st.can_overwrite {
                        ui.checkbox(&mut it.overwrite, format!("Sobrescribir el {} existente", it.kind.map(Kind::label).unwrap_or("asset")));
                    }
                    if it.include || it.kind.is_none() {
                        for e in &st.errors {
                            ui.colored_label(pal.error, format!("✖ {e}"));
                        }
                        for w in &st.warnings {
                            ui.colored_label(warn, format!("⚠ {w}"));
                        }
                    }
                    if let Some(e) = &it.failed {
                        ui.colored_label(pal.error, format!("✖ no se pudo importar: {e}"));
                    }
                });
            }
        });
        if let Some(i) = remove {
            self.items.remove(i);
        }
    }
}

/// Preview with the frame grid and the origin, and the sprite options.
fn sprite_options(ui: &mut egui::Ui, it: &mut Item, i: usize, (w, h): (u32, u32), st: &Status) {
    ui.horizontal_top(|ui| {
        let box_size = Vec2::new(150.0, 110.0);
        let (rect, _) = ui.allocate_exact_size(box_size, egui::Sense::hover());
        crate::tex::checkerboard(ui.painter(), rect, 8.0);
        if it.tex.is_none() {
            if let Some((size, rgba)) = &it.thumb {
                let img = egui::ColorImage::from_rgba_unmultiplied(*size, rgba);
                it.tex = Some(ui.ctx().load_texture(format!("import:{}", it.src.display()), img, TextureOptions::NEAREST));
            }
        }
        if let Some(t) = &it.tex {
            let k = (box_size.x / w as f32).min(box_size.y / h as f32);
            let img_rect = Rect::from_center_size(rect.center(), Vec2::new(w as f32, h as f32) * k);
            let p = ui.painter_at(rect);
            p.image(t.id(), img_rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            let line = Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 80, 200, 200));
            if let Some((n, fw, fh)) = st.slice.filter(|s| s.0 > 1) {
                let cols = if it.mode == 1 { n } else { w / fw.max(1) };
                let rows = if it.mode == 1 { 1 } else { h / fh.max(1) };
                for c in 0..=cols {
                    let x = img_rect.min.x + (c * fw) as f32 * k;
                    p.line_segment([Pos2::new(x, img_rect.min.y), Pos2::new(x, img_rect.min.y + (rows * fh) as f32 * k)], line);
                }
                for r in 0..=rows {
                    let y = img_rect.min.y + (r * fh) as f32 * k;
                    p.line_segment([Pos2::new(img_rect.min.x, y), Pos2::new(img_rect.min.x + (cols * fw) as f32 * k, y)], line);
                }
            }
            if let Some((_, fw, fh)) = st.slice {
                let (ox, oy) = it.origin.point(fw as f32, fh as f32);
                let o = img_rect.min + Vec2::new(ox, oy) * k;
                p.circle_stroke(o, 3.5, Stroke::new(1.5, Color32::from_rgb(255, 230, 0)));
            }
        }
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.label("Frames:");
                ui.radio_value(&mut it.mode, 0, "uno");
                ui.radio_value(&mut it.mode, 1, "tira horizontal");
                ui.radio_value(&mut it.mode, 2, "rejilla");
            });
            ui.horizontal(|ui| match it.mode {
                1 => {
                    ui.label("número de frames:");
                    ui.add(egui::DragValue::new(&mut it.strip).range(1..=w.max(1)));
                }
                2 => {
                    ui.label("tamaño del frame:");
                    ui.add(egui::DragValue::new(&mut it.grid[0]).range(1..=w.max(1)).suffix(" px"));
                    ui.label("×");
                    ui.add(egui::DragValue::new(&mut it.grid[1]).range(1..=h.max(1)).suffix(" px"));
                }
                _ => {
                    ui.label(RichText::new("la imagen entera es un frame").weak());
                    if it.strip > 1 && ui.small_button(format!("¿tira de {}?", it.strip)).on_hover_text("cuadrados del alto de la imagen").clicked() {
                        it.mode = 1;
                    }
                }
            });
            if let Some((n, fw, fh)) = st.slice {
                ui.label(RichText::new(format!("= {n} frame{} de {fw}×{fh} px", if n == 1 { "" } else { "s" })).weak());
            }
            ui.horizontal(|ui| {
                ui.label("Origen:");
                egui::ComboBox::from_id_salt(("import_origin", i)).selected_text(it.origin.label()).show_ui(ui, |ui| {
                    for o in Origin::ALL {
                        ui.selectable_value(&mut it.origin, o, o.label());
                    }
                });
                ui.label("fps:");
                ui.add(egui::DragValue::new(&mut it.fps).range(0.1..=120.0).speed(0.2).max_decimals(2));
            });
            let note = if it.opts().is_default() { "se copia el PNG suelto (sin .toml)" } else { "se copia el PNG y se escribe su .toml" };
            ui.label(RichText::new(note).small().weak());
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::room::tests::temp_project;

    fn png(path: &Path, w: u32, h: u32) {
        let mut img = image::RgbaImage::new(w, h);
        for (x, y, p) in img.enumerate_pixels_mut() {
            *p = image::Rgba([(x * 16) as u8, (y * 16) as u8, 200, 255]);
        }
        img.save(path).unwrap();
    }

    fn wav(path: &Path) {
        // 8 kHz mono 16-bit, 100 samples of silence
        let data = vec![0u8; 200];
        let mut b = b"RIFF".to_vec();
        b.extend((36 + data.len() as u32).to_le_bytes());
        b.extend(b"WAVEfmt ");
        b.extend(16u32.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(8000u32.to_le_bytes());
        b.extend(16000u32.to_le_bytes());
        b.extend(2u16.to_le_bytes());
        b.extend(16u16.to_le_bytes());
        b.extend(b"data");
        b.extend((data.len() as u32).to_le_bytes());
        b.extend(data);
        fs::write(path, b).unwrap();
    }

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("modeditor_import_{tag}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn names_are_clean_and_unique() {
        assert_eq!(sanitize("hero"), "hero");
        assert_eq!(sanitize("PlayerRun"), "player_run");
        assert_eq!(sanitize("Moneda dorada (2)"), "moneda_dorada_2");
        assert_eq!(sanitize("  --árbol-ñu--"), "arbol_nu");
        assert_eq!(sanitize("8bit"), "_8bit");
        assert_eq!(sanitize("###"), "asset");
        assert_eq!(sanitize("hero_strip.v2"), "hero_strip_v2");
        for s in ["PlayerRun", "Moneda dorada (2)", "8bit", "###", "a b c"] {
            assert!(valid_name(&sanitize(s)), "{s}");
        }
        assert_eq!(strip_suffix("hero_run_strip4"), Some(("hero_run", 4)));
        assert_eq!(strip_suffix("Hero_Strip12"), Some(("Hero", 12)));
        assert_eq!(strip_suffix("strip4"), None);
        assert_eq!(strip_suffix("hero_stripes"), None);
        let taken = ["coin", "coin_2"];
        assert_eq!(unique("coin", |n| taken.contains(&n)), "coin_3");
        assert_eq!(unique("gem", |n| taken.contains(&n)), "gem");
    }

    #[test]
    fn kind_by_extension() {
        assert_eq!(Kind::of(Path::new("a/hero.png")), Some(Kind::Sprite));
        assert_eq!(Kind::of(Path::new("HERO.PNG")), Some(Kind::Sprite));
        assert_eq!(Kind::of(Path::new("jump.wav")), Some(Kind::Sound));
        assert_eq!(Kind::of(Path::new("music.OGG")), Some(Kind::Sound));
        assert_eq!(Kind::of(Path::new("pixel.ttf")), Some(Kind::Font));
        assert_eq!(Kind::of(Path::new("pixel.otf")), Some(Kind::Font));
        assert_eq!(Kind::of(Path::new("photo.jpg")), None);
        assert_eq!(Kind::of(Path::new("song.mp3")), None);
        assert_eq!(Kind::of(Path::new("README")), None);
    }

    #[test]
    fn paths_expand() {
        let home = PathBuf::from(std::env::var("HOME").unwrap());
        assert_eq!(expand("~/x/a.png"), home.join("x/a.png"));
        assert_eq!(expand("~"), home);
        assert_eq!(expand(" '/tmp/a b.png' "), PathBuf::from("/tmp/a b.png"));
        assert_eq!(expand("file:///tmp/a%20b%C3%B1.png"), PathBuf::from("/tmp/a bñ.png"));
        assert_eq!(expand("file:///tmp/100%"), PathBuf::from("/tmp/100%"));
    }

    #[test]
    fn sprite_toml_only_when_needed() {
        assert_eq!(sprite_toml("hero.png", &SpriteOpts::default()), None);
        let strip = SpriteOpts { frames: Frames::Strip(4), ..Default::default() };
        assert_eq!(sprite_toml("hero.png", &strip).unwrap(), "image = \"hero.png\"\nframes = 4\n");
        let grid = SpriteOpts { frames: Frames::Grid(16, 8), origin: Origin::Bottom, fps: 12.0 };
        assert_eq!(sprite_toml("hero.png", &grid).unwrap(), "image = \"hero.png\"\nframe_size = [16, 8]\norigin = \"bottom\"\nfps = 12\n");
        let center = SpriteOpts { origin: Origin::Center, ..Default::default() };
        assert_eq!(sprite_toml("a.png", &center).unwrap(), "image = \"a.png\"\norigin = \"center\"\n");
        assert_eq!(strip.slice(64, 16), Ok((4, 16, 16)));
        assert_eq!(grid.slice(32, 16), Ok((4, 16, 8)));
        assert!(SpriteOpts { frames: Frames::Strip(100), ..Default::default() }.slice(64, 16).is_err());
        assert!(SpriteOpts { frames: Frames::Strip(3), ..Default::default() }.leftover(64, 16).is_some());
    }

    #[test]
    fn imported_sprites_load_in_the_engine() {
        let dir = temp_project("import_spr");
        let src = scratch("spr");
        png(&src.join("Hero Run.png"), 64, 16);
        png(&src.join("tiles.png"), 32, 32);
        png(&src.join("rock.png"), 10, 12);

        let strip = SpriteOpts { frames: Frames::Strip(4), origin: Origin::Bottom, fps: 12.0 };
        let a = import(&dir, Kind::Sprite, &src.join("Hero Run.png"), "hero_run", &strip, false).unwrap();
        assert_eq!(a.files, vec!["sprites/hero_run.png", "sprites/hero_run.toml"]);
        assert!(!a.replaced);
        let grid = SpriteOpts { frames: Frames::Grid(16, 16), origin: Origin::Center, fps: DEFAULT_FPS };
        import(&dir, Kind::Sprite, &src.join("tiles.png"), "tiles", &grid, false).unwrap();
        let b = import(&dir, Kind::Sprite, &src.join("rock.png"), "rock", &SpriteOpts::default(), false).unwrap();
        assert_eq!(b.files, vec!["sprites/rock.png"], "defaults: just the loose PNG");

        let p = modengine::project::Project::load(&dir).expect("the engine loads the project");
        let s = p.sprites.iter().find(|s| s.name == "hero_run").unwrap();
        assert_eq!((s.frames.len(), s.fw, s.fh, s.origin, s.fps), (4, 16, 16, (8.0, 16.0), 12.0));
        let t = p.sprites.iter().find(|s| s.name == "tiles").unwrap();
        assert_eq!((t.frames.len(), t.origin), (4, (8.0, 8.0)));
        let r = p.sprites.iter().find(|s| s.name == "rock").unwrap();
        assert_eq!((r.frames.len(), r.fw, r.fh, r.origin), (1, 10, 12, (0.0, 0.0)));
        modengine::Engine::load(&dir).expect("and runs it");

        // the editor sees them too
        let ep = Project::open(&dir).unwrap();
        assert_eq!(ep.sprite("hero_run").unwrap().frames.len(), 4);
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&src);
    }

    #[test]
    fn sounds_and_fonts() {
        let dir = temp_project("import_snd");
        let src = scratch("snd");
        wav(&src.join("Jump.WAV"));
        let mut font = vec![0u8, 1, 0, 0];
        font.extend([0u8; 60]);
        fs::write(src.join("Pixel Font.ttf"), &font).unwrap();
        fs::write(src.join("bad.wav"), b"not a wav at all").unwrap();

        let s = import(&dir, Kind::Sound, &src.join("Jump.WAV"), "snd_jump2", &SpriteOpts::default(), false).unwrap();
        assert_eq!(s.files, vec!["sounds/snd_jump2.wav"]);
        let f = import(&dir, Kind::Font, &src.join("Pixel Font.ttf"), "pixel_font", &SpriteOpts::default(), false).unwrap();
        assert_eq!(f.files, vec!["fonts/pixel_font.ttf"]);
        let p = modengine::project::Project::load(&dir).unwrap();
        assert!(p.sounds.iter().any(|s| s.name == "snd_jump2" && s.data.starts_with(b"RIFF")));
        assert!(p.fonts.iter().any(|f| f.name == "pixel_font" && f.size == 16.0));
        let ep = Project::open(&dir).unwrap();
        assert!(ep.sounds.iter().any(|s| s.name == "snd_jump2") && ep.fonts.iter().any(|f| f.name == "pixel_font"));

        // a .wav that isn't one is caught in the dialog
        let it = Item::new(&src.join("bad.wav"), "bad".into());
        assert!(!it.include);
        assert!(it.status(&BTreeMap::new(), &BTreeSet::new()).errors[0].contains("no parece"));
        // the wrong kind for the extension never imports
        assert!(import(&dir, Kind::Font, &src.join("Jump.WAV"), "x", &SpriteOpts::default(), false).is_err());
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&src);
    }

    #[test]
    fn conflicts_and_overwrite() {
        let dir = temp_project("import_conf");
        let src = scratch("conf");
        png(&src.join("coin.png"), 24, 8);
        let owners = owners(&Project::open(&dir).unwrap());

        // the dialog names it coin_2; typed back to "coin" it is a sprite conflict
        let mut d = ImportDialog::new(None);
        d.add(&src.join("coin.png"), &owners);
        assert_eq!(d.items[0].name, "coin_2");
        d.items[0].name = "coin".into();
        let st = &d.statuses(&owners)[0];
        assert!(st.can_overwrite && !st.errors.is_empty());
        assert!(!d.ready(&owners).0);
        d.items[0].overwrite = true;
        assert!(d.statuses(&owners)[0].errors.is_empty());
        // an object's name can't be replaced by a sprite
        d.items[0].name = "Player".into();
        let st = &d.statuses(&owners)[0];
        assert!(!st.can_overwrite && st.errors[0].contains("objeto"), "{st:?}");

        assert!(import(&dir, Kind::Sprite, &src.join("coin.png"), "coin", &SpriteOpts::default(), false).is_err());
        let three = SpriteOpts { frames: Frames::Strip(3), ..Default::default() };
        let r = import(&dir, Kind::Sprite, &src.join("coin.png"), "coin", &three, true).unwrap();
        assert!(r.replaced);
        assert!(dir.join(".trash/coin.toml").exists(), "the old pixel-art coin is in .trash");
        let p = modengine::project::Project::load(&dir).unwrap();
        let c = p.sprites.iter().find(|s| s.name == "coin").unwrap();
        assert_eq!((c.frames.len(), c.fw), (3, 8));
        // importing the project's own file over itself is refused
        assert!(import(&dir, Kind::Sprite, &dir.join("sprites/coin.png"), "coin", &three, true).is_err());
        assert!(dir.join("sprites/coin.png").exists());
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&src);
    }

    #[test]
    fn statuses_catch_problems() {
        let src = scratch("status");
        png(&src.join("a.png"), 16, 16);
        fs::write(src.join("photo.jpg"), b"jpeg").unwrap();
        fs::write(src.join("broken.png"), b"not a png").unwrap();
        let big = src.join("big.wav");
        wav(&big);
        fs::OpenOptions::new().write(true).open(&big).unwrap().set_len(BIG_FILE + 1).unwrap();
        let owners = BTreeMap::new();
        let mut d = ImportDialog::new(None);
        for f in ["a.png", "photo.jpg", "broken.png", "big.wav"] {
            d.add(&src.join(f), &owners);
        }
        d.add(&src.join("a.png"), &owners); // twice: once
        d.add(&src.join("missing.png"), &owners);
        assert!(d.message.as_deref().unwrap().contains("no existe"));
        assert_eq!(d.items.len(), 4);
        let st = d.statuses(&owners);
        assert!(st[0].errors.is_empty() && st[0].slice == Some((1, 16, 16)));
        assert!(st[1].errors[0].contains("no soportado") && !d.items[1].include);
        assert!(st[2].errors[0].contains("imagen") && !d.items[2].include);
        assert!(st[3].errors.is_empty() && st[3].warnings[0].contains("grande"));
        assert_eq!(d.ready(&owners), (true, 2), "the broken ones are left out");
        // two files can't take the same name
        d.items[3].name = "a".into();
        assert!(d.statuses(&owners)[3].errors[0].contains("lista"));
        // a strip wider than the image
        d.items[0].mode = 1;
        d.items[0].strip = 40;
        assert!(!d.statuses(&owners)[0].errors.is_empty());
        let _ = fs::remove_dir_all(&src);
    }

    #[test]
    fn folder_listing() {
        let src = scratch("list");
        fs::create_dir_all(src.join("Sub")).unwrap();
        fs::create_dir_all(src.join(".hidden")).unwrap();
        for f in ["b.png", "a.wav", "c.ttf", "notes.txt"] {
            fs::write(src.join(f), b"x").unwrap();
        }
        let names = |f| list_folder(&src, f).unwrap().into_iter().map(|e| e.name).collect::<Vec<_>>();
        assert_eq!(names(None), vec!["Sub", "a.wav", "b.png", "c.ttf"]);
        assert_eq!(names(Some(Kind::Sound)), vec!["Sub", "a.wav"]);
        // adding a folder opens it; the last folder is remembered
        let cfg = src.join("cfg/import.txt");
        let mut d = ImportDialog::new(Some(cfg.clone()));
        d.add(&src.join("Sub"), &BTreeMap::new());
        assert_eq!(d.dir, src.join("Sub"));
        assert_eq!(fs::read_to_string(&cfg).unwrap(), src.join("Sub").to_string_lossy());
        assert_eq!(ImportDialog::new(Some(cfg)).dir, src.join("Sub"));
        let _ = fs::remove_dir_all(&src);
    }

    #[test]
    fn os_drops_go_to_the_agent_only_over_its_panel() {
        let agent = Rect::from_min_max(Pos2::new(1000.0, 0.0), Pos2::new(1400.0, 800.0));
        assert_eq!(route_drop(Some(Pos2::new(1200.0, 300.0)), Some(agent), true), DropTarget::Agent);
        assert_eq!(route_drop(Some(Pos2::new(500.0, 300.0)), Some(agent), true), DropTarget::Import);
        assert_eq!(route_drop(Some(Pos2::new(1200.0, 300.0)), None, true), DropTarget::Import, "agent dock hidden");
        assert_eq!(route_drop(None, Some(agent), true), DropTarget::Import, "pointer unknown");
        assert_eq!(route_drop(Some(Pos2::new(500.0, 300.0)), None, false), DropTarget::Ignore);
    }

    #[test]
    fn the_agent_panel_rect_is_found() {
        use egui_kittest::Harness;
        let mut found = None;
        let mut h = Harness::builder().with_size([1000.0, 600.0]).build_ui(|ui| {
            egui::Panel::right("agent").default_size(300.0).show(ui, |ui| ui.label("agente"));
            found = panel_rect(ui, "agent");
        });
        h.run_steps(2);
        drop(h);
        let r = found.expect("panel state");
        assert!(r.max.x > 990.0 && r.contains(Pos2::new(950.0, 300.0)) && !r.contains(Pos2::new(300.0, 300.0)), "{r:?}");
    }

    #[test]
    fn dialog_renders_and_imports() {
        use egui_kittest::{kittest::Queryable, Harness};
        let dir = temp_project("import_ui");
        let src = scratch("ui");
        png(&src.join("hero_strip4.png"), 64, 16);
        wav(&src.join("jump.wav"));
        struct St {
            d: ImportDialog,
            owners: BTreeMap<String, Option<Kind>>,
            out: Outcome,
        }
        let owners = owners(&Project::open(&dir).unwrap());
        let mut d = ImportDialog::new(None);
        d.add(&src.join("hero_strip4.png"), &owners);
        d.add(&src.join("jump.wav"), &owners);
        assert_eq!((d.items[0].name.as_str(), d.items[0].mode, d.items[0].strip), ("hero", 1, 4), "_strip4: a strip of 4");
        let mut h = Harness::builder().with_size([1280.0, 860.0]).allow_missing_glyphs().with_accessibility_check(false).build_ui_state(
            |ui, s: &mut St| {
                let o = s.d.show(ui.ctx(), &s.owners);
                if o != Outcome::Open {
                    s.out = o;
                }
            },
            St { d, owners, out: Outcome::Open },
        );
        h.run_steps(3);
        h.get_by_label("Importar assets");
        h.get_by_label("= 4 frames de 16×16 px");
        h.get_by_label("tira horizontal");
        h.get_by_label("▣ hero_strip4.png"); // the browser opened the file's folder
        assert_eq!(h.state().d.ready(&h.state().owners), (true, 2), "{:?}", h.state().d.statuses(&h.state().owners));
        let b = h.get_by_label("Importar 2 ficheros");
        assert!(b.rect().max.y < 800.0, "the dialog fits in the window: {:?}", b.rect());
        b.click();
        h.run();
        assert_eq!(h.state().out, Outcome::Import);
        let done = h.state_mut().d.run(&dir);
        assert_eq!(done.len(), 2);
        assert!(h.state().d.items.is_empty());
        assert!(dir.join("sprites/hero.toml").exists() && dir.join("sounds/jump.wav").exists());
        let p = modengine::project::Project::load(&dir).unwrap();
        assert_eq!(p.sprites.iter().find(|s| s.name == "hero").unwrap().frames.len(), 4);
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&src);
    }

    #[test]
    fn the_dialog_and_the_command_line_write_the_same_files() {
        let (a, b) = (temp_project("import_same_a"), temp_project("import_same_b"));
        let src = scratch("same");
        png(&src.join("hero_strip4.png"), 64, 16);
        png(&src.join("rock.png"), 10, 12);
        wav(&src.join("jump.wav"));
        // the editor's dialog
        let owners = owners(&Project::open(&a).unwrap());
        let mut d = ImportDialog::new(None);
        for f in ["hero_strip4.png", "rock.png", "jump.wav"] {
            d.add(&src.join(f), &owners);
        }
        d.items[0].origin = Origin::Bottom;
        d.items[0].fps = 12.0;
        assert_eq!(d.run(&a).len(), 3);
        // `modengine import` (what the agent runs): the same choices as options
        let files: Vec<PathBuf> = ["hero_strip4.png", "rock.png", "jump.wav"].iter().map(|f| src.join(f)).collect();
        let hero = modengine::import::Options { origin: Some(Origin::Bottom), fps: Some(12.0), ..Default::default() };
        let plan = modengine::import::plan(&b, &files[..1], &hero);
        assert!(plan.errors.is_empty(), "{:?}", plan.errors);
        let r = modengine::import::execute(&plan, None);
        let rest = modengine::import::plan(&b, &files[1..], &Default::default());
        let r2 = modengine::import::execute(&rest, None);
        assert!(r.errors.is_empty() && r2.errors.is_empty());
        let names: Vec<&str> = r.done.iter().chain(&r2.done).map(|(it, _, _)| it.name.as_str()).collect();
        assert_eq!(names, ["hero", "rock", "jump"]);
        for f in ["sprites/hero.png", "sprites/hero.toml", "sprites/rock.png", "sounds/jump.wav"] {
            assert_eq!(fs::read(a.join(f)).unwrap(), fs::read(b.join(f)).unwrap(), "{f}");
        }
        assert!(!a.join("sprites/rock.toml").exists() && !b.join("sprites/rock.toml").exists());
        let _ = fs::remove_dir_all(&a);
        let _ = fs::remove_dir_all(&b);
        let _ = fs::remove_dir_all(&src);
    }
}
