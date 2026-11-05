//! Minimal .zip (read + write) and .tar.gz writers for exported games and APKs.

use flate2::write::{DeflateEncoder, GzEncoder};
use flate2::Compression;
use std::io::Write;

pub struct ZipEntry {
    pub name: String,
    /// 0 = stored, 8 = deflate.
    pub method: u16,
    pub crc: u32,
    pub size: u32,
    /// The bytes as stored in the archive (compressed when method = 8).
    pub raw: Vec<u8>,
    /// Unix permissions (0 = default).
    pub mode: u32,
}

impl ZipEntry {
    pub fn new(name: &str, data: &[u8], compress: bool, mode: u32) -> ZipEntry {
        let crc = crc32fast::hash(data);
        let (method, raw) = if compress && !data.is_empty() {
            let mut enc = DeflateEncoder::new(Vec::new(), Compression::best());
            enc.write_all(data).unwrap();
            let c = enc.finish().unwrap();
            if c.len() < data.len() { (8, c) } else { (0, data.to_vec()) }
        } else {
            (0, data.to_vec())
        };
        ZipEntry { name: name.replace('\\', "/"), method, crc, size: data.len() as u32, raw, mode }
    }

    pub fn data(&self) -> Result<Vec<u8>, String> {
        match self.method {
            0 => Ok(self.raw.clone()),
            8 => {
                let mut out = Vec::new();
                let mut dec = flate2::write::DeflateDecoder::new(&mut out);
                dec.write_all(&self.raw).map_err(|e| format!("{}: {e}", self.name))?;
                dec.finish().map_err(|e| format!("{}: {e}", self.name))?;
                Ok(out)
            }
            m => Err(format!("{}: unsupported zip method {m}", self.name)),
        }
    }
}

fn u16le(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}
fn u32le(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}

/// Fixed timestamp (1980-01-01) so the same input gives the same archive.
const DOS_TIME: u16 = 0;
const DOS_DATE: u16 = (1 << 5) | 1;

pub fn write_zip(entries: &[ZipEntry]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for e in entries {
        let off = out.len() as u32;
        let name = e.name.as_bytes();
        u32le(&mut out, 0x04034b50);
        u16le(&mut out, 20);
        u16le(&mut out, 0x0800); // UTF-8 names
        u16le(&mut out, e.method);
        u16le(&mut out, DOS_TIME);
        u16le(&mut out, DOS_DATE);
        u32le(&mut out, e.crc);
        u32le(&mut out, e.raw.len() as u32);
        u32le(&mut out, e.size);
        u16le(&mut out, name.len() as u16);
        u16le(&mut out, 0);
        out.extend_from_slice(name);
        out.extend_from_slice(&e.raw);

        u32le(&mut central, 0x02014b50);
        u16le(&mut central, (3 << 8) | 20); // made by Unix, so the mode below counts
        u16le(&mut central, 20);
        u16le(&mut central, 0x0800);
        u16le(&mut central, e.method);
        u16le(&mut central, DOS_TIME);
        u16le(&mut central, DOS_DATE);
        u32le(&mut central, e.crc);
        u32le(&mut central, e.raw.len() as u32);
        u32le(&mut central, e.size);
        u16le(&mut central, name.len() as u16);
        u16le(&mut central, 0);
        u16le(&mut central, 0);
        u16le(&mut central, 0);
        u16le(&mut central, 0);
        let mode = if e.mode == 0 { 0o100644 } else { 0o100000 | e.mode };
        u32le(&mut central, mode << 16);
        u32le(&mut central, off);
        central.extend_from_slice(name);
    }
    let cd_off = out.len() as u32;
    out.extend_from_slice(&central);
    u32le(&mut out, 0x06054b50);
    u16le(&mut out, 0);
    u16le(&mut out, 0);
    u16le(&mut out, entries.len() as u16);
    u16le(&mut out, entries.len() as u16);
    u32le(&mut out, central.len() as u32);
    u32le(&mut out, cd_off);
    u16le(&mut out, 0);
    out
}

/// Every entry of a zip file (APK), keeping the stored bytes as they are.
pub fn read_zip(b: &[u8]) -> Result<Vec<ZipEntry>, String> {
    let r16 = |i: usize| -> Result<u16, String> { b.get(i..i + 2).map(|s| u16::from_le_bytes(s.try_into().unwrap())).ok_or("zip: truncated".into()) };
    let r32 = |i: usize| -> Result<u32, String> { b.get(i..i + 4).map(|s| u32::from_le_bytes(s.try_into().unwrap())).ok_or("zip: truncated".into()) };
    let eocd = (0..b.len().saturating_sub(21)).rev().find(|&i| b[i..i + 4] == [0x50, 0x4b, 0x05, 0x06]).ok_or("zip: no end of central directory")?;
    let count = r16(eocd + 10)? as usize;
    let mut p = r32(eocd + 16)? as usize;
    let mut out = vec![];
    for _ in 0..count {
        if r32(p)? != 0x02014b50 {
            return Err("zip: bad central directory".into());
        }
        let method = r16(p + 10)?;
        let crc = r32(p + 16)?;
        let csize = r32(p + 20)? as usize;
        let size = r32(p + 24)?;
        let (nlen, xlen, clen) = (r16(p + 28)? as usize, r16(p + 30)? as usize, r16(p + 32)? as usize);
        let mode = r32(p + 38)? >> 16;
        let local = r32(p + 42)? as usize;
        let name = String::from_utf8_lossy(b.get(p + 46..p + 46 + nlen).ok_or("zip: truncated")?).to_string();
        let data_at = local + 30 + r16(local + 26)? as usize + r16(local + 28)? as usize;
        let raw = b.get(data_at..data_at + csize).ok_or("zip: truncated entry")?.to_vec();
        out.push(ZipEntry { name, method, crc, size, raw, mode: mode & 0o777 });
        p += 46 + nlen + xlen + clen;
    }
    Ok(out)
}

