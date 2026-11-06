//! Reading the .zip packs of OpenGameArt: list the entries and extract the chosen ones.
//!
//! Stored and deflate entries only (what every zip tool writes); deflate through `flate2`,
//! which the workspace already builds for the exporter, and the CRC-32 is checked with
//! `crc32fast` (also already there). No ZIP64: such archives (>4 GB) get a clear error.

use std::io::Read;
use std::path::{Path, PathBuf};

/// Refuse to inflate entries bigger than this (zip bombs).
pub const MAX_ENTRY: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct ZipItem {
    /// Path inside the archive (`Slime/slime_idle.png`).
    pub name: String,
    pub size: u64,
    pub csize: u64,
    pub method: u16,
    pub crc: u32,
    local: u64,
}

impl ZipItem {
    /// The last path segment.
    pub fn file_name(&self) -> &str {
        self.name.rsplit('/').next().unwrap_or(&self.name)
    }

    pub fn importable(&self) -> bool {
        modengine::import::Kind::of(Path::new(self.file_name())).is_some()
    }
}

fn r16(b: &[u8], i: usize) -> Result<u16, String> {
    b.get(i..i + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or_else(|| "zip: fichero cortado".to_string())
}

fn r32(b: &[u8], i: usize) -> Result<u32, String> {
    b.get(i..i + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| "zip: fichero cortado".to_string())
}

/// The files in the archive (folders and macOS metadata left out), in archive order.
pub fn list(b: &[u8]) -> Result<Vec<ZipItem>, String> {
    let start = b.len().saturating_sub(22 + 65535);
    let eocd = (start..b.len().saturating_sub(21)).rev().find(|&i| b[i..i + 4] == [0x50, 0x4b, 0x05, 0x06]).ok_or("no es un .zip válido (no tiene directorio central)")?;
    let count = r16(b, eocd + 10)? as usize;
    let mut p = r32(b, eocd + 16)? as usize;
    if count == 0xffff || p == 0xffff_ffff {
        return Err("zip de más de 4 GB (ZIP64): no soportado".into());
    }
    let mut out = vec![];
    for _ in 0..count {
        if r32(b, p)? != 0x0201_4b50 {
            return Err("zip: directorio central dañado".into());
        }
        let flags = r16(b, p + 8)?;
        let method = r16(b, p + 10)?;
        let crc = r32(b, p + 16)?;
        let csize = r32(b, p + 20)? as u64;
        let size = r32(b, p + 24)? as u64;
        let (nlen, xlen, clen) = (r16(b, p + 28)? as usize, r16(b, p + 30)? as usize, r16(b, p + 32)? as usize);
        let local = r32(b, p + 42)? as u64;
        let raw = b.get(p + 46..p + 46 + nlen).ok_or("zip: fichero cortado")?;
        // names are UTF-8 with flag 11, else CP437: ASCII in practice
        let name = if flags & 0x800 != 0 { String::from_utf8_lossy(raw).to_string() } else { raw.iter().map(|&c| if c < 128 { c as char } else { '_' }).collect() };
        p += 46 + nlen + xlen + clen;
        let name = name.replace('\\', "/");
        if name.ends_with('/') || name.starts_with("__MACOSX/") || name.rsplit('/').next().is_some_and(|f| f.starts_with("._") || f == ".DS_Store") {
            continue;
        }
        if flags & 1 != 0 {
            return Err(format!("{name}: el zip está cifrado"));
        }
        out.push(ZipItem { name, size, csize, method, crc, local });
    }
    Ok(out)
}

/// The bytes of one entry, checked against its size and CRC.
pub fn extract(b: &[u8], it: &ZipItem) -> Result<Vec<u8>, String> {
    if it.size > MAX_ENTRY {
        return Err(format!("{}: demasiado grande ({} MB)", it.name, it.size / (1024 * 1024)));
    }
    let l = it.local as usize;
    if r32(b, l)? != 0x0403_4b50 {
        return Err(format!("{}: cabecera local dañada", it.name));
    }
    let data_at = l + 30 + r16(b, l + 26)? as usize + r16(b, l + 28)? as usize;
    let raw = b.get(data_at..data_at + it.csize as usize).ok_or_else(|| format!("{}: datos cortados", it.name))?;
    let data = match it.method {
        0 => raw.to_vec(),
        8 => {
            let mut out = Vec::with_capacity(it.size as usize);
            flate2::read::DeflateDecoder::new(raw).take(MAX_ENTRY + 1).read_to_end(&mut out).map_err(|e| format!("{}: {e}", it.name))?;
            out
        }
        m => return Err(format!("{}: método de compresión {m} no soportado", it.name)),
    };
    if data.len() as u64 != it.size || crc32fast::hash(&data) != it.crc {
        return Err(format!("{}: los datos no cuadran con el zip (tamaño o CRC)", it.name));
    }
    Ok(data)
}

/// Extracts the chosen entries into `dir` (flat, by file name: a crafted `../../x` can't
/// escape it; clashing names get `_2`, `_3`…). Returns the written paths in order.
pub fn extract_to(b: &[u8], items: &[&ZipItem], dir: &Path) -> Result<Vec<PathBuf>, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut out: Vec<PathBuf> = vec![];
    for it in items {
        let data = extract(b, it)?;
        let name = crate::net::safe_file_name(it.file_name());
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) => (s.to_string(), format!(".{e}")),
            None => (name.clone(), String::new()),
        };
        let mut p = dir.join(&name);
        let mut n = 2;
        while out.contains(&p) {
            p = dir.join(format!("{stem}_{n}{ext}"));
            n += 1;
        }
        std::fs::write(&p, data).map_err(|e| format!("{}: {e}", p.display()))?;
        out.push(p);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::make_zip;

    #[test]
    fn lists_and_extracts() {
        let big = vec![7u8; 5000];
        let z = make_zip(&[
            ("Slime/", b"", false),
            ("Slime/slime_idle.png", &big, true),
            ("Slime/readme.txt", b"hola", false),
            ("__MACOSX/Slime/._slime_idle.png", b"junk", false),
            ("Other/slime_idle.png", b"second", false),
            ("../../evil.wav", b"RIFF", false),
        ]);
        let items = list(&z).unwrap();
        assert_eq!(items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), vec!["Slime/slime_idle.png", "Slime/readme.txt", "Other/slime_idle.png", "../../evil.wav"]);
        assert_eq!((items[0].size, items[0].method), (5000, 8));
        assert!(items[0].csize < 5000);
        assert!(items[0].importable() && !items[1].importable());
        assert_eq!(extract(&z, &items[0]).unwrap(), big);
        assert_eq!(extract(&z, &items[1]).unwrap(), b"hola");

        let dir = crate::testing::tmp("zip");
        let out = extract_to(&z, &[&items[0], &items[2], &items[3]], &dir.join("x")).unwrap();
        let names: Vec<&std::path::Path> = out.iter().map(|p| p.strip_prefix(&dir).unwrap()).collect();
        let want = ["x/slime_idle.png", "x/slime_idle_2.png", "x/evil.wav"].map(std::path::Path::new);
        assert_eq!(names, want, "flat and inside the folder");
        assert_eq!(std::fs::read(&out[1]).unwrap(), b"second");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn broken_zips_are_reported() {
        assert!(list(b"not a zip").is_err());
        let mut z = make_zip(&[("a.png", b"abcdef", false)]);
        let items = list(&z).unwrap();
        let at = z.windows(6).position(|w| w == b"abcdef").unwrap();
        z[at] = b'X';
        assert!(extract(&z, &items[0]).unwrap_err().contains("CRC"));
        let cut = make_zip(&[("a.png", b"abcdef", false)]);
        assert!(list(&cut[..cut.len() - 30]).is_err());
    }
}
