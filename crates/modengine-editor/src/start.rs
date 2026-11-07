//! Start page (shown when no project is open, like GameMaker's) and the project
//! dialogs that the Archivo menu also uses:
//!
//! - big buttons: ＋ Nuevo proyecto, 📂 Abrir proyecto…, Abrir ejemplo;
//! - recent projects as cards (game name, folder, when it was opened, a thumbnail), with
//!   pin / remove, missing ones greyed out with Localizar…, and a search box;
//! - a path field (typed or pasted, `~` and `file://` work) and OS drops of a project
//!   folder or its game.toml;
//! - Nuevo proyecto: name, location, template, screen size and palette, with a live
//!   preview of the folder it creates and validation;
//! - Abrir proyecto: the built-in folder browser (agent_browser.rs) recognising projects,
//!   or the system dialog (`rfd`, run off the UI thread).
//!
//! The page never opens a project itself: it returns [`Action::Open`] and the app opens
//! it (asking to save unsaved changes first).

use crate::agent_browser::{Browser, Picked, Purpose};
use crate::projects::{self, Recents};
use eframe::egui::{self, Align, Color32, CornerRadius, Key, Layout, Rect, RichText, Sense, Stroke, StrokeKind, TextureHandle, TextureOptions, Vec2};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

#[derive(Debug, PartialEq)]
pub enum Action {
    Open(PathBuf),
}

/// What a running system dialog was opened for.
#[derive(Clone, Debug, PartialEq)]
enum NativeFor {
    Open,
    Location,
    Locate(PathBuf),
}

pub struct NewDialog {
    pub name: String,
    pub location: String,
    /// Index in `modengine::scaffold::TEMPLATES`.
    pub template: usize,
    /// 0 = the template's, else `projects::RESOLUTIONS[i - 1]`.
    pub resolution: usize,
    /// 0 = the template's, else `palettes[i - 1]`.
    pub palette: usize,
    palettes: Vec<crate::palette::Saved>,
    pub error: Option<String>,
    browser: Browser,
    focus: bool,
}

impl NewDialog {
    pub fn new(config: Option<&Path>) -> NewDialog {
        let browser = Browser::with_purpose(Purpose::PickFolder);
        NewDialog {
            name: String::new(),
            location: projects::default_location(config).display().to_string(),
            template: modengine::scaffold::TEMPLATES.iter().position(|t| t.id == "basic").unwrap_or(0),
            resolution: 0,
            palette: 0,
            palettes: crate::palette::list_saved(crate::palette::palettes_dir().as_deref()),
            error: None,
            browser,
            focus: true,
        }
    }

    /// The folder it would create, or why it can't.
    pub fn target(&self) -> Result<PathBuf, String> {
        projects::new_project_target(&self.location, &self.name)
    }

    pub fn options(&self) -> modengine::scaffold::NewProject {
        let t = &modengine::scaffold::TEMPLATES[self.template.min(modengine::scaffold::TEMPLATES.len() - 1)];
        modengine::scaffold::NewProject {
            template: t.id.into(),
            name: Some(self.name.trim().to_string()),
            display: self.resolution.checked_sub(1).and_then(|i| projects::RESOLUTIONS.get(i)).map(|(_, w, h, s)| (*w, *h, *s)),
            palette: self.palette.checked_sub(1).and_then(|i| self.palettes.get(i)).map(|s| s.pal.clone()),
        }
    }

    /// Creates the project; returns its folder.
    pub fn create(&mut self, config: Option<&Path>) -> Result<PathBuf, String> {
        let target = self.target()?;
        modengine::scaffold::create_with(&target, &self.options())?;
        if let Some(loc) = target.parent() {
            projects::remember_location(config, loc);
        }
        Ok(target)
    }
}

/// (width, height, scale) and palette name of a template's game.toml.
fn template_defaults(id: &str) -> (String, String) {
    let game = modengine::scaffold::template_files(id).and_then(|f| f.into_iter().find(|(r, _)| r == "game.toml")).map(|(_, b)| String::from_utf8_lossy(&b).to_string()).unwrap_or_default();
    let t: toml::Table = game.parse().unwrap_or_default();
    let d = t.get("display");
    let n = |k: &str, def: i64| d.and_then(|d| d.get(k)).and_then(|v| v.as_integer()).unwrap_or(def);
    let res = format!("{} × {} · ×{}", n("width", 320), n("height", 180), n("scale", 1));
    let pal = if game.contains("Pear36") { "Pear36".to_string() } else { format!("{} colores propios", t.get("palette").and_then(|p| p.as_table()).map(|p| p.len()).unwrap_or(0)) };
    (res, pal)
}

pub struct StartPage {
    /// The editor's config folder (`None` in tests: nothing is remembered on disk).
    pub config: Option<PathBuf>,
    pub recents: Recents,
    pub search: String,
    pub path_text: String,
    pub error: Option<String>,
    pub examples: Vec<(String, PathBuf)>,
    names: HashMap<PathBuf, Option<String>>,
    thumbs: HashMap<PathBuf, Option<TextureHandle>>,
    pub new_dialog: Option<NewDialog>,
    pub browser: Browser,
    favorites: Vec<PathBuf>,
    /// The recent project being located with the browser.
    locate: Option<PathBuf>,
    native: Option<(NativeFor, Receiver<Option<PathBuf>>)>,
    /// Offer the system dialog (off in tests and screenshots).
    pub native_ok: bool,
}

