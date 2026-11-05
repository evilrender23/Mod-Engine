//! Editor shell: menu, asset browser, tabs, output panel, running the game.

use crate::object::ObjectEditor;
use crate::project::Project;
use crate::room::RoomEditor;
use crate::script::ScriptEditor;
use crate::settings::{write_game_toml, SettingsEditor};
use crate::sound::SoundEditor;
use crate::sprite::SpriteEditor;
use crate::tex::TexCache;
use crate::tomlw::valid_name;
use eframe::egui::{self, Key, Modifiers, RichText, Sense, Vec2};
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};

// ------------------------------------------------------------------ shared context

#[derive(Clone, Copy, PartialEq)]
pub enum LogKind {
    Info,
    Ok,
    Error,
    Game,
}

pub struct LogLine {
    pub kind: LogKind,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Open {
    Room(String),
    Object(String),
    Sprite(String),
    Sound(String),
    Script(String),
    Settings,
}

impl Open {
    /// `room:level1`, `object:Player`, `sprite:coin`, `sound:snd_jump`, `script:scripts/a.ms`, `settings`.
    pub fn parse(s: &str) -> Option<Open> {
        let (k, v) = s.split_once(':').unwrap_or((s, ""));
        Some(match k {
            "room" => Open::Room(v.into()),
            "object" => Open::Object(v.into()),
            "sprite" => Open::Sprite(v.into()),
            "sound" => Open::Sound(v.into()),
            "script" => Open::Script(v.into()),
            "settings" => Open::Settings,
            _ => return None,
        })
    }
}

/// Scripted startup used for automated screenshots.
#[derive(Default)]
pub struct Auto {
    pub open: Vec<Open>,
    pub screenshot: Option<PathBuf>,
    /// Select something in the opened editors (first free instance, the variables pane).
    pub select: bool,
    frames: u32,
    done: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// What every editor gets while drawing.
pub struct Cx<'a> {
    pub project: &'a mut Project,
    pub tex: &'a mut TexCache,
    pub log: &'a mut Vec<LogLine>,
    pub open: &'a mut Vec<Open>,
}

impl Cx<'_> {
    pub fn error(&mut self, s: impl Into<String>) {
        self.log.push(LogLine { kind: LogKind::Error, text: s.into() });
    }
}

// ------------------------------------------------------------------ tabs

pub enum Tab {
    Room(RoomEditor),
    Object(ObjectEditor),
    Sprite(SpriteEditor),
    Sound(SoundEditor),
    Script(ScriptEditor),
    Settings(SettingsEditor),
}

impl Tab {
    fn open_key(&self) -> Open {
        match self {
            Tab::Room(e) => Open::Room(e.name.clone()),
            Tab::Object(e) => Open::Object(e.name.clone()),
            Tab::Sprite(e) => Open::Sprite(e.name.clone()),
            Tab::Sound(e) => Open::Sound(e.name.clone()),
            Tab::Script(e) => Open::Script(e.rel.clone()),
            Tab::Settings(_) => Open::Settings,
        }
    }

    fn title(&self) -> String {
        match self {
            Tab::Room(e) => format!("▦ {}", e.name),
            Tab::Object(e) => format!("◆ {}", e.name),
            Tab::Sprite(e) => format!("▣ {}", e.name),
            Tab::Sound(e) => format!("♪ {}", e.name),
            Tab::Script(e) => format!("✎ {}", e.rel),
            Tab::Settings(_) => "⚙ Juego".into(),
        }
    }

    fn dirty(&self, p: &Project) -> bool {
        match self {
            Tab::Room(e) => e.dirty,
            Tab::Sprite(e) => e.dirty,
            Tab::Sound(e) => e.dirty,
            Tab::Settings(e) => e.dirty,
            Tab::Object(e) => e.script_rel(p).and_then(|r| p.script_index(&r)).map(|i| p.scripts[i].dirty()).unwrap_or(false),
            Tab::Script(e) => p.script_index(&e.rel).map(|i| p.scripts[i].dirty()).unwrap_or(false),
        }
    }

    fn save(&mut self, cx: &mut Cx) -> Result<(), String> {
        match self {
            Tab::Room(e) => e.save(cx),
            Tab::Sprite(e) => e.save(cx),
            Tab::Sound(e) => e.save(cx),
            Tab::Settings(e) => e.save(cx),
            Tab::Object(e) => match e.script_rel(cx.project).and_then(|r| cx.project.script_index(&r)) {
                Some(i) => cx.project.save_script(i),
                None => Ok(()),
            },
            Tab::Script(e) => match cx.project.script_index(&e.rel) {
                Some(i) => cx.project.save_script(i),
                None => Ok(()),
            },
        }
    }

    /// Throws away unsaved changes.
    fn revert(&mut self, p: &mut Project) {
        let rel = match self {
            Tab::Object(e) => e.script_rel(p),
            Tab::Script(e) => Some(e.rel.clone()),
            _ => None,
        };
        if let Some(i) = rel.and_then(|r| p.script_index(&r)) {
            let saved = p.scripts[i].saved.clone();
            p.set_script_text(i, saved);
        }
    }

