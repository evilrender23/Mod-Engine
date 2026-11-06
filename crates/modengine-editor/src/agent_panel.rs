//! The Agente dock: chats with a coding-agent CLI (see `agent.rs`) that works in the project
//! folder. The CLI runs as a child process; reader threads send its output lines over a
//! channel that [`AgentPanel::poll`] drains every frame, so the UI never blocks.
//!
//! The app saves everything before each message and, when [`AgentPanel::poll`] says so,
//! reloads the project from disk and checks it.
//!
//! On top of the chat: a per-project history of conversations that resume their CLI session
//! (`agent_history.rs`), compaction (the CLI's own `/compact` where print mode runs it, a
//! summary otherwise), a context and cost meter, slash commands (`agent_slash.rs`), a queue of
//! messages typed while the agent works, retry / edit-and-resend (forking the session),
//! review of the files each turn changed with Revertir and `/deshacer` (`agent_snapshot.rs`),
//! and attachments from anywhere on disk, URLs and text snippets (`agent_browser.rs`).

pub use crate::agent_history::{Entry, Tone};
use crate::agent::{self, AttachKind, Attachment, Event, Kind, Model, Parser, Permission, Run, ToolLine};
use crate::agent_attach::{self as attach, Sources};
use crate::agent_history::{self as history, Conversation, Store, Summary, Turn, Usage};
use crate::agent_slash::{self as slash, Command as Slash};
use crate::agent_snapshot::{self as snapshot, ChangeKind, DiffLine, Snapshot};
use crate::app::Open;
use eframe::egui::{self, Color32, DragAndDrop, Key, Modifiers, RichText};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
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
    /// Folders starred in "Adjuntar desde el disco".
    #[serde(default)]
    pub favorites: Vec<PathBuf>,
    /// Folders things were last attached from (newest first).
    #[serde(default)]
    pub recent_dirs: Vec<PathBuf>,
    /// Files and folders from outside the project attached lately (newest first).
    #[serde(default)]
    pub recent_paths: Vec<PathBuf>,
    /// Web pages attached lately.
    #[serde(default)]
    pub recent_urls: Vec<String>,
}

pub fn config_path() -> Option<PathBuf> {
    modengine::home_dir().map(|h| h.join(".config/modengine/agent.toml"))
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
    if let Some(home) = modengine::home_dir() {
        dirs.extend([home.join(".local/bin"), home.join(".claude/local"), home.join(".npm-global/bin")]);
    }
    dirs.extend(["/usr/local/bin", "/opt/homebrew/bin"].map(PathBuf::from));
    if let Some(appdata) = std::env::var_os("APPDATA") {
        dirs.push(PathBuf::from(appdata).join("npm"));
    }
    // Windows: the spawnable extensions of PATHEXT, in its order.
    let exts: Vec<String> = if cfg!(windows) {
        let pathext = std::env::var("PATHEXT").unwrap_or_default().to_lowercase();
        let exts: Vec<String> = pathext.split(';').filter(|e| matches!(*e, ".exe" | ".com" | ".cmd" | ".bat")).map(String::from).collect();
        if exts.is_empty() {
            [".exe", ".cmd", ".bat"].map(String::from).to_vec()
        } else {
            exts
        }
    } else {
        vec![String::new()]
    };
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
    if let Some(home) = modengine::home_dir() {
        c.extend([home.join(".local/share/ModEngine/codigo/docs"), home.join(".local/share/ModEngine/docs")]);
    }
    c.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs"));
    c.into_iter().find(|d| d.join("funciones.md").is_file()).map(|d| modengine::canonical(&d).unwrap_or(d))
}

/// The `modengine` command line the agent imports art with: `$MODENGINE_BIN`, next to the
/// editor (also one folder up, for test binaries in `target/<profile>/deps`), or in PATH.
pub fn modengine_tool() -> Option<PathBuf> {
    let name = format!("modengine{}", std::env::consts::EXE_SUFFIX);
    let mut c: Vec<PathBuf> = vec![];
    if let Some(b) = std::env::var_os("MODENGINE_BIN") {
        c.push(b.into());
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        c.push(dir.join(&name));
        if dir.file_name().is_some_and(|n| n == "deps") {
            c.extend(dir.parent().map(|d| d.join(&name)));
        }
    }
    c.into_iter().find(|p| is_executable(p)).map(|p| modengine::canonical(&p).unwrap_or(p)).or_else(|| find_program("modengine"))
}

const INTERNET_TIP: &str = "El agente puede buscar en la web y leer páginas (también en Solo planificar: es de solo lectura). Se guarda por proyecto; también /internet on|off";
const INTERNET_TIP_AGY: &str = "El agente puede leer páginas web (read_url). Antigravity no permite desactivar su búsqueda web por permisos: sin internet solo se le pide que no la use. Se guarda por proyecto; también /internet on|off";

/// Assets a turn brought in, from its changed files: binaries in sprites/, sounds/ and
/// fonts/, and new tile sets (`sprite 'hero'`, `tile set 'ts_cave'`).
pub fn imported_assets(files: &[snapshot::FileChange]) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for f in files.iter().filter(|f| f.kind != ChangeKind::Deleted) {
        let Some((dir, file)) = f.rel.split_once('/') else { continue };
        let Some((stem, ext)) = file.rsplit_once('.') else { continue };
        let ext = ext.to_lowercase();
        let what = match (dir, ext.as_str()) {
            ("sprites", "png") => "sprite",
            ("sounds", "wav" | "ogg") => "sonido",
            ("fonts", "ttf" | "otf") => "fuente",
            ("tilesets", "toml") if f.kind == ChangeKind::Added => "tile set",
            _ => continue,
        };
        let line = format!("{what} '{stem}'");
        if !out.contains(&line) {
            out.push(line);
        }
    }
    out
}

/// What "🛒 Buscar en la Tienda…" writes in the message box (keeping what was typed as the
/// description of what to look for).
pub fn store_instruction(typed: &str) -> String {
    let what = typed.trim();
    let what = if what.is_empty() { "<describe aquí el asset: p. ej. un sprite de slime animado>" } else { what };
    format!(
        "Busca en la Tienda (OpenGameArt) {what}. Prefiere CC0 (o CC-BY / OGA-BY, con su crédito), mira la imagen antes de importarla \
         para elegir los frames, impórtala al proyecto con `modengine store get` y dime qué asset usaste, su autor, su licencia y su URL."
    )
}

/// Assets from the store a turn added to CREDITS.md (`modengine store get` writes a section per
/// asset ending in `<!-- oga:<slug> -->`): `«Title» (licence) → sprite 'x', sonido 'y'`, from the
/// file lines that were not there before.
pub fn store_downloads(before: &str, after: &str) -> Vec<String> {
    let mut out = vec![];
    let mut rest = after;
    while let Some(m) = rest.find("<!-- oga:") {
        let section_start = rest[..m].rfind("\n## ").map(|i| i + 1).unwrap_or(0);
        let section = &rest[section_start..m];
        rest = &rest[m + 9..];
        let title = section.lines().next().and_then(|l| l.strip_prefix("## ")).map(|t| t.rsplit_once(" — ").map(|(a, _)| a).unwrap_or(t)).unwrap_or("?");
        let license = section
            .lines()
            .find_map(|l| l.strip_prefix("- Licencia: ").or_else(|| l.strip_prefix("- Licencias (a elegir): ")))
            .map(|l| {
                // `[CC-BY 3.0](url), [CC0](url)` → `CC-BY 3.0 / CC0`
                l.split("), ").map(|p| p.trim_start_matches('[').split(']').next().unwrap_or(p).trim_matches('*').to_string()).collect::<Vec<_>>().join(" / ")
            })
            .unwrap_or_else(|| "licencia desconocida".into());
        let mut assets: Vec<String> = vec![];
        for l in section.lines().filter(|l| l.starts_with("- Fichero: `") && !before.contains(*l)) {
            let file = l["- Fichero: `".len()..].split('`').next().unwrap_or("");
            let Some((dir, name)) = file.split_once('/') else { continue };
            let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
            let what = match dir {
                "sprites" if file.ends_with(".png") => "sprite",
                "sounds" => "sonido",
                "fonts" => "fuente",
                "tilesets" => "tile set",
                _ => continue,
            };
            let a = format!("{what} '{stem}'");
            if !assets.contains(&a) {
                assets.push(a);
            }
        }
        if !assets.is_empty() {
            out.push(format!("«{title}» ({license}) → {}", assets.join(", ")));
        }
    }
    out
}

