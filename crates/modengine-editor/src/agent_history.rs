//! Conversations of the Agente panel kept on disk, per project:
//! `~/.config/modengine/agent/history/<project>-<hash>/<id>.json` (title, agent, model, CLI
//! session id, timestamps, transcript with tool lines and attachments, turns, usage) plus
//! `state.json` (the open conversation and the panel settings of that project).
//!
//! Also the transcript types themselves ([`Entry`], [`Turn`]) and Markdown export.

use crate::agent::{self, Attachment, ToolLine};
use crate::agent_snapshot::FileChange;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub enum Tone {
    Muted,
    Warning,
    Error,
}

/// One block of the transcript.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Entry {
    User(String),
    /// What went with the user message before it.
    Attached(Vec<Attachment>),
    Text(String),
    Tool(ToolLine),
    Notice(String, Tone),
    /// A line across the transcript ("Conversación compactada").
    Divider(String),
    /// The summary a compaction left (shown folded).
    Summary(String),
    /// Files a turn changed, for review and Revertir.
    Changes { turn: u64, files: Vec<FileChange> },
}

/// One message the user sent, and where the conversation was before it (to retry, edit or
/// fork from there).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Turn {
    pub id: u64,
    /// Index of its first entry.
    pub entry: usize,
    pub prompt: String,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    #[serde(default)]
    pub session_before: String,
    /// Claude Code: the assistant message the session ended on before this turn.
    #[serde(default)]
    pub checkpoint_before: String,
    #[serde(default)]
    pub seed_before: String,
}

/// Context and spending of a conversation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Usage {
    /// Tokens in context after the last turn (0: unknown).
    pub context: u64,
    /// The model's context window (0: unknown).
    pub window: u64,
    /// Dollars of the CLI sessions this conversation left behind (compaction starts new ones).
    pub cost_base: f64,
    /// Dollars of the current CLI session, as it reports them (cumulative).
    pub session_cost: f64,
    /// Output tokens of every turn.
    pub output: u64,
    /// The context number is an estimate (the CLI reports per-turn totals).
    #[serde(default)]
    pub approx: bool,
}

impl Usage {
    pub fn cost(&self) -> f64 {
        self.cost_base + self.session_cost
    }

    /// Context used, 0..1; `None` when the window is unknown.
    pub fn fraction(&self) -> Option<f32> {
        (self.window > 0 && self.context > 0).then(|| (self.context as f64 / self.window as f64).min(1.0) as f32)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    /// Named by the user (no longer titled after the first message).
    #[serde(default)]
    pub renamed: bool,
    #[serde(default)]
    pub project: PathBuf,
    /// [`agent::Kind::id`].
    pub agent: String,
    #[serde(default)]
    pub model: String,
    /// The CLI session that continues it (empty: the next message starts one).
    #[serde(default)]
    pub session: String,
    #[serde(default)]
    pub checkpoint: String,
    /// What a new session must be told first (after compacting or editing with a CLI that
    /// cannot fork).
    #[serde(default)]
    pub seed: String,
    pub created: u64,
    pub updated: u64,
    #[serde(default)]
    pub entries: Vec<Entry>,
    #[serde(default)]
    pub turns: Vec<Turn>,
    #[serde(default)]
    pub usage: Usage,
}

impl Conversation {
    pub fn messages(&self) -> usize {
        self.entries.iter().filter(|e| matches!(e, Entry::User(_))).count()
    }
}

/// A conversation in the history list.
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub id: String,
    pub title: String,
    pub agent: String,
    pub updated: u64,
    pub messages: usize,
    pub cost: f64,
    /// Lower-case title and messages, for the search box.
    pub text: String,
}

impl Summary {
    pub fn matches(&self, query: &str) -> bool {
        let q = query.trim().to_lowercase();
        q.is_empty() || q.split_whitespace().all(|w| self.text.contains(w))
    }
}

/// The open conversation and settings of one project.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ProjectState {
    /// Conversation shown when the project opens again.
    #[serde(default)]
    pub last: String,
    #[serde(default)]
    pub agent: String,
    #[serde(default)]
    pub permission: String,
    /// Compact by itself past this % of the context (0: never).
    #[serde(default)]
    pub autocompact: u8,
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// A new conversation id: time-ordered and unique within this process.
pub fn new_id() -> String {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    format!("{ms:012x}-{:04x}{:04x}", std::process::id() & 0xffff, N.fetch_add(1, std::sync::atomic::Ordering::Relaxed) & 0xffff)
}

