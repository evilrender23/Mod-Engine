//! `game.dat`: a whole project in one file, for exported games.
//!
//! ```text
//! header   "MODPACK\0"  u32 version  u32 file count  u64 index length  u64 checksum
//! index    per file: u16 path length, path (UTF-8, '/' separated), u64 offset, u64 length
//! data     the files, one after another (offsets are relative to the start of data)
//! ```
//!
//! All integers are little endian. The checksum is FNV-1a 64 of index + data. Files are
//! stored in path order and there are no timestamps, so the same project always gives
//! the same bytes.
//!
//! A pack can also be appended to an executable (the exported runner): then the file ends
//! with a 16-byte trailer `u64 pack length` + `"MODPKEND"`.

use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub const MAGIC: &[u8; 8] = b"MODPACK\0";
pub const TRAILER_MAGIC: &[u8; 8] = b"MODPKEND";
pub const VERSION: u32 = 1;
const HEADER: usize = 8 + 4 + 4 + 8 + 8;

/// Folders of a project that never go into a pack (editor data, notes, saves, exports).
pub const SKIP_DIRS: &[&str] = &["notes", "saves", "export", "target"];

pub struct Pack {
    data: Vec<u8>,
    /// path → (start, end) inside `data`.
    index: BTreeMap<String, (usize, usize)>,
}

pub fn fnv1a64(parts: &[&[u8]]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for p in parts {
        for b in *p {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    h
}

/// Should this project-relative path go into the pack? (`/` separated)
pub fn include(rel: &str) -> bool {
    let parts: Vec<&str> = rel.split('/').collect();
    if parts.iter().any(|p| p.starts_with('.') || p.is_empty()) {
        return false;
    }
    if parts.len() > 1 && SKIP_DIRS.contains(&parts[0]) {
        return false;
    }
    let file = parts[parts.len() - 1];
    // F12 screenshots and editor backups lying around the project folder
    !(parts.len() == 1 && file.starts_with("screenshot_") && file.ends_with(".png")) && !file.ends_with('~')
}

/// Every file of a project folder that belongs in a pack: (relative path, absolute path), sorted.
pub fn project_files(dir: &Path) -> Vec<(String, std::path::PathBuf)> {
    fn walk(root: &Path, d: &Path, out: &mut Vec<(String, std::path::PathBuf)>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('\\', "/");
            if p.is_dir() {
                if include(&format!("{rel}/x")) {
                    walk(root, &p, out);
                }
            } else if p.is_file() && include(&rel) {
                out.push((rel, p));
            }
        }
    }
    let mut out = vec![];
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// Builds the pack bytes from (path, contents) pairs (sorted by path here).
pub fn write(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut files: Vec<&(String, Vec<u8>)> = files.iter().collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut index = Vec::new();
    let mut off = 0u64;
    for (name, bytes) in &files {
        index.extend_from_slice(&(name.len() as u16).to_le_bytes());
        index.extend_from_slice(name.as_bytes());
        index.extend_from_slice(&off.to_le_bytes());
        index.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        off += bytes.len() as u64;
    }
    let mut data = Vec::with_capacity(off as usize);
    for (_, bytes) in &files {
        data.extend_from_slice(bytes);
    }
    let mut out = Vec::with_capacity(HEADER + index.len() + data.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(files.len() as u32).to_le_bytes());
    out.extend_from_slice(&(index.len() as u64).to_le_bytes());
    out.extend_from_slice(&fnv1a64(&[&index, &data]).to_le_bytes());
    out.extend_from_slice(&index);
    out.extend_from_slice(&data);
    out
}

/// Packs a project folder.
pub fn pack_dir(dir: &Path) -> Result<Vec<u8>, String> {
    if !dir.join("game.toml").is_file() {
        return Err(format!("{} is not a Mod Engine project (missing game.toml)", dir.display()));
    }
    let mut files = vec![];
    for (rel, p) in project_files(dir) {
        if rel.len() > u16::MAX as usize {
            return Err(format!("{rel}: path too long"));
        }
        files.push((rel, std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?));
    }
    Ok(write(&files))
}

/// `runner` + pack + trailer: a self-contained game executable.
pub fn append_to(runner: &[u8], pack: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(runner.len() + pack.len() + 16);
    out.extend_from_slice(runner);
    out.extend_from_slice(pack);
    out.extend_from_slice(&(pack.len() as u64).to_le_bytes());
    out.extend_from_slice(TRAILER_MAGIC);
    out
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[i..i + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], i: usize) -> u64 {
    u64::from_le_bytes(b[i..i + 8].try_into().unwrap())
}

impl std::fmt::Debug for Pack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Pack({} files, {} bytes)", self.index.len(), self.data.len())
    }
}

