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
use crate::workspace::{LayoutFile, Workspaces, WsAction};
use eframe::egui::{self, Key, Modifiers, RichText, Sense, Vec2};
use std::collections::{HashMap, HashSet};
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
    Font(String),
    Note(String),
    TileSet(String),
    /// Paths, timelines, animation curves and sequences (motion.rs).
    Anim(crate::motion::AnimKind, String),
    /// Particle systems, shaders and extensions (fx.rs).
    Fx(crate::fx::FxKind, String),
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
            "font" => Open::Font(v.into()),
            "note" => Open::Note(v.into()),
            "tileset" => Open::TileSet(v.into()),
            "settings" => Open::Settings,
            k if crate::motion::AnimKind::parse(k).is_some() => Open::Anim(crate::motion::AnimKind::parse(k)?, v.into()),
            k if crate::fx::FxKind::parse(k).is_some() => Open::Fx(crate::fx::FxKind::parse(k)?, v.into()),
            _ => return None,
        })
    }

    /// Inverse of [`Open::parse`].
    pub fn key(&self) -> String {
        match self {
            Open::Room(n) => format!("room:{n}"),
            Open::Object(n) => format!("object:{n}"),
            Open::Sprite(n) => format!("sprite:{n}"),
            Open::Sound(n) => format!("sound:{n}"),
            Open::Script(n) => format!("script:{n}"),
            Open::Font(n) => format!("font:{n}"),
            Open::Note(n) => format!("note:{n}"),
            Open::TileSet(n) => format!("tileset:{n}"),
            Open::Anim(k, n) => format!("{}:{n}", k.key()),
            Open::Fx(k, n) => format!("{}:{n}", k.key()),
            Open::Settings => "settings".into(),
        }
    }
}

/// Scripted startup used for automated screenshots.
#[derive(Default)]
pub struct Auto {
    pub open: Vec<Open>,
    pub screenshot: Option<PathBuf>,
    /// Select something in the opened editors (first free instance, the variables pane).
    pub select: bool,
    /// Zoom the workspace out so every opened window is visible.
    pub fit: bool,
    /// Show the Agente dock.
    pub agent: bool,
    /// Select this layer in the opened room editors (`--layer Mundo`).
    pub layer: Option<String>,
    /// Open the New dialog of this kind (`--new font`).
    pub new: Option<NewKind>,
    /// Click here once the editor is up (`--click 60,14` opens the Crear menu).
    pub click: Option<egui::Pos2>,
    /// Fill the Agente dock with sample state (`agent_panel::AgentPanel::demo`).
    pub agent_demo: Option<String>,
    /// Open the import dialog with these files or folders.
    pub import: Vec<PathBuf>,
    /// Then press its Importar button (end-to-end runs).
    pub import_run: bool,
    /// Pretend this asset is being dragged and held at this screen point (screenshots of
    /// drag-and-drop feedback; the drop never happens).
    pub drag: Option<(Open, egui::Pos2)>,
    /// Config folder for recent projects etc. instead of ~/.config/modengine (screenshots).
    pub config_dir: Option<PathBuf>,
    /// Open the Nuevo proyecto dialog (with this template id, when not empty).
    pub new_project: Option<String>,
    /// Open the Abrir proyecto browser at this folder.
    pub open_project: Option<PathBuf>,
    /// Launch this many copies of the game once the editor is up (`--run 2`).
    pub run: Option<u32>,
    frames: u32,
    shot: bool,
    done: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// What every editor gets while drawing.
pub struct Cx<'a> {
    pub project: &'a mut Project,
    pub tex: &'a mut TexCache,
    pub log: &'a mut Vec<LogLine>,
    pub open: &'a mut Vec<Open>,
    /// The editor's window has focus (keyboard shortcuts like Delete only act there).
    pub active: bool,
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
    Font(crate::font::FontEditor),
    Note(crate::note::NoteEditor),
    TileSet(crate::tileset::TileSetEditor),
    Anim(crate::motion::AnimTab),
    Fx(crate::fx::FxTab),
    Settings(SettingsEditor),
}

impl Tab {
    pub fn open_key(&self) -> Open {
        match self {
            Tab::Room(e) => Open::Room(e.name.clone()),
            Tab::Object(e) => Open::Object(e.name.clone()),
            Tab::Sprite(e) => Open::Sprite(e.name.clone()),
            Tab::Sound(e) => Open::Sound(e.name.clone()),
            Tab::Script(e) => Open::Script(e.rel.clone()),
            Tab::Font(e) => Open::Font(e.name.clone()),
            Tab::Note(e) => Open::Note(e.name.clone()),
            Tab::TileSet(e) => Open::TileSet(e.name.clone()),
            Tab::Anim(e) => Open::Anim(e.kind(), e.name().to_string()),
            Tab::Fx(e) => Open::Fx(e.kind(), e.name().to_string()),
            Tab::Settings(_) => Open::Settings,
        }
    }

    /// Title bar text of the editor's workspace window.
    pub fn window_title(&self) -> String {
        match self {
            Tab::Room(e) => format!("▦ Room: {}", e.name),
            Tab::Object(e) => format!("◆ Objeto: {}", e.name),
            Tab::Sprite(e) => format!("▣ Sprite: {}", e.name),
            Tab::Sound(e) => format!("♪ Sonido: {}", e.name),
            Tab::Script(e) => format!("✎ Script: {}", e.rel),
            Tab::Font(e) => format!("🗛 Fuente: {}", e.name),
            Tab::Note(e) => format!("🗒 Nota: {}", e.name),
            Tab::TileSet(e) => format!("▩ Tile set: {}", e.name),
            Tab::Anim(e) => e.title(),
            Tab::Fx(e) => e.title(),
            Tab::Settings(_) => "⚙ Ajustes del juego".into(),
        }
    }

    pub fn dirty(&self, p: &Project) -> bool {
        match self {
            Tab::Room(e) => e.dirty,
            Tab::Sprite(e) => e.dirty,
            Tab::Sound(e) => e.dirty,
            Tab::Settings(e) => e.dirty,
            Tab::Anim(e) => e.dirty(),
            Tab::Fx(e) => e.dirty(),
            Tab::Font(e) => e.dirty,
            Tab::Note(e) => e.dirty,
            Tab::TileSet(e) => e.dirty,
            Tab::Object(e) => e.script_rel(p).and_then(|r| p.script_index(&r)).map(|i| p.scripts[i].dirty()).unwrap_or(false),
            Tab::Script(e) => p.script_index(&e.rel).map(|i| p.scripts[i].dirty()).unwrap_or(false),
        }
    }