impl StartPage {
    pub fn new(config: Option<PathBuf>) -> StartPage {
        let recents = Recents::load(config.as_ref().map(|c| c.join("recent.txt")));
        let favorites = projects::load_favorites(config.as_deref());
        StartPage {
            config,
            recents,
            search: String::new(),
            path_text: String::new(),
            error: None,
            examples: projects::examples_in(&projects::example_roots()),
            names: HashMap::new(),
            thumbs: HashMap::new(),
            new_dialog: None,
            browser: Browser::with_purpose(Purpose::OpenProject),
            favorites,
            locate: None,
            native: None,
            native_ok: true,
        }
    }

    /// A project was opened: it goes first in the recent list.
    pub fn opened(&mut self, dir: &Path) {
        self.recents.touch(dir);
        self.names.remove(dir);
        self.thumbs.remove(dir);
        self.error = None;
    }

    /// The game's name of a recent project (`None` when it no longer exists).
    fn name_of(&mut self, dir: &Path) -> Option<String> {
        self.names.entry(dir.to_path_buf()).or_insert_with(|| projects::project_name(dir)).clone()
    }

    pub fn open_new_dialog(&mut self) {
        let mut d = NewDialog::new(self.config.as_deref());
        d.browser.native = self.native_ok;
        self.new_dialog = Some(d);
    }

    /// Abrir proyecto…: the built-in browser at the last project's folder.
    pub fn open_browser(&mut self, at: Option<&Path>) {
        let last = self.recents.paths().into_iter().find(|p| p.exists()).and_then(|p| p.parent().map(Path::to_path_buf));
        let start = at.map(Path::to_path_buf).or(last).or_else(projects::documents_dir);
        self.locate = None;
        self.browser.native = self.native_ok;
        self.browser.open_at(start.as_deref());
    }

    /// A path typed, pasted or dropped: a project folder or game.toml opens.
    pub fn open_path(&mut self, path: &Path) -> Option<Action> {
        match projects::project_dir(path) {
            Some(dir) => Some(Action::Open(dir)),
            None if !path.exists() => {
                self.error = Some(format!("No existe: {}", path.display()));
                None
            }
            None => {
                self.error = Some(format!("«{}» no es un proyecto de Mod Engine (no tiene game.toml)", path.display()));
                None
            }
        }
    }

    fn spawn_native(&mut self, ctx: &egui::Context, what: NativeFor, start: Option<PathBuf>) {
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let title = match what {
            NativeFor::Open => "Abrir proyecto de Mod Engine",
            NativeFor::Location => "Dónde crear el proyecto",
            NativeFor::Locate(_) => "Localizar el proyecto",
        };
        // rfd talks to the desktop portal (or the OS); it blocks, so never on the UI thread
        std::thread::spawn(move || {
            let mut d = rfd::FileDialog::new().set_title(title);
            if let Some(s) = start.filter(|s| s.is_dir()) {
                d = d.set_directory(s);
            }
            let _ = tx.send(d.pick_folder());
            ctx.request_repaint();
        });
        self.native = Some((what, rx));
    }

    // ------------------------------------------------------------------ the page

