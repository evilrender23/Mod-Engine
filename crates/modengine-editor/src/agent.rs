//! The coding-agent CLIs the Agente panel can drive: Claude Code (`claude`), Antigravity (`agy`)
//! and Codex (`codex`). Each one says how to run its CLI headless in the project folder and turns
//! its JSON output lines into the panel's [`Event`]s. Nothing here touches the UI or spawns
//! processes, so it is tested directly (`agent_panel.rs` runs the processes).
//!
//! Modelled on the Maker editor's `src/panels/agent/*Backend.cpp`, with the same flags.

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

/// What the agent may do; each CLI maps it onto its own modes. None of them bypasses approvals.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Permission {
    Plan,
    Edit,
    Auto,
}

impl Permission {
    pub const ALL: [Permission; 3] = [Permission::Plan, Permission::Edit, Permission::Auto];

    /// Remembered in `agent.toml`.
    pub fn key(self) -> &'static str {
        match self {
            Permission::Plan => "plan",
            Permission::Edit => "edit",
            Permission::Auto => "auto",
        }
    }

    pub fn from_key(k: &str) -> Option<Permission> {
        Permission::ALL.into_iter().find(|p| p.key() == k)
    }

    pub fn label(self) -> &'static str {
        match self {
            Permission::Plan => "Solo planificar",
            Permission::Edit => "Editar archivos",
            Permission::Auto => "Editar y ejecutar comandos",
        }
    }

    pub fn tip(self) -> &'static str {
        match self {
            Permission::Plan => "El agente lee el proyecto y propone cambios sin editar nada",
            Permission::Edit => "El agente edita archivos dentro de la carpeta del proyecto; fuera de ella se bloquea",
            Permission::Auto => "El agente también ejecuta los comandos que considera seguros; lo arriesgado sigue bloqueado",
        }
    }
}

/// The supported CLIs.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Kind {
    Claude,
    Antigravity,
    Codex,
}

/// A model the CLI can be asked to use.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Model {
    /// What the CLI takes after `--model`.
    pub id: String,
    /// Shown in the menu.
    pub name: String,
    #[serde(default)]
    pub description: String,
}

impl Model {
    fn new(id: &str, name: &str, description: &str) -> Model {
        Model { id: id.into(), name: name.into(), description: description.into() }
    }
}

/// Everything one run needs.
pub struct Run {
    /// As the user typed it.
    pub prompt: String,
    /// Empty for a new conversation.
    pub session: String,
    /// Empty: the one set up in the CLI itself.
    pub model: String,
    pub permission: Permission,
    pub root: PathBuf,
    /// [`engine_context`] written to a file.
    pub context_file: PathBuf,
    /// Folder with `lenguaje.md`, `funciones.md`, `proyecto.md`, when found.
    pub docs: Option<PathBuf>,
    /// Project name, for the context.
    pub name: String,
    /// What the user attached to this message.
    pub attachments: Vec<Attachment>,
}

/// How an attachment reaches the agent.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AttachKind {
    /// A file or folder: the agent opens project files itself; text from outside the project
    /// goes inside the message.
    File,
    /// A picture: handed to the CLIs that take images (Codex `--image`), named for the others.
    Image,
    /// Written into the message (the room selection, Salida lines).
    Text,
}

/// Something the user attached to the next message (modelled on Maker's `Attachment`).
#[derive(Clone, Debug, PartialEq)]
pub struct Attachment {
    pub kind: AttachKind,
    pub icon: &'static str,
    /// Shown in the panel and the transcript; names a Text attachment for the agent.
    pub label: String,
    /// Absolute path (File, Image).
    pub path: PathBuf,
    /// What it is, for the agent ("the object `Player`"); may be empty.
    pub about: String,
    /// The content of a Text attachment.
    pub text: String,
}

/// File names the agent is told to look at as pictures.
pub const IMAGE_EXTS: [&str; 6] = ["png", "jpg", "jpeg", "gif", "webp", "bmp"];

/// Most text one attachment puts in the message; the rest is cut with a note.
pub const INLINE_MAX: usize = 20 * 1024;

impl Attachment {
    /// A file or folder; pictures by their extension.
    pub fn file(path: &Path) -> Attachment {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let image = IMAGE_EXTS.contains(&ext.as_str());
        let icon = if path.is_dir() {
            "📁"
        } else if image {
            "🖼"
        } else {
            "📄"
        };
        Attachment {
            kind: if image { AttachKind::Image } else { AttachKind::File },
            icon,
            label: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string()),
            path,
            about: String::new(),
            text: String::new(),
        }
    }

    /// Text written into the message under `label`.
    pub fn text(icon: &'static str, label: impl Into<String>, text: impl Into<String>) -> Attachment {
        Attachment { kind: AttachKind::Text, icon, label: label.into(), path: PathBuf::new(), about: String::new(), text: text.into() }
    }

    /// Same thing (attaching it again replaces it).
    pub fn same(&self, other: &Attachment) -> bool {
        self.kind == other.kind && if self.kind == AttachKind::Text { self.label == other.label } else { self.path == other.path }
    }
}

/// `text` cut to `max` bytes (on a character boundary) with a note saying how much was left out.
pub fn clip(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut cut = max;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}\n[… truncated by the editor: {} more bytes not shown]", &text[..cut], text.len() - cut)
}

/// A text file small enough to read, clipped to [`INLINE_MAX`]; `None` for folders and binaries.
fn inline_text(path: &Path) -> Option<String> {
    use std::io::Read;
    let total = path.metadata().ok().filter(|m| m.is_file())?.len() as usize;
    let mut bytes = vec![];
    std::fs::File::open(path).ok()?.take(INLINE_MAX as u64 + 4).read_to_end(&mut bytes).ok()?;
    if bytes.contains(&0) {
        return None; // binary
    }
    let text = String::from_utf8_lossy(&bytes);
    if total <= INLINE_MAX {
        return Some(text.into_owned());
    }
    let mut cut = INLINE_MAX.min(text.len());
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    Some(format!("{}\n[… truncated by the editor: {} more bytes not shown]", &text[..cut], total.saturating_sub(cut)))
}

/// A Markdown code block that survives backticks inside `text`.
fn fence(text: &str, lang: &str) -> String {
    let mut marks = "```".to_string();
    while text.contains(&marks) {
        marks.push('`');
    }
    format!("{marks}{lang}\n{text}{}{marks}\n", if text.ends_with('\n') { "" } else { "\n" })
}

