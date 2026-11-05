//! Before-and-after pictures of the project folder around an agent turn: which files the turn
//! added, changed or deleted (with +/- line counts), a unified diff of each, and putting a file
//! back the way it was (Revertir, `/deshacer`).
//!
//! Snapshots live in memory only. Game projects are small, so a turn snapshots every file of
//! the project; caps keep a huge folder from eating memory (files over the cap are compared by
//! size and date and cannot be reverted).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

/// Biggest file kept in a snapshot.
pub const FILE_MAX: u64 = 2 * 1024 * 1024;
/// Most bytes one snapshot keeps.
pub const TOTAL_MAX: u64 = 96 * 1024 * 1024;
/// Most files one snapshot looks at.
pub const FILES_MAX: usize = 6000;

/// Folders never snapshotted.
const SKIP_DIRS: [&str; 4] = [".git", "target", "node_modules", "build"];

#[derive(Clone, Debug)]
pub struct FileState {
    /// The content; `None` when over the caps.
    pub bytes: Option<Arc<Vec<u8>>>,
    pub len: u64,
    pub modified: Option<SystemTime>,
}

impl FileState {
    fn same(&self, other: &FileState) -> bool {
        match (&self.bytes, &other.bytes) {
            (Some(a), Some(b)) => a == b,
            _ => self.len == other.len && self.modified == other.modified,
        }
    }
}

/// Every file of a folder at one moment, by path relative to it.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub root: PathBuf,
    pub files: BTreeMap<PathBuf, FileState>,
}

impl Snapshot {
    pub fn take(root: &Path) -> Snapshot {
        let mut s = Snapshot { root: root.to_path_buf(), files: BTreeMap::new() };
        let mut total = 0u64;
        walk(root, root, &mut s.files, &mut total, 0);
        s
    }

    /// The content of `rel` in this snapshot (`None`: did not exist or too big).
    pub fn bytes(&self, rel: &Path) -> Option<&[u8]> {
        self.files.get(rel).and_then(|f| f.bytes.as_deref().map(|b| b.as_slice()))
    }

    /// Puts `rel` back as it was in this snapshot: rewrites it, or deletes it when it did not exist.
    pub fn revert(&self, rel: &Path) -> Result<(), String> {
        let path = self.root.join(rel);
        match self.files.get(rel) {
            None => {
                if path.exists() {
                    std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", rel.display()))?;
                }
                Ok(())
            }
            Some(FileState { bytes: Some(b), .. }) => {
                if let Some(d) = path.parent() {
                    std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
                }
                std::fs::write(&path, b.as_slice()).map_err(|e| format!("{}: {e}", rel.display()))
            }
            Some(_) => Err(format!("{}: demasiado grande para guardarlo antes del turno", rel.display())),
        }
    }
}

fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, FileState>, total: &mut u64, depth: u32) {
    if depth > 12 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if out.len() >= FILES_MAX {
            return;
        }
        let name = e.file_name().to_string_lossy().to_string();
        let path = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            if !name.starts_with('.') && !SKIP_DIRS.contains(&name.as_str()) {
                walk(root, &path, out, total, depth + 1);
            }
            continue;
        }
        if !ft.is_file() {
            continue;
        }
        let Ok(meta) = e.metadata() else { continue };
        let len = meta.len();
        let bytes = if len <= FILE_MAX && *total + len <= TOTAL_MAX {
            std::fs::read(&path).ok().map(|b| {
                *total += b.len() as u64;
                Arc::new(b)
            })
        } else {
            None
        };
        let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
        out.insert(rel, FileState { bytes, len, modified: meta.modified().ok() });
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
}

/// One file a turn touched.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FileChange {
    /// Relative to the project, with `/`.
    pub rel: String,
    pub kind: ChangeKind,
    pub added: usize,
    pub removed: usize,
    /// Not text (or too big to compare line by line).
    #[serde(default)]
    pub binary: bool,
    /// Put back with Revertir.
    #[serde(default)]
    pub reverted: bool,
}