/// What the "Importar al proyecto" button writes in the message box for these attachments
/// (`None`: no art folder or pictures from outside the project).
pub fn import_instruction(root: Option<&Path>, attachments: &[Attachment]) -> Option<String> {
    let outside = |a: &&Attachment| a.on_disk() && root.is_none_or(|r| !agent::inside(r, &a.path));
    let dirs: Vec<&Attachment> = attachments.iter().filter(outside).filter(|a| a.path.is_dir()).collect();
    let images: Vec<&Attachment> = attachments.iter().filter(outside).filter(|a| a.kind == agent::AttachKind::Image).collect();
    let what = match (dirs.len(), images.len()) {
        (0, 0) | (0, 1) => return None,
        (1, 0) => format!("el arte de la carpeta «{}»", dirs[0].label),
        (0, _) => "estas imágenes".to_string(),
        _ => "el arte adjunto".to_string(),
    };
    Some(format!(
        "Importa al proyecto {what}: mira cada imagen, decide qué es sprite (con sus frames y origen) y qué es tile set, \
         e impórtalo con modengine import. Si hay ficheros de licencia o créditos, añádelos con --credit."
    ))
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

#[cfg(unix)]
const SIGTERM: i32 = 15;
#[cfg(unix)]
const SIGKILL: i32 = 9;

/// Windows has no process groups: `taskkill /T` ends the CLI and every command it started.
#[cfg(windows)]
fn kill_tree(child: &mut Child) {
    let _ = crate::proc::command("taskkill").args(["/PID", &child.id().to_string(), "/T", "/F"]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    let _ = child.kill();
}

fn hard_kill(child: &mut Child) {
    #[cfg(unix)]
    signal_group(child, SIGKILL);
    #[cfg(windows)]
    kill_tree(child);
    let _ = child.kill();
    let _ = child.wait();
}

// ------------------------------------------------------------------ transcript

/// What the app must do after [`AgentPanel::poll`].
#[derive(Default, Debug, PartialEq)]
pub struct Polled {
    /// The agent changed files (or ran commands that may have): reload the project from disk.
    pub reload: bool,
    /// A run ended: check the project.
    pub finished: bool,
    /// Assets the turn that ended brought into the project (`sprite 'hero'`), for Salida.
    pub imported: Vec<String>,
    /// What the turn got from the asset store (`«Slime» (CC0) → sprite 'slime'`), for Salida.
    pub downloaded: Vec<String>,
    /// Project files the agent reported writing during the run that ended.
    pub changed: Vec<PathBuf>,
    /// The chat turn that ended (when it was one): its request and the files it added or
    /// modified (project-relative), for "a workspace per turn".
    pub turn: Option<TurnResult>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TurnResult {
    pub prompt: String,
    pub files: Vec<String>,
    /// Lines (1-based, in the new text) a modified text file changed at, so a script that
    /// defines several objects only opens the ones that changed. Missing: the whole file.
    pub lines: BTreeMap<String, Vec<usize>>,
}

/// Lines of `new` that `old` -> `new` added or changed (a deletion counts at the line after it).
pub fn changed_lines(old: &str, new: &str) -> Vec<usize> {
    let mut out: Vec<usize> = vec![];
    let mut next = 1;
    for l in snapshot::unified(old, new, 0) {
        if l.header {
            continue;
        }
        match l.op {
            snapshot::Op::Add => out.extend(l.new_no),
            snapshot::Op::Del => out.push(next),
            snapshot::Op::Same => {}
        }
        if let Some(n) = l.new_no {
            next = n + 1;
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

enum Msg {
    Line(String),
    StdoutEnd,
    Stderr(String),
}

/// Why a run was started.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Purpose {
    /// A user message (turn id).
    Chat(u64),
    /// `/compact` run by the CLI itself (Claude Code, Antigravity).
    CompactNative,
    /// Asking the agent for a summary to seed a new session (Codex).
    CompactSummary,
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
    failed: bool,
    compacted: bool,
    stderr: String,
    changed: BTreeSet<PathBuf>,
    blocked: usize,
    context_file: PathBuf,
    root: PathBuf,
    purpose: Purpose,
    /// The project before a chat turn, to list and revert what it changed.
    before: Option<Snapshot>,
    /// First transcript entry of this run.
    first_entry: usize,
}

struct Lister {
    kind: Kind,
    rx: Receiver<Vec<Model>>,
}

/// A message typed while the agent was busy, sent when it finishes.
#[derive(Clone, Debug, PartialEq)]
pub struct Queued {
    pub text: String,
    pub attachments: Vec<Attachment>,
}

/// What the next [`AgentPanel::send`] carries besides its text (queued, retried or edited messages).
#[derive(Clone, Debug, Default)]
struct Outgoing {
    attachments: Vec<Attachment>,
    /// Continue from before this turn (index into `turns`), dropping it and what followed.
    fork: Option<usize>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum View {
    Chat,
    History,
}

/// The diff window.
struct DiffView {
    turn: u64,
    rel: String,
    lines: Vec<DiffLine>,
    note: Option<String>,
}

/// Past this share of the context the meter turns amber and the panel suggests compacting.
pub const WARN_AT: f32 = 0.8;

/// What a Codex conversation is compacted into.
const SUMMARY_PROMPT: &str = "Summarise our conversation so far so that it can continue in a new session that will only see your summary. \
Include: what the user wants overall, every decision and preference they stated, what you changed in which files (and why), \
what is still pending or failing, and anything you were about to do. Be complete but concise, use short bullet points, \
write in the user's language, and do not change any file now.";

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
    /// The `modengine` the agent imports art with, instead of [`modengine_tool`] (tests).
    pub tool_override: Option<PathBuf>,
    /// Assets the last turn imported, until `poll` hands them over.
    imported: Vec<String>,
    /// Store assets the last turn added to CREDITS.md, until `poll` hands them over.
    downloaded: Vec<String>,
    pub entries: Vec<Entry>,
    /// The streaming reply, as an index into `entries`.
    open_text: Option<usize>,
    pub input: String,
    other_model: String,
    pub session: String,
    answering_model: String,
    pub status: String,
    run: Option<Running>,
    lister: Option<Lister>,
    /// Lists from this session; failed lookups are not retried until the menu opens again.
    listed: HashMap<Kind, Vec<Model>>,
    list_tried: bool,
    /// Sent with the next message, then cleared.
    pub attachments: Vec<Attachment>,
    /// "Otra ruta" field of the attach menu, its filter of project files, and the last error.
    attach_path: String,
    attach_filter: String,
    attach_error: Option<String>,
    /// Project files for the menu and `@` mentions, listed at most every few seconds.
    files: Vec<String>,
    files_of: Option<(PathBuf, Instant)>,
    /// Highlighted `@` suggestion, and the input the user closed the suggestions for (Esc).
    mention_sel: usize,
    mention_closed: Option<String>,

    // -- the conversation
    /// Saved as `<id>.json`; empty until it is first saved.
    pub conv_id: String,
    pub title: String,
    renamed: bool,
    created: u64,
    pub turns: Vec<Turn>,
    next_turn: u64,
    /// Claude Code: the last assistant message of the session (fork point).
    checkpoint: String,
    /// Told to the next new session (a compaction summary or an edited transcript).
    pub seed: String,
    pub usage: Usage,
    warned_full: bool,
    /// Messages waiting for the agent to finish.
    pub queue: Vec<Queued>,
    /// The queue waits (the last turn failed or was stopped).
    pub queue_paused: bool,
    outgoing: Option<Outgoing>,
    /// A message the panel wants sent (from the queue, Reintentar): returned by `ui`.
    auto_send: Option<String>,
    /// The message box holds the last message again, to resend from before it (turn index).
    pub editing: Option<usize>,
    compact_requested: bool,
    pending_reload: bool,

    // -- the project
    /// Its folder and game name, once known.
    project: Option<(PathBuf, String)>,
    /// `~/.config/modengine/agent/history`; `None`: nothing is saved (tests without config).
    pub history_root: Option<PathBuf>,
    store: Option<Store>,
    /// Compact by itself past this % of the context (0: never).
    pub autocompact: u8,
    /// 🌐 Internet: the agent may search the web and read pages (per project, on by default).
    pub internet: bool,
    /// After each turn, open what it added or changed in a new workspace (per project, on).
    pub turn_workspace: bool,
    /// The request of the chat turn that is running.
    turn_prompt: String,
    /// The chat turn that ended, until `poll` hands it over.
    turn_result: Option<TurnResult>,

    // -- views
    pub view: View,
    history: Vec<Summary>,
    history_query: String,
    renaming: Option<(String, String)>,
    confirm_delete: Option<String>,
    /// Before-snapshots of recent turns, by turn id.
    snaps: Vec<(u64, Snapshot)>,
    diff: Option<DiffView>,
    browser: crate::agent_browser::Browser,
    /// "Texto…" attachment being written: (title, text).
    text_modal: Option<(String, String)>,
    /// Screenshots on their way in: clipboard reads (a thread) and window captures (egui).
    image_rx: Receiver<Result<(egui::ColorImage, &'static str), String>>,
    image_tx: Sender<Result<(egui::ColorImage, &'static str), String>>,
    /// Thumbnails of image attachments, by path.
    thumbs: HashMap<PathBuf, Option<egui::TextureHandle>>,
    /// Where pasted images go (tests use a temp folder).
    paste_dir: Option<PathBuf>,
    slash_sel: usize,
    /// Summaries the user unfolded (entry index).
    unfolded: BTreeSet<usize>,
    /// Demo processes shown in screenshots.
    demo: bool,
}

impl Default for AgentPanel {
    fn default() -> Self {
        AgentPanel::with_config(None)
    }
}

impl AgentPanel {
    /// Loads the remembered choices from `config` (none: defaults, nothing saved). The history
    /// goes next to it, in `agent/history/`.
    pub fn with_config(config: Option<PathBuf>) -> AgentPanel {
        let cfg = config.as_deref().map(Config::load).unwrap_or_default();
        let kind = Kind::from_id(&cfg.agent).unwrap_or(Kind::Claude);
        let permission = Permission::from_key(&cfg.permission).unwrap_or(Permission::Auto);
        let history_root = config.as_deref().and_then(Path::parent).map(|d| d.join("agent/history"));
        let (image_tx, image_rx) = mpsc::channel();
        let mut p = AgentPanel {
            kind,
            permission,
            cfg,
            config_path: config,
            program: None,
            program_override: None,
            tool_override: None,
            imported: vec![],
            downloaded: vec![],
            entries: vec![],
            open_text: None,
            input: String::new(),
            other_model: String::new(),
            session: String::new(),
            answering_model: String::new(),
            status: String::new(),
            run: None,
            lister: None,
            listed: HashMap::new(),
            list_tried: false,
            attachments: vec![],
            attach_path: String::new(),
            attach_filter: String::new(),
            attach_error: None,
            files: vec![],
            files_of: None,
            mention_sel: 0,
            mention_closed: None,
            conv_id: String::new(),
            title: String::new(),
            renamed: false,
            created: 0,
            turns: vec![],
            next_turn: 1,
            checkpoint: String::new(),
            seed: String::new(),
            usage: Usage::default(),
            warned_full: false,
            queue: vec![],
            queue_paused: false,
            outgoing: None,
            auto_send: None,
            editing: None,
            compact_requested: false,
            pending_reload: false,
            project: None,
            history_root,
            store: None,
            autocompact: 0,
            internet: true,
            turn_workspace: true,
            turn_prompt: String::new(),
            turn_result: None,
            view: View::Chat,
            history: vec![],
            history_query: String::new(),
            renaming: None,
            confirm_delete: None,
            snaps: vec![],
            diff: None,
            browser: Default::default(),
            text_modal: None,
            image_rx,
            image_tx,
            thumbs: HashMap::new(),
            paste_dir: None,
            slash_sel: 0,
            unfolded: BTreeSet::new(),
            demo: false,
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
            if !self.demo {
                self.cfg.save(path);
            }
        }
        self.save_state();
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
        // Conversations belong to one CLI: the next message starts a new session that is told
        // what was said so far.
        if !self.session.is_empty() {
            self.session.clear();
            self.checkpoint.clear();
            self.seed = history::transcript_seed(&self.entries, 40 * 1024);
            self.usage.cost_base += self.usage.session_cost;
            self.usage.session_cost = 0.0;
            self.notice(format!("Ahora con {}: sigue la conversación en una sesión nueva, con lo hablado hasta aquí.", kind.name()), Tone::Muted);
        }
        self.usage.window = 0;
        self.list_tried = false;
        self.locate();
        self.save_cfg();
    }

    pub fn set_permission(&mut self, p: Permission) {
        self.permission = p;
        self.save_cfg();
    }

    /// 🌐 Internet on or off for this project (remembered with its other agent settings).
    pub fn set_internet(&mut self, on: bool) {
        self.internet = on;
        self.save_state();
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

    /// The context window of the chosen model when the CLI listed it (Codex).
    fn listed_window(&self) -> u64 {
        let id = self.model();
        let models = self.available_models();
        let m = if id.is_empty() { models.first() } else { models.iter().find(|m| m.id == id) };
        m.map(|m| m.context).unwrap_or(0)
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
                let mut child = crate::proc::command(program).args(&args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
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

    /// Forgets the conversation on screen (it stays in the history) and stops the agent.
    fn clear_conversation(&mut self) {
        if let Some(mut r) = self.run.take() {
            hard_kill(&mut r.child);
            let _ = std::fs::remove_file(&r.context_file);
        }
        self.entries.clear();
        self.open_text = None;
        self.session.clear();
        self.answering_model.clear();
        self.status.clear();
        self.conv_id.clear();
        self.title.clear();
        self.renamed = false;
        self.created = 0;
        self.turns.clear();
        self.next_turn = 1;
        self.checkpoint.clear();
        self.seed.clear();
        self.usage = Usage::default();
        self.warned_full = false;
        self.queue.clear();
        self.queue_paused = false;
        self.outgoing = None;
        self.auto_send = None;
        self.editing = None;
        self.compact_requested = false;
        self.snaps.clear();
        self.diff = None;
        self.unfolded.clear();
    }

    /// ＋ Nueva, `/nueva`: the current conversation stays in the history.
    pub fn new_conversation(&mut self) {
        self.save_conversation();
        self.clear_conversation();
        self.view = View::Chat;
        self.save_state();
    }

    /// The project closes: stop the agent, keep the conversation in its history.
    pub fn close_project(&mut self) {
        self.save_conversation();
        self.clear_conversation();
        self.project = None;
        self.store = None;
        self.history.clear();
        self.view = View::Chat;
    }

    // ------------------------------------------------------------ history

    /// The panel is now about `root`: loads that project's settings and reopens the
    /// conversation that was open in it.
    pub fn enter_project(&mut self, root: &Path, name: &str) {
        let root = modengine::canonical(root).unwrap_or_else(|_| root.to_path_buf());
        if self.project.as_ref().is_some_and(|(r, _)| *r == root) {
            if let Some((_, n)) = &mut self.project {
                *n = name.to_string();
            }
            return;
        }
        if self.project.is_some() {
            // Another project: its own conversations (this one stays in the old one's history).
            self.close_project();
        }
        self.project = Some((root.clone(), name.to_string()));
        if self.demo {
            return;
        }
        self.store = self.history_root.as_ref().map(|h| Store::new(h, &root));
        let Some(store) = &self.store else { return };
        let st = store.state();
        self.autocompact = st.autocompact;
        self.internet = !st.no_internet;
        self.turn_workspace = !st.no_turn_workspace;
        if let Some(k) = Kind::from_id(&st.agent) {
            self.kind = k;
            self.locate();
        }
        if let Some(p) = Permission::from_key(&st.permission) {
            self.permission = p;
        }
        if !st.last.is_empty() {
            self.open_conversation(&st.last);
        }
    }

    fn save_state(&mut self) {
        if self.demo {
            return;
        }
        if let Some(store) = &self.store {
            store.save_state(&history::ProjectState {
                last: self.conv_id.clone(),
                agent: self.kind.id().into(),
                permission: self.permission.key().into(),
                autocompact: self.autocompact,
                no_internet: !self.internet,
                no_turn_workspace: !self.turn_workspace,
            });
        }
    }

    /// The conversation as it is saved.
    pub fn conversation(&self) -> Conversation {
        Conversation {
            id: self.conv_id.clone(),
            title: self.title.clone(),
            renamed: self.renamed,
            project: self.project.as_ref().map(|(r, _)| r.clone()).unwrap_or_default(),
            agent: self.kind.id().into(),
            model: if self.answering_model.is_empty() { self.model() } else { self.answering_model.clone() },
            session: self.session.clone(),
            checkpoint: self.checkpoint.clone(),
            seed: self.seed.clone(),
            created: self.created,
            updated: history::now(),
            entries: self.entries.clone(),
            turns: self.turns.clone(),
            usage: self.usage.clone(),
        }
    }

    /// Writes the conversation to the history (titled after its first message) and remembers
    /// it as the project's open one.
    pub fn save_conversation(&mut self) {
        if self.entries.is_empty() || self.demo {
            return;
        }
        if self.conv_id.is_empty() {
            self.conv_id = history::new_id();
            self.created = history::now();
        }
        if !self.renamed {
            if let Some(first) = self.entries.iter().find_map(|e| if let Entry::User(t) = e { Some(t.clone()) } else { None }) {
                self.title = history::title_from(&first);
            } else if self.title.is_empty() {
                self.title = "Conversación".into();
            }
        }
        let c = self.conversation();
        if let Some(store) = &self.store {
            if let Err(e) = store.save(&c) {
                self.status = format!("No se pudo guardar el historial: {e}");
            }
        }
        self.save_state();
    }

    /// Reopens a saved conversation; the next message resumes its CLI session.
    pub fn open_conversation(&mut self, id: &str) -> bool {
        if self.running() {
            return false;
        }
        let Some(c) = self.store.as_ref().and_then(|s| s.load(id)) else { return false };
        if self.conv_id != c.id {
            self.save_conversation();
        }
        self.clear_conversation();
        if let Some(k) = Kind::from_id(&c.agent) {
            self.kind = k;
            self.locate();
            self.list_tried = false;
        }
        self.conv_id = c.id;
        self.title = c.title;
        self.renamed = c.renamed;
        self.created = c.created;
        self.session = c.session;
        self.checkpoint = c.checkpoint;
        self.seed = c.seed;
        self.entries = c.entries;
        self.next_turn = c.turns.iter().map(|t| t.id + 1).max().unwrap_or(1);
        self.turns = c.turns;
        self.usage = c.usage;
        self.answering_model = c.model;
        self.status = if self.session.is_empty() { "Conversación abierta".into() } else { format!("Conversación abierta: sigue la sesión de {}", self.kind.name()) };
        self.view = View::Chat;
        self.save_state();
        true
    }

    /// Re-reads the history list (opening it, after changes).
    pub fn refresh_history(&mut self) {
        self.history = self.store.as_ref().map(Store::list).unwrap_or_default();
    }

    pub fn show_history(&mut self, query: &str) {
        self.save_conversation();
        self.refresh_history();
        self.history_query = query.to_string();
        self.view = View::History;
    }

    pub fn rename(&mut self, id: &str, title: &str) {
        let title = title.trim();
        if title.is_empty() {
            return;
        }
        if id == self.conv_id || id.is_empty() {
            self.title = title.to_string();
            self.renamed = true;
            self.save_conversation();
        } else if let Some(store) = &self.store {
            if let Some(mut c) = store.load(id) {
                c.title = title.to_string();
                c.renamed = true;
                let _ = store.save(&c);
            }
        }
        self.refresh_history();
    }

    pub fn delete_conversation(&mut self, id: &str) {
        if let Some(store) = &self.store {
            store.delete(id);
        }
        if id == self.conv_id && !self.running() {
            self.clear_conversation();
            self.save_state();
        }
        self.refresh_history();
    }

    /// The Markdown of a conversation (the open one when `id` is it or empty).
    pub fn markdown(&self, id: &str) -> Option<String> {
        if id.is_empty() || id == self.conv_id {
            let mut c = self.conversation();
            if c.title.is_empty() {
                c.title = self.entries.iter().find_map(|e| if let Entry::User(t) = e { Some(history::title_from(t)) } else { None }).unwrap_or_else(|| "Conversación".into());
            }
            return Some(history::to_markdown(&c));
        }
        self.store.as_ref().and_then(|s| s.load(id)).map(|c| history::to_markdown(&c))
    }

    /// Saves a conversation as Markdown in Descargas (or the home folder); returns the file.
    pub fn export(&mut self, id: &str, dir: Option<&Path>) -> Option<PathBuf> {
        let md = self.markdown(id)?;
        let title = md.lines().next().unwrap_or("").trim_start_matches("# ").to_string();
        let home = modengine::home_dir().unwrap_or_else(std::env::temp_dir);
        let dir = dir.map(Path::to_path_buf).unwrap_or_else(|| ["Descargas", "Downloads"].iter().map(|d| home.join(d)).find(|d| d.is_dir()).unwrap_or(home));
        let stem: String = title.chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' }).take(50).collect();
        let mut path = dir.join(format!("agente-{stem}.md"));
        let mut n = 2;
        while path.exists() {
            path = dir.join(format!("agente-{stem}-{n}.md"));
            n += 1;
        }
        match std::fs::write(&path, md) {
            Ok(()) => Some(path),
            Err(e) => {
                self.notice(format!("No se pudo exportar: {e}"), Tone::Error);
                None
            }
        }
    }

    // ------------------------------------------------------------ attachments

    /// Adds `a` to the next message (again: replaces the old one, which may be out of date).
    pub fn attach(&mut self, a: Attachment) {
        self.attachments.retain(|x| !x.same(&a));
        self.remember_recent(&a);
        self.attachments.push(a);
        self.attach_error = None;
    }

    /// A pasted or captured image: saved as PNG outside the project and attached.
    pub fn attach_image(&mut self, img: &egui::ColorImage, prefix: &str) -> bool {
        let dir = self.paste_dir.clone().unwrap_or_else(crate::agent_paste::pasted_dir);
        match crate::agent_paste::save(img, &dir, prefix) {
            Ok(path) => {
                let mut a = Attachment::file(&path);
                a.label = format!("{} {}×{}", if prefix == "editor" { "Captura del editor" } else { "Captura" }, img.size[0], img.size[1]);
                a.about = if prefix == "editor" { "a screenshot of the Mod Engine editor window as the user sees it now".into() } else { "a screenshot the user pasted".into() };
                self.attach(a);
                true
            }
            Err(e) => {
                self.attach_error = Some(format!("No se pudo guardar la imagen: {e}"));
                false
            }
        }
    }

    /// "📋 Pegar imagen": reads the clipboard on a thread (it can block on X11/Wayland).
    pub fn paste_clipboard_image(&mut self) {
        let tx = self.image_tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(crate::agent_paste::clipboard_image().map(|i| (i, "captura")));
        });
    }

    /// "📸 Captura del editor": egui renders the window next frame and hands it back.
    pub fn capture_editor(&mut self, ctx: &egui::Context) {
        let tx = self.image_tx.clone();
        let repaint = ctx.clone();
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img| {
            let _ = tx.send(Ok(((*img).clone(), "editor")));
            repaint.request_repaint();
        })));
        ctx.request_repaint();
    }

    /// Images that arrived from the clipboard thread or a window capture.
    fn receive_images(&mut self) {
        while let Ok(r) = self.image_rx.try_recv() {
            match r {
                Ok((img, prefix)) => {
                    self.attach_image(&img, prefix);
                }
                Err(e) => self.attach_error = Some(e),
            }
        }
    }

    /// Things from outside the project are offered again under "Recientes".
    fn remember_recent(&mut self, a: &Attachment) {
        let root = self.project.as_ref().map(|(r, _)| r.clone());
        let changed = match a.kind {
            AttachKind::Url => {
                self.cfg.recent_urls.retain(|u| *u != a.text);
                self.cfg.recent_urls.insert(0, a.text.clone());
                self.cfg.recent_urls.truncate(8);
                true
            }
            _ if a.on_disk() && !root.as_ref().is_some_and(|r| agent::inside(r, &a.path)) => {
                self.cfg.recent_paths.retain(|p| *p != a.path);
                self.cfg.recent_paths.insert(0, a.path.clone());
                self.cfg.recent_paths.truncate(12);
                let dir = if a.path.is_dir() { Some(a.path.clone()) } else { a.path.parent().map(Path::to_path_buf) };
                if let Some(d) = dir {
                    self.cfg.recent_dirs.retain(|p| *p != d);
                    self.cfg.recent_dirs.insert(0, d);
                    self.cfg.recent_dirs.truncate(8);
                }
                true
            }
            _ => false,
        };
        if changed {
            self.save_cfg();
        }
    }

    /// A file or folder the user picked; says why when it does not exist.
    pub fn attach_path(&mut self, path: &Path) -> bool {
        if !path.exists() {
            self.attach_error = Some(format!("No existe: {}", path.display()));
            return false;
        }
        self.attach(Attachment::file(path));
        true
    }

    /// A web page; false when it is not an http(s) URL.
    pub fn attach_url(&mut self, url: &str) -> bool {
        let u = url.trim();
        if !(u.starts_with("http://") || u.starts_with("https://")) || u.contains(char::is_whitespace) || u.len() < 10 {
            self.attach_error = Some("Escribe una dirección que empiece por http:// o https://".into());
            return false;
        }
        self.attach(Attachment::url(u));
        true
    }

    /// Pasted text that is only paths or `file://` URIs (one per line) is attached instead of
    /// typed; returns whether it was.
    pub fn attach_pasted(&mut self, text: &str) -> bool {
        let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        if lines.is_empty() || lines.len() > 50 {
            return false;
        }
        let paths: Vec<PathBuf> = lines
            .iter()
            .filter(|l| l.starts_with("file://") || l.starts_with('/') || l.starts_with("~/") || (l.len() > 2 && l.as_bytes()[1] == b':' && l.as_bytes()[2] == b'\\'))
            .map(|l| crate::import::expand(l))
            .collect();
        if paths.len() != lines.len() || !paths.iter().all(|p| p.exists()) {
            return false;
        }
        for p in paths {
            self.attach_path(&p);
        }
        true
    }

    /// What `--agent --select` shows in screenshots: the focused asset, the room selection, Salida.
    pub fn attach_defaults(&mut self, src: &Sources) {
        let asset = src.focused.as_ref().or(src.selected.as_ref()).and_then(|o| src.asset(o));
        for a in [asset, src.room_selection(), src.output()].into_iter().flatten() {
            self.attach(a);
        }
    }

    fn project_files(&mut self, dir: &Path) -> Vec<String> {
        let fresh = self.files_of.as_ref().is_some_and(|(d, t)| d == dir && t.elapsed() < Duration::from_secs(5));
        if !fresh {
            self.files = attach::project_files(dir, 5000);
            self.files_of = Some((dir.to_path_buf(), Instant::now()));
        }
        self.files.clone()
    }

    /// Suggestions for the `@word` at the end of the message: assets first, then files.
    pub fn mention_candidates(&mut self, src: &Sources) -> Vec<(String, Attachment)> {
        let Some((_, q)) = attach::mention_at_end(&self.input) else { return vec![] };
        if self.mention_closed.as_deref() == Some(self.input.as_str()) {
            return vec![];
        }
        let Some(p) = src.project else { return vec![] };
        let q = q.to_string();
        let mut out: Vec<(String, Attachment)> = vec![];
        for o in attach::all_assets(p) {
            let label = attach::asset_label(&o);
            if attach::matches(&label, &q) {
                if let Some(a) = attach::asset(p, &o) {
                    out.push((label, a));
                }
            }
            if out.len() >= 8 {
                return out;
            }
        }
        for f in self.project_files(&p.dir) {
            if out.len() >= 8 {
                break;
            }
            if attach::matches(&f, &q) {
                let a = Attachment::file(&p.dir.join(&f));
                if !out.iter().any(|(_, x)| x.path == a.path) {
                    out.push((format!("📄 {f}"), a));
                }
            }
        }
        out
    }

    /// Replaces the `@word` being typed by `@name ` and attaches what it names.
    pub fn pick_mention(&mut self, a: Attachment) {
        if let Some((start, _)) = attach::mention_at_end(&self.input) {
            self.input.truncate(start);
            self.input += &format!("@{} ", a.label);
        }
        self.mention_sel = 0;
        self.attach(a);
    }

    // ------------------------------------------------------------ queue, retry, edit

    /// Puts a message (and the attachments) in the queue; it is sent when the agent finishes.
    pub fn enqueue(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() && self.attachments.is_empty() {
            return;
        }
        self.queue.push(Queued { text: text.to_string(), attachments: std::mem::take(&mut self.attachments) });
        self.input.clear();
    }

    /// Takes the next message the panel wants sent (queued or retried); the app saves and calls
    /// [`AgentPanel::send`] with it. Also called by `ui`.
    pub fn next_to_send(&mut self) -> Option<String> {
        if self.running() || self.program.is_none() {
            return None;
        }
        if let Some(t) = self.auto_send.take() {
            return Some(t);
        }
        if self.queue_paused || self.queue.is_empty() || self.compact_requested {
            return None;
        }
        let q = self.queue.remove(0);
        self.outgoing = Some(Outgoing { attachments: q.attachments, fork: None });
        Some(q.text)
    }

    /// Queued message `i` goes back into the message box to be edited.
    pub fn unqueue(&mut self, i: usize) {
        if i < self.queue.len() {
            let q = self.queue.remove(i);
            if !self.input.trim().is_empty() {
                self.input.push('\n');
            }
            self.input += &q.text;
            for a in q.attachments {
                self.attach(a);
            }
        }
    }

    /// Index into `turns` of the last message, when it can be retried or edited.
    fn last_turn(&self) -> Option<usize> {
        (!self.turns.is_empty() && !self.running()).then(|| self.turns.len() - 1)
    }

    /// `/reintentar`: sends the last message again from before its answer.
    pub fn retry(&mut self) -> Option<String> {
        let i = self.last_turn()?;
        let t = &self.turns[i];
        self.outgoing = Some(Outgoing { attachments: t.attachments.clone(), fork: Some(i) });
        self.queue_paused = false;
        self.auto_send = Some(t.prompt.clone());
        self.auto_send.clone()
    }

    /// ✎ on the last message, `/editar`: puts it back in the message box; sending it continues
    /// the conversation from before it.
    pub fn edit_last(&mut self) -> bool {
        let Some(i) = self.last_turn() else { return false };
        let t = self.turns[i].clone();
        self.input = t.prompt;
        self.attachments = t.attachments;
        self.editing = Some(i);
        true
    }

    pub fn cancel_edit(&mut self) {
        if self.editing.take().is_some() {
            self.input.clear();
            self.attachments.clear();
        }
    }

    /// Drops turn `i` and what came after it, and sets the session up to continue from before
    /// it: Claude Code forks its session at that point; the other CLIs start a new session told
    /// the conversation so far.
    fn rewind_to(&mut self, i: usize) {
        let Some(t) = self.turns.get(i).cloned() else { return };
        self.entries.truncate(t.entry.min(self.entries.len()));
        self.turns.truncate(i);
        self.open_text = None;
        self.session = t.session_before;
        self.checkpoint = t.checkpoint_before;
        self.seed = t.seed_before;
        let fork = self.kind == Kind::Claude && !self.session.is_empty() && !self.checkpoint.is_empty();
        if !self.session.is_empty() && !fork {
            self.session.clear();
            self.checkpoint.clear();
            let before = history::transcript_seed(&self.entries, 40 * 1024);
            self.seed = if self.seed.is_empty() { before } else { format!("{}\n\n{before}", self.seed) };
        }
    }

    // ------------------------------------------------------------ running

    /// Starts the agent on `prompt` (and the attachments) in `root`. The app saves everything
    /// first. While the agent works, the message is queued instead.
    pub fn send(&mut self, prompt: &str, root: &Path, name: &str) {
        let text = prompt.trim().to_string();
        self.enter_project(root, name);
        let out = self.outgoing.take();
        let fork = out.as_ref().and_then(|o| o.fork).or(self.editing.take());
        let attachments = match out {
            Some(o) => o.attachments,
            None => std::mem::take(&mut self.attachments),
        };
        if text.is_empty() && attachments.is_empty() {
            return;
        }
        if self.running() {
            self.queue.push(Queued { text, attachments });
            self.input.clear();
            return;
        }
        if self.program.is_none() {
            self.attachments = attachments;
            return;
        }
        if let Some(i) = fork {
            self.rewind_to(i);
        }
        let turn = Turn {
            id: self.next_turn,
            entry: self.entries.len(),
            prompt: text.clone(),
            attachments: attachments.clone(),
            session_before: self.session.clone(),
            checkpoint_before: self.checkpoint.clone(),
            seed_before: self.seed.clone(),
        };
        let fork_at = if fork.is_some() && self.kind == Kind::Claude && !self.session.is_empty() { self.checkpoint.clone() } else { String::new() };
        if !text.is_empty() {
            self.entries.push(Entry::User(text.clone()));
        }
        if !attachments.is_empty() {
            self.entries.push(Entry::Attached(attachments.clone()));
        }
        let root = self.project.as_ref().map(|(r, _)| r.clone()).unwrap_or_else(|| root.to_path_buf());
        let before = Snapshot::take(&root);
        self.turn_prompt = text.clone();
        if self.start(&text, attachments.clone(), Purpose::Chat(turn.id), false, fork_at, Some(before), turn.entry) {
            self.next_turn += 1;
            self.turns.push(turn);
            self.input.clear();
            self.mention_closed = None;
        } else {
            // Not sent: give the message back.
            self.entries.truncate(turn.entry);
            self.attachments = attachments;
        }
    }

    /// `/compactar`: the CLI's own `/compact` (Claude Code, Antigravity) or a summary that seeds a
    /// new session (Codex). Returns false when there is nothing to compact or it cannot run now.
    pub fn compact(&mut self) -> bool {
        self.compact_requested = false;
        if self.running() || self.program.is_none() || self.project.is_none() {
            return false;
        }
        if self.session.is_empty() {
            self.notice("No hay ninguna sesión que compactar todavía.", Tone::Muted);
            return false;
        }
        let first = self.entries.len();
        match self.kind {
            Kind::Claude | Kind::Antigravity => self.start("/compact", vec![], Purpose::CompactNative, true, String::new(), None, first),
            Kind::Codex => self.start(SUMMARY_PROMPT, vec![], Purpose::CompactSummary, false, String::new(), None, first),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn start(&mut self, prompt: &str, attachments: Vec<Attachment>, purpose: Purpose, raw: bool, fork_at: String, before: Option<Snapshot>, first_entry: usize) -> bool {
        let Some(program) = self.program.clone() else { return false };
        let Some((root, name)) = self.project.clone() else { return false };
        let docs = docs_dir();
        let tool = self.tool_override.clone().or_else(modengine_tool);
        let store_cache = modengine_store::net::cache_dir();
        if let Some(c) = &store_cache {
            // Codex makes it writable with --add-dir, which wants it to exist
            let _ = std::fs::create_dir_all(c);
        }
        let plan = self.permission == Permission::Plan;
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let context_file = std::env::temp_dir().join(format!("modengine-agent-{}-{n}.md", std::process::id()));
        let context = agent::engine_context_for(&name, docs.as_deref(), tool.as_deref().filter(|_| !raw), plan, self.internet, store_cache.as_deref());
        if let Err(e) = std::fs::write(&context_file, context) {
            self.notice(format!("No se pudo escribir un fichero temporal: {e}"), Tone::Error);
            return false;
        }
        let run = Run {
            prompt: prompt.to_string(),
            session: self.session.clone(),
            model: self.model(),
            permission: self.permission,
            root: root.clone(),
            context_file: context_file.clone(),
            docs,
            name,
            attachments,
            raw,
            fork_at,
            seed: if self.session.is_empty() { self.seed.clone() } else { String::new() },
            tool,
            internet: self.internet,
            store_cache,
        };
        self.open_text = None;
        match self.spawn(&program, &run) {
            Ok((child, rx)) => {
                self.status = match purpose {
                    Purpose::Chat(_) => "Trabajando…".into(),
                    _ => "Compactando la conversación…".into(),
                };
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
                    failed: false,
                    compacted: false,
                    stderr: String::new(),
                    changed: BTreeSet::new(),
                    blocked: 0,
                    context_file,
                    root,
                    purpose,
                    before,
                    first_entry,
                });
                true
            }
            Err(e) => {
                let _ = std::fs::remove_file(&context_file);
                self.notice(format!("No se pudo lanzar {}: {e}", program.display()), Tone::Error);
                self.status = "Falló".into();
                false
            }
        }
    }

    fn spawn(&self, program: &Path, run: &Run) -> std::io::Result<(Child, Receiver<Msg>)> {
        let mut args = self.kind.args(run);
        // A .cmd/.bat shim (npm installs) runs through cmd.exe: no line breaks in an argument
        // (Rust refuses them) and at most 8191 characters on the whole command line.
        if cfg!(windows) && program.extension().is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat")) {
            for a in &mut args {
                if a.contains(['\r', '\n']) {
                    *a = a.replace("\r\n", " ").replace(['\r', '\n'], " ");
                }
            }
            // As Rust escapes it for cmd.exe: quotes doubled, each % made 8 characters longer.
            let len = 40 + program.as_os_str().len() + args.iter().map(|a| a.chars().count() + 3 + a.matches('"').count() + 8 * a.matches('%').count()).sum::<usize>();
            if len > 8000 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("el mensaje es demasiado largo para un programa .cmd/.bat en Windows (unos {len} caracteres; el máximo es 8191). Acórtalo o quita adjuntos"),
                ));
            }
        }
        let mut cmd = crate::proc::command(program);
        cmd.args(args).current_dir(&run.root).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
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
        #[cfg(windows)]
        kill_tree(&mut r.child);
        #[cfg(not(any(unix, windows)))]
        let _ = r.child.kill();
        self.status = "Cancelando…".into();
    }

    /// Reads what the agent printed since the last frame. Call every frame.
    pub fn poll(&mut self, ctx: &egui::Context) -> Polled {
        let mut out = Polled { reload: std::mem::take(&mut self.pending_reload), ..Default::default() };
        // `--agent-demo store`: the sample turn's Salida lines go out like a real turn's
        if self.demo && self.run.is_none() && !self.downloaded.is_empty() {
            out.imported = std::mem::take(&mut self.imported);
            out.downloaded = std::mem::take(&mut self.downloaded);
            out.finished = true;
        }
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
        let Some(mut r) = self.run.take() else {
            if self.compact_requested {
                self.compact();
            }
            if !self.queue.is_empty() || self.auto_send.is_some() {
                ctx.request_repaint();
            }
            return out;
        };
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
                out.imported = std::mem::take(&mut self.imported);
                out.downloaded = std::mem::take(&mut self.downloaded);
                out.turn = self.turn_result.take();
                out.reload = true;
                out.finished = true;
                if self.compact_requested {
                    self.compact();
                }
                ctx.request_repaint();
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
        let mut ok = false;
        if r.stop_at.is_some() {
            self.notice("Cancelado.", Tone::Muted);
            self.status = "Cancelado".into();
            // Nothing queued goes out on its own after a cancel; ▶ Seguir sends it.
            if !self.queue.is_empty() {
                self.queue_paused = true;
            }
            self.compact_requested = false;
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
            ok = !r.failed;
            // Fallbacks such as a mode the account cannot use only show up as warnings on stderr.
            for l in r.stderr.lines().filter(|l| l.to_lowercase().starts_with("warning:") && !l.contains("no output and no recorded error")) {
                self.notice(l, Tone::Warning);
            }
        }
        if r.blocked > 0 && r.stop_at.is_none() {
            self.notice("Algunas acciones se bloquearon. Elige qué puede hacer el agente en el menú de permisos.", Tone::Muted);
        }
        match r.purpose {
            Purpose::Chat(turn) => {
                if let Some(before) = r.before.take() {
                    let after = Snapshot::take(&before.root);
                    let files = snapshot::changes(&before, &after);
                    self.imported = imported_assets(&files);
                    let credits = Path::new(modengine_store::credits::FILE);
                    let text = |b: Option<&[u8]>| b.map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
                    self.downloaded = store_downloads(&text(before.bytes(credits)), &text(after.bytes(credits)));
                    let new: Vec<String> = files.iter().filter(|f| f.kind != ChangeKind::Deleted).map(|f| f.rel.clone()).collect();
                    if self.turn_workspace && !new.is_empty() {
                        let mut lines = BTreeMap::new();
                        for f in files.iter().filter(|f| f.kind == ChangeKind::Modified && !f.binary) {
                            let rel = Path::new(&f.rel);
                            if let (Some(a), Some(b)) = (before.bytes(rel), after.bytes(rel)) {
                                lines.insert(f.rel.clone(), changed_lines(&String::from_utf8_lossy(a), &String::from_utf8_lossy(b)));
                            }
                        }
                        self.turn_result = Some(TurnResult { prompt: self.turn_prompt.clone(), files: new, lines });
                    }
                    if !files.is_empty() {
                        self.entries.push(Entry::Changes { turn, files });
                        self.snaps.push((turn, before));
                        if self.snaps.len() > 12 {
                            self.snaps.remove(0);
                        }
                    }
                }
                if ok && !self.session.is_empty() {
                    self.seed.clear();
                }
                if !ok && !self.queue.is_empty() {
                    self.queue_paused = true;
                }
                if ok {
                    self.check_context();
                }
            }
            Purpose::CompactNative => {
                if ok {
                    if !r.compacted {
                        self.entries.push(Entry::Divider("Conversación compactada".into()));
                    }
                    self.status = "Conversación compactada".into();
                    self.warned_full = false;
                }
            }
            Purpose::CompactSummary => {
                let summary: Vec<String> = self.entries[r.first_entry.min(self.entries.len())..]
                    .iter()
                    .filter_map(|e| if let Entry::Text(t) = e { Some(t.clone()) } else { None })
                    .collect();
                let summary = summary.join("\n\n");
                if ok && !summary.trim().is_empty() {
                    self.entries.truncate(r.first_entry);
                    self.entries.push(Entry::Divider("Conversación compactada".into()));
                    self.entries.push(Entry::Summary(summary.clone()));
                    self.usage.cost_base += self.usage.session_cost;
                    self.usage.session_cost = 0.0;
                    self.usage.context = (summary.len() / 4) as u64;
                    self.session.clear();
                    self.checkpoint.clear();
                    self.seed = summary;
                    self.status = "Conversación compactada: el siguiente mensaje sigue con el resumen".into();
                    self.warned_full = false;
                }
            }
        }
        self.save_conversation();
    }

    /// After a turn: warns when the context is filling up and compacts when set to.
    fn check_context(&mut self) {
        let Some(f) = self.usage.fraction() else { return };
        if self.autocompact > 0 && f * 100.0 >= self.autocompact as f32 {
            self.notice(format!("El contexto va por el {:.0} %: compactando la conversación (autocompactar al {} %).", f * 100.0, self.autocompact), Tone::Muted);
            self.compact_requested = true;
        } else if f >= WARN_AT && !self.warned_full {
            self.warned_full = true;
            self.notice(format!("El contexto va por el {:.0} %. /compactar lo resume y libera sitio.", f * 100.0), Tone::Warning);
        }
    }

    /// Applies one event; true when the project should be reloaded from disk now.
    fn handle(&mut self, r: &mut Running, e: Event) -> bool {
        let compacting = !matches!(r.purpose, Purpose::Chat(_));
        match e {
            Event::Session(s) => {
                if !self.session.is_empty() && !s.is_empty() && s != self.session {
                    // A forked or new session: what the old one cost stays counted.
                    self.usage.cost_base += self.usage.session_cost;
                    self.usage.session_cost = 0.0;
                }
                self.session = s;
            }
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
                    if !compacting {
                        self.notice(format!("Modelo: {m}"), Tone::Muted);
                    }
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
            Event::Usage(u) => {
                if let Some(c) = u.context {
                    self.usage.context = c;
                }
                if let Some(w) = u.window {
                    self.usage.window = w;
                }
                if let Some(c) = u.cost {
                    self.usage.session_cost = c;
                }
                if let Some(o) = u.output {
                    self.usage.output += o;
                }
                self.usage.approx = r.kind != Kind::Claude;
                if self.usage.window == 0 {
                    self.usage.window = self.listed_window();
                }
            }
            Event::Compacted(post) => {
                r.compacted = true;
                self.open_text = None;
                self.entries.push(Entry::Divider("Conversación compactada".into()));
                if let Some(t) = post {
                    self.usage.context = t;
                }
            }
            Event::Checkpoint(c) => self.checkpoint = c,
            Event::Finished => {
                r.got_result = true;
                self.status = format!("Hecho en {:.1} s", r.started.elapsed().as_secs_f32());
            }
            Event::Failed(s) => {
                r.got_result = true;
                r.failed = true;
                self.notice(s, Tone::Error);
                self.status = "Falló".into();
            }
        }
        false
    }

    // ------------------------------------------------------------ changes

    fn snapshot_of(&self, turn: u64) -> Option<&Snapshot> {
        self.snaps.iter().find(|(t, _)| *t == turn).map(|(_, s)| s)
    }

    /// Revertir on one file of a turn.
    pub fn revert_file(&mut self, turn: u64, rel: &str) -> Result<(), String> {
        let snap = self.snapshot_of(turn).ok_or("Ya no se guarda cómo estaba antes (solo los últimos turnos de esta sesión del editor)")?;
        snap.revert(Path::new(rel))?;
        for e in &mut self.entries {
            if let Entry::Changes { turn: t, files } = e {
                if *t == turn {
                    for f in files.iter_mut().filter(|f| f.rel == rel) {
                        f.reverted = true;
                    }
                }
            }
        }
        self.pending_reload = true;
        self.save_conversation();
        Ok(())
    }

    /// `/deshacer`: puts back every file the last turn that changed files changed. Returns how
    /// many it reverted.
    pub fn undo_last_turn(&mut self) -> usize {
        if self.running() {
            return 0;
        }
        let found = self.entries.iter().rev().find_map(|e| match e {
            Entry::Changes { turn, files } if files.iter().any(|f| !f.reverted) => Some((*turn, files.iter().filter(|f| !f.reverted).map(|f| f.rel.clone()).collect::<Vec<_>>())),
            _ => None,
        });
        let Some((turn, files)) = found else {
            self.notice("No hay cambios del agente que deshacer.", Tone::Muted);
            return 0;
        };
        let mut done = vec![];
        let mut errors = vec![];
        for f in files {
            match self.revert_file(turn, &f) {
                Ok(()) => done.push(f),
                Err(e) => errors.push(e),
            }
        }
        if !done.is_empty() {
            self.notice(format!("Deshecho: {} vuelve{} a como estaba{} antes del turno.", done.join(", "), if done.len() > 1 { "n" } else { "" }, if done.len() > 1 { "n" } else { "" }), Tone::Muted);
        }
        if !errors.is_empty() {
            self.notice(errors.join("\n"), Tone::Error);
        }
        done.len()
    }

    /// Opens the diff window for one file of a turn (before the turn vs. now).
    pub fn open_diff(&mut self, turn: u64, rel: &str) {
        let now = self.project.as_ref().map(|(r, _)| r.join(rel)).and_then(|p| std::fs::read(p).ok());
        let (lines, note) = match self.snapshot_of(turn) {
            Some(s) => {
                let before = s.bytes(Path::new(rel)).map(|b| String::from_utf8_lossy(b).into_owned());
                let existed = s.files.contains_key(Path::new(rel));
                match (before, now) {
                    (Some(b), Some(n)) if !n.contains(&0) => (snapshot::unified(&b, &String::from_utf8_lossy(&n), 3), None),
                    (None, Some(n)) if !existed && !n.contains(&0) => (snapshot::unified("", &String::from_utf8_lossy(&n), 3), Some("Fichero nuevo".to_string())),
                    (Some(b), None) => (snapshot::unified(&b, "", 3), Some("Borrado".to_string())),
                    _ => (vec![], Some("Fichero binario o demasiado grande para compararlo".to_string())),
                }
            }
            None => (vec![], Some("Ya no se guarda cómo estaba antes del turno (solo los últimos turnos de esta sesión del editor)".to_string())),
        };
        let note = if lines.is_empty() && note.is_none() { Some("Sin diferencias con el disco (¿ya revertido?)".to_string()) } else { note };
        self.diff = Some(DiffView { turn, rel: rel.to_string(), lines, note });
    }

    // ------------------------------------------------------------ commands

    /// Runs a slash command typed in the message box. Returns a message to send (the app saves
    /// first), e.g. for /reintentar.
    pub fn command(&mut self, cmd: Slash, ctx: Option<&egui::Context>) -> Option<String> {
        self.input.clear();
        match cmd {
            Slash::New => self.new_conversation(),
            Slash::History(q) => self.show_history(&q),
            Slash::Compact => {
                if self.running() {
                    self.compact_requested = true;
                    self.status = "Se compactará al terminar".into();
                } else {
                    self.compact();
                }
            }
            Slash::Model(None) => {
                let m = self.model();
                let shown = if m.is_empty() { "el de por defecto".to_string() } else { self.model_name(&m) };
                self.notice(format!("Modelo: {shown}. Cámbialo con /modelo <nombre> o /modelo defecto."), Tone::Muted);
            }
            Slash::Model(Some(m)) => {
                self.set_model(&m);
                self.notice(if m.is_empty() { "Modelo: el de por defecto.".to_string() } else { format!("Modelo: {}.", self.model_name(&m)) }, Tone::Muted);
            }
            Slash::Permission(Some(p)) => {
                self.set_permission(p);
                self.notice(format!("Permisos: {}.", p.label()), Tone::Muted);
            }
            Slash::Permission(None) => self.notice("Usa /permisos plan, /permisos editar o /permisos auto.", Tone::Muted),
            Slash::Agent(Some(k)) => {
                if self.running() {
                    self.notice("Espera a que termine el agente para cambiarlo.", Tone::Warning);
                } else {
                    self.set_kind(k);
                    self.notice(format!("Agente: {}.", k.name()), Tone::Muted);
                }
            }
            Slash::Agent(None) => self.notice("Usa /agente claude, /agente codex o /agente agy.", Tone::Muted),
            Slash::Retry => {
                if self.retry().is_none() {
                    self.notice("No hay ningún mensaje que reintentar ahora.", Tone::Muted);
                }
                return self.auto_send.take();
            }
            Slash::Edit => {
                if !self.edit_last() {
                    self.notice("No hay ningún mensaje que editar ahora.", Tone::Muted);
                }
                return None;
            }
            Slash::Undo => {
                self.undo_last_turn();
            }
            Slash::Cancel => {
                if self.running() {
                    self.stop();
                } else {
                    self.notice("El agente no está trabajando.", Tone::Muted);
                }
            }
            Slash::Export => {
                if self.entries.is_empty() {
                    self.notice("La conversación está vacía.", Tone::Muted);
                } else if let Some(path) = self.export("", None) {
                    if let (Some(ctx), Some(md)) = (ctx, self.markdown("")) {
                        ctx.copy_text(md);
                    }
                    self.notice(format!("Exportada a {} (y copiada).", path.display()), Tone::Muted);
                }
            }
            Slash::Rename(t) => {
                if t.trim().is_empty() {
                    self.notice("Usa /renombrar <título>.", Tone::Muted);
                } else {
                    self.rename("", &t);
                    self.notice(format!("Título: {}", t.trim()), Tone::Muted);
                }
            }
            Slash::AutoCompact(Some(n)) => {
                self.autocompact = n;
                self.save_state();
                self.notice(if n == 0 { "Autocompactar: no.".to_string() } else { format!("Autocompactar al {n} % del contexto.") }, Tone::Muted);
            }
            Slash::AutoCompact(None) => {
                let now = if self.autocompact == 0 { "no".to_string() } else { format!("al {} %", self.autocompact) };
                self.notice(format!("Autocompactar: {now}. Usa /autocompactar 85 o /autocompactar no."), Tone::Muted);
            }
            Slash::Internet(Some(on)) => {
                self.set_internet(on);
                self.notice(if on { "🌐 Internet: sí. El agente puede buscar en la web y leer páginas." } else { "🌐 Internet: no. El agente no busca en la web (solo lee las URLs que adjuntes)." }, Tone::Muted);
            }
            Slash::TurnWorkspace(Some(on)) => {
                self.turn_workspace = on;
                self.save_state();
                self.notice(if on { "Al terminar cada tarea se abre un workspace con lo nuevo." } else { "Lo nuevo ya no se abre en un workspace al terminar." }, Tone::Muted);
            }
            Slash::TurnWorkspace(None) => {
                self.notice(format!("Workspace por tarea: {}. Usa /workspace on o /workspace off.", if self.turn_workspace { "sí" } else { "no" }), Tone::Muted);
            }
            Slash::Internet(None) => {
                self.notice(format!("🌐 Internet: {}. Usa /internet on o /internet off.", if self.internet { "sí" } else { "no" }), Tone::Muted);
            }
            Slash::Cost => {
                let t = self.usage_text(true);
                self.notice(if t.is_empty() { "Todavía no hay datos de uso.".to_string() } else { t }, Tone::Muted);
            }
            Slash::PixelArt(d) if d.trim().is_empty() => self.notice("Usa /pixelart <qué dibujar>, p. ej. /pixelart llave dorada de 16×16 para recoger.", Tone::Muted),
            Slash::PixelArt(d) => return Some(slash::pixel_art_message(d.trim())),
            Slash::Help => self.notice(slash::help(), Tone::Muted),
            Slash::Unknown(n) => self.notice(format!("No conozco /{n}. /ayuda lista los comandos (// envía un mensaje que empieza por /)."), Tone::Warning),
        }
        None
    }

    /// "31,1k / 1M (3 %) · 0,12 $", or the long form for /coste.
    pub fn usage_text(&self, long: bool) -> String {
        let u = &self.usage;
        let mut parts = vec![];
        if u.context > 0 {
            let approx = if u.approx { "≈" } else { "" };
            parts.push(match u.fraction() {
                Some(f) if long => format!("Contexto: {approx}{} de {} tokens ({:.0} %)", history::tokens(u.context), history::tokens(u.window), f * 100.0),
                Some(f) => format!("{approx}{} / {} ({:.0} %)", history::tokens(u.context), history::tokens(u.window), f * 100.0),
                None if long => format!("Contexto: {approx}{} tokens", history::tokens(u.context)),
                None => format!("{approx}{} tokens", history::tokens(u.context)),
            });
        }
        if long && u.output > 0 {
            parts.push(format!("{} tokens escritos", history::tokens(u.output)));
        }
        if u.cost() > 0.0 {
            parts.push(if long { format!("Coste: {}", history::money(u.cost())) } else { history::money(u.cost()) });
        }
        parts.join(" · ")
    }

    // ------------------------------------------------------------ ui

    /// Draws the dock. Returns a message to send: the app saves everything, then calls [`AgentPanel::send`].
    /// `src` is what can be attached from the rest of the editor.
    pub fn ui(&mut self, ui: &mut egui::Ui, src: &Sources) -> Option<String> {
        if let Some(p) = src.project {
            self.enter_project(&p.dir, &p.game.game.name);
        }
        self.receive_images();
        let has_project = src.project.is_some();
        let panel_rect = ui.max_rect();
        let pal = crate::theme::colors(ui.ctx());
        let running = self.running();
        let mut send = if has_project { self.next_to_send() } else { None };

        // Toolbar: agent, history, new conversation, stop, more.
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
                ui.menu_button("⋯", |ui| self.more_menu(ui)).response.on_hover_text("Compactar, exportar, deshacer, autocompactar…");
                if ui.add_enabled(running, egui::Button::new("■")).on_hover_text("Cancelar el trabajo del agente (Esc) — /cancelar").clicked() {
                    self.stop();
                }
                if ui.add_enabled(self.program.is_some(), egui::Button::new("＋")).on_hover_text("Nueva conversación (la actual queda en el historial) — /nueva").clicked() {
                    self.new_conversation();
                }
                let hist = self.view == View::History;
                if ui.selectable_label(hist, "🕘").on_hover_text("Historial: conversaciones anteriores de este proyecto — /historial").clicked() {
                    if hist {
                        self.view = View::Chat;
                    } else {
                        self.show_history("");
                    }
                }
            });
        });
        ui.horizontal(|ui| {
            if running {
                ui.spinner();
            }
            let status = if self.program.is_none() { format!("{} no encontrado", self.kind.name()) } else { self.status.clone() };
            let meter_w = if self.usage.context > 0 || self.usage.cost() > 0.0 { 190.0f32.min(ui.available_width() * 0.6) } else { 0.0 };
            ui.add_sized([(ui.available_width() - meter_w - 8.0).max(40.0), 18.0], egui::Label::new(RichText::new(status).weak()).truncate());
            if meter_w > 0.0 {
                self.meter(ui, meter_w);
            }
        });
        if !self.title.is_empty() && self.view == View::Chat {
            ui.add(egui::Label::new(RichText::new(&self.title).strong().small()).truncate()).on_hover_text("Título de la conversación (/renombrar)");
        }
        ui.separator();

        let Some(_) = self.program.clone() else {
            self.missing_ui(ui);
            return None;
        };

        if self.view == View::History {
            self.history_ui(ui);
            self.windows(ui.ctx());
            return send;
        }

        // Message box and its footer (model, permissions, send) at the bottom.
        let input_id = ui.make_persistent_id("agent_input");
        egui::Panel::bottom("agent_input_area").frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(0, 6))).show(ui, |ui| {
            let focused = ui.memory(|m| m.has_focus(input_id));
            self.queue_ui(ui);
            if self.editing.is_some() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("✎ Editando tu último mensaje: al enviarlo, la conversación sigue desde antes de él.").color(pal.info).small());
                    if ui.small_button("Cancelar").clicked() {
                        self.cancel_edit();
                    }
                });
            }
            // Attachments of the next message, each with its ×.
            if !self.attachments.is_empty() {
                let mut remove = None;
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                    for (i, a) in self.attachments.iter().enumerate() {
                        let thumb = if a.kind == AttachKind::Image {
                            self.thumbs.entry(a.path.clone()).or_insert_with(|| crate::agent_paste::thumbnail(ui.ctx(), &a.path)).clone()
                        } else {
                            None
                        };
                        if image_chip(ui, a, thumb.as_ref()) || (thumb.is_none() && chip(ui, a, true)) {
                            remove = Some(i);
                        }
                    }
                });
                if let Some(i) = remove {
                    self.attachments.remove(i);
                }
                // Art from outside the project: one click writes the instruction to import it.
                if let Some(text) = import_instruction(self.project.as_ref().map(|(r, _)| r.as_path()), &self.attachments) {
                    let plan = self.permission == Permission::Plan;
                    let tip = if plan {
                        "En «Solo planificar» el agente no puede importar: elige «Editar archivos» en el menú de permisos"
                    } else {
                        "Escribe la instrucción para que el agente mire cada imagen y la importe con modengine import (puedes editarla antes de enviar)"
                    };
                    let r = ui.add_enabled(!plan, egui::Button::new("🖼 Importar al proyecto").small()).on_hover_text(tip).on_disabled_hover_text(tip);
                    if r.clicked() {
                        self.input = text;
                        ui.memory_mut(|m| m.request_focus(input_id));
                        move_cursor_to_end(ui.ctx(), input_id, &self.input);
                    }
                }
                ui.add_space(2.0);
            }
            // A screenshot pasted with Ctrl+V (anywhere over the panel, or in the message box).
            let over_panel = ui.ctx().pointer_latest_pos().is_some_and(|p| panel_rect.contains(p));
            if focused || over_panel {
                let images: Vec<std::sync::Arc<egui::ColorImage>> =
                    ui.input(|i| i.events.iter().filter_map(|e| if let egui::Event::PasteImage(img) = e { Some(img.clone()) } else { None }).collect());
                for img in &images {
                    self.attach_image(img, "captura");
                }
                if !images.is_empty() {
                    ui.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::PasteImage(_))));
                }
            }
            // Pasting paths or file:// URIs attaches them.
            if focused {
                let pasted: Vec<String> = ui.input(|i| i.events.iter().filter_map(|e| if let egui::Event::Paste(t) = e { Some(t.clone()) } else { None }).collect());
                for t in pasted {
                    if self.attach_pasted(&t) {
                        ui.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Paste(x) if *x == t)));
                    }
                }
            }
            // `/` suggestions while typing a command, `@` suggestions for the word being typed;
            // arrows move, Tab or Intro pick, Esc closes.
            let cmds = slash::suggestions(&self.input);
            let cands = if cmds.is_empty() { self.mention_candidates(src) } else { vec![] };
            let mut picked = None;
            let mut picked_cmd: Option<&'static slash::Spec> = None;
            let n = if cmds.is_empty() { cands.len() } else { cmds.len() };
            if n > 0 {
                let sel = if cmds.is_empty() { &mut self.mention_sel } else { &mut self.slash_sel };
                *sel = (*sel).min(n - 1);
                if focused {
                    let (down, up, pick, esc) = ui.input_mut(|i| {
                        (
                            i.consume_key(Modifiers::NONE, Key::ArrowDown),
                            i.consume_key(Modifiers::NONE, Key::ArrowUp),
                            i.consume_key(Modifiers::NONE, Key::Tab) || (cmds.is_empty() && i.consume_key(Modifiers::NONE, Key::Enter)),
                            i.consume_key(Modifiers::NONE, Key::Escape),
                        )
                    });
                    if down {
                        *sel = (*sel + 1) % n;
                    }
                    if up {
                        *sel = (*sel + n - 1) % n;
                    }
                    if pick {
                        if cmds.is_empty() {
                            picked = Some(*sel);
                        } else {
                            picked_cmd = Some(cmds[*sel]);
                        }
                    }
                    if esc {
                        if cmds.is_empty() {
                            self.mention_closed = Some(self.input.clone());
                        } else {
                            self.input.clear();
                        }
                    }
                }
                egui::Frame::new()
                    .fill(ui.visuals().window_fill)
                    .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                    .corner_radius(crate::theme::RADIUS)
                    .inner_margin(4)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        if cmds.is_empty() {
                            ui.label(RichText::new("Adjuntar (Tab o Intro; Esc cierra)").weak().small());
                            for (i, (label, a)) in cands.iter().enumerate() {
                                let tip = a.path.display().to_string();
                                if ui.selectable_label(i == self.mention_sel, label).on_hover_text(tip).clicked() {
                                    picked = Some(i);
                                }
                            }
                        } else {
                            ui.label(RichText::new("Comandos (Tab completa, Intro ejecuta, Esc cierra)").weak().small());
                            egui::ScrollArea::vertical().id_salt("agent_slash").max_height(220.0).show(ui, |ui| {
                                for (i, c) in cmds.iter().enumerate() {
                                    let arg = if c.arg.is_empty() { String::new() } else { format!(" {}", c.arg) };
                                    let r = ui.selectable_label(i == self.slash_sel, RichText::new(format!("/{}{arg}", c.name)).monospace()).on_hover_text(c.help);
                                    if r.clicked() {
                                        picked_cmd = Some(c);
                                    }
                                    if i == self.slash_sel {
                                        ui.label(RichText::new(c.help).weak().small());
                                    }
                                }
                            });
                        }
                    });
            }
            if let Some(i) = picked {
                if let Some((_, a)) = cands.into_iter().nth(i) {
                    self.pick_mention(a);
                    move_cursor_to_end(ui.ctx(), input_id, &self.input);
                    ui.memory_mut(|m| m.request_focus(input_id));
                }
            }
            if let Some(c) = picked_cmd {
                self.input = format!("/{}{}", c.name, if c.arg.is_empty() { "" } else { " " });
                self.slash_sel = 0;
                move_cursor_to_end(ui.ctx(), input_id, &self.input);
                ui.memory_mut(|m| m.request_focus(input_id));
            }
            let enter = picked.is_none() && picked_cmd.is_none() && focused && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
            // Esc in the message box cancels the agent (the suggestion popups took theirs above).
            if running && focused && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                self.stop();
            }
            let hint = if running {
                "El agente está trabajando: lo que escribas se envía cuando termine (Esc cancela)".to_string()
            } else {
                format!("Mensaje para {} (Mayús+Intro: nueva línea; @ adjunta; / comandos)", self.kind.name())
            };
            // Screenshots: one click (or Ctrl+V over the panel) instead of saving a file first.
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let small = |t: &str| egui::Button::new(RichText::new(t).small()).frame(false);
                if ui.add(small("📋 Pegar imagen")).on_hover_text("La imagen que tengas copiada: una captura de pantalla (Impr Pant, Spectacle, Recortes…). También Ctrl+V con el ratón sobre el panel").clicked() {
                    self.paste_clipboard_image();
                }
                ui.label(RichText::new("·").weak().small());
                if ui.add(small("📸 Captura del editor")).on_hover_text("Adjunta lo que ves ahora en la ventana del editor").clicked() {
                    self.capture_editor(ui.ctx());
                }
            });
            ui.add(egui::TextEdit::multiline(&mut self.input).id(input_id).hint_text(hint).desired_rows(3).desired_width(f32::INFINITY));
            ui.horizontal_wrapped(|ui| {
                ui.menu_button("📎 Adjuntar", |ui| self.attach_menu(ui, src))
                    .response
                    .on_hover_text("Adjunta contexto al siguiente mensaje: assets, ficheros del proyecto o de cualquier parte del disco, URLs, texto, la selección de la room o la Salida. También puedes soltar aquí assets y ficheros, pegar rutas o escribir @");
                // Model and permissions can't change mid-turn: hidden then, which leaves room for ■ Cancelar.
                if !running {
                ui.scope(|ui| {
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
                    let mut on = self.internet;
                    let tip = match self.kind {
                        Kind::Antigravity => INTERNET_TIP_AGY,
                        _ => INTERNET_TIP,
                    };
                    // compact: highlighted when on (the tip says which)
                    let state = if on { "🌐 Internet: sí" } else { "🌐 Internet: no" };
                    let label = if on { RichText::new("🌐") } else { RichText::new("🌐").weak().strikethrough() };
                    if ui.toggle_value(&mut on, label).on_hover_text(format!("{state}. {tip}")).changed() {
                        self.set_internet(on);
                    }
                });
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let has_text = !self.input.trim().is_empty() || !self.attachments.is_empty();
                    let is_cmd = slash::parse(&self.input).is_some() && self.attachments.is_empty();
                    let can = has_project && has_text;
                    let label = if is_cmd {
                        "Ejecutar"
                    } else if running {
                        "Poner en cola"
                    } else {
                        "Enviar"
                    };
                    let tip = if running { "Intro: se envía cuando el agente termine" } else { "Intro: guarda todo y envía el mensaje" };
                    let b = ui.add_enabled(can || is_cmd, crate::theme::primary_button(ui, label)).on_hover_text(tip);
                    if running {
                        let cancelling = self.run.as_ref().is_some_and(|r| r.stop_at.is_some());
                        let text = if cancelling { "Cancelando…" } else { "■ Cancelar" };
                        let c = egui::Button::new(RichText::new(text).color(pal.error)).stroke(egui::Stroke::new(1.0, pal.error));
                        if ui.add_enabled(!cancelling, c).on_hover_text("Para al agente y a los comandos que lanzó (Esc). Lo que ya cambió se puede revertir; la cola queda en pausa").clicked() {
                            self.stop();
                        }
                    }
                    if b.clicked() || (enter && (can || is_cmd)) {
                        if let Some(cmd) = slash::parse(&self.input).filter(|_| self.attachments.is_empty()) {
                            send = self.command(cmd, Some(ui.ctx())).or(send.take());
                        } else if running {
                            let t = self.input.clone();
                            self.enqueue(&t);
                        } else {
                            // `//text` sends "/text".
                            let t = self.input.trim_start();
                            send = Some(t.strip_prefix("//").map(|r| format!("/{r}")).unwrap_or_else(|| self.input.clone()));
                        }
                    }
                });
            });
            if !has_project {
                ui.label(RichText::new("Abre un proyecto: el agente trabaja en su carpeta.").color(pal.error));
            }
        });

        // Transcript.
        let mut action: Option<Action> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
            if self.entries.is_empty() {
                ui.label(RichText::new("Pide al agente que cambie objetos, rooms, sprites o scripts del proyecto. Escribe / para ver los comandos.").weak());
            }
            let last_user = self.last_turn().map(|i| self.turns[i].entry);
            let last_error = self.entries.iter().rposition(|e| matches!(e, Entry::Notice(_, Tone::Error)));
            let last_error = last_error.filter(|i| *i + 1 >= self.entries.len() && !running && !self.turns.is_empty());
            for (i, e) in self.entries.iter().enumerate() {
                match e {
                    Entry::Attached(list) => {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                            ui.label(RichText::new("📎").weak());
                            for a in list {
                                chip(ui, a, false);
                            }
                        });
                    }
                    Entry::User(t) => {
                        ui.add_space(8.0);
                        let r = egui::Frame::new()
                            .fill(ui.visuals().faint_bg_color)
                            .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                            .corner_radius(crate::theme::RADIUS)
                            .inner_margin(6)
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.label(t);
                            })
                            .response;
                        let last = last_user == Some(i);
                        if ui.rect_contains_pointer(r.rect) && overlay_button(ui, r.rect, ("agent_copy_user", i), "⧉", "Copiar el mensaje") {
                            ui.ctx().copy_text(t.clone());
                        }
                        if last {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 2.0;
                                {
                                    if ui.add(egui::Button::new(RichText::new("✎ Editar").small()).frame(false)).on_hover_text("Edita el mensaje y reenvíalo: la conversación sigue desde antes de él").clicked() {
                                        action = Some(Action::Edit);
                                    }
                                    if ui.add(egui::Button::new(RichText::new("↻ Reintentar").small()).frame(false)).on_hover_text("Vuelve a enviarlo desde antes de la respuesta").clicked() {
                                        action = Some(Action::Retry);
                                    }
                                }
                            });
                        }
                    }
                    Entry::Text(t) => {
                        ui.add_space(4.0);
                        let r = ui.scope(|ui| crate::agent_markdown::show(ui, t)).response;
                        if ui.rect_contains_pointer(r.rect) && !t.is_empty() && overlay_button(ui, r.rect, ("agent_copy_text", i), "⧉", "Copiar la respuesta") {
                            ui.ctx().copy_text(t.clone());
                        }
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
                        if last_error == Some(i) && ui.button("↻ Reintentar").on_hover_text("Vuelve a enviar el último mensaje").clicked() {
                            action = Some(Action::Retry);
                        }
                    }
                    Entry::Divider(t) => {
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            let w = ((ui.available_width() - 180.0) / 2.0).max(10.0);
                            ui.add_sized([w, 10.0], egui::Separator::default().horizontal());
                            ui.label(RichText::new(format!("✂ {t}")).color(pal.info).small());
                            ui.add(egui::Separator::default().horizontal());
                        });
                        ui.add_space(4.0);
                    }
                    Entry::Summary(t) => {
                        let open = self.unfolded.contains(&i);
                        if ui.selectable_label(open, RichText::new(if open { "▾ Resumen" } else { "▸ Resumen de lo anterior" }).weak()).clicked() {
                            action = Some(Action::Fold(i));
                        }
                        if open {
                            egui::Frame::new().fill(ui.visuals().faint_bg_color).corner_radius(crate::theme::RADIUS).inner_margin(6).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                crate::agent_markdown::show(ui, t);
                            });
                        }
                    }
                    Entry::Changes { turn, files } => {
                        let can_revert = self.snaps.iter().any(|(t, _)| t == turn);
                        changes_ui(ui, *turn, files, can_revert, &mut action);
                    }
                }
            }
        });
        match action {
            Some(Action::Edit) => {
                self.edit_last();
                ui.memory_mut(|m| m.request_focus(input_id));
                move_cursor_to_end(ui.ctx(), input_id, &self.input);
            }
            Some(Action::Retry) => send = self.retry().or(send),
            Some(Action::Fold(i)) => {
                if !self.unfolded.remove(&i) {
                    self.unfolded.insert(i);
                }
            }
            Some(Action::Diff(turn, rel)) => self.open_diff(turn, &rel),
            Some(Action::Revert(turn, rel)) => {
                if let Err(e) = self.revert_file(turn, &rel) {
                    self.notice(e, Tone::Error);
                }
            }
            Some(Action::RevertAll(turn)) => {
                let files: Vec<String> = self
                    .entries
                    .iter()
                    .find_map(|e| match e {
                        Entry::Changes { turn: t, files } if *t == turn => Some(files.iter().filter(|f| !f.reverted).map(|f| f.rel.clone()).collect()),
                        _ => None,
                    })
                    .unwrap_or_default();
                for f in files {
                    if let Err(e) = self.revert_file(turn, &f) {
                        self.notice(e, Tone::Error);
                    }
                }
            }
            None => {}
        }
        if send.is_some() {
            self.auto_send = None;
        }
        if !self.list_tried && self.lister.is_none() {
            self.list_models();
        }
        self.drop_zone(ui, panel_rect, src);
        self.windows(ui.ctx());
        send
    }

    /// The context meter: tokens in context against the window, and the cost.
    fn meter(&self, ui: &mut egui::Ui, width: f32) {
        let pal = crate::theme::colors(ui.ctx());
        let text = self.usage_text(false);
        let tip = format!("{}\n/compactar resume la conversación; /coste da el detalle.{}", self.usage_text(true), if self.usage.approx { "\n≈: estimado con lo que informa el programa" } else { "" });
        match self.usage.fraction() {
            Some(f) => {
                let color = if f >= 0.95 {
                    pal.error
                } else if f >= WARN_AT {
                    Color32::from_rgb(0xc0, 0x85, 0x32)
                } else {
                    ui.visuals().selection.bg_fill
                };
                let ink = if f >= WARN_AT { Color32::WHITE } else { ui.visuals().strong_text_color() };
                ui.add(egui::ProgressBar::new(f).desired_width(width).desired_height(16.0).fill(color).text(RichText::new(text).small().color(ink))).on_hover_text(tip);
            }
            None => {
                ui.add_sized([width, 16.0], egui::Label::new(RichText::new(text).small().weak()).truncate()).on_hover_text(tip);
            }
        }
    }

    fn more_menu(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(240.0);
        let running = self.running();
        let has_session = !self.session.is_empty();
        let mut ws = self.turn_workspace;
        if ui.checkbox(&mut ws, "Abrir lo nuevo en un workspace").on_hover_text("Al terminar cada tarea, lo que el agente creó o cambió se abre en ventanas, ordenadas en un workspace nuevo (/workspace)").changed() {
            self.turn_workspace = ws;
            self.save_state();
        }
        ui.separator();
        if ui.add_enabled(has_session, egui::Button::new("✂ Compactar conversación")).on_hover_text("Resume la conversación para liberar contexto (/compactar)").clicked() {
            self.command(Slash::Compact, None);
            ui.close();
        }
        let undoable = self.entries.iter().any(|e| matches!(e, Entry::Changes { turn, files } if files.iter().any(|f| !f.reverted) && self.snaps.iter().any(|(t, _)| t == turn)));
        if ui.add_enabled(undoable && !running, egui::Button::new("↶ Deshacer cambios del último turno")).on_hover_text("/deshacer").clicked() {
            self.undo_last_turn();
            ui.close();
        }
        if ui.add_enabled(!self.entries.is_empty(), egui::Button::new("⤓ Exportar a Markdown")).on_hover_text("La guarda en Descargas y la copia (/exportar)").clicked() {
            self.command(Slash::Export, Some(ui.ctx()));
            ui.close();
        }
        if ui.add_enabled(!self.entries.is_empty(), egui::Button::new("⧉ Copiar como Markdown")).clicked() {
            if let Some(md) = self.markdown("") {
                ui.ctx().copy_text(md);
            }
            ui.close();
        }
        if ui.add_enabled(has_session, egui::Button::new("⧉ Copiar la orden para seguir en un terminal")).on_hover_text("Para seguir esta conversación en un terminal abierto en la carpeta del proyecto").clicked() {
            ui.ctx().copy_text(self.kind.resume_command(&self.session));
            ui.close();
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Autocompactar al");
            let mut v = self.autocompact as f32;
            let mut on = self.autocompact > 0;
            if ui.checkbox(&mut on, "").on_hover_text("Compacta sola al pasar este % del contexto").changed() {
                self.autocompact = if on { 85 } else { 0 };
                self.save_state();
            }
            if on && ui.add(egui::Slider::new(&mut v, 50.0..=98.0).suffix(" %").integer()).changed() {
                self.autocompact = v as u8;
                self.save_state();
            }
        });
        ui.separator();
        if ui.button("? Comandos").clicked() {
            self.command(Slash::Help, None);
            ui.close();
        }
    }

    fn queue_ui(&mut self, ui: &mut egui::Ui) {
        if self.queue.is_empty() {
            return;
        }
        let pal = crate::theme::colors(ui.ctx());
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("En cola ({})", self.queue.len())).small().strong());
            if self.queue_paused {
                ui.label(RichText::new("en pausa: el último turno se canceló o no terminó bien").small().color(pal.error));
                if ui.small_button("▶ Seguir").clicked() {
                    self.queue_paused = false;
                }
            } else {
                ui.label(RichText::new("se envían al terminar el agente").small().weak());
            }
        });
        let mut edit = None;
        let mut remove = None;
        for (i, q) in self.queue.iter().enumerate() {
            egui::Frame::new()
                .stroke(egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color))
                .corner_radius(crate::theme::RADIUS)
                .inner_margin(egui::Margin::symmetric(6, 3))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("⏳").weak());
                        let extra = if q.attachments.is_empty() { String::new() } else { format!("  📎{}", q.attachments.len()) };
                        ui.add(egui::Label::new(RichText::new(format!("{}{extra}", agent::one_line(&q.text, 80))).italics()).truncate()).on_hover_text(&q.text);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add(egui::Button::new("×").frame(false).small()).on_hover_text("Quitar de la cola").clicked() {
                                remove = Some(i);
                            }
                            if ui.add(egui::Button::new("✎").frame(false).small()).on_hover_text("Editar (vuelve al cuadro de mensaje)").clicked() {
                                edit = Some(i);
                            }
                        });
                    });
                });
        }
        if let Some(i) = edit {
            self.unqueue(i);
        } else if let Some(i) = remove {
            self.queue.remove(i);
        }
        ui.add_space(2.0);
    }

    fn history_ui(&mut self, ui: &mut egui::Ui) {
        let pal = crate::theme::colors(ui.ctx());
        ui.horizontal(|ui| {
            if ui.button("← Volver").clicked() {
                self.view = View::Chat;
            }
            ui.add(egui::TextEdit::singleline(&mut self.history_query).hint_text("🔍 Buscar en títulos y mensajes…").desired_width(f32::INFINITY));
        });
        if self.store.is_none() {
            ui.label(RichText::new("Abre un proyecto para ver su historial.").weak());
            return;
        }
        let shown: Vec<Summary> = self.history.iter().filter(|s| s.matches(&self.history_query)).cloned().collect();
        ui.label(RichText::new(format!("{} conversaciones · clic para abrirla y seguirla", shown.len())).weak().small());
        let now = history::now();
        let mut open = None;
        let mut rename = None;
        let mut delete = None;
        let mut export = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("agent_history_list").show(ui, |ui| {
            if shown.is_empty() {
                ui.label(RichText::new(if self.history.is_empty() { "Todavía no hay conversaciones guardadas." } else { "Nada coincide." }).weak());
            }
            for s in &shown {
                let current = s.id == self.conv_id;
                let fill = if current { ui.visuals().selection.bg_fill.gamma_multiply(0.25) } else { ui.visuals().faint_bg_color };
                let r = egui::Frame::new()
                    .fill(fill)
                    .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                    .corner_radius(crate::theme::RADIUS)
                    .inner_margin(6)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        if let Some((id, text)) = &mut self.renaming {
                            if *id == s.id {
                                let r = ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY));
                                r.request_focus();
                                if r.lost_focus() {
                                    if ui.input(|i| i.key_pressed(Key::Enter)) {
                                        rename = Some((id.clone(), text.clone()));
                                    } else {
                                        rename = Some((String::new(), String::new()));
                                    }
                                }
                                return;
                            }
                        }
                        ui.horizontal(|ui| {
                            ui.add(egui::Label::new(RichText::new(&s.title).strong()).truncate());
                            if current {
                                ui.label(RichText::new("abierta").small().color(pal.info));
                            }
                        });
                        let agent = Kind::from_id(&s.agent).map(|k| k.name()).unwrap_or("?");
                        let cost = if s.cost > 0.0 { format!(" · {}", history::money(s.cost)) } else { String::new() };
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("{} · {agent} · {} mensajes{cost}", history::when(s.updated, now), s.messages)).weak().small());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let confirm = self.confirm_delete.as_deref() == Some(s.id.as_str());
                                let del = if confirm { RichText::new("¿Borrar?").color(pal.error).small() } else { RichText::new("🗑").small() };
                                if ui.add(egui::Button::new(del).frame(false)).on_hover_text("Borrar del historial").clicked() {
                                    delete = Some((s.id.clone(), confirm));
                                }
                                if ui.add(egui::Button::new(RichText::new("⤓").small()).frame(false)).on_hover_text("Exportar a Markdown (Descargas) y copiar").clicked() {
                                    export = Some(s.id.clone());
                                }
                                if ui.add(egui::Button::new(RichText::new("✎").small()).frame(false)).on_hover_text("Renombrar").clicked() {
                                    rename = Some((format!("\u{0}{}", s.id), s.title.clone()));
                                }
                            });
                        });
                    })
                    .response;
                let click = ui.interact(r.rect, ui.scope_id().with(("agent_hist_row", &s.id)), egui::Sense::click());
                if click.clicked() && self.renaming.is_none() {
                    open = Some(s.id.clone());
                }
                ui.add_space(4.0);
            }
        });
        if let Some((id, title)) = rename {
            if let Some(id) = id.strip_prefix('\u{0}') {
                self.renaming = Some((id.to_string(), title));
            } else {
                if !id.is_empty() {
                    self.rename(&id, &title);
                }
                self.renaming = None;
            }
        } else if let Some((id, confirmed)) = delete {
            if confirmed {
                self.delete_conversation(&id);
                self.confirm_delete = None;
            } else {
                self.confirm_delete = Some(id);
            }
        } else if let Some(id) = export {
            if let Some(path) = self.export(&id, None) {
                if let Some(md) = self.markdown(&id) {
                    ui.ctx().copy_text(md);
                }
                self.status = format!("Exportada a {}", path.display());
            }
        } else if let Some(id) = open {
            if self.running() {
                self.status = "Espera a que termine el agente para abrir otra conversación".into();
            } else {
                self.open_conversation(&id);
            }
        }
    }

    /// Floating windows: the diff, the disk browser and the text snippet.
    fn windows(&mut self, ctx: &egui::Context) {
        // Diff.
        let mut close = false;
        let mut revert = None;
        if let Some(d) = &self.diff {
            let pal = crate::theme::colors(ctx);
            let can = self.snaps.iter().any(|(t, _)| *t == d.turn);
            let mut open = true;
            egui::Window::new(format!("Cambios: {}", d.rel))
                .id(egui::Id::unique("agent_diff"))
                .open(&mut open)
                .default_size([640.0, 480.0])
                .pivot(egui::Align2::CENTER_CENTER)
                .default_pos(ctx.content_rect().center())
                .resizable(true)
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        let add = d.lines.iter().filter(|l| l.op == snapshot::Op::Add).count();
                        let del = d.lines.iter().filter(|l| l.op == snapshot::Op::Del).count();
                        ui.label(RichText::new(format!("+{add}")).color(crate::theme::GREEN).strong());
                        ui.label(RichText::new(format!("−{del}")).color(crate::theme::RED).strong());
                        ui.label(RichText::new("antes del turno → ahora").weak().small());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add_enabled(can, egui::Button::new("↶ Revertir")).on_hover_text("Devuelve el fichero a como estaba antes del turno").clicked() {
                                revert = Some((d.turn, d.rel.clone()));
                            }
                        });
                    });
                    if let Some(n) = &d.note {
                        ui.label(RichText::new(n).weak());
                    }
                    ui.separator();
                    egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                        let font = egui::TextStyle::Monospace.resolve(ui.style());
                        for l in &d.lines {
                            let (bg, sign, color) = if l.header {
                                (Color32::TRANSPARENT, "", pal.info)
                            } else {
                                match l.op {
                                    snapshot::Op::Add => (crate::theme::GREEN.gamma_multiply(0.15), "+", ui.visuals().text_color()),
                                    snapshot::Op::Del => (crate::theme::RED.gamma_multiply(0.15), "-", ui.visuals().text_color()),
                                    snapshot::Op::Same => (Color32::TRANSPARENT, " ", ui.visuals().weak_text_color()),
                                }
                            };
                            let num = |n: Option<usize>| n.map(|n| format!("{n:>4}")).unwrap_or_else(|| "    ".into());
                            let text = if l.header { l.text.clone() } else { format!("{} {} {sign} {}", num(l.old_no), num(l.new_no), l.text) };
                            egui::Frame::new().fill(bg).show(ui, |ui| {
                                ui.add(egui::Label::new(RichText::new(text).font(font.clone()).color(color)).extend());
                            });
                        }
                    });
                });
            close = !open;
        }
        if let Some((turn, rel)) = revert {
            match self.revert_file(turn, &rel) {
                Ok(()) => self.open_diff(turn, &rel),
                Err(e) => self.notice(e, Tone::Error),
            }
        }
        if close {
            self.diff = None;
        }
        // Disk browser.
        let mut favorites = std::mem::take(&mut self.cfg.favorites);
        let before = favorites.clone();
        let recent = self.cfg.recent_dirs.clone();
        if let crate::agent_browser::Picked::Attach(paths) = self.browser.show(ctx, &mut favorites, &recent) {
            for p in paths {
                self.attach_path(&p);
            }
        }
        let fav_changed = favorites != before;
        self.cfg.favorites = favorites;
        if fav_changed {
            self.save_cfg();
        }
        // Text snippet.
        let mut done = None;
        if let Some((title, text)) = &mut self.text_modal {
            let mut open = true;
            egui::Window::new("Adjuntar texto").id(egui::Id::unique("agent_text_modal")).open(&mut open).collapsible(false).default_size([440.0, 300.0]).pivot(egui::Align2::CENTER_CENTER).default_pos(ctx.content_rect().center()).show(ctx, |ui| {
                ui.add(egui::TextEdit::singleline(title).hint_text("Título (p. ej. «Notas de diseño»)").desired_width(f32::INFINITY));
                egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                    ui.add(egui::TextEdit::multiline(text).hint_text("Pega o escribe aquí el texto: va dentro del mensaje").desired_rows(10).desired_width(f32::INFINITY).code_editor());
                });
                ui.horizontal(|ui| {
                    if ui.add_enabled(!text.trim().is_empty(), crate::theme::primary_button(ui, "Adjuntar")).clicked() {
                        done = Some(true);
                    }
                    if ui.button("Cancelar").clicked() {
                        done = Some(false);
                    }
                });
            });
            if !open {
                done = Some(false);
            }
        }
        if let Some(ok) = done {
            if let Some((title, text)) = self.text_modal.take() {
                if ok {
                    let title = if title.trim().is_empty() { "Texto".to_string() } else { title.trim().to_string() };
                    self.attach(Attachment::text("📝", title, text));
                }
            }
        }
    }

    /// Assets dragged from the browser and files dropped from the file manager onto the dock
    /// are attached.
    fn drop_zone(&mut self, ui: &egui::Ui, rect: egui::Rect, src: &Sources) {
        let ctx = ui.ctx().clone();
        let over = ctx.pointer_latest_pos().is_some_and(|p| rect.contains(p));
        if !over {
            return;
        }
        let asset = DragAndDrop::payload::<Open>(&ctx);
        let files = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if asset.is_some() || files {
            let c = ui.visuals().selection.stroke.color;
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("agent_drop_hint")));
            painter.rect_filled(rect.shrink(2.0), 6.0, c.gamma_multiply(0.08));
            painter.rect_stroke(rect.shrink(3.0), 6.0, egui::Stroke::new(2.0, c), egui::StrokeKind::Inside);
            let what = asset.as_deref().map(attach::asset_label).unwrap_or_else(|| "los ficheros".into());
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, format!("Suelta para adjuntar {what}"), egui::FontId::proportional(14.0), c);
        }
        if asset.is_some() && ctx.input(|i| i.pointer.any_released()) {
            if let Some(o) = DragAndDrop::take_payload::<Open>(&ctx) {
                if let Some(a) = src.asset(&o) {
                    self.attach(a);
                }
            }
        }
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        for p in dropped {
            self.attach_path(&p);
        }
    }

    fn attach_menu(&mut self, ui: &mut egui::Ui, src: &Sources) {
        ui.set_min_width(300.0);
        let add = |ui: &mut egui::Ui, title: &str, item: Option<(String, Attachment)>, missing: &str| -> Option<Attachment> {
            match item {
                Some((what, a)) => ui.button(format!("{title}: {what}")).on_hover_text(attachment_tip(&a)).clicked().then_some(a),
                None => {
                    ui.add_enabled(false, egui::Button::new(title)).on_disabled_hover_text(missing);
                    None
                }
            }
        };
        let asset = |o: &Option<Open>| o.as_ref().and_then(|o| src.asset(o).map(|a| (attach::asset_label(o), a)));
        let mut chosen = vec![];
        chosen.extend(add(ui, "Asset seleccionado", asset(&src.selected), "Selecciona un asset en el navegador"));
        chosen.extend(add(ui, "Ventana enfocada", asset(&src.focused), "No hay ninguna ventana de editor enfocada"));
        chosen.extend(add(
            ui,
            "Selección de la room",
            src.room_selection().map(|a| (a.label.trim_start_matches("Selección en ").to_string(), a)),
            "Selecciona una instancia con ⬚ Mover en la room enfocada",
        ));
        chosen.extend(add(ui, "Salida", src.output().map(|a| (format!("últimas {} líneas", attach::OUTPUT_LINES), a)), "La Salida está vacía"));
        ui.separator();
        // the agent gets assets from OpenGameArt by itself (`modengine store`)
        let can_store = self.permission != Permission::Plan;
        let r = ui.add_enabled(can_store, egui::Button::new("🛒 Buscar en la Tienda…"));
        if r.on_hover_text("Escribe una instrucción para que el agente busque un asset en OpenGameArt, lo mire y lo importe con sus créditos").on_disabled_hover_text("En «Solo planificar» el agente solo puede buscar, no importar").clicked() {
            self.input = store_instruction(&self.input);
            ui.close();
        }
        ui.separator();
        if let Some(p) = src.project {
            ui.menu_button("📄 Fichero del proyecto", |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.attach_filter).hint_text("Filtrar…").desired_width(240.0));
                let files = self.project_files(&p.dir);
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    let mut shown = 0;
                    for f in files.iter().filter(|f| attach::matches(f, &self.attach_filter)) {
                        shown += 1;
                        if shown > 300 {
                            ui.label(RichText::new("… escribe para filtrar").weak());
                            break;
                        }
                        if ui.button(f).clicked() {
                            chosen.push(Attachment::file(&p.dir.join(f)));
                        }
                    }
                });
            });
            ui.menu_button("🖼 Imagen de un sprite", |ui| {
                let pngs: Vec<_> = p.sprites.iter().filter(|s| s.path.extension().is_some_and(|e| e.eq_ignore_ascii_case("png"))).collect();
                if pngs.is_empty() {
                    ui.label(RichText::new("No hay sprites PNG").weak());
                }
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for s in pngs {
                        if ui.button(format!("▣ {}", s.name)).clicked() {
                            chosen.extend(attach::asset(p, &Open::Sprite(s.name.clone())));
                        }
                    }
                });
            });
        }
        ui.separator();
        if ui.button("💽 Desde el disco…").on_hover_text("Cualquier fichero, carpeta o imagen del ordenador (varios a la vez)").clicked() {
            let start = self.cfg.recent_dirs.first().cloned();
            self.browser.open_at(start.as_deref());
            ui.close();
        }
        if ui.button("📋 Pegar imagen (Ctrl+V)").on_hover_text("La imagen que tengas copiada: una captura de pantalla, un recorte…").clicked() {
            self.paste_clipboard_image();
            ui.close();
        }
        if ui.button("📸 Captura del editor").on_hover_text("Lo que ves ahora en la ventana del editor").clicked() {
            self.capture_editor(ui.ctx());
            ui.close();
        }
        if ui.button("📝 Texto…").on_hover_text("Un texto que escribes o pegas: va dentro del mensaje").clicked() {
            self.text_modal = Some((String::new(), String::new()));
            ui.close();
        }
        if !self.cfg.recent_paths.is_empty() || !self.cfg.recent_urls.is_empty() {
            ui.menu_button("🕘 Recientes de fuera del proyecto", |ui| {
                for p in self.cfg.recent_paths.clone() {
                    let icon = if p.is_dir() { "📁" } else { "📄" };
                    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.display().to_string());
                    let r = ui.add_enabled(p.exists(), egui::Button::new(format!("{icon} {name}"))).on_hover_text(p.display().to_string());
                    if r.clicked() {
                        chosen.push(Attachment::file(&p));
                    }
                }
                for u in self.cfg.recent_urls.clone() {
                    if ui.button(format!("🔗 {}", agent::one_line(&u, 50))).on_hover_text(&u).clicked() {
                        chosen.push(Attachment::url(&u));
                    }
                }
            });
        }
        ui.label("Ruta o URL:");
        let mut go = false;
        ui.horizontal(|ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut self.attach_path).hint_text("~/notas.md, /mnt/…, file://… o https://…").desired_width(220.0));
            go = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            go |= ui.add_enabled(!self.attach_path.trim().is_empty(), egui::Button::new("Adjuntar")).clicked();
        });
        if go && !self.attach_path.trim().is_empty() {
            let typed = self.attach_path.trim().to_string();
            let ok = if typed.starts_with("http://") || typed.starts_with("https://") {
                self.attach_url(&typed)
            } else if typed.starts_with("file://") || typed.lines().count() > 1 {
                self.attach_pasted(&typed) || {
                    self.attach_error = Some(format!("No existe: {typed}"));
                    false
                }
            } else {
                let path = attach::resolve_path(src.project.map(|p| p.dir.as_path()), &typed);
                self.attach_path(&path)
            };
            if ok {
                self.attach_path.clear();
                ui.close();
            }
        }
        if let Some(e) = &self.attach_error {
            ui.label(RichText::new(e).color(crate::theme::colors(ui.ctx()).error));
        }
        ui.label(
            RichText::new(
                "De fuera del proyecto el agente solo puede leer lo adjuntado (las carpetas, todo lo de dentro). Las imágenes llegan a Codex con --image; \
                 Claude Code y Antigravity reciben su ruta. Las URLs las descarga el agente. También puedes arrastrar, pegar rutas en el mensaje o escribir @.",
            )
            .weak()
            .small(),
        );
        if !chosen.is_empty() {
            for a in chosen {
                self.attach(a);
            }
            ui.close();
        }
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

    // ------------------------------------------------------------ screenshots

    /// Sample state for `modeditor --agent-demo <what>` screenshots: `history` (the history list
    /// over a sample conversation), `queue` (a running agent with queued messages and the
    /// context meter), `diff` (a turn's changes and the diff window) or `browser` (the disk
    /// browser). Nothing is written to the real history.
    pub fn demo(&mut self, what: &str, root: &Path, name: &str) {
        self.demo = true;
        self.enter_project(root, name);
        let dir = std::env::temp_dir().join(format!("modeditor-agent-demo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store { dir };
        let now = history::now();
        let titles = [
            ("Haz que el jugador salte más alto", "claude", 3, 0.42, 600),
            ("Añade un enemigo que patrulle entre dos paredes", "claude", 7, 1.37, 7_000),
            ("¿Por qué no suena la moneda al recogerla?", "codex", 2, 0.0, 90_000),
            ("Room del nivel 2 con plataformas móviles", "agy", 5, 0.0, 200_000),
            ("Revisa el sprite del jugador (adjunto referencia)", "claude", 4, 0.88, 3_000_000),
        ];
        for (i, (t, a, msgs, cost, ago)) in titles.iter().enumerate() {
            let mut c = Conversation { id: format!("demo{i}"), title: t.to_string(), agent: a.to_string(), created: now - ago - 60, updated: now - ago, ..Default::default() };
            for m in 0..*msgs {
                c.entries.push(Entry::User(format!("{t} ({m})")));
            }
            c.usage.session_cost = *cost;
            let _ = store.save(&c);
        }
        self.store = Some(store);
        self.clear_conversation();
        self.kind = Kind::Claude;
        self.locate();
        self.conv_id = "demo0".into();
        self.title = titles[0].0.into();
        self.session = "4f9c2d1e-demo".into();
        let player = root.join("objects/player.ms");
        self.entries = vec![
            Entry::User("Haz que el jugador salte más alto y que el doble salto sea opcional".into()),
            Entry::Attached(vec![Attachment::file(&player), Attachment::url("https://manual.gamemaker.io/monthly/en/GameMaker_Language/GML_Reference/Movement_And_Collisions/Movement/motion_add.htm")]),
            Entry::Notice("Modelo: claude-opus".into(), Tone::Muted),
            Entry::Tool(ToolLine { icon: "👁", verb: "Leer".into(), detail: "objects/player.ms".into() }),
            Entry::Tool(ToolLine { icon: "✎", verb: "Editar".into(), detail: "objects/player.ms".into() }),
            Entry::Text(
                "## Hecho\nHe subido **jump_speed** de `9` a `12` y añadido la variable `double_jump`:\n\n- `true`: puede saltar otra vez en el aire\n- `false`: *solo un salto*\n\n```ms\nif keyboard_check_pressed(vk_space) and (on_ground or (double_jump and jumps < 2)) {\n    vspeed = -jump_speed\n    jumps += 1\n}\n```\nPruébalo en `level1` con Espacio. Más en [la guía](https://example.com/guia)."
                    .into(),
            ),
            Entry::Changes {
                turn: 1,
                files: vec![
                    snapshot::FileChange { rel: "objects/player.ms".into(), kind: ChangeKind::Modified, added: 6, removed: 2, binary: false, reverted: false },
                    snapshot::FileChange { rel: "rooms/level1.toml".into(), kind: ChangeKind::Modified, added: 1, removed: 1, binary: false, reverted: false },
                ],
            },
            Entry::Divider("Conversación compactada".into()),
            Entry::User("Ahora que el enemigo rebote al pisarlo".into()),
        ];
        self.turns = vec![Turn { id: 1, entry: 0, prompt: "Haz que el jugador salte más alto".into(), ..Default::default() }];
        self.usage = Usage { context: 168_400, window: 200_000, session_cost: 0.42, output: 5_200, ..Default::default() };
        self.status = "Hecho en 18,4 s".into();
        match what {
            "history" => self.show_history(""),
            "paste" => {
                // A pasted screenshot, as Ctrl+V leaves it (MODENGINE_DEMO_PASTE: a PNG to use).
                let png = std::env::var_os("MODENGINE_DEMO_PASTE").map(PathBuf::from).unwrap_or_else(|| root.join("screenshot.png"));
                let img = image::open(&png).ok().map(|i| i.to_rgba8());
                let img = img.map(|i| egui::ColorImage::from_rgba_unmultiplied([i.width() as usize, i.height() as usize], i.as_raw()));
                self.paste_dir = Some(std::env::temp_dir().join("modeditor_agent_demo_paste"));
                self.attach_image(&img.unwrap_or_else(|| egui::ColorImage::filled([480, 270], Color32::from_rgb(70, 120, 200))), "captura");
                self.input = "Mira la captura: el slime atraviesa el suelo al caer".into();
            }
            "queue" => {
                self.entries.push(Entry::Tool(ToolLine { icon: "✎", verb: "Editar".into(), detail: "objects/enemy.ms".into() }));
                {
                    // A harmless long-lived child stands in for the agent.
                    let mut cmd = if cfg!(windows) { crate::proc::command("ping") } else { crate::proc::command("sleep") };
                    if cfg!(windows) {
                        cmd.args(["-n", "120", "127.0.0.1"]);
                    } else {
                        cmd.arg("120");
                    }
                    let child = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
                    if let Ok(child) = child {
                        let (_tx, rx) = mpsc::channel();
                        std::mem::forget(_tx);
                        self.run = Some(Running {
                            child,
                            rx,
                            parser: Parser::new(Kind::Claude, root),
                            kind: Kind::Claude,
                            started: Instant::now(),
                            stop_at: None,
                            exited_at: None,
                            stdout_done: false,
                            got_result: false,
                            failed: false,
                            compacted: false,
                            stderr: String::new(),
                            changed: BTreeSet::new(),
                            blocked: 0,
                            context_file: PathBuf::new(),
                            root: root.to_path_buf(),
                            purpose: Purpose::Chat(2),
                            before: None,
                            first_entry: 0,
                        });
                    }
                }
                self.status = "Editar objects/enemy.ms…".into();
                self.queue = vec![
                    Queued { text: "Después, añade un sonido al rebotar".into(), attachments: vec![] },
                    Queued { text: "Y comprueba el proyecto con modengine check".into(), attachments: vec![Attachment::text("📝", "Notas", "rebote = 0.6")] },
                ];
                self.input = "/comp".into();
            }
            "diff" => {
                let before = "object Player {\n    jump_speed = 9\n    on step {\n        if keyboard_check_pressed(vk_space) and on_ground {\n            vspeed = -jump_speed\n        }\n    }\n}\n";
                let after = "object Player {\n    jump_speed = 12\n    double_jump = true\n    jumps = 0\n    on step {\n        if on_ground { jumps = 0 }\n        if keyboard_check_pressed(vk_space) and (on_ground or (double_jump and jumps < 2)) {\n            vspeed = -jump_speed\n            jumps += 1\n        }\n    }\n}\n";
                self.diff = Some(DiffView { turn: 1, rel: "objects/player.ms".into(), lines: snapshot::unified(before, after, 3), note: None });
            }
            "browser" => {
                let home = modengine::home_dir();
                self.browser.open_at(home.as_deref());
                let first: Vec<PathBuf> = self.browser.rows().iter().filter(|r| r.is_dir).take(2).map(|r| r.path.clone()).collect();
                self.browser.selected = first.into_iter().collect();
                self.attachments = vec![Attachment::file(&std::env::temp_dir()), Attachment::url("https://lospec.com/palette-list/pear36")];
            }
            "store" => {
                // the agent got an asset from the store (what a real Claude Code turn did)
                let tool = "modengine";
                self.title = "Moneda de la Tienda".into();
                let line = |icon: &'static str, verb: &str, detail: &str| Entry::Tool(ToolLine { icon, verb: verb.into(), detail: detail.into() });
                self.entries = vec![
                    Entry::User("busca en la Tienda un sprite de moneda o gema CC0 y añádelo al proyecto con sus créditos".into()),
                    line(">_", "Ejecutar", &format!("{tool} store search coin --tipo 2d --licencia cc0 --json")),
                    line(">_", "Ejecutar", &format!("{tool} store show simple-rotating-coin --json")),
                    Entry::Text("`simple-rotating-coin` es CC0 y una tira limpia de 12 fotogramas. La descargo para verla antes de importarla.".into()),
                    line(">_", "Ejecutar", &format!("{tool} store download simple-rotating-coin --json")),
                    line("👁", "Leer", "~/.cache/modengine/store/files/…/coin.png"),
                    line(">_", "Ejecutar", &format!("{tool} store get simple-rotating-coin . --file coin.png --name coin_gold --frames 12 --origin center --fps 12 --json")),
                    Entry::Text("He añadido el sprite **coin_gold**: una moneda giratoria de 12 fotogramas de 16×16, origen en el centro.\n\n- **Asset:** «Simple rotating coin», de sacio\n- **Licencia:** CC0 (anotada en `CREDITS.md`)\n- **URL:** https://opengameart.org/content/simple-rotating-coin".into()),
                    Entry::Changes {
                        turn: 1,
                        files: vec![
                            snapshot::FileChange { rel: "CREDITS.md".into(), kind: ChangeKind::Added, added: 12, removed: 0, binary: false, reverted: false },
                            snapshot::FileChange { rel: "sprites/coin_gold.png".into(), kind: ChangeKind::Added, added: 0, removed: 0, binary: true, reverted: false },
                            snapshot::FileChange { rel: "sprites/coin_gold.toml".into(), kind: ChangeKind::Added, added: 6, removed: 0, binary: false, reverted: false },
                        ],
                    },
                ];
                self.permission = Permission::Edit;
                self.imported = vec!["sprite 'coin_gold'".into()];
                self.downloaded = vec!["«Simple rotating coin» (CC0) → sprite 'coin_gold'".into()];
                self.input.clear();
            }
            "import" => {
                // an art folder attached (`$MODENGINE_DEMO_ART`, or a sample one) and an import turn done
                let art = std::env::var_os("MODENGINE_DEMO_ART").map(PathBuf::from).unwrap_or_else(|| {
                    let d = std::env::temp_dir().join("Arte del pack");
                    let _ = std::fs::create_dir_all(&d);
                    let strip = image::RgbaImage::from_fn(96, 24, |x, y| if x % 24 > 4 && x % 24 < 20 && y > 2 { image::Rgba([90, 120, 230, 255]) } else { image::Rgba([0, 0, 0, 0]) });
                    let _ = strip.save(d.join("knight run.png"));
                    d
                });
                let att = Attachment::file(&art);
                let prompt = import_instruction(Some(root), std::slice::from_ref(&att)).unwrap_or_default();
                self.title = "Importar el arte del pack".into();
                self.entries = vec![
                    Entry::User(prompt.clone()),
                    Entry::Attached(vec![att.clone()]),
                    Entry::Tool(ToolLine { icon: "👁", verb: "Leer".into(), detail: format!("{}/knight run.png", art.display()) }),
                    Entry::Tool(ToolLine { icon: "▶", verb: "Ejecutar".into(), detail: "modengine inspect-image \"…/Arte del pack\" --json".into() }),
                    Entry::Tool(ToolLine { icon: "▶", verb: "Ejecutar".into(), detail: "modengine import . \"…/knight run.png\" --name knight_run --frames 4 --origin bottom-center".into() }),
                    Entry::Text("Importado **knight_run**: 4 frames de 24×24, origen abajo al centro.".into()),
                    Entry::Changes {
                        turn: 1,
                        files: vec![
                            snapshot::FileChange { rel: "sprites/knight_run.png".into(), kind: ChangeKind::Added, added: 0, removed: 0, binary: true, reverted: false },
                            snapshot::FileChange { rel: "sprites/knight_run.toml".into(), kind: ChangeKind::Added, added: 4, removed: 0, binary: false, reverted: false },
                        ],
                    },
                ];
                self.permission = Permission::Edit;
                self.attachments = vec![att];
                self.input.clear();
            }
            _ => {}
        }
    }
}

/// What a click in the transcript asks for (applied after drawing it).
enum Action {
    Edit,
    Retry,
    Fold(usize),
    Diff(u64, String),
    Revert(u64, String),
    RevertAll(u64),
}

/// The files a turn changed: +/- counts, click for the diff, Revertir per file and for all.
fn changes_ui(ui: &mut egui::Ui, turn: u64, files: &[snapshot::FileChange], can_revert: bool, action: &mut Option<Action>) {
    ui.add_space(4.0);
    egui::Frame::new()
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(crate::theme::RADIUS)
        .inner_margin(6)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let pending = files.iter().filter(|f| !f.reverted).count();
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Archivos cambiados ({})", files.len())).strong().small());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let tip = if can_revert { "Devuelve todos a como estaban antes del turno (/deshacer)" } else { "Solo se pueden revertir los turnos de esta sesión del editor" };
                    if ui.add_enabled(can_revert && pending > 0, egui::Button::new(RichText::new("↶ Revertir todo").small())).on_hover_text(tip).on_disabled_hover_text(tip).clicked() {
                        *action = Some(Action::RevertAll(turn));
                    }
                });
            });
            for f in files {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let (icon, tip) = match f.kind {
                        ChangeKind::Added => ("＋", "Nuevo"),
                        ChangeKind::Modified => ("✎", "Modificado"),
                        ChangeKind::Deleted => ("🗑", "Borrado"),
                    };
                    ui.label(RichText::new(icon).weak()).on_hover_text(tip);
                    let name = if f.reverted { RichText::new(&f.rel).monospace().strikethrough().weak() } else { RichText::new(&f.rel).monospace() };
                    if ui.add(egui::Button::new(name).frame(false)).on_hover_text("Ver los cambios").clicked() {
                        *action = Some(Action::Diff(turn, f.rel.clone()));
                    }
                    if f.binary {
                        ui.label(RichText::new("binario").weak().small());
                    } else {
                        ui.label(RichText::new(format!("+{}", f.added)).color(crate::theme::GREEN).small());
                        ui.label(RichText::new(format!("−{}", f.removed)).color(crate::theme::RED).small());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if f.reverted {
                            ui.label(RichText::new("revertido").weak().small());
                        } else if ui.add_enabled(can_revert, egui::Button::new(RichText::new("Revertir").small())).clicked() {
                            *action = Some(Action::Revert(turn, f.rel.clone()));
                        }
                    });
                });
            }
        });
}