    fn undo(&mut self) {
        match self {
            Tab::Room(e) => e.undo(),
            Tab::Sprite(e) => e.undo(),
            _ => {}
        }
    }

    fn redo(&mut self) {
        match self {
            Tab::Room(e) => e.redo(),
            Tab::Sprite(e) => e.redo(),
            _ => {}
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        match self {
            Tab::Room(e) => e.ui(ui, cx),
            Tab::Object(e) => e.ui(ui, cx),
            Tab::Sprite(e) => e.ui(ui, cx),
            Tab::Sound(e) => e.ui(ui, cx),
            Tab::Script(e) => e.ui(ui, cx),
            Tab::Settings(e) => e.ui(ui, cx),
        }
    }
}

// ------------------------------------------------------------------ dialogs

#[derive(Clone, Copy, PartialEq, Debug)]
enum NewKind {
    PixelSprite,
    PngSprite,
    Sound,
    Object,
    Room,
    Script,
}

enum Dialog {
    New { kind: NewKind, name: String, extra: String, error: Option<String> },
    Delete { what: Open },
    CloseTab { idx: usize },
    Quit,
}

struct Runner {
    child: Child,
    rx: Receiver<String>,
}

#[derive(Default)]
struct Start {
    path: String,
    new_path: String,
    error: Option<String>,
    recent: Vec<String>,
}

pub struct EditorApp {
    project: Option<Project>,
    tabs: Vec<Tab>,
    active: usize,
    tex: TexCache,
    log: Vec<LogLine>,
    runner: Option<Runner>,
    checks: Vec<Receiver<Vec<LogLine>>>,
    start: Start,
    dialog: Option<Dialog>,
    folded: HashMap<&'static str, bool>,
    filter: String,
    quitting: bool,
    auto: Auto,
    theme: crate::theme::Theme,
}

fn recent_file() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/modengine/recent.txt"))
}

fn load_recent() -> Vec<String> {
    recent_file().and_then(|f| std::fs::read_to_string(f).ok()).map(|t| t.lines().filter(|l| !l.is_empty()).map(String::from).collect()).unwrap_or_default()
}

fn remember_recent(path: &Path) {
    let Some(f) = recent_file() else { return };
    let p = path.canonicalize().unwrap_or(path.to_path_buf()).to_string_lossy().to_string();
    let mut list = load_recent();
    list.retain(|x| *x != p);
    list.insert(0, p);
    list.truncate(10);
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(f, list.join("\n"));
}

impl EditorApp {
    pub fn new(cc: &eframe::CreationContext, path: Option<PathBuf>, auto: Auto) -> Self {
        let theme = crate::theme::load();
        crate::theme::install_fonts(&cc.egui_ctx);
        crate::theme::apply(&cc.egui_ctx, theme);
        let mut app = EditorApp {
            project: None,
            tabs: vec![],
            active: 0,
            tex: TexCache::default(),
            log: vec![],
            runner: None,
            checks: vec![],
            start: Start { recent: load_recent(), ..Default::default() },
            dialog: None,
            folded: HashMap::new(),
            filter: String::new(),
            quitting: false,
            auto,
            theme,
        };
        if let Some(p) = path {
            app.open_project(&p);
        }
        app
    }

    fn info(&mut self, kind: LogKind, s: impl Into<String>) {
        self.log.push(LogLine { kind, text: s.into() });
    }

    fn open_project(&mut self, path: &Path) {
        match Project::open(path) {
            Ok(p) => {
                remember_recent(path);
                self.tabs.clear();
                self.tex = TexCache::default();
                self.log.clear();
                self.info(LogKind::Ok, format!("proyecto abierto: {}", path.display()));
                if let Some(e) = &p.game_error {
                    self.info(LogKind::Error, format!("game.toml: {e}"));
                }
                for s in &p.sprites {
                    if let Err(e) = &s.data {
                        self.log.push(LogLine { kind: LogKind::Error, text: e.clone() });
                    }
                }
                self.project = Some(p);
                self.start.error = None;
                self.check(false);
            }
            Err(e) => self.start.error = Some(e),
        }
    }

    fn dirty_any(&self) -> bool {
        let Some(p) = &self.project else { return false };
        p.dirty_scripts() || self.tabs.iter().any(|t| t.dirty(p))
    }

    fn save_all(&mut self) {
        let Some(project) = &mut self.project else { return };
        let mut open = vec![];
        let mut errors = vec![];
        {
            let mut cx = Cx { project, tex: &mut self.tex, log: &mut self.log, open: &mut open };
            for t in &mut self.tabs {
                if t.dirty(cx.project) {
                    if let Err(e) = t.save(&mut cx) {
                        errors.push(e);
                    }
                }
            }
            for i in 0..cx.project.scripts.len() {
                if cx.project.scripts[i].dirty() {
                    if let Err(e) = cx.project.save_script(i) {
                        errors.push(e);
                    }
                }
            }
        }
        for e in errors {
            self.info(LogKind::Error, e);
        }
        self.info(LogKind::Ok, "guardado");
        self.check(false);
    }