    /// The whole start page (the app shows it when no project is open).
    pub fn ui(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        let ctx = ui.ctx().clone();
        // Ctrl+N / Ctrl+Shift+N new, Ctrl+O open
        if self.new_dialog.is_none() && !self.browser.open {
            let (n, o) = ctx.input_mut(|i| (i.consume_key(egui::Modifiers::COMMAND, Key::N) || i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, Key::N), i.consume_key(egui::Modifiers::COMMAND, Key::O)));
            if n {
                self.open_new_dialog();
            }
            if o {
                self.open_browser(None);
            }
        }
        let frame = crate::theme::editor_frame(&ctx).inner_margin(egui::Margin::symmetric(32, 24));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            // header
            ui.horizontal(|ui| {
                ui.label(RichText::new(fa!(cube)).size(30.0).color(accent(ui)));
                ui.vertical(|ui| {
                    ui.label(RichText::new("Mod Engine").size(28.0).strong());
                    ui.label(RichText::new("Editor de juegos 2D: objetos, eventos y rooms").weak());
                });
            });
            ui.add_space(18.0);
            let h = ui.available_height();
            ui.horizontal_top(|ui| {
                // left column: what to do
                ui.allocate_ui_with_layout(Vec2::new(310.0, h), Layout::top_down(Align::Min), |ui| {
                    ui.set_width(310.0);
                    if big_button(ui, fa!(plus), "Nuevo proyecto", "Vacío, plataformas, top-down, showcase…", true).clicked() {
                        self.open_new_dialog();
                    }
                    ui.add_space(8.0);
                    if big_button(ui, fa!(folder_open), "Abrir proyecto…", "Busca la carpeta de un juego en el disco", false).clicked() {
                        self.open_browser(None);
                    }
                    ui.add_space(8.0);
                    let ex = big_button(ui, fa!(gamepad), "Abrir ejemplo", "Coin Climber y los showcases de Mod Engine", false);
                    egui::Popup::menu(&ex).show(|ui| {
                        ui.set_min_width(300.0);
                        if self.examples.is_empty() {
                            ui.label(RichText::new("No encuentro los ejemplos junto al editor").weak());
                        }
                        for (name, dir) in self.examples.clone() {
                            ui.menu_button(format!(concat!(fa!(gamepad), " {}"), name), |ui| {
                                ui.label(RichText::new(dir.display().to_string()).weak().small());
                                if ui.button("Abrir una copia (recomendado)").on_hover_text("Copia el ejemplo a tu carpeta de proyectos: puedes cambiarlo sin miedo").clicked() {
                                    let loc = projects::default_location(self.config.as_deref());
                                    let to = projects::free_folder(&loc, &projects::folder_name(&dir));
                                    match projects::copy_project(&dir, &to) {
                                        Ok(()) => action = Some(Action::Open(to)),
                                        Err(e) => self.error = Some(e),
                                    }
                                    ui.close();
                                }
                                if ui.button("Abrir el original").on_hover_text("Los cambios se pierden al actualizar Mod Engine si está en la carpeta de instalación").clicked() {
                                    action = Some(Action::Open(dir.clone()));
                                    ui.close();
                                }
                            });
                        }
                    });
                    ui.add_space(18.0);
                    ui.label(RichText::new("Abrir una ruta").weak().small());
                    ui.horizontal(|ui| {
                        let r = ui.add(egui::TextEdit::singleline(&mut self.path_text).hint_text("/ruta/a/mi_juego o pega file://…").desired_width(230.0));
                        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                        if (ui.button("Abrir").clicked() || enter) && !self.path_text.trim().is_empty() {
                            let p = crate::import::expand(&self.path_text);
                            action = action.take().or_else(|| self.open_path(&p));
                        }
                    });
                    ui.add_space(6.0);
                    ui.label(RichText::new("También puedes arrastrar a esta ventana la carpeta de un proyecto o su game.toml.").weak().small());
                    if let Some(e) = &self.error {
                        ui.add_space(8.0);
                        ui.label(RichText::new(e).color(crate::theme::colors(ui.ctx()).error));
                    }
                });
                ui.add_space(28.0);
                // right: recent projects
                ui.vertical(|ui| {
                    if let Some(a) = self.recent_cards(ui) {
                        action = Some(a);
                    }
                });
            });
        });
        // folders / game.toml dropped from the OS
        let (hovered, dropped) = ctx.input(|i| (i.raw.hovered_files.len(), i.raw.dropped_files.clone()));
        if hovered > 0 {
            drop_hint(&ctx);
        }
        let paths: Vec<PathBuf> = dropped.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect();
        if !paths.is_empty() {
            match projects::dropped_project(&paths) {
                Some(dir) => action = Some(Action::Open(dir)),
                None => self.error = Some("Suelta una sola carpeta de proyecto (o su game.toml) para abrirla".into()),
            }
        }
        action.or_else(|| self.dialogs(&ctx))
    }

    fn recent_cards(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Proyectos recientes").size(17.0).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.search).hint_text(concat!(fa!(magnifying_glass), " Buscar…")).desired_width(220.0));
            });
        });
        ui.add_space(6.0);
        let names: HashMap<PathBuf, Option<String>> = self.recents.list.clone().iter().map(|r| (r.path.clone(), self.name_of(&r.path))).collect();
        let list = self.recents.sorted(&self.search, |p| names.get(p).cloned().flatten().unwrap_or_default());
        if list.is_empty() {
            ui.add_space(30.0);
            ui.label(RichText::new(if self.recents.list.is_empty() { "Aún no has abierto ningún proyecto. Crea uno nuevo o abre un ejemplo." } else { "Ningún proyecto coincide con la búsqueda." }).weak());
            return None;
        }
        const W: f32 = 230.0;
        const H: f32 = 206.0;
        const GAP: f32 = 14.0;
        let cols = (((ui.available_width() + GAP) / (W + GAP)).floor() as usize).max(1);
        let now = projects::now_secs();
        egui::ScrollArea::vertical().id_salt("start_recents").auto_shrink([false, false]).show(ui, |ui| {
            for row in list.chunks(cols) {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = GAP;
                    for r in row {
                        let name = names.get(&r.path).cloned().flatten();
                        if let Some(a) = self.card(ui, r, name, now, Vec2::new(W, H)) {
                            action = Some(a);
                        }
                    }
                });
                ui.add_space(GAP);
            }
        });
        action
    }

    /// One recent project.
    fn card(&mut self, ui: &mut egui::Ui, r: &projects::Recent, name: Option<String>, now: u64, size: Vec2) -> Option<Action> {
        let mut action = None;
        let missing = name.is_none();
        let title = name.clone().unwrap_or_else(|| projects::folder_name(&r.path));
        let (rect, resp) = ui.allocate_exact_size(size, if missing { Sense::hover() } else { Sense::click() });
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, !missing, &title));
        let v = ui.visuals().clone();
        let hovered = resp.hovered() && !missing;
        let fill = if v.dark_mode { v.extreme_bg_color } else { crate::theme::CARD };
        let stroke = if hovered { Stroke::new(1.5, accent(ui)) } else { v.widgets.noninteractive.bg_stroke };
        let p = ui.painter();
        p.rect(rect, CornerRadius::same(8), fill, stroke, StrokeKind::Inside);
        // thumbnail
        let thumb = Rect::from_min_size(rect.min + Vec2::splat(8.0), Vec2::new(size.x - 16.0, 116.0));
        p.rect_filled(thumb, CornerRadius::same(5), crate::theme::colors(ui.ctx()).canvas);
        let tint = if missing { Color32::from_white_alpha(70) } else { Color32::WHITE };
        if let Some(tex) = self.thumb(ui.ctx(), &r.path) {
            let s = tex.size_vec2();
            let k = ((thumb.width() - 16.0) / s.x).min((thumb.height() - 16.0) / s.y);
            // pixel art scales by whole steps
            let k = if k >= 1.0 { k.floor() } else { k };
            p.image(tex.id(), Rect::from_center_size(thumb.center(), s * k), Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), tint);
        } else {
            p.text(thumb.center(), egui::Align2::CENTER_CENTER, if missing { fa!(triangle_exclamation) } else { fa!(cube) }, egui::FontId::proportional(34.0), v.weak_text_color());
        }
        let text_col = if missing { v.weak_text_color() } else { v.text_color() };
        let mut y = thumb.bottom() + 8.0;
        let galley = |s: &str, size: f32, col: Color32, strong: bool| {
            let mut job = egui::text::LayoutJob::single_section(s.to_string(), egui::TextFormat::simple(egui::FontId::proportional(size), col));
            job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - 20.0);
            if strong {
                job.sections[0].format.font_id = egui::FontId::new(size, egui::FontFamily::Proportional);
            }
            ui.fonts_mut(|f| f.layout_job(job))
        };
        let g = galley(&title, 14.0, text_col, true);
        let gh = g.size().y;
        p.galley(egui::pos2(rect.left() + 10.0, y), g, text_col);
        y += gh + 2.0;
        let g = galley(&r.path.display().to_string(), 11.0, v.weak_text_color(), false);
        let gh = g.size().y;
        p.galley(egui::pos2(rect.left() + 10.0, y), g, v.weak_text_color());
        y += gh + 2.0;
        let when = if missing { "No se encuentra la carpeta".to_string() } else { projects::ago(r.opened, now) };
        let col = if missing { crate::theme::colors(ui.ctx()).error } else { v.weak_text_color() };
        p.text(egui::pos2(rect.left() + 10.0, y), egui::Align2::LEFT_TOP, when, egui::FontId::proportional(11.0), col);
        let path_hover = r.path.display().to_string();
        let resp = resp.on_hover_text(path_hover);
        if resp.clicked() && !missing {
            action = Some(Action::Open(r.path.clone()));
        }
        // pin / remove (drawn after the card so they get the clicks)
        let tools = Rect::from_min_size(egui::pos2(thumb.right() - 58.0, thumb.top() + 4.0), Vec2::new(56.0, 22.0));
        let show_tools = r.pinned || ui.rect_contains_pointer(rect) || missing;
        if show_tools {
            // a child Ui that doesn't move the grid's cursor
            let ui = &mut ui.new_child(egui::UiBuilder::new().max_rect(tools).layout(Layout::right_to_left(Align::Center)));
            {
                if ui.small_button(fa!(xmark)).on_hover_text("Quitar de la lista (no borra nada)").clicked() {
                    self.recents.remove(&r.path);
                }
                let pin = if r.pinned { RichText::new(fa!(thumbtack)) } else { RichText::new(fa!(thumbtack)).weak() };
                if ui.small_button(pin).on_hover_text(if r.pinned { "Desfijar" } else { "Fijar arriba" }).clicked() {
                    self.recents.set_pinned(&r.path, !r.pinned);
                }
            }
        }
        if missing {
            let b = Rect::from_center_size(egui::pos2(thumb.center().x, thumb.bottom() - 20.0), Vec2::new(110.0, 24.0));
            let ui = &mut ui.new_child(egui::UiBuilder::new().max_rect(b).layout(Layout::centered_and_justified(egui::Direction::LeftToRight)));
            if ui.add(egui::Button::new(concat!(fa!(magnifying_glass), " Localizar…"))).on_hover_text("Busca dónde está ahora la carpeta del proyecto").clicked() {
                let start = r.path.ancestors().find(|a| a.is_dir()).map(Path::to_path_buf);
                self.open_browser(start.as_deref());
                self.locate = Some(r.path.clone());
            }
        }
        action
    }

    fn thumb(&mut self, ctx: &egui::Context, dir: &Path) -> Option<TextureHandle> {
        self.thumbs
            .entry(dir.to_path_buf())
            .or_insert_with(|| {
                let (size, px) = projects::thumbnail(dir)?;
                Some(ctx.load_texture(format!("thumb:{}", dir.display()), egui::ColorImage::from_rgba_unmultiplied(size, &px), TextureOptions::NEAREST))
            })
            .clone()
    }

    /// Archivo ▸ Abrir reciente.
    pub fn recent_menu(&mut self, ui: &mut egui::Ui, current: Option<&Path>) -> Option<Action> {
        let mut action = None;
        let list = self.recents.sorted("", |_| String::new());
        let mut shown = 0;
        for r in list.iter().filter(|r| Some(r.path.as_path()) != current).take(15) {
            let name = self.name_of(&r.path);
            let label = format!("{}{}", if r.pinned { concat!(fa!(thumbtack), " ") } else { "" }, name.clone().unwrap_or_else(|| projects::folder_name(&r.path)));
            if ui.add_enabled(name.is_some(), egui::Button::new(label)).on_hover_text(r.path.display().to_string()).clicked() {
                action = Some(Action::Open(r.path.clone()));
                ui.close();
            }
            shown += 1;
        }
        if shown == 0 {
            ui.label(RichText::new("(ninguno)").weak());
        }
        action
    }

    // ------------------------------------------------------------------ dialogs

    /// Nuevo proyecto, the project browser and the system dialog's answer. Called every
    /// frame, with or without a project open.
    pub fn dialogs(&mut self, ctx: &egui::Context) -> Option<Action> {
        let mut action = None;
        // the system dialog answered
        if let Some((what, rx)) = &self.native {
            if let Ok(res) = rx.try_recv() {
                let what = what.clone();
                self.native = None;
                if let Some(p) = res {
                    match what {
                        NativeFor::Open => action = self.open_path(&p),
                        NativeFor::Location => {
                            if let Some(d) = &mut self.new_dialog {
                                d.location = p.display().to_string();
                            }
                        }
                        NativeFor::Locate(old) => action = self.relocate(&old, &p),
                    }
                }
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
            }
        }
        // the project browser (Abrir proyecto / Localizar)
        let recent_dirs: Vec<PathBuf> = self.recents.paths().iter().filter_map(|p| p.parent().map(Path::to_path_buf)).fold(vec![], |mut v, p| {
            if !v.contains(&p) {
                v.push(p);
            }
            v
        });
        let before = self.favorites.clone();
        match self.browser.show(ctx, &mut self.favorites, &recent_dirs) {
            Picked::Attach(paths) => {
                if let Some(p) = paths.first() {
                    action = match self.locate.take() {
                        Some(old) => self.relocate(&old, p),
                        None => self.open_path(p),
                    };
                }
            }
            Picked::Native => {
                let what = self.locate.take().map(NativeFor::Locate).unwrap_or(NativeFor::Open);
                let at = Some(self.browser.dir.clone());
                self.spawn_native(ctx, what, at);
            }
            Picked::Cancel => self.locate = None,
            Picked::Nothing => {}
        }
        if self.favorites != before {
            projects::save_favorites(self.config.as_deref(), &self.favorites);
        }
        if let Some(a) = self.new_project_dialog(ctx) {
            action = Some(a);
        }
        action
    }

    /// Localizar…: the project now lives at `new`.
    fn relocate(&mut self, old: &Path, new: &Path) -> Option<Action> {
        let dir = projects::project_dir(new);
        match dir {
            Some(dir) => {
                self.recents.relocate(old, &dir);
                self.names.clear();
                self.thumbs.remove(old);
                self.error = None;
                None
            }
            None => {
                self.error = Some(format!("«{}» no es un proyecto de Mod Engine (no tiene game.toml)", new.display()));
                None
            }
        }
    }

    fn new_project_dialog(&mut self, ctx: &egui::Context) -> Option<Action> {
        let mut d = self.new_dialog.take()?;
        let mut action = None;
        let mut keep = true;
        let mut create = false;
        let mut native = false;
        let target = d.target();
        egui::Modal::new(egui::Id::unique("new_project_dialog")).show(ctx, |ui| {
            ui.set_width(660.0);
            ui.heading("Nuevo proyecto");
            ui.add_space(6.0);
            egui::Grid::new("new_project_grid").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
                ui.label("Nombre");
                let r = ui.add(egui::TextEdit::singleline(&mut d.name).hint_text("Mi juego").desired_width(420.0));
                if d.focus {
                    r.request_focus();
                    d.focus = false;
                }
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    create = true;
                }
                ui.end_row();

                ui.label("Ubicación");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut d.location).desired_width(330.0));
                    if ui.button(concat!(fa!(folder_open), " Elegir…")).on_hover_text("Elegir la carpeta donde se guardará").clicked() {
                        let at = crate::import::expand(&d.location);
                        let at = at.ancestors().find(|a| a.is_dir()).map(Path::to_path_buf);
                        d.browser.open_at(at.as_deref());
                    }
                });
                ui.end_row();

                ui.label("Plantilla");
                ui.horizontal_wrapped(|ui| {
                    for (i, t) in modengine::scaffold::TEMPLATES.iter().enumerate() {
                        if ui.selectable_label(d.template == i, t.label).on_hover_text(t.description).clicked() {
                            d.template = i;
                        }
                    }
                });
                ui.end_row();
                ui.label("");
                let t = &modengine::scaffold::TEMPLATES[d.template];
                ui.label(RichText::new(t.description).weak().small());
                ui.end_row();

                let (res, pal) = template_defaults(t.id);
                ui.label("Pantalla");
                let res_label = |i: usize| if i == 0 { format!("La de la plantilla ({res})") } else { projects::RESOLUTIONS[i - 1].0.to_string() };
                egui::ComboBox::from_id_salt("new_project_res").width(300.0).selected_text(res_label(d.resolution)).show_ui(ui, |ui| {
                    for i in 0..=projects::RESOLUTIONS.len() {
                        ui.selectable_value(&mut d.resolution, i, res_label(i));
                    }
                });
                ui.end_row();

                ui.label("Paleta");
                ui.vertical(|ui| {
                    let names: Vec<String> = d.palettes.iter().map(|s| format!("{} ({} colores)", s.pal.name, s.pal.colors.len())).collect();
                    let pal_label = |i: usize| if i == 0 { format!("La de la plantilla ({pal})") } else { names[i - 1].clone() };
                    egui::ComboBox::from_id_salt("new_project_palette").width(300.0).selected_text(pal_label(d.palette)).show_ui(ui, |ui| {
                        for i in 0..=names.len() {
                            ui.selectable_value(&mut d.palette, i, pal_label(i));
                        }
                    });
                    let colors = match d.palette.checked_sub(1).and_then(|i| d.palettes.get(i)) {
                        Some(s) => s.pal.colors.clone(),
                        None if pal == "Pear36" => modengine::palette::PEAR36.to_vec(),
                        None => vec![],
                    };
                    if !colors.is_empty() {
                        crate::palette::strip(ui, &colors, 300.0, 10.0);
                    }
                    if d.palette > 0 {
                        ui.label(RichText::new("Los sprites de la plantilla se pasan al color más parecido de la paleta.").weak().small());
                    }
                });
                ui.end_row();
            });
            ui.add_space(8.0);
            ui.separator();
            let pal = crate::theme::colors(ui.ctx());
            match &target {
                Ok(p) => {
                    ui.label(RichText::new("Se creará la carpeta:").weak().small());
                    ui.label(RichText::new(p.display().to_string()).monospace());
                    ui.label(RichText::new(concat!(fa!(check), " Todo listo")).color(pal.ok).small());
                }
                Err(e) => {
                    let preview = crate::import::expand(&d.location).join(d.name.trim());
                    ui.label(RichText::new("Se creará la carpeta:").weak().small());
                    ui.label(RichText::new(preview.display().to_string()).monospace().weak());
                    ui.label(RichText::new(format!(concat!(fa!(xmark), " {}"), e)).color(pal.error).small());
                }
            }
            if let Some(e) = &d.error {
                ui.label(RichText::new(e).color(pal.error));
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.add_enabled(target.is_ok(), crate::theme::primary_button(ui, "Crear proyecto")).clicked() {
                    create = true;
                }
                if ui.button("Cancelar").clicked() {
                    keep = false;
                }
            });
        });
        if ctx.input(|i| i.key_pressed(Key::Escape)) && !d.browser.open {
            keep = false;
        }
        // the folder picker for Ubicación
        let mut favs = self.favorites.clone();
        match d.browser.show(ctx, &mut favs, &[]) {
            Picked::Attach(p) => {
                if let Some(p) = p.first() {
                    d.location = p.display().to_string();
                }
            }
            Picked::Native => native = true,
            _ => {}
        }
        if create && target.is_ok() {
            match d.create(self.config.as_deref()) {
                Ok(dir) => {
                    action = Some(Action::Open(dir));
                    keep = false;
                }
                Err(e) => d.error = Some(e),
            }
        }
        let at = Some(crate::import::expand(&d.location));
        if keep {
            self.new_dialog = Some(d);
        }
        if native {
            self.spawn_native(ctx, NativeFor::Location, at);
        }
        action
    }
}