/// A small button drawn over the top-right corner of `rect` without taking layout space
/// (shown while the pointer is over `rect`). True when clicked.
fn overlay_button(ui: &mut egui::Ui, rect: egui::Rect, id: impl std::hash::Hash + std::fmt::Debug, icon: &str, tip: &str) -> bool {
    let at = egui::Rect::from_min_size(egui::pos2(rect.right() - 24.0, rect.top() + 2.0), egui::vec2(22.0, 20.0));
    let r = ui.interact(at, ui.scope_id().with(id), egui::Sense::click()).on_hover_text(tip);
    let v = ui.visuals();
    let (fill, stroke) = if r.hovered() { (v.widgets.hovered.bg_fill, v.widgets.hovered.bg_stroke) } else { (v.window_fill, v.widgets.noninteractive.bg_stroke) };
    let p = ui.painter();
    p.rect(at, crate::theme::RADIUS, fill, stroke, egui::StrokeKind::Inside);
    p.text(at.center(), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(12.0), v.text_color());
    r.clicked()
}

/// What a chip shows on hover: the path, or the start of the text.
fn attachment_tip(a: &Attachment) -> String {
    let mut t = match a.kind {
        AttachKind::Text => agent::clip(&a.text, 600),
        AttachKind::Url => format!("{}\nEl agente la descarga y la lee", a.text),
        _ => a.path.display().to_string(),
    };
    if a.kind == AttachKind::Image {
        t += "\nImagen: Codex la recibe con --image; Claude Code y Antigravity, su ruta";
    }
    t
}