/// The message with a block listing what the user attached (paths relative to the project
/// when inside it, text inline).
pub fn prompt_with_attachments(run: &Run) -> String {
    if run.attachments.is_empty() {
        return run.prompt.clone();
    }
    let mut t = format!("{}\n\n<attached-by-the-user>\nWith this message the user attached:\n", run.prompt);
    for a in &run.attachments {
        let inside = inside(&run.root, &a.path);
        let shown = if inside { a.path.strip_prefix(&run.root).unwrap_or(&a.path).display().to_string() } else { a.path.display().to_string() };
        let about = if a.about.is_empty() { String::new() } else { format!(" ({})", a.about) };
        match a.kind {
            AttachKind::Image => t += &format!("- The image `{shown}`{about}. Look at it.\n"),
            AttachKind::File if a.path.is_dir() => {
                t += &if inside { format!("- The project folder `{shown}`.\n") } else { format!("- The folder `{shown}`, outside the project.\n") }
            }
            AttachKind::File if inside => t += &format!("- The project file `{shown}`{about}.\n"),
            AttachKind::File => match inline_text(&a.path) {
                Some(content) => {
                    let lang = a.path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                    t += &format!("- The file `{shown}`, from outside the project:\n");
                    t += &fence(&content, &lang);
                }
                None => t += &format!("- The file `{shown}`, from outside the project.\n"),
            },
            AttachKind::Text => {
                t += &format!("- {}{about}:\n", a.label);
                t += &fence(&clip(&a.text, INLINE_MAX), "");
            }
        }
    }
    t + "</attached-by-the-user>"
}

/// Attached files and folders outside the project: the agent is allowed to read just those.
pub fn attached_outside(run: &Run) -> Vec<&Path> {
    run.attachments.iter().filter(|a| a.kind != AttachKind::Text && !inside(&run.root, &a.path)).map(|a| a.path.as_path()).collect()
}

/// Attached pictures, for the CLIs that take images.
pub fn attached_images(run: &Run) -> Vec<&Path> {
    run.attachments.iter().filter(|a| a.kind == AttachKind::Image).map(|a| a.path.as_path()).collect()
}

/// A transcript line for something the agent did.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolLine {
    pub icon: &'static str,
    pub verb: String,
    pub detail: String,
}

/// What one output line means to the panel, whichever CLI wrote it.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The id that continues this conversation (empty: start a new one).
    Session(String),
    /// A streamed reply starts.
    TextBegin,
    /// More of it.
    TextDelta(String),
    /// The whole reply, or the end of the streamed one.
    Text(String),
    /// What the agent is doing.
    Tool(ToolLine),
    /// Progress for the status line.
    Status(String),
    /// What failed and why.
    ToolFailed(String),
    /// What the permission setting refused.
    Blocked(String),
    /// The model that is answering, as the CLI names it.
    Model(String),
    /// Absolute path of a file it wrote.
    FileChanged(PathBuf),
    /// A command ran and may have changed files.
    CommandRan,
    /// Worth showing, not an error.
    Notice(String),
    /// The turn completed.
    Finished,
    /// Why the turn failed.
    Failed(String),
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Claude, Kind::Antigravity, Kind::Codex];

    /// Remembered in `agent.toml`.
    pub fn id(self) -> &'static str {
        match self {
            Kind::Claude => "claude",
            Kind::Antigravity => "agy",
            Kind::Codex => "codex",
        }
    }

    pub fn from_id(id: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.id() == id)
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Claude => "Claude Code",
            Kind::Antigravity => "Antigravity",
            Kind::Codex => "Codex",
        }
    }

    /// Executable looked up on PATH.
    pub fn program(self) -> &'static str {
        self.id()
    }

    /// `(where, command)` pairs shown when the CLI is missing.
    pub fn install_commands(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Kind::Claude => &[
                ("macOS, Linux", "curl -fsSL https://claude.ai/install.sh | bash"),
                ("Windows PowerShell", "irm https://claude.ai/install.ps1 | iex"),
                ("Con Node.js", "npm install -g @anthropic-ai/claude-code"),
            ],
            Kind::Antigravity => &[
                ("macOS, Linux", "curl -fsSL https://antigravity.google/cli/install.sh | bash"),
                ("Windows PowerShell", "irm https://antigravity.google/cli/install.ps1 | iex"),
            ],
            Kind::Codex => &[("Con Node.js", "npm install -g @openai/codex"), ("macOS (Homebrew)", "brew install --cask codex")],
        }
    }

    /// Continues the conversation in a terminal opened in the project folder.
    pub fn resume_command(self, session: &str) -> String {
        match self {
            Kind::Claude => format!("claude --resume {session}"),
            Kind::Antigravity => format!("agy --conversation {session}"),
            Kind::Codex => format!("codex resume {session}"),
        }
    }

    /// The models offered without asking the CLI. Claude's are aliases that always pick the
    /// newest model of each family.
    pub fn builtin_models(self) -> Vec<Model> {
        match self {
            Kind::Claude => vec![
                Model::new("fable", "Fable", "El modelo Fable más reciente"),
                Model::new("opus", "Opus", "El modelo Opus más reciente"),
                Model::new("sonnet", "Sonnet", "El modelo Sonnet más reciente"),
                Model::new("haiku", "Haiku", "El modelo Haiku más reciente: el más rápido"),
            ],
            _ => vec![],
        }
    }

    /// Arguments that make the CLI print its own model list; empty when it cannot.
    pub fn list_models_args(self) -> &'static [&'static str] {
        match self {
            Kind::Claude => &[],
            Kind::Antigravity => &["models"],
            Kind::Codex => &["debug", "models"],
        }
    }

    /// Reads what [`Kind::list_models_args`] printed.
    pub fn parse_models(self, output: &str) -> Vec<Model> {
        match self {
            Kind::Claude => vec![],
            // `agy models` prints a line before the list, then one model per line: "id<TAB>name".
            Kind::Antigravity => output
                .lines()
                .filter_map(|l| {
                    let mut f = l.trim().split('\t');
                    let (id, name) = (f.next()?, f.next()?);
                    (!id.is_empty() && !id.contains(' ')).then(|| Model::new(id, name.trim(), ""))
                })
                .collect(),
            // The catalog the CLI shows in its own picker, in its order.
            Kind::Codex => {
                let v: Value = serde_json::from_str(output).unwrap_or(Value::Null);
                let mut listed: Vec<(i64, Model)> = v["models"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|m| m["visibility"].as_str() == Some("list"))
                    .map(|m| {
                        let id = str_of(&m["slug"]);
                        let name = str_of(&m["display_name"]);
                        let name = if name.is_empty() { id } else { name };
                        (m["priority"].as_i64().unwrap_or(0), Model::new(id, name, str_of(&m["description"])))
                    })
                    .collect();
                listed.sort_by_key(|(p, _)| *p);
                listed.into_iter().map(|(_, m)| m).collect()
            }
        }
    }

    /// Command-line arguments for one run.
    pub fn args(self, run: &Run) -> Vec<String> {
        let mut a: Vec<String> = vec![];
        match self {
            // `claude --print --output-format stream-json`, prompt on stdin, `--resume <session>`.
            Kind::Claude => {
                let mode = match run.permission {
                    Permission::Plan => "plan",
                    Permission::Edit => "acceptEdits",
                    Permission::Auto => "auto",
                };
                a.extend(
                    ["--print", "--output-format", "stream-json", "--verbose", "--include-partial-messages", "--permission-mode", mode, "--append-system-prompt-file"]
                        .map(String::from),
                );
                a.push(run.context_file.to_string_lossy().into());
                if !run.model.is_empty() {
                    a.extend(["--model".into(), run.model.clone()]);
                }
                if !run.session.is_empty() {
                    a.extend(["--resume".into(), run.session.clone()]);
                }
                // The engine docs and attachments from elsewhere live outside the project: allow
                // reading them, not editing them.
                let mut readable = vec![];
                if let Some(docs) = &run.docs {
                    readable.push(format!("Read({}/**)", claude_rule_path(docs)));
                }
                for p in attached_outside(run) {
                    readable.push(if p.is_dir() { format!("Read({}/**)", claude_rule_path(p)) } else { format!("Read({})", claude_rule_path(p)) });
                }
                if !readable.is_empty() {
                    a.push("--allowedTools".into());
                    a.extend(readable);
                }
            }
            // `agy --output-format stream-json --print=<prompt>`, `--conversation <id>` to continue.
            Kind::Antigravity => {
                // accept-edits approves edits anywhere, so writing outside the project needs asking,
                // and print mode refuses everything that asks.
                let mut allow = vec![format!("write_file({})", agy_rule_path(&run.root))];
                if let Some(docs) = &run.docs {
                    allow.push(format!("read_file({})", agy_rule_path(docs)));
                }
                for p in attached_outside(run) {
                    allow.push(format!("read_file({})", agy_rule_path(p)));
                }
                let settings = json!({ "permissions": { "ask": ["write_file(/)"], "allow": allow } });
                let mode = match run.permission {
                    Permission::Plan => "plan",
                    Permission::Edit => "accept-edits",
                    Permission::Auto => "auto",
                };
                a.extend(["--output-format", "stream-json", "--mode", mode, "--settings"].map(String::from));
                a.push(settings.to_string());
                if !run.model.is_empty() {
                    a.extend(["--model".into(), run.model.clone()]);
                }
                if !run.session.is_empty() {
                    a.extend(["--conversation".into(), run.session.clone()]);
                }
                // Print mode refuses every command outside Auto, and a refusal ends the whole turn,
                // so steer the agent to its file tools before it tries an innocent `ls`.
                let mut prompt = prompt_with_context(run);
                if run.permission != Permission::Auto {
                    prompt += "\n\n(From the Mod Engine editor: shell commands are not allowed in this mode and trying one \
                               ends your turn. Use your file tools to list, read, search and edit files.)";
                }
                // agy takes the prompt as the value of --print and reads nothing from stdin.
                a.push(format!("--print={prompt}"));
            }
            // `codex exec --json -` with the prompt on stdin, `codex exec ... resume <thread> -` to continue.
            Kind::Codex => {
                // The sandbox keeps writes inside the working folder; commands run inside it in every
                // mode, so Auto is the same as Edit. Projects are often not Git repositories, which
                // exec otherwise refuses to work in.
                let sandbox = if run.permission == Permission::Plan { "read-only" } else { "workspace-write" };
                a.extend(["exec", "--json", "--skip-git-repo-check", "--sandbox", sandbox].map(String::from));
                if !run.model.is_empty() {
                    a.extend(["--model".into(), run.model.clone()]);
                }
                if !run.session.is_empty() {
                    a.extend(["resume".into(), run.session.clone()]); // which takes its own --image
                }
                // One image per option: the option takes several values and would swallow the "-".
                for p in attached_images(run) {
                    a.push(format!("--image={}", p.display()));
                }
                a.push("-".into()); // prompt on stdin
            }
        }
        a
    }

    /// Written to stdin, which is then closed.
    pub fn input(self, run: &Run) -> String {
        match self {
            Kind::Claude => prompt_with_attachments(run),
            Kind::Antigravity => String::new(),
            Kind::Codex => prompt_with_context(run),
        }
    }
}

