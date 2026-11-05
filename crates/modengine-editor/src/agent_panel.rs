//! The Agente dock: chats with a coding-agent CLI (see `agent.rs`) that works in the project
//! folder. The CLI runs as a child process; reader threads send its output lines over a
//! channel that [`AgentPanel::poll`] drains every frame, so the UI never blocks.
//!
//! The app saves everything before each message and, when [`AgentPanel::poll`] says so,
//! reloads the project from disk and checks it.

use crate::agent::{self, Event, Kind, Model, Parser, Permission, Run, ToolLine};
use eframe::egui::{self, Color32, Key, Modifiers, RichText};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

// ------------------------------------------------------------------ settings

/// `~/.config/modengine/agent.toml`: the chosen agent, mode and model per agent, and the last
/// model list each CLI gave (so the chosen model shows by its name right away).
#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
pub struct Config {
    #[serde(default)]
    pub agent: String,
    #[serde(default)]
    pub permission: String,
    /// Agent id -> model id (missing or empty: the CLI's default).
    #[serde(default)]
    pub model: BTreeMap<String, String>,
    /// Agent id -> its CLI's last model list.
    #[serde(default)]
    pub models: BTreeMap<String, Vec<Model>>,
}

pub fn config_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/modengine/agent.toml"))
}

impl Config {
    pub fn load(path: &Path) -> Config {
        std::fs::read_to_string(path).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) {
        if let (Some(d), Ok(text)) = (path.parent(), toml::to_string(self)) {
            let _ = std::fs::create_dir_all(d);
            let _ = std::fs::write(path, text);
        }
    }
}

// ------------------------------------------------------------------ helpers

/// Desktop launchers often start the editor without the login shell's PATH, so also look
/// where the agent installers put their executables.
pub fn find_program(name: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        dirs.extend([home.join(".local/bin"), home.join(".claude/local"), home.join(".npm-global/bin")]);
    }
    dirs.extend(["/usr/local/bin", "/opt/homebrew/bin"].map(PathBuf::from));
    if let Some(appdata) = std::env::var_os("APPDATA") {
        dirs.push(PathBuf::from(appdata).join("npm"));
    }
    let exts: &[&str] = if cfg!(windows) { &[".exe", ".cmd", ".bat"] } else { &[""] };
    dirs.iter().flat_map(|d| exts.iter().map(move |e| d.join(format!("{name}{e}")))).find(|p| is_executable(p))
}

fn is_executable(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        p.metadata().map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

/// The engine docs (`lenguaje.md`, `funciones.md`, `proyecto.md`): `$MODENGINE_DOCS`, next to
/// the executable, or the source tree the editor was built from.
pub fn docs_dir() -> Option<PathBuf> {
    let mut c: Vec<PathBuf> = vec![];
    if let Some(d) = std::env::var_os("MODENGINE_DOCS") {
        c.push(d.into());
    }
    if let Some(exe) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        c.extend([exe.join("docs"), exe.join("../docs"), exe.join("../share/modengine/docs"), exe.join("../../docs")]);
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        c.extend([home.join(".local/share/ModEngine/codigo/docs"), home.join(".local/share/ModEngine/docs")]);
    }
    c.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs"));
    c.into_iter().find(|d| d.join("funciones.md").is_file()).map(|d| d.canonicalize().unwrap_or(d))
}

/// Signals the whole process group (the CLI and the commands it started).
#[cfg(unix)]
fn signal_group(child: &Child, sig: i32) {
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    // SAFETY: plain libc call; the child leads its own process group (see `spawn`).
    unsafe {
        kill(-(child.id() as i32), sig);
    }
}

const SIGTERM: i32 = 15;
const SIGKILL: i32 = 9;

fn hard_kill(child: &mut Child) {
    #[cfg(unix)]
    signal_group(child, SIGKILL);
    let _ = child.kill();
    let _ = child.wait();
}

// ------------------------------------------------------------------ transcript

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tone {
    Muted,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    User(String),
    Text(String),
    Tool(ToolLine),
    Notice(String, Tone),
}

/// What the app must do after [`AgentPanel::poll`].
#[derive(Default, Debug, PartialEq)]
pub struct Polled {
    /// The agent changed files (or ran commands that may have): reload the project from disk.
    pub reload: bool,
    /// A run ended: check the project.
    pub finished: bool,
    /// Project files the agent reported writing during the run that ended.
    pub changed: Vec<PathBuf>,
}

enum Msg {
    Line(String),
    StdoutEnd,
    Stderr(String),
}

struct Running {
    child: Child,
    rx: Receiver<Msg>,
    parser: Parser,
    kind: Kind,
    started: Instant,
    stop_at: Option<Instant>,
    exited_at: Option<Instant>,
    stdout_done: bool,
    got_result: bool,
    stderr: String,
    changed: BTreeSet<PathBuf>,
    blocked: usize,
    context_file: PathBuf,
    root: PathBuf,
}

struct Lister {
    kind: Kind,
    rx: Receiver<Vec<Model>>,
}