/// An image attachment with its thumbnail (bigger preview on hover) and a ×. Draws nothing and
/// returns false when there is no thumbnail.
fn image_chip(ui: &mut egui::Ui, a: &Attachment, thumb: Option<&egui::TextureHandle>) -> bool {
    let Some(t) = thumb else { return false };
    let mut removed = false;
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(crate::theme::RADIUS)
        .inner_margin(egui::Margin::symmetric(4, 2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let size = t.size_vec2();
                let small = size * (36.0 / size.y.max(1.0)).min(64.0 / size.x.max(1.0));
                ui.add(egui::Image::new(t).fit_to_exact_size(small).corner_radius(3.0)).on_hover_ui(|ui| {
                    ui.add(egui::Image::new(t).max_size(egui::vec2(320.0, 320.0)));
                    ui.label(RichText::new(attachment_tip(a)).small().weak());
                });
                ui.label(&a.label);
                if ui.add(egui::Button::new("×").frame(false).small()).on_hover_text("Quitar").clicked() {
                    removed = true;
                }
            });
        });
    removed
}

/// An attachment as a small rounded chip; with `removable`, a × that returns true when clicked.
fn chip(ui: &mut egui::Ui, a: &Attachment, removable: bool) -> bool {
    let mut removed = false;
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(crate::theme::RADIUS)
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label(RichText::new(a.icon).weak());
                ui.label(&a.label).on_hover_text(attachment_tip(a));
                if removable && ui.add(egui::Button::new("×").frame(false).small()).on_hover_text("Quitar").clicked() {
                    removed = true;
                }
            });
        });
    removed
}

