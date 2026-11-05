//! Screenshots for the Agente panel: images pasted with Ctrl+V (egui's `Event::PasteImage`), read
//! from the clipboard with "📋 Pegar imagen", or a capture of the editor window. They are saved as
//! PNG outside the project (so they never show up in the turn's changed files) and attached like
//! any other image from outside the project.

use eframe::egui::{self, ColorImage};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Pasted images older than this are deleted when a new one is saved.
const KEEP: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// Longest side kept; bigger screenshots are scaled down (agents downscale them anyway).
const MAX_SIDE: u32 = 2560;

/// `~/.cache/modengine/agent/pegadas` (or `$XDG_CACHE_HOME/…`; `%LOCALAPPDATA%` on Windows).
pub fn pasted_dir() -> PathBuf {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let base = if cfg!(windows) {
        env("LOCALAPPDATA").unwrap_or_else(std::env::temp_dir)
    } else {
        env("XDG_CACHE_HOME").or_else(|| env("HOME").map(|h| h.join(".cache"))).unwrap_or_else(std::env::temp_dir)
    };
    base.join("modengine").join("agent").join("pegadas")
}

/// Saves `img` as `<dir>/<prefix>-<date>.png` and returns its path.
pub fn save(img: &ColorImage, dir: &Path, prefix: &str) -> Result<PathBuf, String> {
    let [w, h] = img.size;
    if w == 0 || h == 0 {
        return Err("la imagen está vacía".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    clean_old(dir);
    let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
    let mut buf = image::RgbaImage::from_raw(w as u32, h as u32, rgba).ok_or("tamaño de imagen no válido")?;
    let longest = buf.width().max(buf.height());
    if longest > MAX_SIDE {
        let k = MAX_SIDE as f32 / longest as f32;
        let (nw, nh) = (((buf.width() as f32 * k).round() as u32).max(1), ((buf.height() as f32 * k).round() as u32).max(1));
        buf = image::imageops::resize(&buf, nw, nh, image::imageops::FilterType::Triangle);
    }
    let stamp = chrono_like_stamp();
    let mut path = dir.join(format!("{prefix}-{stamp}.png"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{prefix}-{stamp}-{n}.png"));
        n += 1;
    }
    buf.save(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// `2026-11-05_21-58-03` in local-ish time without a date crate (UTC; only used for file names).
fn chrono_like_stamp() -> String {
    let secs = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // civil-from-days (Howard Hinnant)
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}_{:02}-{:02}-{:02}", rem / 3600, rem / 60 % 60, rem % 60)
}

fn clean_old(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > KEEP);
        if old && e.path().extension().is_some_and(|x| x == "png") {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Reads an image from the system clipboard (blocking: call it off the UI thread).
pub fn clipboard_image() -> Result<ColorImage, String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| format!("no se pudo abrir el portapapeles: {e}"))?;
    match cb.get_image() {
        Ok(img) => Ok(ColorImage::from_rgba_unmultiplied([img.width, img.height], &img.bytes)),
        Err(arboard::Error::ContentNotAvailable) => Err("el portapapeles no tiene ninguna imagen: copia una captura (Impr Pant, Recortes…) y vuelve a pulsar".into()),
        Err(e) => Err(format!("no se pudo leer la imagen del portapapeles: {e}")),
    }
}

/// A small texture of the image at `path`, for chips and their hover preview.
pub fn thumbnail(ctx: &egui::Context, path: &Path) -> Option<egui::TextureHandle> {
    let img = image::open(path).ok()?.thumbnail(320, 320).to_rgba8();
    let ci = ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
    Some(ctx.load_texture(format!("agent_thumb:{}", path.display()), ci, egui::TextureOptions::LINEAR))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_scales_and_cleans() {
        let dir = std::env::temp_dir().join(format!("modeditor_paste_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let img = ColorImage::filled([3000, 1000], egui::Color32::from_rgb(200, 10, 10));
        let p = save(&img, &dir, "captura").unwrap();
        assert!(p.file_name().unwrap().to_string_lossy().starts_with("captura-"));
        let back = image::open(&p).unwrap();
        assert_eq!((back.width(), back.height()), (2560, 853));
        assert_eq!(back.to_rgba8().get_pixel(5, 5).0, [200, 10, 10, 255]);
        // Two in the same second do not overwrite each other.
        let q = save(&ColorImage::filled([4, 4], egui::Color32::WHITE), &dir, "captura").unwrap();
        assert_ne!(p, q);
        assert!(save(&ColorImage::filled([0, 0], egui::Color32::WHITE), &dir, "x").is_err());
        let stamp = chrono_like_stamp();
        assert_eq!((stamp.len(), &stamp[4..5], &stamp[10..11]), (19, "-", "_"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