pub struct AgentPanel {
    pub kind: Kind,
    pub permission: Permission,
    cfg: Config,
    /// Where settings are remembered; `None` in tests.
    config_path: Option<PathBuf>,
    /// The CLI found for `kind`.
    pub program: Option<PathBuf>,
    /// Use this executable for every agent instead of looking it up (tests).
    pub program_override: Option<PathBuf>,
    pub entries: Vec<Entry>,
    /// The streaming reply, as an index into `entries`.
    open_text: Option<usize>,
    pub input: String,
    other_model: String,
    pub session: String,
    session_root: Option<PathBuf>,
    answering_model: String,
    pub status: String,
    run: Option<Running>,
    lister: Option<Lister>,
    /// Lists from this session; failed lookups are not retried until the menu opens again.
    listed: HashMap<Kind, Vec<Model>>,
    list_tried: bool,
}

impl Default for AgentPanel {
    fn default() -> Self {
        AgentPanel::with_config(None)
    }
}

impl AgentPanel {
    /// Loads the remembered choices from `config` (none: defaults, nothing saved).
    pub fn with_config(config: Option<PathBuf>) -> AgentPanel {
        let cfg = config.as_deref().map(Config::load).unwrap_or_default();
        let kind = Kind::from_id(&cfg.agent).unwrap_or(Kind::Claude);
        let permission = Permission::from_key(&cfg.permission).unwrap_or(Permission::Auto);
        let mut p = AgentPanel {
            kind,
            permission,
            cfg,
            config_path: config,
            program: None,
            program_override: None,
            entries: vec![],
            open_text: None,
            input: String::new(),
            other_model: String::new(),
            session: String::new(),
            session_root: None,
            answering_model: String::new(),
            status: String::new(),
            run: None,
            lister: None,
            listed: HashMap::new(),
            list_tried: false,
        };
        p.locate();
        p
    }

    pub fn running(&self) -> bool {
        self.run.is_some()
    }

    fn save_cfg(&mut self) {
        self.cfg.agent = self.kind.id().into();
        self.cfg.permission = self.permission.key().into();
        if let Some(path) = &self.config_path {
            self.cfg.save(path);
        }
    }

    pub fn locate(&mut self) {
        self.program = self.program_override.clone().or_else(|| find_program(self.kind.program()));
    }

    pub fn set_kind(&mut self, kind: Kind) {
        if kind == self.kind {
            return;
        }
        self.kind = kind;
        self.answering_model.clear();
        // Conversations belong to one CLI.
        if !self.session.is_empty() {
            self.session.clear();
            self.notice(format!("Ahora con {}: esto empieza una conversación nueva.", kind.name()), Tone::Muted);
        }
        self.list_tried = false;
        self.locate();
        self.save_cfg();
    }

    pub fn set_permission(&mut self, p: Permission) {
        self.permission = p;
        self.save_cfg();
    }

    /// The chosen model for the current agent; empty for the CLI's default.
    pub fn model(&self) -> String {
        self.cfg.model.get(self.kind.id()).cloned().unwrap_or_default()
    }

    pub fn set_model(&mut self, id: &str) {
        self.cfg.model.insert(self.kind.id().into(), id.trim().into());
        self.answering_model.clear();
        self.save_cfg();
    }

    /// This session's list, else the one the CLI gave last time, else the built-in one.
    pub fn available_models(&self) -> Vec<Model> {
        if let Some(m) = self.listed.get(&self.kind).filter(|m| !m.is_empty()) {
            return m.clone();
        }
        match self.cfg.models.get(self.kind.id()).filter(|m| !m.is_empty()) {
            Some(m) => m.clone(),
            None => self.kind.builtin_models(),
        }
    }

    fn model_name(&self, id: &str) -> String {
        self.available_models().into_iter().find(|m| m.id == id).map(|m| m.name).unwrap_or_else(|| id.to_string())
    }