/// Puts the text cursor of the message box after its last character.
fn move_cursor_to_end(ctx: &egui::Context, id: egui::Id, text: &str) {
    if let Some(mut st) = egui::TextEdit::load_state(ctx, id) {
        let end = egui::text::CCursor::new(text.chars().count());
        st.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
        st.store(ctx, id);
    }
}

impl Drop for AgentPanel {
    fn drop(&mut self) {
        self.save_conversation();
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
    use std::process::Command;

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

    /// An empty project folder next to the fake CLI (which writes its files in its own folder).
    fn empty_project(dir: &Path) -> PathBuf {
        let p = dir.join("project");
        std::fs::create_dir_all(&p).unwrap();
        p
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
                Entry::Changes {
                    turn: 1,
                    files: vec![snapshot::FileChange { rel: "objects/world.ms".into(), kind: ChangeKind::Modified, added: 1, removed: 0, binary: false, reverted: false }]
                },
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
        p.send("hola", &empty_project(&dir), "x");
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
        p.send("hola", &empty_project(&dir), "x");
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
        assert_eq!(p.entries.last(), Some(&Entry::Notice("Cancelado.".into(), Tone::Muted)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancel_pauses_the_queue_and_keeps_changes_revertible() {
        // The fake agent writes a file in the project, then hangs until it is cancelled.
        let tmp = std::env::temp_dir().join(format!("modeditor_agent_cancel_{}", std::process::id()));
        let body = format!("echo '{{\"type\":\"system\",\"session_id\":\"S3\"}}'\necho hecho > '{}/project/nuevo.ms'\nsleep 30", tmp.display());
        let (dir, exe) = fake_cli("cancel", &body);
        let project = empty_project(&dir);
        let mut p = panel(&exe);
        p.send("hola", &project, "x");
        p.enqueue("después");
        let ctx = egui::Context::default();
        let start = Instant::now();
        while !project.join("nuevo.ms").exists() && start.elapsed() < Duration::from_secs(5) {
            p.poll(&ctx);
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(p.command(Slash::Cancel, None), None);
        wait(&mut p);
        assert!(start.elapsed() < Duration::from_secs(10));
        assert!(p.entries.contains(&Entry::Notice("Cancelado.".into(), Tone::Muted)));
        // The queued message waits for ▶ Seguir instead of going out on its own.
        assert!(p.queue_paused && p.next_to_send().is_none() && p.queue.len() == 1);
        // What the agent did before the cancel can be undone.
        assert_eq!(p.undo_last_turn(), 1);
        assert!(!project.join("nuevo.ms").exists());
        // Cancelling with nothing running just says so.
        p.command(Slash::Cancel, None);
        assert_eq!(p.entries.last(), Some(&Entry::Notice("El agente no está trabajando.".into(), Tone::Muted)));
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
        let project = empty_project(&dir);
        p.send("hola", &project, "x");
        wait(&mut p);
        let args = lines(&dir.join("args.txt"));
        assert_eq!(args[..7], ["exec", "--json", "--skip-git-repo-check", "--sandbox", "workspace-write", "-c", "web_search=\"live\""], "Internet is on by default");
        assert_eq!(args.last().map(String::as_str), Some("-"));
        assert!(std::fs::read_to_string(dir.join("stdin.txt")).unwrap().starts_with("<modengine-editor-context>"));
        assert_eq!(p.entries.last(), Some(&Entry::Text("Vale".into())));
        assert_eq!(p.session, "T1");

        // Switching agent starts a new conversation.
        p.set_kind(Kind::Antigravity);
        assert!(p.session.is_empty());
        p.send("hola", &project, "x");
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
        // Attachments from outside the project: a note (inline) and a picture (read by path, or --image).
        let outside = std::env::temp_dir().join(format!("modeditor_smoke_att_{}", std::process::id()));
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("note.txt"), "The codeword is PELICAN-42.\n").unwrap();
        // A plain red square.
        image::RgbaImage::from_pixel(32, 32, image::Rgba([220, 20, 20, 255])).save(outside.join("picture.png")).unwrap();
        p.attach_path(&outside.join("picture.png"));
        p.attach_path(&outside.join("note.txt"));
        p.send(
            "List the object names defined in this project, one per line. Then say the codeword from the attached note, \
             and describe the attached picture in a few words. Do not edit anything.",
            &project,
            "Coin Climber",
        );
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
        assert!(p.entries.iter().any(|e| matches!(e, Entry::Text(t) if t.contains("PELICAN-42"))), "the agent did not see the attached note");
        // Compacting: the CLI's own /compact (Claude Code, Antigravity) or a summary (Codex).
        assert!(p.compact(), "compaction did not start");
        let start = Instant::now();
        while p.running() && start.elapsed() < Duration::from_secs(240) {
            p.poll(&ctx);
            std::thread::sleep(Duration::from_millis(50));
        }
        println!("after compact: session={} status={} usage={:?}", p.session, p.status, p.usage);
        assert!(p.entries.iter().any(|e| matches!(e, Entry::Divider(_))), "{:?}", p.entries.last());
        let _ = std::fs::remove_dir_all(&project);
        let _ = std::fs::remove_dir_all(&outside);
    }

    /// One real turn in "Editar archivos" importing a folder of art from outside a copy of
    /// examples/platformer: `MODENGINE_AGENT_SMOKE=claude cargo test --release -p modengine-editor
    /// -- --ignored smoke_real_cli_imports_art --nocapture` (`MODENGINE_SMOKE_KEEP=1` keeps the folders).
    #[test]
    #[ignore]
    fn smoke_real_cli_imports_art() {
        let Some(kind) = std::env::var("MODENGINE_AGENT_SMOKE").ok().and_then(|k| Kind::from_id(&k)) else { return };
        fn copy_dir(from: &Path, to: &Path) {
            std::fs::create_dir_all(to).unwrap();
            for e in std::fs::read_dir(from).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    copy_dir(&p, &to.join(e.file_name()));
                } else {
                    std::fs::copy(&p, to.join(e.file_name())).unwrap();
                }
            }
        }
        let base = std::env::temp_dir().join(format!("modeditor_smoke_import_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let project = base.join("platformer");
        copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/platformer"), &project);
        let project = project.canonicalize().unwrap();
        let art = base.join("arte descargado");
        std::fs::create_dir_all(&art).unwrap();
        // real downloads from the store cache when present
        let cache = modengine::home_dir().unwrap_or_default().join(".cache/modengine/store/files");
        let mut credits = String::new();
        for (dir, file, credit) in [
            ("92779efe4ffdf578", "slime spritesheet calciumtrice.png", "slime spritesheet calciumtrice.png: «Animated Slime» by Calciumtrice, CC-BY 3.0, https://opengameart.org/content/animated-slime"),
            ("1df5f6b91d1fd194", "forest.png", "forest.png: «Forest Scene Items + Animated Slime/Exp Orbs», CC0, https://opengameart.org/content/forest-scene-items-animated-slimeexp-orbs"),
        ] {
            if std::fs::copy(cache.join(dir).join(file), art.join(file)).is_ok() {
                credits += &format!("{credit}\n");
            }
        }
        // a little knight running (6 frames of 24×24, no hint in the name) and a 16×16 grass tile set
        let knight = image::RgbaImage::from_fn(144, 24, |x, y| {
            let (f, cx) = (x / 24, x % 24);
            let leg = (f % 3) as i32 - 1;
            let body = (8..16).contains(&cx) && (6..16).contains(&y);
            let head = (9..15).contains(&cx) && (1..6).contains(&y);
            let legs = y >= 16 && y < 23 && ((cx as i32 - 9 - leg).abs() <= 1 || (cx as i32 - 14 + leg).abs() <= 1);
            if head { image::Rgba([240, 200, 160, 255]) } else if body { image::Rgba([60, 90, 200, 255]) } else if legs { image::Rgba([60, 40, 30, 255]) } else { image::Rgba([0, 0, 0, 0]) }
        });
        knight.save(art.join("knight run.png")).unwrap();
        let grass = image::RgbaImage::from_fn(64, 48, |x, y| {
            let t = (x / 16) + (y / 16) * 4;
            if y % 16 < 4 && y / 16 == 0 { image::Rgba([60, 170 + (t * 7) as u8, 60, 255]) } else { image::Rgba([120 + (t * 5) as u8, 80, 40 + ((x ^ y) % 7) as u8 * 5, 255]) }
        });
        grass.save(art.join("grass_tiles.png")).unwrap();
        credits += "knight run.png, grass_tiles.png: hechos para esta prueba, dominio público\n";
        std::fs::write(art.join("README.txt"), format!("Créditos del pack\n{credits}")).unwrap();
        let art = art.canonicalize().unwrap();

        let mut p = AgentPanel::with_config(None);
        p.set_kind(kind);
        p.set_permission(Permission::Edit);
        p.set_model(&std::env::var("MODENGINE_AGENT_MODEL").unwrap_or_default());
        assert!(p.program.is_some(), "{} not installed", kind.name());
        p.tool_override = Some(workspace_modengine());
        assert!(p.attach_path(&art));
        let text = import_instruction(Some(&project), &p.attachments).unwrap();
        println!("prompt: {text}");
        let before = snapshot::Snapshot::take(&project);
        p.send(&text, &project, "Coin Climber");
        let ctx = egui::Context::default();
        let (start, mut imported) = (Instant::now(), vec![]);
        while p.running() && start.elapsed() < Duration::from_secs(900) {
            imported.extend(p.poll(&ctx).imported);
            std::thread::sleep(Duration::from_millis(50));
        }
        for e in &p.entries {
            println!("{e:?}");
        }
        println!("status={} imported={imported:?}", p.status);
        let changes = snapshot::changes(&before, &snapshot::Snapshot::take(&project));
        for c in &changes {
            println!("{:?} {} {}", c.kind, c.rel, if c.binary { "(binario)" } else { "" });
        }
        for f in changes.iter().filter(|c| c.rel.ends_with(".toml") && c.kind != ChangeKind::Deleted) {
            println!("--- {}\n{}", f.rel, std::fs::read_to_string(project.join(&f.rel)).unwrap_or_default());
        }
        assert!(!p.running() && p.status.starts_with("Hecho"), "{}", p.status);
        let game = modengine::project::Project::load(&project).expect("the project still loads");
        let new_pngs = changes.iter().filter(|c| c.kind == ChangeKind::Added && c.rel.starts_with("sprites/") && c.rel.ends_with(".png")).count();
        assert!(new_pngs >= 3, "imported {new_pngs} images");
        assert!(changes.iter().any(|c| c.kind == ChangeKind::Added && c.rel.starts_with("tilesets/")), "no tile set");
        assert!(game.sprites.iter().any(|s| s.frames.len() == 6 && s.fw == 24), "the knight strip has 6 frames of 24 px");
        assert!(!imported.is_empty());
        let check = Command::new(workspace_modengine()).args(["check", project.to_str().unwrap(), "--steps", "60"]).output().unwrap();
        println!("{}{}", String::from_utf8_lossy(&check.stdout), String::from_utf8_lossy(&check.stderr));
        assert!(check.status.success());
        if std::env::var_os("MODENGINE_SMOKE_KEEP").is_none() {
            let _ = std::fs::remove_dir_all(&base);
        } else {
            println!("kept {}", base.display());
        }
    }

    /// One real turn in "Editar archivos" getting an asset from the store (OpenGameArt, real
    /// network, polite) into a copy of examples/platformer:
    /// `MODENGINE_AGENT_SMOKE=claude cargo test --release -p modengine-editor -- --ignored
    /// smoke_real_cli_store --nocapture`. `MODENGINE_SMOKE_HOME=<dir>` keeps the project and the
    /// conversation there (`<dir>/.config/modengine/agent.toml`), to open it in the editor after.
    #[test]
    #[ignore]
    fn smoke_real_cli_store() {
        let Some(kind) = std::env::var("MODENGINE_AGENT_SMOKE").ok().and_then(|k| Kind::from_id(&k)) else { return };
        fn copy_dir(from: &Path, to: &Path) {
            std::fs::create_dir_all(to).unwrap();
            for e in std::fs::read_dir(from).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    copy_dir(&p, &to.join(e.file_name()));
                } else {
                    std::fs::copy(&p, to.join(e.file_name())).unwrap();
                }
            }
        }
        let keep = std::env::var_os("MODENGINE_SMOKE_HOME").map(PathBuf::from);
        let base = keep.clone().unwrap_or_else(|| std::env::temp_dir().join(format!("modeditor_smoke_store_{}", std::process::id())));
        let project = base.join("platformer");
        let _ = std::fs::remove_dir_all(&project);
        copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/platformer"), &project);
        let project = project.canonicalize().unwrap();
        let mut p = AgentPanel::with_config(Some(base.join(".config/modengine/agent.toml")));
        p.set_kind(kind);
        p.enter_project(&project, "Coin Climber");
        p.set_permission(Permission::Edit);
        p.set_internet(true);
        p.set_model(&std::env::var("MODENGINE_AGENT_MODEL").unwrap_or_default());
        assert!(p.program.is_some(), "{} not installed", kind.name());
        p.tool_override = Some(workspace_modengine());
        let before = snapshot::Snapshot::take(&project);
        p.send("busca en la Tienda un sprite de moneda o gema CC0 y añádelo al proyecto con sus créditos", &project, "Coin Climber");
        let ctx = egui::Context::default();
        let (start, mut imported, mut downloaded) = (Instant::now(), vec![], vec![]);
        while p.running() && start.elapsed() < Duration::from_secs(900) {
            let o = p.poll(&ctx);
            imported.extend(o.imported);
            downloaded.extend(o.downloaded);
            std::thread::sleep(Duration::from_millis(50));
        }
        for e in &p.entries {
            println!("{e:?}");
        }
        println!("status={} imported={imported:?} downloaded={downloaded:?}", p.status);
        let changes = snapshot::changes(&before, &snapshot::Snapshot::take(&project));
        for c in &changes {
            println!("{:?} {} {}", c.kind, c.rel, if c.binary { "(binario)" } else { "" });
        }
        for f in changes.iter().filter(|c| (c.rel.ends_with(".toml") || c.rel.ends_with(".md")) && c.kind != ChangeKind::Deleted) {
            println!("--- {}\n{}", f.rel, std::fs::read_to_string(project.join(&f.rel)).unwrap_or_default());
        }
        assert!(!p.running() && p.status.starts_with("Hecho"), "{}", p.status);
        assert!(changes.iter().any(|c| c.kind == ChangeKind::Added && c.rel.starts_with("sprites/") && c.rel.ends_with(".png")), "no new sprite");
        let credits = std::fs::read_to_string(project.join("CREDITS.md")).unwrap_or_default();
        assert!(credits.contains("<!-- oga:") && changes.iter().any(|c| c.rel == "CREDITS.md"), "{credits}");
        assert!(!downloaded.is_empty(), "the Salida line");
        assert!(p.entries.iter().any(|e| matches!(e, Entry::Changes { .. })));
        let check = Command::new(workspace_modengine()).args(["check", project.to_str().unwrap(), "--steps", "60"]).output().unwrap();
        println!("{}{}", String::from_utf8_lossy(&check.stdout), String::from_utf8_lossy(&check.stderr));
        assert!(check.status.success());
        if keep.is_none() {
            let _ = std::fs::remove_dir_all(&base);
        }
    }

    /// One real turn in "Editar archivos" drawing a sprite with the pixel art guide and
    /// `modengine pixel-check` on a copy of examples/platformer:
    /// `MODENGINE_AGENT_SMOKE=claude cargo test --release -p modengine-editor -- --ignored
    /// smoke_real_cli_pixel_art --nocapture`. `MODENGINE_SMOKE_HOME=<dir>` keeps the project,
    /// the conversation and a copy of the previews (`<dir>/previews`).
    #[test]
    #[ignore]
    fn smoke_real_cli_pixel_art() {
        let Some(kind) = std::env::var("MODENGINE_AGENT_SMOKE").ok().and_then(|k| Kind::from_id(&k)) else { return };
        fn copy_dir(from: &Path, to: &Path) {
            std::fs::create_dir_all(to).unwrap();
            for e in std::fs::read_dir(from).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    copy_dir(&p, &to.join(e.file_name()));
                } else {
                    std::fs::copy(&p, to.join(e.file_name())).unwrap();
                }
            }
        }
        let keep = std::env::var_os("MODENGINE_SMOKE_HOME").map(PathBuf::from);
        let base = keep.clone().unwrap_or_else(|| std::env::temp_dir().join(format!("modeditor_smoke_pixel_{}", std::process::id())));
        let project = base.join("platformer");
        let _ = std::fs::remove_dir_all(&project);
        copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/platformer"), &project);
        let project = project.canonicalize().unwrap();
        let previews = agent::pixel_preview_dir();
        let _ = std::fs::remove_dir_all(&previews);
        let mut p = AgentPanel::with_config(Some(base.join(".config/modengine/agent.toml")));
        p.set_kind(kind);
        p.enter_project(&project, "Coin Climber");
        p.set_permission(Permission::Edit);
        p.set_internet(false);
        p.set_model(&std::env::var("MODENGINE_AGENT_MODEL").unwrap_or_default());
        assert!(p.program.is_some(), "{} not installed", kind.name());
        p.tool_override = Some(workspace_modengine());
        let before = snapshot::Snapshot::take(&project);
        p.send("Dibuja un sprite de llave dorada de 16×16 para recoger, con la paleta del proyecto", &project, "Coin Climber");
        let ctx = egui::Context::default();
        let start = Instant::now();
        while p.running() && start.elapsed() < Duration::from_secs(900) {
            p.poll(&ctx);
            std::thread::sleep(Duration::from_millis(50));
        }
        for e in &p.entries {
            println!("{e:?}");
        }
        println!("status={} usage={:?}", p.status, p.usage);
        let changes = snapshot::changes(&before, &snapshot::Snapshot::take(&project));
        for c in &changes {
            println!("{:?} {}", c.kind, c.rel);
        }
        assert!(!p.running() && p.status.starts_with("Hecho"), "{}", p.status);
        let tools: Vec<&ToolLine> = p.entries.iter().filter_map(|e| if let Entry::Tool(t) = e { Some(t) } else { None }).collect();
        assert!(tools.iter().any(|t| t.verb == "Leer" && t.detail.ends_with("pixel-art.md")), "did not read the guide");
        assert!(tools.iter().any(|t| t.detail.contains("pixel-check") && t.detail.contains("--preview")), "did not run pixel-check with previews");
        assert!(tools.iter().any(|t| t.verb == "Leer" && t.detail.contains("modengine-pixel-check") && t.detail.ends_with(".png")), "did not open a preview");
        let added: Vec<&str> = changes.iter().filter(|c| c.kind == ChangeKind::Added && c.rel.starts_with("sprites/") && c.rel.ends_with(".toml")).map(|c| c.rel.as_str()).collect();
        assert_eq!(added.len(), 1, "{added:?}");
        let rel = added[0];
        println!("--- {rel}\n{}", std::fs::read_to_string(project.join(rel)).unwrap());
        let game = modengine::project::Project::load(&project).expect("the project still loads");
        let name = Path::new(rel).file_stem().unwrap().to_string_lossy().to_string();
        let s = game.sprites.iter().find(|s| s.name == name).unwrap();
        assert_eq!((s.fw, s.fh), (16, 16));
        // the checker, as the user would run it: no errors and every colour in the palette
        let out_dir = base.join("previews");
        let out = Command::new(workspace_modengine())
            .args(["pixel-check", project.to_str().unwrap(), &name, "--size", "16x16", "--json", "--preview", out_dir.to_str().unwrap()])
            .output()
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
        assert!(out.status.success(), "pixel-check failed");
        let checks = v["sprites"][0]["checks"].as_array().unwrap();
        assert_eq!(checks.iter().find(|c| c["id"] == "closed_palette").unwrap()["status"], "ok", "colours outside the palette");
        let check = Command::new(workspace_modengine()).args(["check", project.to_str().unwrap(), "--steps", "60"]).output().unwrap();
        println!("{}{}", String::from_utf8_lossy(&check.stdout), String::from_utf8_lossy(&check.stderr));
        assert!(check.status.success());
        if keep.is_none() {
            let _ = std::fs::remove_dir_all(&base);
        }
    }