/// What changed between two snapshots of the same folder.
pub fn changes(before: &Snapshot, after: &Snapshot) -> Vec<FileChange> {
    let mut out = vec![];
    let text = |b: Option<&[u8]>| b.filter(|b| !b.contains(&0)).map(|b| String::from_utf8_lossy(b).into_owned());
    for (rel, a) in &after.files {
        let (kind, old) = match before.files.get(rel) {
            Some(b) if b.same(a) => continue,
            Some(_) => (ChangeKind::Modified, before.bytes(rel)),
            None => (ChangeKind::Added, Some(&[][..])),
        };
        out.push(change(rel, kind, text(old), text(after.bytes(rel))));
    }
    for rel in before.files.keys().filter(|r| !after.files.contains_key(*r)) {
        out.push(change(rel, ChangeKind::Deleted, text(before.bytes(rel)), Some(String::new())));
    }
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    out
}

fn change(rel: &Path, kind: ChangeKind, old: Option<String>, new: Option<String>) -> FileChange {
    let rel = rel.to_string_lossy().replace('\\', "/");
    match (old, new) {
        (Some(o), Some(n)) => {
            let (added, removed) = line_counts(&o, &n);
            FileChange { rel, kind, added, removed, binary: false, reverted: false }
        }
        _ => FileChange { rel, kind, added: 0, removed: 0, binary: true, reverted: false },
    }
}

// ------------------------------------------------------------------ diff

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Same,
    Add,
    Del,
}

/// Lines bigger than this product are diffed by trimming the common head and tail only.
const LCS_MAX: usize = 4_000_000;

/// The edit script turning `a` into `b`, line by line (longest common subsequence).
pub fn diff<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<(Op, &'a str)> {
    let head = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let tail = a[head..].iter().rev().zip(b[head..].iter().rev()).take_while(|(x, y)| x == y).count();
    let (am, bm) = (&a[head..a.len() - tail], &b[head..b.len() - tail]);
    let mut out: Vec<(Op, &str)> = a[..head].iter().map(|l| (Op::Same, *l)).collect();
    let (n, m) = (am.len(), bm.len());
    if n * m > LCS_MAX {
        out.extend(am.iter().map(|l| (Op::Del, *l)));
        out.extend(bm.iter().map(|l| (Op::Add, *l)));
    } else {
        // lcs[i][j]: longest common subsequence of am[i..] and bm[j..].
        let w = m + 1;
        let mut lcs = vec![0u32; (n + 1) * w];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                lcs[i * w + j] = if am[i] == bm[j] { lcs[(i + 1) * w + j + 1] + 1 } else { lcs[(i + 1) * w + j].max(lcs[i * w + j + 1]) };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n && j < m {
            if am[i] == bm[j] {
                out.push((Op::Same, am[i]));
                i += 1;
                j += 1;
            } else if lcs[(i + 1) * w + j] >= lcs[i * w + j + 1] {
                out.push((Op::Del, am[i]));
                i += 1;
            } else {
                out.push((Op::Add, bm[j]));
                j += 1;
            }
        }
        out.extend(am[i..].iter().map(|l| (Op::Del, *l)));
        out.extend(bm[j..].iter().map(|l| (Op::Add, *l)));
    }
    out.extend(a[a.len() - tail..].iter().map(|l| (Op::Same, *l)));
    out
}

/// Lines added and removed going from `old` to `new`.
pub fn line_counts(old: &str, new: &str) -> (usize, usize) {
    let (a, b): (Vec<&str>, Vec<&str>) = (old.lines().collect(), new.lines().collect());
    let d = diff(&a, &b);
    (d.iter().filter(|(o, _)| *o == Op::Add).count(), d.iter().filter(|(o, _)| *o == Op::Del).count())
}

/// One line of a unified diff as the diff view shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct DiffLine {
    pub op: Op,
    /// `@@ … @@` hunk header (op is Same).
    pub header: bool,
    pub old_no: Option<usize>,
    pub new_no: Option<usize>,
    pub text: String,
}