    /// Loads the project with the real engine in the background and reports problems.
    /// With `run_steps`, also runs 300 steps without a window.
    fn check(&mut self, run_steps: bool) {
        let Some(p) = &self.project else { return };
        let dir = p.dir.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut out = Vec::new();
            match modengine::Engine::load(&dir) {
                Err(e) => {
                    for l in e.lines() {
                        out.push(LogLine { kind: LogKind::Error, text: l.to_string() });
                    }
                }
                Ok(mut e) => {
                    if run_steps {
                        e.start();
                        for _ in 0..300 {
                            e.tick();
                            e.render();
                            e.audio.clear();
                        }
                        for err in &e.errors {
                            out.push(LogLine { kind: LogKind::Error, text: err.clone() });
                        }
                        if e.errors.is_empty() {
                            out.push(LogLine { kind: LogKind::Ok, text: format!("comprobado: 300 pasos sin errores (room '{}')", e.rooms[e.room].name) });
                        }
                    } else {
                        out.push(LogLine {
                            kind: LogKind::Ok,
                            text: format!("el proyecto carga bien: {} objetos, {} rooms", e.objects.len(), e.rooms.len()),
                        });
                    }
                }
            }
            let _ = tx.send(out);
        });
        self.checks.push(rx);
    }

    fn run_game(&mut self) {
        if self.runner.is_some() {
            self.stop_game();
        }
        self.save_all();
        let Some(p) = &self.project else { return };
        let exe = std::env::current_exe().ok().map(|e| e.with_file_name(if cfg!(windows) { "modengine.exe" } else { "modengine" }));
        let Some(exe) = exe.filter(|e| e.exists()) else {
            self.info(LogKind::Error, "no encuentro el ejecutable `modengine` junto al editor: compílalo con `cargo build -p modengine-cli`");
            return;
        };
        match Command::new(exe).arg("run").arg(&p.dir).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
            Ok(mut child) => {
                let (tx, rx) = mpsc::channel();
                if let Some(out) = child.stdout.take() {
                    let tx = tx.clone();
                    std::thread::spawn(move || BufReader::new(out).lines().map_while(Result::ok).for_each(|l| drop(tx.send(l))));
                }
                if let Some(err) = child.stderr.take() {
                    std::thread::spawn(move || BufReader::new(err).lines().map_while(Result::ok).for_each(|l| drop(tx.send(l))));
                }
                self.info(LogKind::Info, "▶ juego en marcha");
                self.runner = Some(Runner { child, rx });
            }
            Err(e) => self.info(LogKind::Error, format!("no se pudo lanzar el juego: {e}")),
        }
    }

    fn stop_game(&mut self) {
        if let Some(mut r) = self.runner.take() {
            let _ = r.child.kill();
            let _ = r.child.wait();
            self.info(LogKind::Info, "■ juego detenido");
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(r) = &mut self.runner {
            for l in r.rx.try_iter() {
                let kind = if l.starts_with("[error]") { LogKind::Error } else { LogKind::Game };
                self.log.push(LogLine { kind, text: l });
            }
            if let Ok(Some(status)) = r.child.try_wait() {
                let msg = format!("■ el juego terminó ({status})");
                self.runner = None;
                self.info(LogKind::Info, msg);
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let mut done = vec![];
        for (i, rx) in self.checks.iter().enumerate() {
            if let Ok(lines) = rx.try_recv() {
                self.log.extend(lines);
                done.push(i);
            }
        }
        for i in done.into_iter().rev() {
            self.checks.remove(i);
        }
        if !self.checks.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        if self.log.len() > 2000 {
            self.log.drain(..self.log.len() - 2000);
        }
    }

    fn open(&mut self, what: Open) {
        if let Some(i) = self.tabs.iter().position(|t| t.open_key() == what) {
            self.active = i;
            return;
        }
        let Some(p) = &mut self.project else { return };
        let tab = match &what {
            Open::Room(n) => match p.rooms.iter().find(|r| &r.name == n) {
                Some(r) => match RoomEditor::open(n, &r.path) {
                    Ok(e) => Tab::Room(e),
                    Err(e) => return self.info(LogKind::Error, e),
                },
                None => return,
            },
            Open::Object(n) => Tab::Object(ObjectEditor::new(n)),
            Open::Sprite(n) => match p.sprite_entry(n) {
                Some(s) => match SpriteEditor::open(n, &s.path, s.is_toml) {
                    Ok(e) => Tab::Sprite(e),
                    Err(e) => return self.info(LogKind::Error, e),
                },
                None => return,
            },
            Open::Sound(n) => match p.sounds.iter().find(|s| &s.name == n) {
                Some(s) => match SoundEditor::open(n, &s.path, s.is_toml) {
                    Ok(e) => Tab::Sound(e),
                    Err(e) => return self.info(LogKind::Error, e),
                },
                None => return,
            },
            Open::Script(rel) => Tab::Script(ScriptEditor::new(rel)),
            Open::Settings => Tab::Settings(SettingsEditor::new(p)),
        };
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
    }

    fn close_tab(&mut self, idx: usize) {
        if idx < self.tabs.len() {
            self.tabs.remove(idx);
            if self.active >= self.tabs.len() {
                self.active = self.tabs.len().saturating_sub(1);
            }
        }
    }

    // ------------------------------------------------------------ ui pieces

    fn start_screen(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(60.0);
                ui.heading(RichText::new("Mod Engine").size(36.0).strong());
                ui.label("editor de juegos 2D: objetos, eventos y rooms");
                ui.add_space(30.0);
            });
            ui.horizontal(|ui| {
                ui.add_space(((ui.available_width() - 540.0) / 2.0).max(0.0));
                ui.vertical(|ui| {
                ui.set_width(540.0);
                ui.group(|ui| {
                    ui.strong("Abrir un proyecto");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.start.path).hint_text("/ruta/a/mi_juego").desired_width(380.0));
                        if ui.button("Abrir").clicked() {
                            let p = PathBuf::from(self.start.path.trim());
                            self.open_project(&p);
                        }
                    });
                    if !self.start.recent.is_empty() {
                        ui.add_space(6.0);
                        ui.label("Recientes:");
                        for r in self.start.recent.clone() {
                            if ui.link(&r).clicked() {
                                self.open_project(Path::new(&r));
                            }
                        }
                    }
                });
                ui.add_space(12.0);
                ui.group(|ui| {
                    ui.strong("Crear un proyecto nuevo");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.start.new_path).hint_text("/ruta/a/nuevo_juego").desired_width(380.0));
                        if ui.button("Crear").clicked() {
                            let p = PathBuf::from(self.start.new_path.trim());
                            match modengine::scaffold::create(&p) {
                                Ok(()) => self.open_project(&p),
                                Err(e) => self.start.error = Some(e),
                            }
                        }
                    });
                });
                if let Some(e) = &self.start.error {
                    ui.colored_label(crate::theme::colors(ui.ctx()).error, e);
                }
                });
            });
        });
    }

    fn menu(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("Archivo", |ui| {
                if ui.button("Guardar todo    Ctrl+S").clicked() {
                    self.save_all();
                    ui.close();
                }
                if ui.button("Cerrar proyecto").clicked() {
                    if self.dirty_any() {
                        self.save_all();
                    }
                    self.stop_game();
                    self.project = None;
                    self.tabs.clear();
                    self.start.recent = load_recent();
                    ui.close();
                }
                ui.separator();
                if ui.button("Salir").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    ui.close();
                }
            });
            ui.menu_button("Crear", |ui| {
                for (label, kind) in [
                    ("Sprite (pixel art)", NewKind::PixelSprite),
                    ("Sprite (importar PNG)", NewKind::PngSprite),
                    ("Sonido", NewKind::Sound),
                    ("Objeto", NewKind::Object),
                    ("Room", NewKind::Room),
                    ("Script", NewKind::Script),
                ] {
                    if ui.button(label).clicked() {
                        self.dialog = Some(Dialog::New { kind, name: String::new(), extra: String::new(), error: None });
                        ui.close();
                    }
                }
            });
            ui.menu_button("Juego", |ui| {
                if ui.button("▶ Ejecutar    F5").clicked() {
                    self.run_game();
                    ui.close();
                }
                if ui.button("✔ Comprobar    F6").clicked() {
                    self.check(true);
                    ui.close();
                }
                if ui.add_enabled(self.runner.is_some(), egui::Button::new("■ Detener")).clicked() {
                    self.stop_game();
                    ui.close();
                }
                ui.separator();
                if ui.button("⚙ Ajustes del juego").clicked() {
                    self.open(Open::Settings);
                    ui.close();
                }
            });
            ui.menu_button("Editar", |ui| {
                if ui.button("Deshacer    Ctrl+Z").clicked() {
                    if let Some(t) = self.tabs.get_mut(self.active) {
                        t.undo();
                    }
                    ui.close();
                }
                if ui.button("Rehacer    Ctrl+Y").clicked() {
                    if let Some(t) = self.tabs.get_mut(self.active) {
                        t.redo();
                    }
                    ui.close();
                }
            });
            ui.menu_button("Ver", |ui| {
                use crate::theme::Theme;
                for (t, label) in [(Theme::Light, "Tema claro"), (Theme::Dark, "Tema oscuro")] {
                    if ui.radio(self.theme == t, label).clicked() {
                        self.theme = t;
                        crate::theme::apply(ui.ctx(), t);
                        crate::theme::save(t);
                        ui.close();
                    }
                }
            });
            ui.add_space(20.0);
            let running = self.runner.is_some();
            if ui.add(crate::theme::primary_button(ui, if running { "⟳ Reiniciar" } else { "▶ Ejecutar" })).on_hover_text("F5: guarda todo y lanza el juego").clicked() {
                self.run_game();
            }
            if running && ui.button("■ Detener").clicked() {
                self.stop_game();
            }
            if ui.button("✔ Comprobar").on_hover_text("F6: carga el proyecto y ejecuta 300 pasos sin ventana").clicked() {
                self.check(true);
            }
            if let Some(p) = &self.project {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(p.dir.display().to_string()).weak());
                    ui.label(RichText::new(&p.game.game.name).strong());
                });
            }
        });
    }

    fn section(&mut self, ui: &mut egui::Ui, key: &'static str, title: &str, add: Option<NewKind>) -> bool {
        let open = *self.folded.get(key).unwrap_or(&true);
        let mut new_open = open;
        ui.horizontal(|ui| {
            let arrow = if open { "▾" } else { "▸" };
            if ui.add(egui::Label::new(RichText::new(format!("{arrow} {title}")).strong()).sense(Sense::click())).clicked() {
                new_open = !open;
            }
            if let Some(kind) = add {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("+").on_hover_text("crear").clicked() {
                        self.dialog = Some(Dialog::New { kind, name: String::new(), extra: String::new(), error: None });
                    }
                });
            }
        });
        self.folded.insert(key, new_open);
        open
    }

    fn asset_row(&mut self, ui: &mut egui::Ui, label: &str, sprite: Option<String>, what: Open, deletable: bool) {
        let f = self.filter.to_lowercase();
        if !f.is_empty() && !label.to_lowercase().contains(&f) {
            return;
        }
        let active = self.tabs.get(self.active).map(|t| t.open_key() == what).unwrap_or(false);
        let resp = ui
            .horizontal(|ui| {
                ui.add_space(10.0);
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                if let (Some(s), Some(p)) = (&sprite, &self.project) {
                    if let Some(t) = self.tex.get(ui.ctx(), p, s) {
                        t.draw_fit(ui.painter(), 0, rect);
                    }
                }
                ui.add(egui::Button::selectable(active, label).frame(false))
            })
            .inner;
        if resp.clicked() {
            self.open(what.clone());
        }
        resp.context_menu(|ui| {
            if ui.button("Abrir").clicked() {
                self.open(what.clone());
                ui.close();
            }
            if deletable && ui.button("Eliminar…").clicked() {
                self.dialog = Some(Dialog::Delete { what: what.clone() });
                ui.close();
            }
        });
    }

    fn asset_browser(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("🔍");
            ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("filtrar").desired_width(f32::INFINITY));
        });
        ui.separator();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let Some(p) = &self.project else { return };
            let sprites: Vec<String> = p.sprite_names();
            let sounds: Vec<String> = p.sounds.iter().map(|s| s.name.clone()).collect();
            let objects: Vec<(String, Option<String>, Option<String>)> =
                p.objects.iter().map(|o| (o.name.clone(), p.object_sprite(&o.name), o.parent.clone())).collect();
            let rooms: Vec<String> = p.room_names();
            let start = p.game.game.start_room.clone();
            let scripts: Vec<String> = p.scripts.iter().map(|s| s.rel.clone()).collect();
            let fonts: Vec<String> = p.fonts.iter().map(|s| s.name.clone()).collect();

            if self.section(ui, "sprites", "Sprites", Some(NewKind::PixelSprite)) {
                for s in sprites {
                    self.asset_row(ui, &s, Some(s.clone()), Open::Sprite(s.clone()), true);
                }
            }
            if self.section(ui, "objects", "Objetos", Some(NewKind::Object)) {
                for (o, spr, parent) in objects {
                    let label = match parent {
                        Some(pa) => format!("{o}  ‹ {pa}"),
                        None => o.clone(),
                    };
                    self.asset_row(ui, &label, spr, Open::Object(o.clone()), true);
                }
            }
            if self.section(ui, "rooms", "Rooms", Some(NewKind::Room)) {
                for r in rooms {
                    let label = if start.as_deref() == Some(r.as_str()) { format!("{r}  ★") } else { r.clone() };
                    self.asset_row(ui, &label, None, Open::Room(r.clone()), true);
                }
            }
            if self.section(ui, "sounds", "Sonidos", Some(NewKind::Sound)) {
                for s in sounds {
                    self.asset_row(ui, &s, None, Open::Sound(s.clone()), true);
                }
            }
            if self.section(ui, "scripts", "Scripts", Some(NewKind::Script)) {
                for s in scripts {
                    self.asset_row(ui, &s, None, Open::Script(s.clone()), true);
                }
            }
            if !fonts.is_empty() && self.section(ui, "fonts", "Fuentes", None) {
                for f in fonts {
                    ui.horizontal(|ui| {
                        ui.add_space(30.0);
                        ui.label(f);
                    });
                }
            }
            ui.add_space(6.0);
            if ui.button("⚙ Ajustes del juego").clicked() {
                self.open(Open::Settings);
            }
        });
    }

    fn output(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("Salida");
            if ui.small_button("limpiar").clicked() {
                self.log.clear();
            }
            if self.runner.is_some() {
                ui.colored_label(crate::theme::colors(ui.ctx()).ok, "● juego en marcha");
            }
        });
        let pal = crate::theme::colors(ui.ctx());
        egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
            for l in &self.log {
                let c = match l.kind {
                    LogKind::Info => pal.info,
                    LogKind::Ok => pal.ok,
                    LogKind::Error => pal.error,
                    LogKind::Game => pal.game,
                };
                ui.label(RichText::new(&l.text).monospace().color(c));
            }
        });
    }

    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        let mut close = None;
        ui.horizontal_wrapped(|ui| {
            for (i, t) in self.tabs.iter().enumerate() {
                let dirty = self.project.as_ref().map(|p| t.dirty(p)).unwrap_or(false);
                let title = if dirty { format!("{} ●", t.title()) } else { t.title() };
                let r = ui.add(egui::Button::selectable(i == self.active, title));
                if r.clicked() {
                    self.active = i;
                }
                if r.middle_clicked() {
                    close = Some(i);
                }
                if ui.small_button("×").clicked() {
                    close = Some(i);
                }
                ui.add_space(6.0);
            }
        });
        if let Some(i) = close {
            let dirty = self.project.as_ref().map(|p| self.tabs[i].dirty(p)).unwrap_or(false);
            if dirty {
                self.dialog = Some(Dialog::CloseTab { idx: i });
            } else {
                self.close_tab(i);
            }
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.dialog.take() else { return };
        let mut keep = true;
        let mut next: Option<Dialog> = None;
        match dialog {
            Dialog::New { kind, mut name, mut extra, mut error } => {
                let mut create = false;
                egui::Modal::new(egui::Id::unique("new_asset")).show(ctx, |ui| {
                    ui.heading(match kind {
                        NewKind::PixelSprite => "Nuevo sprite (pixel art)",
                        NewKind::PngSprite => "Importar PNG como sprite",
                        NewKind::Sound => "Nuevo sonido",
                        NewKind::Object => "Nuevo objeto",
                        NewKind::Room => "Nueva room",
                        NewKind::Script => "Nuevo script",
                    });
                    ui.horizontal(|ui| {
                        ui.label("Nombre:");
                        let r = ui.text_edit_singleline(&mut name);
                        r.request_focus();
                        if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                            create = true;
                        }
                    });
                    if kind == NewKind::PngSprite {
                        ui.horizontal(|ui| {
                            ui.label("Fichero PNG:");
                            ui.text_edit_singleline(&mut extra);
                        });
                    }
                    if kind == NewKind::PixelSprite {
                        ui.horizontal(|ui| {
                            ui.label("Tamaño:");
                            if extra.is_empty() {
                                extra = "16x16".into();
                            }
                            ui.add(egui::TextEdit::singleline(&mut extra).desired_width(60.0));
                        });
                    }
                    if let Some(e) = &error {
                        ui.colored_label(crate::theme::colors(ui.ctx()).error, e);
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Crear").clicked() {
                            create = true;
                        }
                        if ui.button("Cancelar").clicked() {
                            keep = false;
                        }
                    });
                });
                if create {
                    match self.create_asset(kind, name.trim(), extra.trim()) {
                        Ok(what) => {
                            keep = false;
                            self.open(what);
                        }
                        Err(e) => error = Some(e),
                    }
                }
                if keep {
                    next = Some(Dialog::New { kind, name, extra, error });
                }
            }
            Dialog::Delete { what } => {
                egui::Modal::new(egui::Id::unique("delete_asset")).show(ctx, |ui| {
                    ui.heading("¿Eliminar?");
                    ui.label(format!("{what:?}"));
                    ui.label(RichText::new("El fichero se mueve a la carpeta .trash del proyecto.").weak());
                    ui.horizontal(|ui| {
                        if ui.button("Eliminar").clicked() {
                            if let Err(e) = self.delete_asset(&what) {
                                self.info(LogKind::Error, e);
                            }
                            keep = false;
                        }
                        if ui.button("Cancelar").clicked() {
                            keep = false;
                        }
                    });
                });
                if keep {
                    next = Some(Dialog::Delete { what });
                }
            }
            Dialog::CloseTab { idx } => {
                egui::Modal::new(egui::Id::unique("close_tab")).show(ctx, |ui| {
                    ui.heading("Hay cambios sin guardar");
                    ui.horizontal(|ui| {
                        if ui.button("Guardar").clicked() {
                            if let Some(p) = &mut self.project {
                                let mut open = vec![];
                                let mut cx = Cx { project: p, tex: &mut self.tex, log: &mut self.log, open: &mut open };
                                if let Err(e) = self.tabs[idx].save(&mut cx) {
                                    cx.error(e);
                                }
                            }
                            self.close_tab(idx);
                            keep = false;
                        }
                        if ui.button("Descartar").clicked() {
                            if let Some(p) = &mut self.project {
                                self.tabs[idx].revert(p);
                            }
                            self.close_tab(idx);
                            keep = false;
                        }
                        if ui.button("Cancelar").clicked() {
                            keep = false;
                        }
                    });
                });
                if keep {
                    next = Some(Dialog::CloseTab { idx });
                }
            }
            Dialog::Quit => {
                egui::Modal::new(egui::Id::unique("quit")).show(ctx, |ui| {
                    ui.heading("Hay cambios sin guardar");
                    ui.horizontal(|ui| {
                        if ui.button("Guardar y salir").clicked() {
                            self.save_all();
                            self.quitting = true;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            keep = false;
                        }
                        if ui.button("Salir sin guardar").clicked() {
                            self.quitting = true;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            keep = false;
                        }
                        if ui.button("Cancelar").clicked() {
                            keep = false;
                        }
                    });
                });
                if keep {
                    next = Some(Dialog::Quit);
                }
            }
        }
        self.dialog = next;
    }

    fn create_asset(&mut self, kind: NewKind, name: &str, extra: &str) -> Result<Open, String> {
        let Some(p) = &mut self.project else { return Err("no hay proyecto".into()) };
        if kind != NewKind::Script && !valid_name(name) {
            return Err("usa letras, números y _ (sin empezar por número)".into());
        }
        if kind != NewKind::Script && p.taken_names().contains(name) {
            return Err(format!("ya existe un asset llamado '{name}'"));
        }
        let write = |path: PathBuf, text: String| -> Result<(), String> {
            if path.exists() {
                return Err(format!("{} ya existe", path.display()));
            }
            std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::write(&path, text).map_err(|e| e.to_string())
        };
        let what = match kind {
            NewKind::PixelSprite => {
                let (w, h) = extra.split_once('x').and_then(|(a, b)| Some((a.trim().parse::<usize>().ok()?, b.trim().parse::<usize>().ok()?))).unwrap_or((16, 16));
                let (w, h) = (w.clamp(1, 256), h.clamp(1, 256));
                let row = ".".repeat(w);
                let rows = vec![row; h].join("\n");
                write(p.dir.join("sprites").join(format!("{name}.toml")), format!("origin = \"center\"\npixels = ['''\n{rows}\n''']\n"))?;
                p.reload_sprites();
                Open::Sprite(name.into())
            }
            NewKind::PngSprite => {
                let src = PathBuf::from(extra);
                if !src.is_file() {
                    return Err(format!("no existe el fichero '{extra}'"));
                }
                let dst = p.dir.join("sprites").join(format!("{name}.png"));
                std::fs::create_dir_all(dst.parent().unwrap()).map_err(|e| e.to_string())?;
                std::fs::copy(&src, &dst).map_err(|e| e.to_string())?;
                p.reload_sprites();
                Open::Sprite(name.into())
            }
            NewKind::Sound => {
                write(
                    p.dir.join("sounds").join(format!("{name}.toml")),
                    "[synth]\nwave = \"square\"\nfreq = 440\nduration = 0.15\nvolume = 0.3\n".into(),
                )?;
                p.reload_lists();
                Open::Sound(name.into())
            }
            NewKind::Object => {
                let rel = format!("objects/{name}.ms");
                if p.script_index(&rel).is_some() || p.dir.join(&rel).exists() {
                    return Err(format!("{rel} ya existe"));
                }
                let i = p.add_script(&rel, format!("object {name} {{\n    on create {{\n    }}\n}}\n"));
                p.save_script(i)?;
                Open::Object(name.into())
            }
            NewKind::Room => {
                write(
                    p.dir.join("rooms").join(format!("{name}.toml")),
                    "width = 320\nheight = 180\nbackground = \"#29366f\"\n\n[[layer]]\nname = \"Instances\"\ntype = \"instances\"\ncell = [16, 16]\n".into(),
                )?;
                if !p.game.game.rooms.is_empty() {
                    p.game.game.rooms.push(name.to_string());
                    std::fs::write(p.dir.join("game.toml"), write_game_toml(&p.game)).map_err(|e| e.to_string())?;
                }
                p.reload_lists();
                Open::Room(name.into())
            }
            NewKind::Script => {
                let file = name.trim_end_matches(".ms");
                if !valid_name(file) {
                    return Err("nombre de fichero no válido".into());
                }
                let rel = format!("scripts/{file}.ms");
                if p.script_index(&rel).is_some() {
                    return Err(format!("{rel} ya existe"));
                }
                let i = p.add_script(&rel, "// Funciones globales y código que se ejecuta al empezar el juego.\n".into());
                p.save_script(i)?;
                Open::Script(rel)
            }
        };
        Ok(what)
    }

    fn delete_asset(&mut self, what: &Open) -> Result<(), String> {
        let Some(p) = &mut self.project else { return Ok(()) };
        match what {
            Open::Sprite(n) => {
                if let Some(s) = p.sprite_entry(n) {
                    let path = s.path.clone();
                    if s.is_toml {
                        if let Ok(t) = toml::from_str::<toml::Table>(&std::fs::read_to_string(&path).unwrap_or_default()) {
                            if let Some(img) = t.get("image").and_then(|v| v.as_str()) {
                                let png = path.with_file_name(img);
                                if png.exists() {
                                    p.trash(&png)?;
                                }
                            }
                        }
                    }
                    p.trash(&path)?;
                }
                p.reload_sprites();
            }
            Open::Sound(n) => {
                if let Some(s) = p.sounds.iter().find(|s| &s.name == n) {
                    let path = s.path.clone();
                    p.trash(&path)?;
                }
                p.reload_lists();
            }
            Open::Room(n) => {
                if let Some(r) = p.rooms.iter().find(|r| &r.name == n) {
                    let path = r.path.clone();
                    p.trash(&path)?;
                }
                if p.game.game.rooms.iter().any(|r| r == n) {
                    p.game.game.rooms.retain(|r| r != n);
                    std::fs::write(p.dir.join("game.toml"), write_game_toml(&p.game)).map_err(|e| e.to_string())?;
                }
                p.reload_lists();
            }
            Open::Object(n) => {
                if let Some(o) = p.object(n) {
                    let si = o.script;
                    let text = crate::objsrc::remove(&p.scripts[si].text, n);
                    if text.trim().is_empty() {
                        let path = p.scripts[si].path.clone();
                        if path.exists() {
                            p.trash(&path)?;
                        }
                        p.scripts.remove(si);
                        p.rescan_objects();
                    } else {
                        p.set_script_text(si, text);
                        p.save_script(si)?;
                    }
                }
            }
            Open::Script(rel) => {
                if let Some(i) = p.script_index(rel) {
                    let path = p.scripts[i].path.clone();
                    if path.exists() {
                        p.trash(&path)?;
                    }
                    p.scripts.remove(i);
                    p.rescan_objects();
                }
            }
            Open::Settings => {}
        }
        if let Some(i) = self.tabs.iter().position(|t| &t.open_key() == what) {
            self.close_tab(i);
        }
        self.info(LogKind::Info, format!("eliminado {what:?} (en .trash)"));
        Ok(())
    }

    fn run_auto(&mut self, ctx: &egui::Context) {
        self.auto.frames += 1;
        if self.auto.frames == 2 {
            for o in std::mem::take(&mut self.auto.open) {
                self.open(o);
            }
            if self.auto.select {
                for t in &mut self.tabs {
                    match t {
                        Tab::Room(r) => r.auto_select(),
                        Tab::Object(o) => o.show_vars(),
                        _ => {}
                    }
                }
            }
        }
        let Some(path) = self.auto.screenshot.clone() else { return };
        ctx.request_repaint();
        if self.auto.frames == 30 {
            let done = self.auto.done.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img| {
                let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
                match image::save_buffer(&path, &bytes, img.size[0] as u32, img.size[1] as u32, image::ExtendedColorType::Rgba8) {
                    Ok(()) => println!("saved {}", path.display()),
                    Err(e) => eprintln!("screenshot: {e}"),
                }
                done.store(true, std::sync::atomic::Ordering::SeqCst);
            })));
        }
        if self.auto.done.load(std::sync::atomic::Ordering::SeqCst) {
            self.quitting = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let (save, run, check, undo, redo, close) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::COMMAND, Key::S),
                i.consume_key(Modifiers::NONE, Key::F5),
                i.consume_key(Modifiers::NONE, Key::F6),
                i.consume_key(Modifiers::COMMAND, Key::Z),
                i.consume_key(Modifiers::COMMAND, Key::Y) || i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z),
                i.consume_key(Modifiers::COMMAND, Key::W),
            )
        });
        if save {
            self.save_all();
        }
        if run {
            self.run_game();
        }
        if check {
            self.check(true);
        }
        // text fields handle their own undo; only forward when nothing has keyboard focus
        let typing = ctx.memory(|m| m.focused().is_some());
        if !typing {
            if undo {
                if let Some(t) = self.tabs.get_mut(self.active) {
                    t.undo();
                }
            }
            if redo {
                if let Some(t) = self.tabs.get_mut(self.active) {
                    t.redo();
                }
            }
        }
        if close && !self.tabs.is_empty() {
            let i = self.active;
            let dirty = self.project.as_ref().map(|p| self.tabs[i].dirty(p)).unwrap_or(false);
            if dirty {
                self.dialog = Some(Dialog::CloseTab { idx: i });
            } else {
                self.close_tab(i);
            }
        }
    }
}

impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        self.run_auto(&ctx);

        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting {
            if self.dirty_any() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.dialog = Some(Dialog::Quit);
            } else {
                self.stop_game();
            }
        }

        if self.project.is_none() {
            self.start_screen(ui);
            return;
        }
        self.shortcuts(&ctx);

        let side = crate::theme::sidebar_frame(&ctx);
        egui::Panel::top("menu").frame(side.inner_margin(egui::Margin::symmetric(8, 4))).show(ui, |ui| self.menu(ui));
        egui::Panel::bottom("output").frame(side).resizable(true).default_size(150.0).show(ui, |ui| self.output(ui));
        egui::Panel::right("assets").frame(side).resizable(true).default_size(250.0).show(ui, |ui| self.asset_browser(ui));
        egui::CentralPanel::default().frame(crate::theme::editor_frame(&ctx)).show(ui, |ui| {
            if self.tabs.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new("Abre un asset desde el navegador de la derecha\n(o créalo con el botón +)").weak().size(16.0));
                });
                return;
            }
            self.tab_bar(ui);
            ui.separator();
            let mut open = Vec::new();
            if let (Some(p), Some(tab)) = (&mut self.project, self.tabs.get_mut(self.active)) {
                let mut cx = Cx { project: p, tex: &mut self.tex, log: &mut self.log, open: &mut open };
                tab.ui(ui, &mut cx);
            }
            for o in open {
                self.open(o);
            }
        });
        self.dialogs(&ctx);
    }
}

impl Drop for EditorApp {
    fn drop(&mut self) {
        self.stop_game();
    }
}