    /// One real turn in Solo planificar that needs the web (🌐 Internet on):
    /// `MODENGINE_AGENT_SMOKE=claude cargo test --release -p modengine-editor -- --ignored
    /// smoke_real_cli_web --nocapture`.
    #[test]
    #[ignore]
    fn smoke_real_cli_web() {
        let Some(kind) = std::env::var("MODENGINE_AGENT_SMOKE").ok().and_then(|k| Kind::from_id(&k)) else { return };
        let project = crate::room::tests::temp_project("smoke_web").canonicalize().unwrap();
        let mut p = AgentPanel::with_config(None);
        p.set_kind(kind);
        p.set_permission(Permission::Plan);
        p.set_internet(true);
        p.set_model(&std::env::var("MODENGINE_AGENT_MODEL").unwrap_or_default());
        assert!(p.program.is_some(), "{} not installed", kind.name());
        p.send("busca en internet qué licencia tiene el pack Kenney Pixel Platformer y dame la URL. No edites nada.", &project, "Web");
        let ctx = egui::Context::default();
        let start = Instant::now();
        while p.running() && start.elapsed() < Duration::from_secs(400) {
            p.poll(&ctx);
            std::thread::sleep(Duration::from_millis(50));
        }
        for e in &p.entries {
            println!("{e:?}");
        }
        assert!(!p.running() && p.status.starts_with("Hecho"), "{}", p.status);
        assert!(p.entries.iter().any(|e| matches!(e, Entry::Tool(t) if t.icon == "🌐")), "no web tool line");
        assert!(p.entries.iter().any(|e| matches!(e, Entry::Text(t) if t.contains("http"))), "no URL cited");
        let _ = std::fs::remove_dir_all(&project);
    }