/// A unified diff with `context` lines around each change.
pub fn unified(old: &str, new: &str, context: usize) -> Vec<DiffLine> {
    let (a, b): (Vec<&str>, Vec<&str>) = (old.lines().collect(), new.lines().collect());
    let d = diff(&a, &b);
    // Line numbers of every op.
    let mut nums = Vec::with_capacity(d.len());
    let (mut o, mut n) = (0, 0);
    for (op, _) in &d {
        match op {
            Op::Same => {
                o += 1;
                n += 1;
                nums.push((Some(o), Some(n)));
            }
            Op::Del => {
                o += 1;
                nums.push((Some(o), None));
            }
            Op::Add => {
                n += 1;
                nums.push((None, Some(n)));
            }
        }
    }
    let changed: Vec<usize> = d.iter().enumerate().filter(|(_, (op, _))| *op != Op::Same).map(|(i, _)| i).collect();
    let mut out = vec![];
    let mut i = 0;
    while i < changed.len() {
        let start = changed[i].saturating_sub(context);
        let mut end = changed[i];
        while i < changed.len() && changed[i] <= end + 2 * context + 1 {
            end = changed[i];
            i += 1;
        }
        let end = (end + context + 1).min(d.len());
        let first_old = (start..end).find_map(|k| nums[k].0).unwrap_or(o);
        let first_new = (start..end).find_map(|k| nums[k].1).unwrap_or(n);
        let olds = (start..end).filter(|k| nums[*k].0.is_some()).count();
        let news = (start..end).filter(|k| nums[*k].1.is_some()).count();
        out.push(DiffLine { op: Op::Same, header: true, old_no: None, new_no: None, text: format!("@@ -{first_old},{olds} +{first_new},{news} @@") });
        for k in start..end {
            out.push(DiffLine { op: d[k].0, header: false, old_no: nums[k].0, new_no: nums[k].1, text: d[k].1.to_string() });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("modeditor_snap_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("objects")).unwrap();
        d
    }

    #[test]
    fn line_diff_and_unified() {
        assert_eq!(line_counts("a\nb\nc\n", "a\nB\nc\nd\n"), (2, 1));
        assert_eq!(line_counts("x", "x"), (0, 0));
        let old: String = (1..=20).map(|i| format!("l{i}\n")).collect();
        let new = old.replace("l3\n", "L3\n").replace("l18\n", "");
        let u = unified(&old, &new, 2);
        let headers: Vec<&str> = u.iter().filter(|l| l.header).map(|l| l.text.as_str()).collect();
        assert_eq!(headers, ["@@ -1,5 +1,5 @@", "@@ -16,5 +16,4 @@"]);
        assert!(u.iter().any(|l| l.op == Op::Del && l.text == "l3" && l.old_no == Some(3)));
        assert!(u.iter().any(|l| l.op == Op::Add && l.text == "L3" && l.new_no == Some(3)));
        assert!(u.iter().any(|l| l.op == Op::Del && l.text == "l18"));
    }

    #[test]
    fn snapshots_find_and_revert_changes() {
        let d = dir("revert");
        std::fs::write(d.join("game.toml"), "name = 'a'\n").unwrap();
        std::fs::write(d.join("objects/p.ms"), "object P {\n}\n").unwrap();
        std::fs::write(d.join("gone.txt"), "bye\n").unwrap();
        std::fs::create_dir_all(d.join(".git")).unwrap();
        std::fs::write(d.join(".git/HEAD"), "x").unwrap();
        let before = Snapshot::take(&d);
        assert!(!before.files.contains_key(Path::new(".git/HEAD")));

        std::fs::write(d.join("objects/p.ms"), "object P {\n  on step { x += 1 }\n}\n").unwrap();
        std::fs::write(d.join("objects/new.ms"), "object N {}\n").unwrap();
        std::fs::remove_file(d.join("gone.txt")).unwrap();
        let after = Snapshot::take(&d);
        let c = changes(&before, &after);
        let summary: Vec<(&str, ChangeKind, usize, usize)> = c.iter().map(|c| (c.rel.as_str(), c.kind, c.added, c.removed)).collect();
        assert_eq!(
            summary,
            [("gone.txt", ChangeKind::Deleted, 0, 1), ("objects/new.ms", ChangeKind::Added, 1, 0), ("objects/p.ms", ChangeKind::Modified, 1, 0)]
        );
        for c in &c {
            before.revert(Path::new(&c.rel)).unwrap();
        }
        assert!(changes(&before, &Snapshot::take(&d)).is_empty());
        assert_eq!(std::fs::read_to_string(d.join("gone.txt")).unwrap(), "bye\n");
        assert!(!d.join("objects/new.ms").exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