    /// Asks the CLI for its models in the background (`codex debug models`, `agy models`).
    fn list_models(&mut self) {
        let args = self.kind.list_models_args();
        if args.is_empty() || self.lister.is_some() || self.listed.contains_key(&self.kind) || self.list_tried {
            return;
        }
        let Some(program) = self.program.clone() else { return };
        self.list_tried = true;
        let kind = self.kind;
        let (tx, rx) = mpsc::channel();
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        std::thread::spawn(move || {
            let out = (|| {
                let mut child = Command::new(program).args(&args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
                let mut stdout = child.stdout.take()?;
                let reader = std::thread::spawn(move || {
                    let mut s = String::new();
                    let _ = stdout.read_to_string(&mut s);
                    s
                });
                // Listing may go over the network; a CLI that hangs is given up on.
                let start = Instant::now();
                loop {
                    match child.try_wait() {
                        Ok(Some(st)) if st.success() => break,
                        Ok(Some(_)) | Err(_) => return None,
                        Ok(None) if start.elapsed() > Duration::from_secs(30) => {
                            let _ = child.kill();
                            let _ = child.wait();
                            return None;
                        }
                        Ok(None) => std::thread::sleep(Duration::from_millis(100)),
                    }
                }
                reader.join().ok()
            })();
            let _ = tx.send(out.map(|o| kind.parse_models(&o)).unwrap_or_default());
        });
        self.lister = Some(Lister { kind, rx });
    }

    // ------------------------------------------------------------ transcript

    fn notice(&mut self, text: impl Into<String>, tone: Tone) {
        self.open_text = None;
        self.entries.push(Entry::Notice(text.into().trim().to_string(), tone));
    }

    pub fn new_conversation(&mut self) {
        if let Some(mut r) = self.run.take() {
            hard_kill(&mut r.child);
            let _ = std::fs::remove_file(&r.context_file);
        }
        self.entries.clear();
        self.open_text = None;
        self.session.clear();
        self.session_root = None;
        self.answering_model.clear();
        self.status.clear();
    }

    /// The project closes: stop the agent and forget the conversation.
    pub fn close_project(&mut self) {
        self.new_conversation();
    }

    // ------------------------------------------------------------ running

    /// Starts the agent on `prompt` in `root`. The app saves everything first.
    pub fn send(&mut self, prompt: &str, root: &Path, name: &str) {
        let text = prompt.trim();
        if text.is_empty() || self.running() {
            return;
        }
        let Some(program) = self.program.clone() else { return };
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        if !self.session.is_empty() && self.session_root.as_deref() != Some(&root) {
            self.session.clear();
            self.notice("El proyecto cambió: esto empieza una conversación nueva.", Tone::Muted);
        }
        let docs = docs_dir();
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let context_file = std::env::temp_dir().join(format!("modengine-agent-{}-{n}.md", std::process::id()));
        if let Err(e) = std::fs::write(&context_file, agent::engine_context(name, docs.as_deref())) {
            self.notice(format!("No se pudo escribir un fichero temporal: {e}"), Tone::Error);
            return;
        }
        let run = Run {
            prompt: text.to_string(),
            session: self.session.clone(),
            model: self.model(),
            permission: self.permission,
            root: root.clone(),
            context_file: context_file.clone(),
            docs,
            name: name.to_string(),
        };
        self.entries.push(Entry::User(text.to_string()));
        self.open_text = None;
        match self.spawn(&program, &run) {
            Ok((child, rx)) => {
                self.input.clear();
                self.session_root = Some(root.clone());
                self.status = "Trabajando…".into();
                self.run = Some(Running {
                    child,
                    rx,
                    parser: Parser::new(self.kind, &root),
                    kind: self.kind,
                    started: Instant::now(),
                    stop_at: None,
                    exited_at: None,
                    stdout_done: false,
                    got_result: false,
                    stderr: String::new(),
                    changed: BTreeSet::new(),
                    blocked: 0,
                    context_file,
                    root,
                });
            }
            Err(e) => {
                let _ = std::fs::remove_file(&context_file);
                self.notice(format!("No se pudo lanzar {}: {e}", program.display()), Tone::Error);
                self.status = "Falló".into();
            }
        }
    }

    fn spawn(&self, program: &Path, run: &Run) -> std::io::Result<(Child, Receiver<Msg>)> {
        let mut cmd = Command::new(program);
        cmd.args(self.kind.args(run)).current_dir(&run.root).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        // Its own process group, so Detener also stops the commands it started.
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        let mut child = cmd.spawn()?;
        let (tx, rx): (Sender<Msg>, Receiver<Msg>) = mpsc::channel();
        let input = self.kind.input(run);
        if let Some(mut stdin) = child.stdin.take() {
            // A thread, so a long prompt never blocks the UI; dropping it closes stdin.
            std::thread::spawn(move || {
                let _ = stdin.write_all(input.as_bytes());
            });
        }
        if let Some(out) = child.stdout.take() {
            let tx = tx.clone();
            std::thread::spawn(move || {
                for l in BufReader::new(out).lines().map_while(Result::ok) {
                    let _ = tx.send(Msg::Line(l));
                }
                let _ = tx.send(Msg::StdoutEnd);
            });
        }
        if let Some(err) = child.stderr.take() {
            std::thread::spawn(move || {
                for l in BufReader::new(err).lines().map_while(Result::ok) {
                    let _ = tx.send(Msg::Stderr(l));
                }
            });
        }
        Ok((child, rx))
    }

    /// Detener: asks politely, then kills after 3 s.
    pub fn stop(&mut self) {
        let Some(r) = &mut self.run else { return };
        if r.stop_at.is_some() {
            return;
        }
        r.stop_at = Some(Instant::now());
        #[cfg(unix)]
        signal_group(&r.child, SIGTERM);
        #[cfg(not(unix))]
        let _ = r.child.kill();
        self.status = "Deteniendo…".into();
    }

    /// Reads what the agent printed since the last frame. Call every frame.
    pub fn poll(&mut self, ctx: &egui::Context) -> Polled {
        let mut out = Polled::default();
        if let Some(l) = &self.lister {
            if let Ok(models) = l.rx.try_recv() {
                let kind = l.kind;
                self.lister = None;
                if !models.is_empty() {
                    self.cfg.models.insert(kind.id().into(), models.clone());
                    self.listed.insert(kind, models);
                    self.save_cfg();
                }
            } else {
                ctx.request_repaint_after(Duration::from_millis(200));
            }
        }
        let Some(mut r) = self.run.take() else { return out };
        let mut events = vec![];
        for m in r.rx.try_iter() {
            match m {
                Msg::Line(l) => {
                    let l = l.trim();
                    if l.is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<serde_json::Value>(l) {
                        Ok(v) if v.is_object() => events.extend(r.parser.parse(&v).into_iter().map(Some)),
                        _ => events.push(Some(Event::Notice(l.to_string()))),
                    }
                }
                Msg::StdoutEnd => r.stdout_done = true,
                Msg::Stderr(l) => {
                    r.stderr.push_str(&l);
                    r.stderr.push('\n');
                    if r.stderr.len() > 16 * 1024 {
                        let cut = r.stderr.len() - 16 * 1024;
                        let cut = (cut..r.stderr.len()).find(|i| r.stderr.is_char_boundary(*i)).unwrap_or(0);
                        r.stderr.drain(..cut);
                    }
                }
            }
        }
        for e in events.into_iter().flatten() {
            if self.handle(&mut r, e) {
                out.reload = true;
            }
        }
        if let Some(t) = r.stop_at {
            if t.elapsed() > Duration::from_secs(3) {
                #[cfg(unix)]
                signal_group(&r.child, SIGKILL);
                let _ = r.child.kill();
            }
        }
        let exited = match r.child.try_wait() {
            Ok(Some(st)) => Some(st),
            Ok(None) => None,
            Err(_) => r.child.wait().ok(),
        };
        if let Some(st) = exited {
            r.exited_at.get_or_insert_with(Instant::now);
            // Its output may still be in the pipe; a grandchild that keeps it open is not waited for.
            if r.stdout_done || r.exited_at.is_some_and(|t| t.elapsed() > Duration::from_secs(2)) {
                out.changed = r.changed.iter().cloned().collect();
                self.finish(r, st);
                out.reload = true;
                out.finished = true;
                return out;
            }
        }
        self.run = Some(r);
        ctx.request_repaint_after(Duration::from_millis(50));
        out
    }

    fn finish(&mut self, mut r: Running, status: std::process::ExitStatus) {
        // Whatever stderr the reader thread still had.
        for m in r.rx.try_iter() {
            if let Msg::Stderr(l) = m {
                r.stderr.push_str(&l);
                r.stderr.push('\n');
            }
        }
        let _ = std::fs::remove_file(&r.context_file);
        self.open_text = None;
        let name = r.kind.name();
        if r.stop_at.is_some() {
            self.notice("Detenido.", Tone::Muted);
            self.status = "Detenido".into();
        } else if !r.got_result {
            // Bad arguments, a crash or a missing login: the reason is on stderr.
            let lines: Vec<&str> = r.stderr.trim().lines().collect();
            let details = lines[lines.len().saturating_sub(12)..].join("\n");
            let what = match status.code() {
                Some(c) => format!("{name} terminó con código {c}"),
                None => format!("{name} se detuvo inesperadamente"),
            };
            self.notice(if details.trim().is_empty() { format!("{what}.") } else { format!("{what}:\n{details}") }, Tone::Error);
            self.status = "Falló".into();
        } else {
            // Fallbacks such as a mode the account cannot use only show up as warnings on stderr.
            for l in r.stderr.lines().filter(|l| l.to_lowercase().starts_with("warning:")) {
                self.notice(l, Tone::Warning);
            }
        }
        if r.blocked > 0 && r.stop_at.is_none() {
            self.notice("Algunas acciones se bloquearon. Elige qué puede hacer el agente en el menú de permisos.", Tone::Muted);
        }
    }

    /// Applies one event; true when the project should be reloaded from disk now.
    fn handle(&mut self, r: &mut Running, e: Event) -> bool {
        match e {
            Event::Session(s) => self.session = s,
            Event::TextBegin => {
                self.entries.push(Entry::Text(String::new()));
                self.open_text = Some(self.entries.len() - 1);
            }
            Event::TextDelta(d) => {
                if let Some(Entry::Text(t)) = self.open_text.and_then(|i| self.entries.get_mut(i)) {
                    t.push_str(&d);
                }
            }
            Event::Text(t) => {
                match self.open_text.and_then(|i| self.entries.get_mut(i)) {
                    Some(Entry::Text(open)) => *open = t,
                    _ if !t.trim().is_empty() => self.entries.push(Entry::Text(t)),
                    _ => {}
                }
                self.open_text = None;
            }
            Event::Tool(t) => {
                self.open_text = None;
                self.status = if t.detail.is_empty() { format!("{}…", t.verb) } else { format!("{} {}…", t.verb, t.detail) };
                self.entries.push(Entry::Tool(t));
            }
            Event::Status(s) => self.status = s,
            Event::ToolFailed(s) => self.notice(s, Tone::Warning),
            Event::Blocked(s) => {
                r.blocked += 1;
                self.notice(format!("Bloqueado por los permisos: {s}"), Tone::Warning);
            }
            Event::Model(m) => {
                if m != self.answering_model {
                    self.notice(format!("Modelo: {m}"), Tone::Muted);
                    self.answering_model = m;
                }
            }
            Event::FileChanged(p) => {
                if agent::inside(&r.root, &p) {
                    r.changed.insert(p);
                    return true;
                }
            }
            Event::CommandRan => return true,
            Event::Notice(s) => self.notice(s, Tone::Muted),
            Event::Finished => {
                r.got_result = true;
                self.status = format!("Hecho en {:.1} s", r.started.elapsed().as_secs_f32());
            }
            Event::Failed(s) => {
                r.got_result = true;
                self.notice(s, Tone::Error);
                self.status = "Falló".into();
            }
        }
        false
    }

    // ------------------------------------------------------------ ui

    /// Draws the dock. Returns a message to send: the app saves everything, then calls [`AgentPanel::send`].
    pub fn ui(&mut self, ui: &mut egui::Ui, has_project: bool) -> Option<String> {
        let pal = crate::theme::colors(ui.ctx());
        let running = self.running();
        let mut send = None;

        // Toolbar: agent, new conversation, stop, status.
        ui.horizontal(|ui| {
            ui.strong("Agente");
            ui.add_enabled_ui(!running, |ui| {
                let mut kind = self.kind;
                egui::ComboBox::from_id_salt("agent_kind").selected_text(self.kind.name()).show_ui(ui, |ui| {
                    for k in Kind::ALL {
                        let installed = self.program_override.is_some() || find_program(k.program()).is_some();
                        let label = if installed { k.name().to_string() } else { format!("{} (no instalado)", k.name()) };
                        ui.selectable_value(&mut kind, k, label);
                    }
                });
                self.set_kind(kind);
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(running, egui::Button::new("■ Detener")).on_hover_text("Detiene el agente").clicked() {
                    self.stop();
                }
                if ui.add_enabled(self.program.is_some(), egui::Button::new("＋ Nueva")).on_hover_text("Nueva conversación").clicked() {
                    self.new_conversation();
                }
                if !self.session.is_empty()
                    && ui.small_button("⧉").on_hover_text("Copiar la orden para seguir esta conversación en un terminal abierto en la carpeta del proyecto").clicked()
                {
                    ui.ctx().copy_text(self.kind.resume_command(&self.session));
                }
            });
        });
        ui.horizontal(|ui| {
            if running {
                ui.spinner();
            }
            let status = if self.program.is_none() { format!("{} no encontrado", self.kind.name()) } else { self.status.clone() };
            ui.add(egui::Label::new(RichText::new(status).weak()).truncate());
        });
        ui.separator();

        let Some(_) = self.program.clone() else {
            self.missing_ui(ui);
            return None;
        };

        // Message box and its footer (model, permissions, send) at the bottom.
        let input_id = ui.make_persistent_id("agent_input");
        egui::Panel::bottom("agent_input_area").frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(0, 6))).show(ui, |ui| {
            let enter = ui.memory(|m| m.has_focus(input_id)) && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
            ui.add(
                egui::TextEdit::multiline(&mut self.input)
                    .id(input_id)
                    .hint_text(format!("Mensaje para {} (Mayús+Intro: nueva línea)", self.kind.name()))
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            );
            ui.horizontal_wrapped(|ui| {
                ui.add_enabled_ui(!running, |ui| {
                    self.model_combo(ui);
                    let mut perm = self.permission;
                    egui::ComboBox::from_id_salt("agent_perm").selected_text(perm.label()).show_ui(ui, |ui| {
                        for p in Permission::ALL {
                            ui.selectable_value(&mut perm, p, p.label()).on_hover_text(p.tip());
                        }
                    });
                    if perm != self.permission {
                        self.set_permission(perm);
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let can = !running && has_project && !self.input.trim().is_empty();
                    let b = ui.add_enabled(can, crate::theme::primary_button(ui, "Enviar")).on_hover_text("Intro: guarda todo y envía el mensaje");
                    if b.clicked() || (enter && can) {
                        send = Some(self.input.clone());
                    }
                });
            });
            if !has_project {
                ui.label(RichText::new("Abre un proyecto: el agente trabaja en su carpeta.").color(pal.error));
            }
        });

        // Transcript.
        egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
            if self.entries.is_empty() {
                ui.label(RichText::new("Pide al agente que cambie objetos, rooms, sprites o scripts del proyecto.").weak());
            }
            for e in &self.entries {
                match e {
                    Entry::User(t) => {
                        ui.add_space(8.0);
                        egui::Frame::new()
                            .fill(ui.visuals().faint_bg_color)
                            .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                            .corner_radius(crate::theme::RADIUS)
                            .inner_margin(6)
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.label(t);
                            });
                    }
                    Entry::Text(t) => {
                        ui.add_space(4.0);
                        text_ui(ui, t);
                    }
                    Entry::Tool(t) => {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            ui.label(RichText::new(t.icon).weak());
                            ui.label(RichText::new(&t.verb).strong());
                            if !t.detail.is_empty() {
                                ui.label(RichText::new(&t.detail).weak().monospace());
                            }
                        });
                    }
                    Entry::Notice(t, tone) => {
                        let (icon, c) = match tone {
                            Tone::Muted => ("ℹ", pal.weak),
                            Tone::Warning => ("⚠", Color32::from_rgb(0xc0, 0x85, 0x32)),
                            Tone::Error => ("⛔", pal.error),
                        };
                        ui.label(RichText::new(format!("{icon} {t}")).color(c));
                    }
                }
            }
        });
        if !self.list_tried && self.lister.is_none() {
            self.list_models();
        }
        send
    }

    fn model_combo(&mut self, ui: &mut egui::Ui) {
        let current = self.model();
        let label = if current.is_empty() { "Modelo por defecto".to_string() } else { self.model_name(&current) };
        let mut pick: Option<String> = None;
        let r = egui::ComboBox::from_id_salt("agent_model").selected_text(label).show_ui(ui, |ui| {
            if ui.selectable_label(current.is_empty(), "Por defecto").on_hover_text(format!("El modelo configurado en {}", self.kind.name())).clicked() {
                pick = Some(String::new());
            }
            ui.separator();
            let models = self.available_models();
            let known = current.is_empty() || models.iter().any(|m| m.id == current);
            for m in &models {
                let tip = if m.description.is_empty() { m.id.clone() } else { format!("{} ({})", m.description, m.id) };
                if ui.selectable_label(m.id == current, &m.name).on_hover_text(tip).clicked() {
                    pick = Some(m.id.clone());
                }
            }
            if !known {
                ui.selectable_label(true, &current).on_hover_text("Elegido con «Otro modelo»");
            }
            if self.lister.is_some() {
                ui.label(RichText::new(format!("Buscando los modelos de {}…", self.kind.name())).weak());
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.other_model).hint_text("Otro modelo…").desired_width(130.0))
                    .on_hover_text(format!("Cualquier nombre que {} acepte tras --model", self.kind.program()));
                if ui.add_enabled(!self.other_model.trim().is_empty(), egui::Button::new("Usar")).clicked() {
                    pick = Some(std::mem::take(&mut self.other_model));
                }
            });
        });
        // Opening the menu retries a lookup that failed (offline, not signed in).
        if r.response.clicked() && self.lister.is_none() && !self.listed.contains_key(&self.kind) {
            self.list_tried = false;
        }
        if let Some(id) = pick {
            self.set_model(&id);
        }
    }

    fn missing_ui(&mut self, ui: &mut egui::Ui) {
        let k = self.kind;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.heading(format!("{} no está instalado", k.name()));
            ui.label(format!(
                "Este panel conversa con {}, que trabaja en la carpeta del proyecto para escribir y editar sus objetos, rooms, sprites y scripts. \
                 No se encontró la orden `{}` en este ordenador.",
                k.name(),
                k.program()
            ));
            ui.add_space(4.0);
            ui.label(format!("Instálalo y luego inicia sesión ejecutando `{}` una vez en un terminal:", k.program()));
            for (place, cmd) in k.install_commands() {
                ui.add_space(4.0);
                ui.label(RichText::new(*place).weak());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(*cmd).monospace());
                    if ui.small_button("⧉").on_hover_text("Copiar").clicked() {
                        ui.ctx().copy_text(cmd.to_string());
                    }
                });
            }
            ui.add_space(8.0);
            ui.label("O elige otro agente en el menú de arriba.");
            if ui.button("Comprobar de nuevo").clicked() {
                self.locate();
            }
        });
    }
}