    #[test]
    fn sends_attachments_to_fake_claude() {
        let project = crate::room::tests::temp_project("agent_att").canonicalize().unwrap();
        let outside = std::env::temp_dir().join(format!("modeditor_agent_outside_{}.md", std::process::id()));
        std::fs::write(&outside, "nota de fuera").unwrap();
        let (dir, exe) = fake_cli("att", r#"echo '{"type":"result","subtype":"success","session_id":"S3"}'"#);
        let mut p = panel(&exe);
        let proj = crate::project::Project::open(&project).unwrap();
        let log = vec![crate::app::LogLine { kind: crate::app::LogKind::Error, text: "rooms/x.toml: falta el objeto Foo".into() }];
        let src = Sources { project: Some(&proj), selected: Some(Open::Object(proj.objects[0].name.clone())), log: &log, ..Default::default() };
        p.attach_defaults(&src);
        assert!(p.attach_path(&outside));
        assert!(!p.attach_path(Path::new("/no/such/file")) && p.attach_error.is_some());
        // Attaching the same thing twice keeps one.
        p.attach(Attachment::file(&outside));
        assert_eq!(p.attachments.len(), 3, "{:?}", p.attachments);
        // An empty message with attachments is sent.
        p.send("", &project, "Coin Climber");
        wait(&mut p);
        let stdin = std::fs::read_to_string(dir.join("stdin.txt")).unwrap();
        let rel = proj.scripts[proj.objects[0].script].rel.clone();
        assert!(stdin.contains(&format!("- The project file `{rel}` (defines the object `{}`).", proj.objects[0].name)), "{stdin}");
        assert!(stdin.contains("- Salida (the editor's output log") && stdin.contains("[error] rooms/x.toml: falta el objeto Foo"), "{stdin}");
        assert!(stdin.contains("from outside the project:\n```md\nnota de fuera\n```"), "{stdin}");
        let args = lines(&dir.join("args.txt"));
        let outside_rule = format!("Read(/{})", outside.canonicalize().unwrap().display());
        assert!(args.contains(&outside_rule), "{args:?}");
        // The transcript notes what went, and the next message starts without attachments.
        assert!(matches!(&p.entries[0], Entry::Attached(a) if a.len() == 3), "{:?}", p.entries);
        assert!(p.attachments.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&project);
        let _ = std::fs::remove_file(&outside);
    }

    #[test]
    fn mentions_attach_assets_and_files() {
        let project = crate::room::tests::temp_project("agent_at").canonicalize().unwrap();
        let proj = crate::project::Project::open(&project).unwrap();
        let src = Sources { project: Some(&proj), ..Default::default() };
        let mut p = AgentPanel::with_config(None);
        let room = proj.rooms[0].name.clone();
        p.input = format!("cambia @{}", &room[..room.len().min(3)]);
        let c = p.mention_candidates(&src);
        assert!(c.iter().any(|(l, _)| *l == format!("▦ {room}")), "{c:?}");
        let (_, a) = c.into_iter().find(|(l, _)| *l == format!("▦ {room}")).unwrap();
        p.pick_mention(a);
        assert_eq!(p.input, format!("cambia @{room} "));
        assert_eq!(p.attachments[0].path, project.join(format!("rooms/{room}.toml")));
        assert!(p.mention_candidates(&src).is_empty());
        // Project files show up too.
        p.input = "@game.to".into();
        assert!(p.mention_candidates(&src).iter().any(|(l, _)| l == "📄 game.toml"));
        let _ = std::fs::remove_dir_all(&project);
    }

    #[test]
    fn chips_render_remove_and_accept_dropped_assets() {
        use egui_kittest::{kittest::Queryable, Harness};
        let (dir, exe) = fake_cli("chips", "true");
        let project = crate::room::tests::temp_project("agent_chips").canonicalize().unwrap();
        let mut p = panel(&exe);
        p.attachments = vec![Attachment::file(&project.join("game.toml")), Attachment::text("☰", "Salida", "ok")];
        p.entries = vec![Entry::User("mira".into()), Entry::Attached(vec![Attachment::text("⬚", "Selección en level1", "{ x = 1 }")])];
        let proj = crate::project::Project::open(&project).unwrap();
        let room = proj.rooms[0].name.clone();
        let mut h = Harness::builder().with_size([420.0, 700.0]).allow_missing_glyphs().with_accessibility_check(false).build_ui_state(
            |ui, (p, proj): &mut (AgentPanel, crate::project::Project)| {
                let src = Sources { project: Some(proj), ..Default::default() };
                p.ui(ui, &src);
            },
            (p, proj),
        );
        h.run();
        h.get_by_label("game.toml");
        h.get_by_label("Salida");
        h.get_by_label("Selección en level1");
        h.get_by_label("📎 Adjuntar");
        // × removes the first chip.
        h.get_all_by_label("×").next().unwrap().click();
        h.run();
        assert_eq!(h.state().0.attachments.len(), 1);
        assert!(h.query_by_label("game.toml").is_none());
        // An asset dragged from the browser and released over the dock is attached.
        let at = egui::pos2(200.0, 300.0);
        h.event(egui::Event::PointerMoved(at));
        h.run();
        DragAndDrop::set_payload(&h.ctx, Open::Room(room.clone()));
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run();
        let atts = &h.state().0.attachments;
        assert_eq!(atts.len(), 2, "{atts:?}");
        assert_eq!(atts[1].label, room);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&project);
    }