    pub fn save(&mut self, cx: &mut Cx) -> Result<(), String> {
        match self {
            Tab::Room(e) => e.save(cx),
            Tab::Sprite(e) => e.save(cx),
            Tab::Sound(e) => e.save(cx),
            Tab::Settings(e) => e.save(cx),
            Tab::Anim(e) => e.save(cx),
            Tab::Fx(e) => e.save(cx),
            Tab::Font(e) => e.save(cx),
            Tab::Note(e) => e.save(cx),
            Tab::TileSet(e) => e.save(cx),
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
    pub fn revert(&mut self, p: &mut Project) {
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

    pub fn undo(&mut self) {
        match self {
            Tab::Room(e) => e.undo(),
            Tab::Anim(e) => e.undo(),
            Tab::Fx(e) => e.undo(),
            Tab::Sprite(e) => e.undo(),
            Tab::TileSet(e) => e.undo(),
            _ => {}
        }
    }

    pub fn redo(&mut self) {
        match self {
            Tab::Room(e) => e.redo(),
            Tab::Anim(e) => e.redo(),
            Tab::Fx(e) => e.redo(),
            Tab::Sprite(e) => e.redo(),
            Tab::TileSet(e) => e.redo(),
            _ => {}
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        match self {
            Tab::Room(e) => e.ui(ui, cx),
            Tab::Object(e) => e.ui(ui, cx),
            Tab::Sprite(e) => e.ui(ui, cx),
            Tab::Sound(e) => e.ui(ui, cx),
            Tab::Script(e) => e.ui(ui, cx),
            Tab::Font(e) => e.ui(ui, cx),
            Tab::Note(e) => e.ui(ui, cx),
            Tab::TileSet(e) => e.ui(ui, cx),
            Tab::Settings(e) => e.ui(ui, cx),
            Tab::Anim(e) => e.ui(ui, cx),
            Tab::Fx(e) => e.ui(ui, cx),
        }
    }
}

/// Creates the editor for an asset. `Err(None)` when the asset doesn't exist.
fn make_tab(p: &Project, what: &Open) -> Result<Tab, Option<String>> {
    Ok(match what {
        Open::Room(n) => {
            let r = p.rooms.iter().find(|r| &r.name == n).ok_or(None)?;
            Tab::Room(RoomEditor::open(n, &r.path).map_err(Some)?)
        }
        Open::Object(n) => {
            p.object(n).ok_or(None)?;
            Tab::Object(ObjectEditor::new(n))
        }
        Open::Sprite(n) => {
            let s = p.sprite_entry(n).ok_or(None)?;
            Tab::Sprite(SpriteEditor::open(n, &s.path, s.is_toml).map_err(Some)?)
        }
        Open::Sound(n) => {
            let s = p.sounds.iter().find(|s| &s.name == n).ok_or(None)?;
            Tab::Sound(SoundEditor::open(n, &s.path, s.is_toml).map_err(Some)?)
        }
        Open::Script(rel) => {
            p.script_index(rel).ok_or(None)?;
            Tab::Script(ScriptEditor::new(rel))
        }
        Open::Font(n) => {
            let s = p.fonts.iter().find(|s| &s.name == n).ok_or(None)?;
            Tab::Font(crate::font::FontEditor::open(n, &s.path, s.is_toml).map_err(Some)?)
        }
        Open::Note(n) => {
            let s = p.notes.iter().find(|s| &s.name == n).ok_or(None)?;
            Tab::Note(crate::note::NoteEditor::open(n, &s.path).map_err(Some)?)
        }
        Open::TileSet(n) => {
            let s = p.tilesets.iter().find(|s| &s.name == n).ok_or(None)?;
            Tab::TileSet(crate::tileset::TileSetEditor::open(n, &s.path).map_err(Some)?)
        }
        Open::Anim(k, n) => Tab::Anim(crate::motion::AnimTab::open(p, *k, n)?),
        Open::Fx(k, n) => Tab::Fx(crate::fx::FxTab::open(p, *k, n)?),
        Open::Settings => Tab::Settings(SettingsEditor::new(p)),
    })
}

// ------------------------------------------------------------------ dialogs

/// What the New dialog creates (see the Crear menu table in `kinds.rs`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum NewKind {
    PixelSprite,
    PngSprite,
    Sound,
    Object,
    Room,
    Script,
    Font,
    Note,
    TileSet,
    /// Path, timeline, animation curve or sequence.
    Anim(crate::motion::AnimKind),
    /// Particle system, shader or extension.
    Fx(crate::fx::FxKind),
}

enum Dialog {
    New { kind: NewKind, name: String, extra: String, error: Option<String> },
    Delete { what: Open },
    CloseWin { id: u64 },
    Quit,
    /// Unsaved changes before opening another project (`Some`) or closing this one.
    Switch { to: Option<PathBuf> },
}

/// The game(s) launched with ▶ Ejecutar (several for multiplayer tests).
struct Runner {
    children: Vec<Child>,
    rx: Receiver<String>,
}

pub struct EditorApp {
    project: Option<Project>,
    ws: Workspaces,
    /// Inspector, asset browser, output and agent docks shown.
    docks: [bool; 4],
    agent: crate::agent_panel::AgentPanel,
    /// Modification times of the project files when the agent last started or the project was
    /// last reloaded, to tell which open editors to reload.
    stamps: HashMap<PathBuf, std::time::SystemTime>,
    /// Asset selected in the browser (shown in the inspector).
    selected: Option<Open>,
    tex: TexCache,
    log: Vec<LogLine>,
    runner: Option<Runner>,
    checks: Vec<Receiver<Vec<LogLine>>>,
    /// Start page, recent projects and the Nuevo / Abrir proyecto dialogs (start.rs).
    start: crate::start::StartPage,
    dialog: Option<Dialog>,
    folded: HashMap<&'static str, bool>,
    filter: String,
    quitting: bool,
    auto: Auto,
    theme: crate::theme::Theme,
    /// Background updates from the `master` branch (installed copies only).
    upd: crate::update::Actualizador,
    /// The user closed the "update found/ready" bar for this session.
    upd_oculta: bool,
    /// Custom order of the asset browser sections (saved with the layout).
    order: crate::dnd::AssetOrder,
    /// The asset browser was the last thing clicked: Delete / Escape act on it.
    browser_focus: bool,
    /// Screen rectangle of the asset browser dock in the last frame.
    browser_rect: egui::Rect,
    /// Ctrl+F: focus the asset filter on the next frame.
    focus_filter: bool,
    /// The Importar assets dialog, when open.
    import: Option<crate::import::ImportDialog>,
}

/// Browser section of an asset (key of its custom order).
fn section_key(o: &Open) -> Option<&'static str> {
    Some(match o {
        Open::Sprite(_) => "sprites",
        Open::Object(_) => "objects",
        Open::Room(_) => "rooms",
        Open::Sound(_) => "sounds",
        Open::Script(_) => "scripts",
        Open::Font(_) => "fonts",
        Open::Note(_) => "notes",
        Open::TileSet(_) => "tilesets",
        Open::Anim(k, _) => k.section(),
        Open::Fx(k, _) => k.section(),
        Open::Settings => return None,
    })
}

use crate::kinds::Row;

impl EditorApp {
    pub fn new(cc: &eframe::CreationContext, path: Option<PathBuf>, auto: Auto) -> Self {
        let theme = crate::theme::load();
        crate::theme::install_fonts(&cc.egui_ctx);
        crate::theme::apply(&cc.egui_ctx, theme);
        crate::zoom::install(&cc.egui_ctx);
        let mut app = EditorApp {
            project: None,
            ws: Workspaces::default(),
            docks: [true, true, true, auto.agent],
            agent: crate::agent_panel::AgentPanel::with_config(crate::agent_panel::config_path()),
            stamps: HashMap::new(),
            selected: None,
            tex: TexCache::default(),
            log: vec![],
            runner: None,
            checks: vec![],
            start: crate::start::StartPage::new(auto.config_dir.clone().or_else(crate::projects::config_dir)),
            dialog: None,
            folded: HashMap::new(),
            filter: String::new(),
            quitting: false,
            auto,
            theme,
            upd: crate::update::Actualizador::default(),
            upd_oculta: false,
            order: Default::default(),
            browser_focus: false,
            browser_rect: egui::Rect::NOTHING,
            focus_filter: false,
            import: None,
        };
        if app.auto.screenshot.is_none() {
            app.upd.vigilar(&cc.egui_ctx);
        }
        app.start.native_ok = app.auto.screenshot.is_none();
        if let Some(p) = path {
            app.open_project(&p);
        }
        app
    }

    fn info(&mut self, kind: LogKind, s: impl Into<String>) {
        self.log.push(LogLine { kind, text: s.into() });
    }

    fn open_project(&mut self, path: &Path) {
        // a game.toml opens its folder
        let dir = crate::projects::project_dir(path).unwrap_or_else(|| path.to_path_buf());
        let path = dir.as_path();
        match Project::open(path) {
            Ok(p) => {
                self.stop_game();
                self.start.opened(path);
                self.save_layout();
                self.agent.close_project();
                self.ws = Workspaces::default();
                self.selected = None;
                self.order.clear();
                self.filter.clear();
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
                self.load_layout();
                self.check(false);
            }
            Err(e) => {
                let e = format!("no se pudo abrir el proyecto: {e}");
                if self.project.is_some() {
                    self.info(LogKind::Error, e.clone());
                }
                self.start.error = Some(e);
            }
        }
    }

    /// Opens another project (`Some`) or closes this one, asking first about unsaved changes.
    fn request_switch(&mut self, to: Option<PathBuf>) {
        if self.dirty_any() {
            self.dialog = Some(Dialog::Switch { to });
        } else {
            self.switch_to(to);
        }
    }

    fn switch_to(&mut self, to: Option<PathBuf>) {
        match to {
            Some(p) => self.open_project(&p),
            None => self.close_project(),
        }
    }

    fn close_project(&mut self) {
        self.stop_game();
        self.save_layout();
        self.agent.close_project();
        self.project = None;
        self.ws = Workspaces::default();
        self.selected = None;
    }

    /// Folder of the open project (tests).
    #[cfg(test)]
    pub fn project_dir(&self) -> Option<&Path> {
        self.project.as_ref().map(|p| p.dir.as_path())
    }

    fn dirty_any(&self) -> bool {
        let Some(p) = &self.project else { return false };
        p.dirty_scripts() || self.ws.wins().any(|w| w.tab.dirty(p))
    }

    fn save_all(&mut self) {
        let Some(project) = &mut self.project else { return };
        let mut open = vec![];
        let mut errors = vec![];
        {
            let mut cx = Cx { project, tex: &mut self.tex, log: &mut self.log, open: &mut open, active: false };
            for t in self.ws.tabs_mut() {
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
        self.save_layout();
        self.check(false);
    }

    // ------------------------------------------------------------ workspace layout

    fn load_layout(&mut self) {
        let Some(p) = &self.project else { return };
        if self.auto.screenshot.is_some() {
            return; // reproducible screenshots: start from an empty workspace
        }
        let Some(f) = crate::workspace::layout_path(&p.dir).and_then(|f| std::fs::read_to_string(f).ok()).and_then(|t| toml::from_str::<LayoutFile>(&t).ok()) else {
            return;
        };
        self.ws = Workspaces::from_file(&f, |o| make_tab(p, o).ok());
        self.order = f.order.clone();
        self.docks = [f.docks[0], f.docks[1], f.docks[2], f.agent || self.auto.agent];
    }

    fn save_layout(&self) {
        let Some(p) = &self.project else { return };
        if self.auto.screenshot.is_some() {
            return;
        }
        let Some(path) = crate::workspace::layout_path(&p.dir) else { return };
        let mut f = self.ws.to_file(&p.dir.to_string_lossy(), self.docks);
        f.order = self.order.clone();
        if let (Some(d), Ok(text)) = (path.parent(), toml::to_string(&f)) {
            let _ = std::fs::create_dir_all(d);
            let _ = std::fs::write(path, text);
        }
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
        self.run_games(1);
    }

    /// Launches `count` copies of the game (`--instance N` places them side by side and
    /// tags their output `[N]`): the quick way to try multiplayer on one computer.
    fn run_games(&mut self, count: u32) {
        if self.runner.is_some() {
            self.stop_game();
        }
        self.save_all();
        let Some(dir) = self.project.as_ref().map(|p| p.dir.clone()) else { return };
        let exe = std::env::current_exe().ok().map(|e| e.with_file_name(if cfg!(windows) { "modengine.exe" } else { "modengine" }));
        let Some(exe) = exe.filter(|e| e.exists()) else {
            self.info(LogKind::Error, "no encuentro el ejecutable `modengine` junto al editor: compílalo con `cargo build -p modengine-cli`");
            return;
        };
        let (tx, rx) = mpsc::channel();
        let mut children = Vec::new();
        for n in 1..=count.max(1) {
            if n > 1 {
                // starting two OpenGL windows at the very same moment garbled the text of
                // the second one (NVIDIA): give the first a head start
                std::thread::sleep(std::time::Duration::from_millis(600));
            }
            let mut cmd = Command::new(&exe);
            cmd.arg("run").arg(&dir);
            if count > 1 {
                cmd.arg("--instance").arg(n.to_string());
            }
            match cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
                Ok(mut child) => {
                    // with several copies every line says which one it comes from
                    let tag = if count > 1 { format!("[{n}] ") } else { String::new() };
                    if let Some(out) = child.stdout.take() {
                        let (tx, tag) = (tx.clone(), tag.clone());
                        std::thread::spawn(move || BufReader::new(out).lines().map_while(Result::ok).for_each(|l| drop(tx.send(format!("{tag}{l}")))));
                    }
                    if let Some(err) = child.stderr.take() {
                        let tx = tx.clone();
                        std::thread::spawn(move || BufReader::new(err).lines().map_while(Result::ok).for_each(|l| drop(tx.send(format!("{tag}{l}")))));
                    }
                    children.push(child);
                }
                Err(e) => self.info(LogKind::Error, format!("no se pudo lanzar el juego: {e}")),
            }
        }
        if children.is_empty() {
            return;
        }
        self.info(LogKind::Info, if count > 1 { format!("▶ {count} copias del juego en marcha (multijugador)") } else { "▶ juego en marcha".to_string() });
        self.runner = Some(Runner { children, rx });
    }

    fn stop_game(&mut self) {
        if let Some(mut r) = self.runner.take() {
            for c in &mut r.children {
                let _ = c.kill();
                let _ = c.wait();
            }
            self.info(LogKind::Info, "■ juego detenido");
        }
    }

    /// Modification time of every project file (hidden folders and saves/ left out).
    fn file_stamps(dir: &Path) -> HashMap<PathBuf, std::time::SystemTime> {
        fn walk(d: &Path, out: &mut HashMap<PathBuf, std::time::SystemTime>) {
            let Ok(rd) = std::fs::read_dir(d) else { return };
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with('.') || name == "saves" || name == "target" {
                    continue;
                }
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if let Ok(t) = e.metadata().and_then(|m| m.modified()) {
                    out.insert(p, t);
                }
            }
        }
        let mut out = HashMap::new();
        walk(dir, &mut out);
        out
    }

    /// Picks up what the agent changed on disk without clobbering unsaved edits: scripts,
    /// game.toml, the asset lists and sprites, and the open room/sprite/sound/settings editors
    /// whose files changed and that have no unsaved changes.
    fn reload_from_disk(&mut self) {
        let Some(p) = &mut self.project else { return };
        let now = Self::file_stamps(&p.dir);
        let changed: HashSet<PathBuf> = now.iter().filter(|(f, t)| self.stamps.get(*f) != Some(*t)).map(|(f, _)| f.clone()).collect();
        self.stamps = now;
        let synced = p.sync_from_disk();
        if !changed.is_empty() || synced.game {
            p.reload_lists();
        }
        let mut lines = vec![];
        if !synced.scripts.is_empty() {
            lines.push(LogLine { kind: LogKind::Info, text: format!("recargado del disco: {}", synced.scripts.join(", ")) });
        }
        for c in &synced.conflicts {
            lines.push(LogLine { kind: LogKind::Error, text: format!("{c} cambió en disco pero tiene cambios sin guardar: se conservan los tuyos (guardar sobrescribe la versión del disco)") });
        }
        // A sprite is its .toml and/or its .png: either one counts.
        let touched = |path: &Path| changed.iter().any(|c| c == path || (c.parent() == path.parent() && c.file_stem() == path.file_stem()));
        for w in self.ws.wins_mut() {
            let key = w.tab.open_key();
            let path = match &key {
                Open::Room(n) => p.rooms.iter().find(|r| &r.name == n).map(|r| r.path.clone()),
                Open::Sprite(n) => p.sprite_entry(n).map(|s| s.path.clone()),
                Open::Sound(n) => p.sounds.iter().find(|s| &s.name == n).map(|s| s.path.clone()),
                Open::Settings => Some(p.dir.join("game.toml")),
                Open::Anim(k, n) => p.motion.path(*k, n),
                Open::Fx(k, n) => p.fx.path(*k, n),
                Open::Font(n) => p.fonts.iter().find(|s| &s.name == n).map(|s| s.path.clone()),
                Open::Note(n) => p.notes.iter().find(|s| &s.name == n).map(|s| s.path.clone()),
                Open::TileSet(n) => p.tilesets.iter().find(|s| &s.name == n).map(|s| s.path.clone()),
                Open::Object(_) | Open::Script(_) => None, // they show the project's scripts, synced above
            };
            let Some(path) = path.filter(|f| touched(f)) else { continue };
            if w.tab.dirty(p) {
                lines.push(LogLine { kind: LogKind::Error, text: format!("{} cambió en disco pero su editor tiene cambios sin guardar: se conservan los tuyos", path.strip_prefix(&p.dir).unwrap_or(&path).display()) });
            } else if let Ok(tab) = make_tab(p, &key) {
                w.tab = tab;
            }
        }
        self.log.extend(lines);
    }

    /// What the Agente panel can attach this frame.
    fn attach_sources<'a>(project: &'a Option<Project>, ws: &Workspaces, selected: &Option<Open>, log: &'a [LogLine]) -> crate::agent_attach::Sources<'a> {
        let focused = ws.focused();
        let room_selection = focused.and_then(|w| match &w.tab {
            Tab::Room(r) => r.selection_toml().map(|t| (r.name.clone(), t)),
            _ => None,
        });
        crate::agent_attach::Sources { project: project.as_ref(), selected: selected.clone(), focused: focused.map(|w| w.tab.open_key()), room_selection, log }
    }

    /// Sends a message to the agent, after saving everything so it sees what the user sees.
    fn send_to_agent(&mut self, prompt: String) {
        self.save_all();
        let Some(p) = &self.project else { return };
        let (dir, name) = (p.dir.clone(), p.game.game.name.clone());
        self.stamps = Self::file_stamps(&dir);
        self.agent.send(&prompt, &dir, &name);
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let agent = self.agent.poll(ctx);
        if agent.reload {
            self.reload_from_disk();
        }
        if agent.finished {
            if !agent.imported.is_empty() {
                self.info(LogKind::Ok, format!("Importado por el agente: {}", agent.imported.join(", ")));
            }
            for d in &agent.downloaded {
                self.info(LogKind::Ok, format!("Descargado por el agente de la Tienda: {d}"));
            }
            if !agent.changed.is_empty() {
                let dir = self.project.as_ref().map(|p| p.dir.clone()).unwrap_or_default();
                let names: Vec<String> = agent.changed.iter().map(|f| f.strip_prefix(&dir).unwrap_or(f).display().to_string()).collect();
                self.info(LogKind::Info, format!("el agente cambió: {}", names.join(", ")));
            }
            self.check(false);
        }
        if let Some(r) = &mut self.runner {
            for l in r.rx.try_iter() {
                let kind = if l.starts_with("[error]") || l.contains("] [error]") { LogKind::Error } else { LogKind::Game };
                self.log.push(LogLine { kind, text: l });
            }
            let statuses: Vec<Option<std::process::ExitStatus>> = r.children.iter_mut().map(|c| c.try_wait().ok().flatten()).collect();
            if statuses.iter().all(|s| s.is_some()) {
                let msg = match statuses.last() {
                    Some(Some(status)) if statuses.len() == 1 => format!("■ el juego terminó ({status})"),
                    _ => "■ las copias del juego terminaron".to_string(),
                };
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
        self.open_from(what, None);
    }

    /// Opens an editor window, chained to `parent` (the window it was opened from).
    /// An editor that is already open is focused instead.
    fn open_from(&mut self, what: Open, parent: Option<u64>) {
        self.selected = Some(what.clone());
        if let Some(id) = self.ws.find(&what) {
            self.ws.focus(id);
            return;
        }
        let Some(p) = &self.project else { return };
        match make_tab(p, &what) {
            Ok(tab) => {
                self.ws.add(tab, parent);
            }
            Err(Some(e)) => self.info(LogKind::Error, e),
            Err(None) => {}
        }
    }

    fn close_win(&mut self, id: u64) {
        self.ws.remove(id);
    }

    /// Closes a window, asking first when it has unsaved changes.
    fn request_close(&mut self, id: u64) {
        let dirty = match (&self.project, self.ws.wins().find(|w| w.id == id)) {
            (Some(p), Some(w)) => w.tab.dirty(p),
            _ => false,
        };
        if dirty {
            self.dialog = Some(Dialog::CloseWin { id });
        } else {
            self.close_win(id);
        }
    }

    // ------------------------------------------------------------ ui pieces

    fn start_screen(&mut self, ui: &mut egui::Ui) {
        if let Some(crate::start::Action::Open(p)) = self.start.ui(ui) {
            self.open_project(&p);
        }
    }

    fn menu(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("Archivo", |ui| {
                if ui.button("＋ Nuevo proyecto…    Ctrl+Mayús+N").clicked() {
                    self.start.open_new_dialog();
                    ui.close();
                }
                if ui.button("📂 Abrir proyecto…    Ctrl+O").clicked() {
                    self.start.open_browser(None);
                    ui.close();
                }
                ui.menu_button("Abrir reciente", |ui| {
                    let current = self.project.as_ref().map(|p| p.dir.clone());
                    if let Some(crate::start::Action::Open(p)) = self.start.recent_menu(ui, current.as_deref()) {
                        self.request_switch(Some(p));
                    }
                });
                if ui.button("🗁 Mostrar en el gestor de archivos").clicked() {
                    if let Some(p) = &self.project {
                        if let Err(e) = crate::projects::show_in_file_manager(&p.dir) {
                            self.info(LogKind::Error, e);
                        }
                    }
                    ui.close();
                }
                ui.separator();
                if ui.button("Guardar todo    Ctrl+S").clicked() {
                    self.save_all();
                    ui.close();
                }
                if ui.button("Importar assets…").on_hover_text("PNG, WAV, OGG, TTF u OTF de cualquier carpeta").clicked() {
                    self.open_import(&[], None);
                    ui.close();
                }
                if ui.button("🎨 Paletas (Lospec)…").on_hover_text("Descargar o importar paletas y aplicarlas al proyecto").clicked() {
                    crate::palette::open_dialog();
                    ui.close();
                }
                if ui.button("🛒 Tienda de assets (OpenGameArt)…").on_hover_text("Buscar e importar sprites, sonidos y fuentes gratuitos, con sus créditos").clicked() {
                    crate::store::open_dialog();
                    ui.close();
                }
                ui.separator();
                if ui.button("📦 Exportar…").on_hover_text("Crear el juego para Windows (.exe), Linux o Android (.apk)").clicked() {
                    self.save_all();
                    crate::export::open_dialog();
                    ui.close();
                }
                ui.separator();
                if ui.button("Cerrar proyecto").clicked() {
                    self.request_switch(None);
                    ui.close();
                }
                ui.separator();
                if ui.button("Salir").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    ui.close();
                }
            });
            ui.menu_button("Crear", |ui| {
                if let Some(kind) = crate::kinds::create_menu(ui) {
                    self.new_dialog(kind);
                }
            });
            ui.menu_button("Juego", |ui| {
                if ui.button("▶ Ejecutar    F5").clicked() {
                    self.run_game();
                    ui.close();
                }
                if ui.button("▶▶ Ejecutar 2 instancias").on_hover_text("multijugador: dos copias del juego, una al lado de la otra (en una pulsa H para ser host y en la otra J para unirte)").clicked() {
                    self.run_games(2);
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
                    if let Some(w) = self.ws.focused_mut() {
                        w.tab.undo();
                    }
                    ui.close();
                }
                if ui.button("Rehacer    Ctrl+Y").clicked() {
                    if let Some(w) = self.ws.focused_mut() {
                        w.tab.redo();
                    }
                    ui.close();
                }
            });
            ui.menu_button("Workspace", |ui| {
                if ui.button("Nuevo workspace").clicked() {
                    self.ws.add_workspace();
                    ui.close();
                }
                if ui.button("Ver todas las ventanas    Ctrl+0").clicked() {
                    self.ws.fit_all();
                    ui.close();
                }
                if ui.button("Zoom al 100 %").clicked() {
                    self.ws.reset_zoom();
                    ui.close();
                }
                ui.separator();
                if ui.button("Cerrar ventana    Ctrl+W").clicked() {
                    if let Some(id) = self.ws.focused().map(|w| w.id) {
                        self.request_close(id);
                    }
                    ui.close();
                }
                if ui.button("Cerrar todas las ventanas").clicked() {
                    let ids: Vec<u64> = self.ws.cur().wins.iter().map(|w| w.id).collect();
                    for id in ids {
                        self.request_close(id);
                    }
                    ui.close();
                }
            });
            ui.menu_button("Ver", |ui| {
                for (i, label) in ["Inspector", "Navegador de assets", "Salida", "Agente"].into_iter().enumerate() {
                    ui.checkbox(&mut self.docks[i], label);
                }
                if ui.button("🛒 Tienda de assets").clicked() {
                    crate::store::open_dialog();
                    ui.close();
                }
                ui.separator();
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
            ui.menu_button("Ayuda", |ui| {
                ui.label(RichText::new(format!("Mod Engine {}", crate::update::descripcion_version())).weak());
                ui.separator();
                let instalada = crate::update::version_instalada().is_some();
                if ui.add_enabled(instalada, egui::Button::new("Buscar actualizaciones")).on_disabled_hover_text("Solo las copias instaladas con instalar.sh / instalar.ps1 se actualizan solas").clicked() {
                    self.upd_oculta = false;
                    self.upd.buscar_ahora(crate::update::automatico());
                    ui.close();
                }
                let mut auto = crate::update::automatico();
                if ui.add_enabled(instalada, egui::Checkbox::new(&mut auto, "Actualizar automáticamente")).changed() {
                    crate::update::cambiar_automatico(auto);
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
            if ui.button("📦 Exportar").on_hover_text("Crear el juego para Windows, Linux o Android").clicked() {
                self.save_all();
                crate::export::open_dialog();
            }
            if ui.button("🛒 Tienda").on_hover_text("Tienda de assets gratuitos (OpenGameArt.org)").clicked() {
                crate::store::open_dialog();
            }
            ui.toggle_value(&mut self.docks[3], "✦ Agente").on_hover_text("Panel del agente de programación (Claude Code, Antigravity o Codex)");
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
            let head = ui.add(egui::Label::new(RichText::new(format!("{arrow} {title}")).strong()).sense(Sense::click()));
            if head.clicked() {
                new_open = !open;
            }
            // sprites, sounds and fonts can come from files elsewhere on disk
            let import = crate::import::section_kind(key);
            // right click: import (sprites, sounds, fonts) and the whole Crear menu
            head.context_menu(|ui| {
                if let Some(k) = import {
                    if ui.button(crate::import::menu_label(k)).clicked() {
                        self.open_import(&[], Some(k));
                        ui.close();
                    }
                    ui.separator();
                }
                if let Some(kind) = crate::kinds::create_menu(ui) {
                    self.new_dialog(kind);
                }
            });
            if add.is_some() || import.is_some() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(kind) = add {
                        if ui.small_button("+").on_hover_text("crear").clicked() {
                            self.new_dialog(kind);
                        }
                    }
                    if let Some(k) = import {
                        if ui.small_button("⬇").on_hover_text(crate::import::menu_label(k)).clicked() {
                            self.open_import(&[], Some(k));
                        }
                    }
                });
            }
        });
        self.folded.insert(key, new_open);
        open
    }

    /// Draws one asset of the browser. Returns its row rectangle (None when filtered out).
    fn asset_row(&mut self, ui: &mut egui::Ui, row: &Row, deletable: bool) -> Option<egui::Rect> {
        let f = self.filter.to_lowercase();
        if !f.is_empty() && !row.label.to_lowercase().contains(&f) {
            return None;
        }
        let what = &row.what;
        let dragged = egui::DragAndDrop::payload::<Open>(ui.ctx()).is_some_and(|p| *p == *what);
        let active = dragged || self.selected.as_ref() == Some(what) || self.ws.focused().is_some_and(|w| w.tab.open_key() == *what);
        let ir = ui.horizontal(|ui| {
            ui.add_space(10.0);
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
            if let (Some(s), Some(p)) = (&row.sprite, &self.project) {
                if let Some(t) = self.tex.get(ui.ctx(), p, s) {
                    t.draw_fit(ui.painter(), 0, rect);
                }
            }
            // click: select · double click: open · drag: reorder or drop somewhere else
            ui.add(egui::Button::selectable(active, &row.label).frame(false).sense(Sense::click_and_drag()))
        });
        let resp = ir.inner;
        crate::dnd::source(&resp, || what.clone());
        if resp.clicked() || resp.drag_started() {
            self.selected = Some(what.clone());
            self.browser_focus = true;
        }
        if resp.double_clicked() {
            self.open(what.clone());
        }
        resp.on_hover_text("clic: inspector · doble clic: abrir · arrastrar: reordenar o soltar en el workspace, una room o un objeto").context_menu(|ui| {
            if ui.button("Abrir").clicked() {
                self.open(what.clone());
                ui.close();
            }
            if deletable && ui.button("Eliminar…    Supr").clicked() {
                self.dialog = Some(Dialog::Delete { what: what.clone() });
                ui.close();
            }
        });
        Some(ir.response.rect)
    }

    /// A browser section's rows, in the user's order, reorderable by dragging.
    fn asset_list(&mut self, ui: &mut egui::Ui, key: &'static str, rows: Vec<Row>) {
        let mut rects = vec![];
        for r in &rows {
            if let Some(rect) = self.asset_row(ui, r, true) {
                rects.push(rect);
            }
        }
        // reordering only makes sense with every row visible
        if !self.filter.is_empty() || rects.len() != rows.len() {
            return;
        }
        let Some(dragged) = egui::DragAndDrop::payload::<Open>(ui.ctx()) else { return };
        if section_key(&dragged) != Some(key) {
            return;
        }
        let Some(from) = rows.iter().position(|r| r.what == *dragged) else { return };
        let area = crate::dnd::list_area(ui, &rects, true);
        if let Some((from, to)) = crate::dnd::reorder_in(ui, area, &rects, true, from) {
            egui::DragAndDrop::clear_payload(ui.ctx());
            let mut names: Vec<String> = rows.into_iter().map(|r| r.name).collect();
            crate::dnd::move_item(&mut names, from, to);
            self.order.insert(key.to_string(), names);
            self.save_layout();
        }
    }

    fn sorted(&self, key: &str, names: Vec<String>) -> Vec<String> {
        match self.order.get(key) {
            Some(o) => crate::dnd::apply_order(names, o),
            None => names,
        }
    }

    fn asset_browser(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("🔍");
            let r = ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("filtrar  (Ctrl+F)").desired_width(f32::INFINITY));
            if std::mem::take(&mut self.focus_filter) {
                r.request_focus();
            }
            if (r.has_focus() || r.lost_focus()) && ui.input(|i| i.key_pressed(Key::Escape)) {
                self.filter.clear();
                r.surrender_focus();
            }
        });
        ui.separator();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if self.project.is_none() {
                return;
            }
            // one section per asset kind (table in kinds.rs), rows in the user's order
            for s in crate::kinds::SECTIONS {
                let Some(p) = &self.project else { return };
                let mut rows = (s.rows)(p);
                let order = self.sorted(s.key, rows.iter().map(|r| r.name.clone()).collect());
                rows.sort_by_key(|r| order.iter().position(|n| *n == r.name));
                if self.section(ui, s.key, s.title, s.new) {
                    self.asset_list(ui, s.key, rows);
                }
            }
            ui.add_space(6.0);
            if ui.button("⚙ Ajustes del juego").clicked() {
                self.open(Open::Settings);
            }
            // right click on the empty space below: the Crear menu
            let rest = ui.available_size().max(Vec2::new(ui.available_width(), 40.0));
            let bg = ui.allocate_response(rest, Sense::click());
            bg.context_menu(|ui| {
                if let Some(kind) = crate::kinds::create_menu(ui) {
                    self.new_dialog(kind);
                }
            });
        });
    }

    /// An asset dropped on the empty workspace canvas: open it there (or move it there).
    fn drop_on_canvas(&mut self, what: Open, pos: egui::Pos2) {
        self.selected = Some(what.clone());
        if self.ws.find(&what).is_none() {
            let Some(p) = &self.project else { return };
            match make_tab(p, &what) {
                Ok(tab) => {
                    self.ws.add(tab, None);
                }
                Err(Some(e)) => return self.info(LogKind::Error, e),
                Err(None) => return,
            }
        }
        if let Some(id) = self.ws.find(&what) {
            self.ws.place_at(id, pos);
        }
    }

    /// Workspace tabs plus the canvas with the open editors.
    fn workspace(&mut self, ui: &mut egui::Ui) {
        let dirty: HashSet<u64> = match &self.project {
            Some(p) => self.ws.wins().filter(|w| w.tab.dirty(p)).map(|w| w.id).collect(),
            None => HashSet::new(),
        };
        let mut actions = vec![];
        egui::Frame::new().fill(crate::theme::sidebar_frame(ui.ctx()).fill).inner_margin(egui::Margin::symmetric(6, 3)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            actions.extend(self.ws.tab_bar(ui, &dirty));
        });
        let mut opens: Vec<(Open, u64)> = vec![];
        // while the asset browser has the keyboard, Delete belongs to it
        let focused = self.ws.cur().focus.filter(|_| !self.browser_focus);
        let (project, tex, log) = (&mut self.project, &mut self.tex, &mut self.log);
        let Some(project) = project.as_mut() else { return };
        actions.extend(self.ws.canvas(ui, &dirty, |ui, w| {
            let mut open = vec![];
            let mut cx = Cx { project, tex, log, open: &mut open, active: focused == Some(w.id) };
            w.tab.ui(ui, &mut cx);
            opens.extend(open.into_iter().map(|o| (o, w.id)));
        }));
        for (o, from) in opens {
            self.open_from(o, Some(from));
        }
        for a in actions {
            match a {
                WsAction::Close(id) => self.request_close(id),
                WsAction::DropAsset(what, pos) => self.drop_on_canvas(what, pos),
                WsAction::CloseWorkspace(i) => {
                    let ids: Vec<u64> = self.ws.list.get(i).map(|w| w.wins.iter().map(|x| x.id).collect()).unwrap_or_default();
                    for id in ids {
                        self.request_close(id);
                    }
                    self.ws.remove_workspace(i);
                }
            }
        }
    }

    /// Left dock: details of the asset selected in the browser (or the focused window).
    fn inspector(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("Inspector");
        });
        ui.separator();
        let what = self.selected.clone().or_else(|| self.ws.focused().map(|w| w.tab.open_key()));
        let Some(what) = what else {
            ui.label(RichText::new("Selecciona un asset en el navegador para ver sus propiedades.").weak());
            return;
        };
        let Some(p) = &self.project else { return };
        let (kind, name) = match &what {
            Open::Room(n) => ("Room", n.clone()),
            Open::Object(n) => ("Objeto", n.clone()),
            Open::Sprite(n) => ("Sprite", n.clone()),
            Open::Sound(n) => ("Sonido", n.clone()),
            Open::Script(n) => ("Script", n.clone()),
            Open::Font(n) => ("Fuente", n.clone()),
            Open::Note(n) => ("Nota", n.clone()),
            Open::TileSet(n) => ("Tile set", n.clone()),
            Open::Anim(k, n) => (k.label(), n.clone()),
            Open::Fx(k, n) => (k.label(), n.clone()),
            Open::Settings => ("Juego", p.game.game.name.clone()),
        };
        ui.label(RichText::new(format!("Inspector: {name}")).weak());
        ui.add_space(4.0);
        let preview_sprite = match &what {
            Open::Sprite(n) => Some(n.clone()),
            Open::Object(n) => p.object_sprite(n),
            _ => None,
        };
        let mut open_it = false;
        let mut play: Option<PathBuf> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(56.0), Sense::hover());
                crate::tex::checkerboard(ui.painter(), r, 7.0);
                if let Some(t) = preview_sprite.as_deref().and_then(|s| self.tex.get(ui.ctx(), p, s)) {
                    t.draw_fit(ui.painter(), 0, r.shrink(4.0));
                } else {
                    let icon = match &what {
                        Open::Room(_) => "▦",
                        Open::Sound(_) => "♪",
                        Open::Script(_) => "✎",
                        Open::Font(_) => "🗛",
                        Open::Note(_) => "🗒",
                        Open::TileSet(_) => "▩",
                        Open::Settings => "⚙",
                        Open::Anim(k, _) => k.icon(),
                        Open::Fx(k, _) => k.icon(),
                        _ => "◆",
                    };
                    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(26.0), ui.visuals().text_color());
                }
                ui.vertical(|ui| {
                    ui.label(RichText::new(kind).weak());
                    ui.label(RichText::new(&name).strong().monospace());
                    if ui.button("Abrir editor").clicked() {
                        open_it = true;
                    }
                });
            });
            ui.add_space(8.0);
            egui::Grid::new("insp").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
                let mut row = |k: &str, v: String| {
                    ui.label(RichText::new(k).weak());
                    ui.label(v);
                    ui.end_row();
                };
                match &what {
                    Open::Sprite(n) => {
                        if let Some(s) = p.sprite(n) {
                            row("tamaño", format!("{} × {} px", s.fw, s.fh));
                            row("fotogramas", s.frames.len().to_string());
                            row("fps", modscript::fmt_num(s.fps));
                            row("origen", format!("{}, {}", modscript::fmt_num(s.origin.0), modscript::fmt_num(s.origin.1)));
                            row("máscara", format!("{:?}", s.bbox.map(|v| v as i64)));
                        }
                        if let Some(e) = p.sprite_entry(n) {
                            row("fichero", e.path.strip_prefix(&p.dir).unwrap_or(&e.path).display().to_string());
                        }
                        let users: Vec<String> = p.objects.iter().filter(|o| p.object_sprite(&o.name).as_deref() == Some(n.as_str())).map(|o| o.name.clone()).collect();
                        if !users.is_empty() {
                            row("usado por", users.join(", "));
                        }
                    }
                    Open::Object(n) => {
                        if let Some(o) = p.object(n) {
                            row("padre", o.parent.clone().unwrap_or_else(|| "—".into()));
                            row("sprite", p.object_sprite(n).unwrap_or_else(|| "—".into()));
                            row("fichero", p.scripts[o.script].rel.clone());
                            if let Some(m) = &o.model {
                                let ev: Vec<String> = m
                                    .items
                                    .iter()
                                    .filter_map(|i| match &i.kind {
                                        crate::objsrc::ItemKind::Event { name, arg, .. } => Some(match arg {
                                            Some(a) => format!("{name}({a})"),
                                            None => name.clone(),
                                        }),
                                        _ => None,
                                    })
                                    .collect();
                                row("eventos", if ev.is_empty() { "—".into() } else { ev.join(", ") });
                            }
                            let kids: Vec<String> = p.objects.iter().filter(|c| c.parent.as_deref() == Some(n.as_str())).map(|c| c.name.clone()).collect();
                            if !kids.is_empty() {
                                row("hijos", kids.join(", "));
                            }
                            let vars = crate::typedvar::object_vars(p, n);
                            for v in vars {
                                row(&v.name, format!("{}  ({})", v.value, v.ty.to_source()));
                            }
                        }
                    }
                    Open::Room(n) => {
                        if let Some(r) = p.rooms.iter().find(|r| &r.name == n) {
                            if let Ok(t) = std::fs::read_to_string(&r.path).map_err(|e| e.to_string()).and_then(|t| toml::from_str::<modengine::project::RoomToml>(&t).map_err(|e| e.to_string())) {
                                row("tamaño", match (t.width, t.height) {
                                    (Some(w), Some(h)) => format!("{w} × {h}"),
                                    _ => "automático".into(),
                                });
                                row("capas", t.layers.iter().map(|l| l.kind.clone()).collect::<Vec<_>>().join(", "));
                                row("instancias libres", t.layers.iter().map(|l| l.instances.len()).sum::<usize>().to_string());
                                row("persistente", if t.persistent { "sí" } else { "no" }.into());
                                if let Some(f) = &t.camera.follow {
                                    row("cámara sigue a", f.clone());
                                }
                            }
                            let start = p.game.game.start_room.as_deref() == Some(n.as_str());
                            row("room inicial", if start { "★ sí" } else { "no" }.into());
                        }
                    }
                    Open::Sound(n) => {
                        if let Some(s) = p.sounds.iter().find(|s| &s.name == n) {
                            row("fichero", s.path.strip_prefix(&p.dir).unwrap_or(&s.path).display().to_string());
                            row("tipo", if s.is_toml { "sintetizado / TOML" } else { "audio" }.into());
                            if !s.is_toml {
                                play = Some(s.path.clone());
                            }
                        }
                    }
                    Open::Script(rel) => {
                        if let Some(i) = p.script_index(rel) {
                            let sc = &p.scripts[i];
                            row("líneas", sc.text.lines().count().to_string());
                            let objs: Vec<String> = p.objects.iter().filter(|o| o.script == i).map(|o| o.name.clone()).collect();
                            if !objs.is_empty() {
                                row("objetos", objs.join(", "));
                            }
                            row("estado", if sc.dirty() { "● sin guardar" } else { "guardado" }.into());
                        }
                    }
                    Open::Anim(..) => {
                        for (k, v) in crate::motion::inspector_rows(p, &what) {
                            row(k, v);
                        }
                    }
                    Open::Fx(..) => {
                        for (k, v) in crate::fx::inspector_rows(p, &what) {
                            row(k, v);
                        }
                    }
                    Open::Settings => {
                        let d = &p.game.display;
                        row("resolución", format!("{} × {}", d.width, d.height));
                        row("rooms", p.rooms.len().to_string());
                    }
                    Open::Font(_) | Open::Note(_) | Open::TileSet(_) => {
                        for (k, v) in crate::kinds::inspector_rows(p, &what) {
                            row(k, v);
                        }
                    }
                }
            });
            if let Some(path) = &play {
                if ui.button("▶ Escuchar").clicked() {
                    if let Err(e) = crate::sound::play_file(path) {
                        ui.label(e);
                    }
                }
            }
        });
        if open_it {
            self.open(what);
        }
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

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.dialog.take() else { return };
        let mut keep = true;
        let mut next: Option<Dialog> = None;
        match dialog {
            Dialog::New { kind, mut name, mut extra, mut error } => {
                let mut create = false;
                egui::Modal::new(egui::Id::unique("new_asset")).show(ctx, |ui| {
                    ui.heading(crate::kinds::new_title(kind));
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
                    if let Some(p) = &self.project {
                        crate::kinds::new_extra(ui, kind, &mut extra, p);
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
            Dialog::CloseWin { id } => {
                let title = self.ws.wins().find(|w| w.id == id).map(|w| w.tab.window_title()).unwrap_or_default();
                egui::Modal::new(egui::Id::unique("close_win")).show(ctx, |ui| {
                    ui.heading("Hay cambios sin guardar");
                    ui.label(title);
                    ui.horizontal(|ui| {
                        if ui.button("Guardar").clicked() {
                            if let (Some(p), Some(w)) = (&mut self.project, self.ws.get_mut(id)) {
                                let mut open = vec![];
                                let mut cx = Cx { project: p, tex: &mut self.tex, log: &mut self.log, open: &mut open, active: false };
                                if let Err(e) = w.tab.save(&mut cx) {
                                    cx.error(e);
                                }
                            }
                            self.close_win(id);
                            keep = false;
                        }
                        if ui.button("Descartar").clicked() {
                            if let (Some(p), Some(w)) = (&mut self.project, self.ws.get_mut(id)) {
                                w.tab.revert(p);
                            }
                            self.close_win(id);
                            keep = false;
                        }
                        if ui.button("Cancelar").clicked() {
                            keep = false;
                        }
                    });
                });
                if keep {
                    next = Some(Dialog::CloseWin { id });
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
            Dialog::Switch { to } => {
                let mut go = false;
                egui::Modal::new(egui::Id::unique("switch_project")).show(ctx, |ui| {
                    ui.heading("Hay cambios sin guardar");
                    ui.label(if to.is_some() { "¿Guardar antes de abrir el otro proyecto?" } else { "¿Guardar antes de cerrar el proyecto?" });
                    ui.horizontal(|ui| {
                        if ui.add(crate::theme::primary_button(ui, "Guardar y continuar")).clicked() {
                            self.save_all();
                            go = true;
                        }
                        if ui.button("Descartar cambios").clicked() {
                            go = true;
                        }
                        if ui.button("Cancelar").clicked() {
                            keep = false;
                        }
                    });
                });
                if go {
                    self.switch_to(to);
                } else if keep {
                    next = Some(Dialog::Switch { to });
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
            NewKind::Font => crate::font::create(p, name, extra)?,
            NewKind::Note => crate::note::create(p, name)?,
            NewKind::TileSet => crate::tileset::create(p, name, extra)?,
            NewKind::Anim(k) => {
                crate::motion::create(p, k, name)?;
                Open::Anim(k, name.into())
            }
            NewKind::Fx(k) => {
                crate::fx::create(p, k, name)?;
                Open::Fx(k, name.into())
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
            Open::Font(n) | Open::TileSet(n) | Open::Note(n) => {
                let list = match what {
                    Open::Font(_) => p.fonts.iter().find(|s| &s.name == n).map(|s| s.path.clone()),
                    Open::TileSet(_) => p.tilesets.iter().find(|s| &s.name == n).map(|s| s.path.clone()),
                    _ => p.notes.iter().find(|s| &s.name == n).map(|s| s.path.clone()),
                };
                if let Some(path) = list {
                    // a font's TTF goes with its .toml
                    let mut files = vec![path.clone()];
                    if let Some(f) = std::fs::read_to_string(&path).ok().and_then(|t| t.parse::<toml::Table>().ok()).and_then(|t| t.get("file").and_then(|v| v.as_str()).map(String::from)) {
                        files.push(path.with_file_name(f));
                    }
                    for f in files.into_iter().filter(|f| f.exists()) {
                        p.trash(&f)?;
                    }
                }
                p.reload_lists();
            }
            Open::Anim(k, n) => crate::motion::delete(p, *k, n)?,
            Open::Fx(k, n) => crate::fx::delete(p, *k, n)?,
            Open::Settings => {}
        }
        if let Some(id) = self.ws.find(what) {
            self.close_win(id);
        }
        if self.selected.as_ref() == Some(what) {
            self.selected = None;
        }
        self.info(LogKind::Info, format!("eliminado {what:?} (en .trash)"));
        Ok(())
    }

    fn run_auto(&mut self, ctx: &egui::Context) {
        self.auto.frames += 1;
        if self.auto.frames == 2 {
            // each `--open` is chained to the previous one, like opening it from that window
            let mut prev = None;
            for o in std::mem::take(&mut self.auto.open) {
                self.open_from(o.clone(), prev);
                prev = self.ws.find(&o);
            }
            if self.auto.select {
                for t in self.ws.tabs_mut() {
                    match t {
                        Tab::Room(r) => r.auto_select(),
                        Tab::Anim(a) => a.auto_select(),
                        Tab::Object(o) => o.show_vars(),
                        Tab::TileSet(t) => t.auto_select(),
                        _ => {}
                    }
                }
            }
            if let Some(name) = self.auto.layer.clone() {
                for t in self.ws.tabs_mut() {
                    if let Tab::Room(r) = t {
                        r.select_layer(&name);
                    }
                }
            }
        }
        // `--agent --select`: show sample attachments in the Agente dock.
        if self.auto.agent && self.auto.select && self.auto.frames == 20 {
            let src = Self::attach_sources(&self.project, &self.ws, &self.selected, &self.log);
            self.agent.attach_defaults(&src);
        }
        if let (Some(what), Some(p), 3) = (&self.auto.agent_demo, &self.project, self.auto.frames) {
            self.agent.demo(what, &p.dir, &p.game.game.name);
        }
        if self.auto.frames == 3 {
            if let Some(n) = self.auto.run.take() {
                self.run_games(n);
            }
        }
        if self.auto.frames == 2 {
            if let Some(k) = self.auto.new.take() {
                self.new_dialog(k);
            }
        }
        if self.auto.frames == 2 {
            if let Some(t) = self.auto.new_project.take() {
                self.start.open_new_dialog();
                if let (Some(d), Some(i)) = (&mut self.start.new_dialog, modengine::scaffold::TEMPLATES.iter().position(|x| Some(x) == modengine::scaffold::template(&t))) {
                    d.template = i;
                    d.name = "Mi juego".into();
                }
            }
            if let Some(dir) = self.auto.open_project.take() {
                self.start.open_browser(Some(&dir));
            }
        }
        if self.auto.frames == 2 && !self.auto.import.is_empty() {
            let paths = std::mem::take(&mut self.auto.import);
            self.open_import(&paths, None);
        }
        if self.auto.import_run && self.auto.frames == 10 {
            self.run_import();
        }
        if self.auto.fit && self.auto.frames == 4 {
            self.ws.fit_all();
        }
        if let Some((what, _)) = &self.auto.drag {
            if self.auto.frames >= 6 {
                egui::DragAndDrop::set_payload(ctx, what.clone());
            }
        }
        let Some(path) = self.auto.screenshot.clone() else { return };
        ctx.request_repaint();
        // the Tienda de assets loads thumbnails at 1 request / 2 s: wait for them (max ~2.5 min)
        if self.auto.frames >= 30 && !self.auto.shot && (!crate::store::busy() || ctx.input(|i| i.time) > 150.0) {
            self.auto.shot = true;
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

    /// Opens the New dialog of an asset kind (Crear menu, "+" buttons, Alt shortcuts).
    fn new_dialog(&mut self, kind: NewKind) {
        let extra = match (&self.project, kind) {
            (Some(p), NewKind::TileSet) => p.sprite_names().first().cloned().unwrap_or_default(),
            _ => String::new(),
        };
        self.dialog = Some(Dialog::New { kind, name: String::new(), extra, error: None });
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        // Alt+letter: GameMaker's Create asset shortcuts (kinds.rs)
        if self.dialog.is_none() {
            if let Some(kind) = crate::kinds::shortcut(ctx) {
                self.new_dialog(kind);
            }
        }
        let (find, escape, delete) = ctx.input_mut(|i| (i.consume_key(Modifiers::COMMAND, Key::F), i.key_pressed(Key::Escape), i.key_pressed(Key::Delete)));
        // a click anywhere outside the asset browser takes the keyboard away from it
        if ctx.input(|i| i.pointer.any_pressed()) && !ctx.pointer_interact_pos().is_some_and(|p| self.browser_rect.contains(p)) {
            self.browser_focus = false;
        }
        if find {
            self.docks[1] = true;
            self.focus_filter = true;
        }
        let (new_project, open_project) = ctx.input_mut(|i| (i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::N), i.consume_key(Modifiers::COMMAND, Key::O)));
        if new_project {
            self.start.open_new_dialog();
        }
        if open_project {
            self.start.open_browser(None);
        }
        let typing = ctx.memory(|m| m.focused().is_some());
        if self.browser_focus && !typing && self.dialog.is_none() {
            if escape && !self.filter.is_empty() && !egui::DragAndDrop::has_any_payload(&ctx) {
                self.filter.clear();
            }
            if delete {
                if let Some(what) = self.selected.clone().filter(|w| *w != Open::Settings) {
                    self.dialog = Some(Dialog::Delete { what });
                }
            }
        }
        let (save, run, check, undo, redo, close, fit) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::COMMAND, Key::S),
                i.consume_key(Modifiers::NONE, Key::F5),
                i.consume_key(Modifiers::NONE, Key::F6),
                i.consume_key(Modifiers::COMMAND, Key::Z),
                i.consume_key(Modifiers::COMMAND, Key::Y) || i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z),
                i.consume_key(Modifiers::COMMAND, Key::W),
                i.consume_key(Modifiers::COMMAND, Key::Num0),
            )
        });
        if fit {
            self.ws.fit_all();
        }
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
        if !typing {
            if undo {
                if let Some(w) = self.ws.focused_mut() {
                    w.tab.undo();
                }
            }
            if redo {
                if let Some(w) = self.ws.focused_mut() {
                    w.tab.redo();
                }
            }
        }
        if close {
            if let Some(id) = self.ws.focused().map(|w| w.id) {
                self.request_close(id);
            }
        }
    }
}

impl eframe::App for EditorApp {
    /// `--drag`: keep the pointer where the fake drag is held.
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        // `--click x,y`: one click a few frames in (screenshots of menus)
        if let Some(pos) = self.auto.click {
            let ev = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
            match self.auto.frames {
                8 => raw_input.events.extend([egui::Event::PointerMoved(pos), ev(true)]),
                9 => raw_input.events.push(ev(false)),
                10.. => raw_input.events.push(egui::Event::PointerMoved(pos)),
                _ => {}
            }
        }
        if let Some((_, pos)) = &self.auto.drag {
            if self.auto.frames >= 5 {
                raw_input.events.push(egui::Event::PointerMoved(*pos));
            }
        }
    }

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
        self.update_bar(ui);
        if self.docks[2] {
            egui::Panel::bottom("output").frame(side).resizable(true).default_size(170.0).show(ui, |ui| self.output(ui));
        }
        if self.docks[3] {
            let src = Self::attach_sources(&self.project, &self.ws, &self.selected, &self.log);
            let agent = &mut self.agent;
            let send = egui::Panel::right("agent").frame(side).resizable(true).default_size(360.0).min_size(260.0).show(ui, |ui| agent.ui(ui, &src)).inner;
            if let Some(prompt) = send {
                self.send_to_agent(prompt);
            }
        }
        if self.docks[1] {
            self.browser_rect = egui::Panel::right("assets").frame(side).resizable(true).default_size(260.0).show(ui, |ui| self.asset_browser(ui)).response.rect;
        }
        if self.docks[0] {
            egui::Panel::left("inspector").frame(side).resizable(true).default_size(280.0).show(ui, |ui| self.inspector(ui));
        }
        egui::CentralPanel::default().frame(crate::theme::editor_frame(&ctx).inner_margin(0)).show(ui, |ui| self.workspace(ui));
        self.dialogs(&ctx);
        if let Some(crate::start::Action::Open(p)) = self.start.dialogs(&ctx) {
            self.request_switch(Some(p));
        }
        self.import_frame(ui);
        crate::palette::dialog_frame(&ctx, self.project.as_mut(), &mut self.log);
        crate::export::dialog_frame(&ctx, self.project.as_mut(), &mut self.log);
        // the store hides while the import dialog it opened is up, then comes back
        if self.import.is_none() {
            let files = crate::store::dialog_frame(&ctx, self.project.is_some());
            if !files.is_empty() {
                // not remembered as the browser's last folder (it is the store's cache)
                self.import.get_or_insert_with(|| crate::import::ImportDialog::new(None));
                self.open_import(&files, None);
            }
        }
        crate::dnd::ghost(&ctx);
    }
}

impl EditorApp {
    /// Thin bar under the menu while an update is being searched, built or is ready.
    fn update_bar(&mut self, ui: &mut egui::Ui) {
        use crate::update::Estado;
        let estado = self.upd.estado();
        let mostrar = match &estado {
            Estado::Inactiva | Estado::Buscando => false,
            Estado::AlDia | Estado::Error(_) | Estado::Nueva { .. } => !self.upd_oculta,
            Estado::Compilando { .. } | Estado::Lista { .. } => true,
        };
        if !mostrar {
            return;
        }
        let ctx = ui.ctx().clone();
        let pal = crate::theme::colors(&ctx);
        let mut frame = crate::theme::sidebar_frame(&ctx).inner_margin(egui::Margin::symmetric(10, 4));
        frame.fill = if ctx.global_style().visuals.dark_mode { egui::Color32::from_rgb(30, 40, 60) } else { egui::Color32::from_rgb(230, 238, 255) };
        egui::Panel::top("update_bar").frame(frame).show(ui, |ui| {
            ui.horizontal(|ui| match &estado {
                Estado::AlDia => {
                    ui.colored_label(pal.ok, format!("✔ Mod Engine está al día ({})", crate::update::descripcion_version()));
                    if ui.small_button("×").clicked() {
                        self.upd_oculta = true;
                    }
                }
                Estado::Error(e) => {
                    ui.colored_label(pal.error, format!("No se pudo actualizar: {e}"));
                    if ui.small_button("×").clicked() {
                        self.upd_oculta = true;
                    }
                }
                Estado::Nueva { ultimo } => {
                    ui.label(format!("Hay una versión nueva: {} — {}", crate::update::corto(&ultimo.sha), ultimo.mensaje.clone().unwrap_or_default()));
                    if ui.add(crate::theme::primary_button(ui, "Compilar e instalar")).clicked() {
                        if let Err(e) = self.upd.instalar() {
                            self.info(LogKind::Error, e);
                        }
                    }
                    if ui.small_button("Ahora no").clicked() {
                        self.upd_oculta = true;
                    }
                }
                Estado::Compilando { paso, total, texto } => {
                    ui.spinner();
                    let t = if *total > 0 { format!("Actualizando Mod Engine en segundo plano · [{paso}/{total}] {texto}") } else { "Actualizando Mod Engine en segundo plano…".into() };
                    ui.label(t);
                }
                Estado::Lista { commit } => {
                    ui.colored_label(pal.ok, format!("La versión {commit} está lista. Se aplica al reiniciar el editor."));
                    if ui.add(crate::theme::primary_button(ui, "Reiniciar ahora")).clicked() {
                        self.save_all();
                        self.upd.aplicar();
                        self.quitting = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
                _ => {}
            });
        });
    }
}

// ------------------------------------------------------------------ importing external files

impl EditorApp {
    /// Opens the Importar assets dialog (or adds to the open one) with these files or folders.
    fn open_import(&mut self, paths: &[PathBuf], filter: Option<crate::import::Kind>) {
        let Some(p) = &self.project else { return };
        let owners = crate::import::owners(p);
        let d = self.import.get_or_insert_with(|| crate::import::ImportDialog::new(crate::import::last_dir_file()));
        if filter.is_some() {
            d.filter = filter;
        }
        for path in paths {
            d.add(path, &owners);
        }
    }

    /// The import dialog and files dropped from the OS (after the Agente panel had its turn).
    fn import_frame(&mut self, ui: &egui::Ui) {
        let ctx = ui.ctx().clone();
        let (hovered, dropped, pointer) = ctx.input(|i| (i.raw.hovered_files.len(), i.raw.dropped_files.clone(), i.pointer.latest_pos()));
        if hovered > 0 || !dropped.is_empty() {
            let agent = if self.docks[3] { crate::import::panel_rect(ui, "agent") } else { None };
            if crate::import::route_drop(pointer, agent, self.project.is_some()) == crate::import::DropTarget::Import {
                if hovered > 0 {
                    crate::import::paint_drop_hint(&ctx, hovered);
                }
                let paths: Vec<PathBuf> = dropped.iter().map(|f| f.path().to_path_buf()).collect();
                // a project folder or its game.toml opens that project instead
                if let Some(dir) = crate::projects::dropped_project(&paths) {
                    self.request_switch(Some(dir));
                } else if !paths.is_empty() {
                    self.open_import(&paths, None);
                }
            }
        }
        let (Some(d), Some(p)) = (&mut self.import, &self.project) else { return };
        let owners = crate::import::owners(p);
        match d.show(&ctx, &owners) {
            crate::import::Outcome::Open => {}
            crate::import::Outcome::Cancel => self.import = None,
            crate::import::Outcome::Import => self.run_import(),
        }
    }

    /// The dialog's Importar button.
    fn run_import(&mut self) {
        let (Some(d), Some(p)) = (&mut self.import, &self.project) else { return };
        let done = d.run(&p.dir);
        if d.items.is_empty() {
            self.import = None;
        }
        self.finish_import(done);
    }

    /// After importing: reload the lists, refresh editors of replaced assets, select the last
    /// one, log it and check the project with the engine.
    fn finish_import(&mut self, done: Vec<crate::import::Imported>) {
        use crate::import::OpenImported;
        if done.is_empty() {
            return;
        }
        let Some(p) = &mut self.project else { return };
        p.reload_lists();
        let mut lines = vec![];
        for imp in &done {
            lines.push(LogLine { kind: LogKind::Ok, text: format!("Importado: {} '{}' → {}", imp.kind.label(), imp.name, imp.files.join(", ")) });
            if let Some(Err(e)) = p.sprite_entry(&imp.name).map(|s| &s.data).filter(|_| imp.kind == crate::import::Kind::Sprite) {
                lines.push(LogLine { kind: LogKind::Error, text: e.clone() });
            }
            let Some(what) = imp.open().filter(|_| imp.replaced) else { continue };
            for w in self.ws.wins_mut() {
                if w.tab.open_key() == what && !w.tab.dirty(p) {
                    if let Ok(tab) = make_tab(p, &what) {
                        w.tab = tab;
                    }
                }
            }
        }
        if let Some(what) = done.iter().rev().find_map(|i| i.open()) {
            if let Some(k) = section_key(&what) {
                self.folded.insert(k, true);
            }
            self.selected = Some(what);
        }
        if let Some(dir) = self.project.as_ref().map(|p| p.dir.clone()) {
            lines.extend(crate::store::credit_imported(&dir, &done));
        }
        self.log.extend(lines);
        self.check(false);
    }
}

impl Drop for EditorApp {
    fn drop(&mut self) {
        self.upd.al_salir();
        self.save_layout();
        self.stop_game();
        self.agent.close_project(); // kills a running agent
    }
}
