//! Fakes for the store's tests (and `MODENGINE_STORE_FIXTURES`): nothing here touches the
//! network.

use crate::net::{Http, NetError};
use crate::oga::{allowed_url, BASE};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// A fresh temporary folder for a test.
pub fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("modeditor_store_{tag}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

/// Trimmed copies of real OpenGameArt pages.
pub fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/oga")
}

pub fn fixture(name: &str) -> Vec<u8> {
    fs::read(fixture_dir().join(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// Serves canned responses and counts requests.
#[derive(Default)]
pub struct Mock {
    pub pages: HashMap<String, Result<Vec<u8>, NetError>>,
    pub hits: Mutex<Vec<String>>,
}

impl Http for Mock {
    fn get(&self, url: &str) -> Result<Vec<u8>, NetError> {
        assert!(allowed_url(url), "the store only asks for allowed URLs: {url}");
        self.hits.lock().unwrap().push(url.to_string());
        self.pages.get(url).cloned().unwrap_or(Err(NetError::Http(404)))
    }
}

/// Pages from a folder instead of OpenGameArt (`MODENGINE_STORE_FIXTURES=<dir>`, for tests of
/// `modengine store` and of the agent running it):
/// * `/art-search-advanced?…` → `search.html`
/// * `/content/<slug>` → `<slug>.html`
/// * `/sites/default/files/<path>` → `files/<path, percent-decoded>`
///
/// Every request is appended to `requests.log` (`<unix ms> <url>`), so tests can count them
/// and check the spacing between processes.
pub struct FixtureHttp {
    pub dir: PathBuf,
}

impl FixtureHttp {
    pub fn new(dir: PathBuf) -> FixtureHttp {
        FixtureHttp { dir }
    }

    fn file(&self, url: &str) -> Option<PathBuf> {
        let path = url.strip_prefix(BASE)?;
        if path.starts_with("/art-search-advanced?") {
            Some(self.dir.join("search.html"))
        } else if let Some(slug) = path.strip_prefix("/content/") {
            Some(self.dir.join(format!("{}.html", slug.split(['#', '?', '/']).next()?)))
        } else {
            let rest = path.strip_prefix("/sites/default/files/")?;
            let rel = modengine::import::percent_decode(rest);
            (!rel.split('/').any(|p| p == ".." || p.is_empty())).then(|| self.dir.join("files").join(rel))
        }
    }
}

impl Http for FixtureHttp {
    fn get(&self, url: &str) -> Result<Vec<u8>, NetError> {
        if !allowed_url(url) {
            return Err(NetError::Refused(url.into()));
        }
        let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        if let Ok(mut log) = fs::OpenOptions::new().create(true).append(true).open(self.dir.join("requests.log")) {
            let _ = writeln!(log, "{ms} {url}");
        }
        self.file(url).and_then(|p| fs::read(p).ok()).ok_or(NetError::Http(404))
    }
}

/// A folder for [`FixtureHttp`] (`MODENGINE_STORE_FIXTURES`) with the real pages of the
/// fixtures and files the size those pages announce: `animated-slime` (CC-BY 3.0, one PNG
/// strip of 4 frames of 16×16), `pixel-art-animated-slime` (CC0, `Slime.zip` with two strips
/// and a licence) and `tiny-creatures`; any search returns the `slime` results page.
pub fn fixture_site(dir: &Path) -> PathBuf {
    let _ = fs::create_dir_all(dir.join("files"));
    for slug in ["animated-slime", "pixel-art-animated-slime", "tiny-creatures"] {
        fs::write(dir.join(format!("{slug}.html")), fixture(&format!("{slug}.html"))).unwrap();
    }
    fs::write(dir.join("search.html"), fixture("search_slime_cc0.html")).unwrap();
    let strip = |w: u32, h: u32| {
        let mut b = std::io::Cursor::new(vec![]);
        image::RgbaImage::from_fn(w, h, |x, y| if x % 16 > 2 && x % 16 < 13 && y > 3 { image::Rgba([90, 200, 90, 255]) } else { image::Rgba([0, 0, 0, 0]) })
            .write_to(&mut b, image::ImageFormat::Png)
            .unwrap();
        b.into_inner()
    };
    // a PNG may carry bytes after its end: padded to the size on the page
    let mut sheet = strip(64, 16);
    sheet.resize(12041, 0);
    fs::write(dir.join("files/slime spritesheet calciumtrice_0.png"), sheet).unwrap();
    let mut z = make_zip(&[("Slime/slime_idle_strip4.png", &strip(64, 16), true), ("Slime/slime_move_strip2.png", &strip(32, 16), true), ("Slime/LICENSE.txt", b"CC0 by rvros", false)]);
    // the zip comment fills it up to the size on the page
    let n = z.len();
    let pad = 12061 - n;
    z[n - 2..].copy_from_slice(&(pad as u16).to_le_bytes());
    z.resize(12061, b' ');
    let url = crate::oga::parse_detail(&String::from_utf8_lossy(&fixture("pixel-art-animated-slime.html")), "pixel-art-animated-slime").unwrap().files[0].url.clone();
    let rel = modengine::import::percent_decode(url.strip_prefix(&format!("{BASE}/sites/default/files/")).unwrap());
    let to = dir.join("files").join(rel);
    let _ = fs::create_dir_all(to.parent().unwrap());
    fs::write(to, z).unwrap();
    dir.to_path_buf()
}

/// A small zip writer (stored or deflate entries).
pub fn make_zip(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
    let mut out = vec![];
    let mut central = vec![];
    for (name, data, deflate) in files {
        let (method, raw) = if *deflate {
            let mut e = flate2::write::DeflateEncoder::new(vec![], flate2::Compression::default());
            e.write_all(data).unwrap();
            (8u16, e.finish().unwrap())
        } else {
            (0u16, data.to_vec())
        };
        let crc = crc32fast::hash(data);
        let off = out.len() as u32;
        let h = |v: &mut Vec<u8>, central: bool| {
            v.extend(if central { 0x0201_4b50u32 } else { 0x0403_4b50 }.to_le_bytes());
            if central {
                v.extend(20u16.to_le_bytes());
            }
            v.extend(20u16.to_le_bytes());
            v.extend(0x800u16.to_le_bytes());
            v.extend(method.to_le_bytes());
            v.extend([0u8; 4]);
            v.extend(crc.to_le_bytes());
            v.extend((raw.len() as u32).to_le_bytes());
            v.extend((data.len() as u32).to_le_bytes());
            v.extend((name.len() as u16).to_le_bytes());
            v.extend(0u16.to_le_bytes());
            if central {
                v.extend([0u8; 6]); // comment length, disk, internal attributes
                v.extend(0u32.to_le_bytes()); // external attributes
                v.extend(off.to_le_bytes());
            }
            v.extend(name.as_bytes());
        };
        h(&mut out, false);
        out.extend(&raw);
        h(&mut central, true);
    }
    let cd = out.len() as u32;
    out.extend(&central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0u8; 4]);
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((central.len() as u32).to_le_bytes());
    out.extend(cd.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out
}