/// What the agent is told about the engine: a system prompt for Claude Code, and the head of
/// the first message for the CLIs without one.
pub fn engine_context(name: &str, docs: Option<&Path>) -> String {
    let mut c = format!(
        "You are working inside Mod Engine, a 2D game engine in the spirit of GameMaker, through the \
         \"Agente\" panel of its visual editor (modeditor). The current folder is the open Mod Engine project \
         \"{name}\". Help the user build their game by writing and editing the project's files. Keep every \
         change inside this folder.\n\n\
         # Project layout\n\
         - game.toml: game name, start room, room order, display settings and the shared pixel-art palette.\n\
         - objects/*.ms and scripts/*.ms: ModScript source. `object Name : Parent {{ ... }}` blocks define \
         objects (variables, `sprite = name`, and events such as `on create`, `on step`, `on draw`, \
         `on collision(Other)`); top-level functions and code go in scripts/.\n\
         - sprites/: a PNG (optionally with a .toml of the same name for frames, origin, fps, mask) or a .toml \
         with pixel art drawn as characters of the palette (`pixels = ['''...''']`, `.` is transparent).\n\
         - rooms/*.toml: size, background, camera and layers (tiles and instances).\n\
         - sounds/: .wav/.ogg files or .toml synthesized sounds. fonts/: .ttf/.otf.\n\
         Every asset is named by its file name without extension, and all assets share one namespace.\n\n"
    );
    match docs {
        Some(d) => c += &format!(
            "# Reference\n\
             The engine documentation is in {dir} (Spanish): lenguaje.md (the ModScript language), \
             funciones.md (every built-in function and instance variable) and proyecto.md (every file format). \
             Read the relevant one before using a function, event or setting you have not seen in this \
             project's files.\n",
            dir = d.display()
        ),
        None => c += "There is no reference documentation at hand; learn the language and file formats from the project's existing files.\n",
    }
    c += "Before writing a new object, room or sprite, read an existing one and follow its style. \
          `modengine check .` loads the project and reports errors (if you may run commands).\n\n\
          # The editor\n\
          The editor saves the user's open edits before each message and reloads the files you change, \
          then checks the project and shows engine errors in its Salida panel.\n\n\
          # Replies\n\
          Keep replies short and plain (no tables): say what you changed and how to try it (which room, \
          which keys to press). Reply in the user's language.\n";
    c
}

/// The prompt with the engine context in front on a new conversation, for CLIs without a
/// system prompt option.
pub fn prompt_with_context(run: &Run) -> String {
    if !run.session.is_empty() {
        return prompt_with_attachments(run);
    }
    format!("<modengine-editor-context>\n{}</modengine-editor-context>\n\n{}", engine_context(&run.name, run.docs.as_deref()), prompt_with_attachments(run))
}