/// Title from the first message.
pub fn title_from(text: &str) -> String {
    let t = agent::one_line(text.trim(), 60);
    if t.is_empty() {
        "Conversación".into()
    } else {
        t
    }
}

fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// The folder of one project's conversations: its name (readable) and a hash of its path.
pub fn project_dir(root: &Path, project: &Path) -> PathBuf {
    let canon = project.canonicalize().unwrap_or_else(|_| project.to_path_buf());
    let name: String = canon
        .file_name()
        .map(|n| n.to_string_lossy().chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).take(40).collect())
        .unwrap_or_default();
    root.join(format!("{name}-{:016x}", fnv(&canon.to_string_lossy())))
}

/// One project's conversations on disk.
#[derive(Clone, Debug)]
pub struct Store {
    pub dir: PathBuf,
}

impl Store {
    pub fn new(root: &Path, project: &Path) -> Store {
        Store { dir: project_dir(root, project) }
    }

    fn path(&self, id: &str) -> PathBuf {
        let safe: String = id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
        self.dir.join(format!("{safe}.json"))
    }

    fn write(&self, path: &Path, text: &str) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)
    }

    pub fn save(&self, c: &Conversation) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(c).map_err(std::io::Error::other)?;
        self.write(&self.path(&c.id), &text)
    }

    pub fn load(&self, id: &str) -> Option<Conversation> {
        std::fs::read_to_string(self.path(id)).ok().and_then(|t| serde_json::from_str(&t).ok())
    }

    pub fn delete(&self, id: &str) -> bool {
        std::fs::remove_file(self.path(id)).is_ok()
    }

    /// Every saved conversation, newest first.
    pub fn list(&self) -> Vec<Summary> {
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return vec![] };
        let mut out: Vec<Summary> = rd
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json") && e.file_name() != "state.json")
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .filter_map(|t| serde_json::from_str::<Conversation>(&t).ok())
            .map(|c| {
                let mut text = c.title.to_lowercase();
                for e in &c.entries {
                    if let Entry::User(t) | Entry::Text(t) = e {
                        if text.len() < 64 * 1024 {
                            text.push('\n');
                            text += &t.to_lowercase();
                        }
                    }
                }
                Summary { id: c.id.clone(), title: c.title.clone(), agent: c.agent.clone(), updated: c.updated, messages: c.messages(), cost: c.usage.cost(), text }
            })
            .collect();
        out.sort_by(|a, b| b.updated.cmp(&a.updated).then_with(|| b.id.cmp(&a.id)));
        out
    }

    pub fn state(&self) -> ProjectState {
        std::fs::read_to_string(self.dir.join("state.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save_state(&self, s: &ProjectState) {
        if let Ok(text) = serde_json::to_string_pretty(s) {
            let _ = self.write(&self.dir.join("state.json"), &text);
        }
    }
}

/// The conversation as Markdown, for /exportar.
pub fn to_markdown(c: &Conversation) -> String {
    let kind = agent::Kind::from_id(&c.agent).map(|k| k.name()).unwrap_or("agente");
    let mut t = format!("# {}\n\n", c.title);
    t += &format!("*{kind}{} · {} · {} mensajes", if c.model.is_empty() { String::new() } else { format!(" ({})", c.model) }, date(c.created), c.messages());
    if c.usage.cost() > 0.0 {
        t += &format!(" · {}", money(c.usage.cost()));
    }
    t += "*\n\n";
    for e in &c.entries {
        match e {
            Entry::User(u) => t += &format!("---\n\n**Tú:**\n\n{}\n\n", u.trim()),
            Entry::Attached(list) => {
                let names: Vec<String> = list.iter().map(|a| if a.on_disk() { format!("`{}`", a.path.display()) } else { a.label.clone() }).collect();
                t += &format!("📎 {}\n\n", names.join(", "));
            }
            Entry::Text(x) => t += &format!("{}\n\n", x.trim()),
            Entry::Tool(l) => t += &format!("- {} **{}** {}\n", l.icon, l.verb, l.detail),
            Entry::Notice(n, tone) => {
                let icon = match tone {
                    Tone::Muted => "ℹ",
                    Tone::Warning => "⚠",
                    Tone::Error => "⛔",
                };
                t += &format!("> {icon} {}\n\n", n.replace('\n', "\n> "));
            }
            Entry::Divider(d) => t += &format!("\n---\n\n*— {d} —*\n\n"),
            Entry::Summary(s) => t += &format!("<details><summary>Resumen</summary>\n\n{}\n\n</details>\n\n", s.trim()),
            Entry::Changes { files, .. } => {
                t += "Archivos cambiados:\n\n";
                for f in files {
                    t += &format!("- `{}` +{} −{}{}\n", f.rel, f.added, f.removed, if f.reverted { " (revertido)" } else { "" });
                }
                t += "\n";
            }
        }
    }
    t
}

/// The transcript as plain text for a new session that must carry on from it (agents that
/// cannot fork); the most recent part when it is long.
pub fn transcript_seed(entries: &[Entry], max: usize) -> String {
    let mut t = String::new();
    for e in entries {
        match e {
            Entry::User(u) => t += &format!("User: {}\n\n", u.trim()),
            Entry::Text(x) => t += &format!("Assistant: {}\n\n", x.trim()),
            Entry::Tool(l) => t += &format!("(assistant used: {} {})\n", l.verb, l.detail),
            Entry::Summary(s) => t += &format!("Summary of what came before: {}\n\n", s.trim()),
            _ => {}
        }
    }
    if t.len() > max {
        let mut cut = t.len() - max;
        while !t.is_char_boundary(cut) {
            cut += 1;
        }
        t = format!("[… earlier part left out]\n{}", &t[cut..]);
    }
    t
}

pub fn money(d: f64) -> String {
    if d < 0.01 {
        format!("{d:.3} $").replace('.', ",")
    } else {
        format!("{d:.2} $").replace('.', ",")
    }
}

/// 1234 -> "1,2k", 1_000_000 -> "1M".
pub fn tokens(n: u64) -> String {
    if n >= 1_000_000 {
        let m = n as f64 / 1e6;
        if m.fract() < 0.05 { format!("{m:.0}M") } else { format!("{m:.1}M").replace('.', ",") }
    } else if n >= 1000 {
        let k = n as f64 / 1e3;
        if k >= 100.0 { format!("{k:.0}k") } else { format!("{k:.1}k").replace('.', ",") }
    } else {
        n.to_string()
    }
}

/// Local date and time, `dd/mm/aaaa hh:mm`.
pub fn date(secs: u64) -> String {
    let (y, mo, d, h, mi) = local(secs);
    format!("{d:02}/{mo:02}/{y} {h:02}:{mi:02}")
}

/// "hoy 17:23", "ayer 09:10", "05/11 12:00" or "05/11/2025".
pub fn when(secs: u64, now: u64) -> String {
    let (y, mo, d, h, mi) = local(secs);
    let (ny, nmo, nd, _, _) = local(now);
    let (yy, ymo, yd, _, _) = local(now.saturating_sub(86_400));
    if (y, mo, d) == (ny, nmo, nd) {
        format!("hoy {h:02}:{mi:02}")
    } else if (y, mo, d) == (yy, ymo, yd) {
        format!("ayer {h:02}:{mi:02}")
    } else if y == ny {
        format!("{d:02}/{mo:02} {h:02}:{mi:02}")
    } else {
        format!("{d:02}/{mo:02}/{y}")
    }
}

/// (year, month, day, hour, minute) in local time (UTC where it cannot be found).
fn local(secs: u64) -> (i64, u32, u32, u32, u32) {
    let offset = utc_offset(secs);
    let t = secs as i64 + offset;
    let days = t.div_euclid(86_400);
    let rem = t.rem_euclid(86_400);
    // Civil from days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d, (rem / 3600) as u32, (rem % 3600 / 60) as u32)
}

#[cfg(unix)]
fn utc_offset(secs: u64) -> i64 {
    // glibc and the BSDs/macOS start `struct tm` with nine ints followed by `tm_gmtoff`.
    #[repr(C)]
    struct Tm {
        ints: [i32; 9],
        gmtoff: i64,
        zone: *const i8,
    }
    extern "C" {
        fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
    }
    let t = secs as i64;
    let mut tm = Tm { ints: [0; 9], gmtoff: 0, zone: std::ptr::null() };
    // SAFETY: plain libc call writing into a struct laid out like `struct tm`'s head.
    let ok = unsafe { !localtime_r(&t, &mut tm).is_null() };
    if ok && tm.gmtoff.abs() < 86_400 {
        tm.gmtoff
    } else {
        0
    }
}

#[cfg(not(unix))]
fn utc_offset(_secs: u64) -> i64 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AttachKind, ToolLine};

    pub(crate) fn sample(id: &str, title: &str, updated: u64) -> Conversation {
        Conversation {
            id: id.into(),
            title: title.into(),
            agent: "claude".into(),
            session: "S1".into(),
            created: updated - 60,
            updated,
            entries: vec![
                Entry::User("haz que el jugador salte más".into()),
                Entry::Attached(vec![Attachment::file(Path::new("/tmp")), Attachment::url("https://example.com/doc"), Attachment::text("✎", "Nota", "hola")]),
                Entry::Tool(ToolLine { icon: "✎", verb: "Editar".into(), detail: "objects/player.ms".into() }),
                Entry::Text("He subido `jump_speed` a **12**.".into()),
                Entry::Changes { turn: 1, files: vec![] },
                Entry::Divider("Conversación compactada".into()),
                Entry::Summary("El jugador salta más.".into()),
                Entry::Notice("ojo".into(), Tone::Warning),
            ],
            turns: vec![Turn { id: 1, entry: 0, prompt: "haz que el jugador salte más".into(), ..Default::default() }],
            usage: Usage { context: 31_000, window: 1_000_000, session_cost: 0.12, ..Default::default() },
            ..Default::default()
        }
    }

    #[test]
    fn store_round_trip_list_search_delete() {
        let root = std::env::temp_dir().join(format!("modeditor_hist_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let project = std::env::temp_dir();
        let store = Store::new(&root, &project);
        assert!(store.dir.file_name().unwrap().to_string_lossy().contains('-'));
        let a = sample("a1", "Salto", 1_000_000);
        let b = sample("b2", "Enemigos", 2_000_000);
        store.save(&a).unwrap();
        store.save(&b).unwrap();
        let back = store.load("a1").unwrap();
        assert_eq!(back, a);
        assert!(matches!(&back.entries[1], Entry::Attached(v) if v[1].kind == AttachKind::Url && v[2].icon == "✎"));
        let list = store.list();
        assert_eq!(list.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["b2", "a1"]);
        assert_eq!((list[0].messages, list[0].cost), (1, 0.12));
        assert!(list[1].matches("salto JUGADOR"));
        assert!(!list[1].matches("dragón"));
        let st = ProjectState { last: "b2".into(), autocompact: 85, ..Default::default() };
        store.save_state(&st);
        assert_eq!(store.state(), st);
        assert_eq!(store.list().len(), 2, "state.json is not a conversation");
        assert!(store.delete("a1"));
        assert_eq!(store.list().len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn markdown_and_seed() {
        let c = sample("x", "Salto", 1_700_000_000);
        let md = to_markdown(&c);
        assert!(md.starts_with("# Salto\n\n*Claude Code · "), "{md}");
        assert!(md.contains("**Tú:**\n\nhaz que el jugador salte más"));
        assert!(md.contains("- ✎ **Editar** objects/player.ms"));
        assert!(md.contains("*— Conversación compactada —*"));
        let seed = transcript_seed(&c.entries, 10_000);
        assert!(seed.starts_with("User: haz que") && seed.contains("Assistant: He subido"));
        assert!(transcript_seed(&c.entries, 20).starts_with("[… earlier part left out]"));
    }

    #[test]
    fn numbers_and_dates() {
        assert_eq!(tokens(950), "950");
        assert_eq!(tokens(31_082), "31,1k");
        assert_eq!(tokens(272_000), "272k");
        assert_eq!(tokens(1_000_000), "1M");
        assert_eq!(money(0.1209), "0,12 $");
        assert_eq!(title_from("  arregla\nel salto"), "arregla…");
        // 2026-11-05 12:00 UTC: the date is right in any zone within ±11 h.
        assert!(date(1_793_880_000).starts_with("05/11/2026"), "{}", date(1_793_880_000));
        let now = 1_793_880_000;
        assert!(when(now, now).starts_with("hoy "));
        assert!(when(now - 86_400, now).starts_with("ayer "));
        assert!(when(now - 400 * 86_400, now).ends_with("2025"));
    }
}