/// A .tar.gz with the given files: (path, data, unix mode). Folders are created implicitly.
pub fn write_tar_gz(files: &[(String, Vec<u8>, u32)]) -> Vec<u8> {
    let mut tar = Vec::new();
    let mut dirs: Vec<String> = vec![];
    for (path, _, _) in files {
        let parts: Vec<&str> = path.split('/').collect();
        for i in 1..parts.len() {
            let d = parts[..i].join("/") + "/";
            if !dirs.contains(&d) {
                dirs.push(d);
            }
        }
    }
    for d in &dirs {
        tar_entry(&mut tar, d, &[], 0o755, b'5');
    }
    for (path, data, mode) in files {
        tar_entry(&mut tar, path, data, *mode, b'0');
    }
    tar.extend_from_slice(&[0u8; 1024]);
    let mut gz = GzEncoder::new(Vec::new(), Compression::best());
    gz.write_all(&tar).unwrap();
    gz.finish().unwrap()
}

fn tar_entry(out: &mut Vec<u8>, path: &str, data: &[u8], mode: u32, kind: u8) {
    let mut h = [0u8; 512];
    let (prefix, name) = if path.len() > 100 {
        let cut = path[..path.len().min(155)].rfind('/').unwrap_or(0);
        (&path[..cut], &path[cut + 1..])
    } else {
        ("", path)
    };
    h[..name.len().min(100)].copy_from_slice(&name.as_bytes()[..name.len().min(100)]);
    let oct = |h: &mut [u8; 512], at: usize, len: usize, v: u64| {
        let s = format!("{:0w$o}\0", v, w = len - 1);
        h[at..at + len].copy_from_slice(s.as_bytes());
    };
    oct(&mut h, 100, 8, mode as u64);
    oct(&mut h, 108, 8, 0);
    oct(&mut h, 116, 8, 0);
    oct(&mut h, 124, 12, data.len() as u64);
    oct(&mut h, 136, 12, 0);
    h[148..156].copy_from_slice(b"        ");
    h[156] = kind;
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    h[345..345 + prefix.len()].copy_from_slice(prefix.as_bytes());
    let sum: u32 = h.iter().map(|b| *b as u32).sum();
    let s = format!("{:06o}\0 ", sum);
    h[148..156].copy_from_slice(s.as_bytes());
    out.extend_from_slice(&h);
    out.extend_from_slice(data);
    let pad = (512 - data.len() % 512) % 512;
    out.extend(std::iter::repeat_n(0u8, pad));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zip_round_trip() {
        let big = vec![7u8; 5000];
        let z = write_zip(&[ZipEntry::new("Game/Game.exe", &big, true, 0o755), ZipEntry::new("Game/README.txt", b"hola", false, 0)]);
        assert_eq!(z, write_zip(&[ZipEntry::new("Game/Game.exe", &big, true, 0o755), ZipEntry::new("Game/README.txt", b"hola", false, 0)]), "deterministic");
        let r = read_zip(&z).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!((r[0].name.as_str(), r[0].method, r[0].mode), ("Game/Game.exe", 8, 0o755));
        assert_eq!(r[0].data().unwrap(), big);
        assert_eq!(r[1].data().unwrap(), b"hola");
        assert_eq!(r[1].crc, crc32fast::hash(b"hola"));
        // re-writing raw entries keeps them byte for byte
        assert_eq!(write_zip(&r), z);
    }

    #[test]
    fn tar_gz_has_folders_files_and_modes() {
        let t = write_tar_gz(&[("Game/Game".into(), vec![1, 2, 3], 0o755), ("Game/README.txt".into(), b"x".to_vec(), 0o644)]);
        let mut raw = Vec::new();
        let mut d = flate2::write::GzDecoder::new(&mut raw);
        d.write_all(&t).unwrap();
        d.finish().unwrap();
        assert_eq!(&raw[..5], b"Game/");
        assert_eq!(raw[156], b'5');
        assert_eq!(&raw[512..521], b"Game/Game");
        assert_eq!(&raw[512 + 100..512 + 107], b"0000755");
        assert_eq!(&raw[512 + 257..512 + 262], b"ustar");
        assert_eq!(&raw[1024..1027], &[1, 2, 3]);
    }
}