fn accent(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        ui.visuals().selection.stroke.color
    } else {
        crate::theme::ACCENT
    }
}

/// A big start page button: icon, title and a line of help.
fn big_button(ui: &mut egui::Ui, icon: &str, title: &str, help: &str, primary: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 62.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, title));
    let v = ui.visuals();
    let acc = accent(ui);
    let (fill, fg, sub) = if primary {
        // white text needs a deep blue in the dark theme too
        let base = if v.dark_mode { Color32::from_rgb(38, 86, 196) } else { acc };
        let f = if resp.hovered() { base.gamma_multiply(0.9) } else { base };
        (f, Color32::WHITE, Color32::from_white_alpha(210))
    } else {
        let f = if resp.hovered() { v.widgets.hovered.weak_bg_fill } else if v.dark_mode { v.widgets.inactive.weak_bg_fill } else { crate::theme::CARD };
        (f, v.text_color(), v.weak_text_color())
    };
    let stroke = if primary { Stroke::NONE } else if resp.hovered() { Stroke::new(1.0, acc) } else { v.widgets.inactive.bg_stroke };
    let p = ui.painter();
    p.rect(rect, CornerRadius::same(8), fill, stroke, StrokeKind::Inside);
    p.text(egui::pos2(rect.left() + 26.0, rect.center().y), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(24.0), fg);
    p.text(egui::pos2(rect.left() + 50.0, rect.top() + 12.0), egui::Align2::LEFT_TOP, title, egui::FontId::proportional(16.0), fg);
    p.text(egui::pos2(rect.left() + 50.0, rect.top() + 34.0), egui::Align2::LEFT_TOP, help, egui::FontId::proportional(11.5), sub);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// "Suelta para abrir" over the whole window while files are dragged over it.