    #[test]
    fn pasted_screenshots_are_saved_attached_and_shown() {
        use egui_kittest::{kittest::Queryable, Harness};
        let (dir, exe) = fake_cli("paste", "true");
        let project = crate::room::tests::temp_project("agent_paste").canonicalize().unwrap();
        let mut p = panel(&exe);
        p.paste_dir = Some(dir.join("pegadas"));
        let proj = crate::project::Project::open(&project).unwrap();
        let mut h = Harness::builder().with_size([420.0, 700.0]).allow_missing_glyphs().with_accessibility_check(false).build_ui_state(
            |ui, (p, proj): &mut (AgentPanel, crate::project::Project)| {
                let src = Sources { project: Some(proj), ..Default::default() };
                p.ui(ui, &src);
            },
            (p, proj),
        );
        h.run();
        h.get_by_label("📋 Pegar imagen");
        h.get_by_label("📸 Captura del editor");
        // Ctrl+V of an image while the pointer is over the panel.
        h.event(egui::Event::PointerMoved(egui::pos2(200.0, 300.0)));
        let img = egui::ColorImage::filled([64, 32], Color32::from_rgb(10, 200, 30));
        h.event(egui::Event::PasteImage(std::sync::Arc::new(img)));
        h.run();
        let a = h.state().0.attachments.last().cloned().expect("attached");
        assert_eq!(a.kind, AttachKind::Image);
        assert_eq!(a.label, "Captura 64×32");
        assert!(a.path.starts_with(dir.join("pegadas")) && a.path.extension().unwrap() == "png");
        assert_eq!(image::open(&a.path).unwrap().to_rgba8().get_pixel(1, 1).0, [10, 200, 30, 255]);
        // Its chip has a thumbnail and the label; × removes it.
        h.get_by_label("Captura 64×32");
        assert!(h.state().0.thumbs.get(&a.path).is_some_and(|t| t.is_some()));
        h.get_all_by_label("×").next().unwrap().click();
        h.run();
        assert!(h.state().0.attachments.is_empty());
        // The image goes outside the project, so an agent gets read access to it.
        let images: Vec<_> = vec![a.clone()];
        assert!(!a.path.starts_with(&project) && images.len() == 1);
        let _ = std::fs::remove_dir_all(&dir);
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
            p.cfg.models.insert("codex".into(), vec![Model { id: "gpt-x".into(), name: "GPT X".into(), description: String::new(), context: 0 }]);
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
                p.ui(ui, &Sources::default());
            },
            p,
        );
        h.run();
        h.get_by_label("Editar");
        h.get_by_label("uno");
        h.get_by_label("Enviar");
        // Without the CLI the panel says how to install it.
        h.state_mut().program_override = None;
        h.state_mut().program = None;
        h.run();
        h.get_by_label("Comprobar de nuevo");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A panel that keeps its history in a temp folder.
    fn panel_with_history(exe: &Path, tag: &str) -> (AgentPanel, PathBuf) {
        let cfg = std::env::temp_dir().join(format!("modeditor_agent_hist_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&cfg);
        let mut p = AgentPanel::with_config(Some(cfg.join("agent.toml")));
        p.program_override = Some(exe.to_path_buf());
        p.locate();
        p.list_tried = true;
        (p, cfg)
    }

    const CLAUDE_TURN: &str = r#"echo '{"type":"system","subtype":"init","session_id":"S-h"}'
echo '{"type":"assistant","session_id":"S-h","uuid":"U-'$$'","message":{"model":"m","content":[{"type":"text","text":"Vale."}],"usage":{"input_tokens":10,"cache_creation_input_tokens":100,"cache_read_input_tokens":1890,"output_tokens":5}}}'
echo '{"type":"result","subtype":"success","session_id":"S-h","total_cost_usd":0.25,"usage":{"output_tokens":5},"modelUsage":{"m":{"contextWindow":10000}}}'"#;

    #[test]
    fn history_saves_restores_and_resumes() {
        let project = crate::room::tests::temp_project("agent_hist").canonicalize().unwrap();
        let (dir, exe) = fake_cli("hist", CLAUDE_TURN);
        let (mut p, cfg) = panel_with_history(&exe, "a");
        p.send("Haz que el jugador salte más", &project, "Coin Climber");
        wait(&mut p);
        assert_eq!(p.session, "S-h");
        assert_eq!(p.title, "Haz que el jugador salte más");
        assert_eq!((p.usage.context, p.usage.window, p.usage.cost()), (2005, 10000, 0.25));
        let id = p.conv_id.clone();
        assert!(!id.is_empty());
        drop(p);
        // Opening the project again brings the conversation back and resumes its session.
        let (mut p, _) = (AgentPanel::with_config(Some(cfg.join("agent.toml"))), ());
        p.program_override = Some(exe.clone());
        p.locate();
        p.list_tried = true;
        p.enter_project(&project, "Coin Climber");
        assert_eq!((p.conv_id.as_str(), p.session.as_str()), (id.as_str(), "S-h"));
        assert_eq!(p.entries[0], Entry::User("Haz que el jugador salte más".into()));
        p.send("y más rápido", &project, "Coin Climber");
        wait(&mut p);
        assert!(lines(&dir.join("args.txt")).windows(2).any(|w| w == ["--resume", "S-h"]));
        // A second conversation; the list has both, newest first; rename, export and delete.
        p.new_conversation();
        p.send("Otra cosa", &project, "Coin Climber");
        wait(&mut p);
        p.show_history("");
        assert_eq!(p.view, View::History);
        assert_eq!(p.history.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(), ["Otra cosa", "Haz que el jugador salte más"]);
        p.rename(&id, "Salto");
        assert!(p.history.iter().any(|s| s.title == "Salto"));
        let out = p.export(&id, Some(&cfg)).unwrap();
        assert!(std::fs::read_to_string(&out).unwrap().starts_with("# Salto\n"));
        assert!(p.open_conversation(&id));
        assert_eq!(p.title, "Salto");
        p.delete_conversation(&id);
        assert!(p.entries.is_empty() && p.history.len() == 1);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&cfg);
        let _ = std::fs::remove_dir_all(&project);
    }

    #[test]
    fn queues_messages_while_running() {
        let project = crate::room::tests::temp_project("agent_queue").canonicalize().unwrap();
        let (dir, exe) = fake_cli("queue", &format!("sleep 0.4\n{CLAUDE_TURN}"));
        let mut p = panel(&exe);
        p.send("primero", &project, "x");
        assert!(p.running());
        p.attach(Attachment::text("📝", "Nota", "abc"));
        p.input = "segundo".into();
        p.send("segundo", &project, "x");
        p.enqueue("tercero");
        assert_eq!(p.queue.iter().map(|q| q.text.as_str()).collect::<Vec<_>>(), ["segundo", "tercero"]);
        assert_eq!(p.queue[0].attachments.len(), 1, "the attachments go with the queued message");
        assert!(p.next_to_send().is_none(), "nothing is sent while the agent works");
        // Editing one puts it back in the box.
        p.unqueue(1);
        assert_eq!((p.input.as_str(), p.queue.len()), ("tercero", 1));
        wait(&mut p);
        let next = p.next_to_send().unwrap();
        assert_eq!(next, "segundo");
        p.send(&next, &project, "x");
        wait(&mut p);
        assert!(std::fs::read_to_string(dir.join("stdin.txt")).unwrap().contains("- Nota:\n```\nabc"));
        assert!(p.queue.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&project);
    }

    #[test]
    fn failed_turn_pauses_the_queue_and_retry_resends() {
        let project = crate::room::tests::temp_project("agent_retry").canonicalize().unwrap();
        let (dir, exe) = fake_cli(
            "retry",
            r#"if [ -f "$(dirname "$0")/ok" ]; then
echo '{"type":"result","subtype":"success","session_id":"S-r"}'
else
sleep 0.3; echo '{"type":"result","subtype":"error_during_execution","is_error":true,"session_id":"S-r","errors":["se cayó"]}'
fi"#,
        );
        let mut p = panel(&exe);
        p.send("hazlo", &project, "x");
        p.enqueue("luego esto");
        wait(&mut p);
        assert!(p.queue_paused && p.next_to_send().is_none());
        assert!(matches!(p.entries.last(), Some(Entry::Notice(t, Tone::Error)) if t.contains("se cayó")));
        std::fs::write(dir.join("ok"), "").unwrap();
        let again = p.retry().unwrap();
        assert_eq!(p.next_to_send().as_deref(), Some("hazlo"));
        p.send(&again, &project, "x");
        wait(&mut p);
        // The failed attempt is gone from the transcript; the turn is there once.
        assert_eq!(p.entries.iter().filter(|e| matches!(e, Entry::User(_))).count(), 1, "{:?}", p.entries);
        assert!(!p.entries.iter().any(|e| matches!(e, Entry::Notice(_, Tone::Error))));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&project);
    }

    #[test]
    fn compact_uses_claude_slash_command() {
        let project = crate::room::tests::temp_project("agent_compact").canonicalize().unwrap();
        let (dir, exe) = fake_cli(
            "compact",
            &format!(
                r#"if grep -q '^/compact' "$(dirname "$0")/stdin.txt"; then
echo '{{"type":"system","subtype":"status","status":"compacting","session_id":"S-h"}}'
echo '{{"type":"system","subtype":"compact_boundary","session_id":"S-h","compact_metadata":{{"trigger":"manual","pre_tokens":2005,"post_tokens":300}}}}'
echo '{{"type":"result","subtype":"success","session_id":"S-h","total_cost_usd":0.3}}'
else
{CLAUDE_TURN}
fi"#
            ),
        );
        let mut p = panel(&exe);
        assert!(!p.compact(), "nothing to compact without a session");
        p.send("hola", &project, "x");
        wait(&mut p);
        assert_eq!(p.command(Slash::Compact, None), None);
        assert!(p.running());
        wait(&mut p);
        let args = lines(&dir.join("args.txt"));
        assert!(args.windows(2).any(|w| w == ["--resume", "S-h"]), "{args:?}");
        assert_eq!(std::fs::read_to_string(dir.join("stdin.txt")).unwrap(), "/compact");
        assert_eq!(p.entries.iter().filter(|e| matches!(e, Entry::Divider(_))).count(), 1, "{:?}", p.entries);
        assert_eq!(p.usage.context, 300);
        assert_eq!(p.session, "S-h", "Claude keeps the same session");
        assert!((p.usage.cost() - 0.3).abs() < 1e-9);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&project);
    }

    #[test]
    fn compact_codex_seeds_a_new_session_with_a_summary() {
        let project = crate::room::tests::temp_project("agent_compact_cx").canonicalize().unwrap();
        let (dir, exe) = fake_cli(
            "compact_cx",
            r#"if grep -q 'Summarise our conversation' "$(dirname "$0")/stdin.txt"; then
echo '{"type":"thread.started","thread_id":"T1"}'
echo '{"type":"item.completed","item":{"id":"s","type":"agent_message","text":"- RESUMEN: salto a 12"}}'
echo '{"type":"turn.completed","usage":{"input_tokens":900,"output_tokens":40}}'
else
echo '{"type":"thread.started","thread_id":"T1"}'
echo '{"type":"item.completed","item":{"id":"m","type":"agent_message","text":"Hecho"}}'
echo '{"type":"turn.completed","usage":{"input_tokens":5000,"cached_input_tokens":4000,"output_tokens":50}}'
fi"#,
        );
        let mut p = panel(&exe);
        p.set_kind(Kind::Codex);
        p.send("sube el salto", &project, "x");
        wait(&mut p);
        assert_eq!((p.session.as_str(), p.usage.context, p.usage.approx), ("T1", 5050, true));
        assert!(p.compact());
        wait(&mut p);
        assert!(p.session.is_empty() && p.seed.contains("RESUMEN"), "{:?}", p.entries);
        assert!(matches!(&p.entries[p.entries.len() - 2..], [Entry::Divider(_), Entry::Summary(s)] if s.contains("RESUMEN")), "{:?}", p.entries);
        p.send("sigue", &project, "x");
        wait(&mut p);
        let args = lines(&dir.join("args.txt"));
        assert!(!args.contains(&"resume".to_string()), "{args:?}");
        let stdin = std::fs::read_to_string(dir.join("stdin.txt")).unwrap();
        assert!(stdin.contains("<earlier-conversation>") && stdin.contains("RESUMEN: salto a 12") && stdin.contains("sigue"), "{stdin}");
        assert!(p.seed.is_empty(), "the seed is used once");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&project);
    }

    #[test]
    fn changes_are_listed_diffed_and_undone() {
        let project = crate::room::tests::temp_project("agent_undo").canonicalize().unwrap();
        let target = project.join("objects/world.ms");
        let original = std::fs::read_to_string(&target).unwrap();
        let (dir, exe) = fake_cli(
            "undo",
            &format!(
                "echo '// agent line' >> '{t}'\necho 'nuevo' > '{n}'\necho '{{\"type\":\"result\",\"subtype\":\"success\",\"session_id\":\"S-u\"}}'",
                t = target.display(),
                n = project.join("notes.txt").display()
            ),
        );
        let mut p = panel(&exe);
        p.send("cambia cosas", &project, "x");
        wait(&mut p);
        let Some(Entry::Changes { turn, files }) = p.entries.last().cloned() else { panic!("{:?}", p.entries) };
        let summary: Vec<(&str, ChangeKind, usize, usize)> = files.iter().map(|f| (f.rel.as_str(), f.kind, f.added, f.removed)).collect();
        assert_eq!(summary, [("notes.txt", ChangeKind::Added, 1, 0), ("objects/world.ms", ChangeKind::Modified, 1, 0)]);
        p.open_diff(turn, "objects/world.ms");
        let d = p.diff.as_ref().unwrap();
        assert!(d.lines.iter().any(|l| l.op == snapshot::Op::Add && l.text == "// agent line"), "{:?}", d.lines);
        // Revertir one file, then /deshacer the rest.
        p.revert_file(turn, "notes.txt").unwrap();
        assert!(!project.join("notes.txt").exists());
        assert_eq!(p.command(Slash::Undo, None), None);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), original);
        assert!(matches!(p.entries.iter().find(|e| matches!(e, Entry::Changes { .. })), Some(Entry::Changes { files, .. }) if files.iter().all(|f| f.reverted)));
        assert!(p.poll(&egui::Context::default()).reload, "the app reloads what was reverted");
        assert_eq!(p.undo_last_turn(), 0);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&project);
    }

    #[test]
    fn editing_the_last_message_forks_the_claude_session() {
        let project = crate::room::tests::temp_project("agent_fork").canonicalize().unwrap();
        let (dir, exe) = fake_cli("fork", CLAUDE_TURN);
        let mut p = panel(&exe);
        p.send("uno", &project, "x");
        wait(&mut p);
        let first_checkpoint = p.checkpoint.clone();
        assert!(first_checkpoint.starts_with("U-"));
        p.send("dos", &project, "x");
        wait(&mut p);
        assert!(p.edit_last());
        assert_eq!(p.input, "dos");
        p.input = "dos, mejor así".into();
        p.send("dos, mejor así", &project, "x");
        wait(&mut p);
        let args = lines(&dir.join("args.txt"));
        assert!(args.windows(2).any(|w| w == ["--resume", "S-h"]), "{args:?}");
        assert!(args.contains(&"--fork-session".to_string()));
        assert!(args.windows(2).any(|w| w[0] == "--resume-session-at" && w[1] == first_checkpoint), "{args:?}");
        let users: Vec<&str> = p.entries.iter().filter_map(|e| if let Entry::User(t) = e { Some(t.as_str()) } else { None }).collect();
        assert_eq!(users, ["uno", "dos, mejor así"]);
        // Codex cannot fork: editing starts a new session told the conversation so far.
        p.set_kind(Kind::Codex);
        p.session = "T-x".into();
        p.turns.last_mut().unwrap().session_before = "T-x".into();
        assert!(p.edit_last());
        p.send("dos, otra vez", &project, "x");
        wait(&mut p);
        let stdin = std::fs::read_to_string(dir.join("stdin.txt")).unwrap();
        assert!(!lines(&dir.join("args.txt")).contains(&"resume".to_string()));
        assert!(stdin.contains("User: uno") && !stdin.contains("User: dos, mejor así") && stdin.ends_with("dos, otra vez"), "{stdin}");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&project);
    }

    #[test]
    fn slash_commands_and_pasted_paths() {
        let (dir, exe) = fake_cli("slash", "true");
        let mut p = panel(&exe);
        p.command(Slash::Model(Some("opus".into())), None);
        assert_eq!(p.model(), "opus");
        p.command(Slash::Permission(Some(Permission::Plan)), None);
        assert_eq!(p.permission, Permission::Plan);
        p.command(Slash::Agent(Some(Kind::Codex)), None);
        assert_eq!(p.kind, Kind::Codex);
        p.command(Slash::AutoCompact(Some(80)), None);
        assert_eq!(p.autocompact, 80);
        p.command(Slash::Help, None);
        assert!(matches!(p.entries.last(), Some(Entry::Notice(t, _)) if t.contains("/compactar")));
        p.command(Slash::Unknown("xyz".into()), None);
        assert!(matches!(p.entries.last(), Some(Entry::Notice(t, Tone::Warning)) if t.contains("/xyz")));
        // Pasted paths and file:// URIs attach; other text does not.
        let f = dir.join("args dir");
        std::fs::create_dir_all(&f).unwrap();
        assert!(p.attach_pasted(&format!("file://{}\n{}", f.display().to_string().replace(' ', "%20"), dir.join("claude").display())));
        assert_eq!(p.attachments.len(), 2);
        assert!(!p.attach_pasted("hola mundo"));
        assert!(!p.attach_pasted("/no/existe"));
        assert!(p.attach_url("https://example.com/a?b=1"));
        assert!(!p.attach_url("example.com"));
        assert_eq!(p.attachments[2].kind, AttachKind::Url);
        // Recent outside attachments are remembered.
        assert!(p.cfg.recent_paths.contains(&f.canonicalize().unwrap()) && p.cfg.recent_urls[0] == "https://example.com/a?b=1");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn autocompacts_past_the_threshold() {
        let project = crate::room::tests::temp_project("agent_auto").canonicalize().unwrap();
        let (dir, exe) = fake_cli(
            "auto",
            &format!(
                r#"if grep -q '^/compact' "$(dirname "$0")/stdin.txt"; then
echo '{{"type":"system","subtype":"compact_boundary","session_id":"S-h","compact_metadata":{{"post_tokens":100}}}}'
echo '{{"type":"result","subtype":"success","session_id":"S-h"}}'
else
{CLAUDE_TURN}
fi"#
            ),
        );
        let mut p = panel(&exe);
        p.autocompact = 15; // 2005 of 10000 tokens is past it
        p.send("hola", &project, "x");
        // The compaction starts right after the turn (wait() waits for both).
        wait(&mut p);
        assert_eq!(p.usage.context, 100);
        assert!(p.entries.iter().any(|e| matches!(e, Entry::Divider(_))));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&project);
    }

    #[test]
    fn history_queue_and_meter_render() {
        use egui_kittest::{kittest::Queryable, Harness};
        let (dir, exe) = fake_cli("render", "true");
        let project = crate::room::tests::temp_project("agent_render").canonicalize().unwrap();
        let proj = crate::project::Project::open(&project).unwrap();
        let mut p = panel(&exe);
        p.demo("history", &project, "Coin Climber");
        let mut h = Harness::builder().with_size([420.0, 760.0]).allow_missing_glyphs().with_accessibility_check(false).build_ui_state(
            |ui, (p, proj): &mut (AgentPanel, crate::project::Project)| {
                let src = Sources { project: Some(proj), ..Default::default() };
                p.ui(ui, &src);
            },
            (p, proj),
        );
        h.run();
        h.get_by_label("Haz que el jugador salte más alto");
        h.get_by_label("Room del nivel 2 con plataformas móviles");
        h.get_by_label("← Volver").click();
        h.run();
        assert_eq!(h.state().0.view, View::Chat);
        h.get_by_label("Archivos cambiados (2)");
        h.get_by_label("✂ Conversación compactada");
        h.get_by_label("jump_speed");
        // The queue.
        h.state_mut().0.queue = vec![Queued { text: "luego esto".into(), attachments: vec![] }];
        h.state_mut().0.queue_paused = true;
        h.run();
        h.get_by_label("En cola (1)");
        h.get_by_label("luego esto");
        // Typing / lists the commands.
        h.state_mut().0.input = "/comp".into();
        h.run();
        h.get_by_label("/compactar");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&project);
    }

    /// The `modengine` built from this workspace (built now when it is missing).
    fn workspace_modengine() -> PathBuf {
        let exe = std::env::current_exe().unwrap();
        let profile_dir = exe.parent().unwrap().parent().unwrap().to_path_buf(); // target/<profile>
        let bin = profile_dir.join(format!("modengine{}", std::env::consts::EXE_SUFFIX));
        if !bin.exists() {
            let mut c = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
            c.args(["build", "-p", "modengine-cli", "--bin", "modengine"]).current_dir(env!("CARGO_MANIFEST_DIR"));
            if profile_dir.file_name().is_some_and(|n| n == "release") {
                c.arg("--release");
            }
            assert!(c.status().unwrap().success(), "could not build modengine");
        }
        bin
    }

    /// An art folder outside the project: a strip named as one, a tile set and a licence.
    fn art_folder(tag: &str) -> PathBuf {
        let art = std::env::temp_dir().join(format!("modeditor_agent_art_{tag}_{}", std::process::id())).join("Arte del usuario");
        let _ = std::fs::remove_dir_all(&art);
        std::fs::create_dir_all(&art).unwrap();
        image::RgbaImage::from_fn(64, 16, |x, _| if x % 16 > 2 && x % 16 < 13 { image::Rgba([220, 60, 60, 255]) } else { image::Rgba([0, 0, 0, 0]) }).save(art.join("hero_strip4.png")).unwrap();
        image::RgbaImage::from_fn(32, 32, |x, y| image::Rgba([(x / 16 * 100) as u8, (y / 16 * 100) as u8, 80, 255])).save(art.join("cave_tiles.png")).unwrap();
        std::fs::write(art.join("LICENSE.txt"), "CC0, by Tester\n").unwrap();
        art.canonicalize().unwrap()
    }

    #[test]
    fn fake_claude_imports_attached_art_and_undo_removes_it() {
        let project = crate::room::tests::temp_project("agent_import").canonicalize().unwrap();
        let before = snapshot::Snapshot::take(&project);
        let art = art_folder("fake");
        let bin = workspace_modengine();
        // The fake Claude runs the importer the editor allowed it to (read from its own argv).
        let body = format!(
            r#"tool=$(sed -n 's/^Bash(\(.*\) import \*)$/\1/p' '{d}/args.txt' | head -n 1)
echo '{{"type":"system","subtype":"init","session_id":"S-imp"}}'
echo '{{"type":"assistant","session_id":"S-imp","message":{{"content":[{{"type":"tool_use","id":"b1","name":"Bash","input":{{"command":"modengine import . art --credit CC0"}}}}]}}}}'
"$tool" import . '{art}' --json --origin bottom-center --credit 'CC0, by Tester' > '{d}/import.json' 2> '{d}/import.err'
echo '{{"type":"user","session_id":"S-imp","message":{{"content":[{{"type":"tool_result","tool_use_id":"b1","content":"ok"}}]}}}}'
echo '{{"type":"result","subtype":"success","session_id":"S-imp","result":"Importados hero y ts_cave_tiles."}}'"#,
            d = std::env::temp_dir().join(format!("modeditor_agent_import_{}", std::process::id())).display(),
            art = art.display()
        );
        let (dir, exe) = fake_cli("import", &body);
        let mut p = panel(&exe);
        p.tool_override = Some(bin.clone());
        p.set_permission(Permission::Edit);
        assert!(p.attach_path(&art));
        let text = import_instruction(Some(&project), &p.attachments).expect("an art folder offers the import");
        assert!(text.contains("«Arte del usuario»") && text.contains("modengine import") && text.contains("--credit"), "{text}");
        p.send(&text, &project, "x");
        let ctx = egui::Context::default();
        let (mut imported, mut turn, start) = (vec![], None, Instant::now());
        while p.running() && start.elapsed() < Duration::from_secs(30) {
            let out = p.poll(&ctx);
            imported.extend(out.imported);
            turn = turn.or(out.turn);
            std::thread::sleep(Duration::from_millis(20));
        }
        let args = lines(&dir.join("args.txt"));
        assert!(args.windows(2).any(|w| w == ["--permission-mode", "acceptEdits"]));
        assert!(args.contains(&format!("Bash({} import *)", bin.display())) && args.contains(&format!("Bash({} inspect-image *)", bin.display())), "{args:?}");
        assert!(std::fs::read_to_string(dir.join("stdin.txt")).unwrap().contains("licence/credit files"));
        let out = std::fs::read_to_string(dir.join("import.json")).unwrap();
        assert!(out.contains("\"ok\": true"), "{out}{}", std::fs::read_to_string(dir.join("import.err")).unwrap_or_default());
        // the assets are in the project and load
        let game = modengine::project::Project::load(&project).unwrap();
        let hero = game.sprites.iter().find(|s| s.name == "hero").unwrap();
        assert_eq!((hero.frames.len(), hero.origin), (4, (8.0, 16.0)));
        assert!(game.tilesets.iter().any(|t| t.name == "ts_cave_tiles"));
        assert!(std::fs::read_to_string(project.join("CREDITS.md")).unwrap().contains("CC0, by Tester"));
        assert_eq!(imported, ["sprite 'hero'", "sprite 'spr_cave_tiles'", "tile set 'ts_cave_tiles'"]);
        // the turn hands over its request and new files, which open as these windows
        let turn = turn.expect("the turn's new files, for its workspace");
        assert_eq!(turn.prompt, text.trim());
        let ed = crate::project::Project::open(&project).unwrap();
        assert_eq!(
            crate::agent_workspace::assets_for(&ed, &turn.files, &turn.lines),
            [Open::Sprite("hero".into()), Open::Sprite("spr_cave_tiles".into()), Open::TileSet("ts_cave_tiles".into())]
        );
        // listed as created in the turn's changes, binaries included...
        let Some(Entry::Changes { files, .. }) = p.entries.iter().rev().find(|e| matches!(e, Entry::Changes { .. })).cloned() else { panic!("{:?}", p.entries) };
        let added: Vec<(&str, bool)> = files.iter().filter(|f| f.kind == ChangeKind::Added).map(|f| (f.rel.as_str(), f.binary)).collect();
        assert_eq!(
            added,
            [("CREDITS.md", false), ("sprites/hero.png", true), ("sprites/hero.toml", false), ("sprites/spr_cave_tiles.png", true), ("tilesets/ts_cave_tiles.toml", false)]
        );
        // ...and /deshacer removes them all
        assert_eq!(p.undo_last_turn(), 5);
        let after = snapshot::Snapshot::take(&project);
        assert!(snapshot::changes(&before, &after).is_empty(), "{:?}", snapshot::changes(&before, &after));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&project);
        let _ = std::fs::remove_dir_all(art.parent().unwrap());
    }

    #[test]
    fn attached_art_offers_the_import_button() {
        use egui_kittest::{kittest::Queryable, Harness};
        let (dir, exe) = fake_cli("import_btn", "true");
        let project = crate::room::tests::temp_project("agent_import_btn").canonicalize().unwrap();
        let art = art_folder("btn");
        // a project file or one picture is not "art to import"; a folder or several pictures are
        assert_eq!(import_instruction(Some(&project), &[Attachment::file(&project.join("game.toml"))]), None);
        assert_eq!(import_instruction(Some(&project), &[Attachment::file(&art.join("hero_strip4.png"))]), None);
        let two = [Attachment::file(&art.join("hero_strip4.png")), Attachment::file(&art.join("cave_tiles.png"))];
        assert!(import_instruction(Some(&project), &two).unwrap().starts_with("Importa al proyecto estas imágenes"));
        let mut p = panel(&exe);
        p.set_permission(Permission::Edit);
        p.attach_path(&art);
        let proj = crate::project::Project::open(&project).unwrap();
        let mut h = Harness::builder().with_size([420.0, 700.0]).allow_missing_glyphs().with_accessibility_check(false).build_ui_state(
            |ui, (p, proj): &mut (AgentPanel, crate::project::Project)| {
                let src = Sources { project: Some(proj), ..Default::default() };
                p.ui(ui, &src);
            },
            (p, proj),
        );
        h.run();
        h.get_by_label("Arte del usuario");
        h.get_by_label("🖼 Importar al proyecto").click();
        h.run();
        assert!(h.state().0.input.starts_with("Importa al proyecto el arte de la carpeta «Arte del usuario»"), "{}", h.state().0.input);
        // Solo planificar can't import: the button does nothing
        h.state_mut().0.input.clear();
        h.state_mut().0.set_permission(Permission::Plan);
        h.run();
        h.get_by_label("🖼 Importar al proyecto").click();
        h.run();
        assert!(h.state().0.input.is_empty());
        drop(h);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&project);
        let _ = std::fs::remove_dir_all(art.parent().unwrap());
    }

    #[test]
    fn fake_claude_gets_a_store_asset_and_undo_removes_it() {
        let project = crate::room::tests::temp_project("agent_store").canonicalize().unwrap();
        let before = snapshot::Snapshot::take(&project);
        let bin = workspace_modengine();
        let work = std::env::temp_dir().join(format!("modeditor_store_site_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&work);
        let site = modengine_store::testing::fixture_site(&work.join("site"));
        // The fake Claude runs `modengine store get` as allowed by its own argv, against the
        // canned OpenGameArt pages.
        let body = format!(
            r#"tool=$(sed -n 's/^Bash(\(.*\) store \*)$/\1/p' '{d}/args.txt' | head -n 1)
export MODENGINE_STORE_FIXTURES='{site}' XDG_CACHE_HOME='{cache}' MODENGINE_STORE_INTERVAL_MS=0
echo '{{"type":"system","subtype":"init","session_id":"S-store"}}'
echo '{{"type":"assistant","session_id":"S-store","message":{{"content":[{{"type":"tool_use","id":"b1","name":"Bash","input":{{"command":"modengine store get animated-slime ."}}}}]}}}}'
"$tool" store get animated-slime . --frames 4 --name oga_slime --origin bottom-center --json > '{d}/get.json' 2> '{d}/get.err'
echo '{{"type":"user","session_id":"S-store","message":{{"content":[{{"type":"tool_result","tool_use_id":"b1","content":"ok"}}]}}}}'
echo '{{"type":"result","subtype":"success","session_id":"S-store","result":"Importado slime (CC-BY 3.0)."}}'"#,
            d = std::env::temp_dir().join(format!("modeditor_agent_store_{}", std::process::id())).display(),
            site = site.display(),
            cache = work.join("cache").display(),
        );
        let (dir, exe) = fake_cli("store", &body);
        let mut p = panel(&exe);
        p.tool_override = Some(bin.clone());
        p.set_permission(Permission::Edit);
        p.send(&store_instruction("un slime animado"), &project, "x");
        let ctx = egui::Context::default();
        let (mut downloaded, mut imported, start) = (vec![], vec![], Instant::now());
        while p.running() && start.elapsed() < Duration::from_secs(30) {
            let o = p.poll(&ctx);
            downloaded.extend(o.downloaded);
            imported.extend(o.imported);
            std::thread::sleep(Duration::from_millis(20));
        }
        let args = lines(&dir.join("args.txt"));
        assert!(args.contains(&format!("Bash({} store *)", bin.display())), "{args:?}");
        let out = std::fs::read_to_string(dir.join("get.json")).unwrap_or_default();
        assert!(out.contains("\"ok\": true"), "{out}{}", std::fs::read_to_string(dir.join("get.err")).unwrap_or_default());
        let game = modengine::project::Project::load(&project).unwrap();
        let slime = game.sprites.iter().find(|s| s.name == "oga_slime").unwrap();
        assert_eq!((slime.frames.len(), slime.origin), (4, (8.0, 16.0)));
        assert!(std::fs::read_to_string(project.join("CREDITS.md")).unwrap().contains("<!-- oga:animated-slime -->"));
        assert_eq!(downloaded, ["«Animated Slime» (CC-BY 3.0) → sprite 'oga_slime'"]);
        assert_eq!(imported, ["sprite 'oga_slime'"]);
        // created files in the turn's changes (the PNG as a binary), and /deshacer removes them
        let Some(Entry::Changes { files, .. }) = p.entries.iter().rev().find(|e| matches!(e, Entry::Changes { .. })).cloned() else { panic!("{:?}", p.entries) };
        let added: Vec<(&str, bool)> = files.iter().filter(|f| f.kind == ChangeKind::Added).map(|f| (f.rel.as_str(), f.binary)).collect();
        assert_eq!(added, [("CREDITS.md", false), ("sprites/oga_slime.png", true), ("sprites/oga_slime.toml", false)]);
        assert_eq!(p.undo_last_turn(), 3);
        assert!(snapshot::changes(&before, &snapshot::Snapshot::take(&project)).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&project);
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn store_downloads_come_from_credits() {
        let c = modengine_store::credits::Credit {
            slug: "coins".into(),
            title: "Spinning Coins".into(),
            author: "A".into(),
            licenses: vec![("CC0".into(), Some("https://creativecommons.org/publicdomain/zero/1.0/".into()))],
            url: "https://opengameart.org/content/coins".into(),
            source_file: "coins.png".into(),
            ..Default::default()
        };
        let one = modengine_store::credits::add_credit("", &c, &["sprites/coin.png".into(), "sprites/coin.toml".into()], "2026-11-06");
        assert_eq!(store_downloads("", &one), ["«Spinning Coins» (CC0) → sprite 'coin'"]);
        assert!(store_downloads(&one, &one).is_empty(), "nothing new");
        let two = modengine_store::credits::add_credit(&one, &c, &["sounds/clink.ogg".into()], "2026-11-06");
        assert_eq!(store_downloads(&one, &two), ["«Spinning Coins» (CC0) → sonido 'clink'"]);
        let local = modengine::credits::add_local(&one, &modengine::credits::LocalCredit { slug: "p".into(), title: "p".into(), text: "x".into() }, &[("sprites/h.png".into(), "h.png".into())], "d");
        assert!(store_downloads(&one, &local).is_empty(), "local imports are not from the store");
        assert!(store_instruction("").contains("Tienda (OpenGameArt)") && store_instruction("una moneda").contains("una moneda"));
    }

    #[test]
    fn internet_is_per_project_and_reaches_the_cli() {
        let (dir, exe) = fake_cli("internet", r#"echo '{"type":"result","subtype":"success","session_id":"S-net"}'"#);
        let cfg = dir.join("cfg");
        let a = crate::room::tests::temp_project("agent_net_a").canonicalize().unwrap();
        let b = crate::room::tests::temp_project("agent_net_b").canonicalize().unwrap();
        let mut p = AgentPanel::with_config(Some(cfg.join("agent.toml")));
        p.program_override = Some(exe.clone());
        p.locate();
        p.list_tried = true;
        p.enter_project(&a, "A");
        assert!(p.internet, "on by default");
        p.command(Slash::Internet(Some(false)), None);
        assert!(!p.internet);
        assert!(matches!(p.entries.last(), Some(Entry::Notice(t, _)) if t.contains("Internet: no")));
        p.send("hola", &a, "A");
        wait(&mut p);
        let args = lines(&dir.join("args.txt"));
        assert!(args.windows(3).any(|w| w == ["--disallowedTools", "WebSearch", "WebFetch"]), "{args:?}");
        // another project has its own setting; coming back finds it off
        p.enter_project(&b, "B");
        assert!(p.internet);
        p.set_permission(Permission::Plan);
        p.send("hola", &b, "B");
        wait(&mut p);
        let args = lines(&dir.join("args.txt"));
        assert!(args.contains(&"WebSearch".to_string()) && !args.contains(&"--disallowedTools".to_string()), "Solo planificar can search too: {args:?}");
        let mut q = AgentPanel::with_config(Some(cfg.join("agent.toml")));
        q.enter_project(&a, "A");
        assert!(!q.internet, "remembered per project");
        q.enter_project(&b, "B");
        assert!(q.internet);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }
}