impl Pack {
    pub fn parse(bytes: Vec<u8>) -> Result<Pack, String> {
        if bytes.len() < HEADER || &bytes[..8] != MAGIC {
            return Err("not a Mod Engine game pack".into());
        }
        let version = u32_at(&bytes, 8);
        if version != VERSION {
            return Err(format!("game pack version {version} is not supported (this runner reads version {VERSION})"));
        }
        let count = u32_at(&bytes, 12) as usize;
        let index_len = u64_at(&bytes, 16) as usize;
        let sum = u64_at(&bytes, 24);
        if HEADER + index_len > bytes.len() {
            return Err("game pack is truncated".into());
        }
        let (index, data) = bytes[HEADER..].split_at(index_len);
        if fnv1a64(&[index, data]) != sum {
            return Err("game pack is damaged (checksum mismatch)".into());
        }
        let mut map = BTreeMap::new();
        let mut i = 0;
        for _ in 0..count {
            let bad = || "game pack index is damaged".to_string();
            let n = u16::from_le_bytes(index.get(i..i + 2).ok_or_else(bad)?.try_into().unwrap()) as usize;
            i += 2;
            let name = std::str::from_utf8(index.get(i..i + n).ok_or_else(bad)?).map_err(|_| bad())?.to_string();
            i += n;
            let off = u64_at(index.get(i..i + 16).ok_or_else(bad)?, 0) as usize;
            let len = u64_at(index, i + 8) as usize;
            i += 16;
            if off + len > data.len() {
                return Err(bad());
            }
            map.insert(name, (HEADER + index_len + off, HEADER + index_len + off + len));
        }
        Ok(Pack { data: bytes, index: map })
    }

    pub fn file(&self, rel: &str) -> Option<&[u8]> {
        self.index.get(rel).map(|&(a, b)| &self.data[a..b])
    }