fn drop_hint(ctx: &egui::Context) {
    let rect = ctx.content_rect();
    let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("start_drop_hint")));
    p.rect_filled(rect, 0.0, Color32::from_black_alpha(90));
    p.text(rect.center(), egui::Align2::CENTER_CENTER, "Suelta la carpeta del proyecto (o su game.toml) para abrirlo", egui::FontId::proportional(22.0), Color32::WHITE);
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;
    use egui_kittest::Harness;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("modeditor_start_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    struct St {
        page: StartPage,
        actions: Vec<Action>,
    }

    fn harness(page: StartPage) -> Harness<'static, St> {
        Harness::builder().with_size([1280.0, 800.0]).allow_missing_glyphs().with_accessibility_check(false).build_ui_state(
            |ui, s: &mut St| {
                if let Some(a) = s.page.ui(ui) {
                    s.actions.push(a);
                }
            },
            St { page, actions: vec![] },
        )
    }

    fn page(config: &Path) -> StartPage {
        let mut p = StartPage::new(Some(config.to_path_buf()));
        p.native_ok = false;
        p
    }

    #[test]
    fn start_page_shows_recents_and_opens_them() {
        let d = temp("recents");
        let cfg = d.join("cfg");
        let game = d.join("mi_juego");
        modengine::scaffold::create_with(&game, &modengine::scaffold::NewProject { name: Some("Juego de prueba".into()), ..Default::default() }).unwrap();
        let mut p = page(&cfg);
        p.recents.touch(&game);
        p.recents.touch_at(&d.join("borrado"), 1);
        let mut h = harness(p);
        h.run_steps(3);
        for l in ["Nuevo proyecto", "Abrir proyecto…", "Abrir ejemplo", "Proyectos recientes"] {
            h.get_by_label(l);
        }
        h.get_by_label(concat!(fa!(magnifying_glass), " Localizar…"));
        // the search filters the cards
        h.state_mut().page.search = "prueba".into();
        h.run_steps(2);
        assert!(h.query_by_label("borrado").is_none());
        h.state_mut().page.search.clear();
        h.run_steps(2);
        // clicking a card opens the project
        h.get_by_label("Juego de prueba").click();
        h.run_steps(2);
        assert_eq!(h.state().actions, vec![Action::Open(modengine::canonical(&game).unwrap())]);
        // the missing one is removed with ×
        h.state_mut().page.recents.list.retain(|r| r.path.exists());
        h.run_steps(2);
        assert!(h.query_by_label(concat!(fa!(magnifying_glass), " Localizar…")).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn new_project_dialog_creates_and_opens() {
        let d = temp("new");
        let cfg = d.join("cfg");
        let mut h = harness(page(&cfg));
        h.run_steps(2);
        h.get_by_label("Nuevo proyecto").click();
        h.run_steps(3);
        assert!(h.state().page.new_dialog.is_some());
        h.get_by_label("Plataformas").click();
        h.run_steps(2);
        // no name yet: the button is disabled and the reason shown
        h.get_by_label_contains("Escribe un nombre");
        {
            let dlg = h.state_mut().page.new_dialog.as_mut().unwrap();
            dlg.location = d.join("proyectos").display().to_string();
            dlg.name = "Mi Plataformas".into();
            dlg.resolution = 2;
        }
        h.run_steps(2);
        h.get_by_label_contains("Todo listo");
        h.get_by_label("Crear proyecto").click();
        h.run_steps(3);
        let dir = d.join("proyectos/Mi Plataformas");
        assert_eq!(h.state().actions, vec![Action::Open(dir.clone())]);
        assert!(h.state().page.new_dialog.is_none());
        let game = std::fs::read_to_string(dir.join("game.toml")).unwrap();
        assert!(game.contains("name = \"Mi Plataformas\"") && game.contains("width = 480"), "{game}");
        assert!(modengine::Engine::load(&dir).is_ok());
        // the location is remembered for next time
        assert_eq!(projects::default_location(Some(&cfg)), d.join("proyectos"));
        // creating it again is refused before writing anything
        h.state_mut().page.open_new_dialog();
        h.state_mut().page.new_dialog.as_mut().unwrap().name = "Mi Plataformas".into();
        h.run_steps(2);
        h.get_by_label_contains("ya hay un proyecto");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn location_picker_opens_above_the_new_project_dialog() {
        let d = temp("pick_above");
        let target = d.join("juegos");
        std::fs::create_dir_all(&target).unwrap();
        let mut h = harness(page(&d.join("cfg")));
        h.run_steps(2);
        h.get_by_label("Nuevo proyecto").click();
        h.run_steps(3);
        h.state_mut().page.new_dialog.as_mut().unwrap().location = target.display().to_string();
        h.run_steps(2);
        h.get_by_label(concat!(fa!(folder_open), " Elegir…")).click();
        h.run_steps(3);
        // the picker's own button must take the click, not the modal behind it
        h.get_by_label("Elegir «juegos»").click();
        h.run_steps(3);
        let dlg = h.state().page.new_dialog.as_ref().expect("the new project dialog stays open");
        assert!(!dlg.browser.open, "the picker closed after choosing");
        assert_eq!(PathBuf::from(&dlg.location), target);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn browser_recognises_projects_and_paths_open() {
        let d = temp("browse");
        let cfg = d.join("cfg");
        modengine::scaffold::create_with(&d.join("uno"), &modengine::scaffold::NewProject { name: Some("Juego Uno".into()), ..Default::default() }).unwrap();
        std::fs::create_dir_all(d.join("carpeta")).unwrap();
        std::fs::write(d.join("nota.txt"), "x").unwrap();
        let mut h = harness(page(&cfg));
        h.state_mut().page.open_browser(Some(&d));
        h.run_steps(3);
        h.get_by_label_contains(concat!(fa!(gamepad), " uno"));
        h.get_by_label_contains(concat!(fa!(folder), " carpeta"));
        assert!(h.query_by_label_contains("nota.txt").is_none(), "only folders and game.toml");
        // pick the project row, then Abrir proyecto
        h.get_by_label_contains(concat!(fa!(gamepad), " uno")).click();
        h.run_steps(2);
        // (the window title says Abrir proyecto too)
        h.get_by_role_and_label(egui::Role::Button, "Abrir proyecto").click();
        h.run_steps(2);
        assert_eq!(h.state().actions, vec![Action::Open(d.join("uno"))]);
        // a pasted game.toml path, and something that isn't a project
        let mut p = page(&cfg);
        assert_eq!(p.open_path(&d.join("uno/game.toml")), Some(Action::Open(d.join("uno"))));
        assert_eq!(p.open_path(&d.join("carpeta")), None);
        assert!(p.error.as_deref().unwrap().contains("no es un proyecto"));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The whole editor: Nuevo proyecto on the start page creates the project and the
    /// editor opens it; `modeditor <dir>/game.toml` opens the folder.
    #[test]
    fn editor_creates_and_opens_a_project() {
        let d = temp("app");
        let cfg = d.join("cfg");
        projects::remember_location(Some(&cfg), &d.join("proyectos"));
        // screenshot mode: no update checks, no layout files written to ~/.config
        let auto = || {
            let mut a = crate::app::Auto::default();
            a.config_dir = Some(cfg.clone());
            a.screenshot = Some(d.join("unused.png"));
            a
        };
        let mut h = Harness::builder().with_size([1400.0, 860.0]).allow_missing_glyphs().with_accessibility_check(false).build_eframe(|cc| crate::app::EditorApp::new(cc, None, auto()));
        h.run_steps(3);
        assert!(h.state().project_dir().is_none());
        h.get_by_label("Nuevo proyecto").click();
        h.run_steps(3);
        // the name field has the focus
        h.event(egui::Event::Text("Juego App".into()));
        h.run_steps(2);
        h.get_by_label("Top-down").click();
        h.run_steps(2);
        h.get_by_label_contains("Todo listo");
        h.get_by_label("Crear proyecto").click();
        h.run_steps(4);
        let dir = d.join("proyectos/Juego App");
        assert_eq!(h.state().project_dir(), Some(dir.as_path()));
        assert!(dir.join("objects/player.ms").exists());
        // the menu bar of the open project is there, with the project actions
        h.get_by_label("Archivo").click();
        h.run_steps(2);
        h.get_by_label_contains("Abrir proyecto…");
        h.get_by_label("Abrir reciente");
        h.get_by_label("Cerrar proyecto").click();
        h.run_steps(3);
        assert!(h.state().project_dir().is_none(), "back to the start page");
        // it is the first recent project now
        h.get_by_label("Juego App");
        drop(h);

        let h = Harness::builder().with_size([1400.0, 860.0]).allow_missing_glyphs().with_accessibility_check(false).build_eframe(|cc| crate::app::EditorApp::new(cc, Some(dir.join("game.toml")), auto()));
        assert_eq!(h.state().project_dir(), Some(dir.as_path()));
        drop(h);
        let _ = std::fs::remove_dir_all(&d);
    }
}