/// Claude permission rules write absolute paths as "//path".
fn claude_rule_path(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    let s = match s.as_bytes() {
        [d, b':', ..] => format!("/{}{}", (*d as char).to_ascii_lowercase(), &s[2..]),
        _ => s,
    };
    format!("/{}", s.trim_end_matches('/'))
}

/// Antigravity rules take forward slashes and no drive letter; a directory covers everything inside it.
fn agy_rule_path(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    let s = match s.as_bytes() {
        [_, b':', ..] => s[2..].to_string(),
        _ => s,
    };
    s.trim_end_matches('/').to_string()
}

fn str_of(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

/// First line of `text`, cut to `max` characters, with "…" when something was left out.
pub fn one_line(text: &str, max: usize) -> String {
    let t = text.trim();
    let first = t.lines().next().unwrap_or("");
    let cut = first.chars().count() > max || first.len() < t.len();
    let line: String = first.chars().take(max).collect();
    if cut {
        line + "…"
    } else {
        line
    }
}

/// `path` relative to the project when inside it.
pub fn display_path(root: &Path, path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    let abs = absolute(root, path);
    abs.strip_prefix(root).unwrap_or(&abs).display().to_string()
}

/// `path` made absolute against the project folder and cleaned of `.`/`..`.
pub fn absolute(root: &Path, path: &str) -> PathBuf {
    let p = Path::new(path);
    let joined = if p.is_absolute() { p.to_path_buf() } else { root.join(p) };
    let mut out = PathBuf::new();
    for c in joined.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}

pub fn inside(root: &Path, path: &Path) -> bool {
    path.starts_with(root)
}

fn tool(icon: &'static str, verb: &str, detail: impl Into<String>) -> ToolLine {
    ToolLine { icon, verb: verb.into(), detail: detail.into() }
}

/// For tools no backend knows: "browser_click" with `{ "Selector": "#ok" }` reads "Browser click #ok".
fn generic_tool(name: &str, input: &Value) -> ToolLine {
    let mut verb = name.replace('_', " ");
    if let Some(f) = verb.get(0..1) {
        verb = f.to_uppercase() + &verb[1..];
    }
    let detail = input.as_object().and_then(|o| o.values().find_map(|v| v.as_str())).map(|s| one_line(s, 120)).unwrap_or_default();
    tool("⚙", &verb, detail)
}

/// Turns the output lines of one CLI into events; keeps what a run needs to remember.
pub struct Parser {
    kind: Kind,
    root: PathBuf,
    // Claude
    session: String,
    model: String,
    text_open: bool,
    tool_names: HashMap<String, String>,
    edit_targets: HashMap<String, PathBuf>,
    denied: HashSet<String>,
    // Codex
    shown: HashSet<String>,
    // Antigravity
    text_step: Option<i64>,
    text: String,
    shown_steps: HashSet<i64>,
    step_blocked: bool,
}

impl Parser {
    pub fn new(kind: Kind, root: &Path) -> Parser {
        Parser {
            kind,
            root: root.to_path_buf(),
            session: String::new(),
            model: String::new(),
            text_open: false,
            tool_names: HashMap::new(),
            edit_targets: HashMap::new(),
            denied: HashSet::new(),
            shown: HashSet::new(),
            text_step: None,
            text: String::new(),
            shown_steps: HashSet::new(),
            step_blocked: false,
        }
    }

    /// One line of output (a JSON object).
    pub fn parse(&mut self, line: &Value) -> Vec<Event> {
        let mut ev = vec![];
        match self.kind {
            Kind::Claude => self.claude(line, &mut ev),
            Kind::Codex => self.codex(line, &mut ev),
            Kind::Antigravity => self.agy(line, &mut ev),
        }
        ev
    }

    fn abs(&self, p: &str) -> PathBuf {
        absolute(&self.root, p)
    }

    fn file_tool(&self, icon: &'static str, verb: &str, path: &str) -> ToolLine {
        tool(icon, verb, display_path(&self.root, path))
    }

    // ------------------------------------------------------------ Claude Code

    fn claude(&mut self, line: &Value, ev: &mut Vec<Event>) {
        let session = str_of(&line["session_id"]);
        if !session.is_empty() && session != self.session {
            self.session = session.into();
            ev.push(Event::Session(session.into()));
        }
        // Subagents work out of sight; their file changes still show through their tool calls.
        let nested = !str_of(&line["parent_tool_use_id"]).is_empty();
        match str_of(&line["type"]) {
            "stream_event" if !nested => self.claude_stream(&line["event"], ev),
            "assistant" => self.claude_assistant(line, nested, ev),
            "user" => self.claude_results(line, ev),
            "result" => self.claude_result(line, ev),
            "system" => match str_of(&line["subtype"]) {
                "permission_denied" => {
                    self.denied.insert(str_of(&line["tool_use_id"]).into());
                    // Plan mode ends by asking to leave it, which print mode always refuses.
                    let t = str_of(&line["tool_name"]);
                    if t != "ExitPlanMode" {
                        ev.push(Event::Blocked(t.into()));
                    }
                }
                "compact_boundary" => ev.push(Event::Notice("Los mensajes anteriores se resumieron para hacer sitio.".into())),
                _ => {}
            },
            _ => {}
        }
    }

    fn claude_model(&mut self, model: &str, ev: &mut Vec<Event>) {
        if !model.is_empty() && model != self.model {
            self.model = model.into();
            ev.push(Event::Model(model.into()));
        }
    }

    fn claude_stream(&mut self, s: &Value, ev: &mut Vec<Event>) {
        match str_of(&s["type"]) {
            "message_start" => self.claude_model(str_of(&s["message"]["model"]), ev),
            "content_block_start" => match str_of(&s["content_block"]["type"]) {
                "text" => {
                    self.text_open = true;
                    ev.push(Event::TextBegin);
                    ev.push(Event::Status("Escribiendo…".into()));
                }
                "thinking" => ev.push(Event::Status("Pensando…".into())),
                _ => {}
            },
            "content_block_delta" if self.text_open => {
                if str_of(&s["delta"]["type"]) == "text_delta" {
                    ev.push(Event::TextDelta(str_of(&s["delta"]["text"]).into()));
                }
            }
            _ => {}
        }
    }

    fn claude_assistant(&mut self, line: &Value, nested: bool, ev: &mut Vec<Event>) {
        if !nested {
            self.claude_model(str_of(&line["message"]["model"]), ev);
        }
        for b in line["message"]["content"].as_array().into_iter().flatten() {
            match str_of(&b["type"]) {
                "text" if !nested => {
                    self.text_open = false;
                    ev.push(Event::Text(str_of(&b["text"]).into()));
                }
                "tool_use" => {
                    let id = str_of(&b["id"]).to_string();
                    let name = str_of(&b["name"]).to_string();
                    let input = &b["input"];
                    let file = str_of(&input[if name == "NotebookEdit" { "notebook_path" } else { "file_path" }]);
                    if matches!(name.as_str(), "Write" | "Edit" | "MultiEdit" | "NotebookEdit") {
                        self.edit_targets.insert(id.clone(), self.abs(file));
                    }
                    let t = |k: &str| one_line(str_of(&input[k]), 120);
                    let line = match name.as_str() {
                        "Read" => tool("👁", "Leer", display_path(&self.root, file)),
                        "Write" => self.file_tool("📄", "Escribir", file),
                        "Edit" | "MultiEdit" | "NotebookEdit" => self.file_tool("✎", "Editar", file),
                        "Glob" => tool("🔍", "Buscar archivos", t("pattern")),
                        "Grep" => tool("🔍", "Buscar", t("pattern")),
                        "Bash" | "PowerShell" => tool(">_", "Ejecutar", t("command")),
                        "WebFetch" => tool("🌐", "Descargar", t("url")),
                        "WebSearch" => tool("🌐", "Buscar en la web", t("query")),
                        "TodoWrite" => tool("☰", "Actualizar plan", ""),
                        "ExitPlanMode" => tool("☰", "Plan listo", "elige «Editar archivos» para aplicarlo"),
                        "Task" | "Agent" => tool("…", "Agente", t("description")),
                        _ => tool("⚙", &name, ""),
                    };
                    self.tool_names.insert(id, name);
                    ev.push(Event::Tool(line));
                }
                _ => {}
            }
        }
    }

    fn claude_results(&mut self, line: &Value, ev: &mut Vec<Event>) {
        for b in line["message"]["content"].as_array().into_iter().flatten() {
            if str_of(&b["type"]) != "tool_result" {
                continue;
            }
            let id = str_of(&b["tool_use_id"]);
            let name = self.tool_names.get(id).cloned().unwrap_or_default();
            if b["is_error"].as_bool() == Some(true) {
                if !self.denied.contains(id) {
                    let text = match &b["content"] {
                        Value::String(s) => s.clone(),
                        Value::Array(parts) => parts.iter().filter(|p| p["type"] == "text").map(|p| format!("{}\n", str_of(&p["text"]))).collect(),
                        _ => String::new(),
                    };
                    let text = text.replace("<tool_use_error>", "").replace("</tool_use_error>", "");
                    ev.push(Event::ToolFailed(format!("{name} falló: {}", one_line(&text, 120))));
                }
            } else if let Some(p) = self.edit_targets.get(id) {
                ev.push(Event::FileChanged(p.clone()));
            } else if name == "Bash" || name == "PowerShell" {
                ev.push(Event::CommandRan);
            }
        }
    }

    fn claude_result(&mut self, line: &Value, ev: &mut Vec<Event>) {
        if line["is_error"].as_bool() != Some(true) && str_of(&line["subtype"]) == "success" {
            ev.push(Event::Finished);
            return;
        }
        let mut msg = str_of(&line["result"]).to_string();
        if msg.is_empty() {
            let errors: Vec<&str> = line["errors"].as_array().into_iter().flatten().filter_map(|e| e.as_str()).collect();
            msg = if errors.is_empty() { str_of(&line["subtype"]).into() } else { errors.join("\n") };
        }
        if msg.contains("/login") {
            msg += "\nInicia sesión ejecutando `claude` una vez en un terminal.";
        }
        let lost = msg.contains("No conversation found");
        if lost {
            msg += "\nEl siguiente mensaje empieza una conversación nueva.";
        }
        ev.push(Event::Failed(msg));
        if lost {
            ev.push(Event::Session(String::new()));
        }
    }

    // ------------------------------------------------------------ Codex

    fn codex(&mut self, line: &Value, ev: &mut Vec<Event>) {
        match str_of(&line["type"]) {
            "thread.started" => ev.push(Event::Session(str_of(&line["thread_id"]).into())),
            "turn.started" => ev.push(Event::Status("Trabajando…".into())),
            t @ ("item.started" | "item.updated" | "item.completed") => self.codex_item(&line["item"], t == "item.completed", ev),
            "turn.completed" => ev.push(Event::Finished),
            "turn.failed" => ev.push(Event::Failed(str_of(&line["error"]["message"]).into())),
            "error" => ev.push(Event::ToolFailed(str_of(&line["message"]).into())),
            _ => {}
        }
    }

    /// Items arrive started, maybe updated, then completed; the first sighting adds the transcript line.
    fn codex_item(&mut self, item: &Value, completed: bool, ev: &mut Vec<Event>) {
        let status = str_of(&item["status"]);
        let first = self.shown.insert(str_of(&item["id"]).to_string());
        match str_of(&item["type"]) {
            "agent_message" => {
                if completed {
                    ev.push(Event::Text(str_of(&item["text"]).into()));
                }
            }
            "reasoning" => ev.push(Event::Status("Pensando…".into())),
            "command_execution" => {
                let cmd = one_line(str_of(&item["command"]), 120);
                if first {
                    ev.push(Event::Tool(tool(">_", "Ejecutar", cmd.clone())));
                }
                if !completed {
                    return;
                }
                if status == "declined" {
                    ev.push(Event::Blocked(cmd));
                } else {
                    if status == "failed" {
                        ev.push(Event::ToolFailed(format!("La orden falló con código {}", item["exit_code"].as_i64().unwrap_or(0))));
                    }
                    ev.push(Event::CommandRan);
                }
            }
            "file_change" => {
                for c in item["changes"].as_array().into_iter().flatten() {
                    let path = str_of(&c["path"]);
                    if first {
                        ev.push(Event::Tool(match str_of(&c["kind"]) {
                            "add" => self.file_tool("📄", "Escribir", path),
                            "delete" => self.file_tool("🗑", "Borrar", path),
                            _ => self.file_tool("✎", "Editar", path),
                        }));
                    }
                    if completed && status == "completed" {
                        ev.push(Event::FileChanged(self.abs(path)));
                    }
                }
                if completed && status == "failed" {
                    ev.push(Event::ToolFailed("El cambio de archivo falló".into()));
                }
            }
            "mcp_tool_call" => {
                if first {
                    ev.push(Event::Tool(generic_tool(&format!("{}_{}", str_of(&item["server"]), str_of(&item["tool"])), &Value::Null)));
                }
                if completed && status == "failed" {
                    ev.push(Event::ToolFailed(str_of(&item["error"]["message"]).into()));
                }
            }
            "web_search" => {
                if first {
                    ev.push(Event::Tool(tool("🌐", "Buscar en la web", one_line(str_of(&item["query"]), 120))));
                }
            }
            "todo_list" => {
                if first {
                    ev.push(Event::Tool(tool("☰", "Actualizar plan", "")));
                }
            }
            "error" => ev.push(Event::ToolFailed(str_of(&item["message"]).into())),
            _ => {}
        }
    }

    // ------------------------------------------------------------ Antigravity

    fn agy(&mut self, line: &Value, ev: &mut Vec<Event>) {
        match str_of(&line["event"]) {
            "init" => {
                ev.push(Event::Session(str_of(&line["conversation_id"]).into()));
                let m = str_of(&line["init"]["model"]);
                if !m.is_empty() {
                    ev.push(Event::Model(m.into()));
                }
            }
            "step_update" => self.agy_step(&line["step_update"], ev),
            "result" => self.agy_result(&line["result"], ev),
            _ => {}
        }
    }

    fn agy_path(p: &Value) -> &str {
        ["Path", "AbsolutePath", "TargetFile", "DirectoryPath"].iter().map(|k| str_of(&p[*k])).find(|s| !s.is_empty()).unwrap_or("")
    }

    fn agy_step(&mut self, step: &Value, ev: &mut Vec<Event>) {
        let index = step["step_index"].as_i64().unwrap_or(0);
        let state = str_of(&step["state"]);
        match str_of(&step["step_type"]) {
            "agent_response" => {
                // One step per reply; its text arrives in pieces and the step ends DONE.
                let delta = str_of(&step["text_delta"]);
                if !delta.is_empty() {
                    if self.text_step != Some(index) {
                        self.text_step = Some(index);
                        self.text.clear();
                        ev.push(Event::TextBegin);
                        ev.push(Event::Status("Escribiendo…".into()));
                    }
                    self.text += delta;
                    ev.push(Event::TextDelta(delta.into()));
                } else if state == "ACTIVE" {
                    ev.push(Event::Status("Pensando…".into()));
                }
                if state != "ACTIVE" && self.text_step == Some(index) {
                    ev.push(Event::Text(std::mem::take(&mut self.text)));
                    self.text_step = None;
                }
            }
            "tool" => {
                let info = &step["tool_info"];
                let name = match str_of(&info["name"]) {
                    "" => str_of(&step["tool_name"]),
                    n => n,
                }
                .to_string();
                let params = &info["parameters"];
                let path = Self::agy_path(params);
                let line = match name.as_str() {
                    "read_file" => tool("👁", "Leer", display_path(&self.root, path)),
                    "write_file" => self.file_tool("📄", "Escribir", path),
                    "edit_file" => self.file_tool("✎", "Editar", path),
                    "list_dir" => tool("📁", "Listar", display_path(&self.root, path)),
                    "run_command" => tool(">_", "Ejecutar", one_line(str_of(&params["CommandLine"]), 120)),
                    _ => generic_tool(&name, params),
                };
                if self.shown_steps.insert(index) {
                    ev.push(Event::Tool(line.clone()));
                }
                if state == "ERROR" {
                    let msg = str_of(&info["error"]["message"]);
                    if msg.to_lowercase().contains("permission") {
                        self.step_blocked = true;
                        ev.push(Event::Blocked(format!("{} {}", line.verb, line.detail).trim().into()));
                    } else {
                        ev.push(Event::ToolFailed(format!("{} falló: {}", line.verb, one_line(msg, 120))));
                    }
                } else if state == "DONE" {
                    // A write refused by an ask rule also ends DONE; the panel only counts project files.
                    if name == "write_file" || name == "edit_file" {
                        ev.push(Event::FileChanged(self.abs(path)));
                    } else if name == "run_command" {
                        ev.push(Event::CommandRan);
                    }
                }
            }
            _ => {}
        }
    }

    fn agy_result(&mut self, r: &Value, ev: &mut Vec<Event>) {
        // Refusals that never reached a tool step (an ask rule in print mode) are only listed here.
        if !self.step_blocked {
            for d in r["denied_actions"].as_array().into_iter().flatten() {
                let s = match str_of(&d["display_name"]) {
                    "" => str_of(&d["action"]),
                    n => n,
                };
                ev.push(Event::Blocked(s.into()));
            }
        }
        let status = str_of(&r["status"]);
        if status == "SUCCESS" {
            ev.push(Event::Finished);
        } else {
            let resp = str_of(&r["response"]);
            ev.push(Event::Failed(if resp.is_empty() { format!("Antigravity se detuvo: {status}") } else { resp.into() }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(permission: Permission, session: &str, model: &str) -> Run {
        Run {
            prompt: "haz algo".into(),
            session: session.into(),
            model: model.into(),
            permission,
            root: "/p/game".into(),
            context_file: "/tmp/ctx.md".into(),
            docs: Some("/opt/me/docs".into()),
            name: "Juego".into(),
            attachments: vec![],
        }
    }

    fn parse_all(kind: Kind, lines: &[&str]) -> Vec<Event> {
        let mut p = Parser::new(kind, Path::new("/p/game"));
        lines.iter().flat_map(|l| p.parse(&serde_json::from_str(l).unwrap())).collect()
    }

    #[test]
    fn claude_args_per_mode() {
        let a = Kind::Claude.args(&run(Permission::Edit, "", ""));
        assert_eq!(
            a,
            [
                "--print", "--output-format", "stream-json", "--verbose", "--include-partial-messages", "--permission-mode", "acceptEdits",
                "--append-system-prompt-file", "/tmp/ctx.md", "--allowedTools", "Read(//opt/me/docs/**)"
            ]
        );
        let a = Kind::Claude.args(&run(Permission::Plan, "S1", "opus"));
        assert!(a.windows(2).any(|w| w == ["--permission-mode", "plan"]));
        assert!(a.windows(2).any(|w| w == ["--model", "opus"]));
        assert!(a.windows(2).any(|w| w == ["--resume", "S1"]));
        assert!(Kind::Claude.args(&run(Permission::Auto, "", "")).windows(2).any(|w| w == ["--permission-mode", "auto"]));
        // The prompt goes on stdin, the context in the system prompt file.
        assert_eq!(Kind::Claude.input(&run(Permission::Auto, "", "")), "haz algo");
    }

    #[test]
    fn codex_args_per_mode() {
        assert_eq!(Kind::Codex.args(&run(Permission::Plan, "", "")), ["exec", "--json", "--skip-git-repo-check", "--sandbox", "read-only", "-"]);
        assert_eq!(
            Kind::Codex.args(&run(Permission::Auto, "T9", "gpt-x")),
            ["exec", "--json", "--skip-git-repo-check", "--sandbox", "workspace-write", "--model", "gpt-x", "resume", "T9", "-"]
        );
        // New conversations get the engine context in front; continued ones don't.
        assert!(Kind::Codex.input(&run(Permission::Edit, "", "")).starts_with("<modengine-editor-context>"));
        assert_eq!(Kind::Codex.input(&run(Permission::Edit, "T9", "")), "haz algo");
    }

    #[test]
    fn agy_args_per_mode() {
        let a = Kind::Antigravity.args(&run(Permission::Edit, "C1", "gemini"));
        assert_eq!(&a[..4], ["--output-format", "stream-json", "--mode", "accept-edits"]);
        assert_eq!(a[4], "--settings");
        let s: Value = serde_json::from_str(&a[5]).unwrap();
        assert_eq!(s["permissions"]["ask"], json!(["write_file(/)"]));
        assert_eq!(s["permissions"]["allow"], json!(["write_file(/p/game)", "read_file(/opt/me/docs)"]));
        assert!(a.windows(2).any(|w| w == ["--model", "gemini"]));
        assert!(a.windows(2).any(|w| w == ["--conversation", "C1"]));
        let last = a.last().unwrap();
        assert!(last.starts_with("--print=haz algo") && last.contains("shell commands are not allowed"));
        let a = Kind::Antigravity.args(&run(Permission::Auto, "", ""));
        assert!(a.windows(2).any(|w| w == ["--mode", "auto"]));
        let last = a.last().unwrap();
        assert!(last.starts_with("--print=<modengine-editor-context>") && !last.contains("shell commands are not allowed"));
        assert!(Kind::Antigravity.args(&run(Permission::Plan, "", "")).windows(2).any(|w| w == ["--mode", "plan"]));
        assert_eq!(Kind::Antigravity.input(&run(Permission::Auto, "", "")), "");
    }

    #[test]
    fn parses_model_lists() {
        let codex = r#"{"models":[{"slug":"b","display_name":"B","priority":2,"visibility":"list"},
            {"slug":"hidden","priority":0,"visibility":"hide"},{"slug":"a","display_name":"","description":"d","priority":1,"visibility":"list"}]}"#;
        assert_eq!(Kind::Codex.parse_models(codex), vec![Model::new("a", "a", "d"), Model::new("b", "B", "")]);
        let agy = "Fetching available models...\ngemini-3\tGemini 3 (High)\nclaude-x\tClaude X\n";
        assert_eq!(Kind::Antigravity.parse_models(agy), vec![Model::new("gemini-3", "Gemini 3 (High)", ""), Model::new("claude-x", "Claude X", "")]);
        assert_eq!(Kind::Claude.builtin_models().iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["fable", "opus", "sonnet", "haiku"]);
    }

    #[test]
    fn claude_stream() {
        let ev = parse_all(
            Kind::Claude,
            &[
                r#"{"type":"system","subtype":"init","session_id":"S1"}"#,
                r#"{"type":"stream_event","session_id":"S1","event":{"type":"message_start","message":{"model":"claude-opus"}}}"#,
                r#"{"type":"stream_event","session_id":"S1","event":{"type":"content_block_start","content_block":{"type":"text"}}}"#,
                r#"{"type":"stream_event","session_id":"S1","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Hola"}}}"#,
                r#"{"type":"assistant","session_id":"S1","message":{"model":"claude-opus","content":[{"type":"text","text":"Hola"},
                    {"type":"tool_use","id":"t1","name":"Edit","input":{"file_path":"/p/game/objects/player.ms"}},
                    {"type":"tool_use","id":"t2","name":"Bash","input":{"command":"modengine check ."}}]}}"#,
                r#"{"type":"user","session_id":"S1","message":{"content":[{"type":"tool_result","tool_use_id":"t1"},{"type":"tool_result","tool_use_id":"t2"}]}}"#,
                r#"{"type":"result","subtype":"success","session_id":"S1","result":"Hola"}"#,
            ],
        );
        assert_eq!(
            ev,
            vec![
                Event::Session("S1".into()),
                Event::Model("claude-opus".into()),
                Event::TextBegin,
                Event::Status("Escribiendo…".into()),
                Event::TextDelta("Hola".into()),
                Event::Text("Hola".into()),
                Event::Tool(tool("✎", "Editar", "objects/player.ms")),
                Event::Tool(tool(">_", "Ejecutar", "modengine check .")),
                Event::FileChanged("/p/game/objects/player.ms".into()),
                Event::CommandRan,
                Event::Finished,
            ]
        );
    }

    #[test]
    fn claude_errors() {
        let ev = parse_all(
            Kind::Claude,
            &[
                r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Write","input":{"file_path":"../x.ms"}}]}}"#,
                r#"{"type":"system","subtype":"permission_denied","tool_use_id":"t1","tool_name":"Write"}"#,
                r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"no"}]}}"#,
                r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"No conversation found with session ID x"}"#,
            ],
        );
        assert_eq!(ev[0], Event::Tool(tool("📄", "Escribir", "/p/x.ms")));
        assert_eq!(ev[1], Event::Blocked("Write".into()));
        assert!(matches!(&ev[2], Event::Failed(m) if m.contains("conversación nueva")));
        assert_eq!(ev[3], Event::Session(String::new()));
        assert_eq!(ev.len(), 4);
    }

    #[test]
    fn codex_stream() {
        let ev = parse_all(
            Kind::Codex,
            &[
                r#"{"type":"thread.started","thread_id":"T1"}"#,
                r#"{"type":"item.started","item":{"id":"i1","type":"command_execution","command":"ls","status":"in_progress"}}"#,
                r#"{"type":"item.completed","item":{"id":"i1","type":"command_execution","command":"ls","status":"completed","exit_code":0}}"#,
                r#"{"type":"item.completed","item":{"id":"i2","type":"file_change","status":"completed","changes":[{"path":"rooms/a.toml","kind":"update"}]}}"#,
                r#"{"type":"item.completed","item":{"id":"i3","type":"agent_message","text":"Listo"}}"#,
                r#"{"type":"turn.completed"}"#,
            ],
        );
        assert_eq!(
            ev,
            vec![
                Event::Session("T1".into()),
                Event::Tool(tool(">_", "Ejecutar", "ls")),
                Event::CommandRan,
                Event::Tool(tool("✎", "Editar", "rooms/a.toml")),
                Event::FileChanged("/p/game/rooms/a.toml".into()),
                Event::Text("Listo".into()),
                Event::Finished,
            ]
        );
    }

    #[test]
    fn agy_stream() {
        let ev = parse_all(
            Kind::Antigravity,
            &[
                r#"{"event":"init","conversation_id":"C1","init":{"model":"gemini"}}"#,
                r#"{"event":"step_update","step_update":{"step_index":1,"step_type":"tool","state":"ACTIVE","tool_info":{"name":"edit_file","parameters":{"TargetFile":"/p/game/game.toml"}}}}"#,
                r#"{"event":"step_update","step_update":{"step_index":1,"step_type":"tool","state":"DONE","tool_info":{"name":"edit_file","parameters":{"TargetFile":"/p/game/game.toml"}}}}"#,
                r#"{"event":"step_update","step_update":{"step_index":2,"step_type":"agent_response","state":"ACTIVE","text_delta":"Ho"}}"#,
                r#"{"event":"step_update","step_update":{"step_index":2,"step_type":"agent_response","state":"ACTIVE","text_delta":"la"}}"#,
                r#"{"event":"step_update","step_update":{"step_index":2,"step_type":"agent_response","state":"DONE"}}"#,
                r#"{"event":"result","result":{"status":"SUCCESS"}}"#,
            ],
        );
        assert_eq!(
            ev,
            vec![
                Event::Session("C1".into()),
                Event::Model("gemini".into()),
                Event::Tool(tool("✎", "Editar", "game.toml")),
                Event::FileChanged("/p/game/game.toml".into()),
                Event::TextBegin,
                Event::Status("Escribiendo…".into()),
                Event::TextDelta("Ho".into()),
                Event::TextDelta("la".into()),
                Event::Text("Hola".into()),
                Event::Finished,
            ]
        );
    }

    /// A project folder and a folder outside it, with a few files.
    fn attach_dirs(tag: &str) -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("modeditor_attach_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (game, other) = (base.join("game"), base.join("other"));
        std::fs::create_dir_all(game.join("objects")).unwrap();
        std::fs::create_dir_all(game.join("sprites")).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(game.join("objects/player.ms"), "object Player {}\n").unwrap();
        std::fs::write(game.join("sprites/hero.png"), [0x89u8, b'P', b'N', b'G', 0, 1]).unwrap();
        std::fs::write(other.join("notes.md"), "# Ideas\nuse ```code```\n").unwrap();
        std::fs::write(other.join("big.txt"), "é".repeat(INLINE_MAX)).unwrap(); // 2 bytes each
        std::fs::write(other.join("blob.bin"), [1u8, 0, 2]).unwrap();
        std::fs::write(other.join("shot.png"), [0x89u8, b'P', b'N', b'G', 0]).unwrap();
        (game.canonicalize().unwrap(), other.canonicalize().unwrap())
    }

    fn run_with(root: &Path, attachments: Vec<Attachment>) -> Run {
        Run { root: root.to_path_buf(), attachments, ..run(Permission::Edit, "", "") }
    }

    #[test]
    fn prompt_lists_attachments() {
        let (game, other) = attach_dirs("prompt");
        let mut player = Attachment::file(&game.join("objects/player.ms"));
        player.about = "the object `Player`".into();
        let r = run_with(
            &game,
            vec![
                player,
                Attachment::file(&game.join("sprites/hero.png")),
                Attachment::file(&other.join("notes.md")),
                Attachment::file(&other.join("big.txt")),
                Attachment::file(&other.join("blob.bin")),
                Attachment::file(&other),
                Attachment::text("☰", "Salida", "error: x\n".repeat(5000)),
            ],
        );
        let p = prompt_with_attachments(&r);
        assert!(p.starts_with("haz algo\n\n<attached-by-the-user>\n") && p.ends_with("</attached-by-the-user>"), "{p}");
        // Project files by their relative path; the agent opens them itself.
        assert!(p.contains("- The project file `objects/player.ms` (the object `Player`).\n"), "{p}");
        assert!(!p.contains("object Player {}"));
        assert!(p.contains("- The image `sprites/hero.png`. Look at it.\n"));
        // Text from outside the project goes inline, in a fence longer than its own backticks.
        let o = other.display();
        assert!(p.contains(&format!("- The file `{o}/notes.md`, from outside the project:\n````md\n# Ideas\nuse ```code```\n````\n")), "{p}");
        // Big text is clipped with a note; binaries and folders are only named.
        assert!(p.contains(&format!("[… truncated by the editor: {} more bytes not shown]", INLINE_MAX)), "{p}");
        assert!(p.contains(&format!("- The file `{o}/blob.bin`, from outside the project.\n")));
        assert!(p.contains(&format!("- The folder `{o}`, outside the project.\n")));
        assert!(p.contains("- Salida:\n```\nerror: x\n"));
        assert!(p.len() < 3 * INLINE_MAX + 2000, "{}", p.len());
        // Nothing attached: the message as typed.
        assert_eq!(prompt_with_attachments(&run_with(&game, vec![])), "haz algo");
        let _ = std::fs::remove_dir_all(game.parent().unwrap());
    }

    #[test]
    fn clip_respects_char_boundaries() {
        assert_eq!(clip("hola", 10), "hola");
        assert_eq!(clip("ééé", 3), "é\n[… truncated by the editor: 4 more bytes not shown]");
    }

    #[test]
    fn attachments_reach_each_cli() {
        let (game, other) = attach_dirs("argv");
        let atts = vec![
            Attachment::file(&game.join("sprites/hero.png")),
            Attachment::file(&other.join("shot.png")),
            Attachment::file(&other.join("notes.md")),
            Attachment::file(&other),
            Attachment::text("⬚", "Selección", "x = 1"),
        ];
        let r = run_with(&game, atts);
        let o = other.display().to_string();
        // Claude: on stdin, and permission to read only those files outside the project.
        let a = Kind::Claude.args(&r);
        let at = a.iter().position(|x| x == "--allowedTools").unwrap();
        assert_eq!(
            a[at + 1..],
            ["Read(//opt/me/docs/**)".to_string(), format!("Read(/{o}/shot.png)"), format!("Read(/{o}/notes.md)"), format!("Read(/{o}/**)")]
        );
        let input = Kind::Claude.input(&r);
        assert!(input.contains("<attached-by-the-user>") && input.contains("- Selección:\n```\nx = 1\n```"));
        // Codex: every image with --image=, before the "-" that reads stdin, also when resuming.
        let a = Kind::Codex.args(&r);
        assert_eq!(a[a.len() - 3..], [format!("--image={}/sprites/hero.png", game.display()), format!("--image={o}/shot.png"), "-".to_string()]);
        let a = Kind::Codex.args(&Run { session: "T9".into(), ..run_with(&game, r.attachments.clone()) });
        assert_eq!(a[a.len() - 5..a.len() - 2], ["resume".to_string(), "T9".into(), format!("--image={}/sprites/hero.png", game.display())]);
        assert!(Kind::Codex.input(&r).contains("Look at it."));
        // Antigravity: read rules for what is outside the project, the list in --print.
        let a = Kind::Antigravity.args(&r);
        let s: Value = serde_json::from_str(&a[5]).unwrap();
        assert_eq!(
            s["permissions"]["allow"],
            json!([format!("write_file({})", game.display()), "read_file(/opt/me/docs)", format!("read_file({o}/shot.png)"), format!("read_file({o}/notes.md)"), format!("read_file({o})")])
        );
        assert!(a.last().unwrap().contains("<attached-by-the-user>"));
        // Without attachments nothing changes.
        assert!(!Kind::Codex.args(&run_with(&game, vec![])).iter().any(|x| x.starts_with("--image")));
        let _ = std::fs::remove_dir_all(game.parent().unwrap());
    }

    #[test]
    fn context_points_at_docs() {
        let c = engine_context("Coin Climber", Some(Path::new("/opt/me/docs")));
        assert!(c.contains("\"Coin Climber\"") && c.contains("/opt/me/docs") && c.contains("funciones.md"));
        assert!(engine_context("x", None).contains("no reference documentation"));
    }
}