    pub fn files(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.index.iter().map(|(k, &(a, b))| (k.as_str(), &self.data[a..b]))
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    pub fn is_dir(&self, rel: &str) -> bool {
        rel.is_empty() || self.index.range(format!("{rel}/")..).next().is_some_and(|(k, _)| k.starts_with(&format!("{rel}/")))
    }

    /// Direct children of a folder: (name, is_dir), sorted.
    pub fn children(&self, rel: &str) -> Vec<(String, bool)> {
        let prefix = if rel.is_empty() { String::new() } else { format!("{rel}/") };
        let mut out: Vec<(String, bool)> = vec![];
        for (k, _) in self.index.range(prefix.clone()..) {
            let Some(rest) = k.strip_prefix(&prefix) else { break };
            let (name, is_dir) = match rest.find('/') {
                Some(i) => (&rest[..i], true),
                None => (rest, false),
            };
            if out.last().map(|(n, _)| n.as_str()) != Some(name) {
                out.push((name.to_string(), is_dir));
            }
        }
        out
    }

    /// Reads a pack file (`game.dat`) or an executable with a pack appended.
    pub fn open(path: &Path) -> Result<Pack, String> {
        let mut f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut head = [0u8; 8];
        let is_pack = f.read_exact(&mut head).is_ok() && &head == MAGIC;
        if is_pack {
            let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            return Pack::parse(bytes);
        }
        match appended(&mut f).map_err(|e| format!("{}: {e}", path.display()))? {
            Some(bytes) => Pack::parse(bytes),
            None => Err(format!("{}: has no game pack", path.display())),
        }
    }
}

/// The pack appended to an executable, if it has one.
pub fn appended<R: Read + Seek>(f: &mut R) -> std::io::Result<Option<Vec<u8>>> {
    let len = f.seek(SeekFrom::End(0))?;
    if len < 16 {
        return Ok(None);
    }
    f.seek(SeekFrom::End(-16))?;
    let mut t = [0u8; 16];
    f.read_exact(&mut t)?;
    if &t[8..] != TRAILER_MAGIC {
        return Ok(None);
    }
    let plen = u64_at(&t, 0);
    if plen + 16 > len {
        return Ok(None);
    }
    f.seek(SeekFrom::Start(len - 16 - plen))?;
    let mut bytes = vec![0u8; plen as usize];
    f.read_exact(&mut bytes)?;
    Ok(Some(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_listing() {
        let files = vec![
            ("sprites/b.png".to_string(), vec![1, 2, 3]),
            ("game.toml".to_string(), b"[game]\nname = \"x\"\n".to_vec()),
            ("sprites/a.toml".to_string(), vec![]),
            ("shaders/glow/glow.fsh".to_string(), b"void main(){}".to_vec()),
        ];
        let bytes = write(&files);
        assert_eq!(bytes, write(&files.iter().rev().cloned().collect::<Vec<_>>()), "deterministic");
        let p = Pack::parse(bytes.clone()).unwrap();
        assert_eq!(p.len(), 4);
        assert_eq!(p.file("sprites/b.png"), Some(&[1u8, 2, 3][..]));
        assert_eq!(p.file("sprites/a.toml"), Some(&[][..]));
        assert_eq!(p.file("nope"), None);
        assert!(p.is_dir("sprites") && p.is_dir("shaders/glow") && !p.is_dir("sprite") && !p.is_dir("game.toml"));
        assert_eq!(p.children(""), vec![("game.toml".into(), false), ("shaders".into(), true), ("sprites".into(), true)]);
        assert_eq!(p.children("sprites"), vec![("a.toml".into(), false), ("b.png".into(), false)]);
        // damage is detected
        let mut bad = bytes.clone();
        let n = bad.len();
        bad[n - 1] ^= 0xff;
        assert!(Pack::parse(bad).unwrap_err().contains("checksum"));
        let mut v2 = bytes;
        v2[8] = 9;
        assert!(Pack::parse(v2).unwrap_err().contains("version"));
    }

    #[test]
    fn trailer_detection() {
        let pack = write(&[("game.toml".to_string(), b"x".to_vec())]);
        let exe = append_to(b"\x7fELF fake runner bytes", &pack);
        let got = appended(&mut std::io::Cursor::new(&exe)).unwrap().unwrap();
        assert_eq!(got, pack);
        assert!(appended(&mut std::io::Cursor::new(b"\x7fELF plain runner, no pack".to_vec())).unwrap().is_none());
        assert!(appended(&mut std::io::Cursor::new(b"short".to_vec())).unwrap().is_none());
        let dir = std::env::temp_dir().join(format!("modengine_trailer_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("game"), &exe).unwrap();
        std::fs::write(dir.join("game.dat"), &pack).unwrap();
        assert_eq!(Pack::open(&dir.join("game")).unwrap().file("game.toml"), Some(&b"x"[..]));
        assert_eq!(Pack::open(&dir.join("game.dat")).unwrap().file("game.toml"), Some(&b"x"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn include_rules() {
        assert!(include("game.toml") && include("sprites/a.png") && include("objects/player.ms"));
        assert!(!include("notes/todo.toml") && !include("saves/slot.json") && !include(".modengine/state.json"));
        assert!(!include(".trash/x.png") && !include("sprites/.hidden.png") && !include("screenshot_1.png"));
        assert!(!include("export/linux/Game") && include("sprites/notes.png"));
    }
}