/// A reply: plain text with fenced code blocks in monospace and `#` headings in bold.
fn text_ui(ui: &mut egui::Ui, text: &str) {
    let mut in_code = false;
    let mut block = String::new();
    let flush = |ui: &mut egui::Ui, block: &mut String, code: bool| {
        let t = block.trim_end_matches('\n');
        if !t.is_empty() {
            if code {
                egui::Frame::new().fill(ui.visuals().extreme_bg_color).corner_radius(crate::theme::RADIUS).inner_margin(4).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(t).monospace());
                });
            } else {
                for line in t.lines() {
                    let h = line.trim_start_matches('#');
                    if h.len() < line.len() && h.starts_with(' ') {
                        ui.label(RichText::new(h.trim()).strong());
                    } else {
                        let l = line.replace("**", "");
                        let l = match l.strip_prefix("- ").or_else(|| l.strip_prefix("* ")) {
                            Some(rest) => format!("• {rest}"),
                            None => l,
                        };
                        ui.label(l);
                    }
                }
            }
        }
        block.clear();
    };
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            flush(ui, &mut block, in_code);
            in_code = !in_code;
            continue;
        }
        block.push_str(line);
        block.push('\n');
    }
    flush(ui, &mut block, in_code);
}

impl Drop for AgentPanel {
    fn drop(&mut self) {
        if let Some(mut r) = self.run.take() {
            hard_kill(&mut r.child);
            let _ = std::fs::remove_file(&r.context_file);
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A folder with a fake agent CLI: it records its arguments and stdin, then prints `script`.
    fn fake_cli(tag: &str, body: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("modeditor_agent_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("claude");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{d}/args.txt'\ncat > '{d}/stdin.txt'\n{body}\n",
            d = dir.display()
        );
        std::fs::write(&exe, script).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        (dir, exe)
    }

    fn panel(exe: &Path) -> AgentPanel {
        let mut p = AgentPanel::with_config(None);
        p.program_override = Some(exe.to_path_buf());
        p.locate();
        p.list_tried = true; // no model lookups in tests
        p
    }

    /// Polls until the run ends; returns whether any poll asked for a reload.
    fn wait(p: &mut AgentPanel) -> (bool, bool) {
        let ctx = egui::Context::default();
        let (mut reload, mut finished) = (false, false);
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(15) {
            let r = p.poll(&ctx);
            reload |= r.reload;
            finished |= r.finished;
            if !p.running() {
                return (reload, finished);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("the fake agent did not finish");
    }

    fn lines(path: &Path) -> Vec<String> {
        std::fs::read_to_string(path).unwrap().lines().map(String::from).collect()
    }

    #[test]
    fn runs_fake_claude_streams_and_resumes() {
        let project = crate::room::tests::temp_project("agent");
        let project = project.canonicalize().unwrap();
        let target = project.join("objects/world.ms");
        let body = format!(
            r#"echo '{{"type":"system","subtype":"init","session_id":"S-fake"}}'
echo '{{"type":"stream_event","session_id":"S-fake","event":{{"type":"message_start","message":{{"model":"claude-test"}}}}}}'
echo '{{"type":"stream_event","session_id":"S-fake","event":{{"type":"content_block_start","content_block":{{"type":"text"}}}}}}'
echo '{{"type":"stream_event","session_id":"S-fake","event":{{"type":"content_block_delta","delta":{{"type":"text_delta","text":"Hay "}}}}}}'
echo '{{"type":"stream_event","session_id":"S-fake","event":{{"type":"content_block_delta","delta":{{"type":"text_delta","text":"objetos"}}}}}}'
echo '{{"type":"assistant","session_id":"S-fake","message":{{"model":"claude-test","content":[{{"type":"text","text":"Hay objetos."}},{{"type":"tool_use","id":"t1","name":"Edit","input":{{"file_path":"{t}"}}}}]}}}}'
echo '// edited by the agent' >> '{t}'
echo '{{"type":"user","session_id":"S-fake","message":{{"content":[{{"type":"tool_result","tool_use_id":"t1","content":"ok"}}]}}}}'
echo 'not json'
echo '{{"type":"result","subtype":"success","session_id":"S-fake","result":"Hay objetos."}}'"#,
            t = target.display()
        );
        let (dir, exe) = fake_cli("ok", &body);
        let mut p = panel(&exe);
        p.input = "lista los objetos".into();
        p.send("lista los objetos", &project, "Coin Climber");
        assert!(p.running() && p.input.is_empty());
        let (reload, finished) = wait(&mut p);
        assert!(reload && finished);

        // Default mode is "Editar y ejecutar comandos"; the prompt goes on stdin; the context file is gone.
        let args = lines(&dir.join("args.txt"));
        assert_eq!(&args[..7], ["--print", "--output-format", "stream-json", "--verbose", "--include-partial-messages", "--permission-mode", "auto"]);
        assert_eq!(args[7], "--append-system-prompt-file");
        assert!(!Path::new(&args[8]).exists(), "context file not cleaned up");
        assert!(!args.contains(&"--resume".to_string()));
        assert_eq!(std::fs::read_to_string(dir.join("stdin.txt")).unwrap(), "lista los objetos");

        assert_eq!(p.session, "S-fake");
        assert_eq!(
            p.entries,
            vec![
                Entry::User("lista los objetos".into()),
                Entry::Notice("Modelo: claude-test".into(), Tone::Muted),
                Entry::Text("Hay objetos.".into()),
                Entry::Tool(ToolLine { icon: "✎", verb: "Editar".into(), detail: "objects/world.ms".into() }),
                Entry::Notice("not json".into(), Tone::Muted),
            ]
        );
        assert!(p.status.starts_with("Hecho en"), "{}", p.status);
        assert!(std::fs::read_to_string(&target).unwrap().ends_with("// edited by the agent\n"));

        // The next message continues the conversation, in plan mode with a chosen model.
        p.set_permission(Permission::Plan);
        p.set_model("haiku");
        p.send("y ahora?", &project, "Coin Climber");
        wait(&mut p);
        let args = lines(&dir.join("args.txt"));
        assert!(args.windows(2).any(|w| w == ["--permission-mode", "plan"]));
        assert!(args.windows(2).any(|w| w == ["--model", "haiku"]));
        assert!(args.windows(2).any(|w| w == ["--resume", "S-fake"]));

        // A new conversation forgets it.
        p.new_conversation();
        assert!(p.entries.is_empty() && p.session.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&project);
    }

    #[test]
    fn reports_failures_from_stderr() {
        let (dir, exe) = fake_cli("fail", "echo 'error: not logged in' >&2\nexit 3");
        let mut p = panel(&exe);
        p.send("hola", &dir, "x");
        let (_, finished) = wait(&mut p);
        assert!(finished);
        assert!(
            matches!(p.entries.last(), Some(Entry::Notice(t, Tone::Error)) if t.contains("código 3") && t.contains("not logged in")),
            "{:?}",
            p.entries
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stop_kills_the_agent_and_its_commands() {
        let (dir, exe) = fake_cli("stop", "echo '{\"type\":\"system\",\"session_id\":\"S2\"}'\nsleep 30 &\nsleep 30");
        let mut p = panel(&exe);
        p.send("hola", &dir, "x");
        let ctx = egui::Context::default();
        let start = Instant::now();
        while p.session.is_empty() && start.elapsed() < Duration::from_secs(5) {
            p.poll(&ctx);
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(p.session, "S2");
        p.stop();
        wait(&mut p);
        assert!(start.elapsed() < Duration::from_secs(10));
        assert_eq!(p.entries.last(), Some(&Entry::Notice("Detenido.".into(), Tone::Muted)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn codex_and_agy_get_their_own_arguments() {
        let (dir, exe) = fake_cli(
            "codex",
            r#"echo '{"type":"thread.started","thread_id":"T1"}'
echo '{"type":"item.completed","item":{"id":"m","type":"agent_message","text":"Vale"}}'
echo '{"type":"turn.completed"}'"#,
        );
        let mut p = panel(&exe);
        p.set_kind(Kind::Codex);
        p.set_permission(Permission::Edit);
        p.send("hola", &dir, "x");
        wait(&mut p);
        assert_eq!(lines(&dir.join("args.txt")), ["exec", "--json", "--skip-git-repo-check", "--sandbox", "workspace-write", "-"]);
        assert!(std::fs::read_to_string(dir.join("stdin.txt")).unwrap().starts_with("<modengine-editor-context>"));
        assert_eq!(p.entries.last(), Some(&Entry::Text("Vale".into())));
        assert_eq!(p.session, "T1");

        // Switching agent starts a new conversation.
        p.set_kind(Kind::Antigravity);
        assert!(p.session.is_empty());
        p.send("hola", &dir, "x");
        wait(&mut p);
        let args = lines(&dir.join("args.txt"));
        assert_eq!(&args[..4], ["--output-format", "stream-json", "--mode", "accept-edits"]);
        assert!(args.iter().any(|a| a.starts_with("--print=<modengine-editor-context>")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Talks to a real, signed-in CLI: `MODENGINE_AGENT_SMOKE=claude cargo test -p modengine-editor -- --ignored smoke`.
    #[test]
    #[ignore]
    fn smoke_real_cli() {
        let Some(kind) = std::env::var("MODENGINE_AGENT_SMOKE").ok().and_then(|k| Kind::from_id(&k)) else { return };
        let project = crate::room::tests::temp_project("smoke").canonicalize().unwrap();
        let mut p = AgentPanel::with_config(None);
        p.set_kind(kind);
        p.set_permission(Permission::Plan);
        p.set_model(&std::env::var("MODENGINE_AGENT_MODEL").unwrap_or_default());
        assert!(p.program.is_some(), "{} not installed", kind.name());
        p.send("List the object names defined in this project, one per line. Do not edit anything.", &project, "Coin Climber");
        let ctx = egui::Context::default();
        let start = Instant::now();
        while p.running() && start.elapsed() < Duration::from_secs(240) {
            p.poll(&ctx);
            std::thread::sleep(Duration::from_millis(50));
        }
        for e in &p.entries {
            println!("{e:?}");
        }
        println!("session={} status={}", p.session, p.status);
        // The reply itself is up to the model (Antigravity may end its turn on a permission block).
        assert!(!p.running() && !p.session.is_empty() && p.status.starts_with("Hecho"), "{}", p.status);
        let _ = std::fs::remove_dir_all(&project);
    }

    #[test]
    fn remembers_choices() {
        let dir = std::env::temp_dir().join(format!("modeditor_agent_cfg_{}", std::process::id()));
        let path = dir.join("agent.toml");
        let _ = std::fs::remove_dir_all(&dir);
        {
            let mut p = AgentPanel::with_config(Some(path.clone()));
            assert_eq!((p.kind, p.permission), (Kind::Claude, Permission::Auto));
            p.set_kind(Kind::Codex);
            p.set_model("gpt-x");
            p.set_permission(Permission::Plan);
            p.cfg.models.insert("codex".into(), vec![Model { id: "gpt-x".into(), name: "GPT X".into(), description: String::new() }]);
            p.save_cfg();
        }
        let p = AgentPanel::with_config(Some(path.clone()));
        assert_eq!((p.kind, p.permission, p.model()), (Kind::Codex, Permission::Plan, "gpt-x".to_string()));
        assert_eq!(p.model_name("gpt-x"), "GPT X");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn panel_renders_and_explains_a_missing_cli() {
        use egui_kittest::{kittest::Queryable, Harness};
        let (dir, exe) = fake_cli("ui", "true");
        let mut p = panel(&exe);
        p.entries = vec![
            Entry::User("hola".into()),
            Entry::Text("# Hecho\n- uno\n```\ncode\n```".into()),
            Entry::Tool(ToolLine { icon: "✎", verb: "Editar".into(), detail: "objects/world.ms".into() }),
        ];
        let mut h = Harness::builder().with_size([420.0, 700.0]).allow_missing_glyphs().with_accessibility_check(false).build_ui_state(
            |ui, p: &mut AgentPanel| {
                p.ui(ui, true);
            },
            p,
        );
        h.run();
        h.get_by_label("Editar");
        h.get_by_label("• uno");
        h.get_by_label("Enviar");
        // Without the CLI the panel says how to install it.
        h.state_mut().program_override = None;
        h.state_mut().program = None;
        h.run();
        h.get_by_label("Comprobar de nuevo");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
