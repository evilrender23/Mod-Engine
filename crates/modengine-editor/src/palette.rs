//! Paletas (Lospec): download palettes from lospec.com or import .hex/.gpl/.pal/.json/
//! .png files, keep them in ~/.config/modengine/palettes/, apply one to the project's
//! `[palette]` (adding the missing colours, or replacing it and remapping every sprite
//! to the nearest colour, with a backup to undo) and the swatch grid that the sprite
//! editor and Ajustes del juego share.
//!
//! Downloads shell out to `curl` on a background thread: it ships with Linux, macOS and
//! Windows 10+, so the editor needs no HTTP/TLS crates, and the UI thread never blocks.

use crate::app::{LogKind, LogLine};
use crate::project::Project;
use crate::tex::{checkerboard, rgb};
use crate::tomlw::{block, q};
use eframe::egui::{self, Color32, Rect, RichText, Sense, Stroke, StrokeKind, TextureHandle, TextureOptions, Vec2};
use modengine::palette::{self as pal, Palette, PALETTE_CHARS};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};

// ------------------------------------------------------------------ saved palettes

/// Where downloaded / imported palettes are kept (`~/.config/modengine/palettes`).
pub fn palettes_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(|h| PathBuf::from(h).join(".config/modengine/palettes"))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Saved {
    pub slug: String,
    pub pal: Palette,
    /// `None` for the built-in Pear36.
    pub path: Option<PathBuf>,
}

pub fn slugify(name: &str) -> String {
    let mut s = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
        } else if !s.ends_with('-') && !s.is_empty() {
            s.push('-');
        }
    }
    let s = s.trim_end_matches('-').to_string();
    if s.is_empty() {
        "paleta".into()
    } else {
        s
    }
}

/// Built-in Pear36 first, then every saved palette by name.
pub fn list_saved(dir: Option<&Path>) -> Vec<Saved> {
    let builtin = Saved { slug: "pear36".into(), pal: Palette::pear36(), path: None };
    let mut v = vec![];
    if let Some(rd) = dir.and_then(|d| std::fs::read_dir(d).ok()) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "json") {
                if let Ok(pl) = std::fs::read(&p).map_err(|e| e.to_string()).and_then(|b| Palette::parse_any(&p.to_string_lossy(), &b)) {
                    let slug = p.file_stem().unwrap_or_default().to_string_lossy().to_string();
                    if !(slug == "pear36" && pl.colors == builtin.pal.colors) {
                        v.push(Saved { slug, pal: pl, path: Some(p) });
                    }
                }
            }
        }
    }
    v.sort_by_key(|s| s.pal.name.to_lowercase());
    v.insert(0, builtin);
    v
}

pub fn save_palette(dir: &Path, slug: &str, p: &Palette) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(format!("{slug}.json"));
    std::fs::write(&path, p.to_json()).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

// ------------------------------------------------------------------ downloading

pub type Fetch = fn(&str) -> Result<Vec<u8>, String>;

/// GET with `curl` (no shell, fails on HTTP errors, 20 s limit).
pub fn curl_fetch(url: &str) -> Result<Vec<u8>, String> {
    let mut cmd = std::process::Command::new("curl");
    cmd.args(["-fsSL", "--max-time", "20", "-A", "ModEngine-editor", url]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().map_err(|e| format!("no se pudo ejecutar curl: {e}"))?;
    if out.status.success() {
        return Ok(out.stdout);
    }
    Err(match out.status.code() {
        Some(22) => "Lospec no tiene esa paleta (¿está bien escrito el nombre?)".into(),
        Some(6) | Some(7) => "sin conexión con lospec.com".into(),
        Some(28) => "lospec.com no responde (tiempo agotado)".into(),
        _ => format!("curl: {}", String::from_utf8_lossy(&out.stderr).trim()),
    })
}

/// Downloads `<slug>.json` from Lospec and parses it.
pub fn download(slug: &str, fetch: Fetch) -> Result<Palette, String> {
    let bytes = fetch(&pal::lospec_json_url(slug))?;
    let mut p = pal::parse_json(&String::from_utf8_lossy(&bytes)).map_err(|e| format!("respuesta de Lospec no válida: {e}"))?;
    if p.name.is_empty() {
        p.name = slug.into();
    }
    Ok(p)
}

// ------------------------------------------------------------------ applying to a project

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    /// Add the colours the project lacks; existing characters keep their colour.
    Merge,
    /// The palette becomes exactly this one and every sprite is remapped.
    Replace,
}

#[derive(Clone, Debug)]
pub struct SpriteChange {
    pub name: String,
    pub path: PathBuf,
    /// Frame 0 before/after, for the preview: rows and the colours they use.
    pub before: (String, BTreeMap<char, u32>),
    pub after: String,
    /// New file contents.
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub palette: BTreeMap<String, String>,
    pub sprites: Vec<SpriteChange>,
    /// Colours that didn't get a character (palette bigger than the character set).
    pub skipped: usize,
    pub added: usize,
    pub comment: String,
}

fn to_colors(m: &BTreeMap<String, String>) -> BTreeMap<char, u32> {
    m.iter().filter_map(|(k, v)| Some((k.chars().next()?, modengine::draw::parse_color(v)?))).collect()
}

pub fn credit(p: &Palette) -> String {
    if p.author.is_empty() {
        format!("# Paleta: {}. '.' es transparente.", p.name)
    } else {
        format!("# Paleta: {} de {} (lospec.com). '.' es transparente.", p.name, p.author)
    }
}

/// What applying `p` would change. `sprites`: (name, path) of the pixel-art TOML sprites.
pub fn plan(game_palette: &BTreeMap<String, String>, sprites: &[(String, PathBuf)], p: &Palette, mode: Mode) -> Result<Plan, String> {
    let comment = credit(p);
    if mode == Mode::Merge || game_palette.is_empty() && sprites.is_empty() {
        let (palette, skipped) = if game_palette.is_empty() { (p.char_map(), p.colors.len().saturating_sub(pal::max_colors())) } else { pal::merge_chars(game_palette, p) };
        let added = palette.len() - game_palette.len().min(palette.len());
        return Ok(Plan { palette, sprites: vec![], skipped, added, comment });
    }
    let palette = p.char_map();
    let global = to_colors(game_palette);
    let mut changes = vec![];
    for (name, path) in sprites {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let doc: toml::Table = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        let Some(frames) = doc.get("pixels").and_then(|v| v.as_array()) else { continue };
        let frames: Vec<String> = frames.iter().filter_map(|f| f.as_str().map(String::from)).collect();
        let local: BTreeMap<String, String> = doc.get("palette").and_then(|v| v.as_table()).map(|t| t.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect()).unwrap_or_default();
        let mut colors = global.clone();
        colors.extend(to_colors(&local));
        let table = pal::remap_table(&colors, p);
        let new_frames: Vec<String> = frames.iter().map(|f| pal::remap_rows(f, &table)).collect();
        if new_frames == frames && local.is_empty() {
            continue;
        }
        changes.push(SpriteChange {
            name: name.clone(),
            path: path.clone(),
            before: (frames.first().cloned().unwrap_or_default(), colors),
            after: new_frames.first().cloned().unwrap_or_default(),
            text: set_sprite_pixels(&text, &new_frames)?,
        });
    }
    let added = palette.keys().filter(|k| !game_palette.contains_key(*k)).count();
    Ok(Plan { palette, sprites: changes, skipped: p.colors.len().saturating_sub(pal::max_colors()), added, comment })
}

/// The `[palette]` table as TOML text (quoted one-character keys, palette order).
fn palette_table_text(map: &BTreeMap<String, String>) -> String {
    let mut s = String::from("[palette]\n");
    for (k, c) in pal::ordered(map) {
        s += &format!("{} = {}\n", q(&k.to_string()), q(&pal::hex(c)));
    }
    s
}

/// Rewrites only `[palette]` of a game.toml, keeping every other key and comment.
/// `replace`: the table becomes `map` (with `comment` above it); otherwise only the
/// missing keys are appended.
pub fn set_game_palette(text: &str, map: &BTreeMap<String, String>, replace: bool, comment: &str) -> Result<String, String> {
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("game.toml: {e}"))?;
    let mini: toml_edit::DocumentMut = palette_table_text(map).parse().map_err(|e| format!("{e}"))?;
    let mut new_t = mini["palette"].as_table().cloned().ok_or("palette")?;
    let max_pos = doc.iter().filter_map(|(_, i)| i.as_table().and_then(|t| t.position())).max().unwrap_or(0);
    match doc.get_mut("palette").and_then(|i| i.as_table_mut()) {
        Some(t) if !replace => {
            for (k, item) in new_t.iter() {
                if !t.contains_key(k) {
                    if let Some((key, _)) = new_t.get_key_value(k) {
                        t.insert_formatted(key, item.clone());
                    }
                }
            }
        }
        Some(t) => {
            new_t.set_position(t.position().unwrap_or(max_pos + 1));
            new_t.decor_mut().set_prefix(format!("\n{comment}\n"));
            *t = new_t;
        }
        None => {
            new_t.set_position(max_pos + 1);
            new_t.decor_mut().set_prefix(format!("\n{comment}\n"));
            doc.insert("palette", toml_edit::Item::Table(new_t));
        }
    }
    Ok(doc.to_string())
}

/// Replaces `pixels` of a sprite TOML and drops its own `palette` (every colour now
/// comes from the project palette). Other keys and comments stay.
pub fn set_sprite_pixels(text: &str, frames: &[String]) -> Result<String, String> {
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
    let mut arr = toml_edit::Array::new();
    for f in frames {
        let v: toml_edit::Value = block(f).parse().map_err(|e| format!("{e}"))?;
        arr.push(v);
    }
    doc.remove("palette");
    doc["pixels"] = toml_edit::value(arr);
    Ok(doc.to_string())
}

const BACKUPS: &str = ".modengine/palette_backup";

/// Writes game.toml and the remapped sprites, after copying the originals to
/// `.modengine/palette_backup/<time>/`. Returns the backup folder.
pub fn apply(dir: &Path, plan: &Plan, mode: Mode) -> Result<PathBuf, String> {
    let game = dir.join("game.toml");
    let text = std::fs::read_to_string(&game).map_err(|e| format!("{}: {e}", game.display()))?;
    let new_game = set_game_palette(&text, &plan.palette, mode == Mode::Replace, &plan.comment)?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let bdir = dir.join(BACKUPS).join(format!("{stamp:015}"));
    let mut rels = vec!["game.toml".to_string()];
    for s in &plan.sprites {
        rels.push(s.path.strip_prefix(dir).map_err(|_| format!("{} está fuera del proyecto", s.path.display()))?.to_string_lossy().replace('\\', "/"));
    }
    for r in &rels {
        let to = bdir.join(r);
        std::fs::create_dir_all(to.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::copy(dir.join(r), &to).map_err(|e| format!("copia de seguridad de {r}: {e}"))?;
    }
    std::fs::write(bdir.join("files.txt"), rels.join("\n")).map_err(|e| e.to_string())?;
    std::fs::write(&game, new_game).map_err(|e| format!("{}: {e}", game.display()))?;
    for s in &plan.sprites {
        std::fs::write(&s.path, &s.text).map_err(|e| format!("{}: {e}", s.path.display()))?;
    }
    bump();
    Ok(bdir)
}

/// The most recent backup, if any.
pub fn last_backup(dir: &Path) -> Option<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir.join(BACKUPS)).ok()?.flatten().map(|e| e.path()).filter(|p| p.join("files.txt").exists()).collect();
    v.sort();
    v.pop()
}

/// Puts the backed up files back and deletes the backup.
pub fn restore(dir: &Path, backup: &Path) -> Result<usize, String> {
    let list = std::fs::read_to_string(backup.join("files.txt")).map_err(|e| e.to_string())?;
    let mut n = 0;
    for r in list.lines().filter(|l| !l.is_empty() && !l.contains("..")) {
        std::fs::copy(backup.join(r), dir.join(r)).map_err(|e| format!("{r}: {e}"))?;
        n += 1;
    }
    let _ = std::fs::remove_dir_all(backup);
    bump();
    Ok(n)
}

/// Bumped whenever palette files change on disk: open sprite/settings editors without
/// unsaved changes reload themselves.
static GEN: AtomicU64 = AtomicU64::new(0);

pub fn generation() -> u64 {
    GEN.load(Ordering::SeqCst)
}

fn bump() {
    GEN.fetch_add(1, Ordering::SeqCst);
}

/// Name of the saved palette whose colours are exactly the project's (or that it contains).
pub fn describe(game_palette: &BTreeMap<String, String>, saved: &[Saved]) -> String {
    if game_palette.is_empty() {
        return "vacía (los sprites usan Pear36 por defecto)".into();
    }
    let mine: std::collections::BTreeSet<u32> = to_colors(game_palette).values().copied().collect();
    for s in saved {
        let theirs: std::collections::BTreeSet<u32> = s.pal.colors.iter().copied().collect();
        if theirs == mine {
            return s.pal.name.clone();
        }
    }
    for s in saved {
        if s.pal.colors.iter().all(|c| mine.contains(c)) {
            return format!("propia, incluye {}", s.pal.name);
        }
    }
    "propia".into()
}

/// The pixel-art TOML sprites of the project.
pub fn pixel_sprites(p: &Project) -> Vec<(String, PathBuf)> {
    p.sprites.iter().filter(|s| s.is_toml).map(|s| (s.name.clone(), s.path.clone())).collect()
}

// ------------------------------------------------------------------ swatches

/// Clicks on a [`swatch_grid`].
#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub struct Picked {
    pub primary: Option<char>,
    pub secondary: Option<char>,
}

/// A grid of colour swatches (`'.'` = transparent when present). Left click picks the
/// primary colour, right click the secondary; hovering shows the character, hex and index.
pub fn swatch_grid(ui: &mut egui::Ui, entries: &[(char, Option<[u8; 3]>)], primary: char, secondary: char, size: f32, show_chars: bool) -> Picked {
    let mut out = Picked::default();
    let sel = crate::theme::colors(ui.ctx()).selection;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::splat(2.0);
        for (i, (c, col)) in entries.iter().enumerate() {
            let (r, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
            match col {
                Some(col) => {
                    ui.painter().rect_filled(r, 2.0, rgb(*col));
                }
                None => checkerboard(ui.painter(), r, size / 4.0),
            }
            if show_chars && size >= 18.0 {
                let fg = col.map(|c| if (c[0] as u32 * 299 + c[1] as u32 * 587 + c[2] as u32 * 114) > 140_000 { Color32::BLACK } else { Color32::WHITE }).unwrap_or(Color32::GRAY);
                ui.painter().text(r.right_bottom() - Vec2::new(2.0, 1.0), egui::Align2::RIGHT_BOTTOM, c.to_string(), egui::FontId::monospace((size * 0.42).max(9.0)), fg.gamma_multiply(0.8));
            }
            if *c == primary {
                ui.painter().rect_stroke(r, 2.0, Stroke::new(2.0, Color32::WHITE), StrokeKind::Outside);
                ui.painter().rect_stroke(r.expand(2.0), 2.0, Stroke::new(1.0, Color32::BLACK), StrokeKind::Outside);
            }
            if *c == secondary {
                let t = r.left_top();
                ui.painter().add(egui::Shape::convex_polygon(vec![t, t + Vec2::new(size * 0.45, 0.0), t + Vec2::new(0.0, size * 0.45)], sel, Stroke::new(1.0, Color32::BLACK)));
            }
            let tip = match col {
                Some(col) => format!("'{c}'  {}  · índice {}\nclic: color principal · clic derecho: secundario", crate::tomlw::color_hex(*col), PALETTE_CHARS.find(*c).map(|x| x.to_string()).unwrap_or_else(|| i.to_string())),
                None => "'.' transparente\nclic: color principal · clic derecho: secundario".into(),
            };
            let resp = resp.on_hover_text(tip);
            if resp.clicked() {
                out.primary = Some(*c);
            }
            if resp.secondary_clicked() {
                out.secondary = Some(*c);
            }
        }
    });
    out
}

/// A thin strip with every colour of a palette.
pub fn strip(ui: &mut egui::Ui, colors: &[u32], width: f32, height: f32) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let n = colors.len().max(1) as f32;
    for (i, c) in colors.iter().enumerate() {
        let x0 = r.left() + r.width() * i as f32 / n;
        let x1 = r.left() + r.width() * (i + 1) as f32 / n;
        ui.painter().rect_filled(Rect::from_min_max(egui::pos2(x0, r.top()), egui::pos2(x1.ceil(), r.bottom())), 0.0, rgb(rgb3(*c)));
    }
    resp
}

pub fn rgb3(c: u32) -> [u8; 3] {
    [(c >> 16) as u8, (c >> 8) as u8, c as u8]
}

fn frame_image(rows: &str, colors: &BTreeMap<char, u32>) -> egui::ColorImage {
    let lines: Vec<&str> = rows.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let w = lines.iter().map(|l| l.chars().count()).max().unwrap_or(1).max(1);
    let h = lines.len().max(1);
    let mut px = vec![0u8; w * h * 4];
    for (y, l) in lines.iter().enumerate() {
        for (x, c) in l.chars().enumerate() {
            if let Some(col) = colors.get(&c) {
                let [r, g, b] = rgb3(*col);
                px[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&[r, g, b, 255]);
            }
        }
    }
    egui::ColorImage::from_rgba_unmultiplied([w, h], &px)
}

// ------------------------------------------------------------------ the dialog

thread_local! {
    static DIALOG: RefCell<Option<PaletteDialog>> = const { RefCell::new(None) };
}

/// Opens the Paletas dialog (sprite editor, Ajustes del juego, Archivo menu).
pub fn open_dialog() {
    DIALOG.with(|d| {
        d.borrow_mut().get_or_insert_with(|| PaletteDialog::new(palettes_dir(), curl_fetch));
    });
}

/// `--palettes <slug>[:replace]`: open it with this palette (and mode) selected.
pub fn open_dialog_with(sel: &str) {
    open_dialog();
    DIALOG.with(|d| {
        if let Some(d) = d.borrow_mut().as_mut() {
            let (slug, mode) = sel.split_once(':').unwrap_or((sel, ""));
            if let Some(i) = d.saved.iter().position(|s| s.slug == slug) {
                d.sel = i;
            }
            if mode == "replace" {
                d.mode = Mode::Replace;
            }
        }
    });
}

#[cfg(test)]
pub fn is_open() -> bool {
    DIALOG.with(|d| d.borrow().is_some())
}

/// Draws the dialog when open; called once per frame by the app.
pub fn dialog_frame(ctx: &egui::Context, project: Option<&mut Project>, log: &mut Vec<LogLine>) {
    let Some(mut d) = DIALOG.with(|d| d.borrow_mut().take()) else { return };
    if d.show(ctx, project, log) {
        DIALOG.with(|slot| *slot.borrow_mut() = Some(d));
    }
}

pub struct PaletteDialog {
    pub dir: Option<PathBuf>,
    pub saved: Vec<Saved>,
    pub sel: usize,
    pub slug: String,
    pub file: String,
    pub status: Option<(bool, String)>,
    pending: Option<(String, Receiver<Result<Palette, String>>)>,
    pub mode: Mode,
    fetch: Fetch,
    /// (selected slug, mode, generation, game palette) -> plan, and its preview textures.
    plan: Option<((String, Mode, u64, BTreeMap<String, String>), Result<Plan, String>, Vec<(String, TextureHandle, TextureHandle)>)>,
}

impl PaletteDialog {
    pub fn new(dir: Option<PathBuf>, fetch: Fetch) -> Self {
        let saved = list_saved(dir.as_deref());
        PaletteDialog { dir, saved, sel: 0, slug: String::new(), file: String::new(), status: None, pending: None, mode: Mode::Merge, fetch, plan: None }
    }

    fn select_slug(&mut self, slug: &str) {
        self.saved = list_saved(self.dir.as_deref());
        self.sel = self.saved.iter().position(|s| s.slug == slug).unwrap_or(0);
    }

    /// Keeps a palette (downloaded or imported) in the library and selects it.
    pub fn add(&mut self, slug: &str, p: Palette) {
        match &self.dir {
            Some(dir) => match save_palette(dir, slug, &p) {
                Ok(_) => {
                    self.status = Some((true, format!("«{}» guardada ({} colores)", p.name, p.colors.len())));
                    self.select_slug(slug);
                }
                Err(e) => self.status = Some((false, e)),
            },
            None => {
                self.saved.push(Saved { slug: slug.into(), pal: p, path: None });
                self.sel = self.saved.len() - 1;
            }
        }
    }

    pub fn start_download(&mut self) {
        let Some(slug) = pal::lospec_slug(&self.slug) else {
            self.status = Some((false, "escribe el nombre de una paleta de Lospec (p. ej. pear36) o su URL".into()));
            return;
        };
        let (tx, rx) = mpsc::channel();
        let fetch = self.fetch;
        let s = slug.clone();
        std::thread::spawn(move || {
            let _ = tx.send(download(&s, fetch));
        });
        self.status = Some((true, format!("descargando {slug} de lospec.com…")));
        self.pending = Some((slug, rx));
    }

    /// Polls a running download; true when it finished this call.
    pub fn poll(&mut self) -> bool {
        let Some((slug, rx)) = &self.pending else { return false };
        match rx.try_recv() {
            Ok(r) => {
                let slug = slug.clone();
                self.pending = None;
                match r {
                    Ok(p) => self.add(&slug, p),
                    Err(e) => self.status = Some((false, e)),
                }
                true
            }
            Err(mpsc::TryRecvError::Empty) => false,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.pending = None;
                self.status = Some((false, "la descarga falló".into()));
                true
            }
        }
    }

    pub fn import_file(&mut self) {
        let path = crate::import::expand(self.file.trim());
        match std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display())).and_then(|b| Palette::parse_any(&path.to_string_lossy(), &b)) {
            Ok(p) => {
                let slug = slugify(&p.name);
                self.add(&slug, p);
            }
            Err(e) => self.status = Some((false, e)),
        }
    }

    fn current(&self) -> &Saved {
        &self.saved[self.sel.min(self.saved.len() - 1)]
    }

    /// Returns false when the dialog was closed.
    fn show(&mut self, ctx: &egui::Context, mut project: Option<&mut Project>, log: &mut Vec<LogLine>) -> bool {
        if self.poll() {
            ctx.request_repaint();
        }
        if self.pending.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let mut close = false;
        let screen = ctx.content_rect();
        let resp = egui::Modal::new(egui::Id::unique("palette_dialog")).show(ctx, |ui| {
                ui.set_width(860.0_f32.min(screen.width() - 60.0));
                ui.heading("🎨 Paletas (Lospec)");
                ui.add_space(4.0);
                self.top(ui);
                ui.separator();
                let h = (screen.height() - 560.0).clamp(140.0, 300.0);
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(230.0);
                        self.library(ui, h);
                    });
                    ui.add_space(10.0);
                    ui.vertical(|ui| {
                        ui.set_min_height(h);
                        self.details(ui);
                    });
                });
                ui.separator();
                match project.as_deref_mut() {
                    Some(p) => self.project_ui(ui, p, log),
                    None => {
                        ui.label(RichText::new("Abre un proyecto para aplicarle una paleta.").weak());
                    }
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Cerrar").clicked() {
                            close = true;
                        }
                    });
                });
            });
        !(close || resp.should_close())
    }

    fn top(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("palette_top").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label("Lospec");
            ui.horizontal(|ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut self.slug).hint_text("pear36  o  https://lospec.com/palette-list/pear36").desired_width(400.0));
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let ok = pal::lospec_slug(&self.slug).is_some() && self.pending.is_none();
                if (ui.add_enabled(ok, egui::Button::new("⬇ Descargar")).clicked() || enter && ok) && ok {
                    self.start_download();
                }
                if self.pending.is_some() {
                    ui.spinner();
                }
                ui.hyperlink_to("lospec.com", "https://lospec.com/palette-list");
            });
            ui.end_row();
            ui.label("Archivo");
            ui.horizontal(|ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut self.file).hint_text("~/Descargas/paleta.gpl  (.hex .gpl .pal .json .png)").desired_width(400.0));
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let path = crate::import::expand(self.file.trim());
                if (ui.add_enabled(path.is_file(), egui::Button::new("📂 Importar")).clicked() || enter && path.is_file()) && path.is_file() {
                    self.import_file();
                }
            });
            ui.end_row();
        });
        // a tiny browser: palette files in the folder being typed
        let path = crate::import::expand(self.file.trim());
        let folder = if path.is_dir() { Some(path) } else { path.parent().filter(|p| p.is_dir() && !self.file.trim().is_empty()).map(Path::to_path_buf) };
        if let Some(folder) = folder {
            let mut files: Vec<PathBuf> = std::fs::read_dir(&folder)
                .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().and_then(|x| x.to_str()).is_some_and(|x| ["hex", "gpl", "pal", "json", "png"].contains(&x.to_ascii_lowercase().as_str()))).collect())
                .unwrap_or_default();
            files.sort();
            if !files.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("En esta carpeta:").weak());
                    for f in files.iter().take(12) {
                        if ui.link(f.file_name().unwrap_or_default().to_string_lossy()).clicked() {
                            self.file = f.to_string_lossy().to_string();
                            self.import_file();
                        }
                    }
                });
            }
        }
        if let Some((ok, s)) = &self.status {
            let c = crate::theme::colors(ui.ctx());
            ui.colored_label(if *ok { c.ok } else { c.error }, s);
        }
    }

    fn library(&mut self, ui: &mut egui::Ui, h: f32) {
        ui.strong("Paletas guardadas");
        egui::ScrollArea::vertical().id_salt("palette_list").max_height(h).auto_shrink([false, false]).show(ui, |ui| {
            for i in 0..self.saved.len() {
                let s = &self.saved[i];
                let label = if s.path.is_none() { format!("★ {}  ·  {} (por defecto)", s.pal.name, s.pal.colors.len()) } else { format!("{}  ·  {}", s.pal.name, s.pal.colors.len()) };
                let colors = s.pal.colors.clone();
                let r = ui.selectable_label(self.sel == i, label);
                if r.clicked() {
                    self.sel = i;
                }
                strip(ui, &colors, 210.0, 8.0);
                ui.add_space(4.0);
            }
        });
    }

    fn details(&mut self, ui: &mut egui::Ui) {
        let s = self.current().clone();
        ui.heading(&s.pal.name);
        let by = if s.pal.author.is_empty() { String::new() } else { format!("de {} · ", s.pal.author) };
        let src = match &s.path {
            None => "incluida en Mod Engine".to_string(),
            Some(p) => p.display().to_string(),
        };
        ui.label(RichText::new(format!("{by}{} colores · {src}", s.pal.colors.len())).weak());
        if s.pal.colors.len() > pal::max_colors() {
            ui.colored_label(crate::theme::colors(ui.ctx()).error, format!("Solo los primeros {} colores tienen carácter para pixel art.", pal::max_colors()));
        }
        ui.add_space(4.0);
        let entries: Vec<(char, Option<[u8; 3]>)> = s.pal.colors.iter().zip(PALETTE_CHARS.chars()).map(|(c, ch)| (ch, Some(rgb3(*c)))).collect();
        swatch_grid(ui, &entries, '\0', '\0', 30.0, true);
        ui.add_space(4.0);
        ui.label(RichText::new("El carácter de cada color (abajo a la derecha) es el que usan los sprites: 0-9, A-Z, a-z…").weak().small());
        if s.path.is_some() && ui.small_button("🗑 Quitar de la lista").clicked() {
            if let Some(p) = &s.path {
                let _ = std::fs::remove_file(p);
            }
            self.saved = list_saved(self.dir.as_deref());
            self.sel = 0;
        }
    }

    fn project_ui(&mut self, ui: &mut egui::Ui, p: &mut Project, log: &mut Vec<LogLine>) {
        let s = self.current().clone();
        ui.horizontal(|ui| {
            ui.strong("Paleta del proyecto:");
            ui.label(describe(&p.game.palette, &self.saved));
            let cols: Vec<u32> = pal::ordered(&p.game.palette).into_iter().map(|(_, c)| c).collect();
            if !cols.is_empty() {
                strip(ui, &cols, 220.0, 12.0);
            }
        });
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.mode, Mode::Merge, "Añadir los colores que falten").on_hover_text("Los caracteres que ya usan los sprites no cambian.");
            ui.radio_value(&mut self.mode, Mode::Replace, "Reemplazar y ajustar los sprites al color más cercano").on_hover_text("La paleta pasa a ser exactamente esta; cada píxel toma el color más parecido. Se guarda una copia para deshacer.");
        });
        let key = (s.slug.clone(), self.mode, generation(), p.game.palette.clone());
        if self.plan.as_ref().map(|(k, _, _)| k != &key).unwrap_or(true) {
            let plan = plan(&p.game.palette, &pixel_sprites(p), &s.pal, self.mode);
            let mut tex = vec![];
            if let Ok(pl) = &plan {
                let after = to_colors(&pl.palette);
                for c in pl.sprites.iter().take(24) {
                    let b = ui.ctx().load_texture(format!("palprev_b:{}", c.name), frame_image(&c.before.0, &c.before.1), TextureOptions::NEAREST);
                    let a = ui.ctx().load_texture(format!("palprev_a:{}", c.name), frame_image(&c.after, &after), TextureOptions::NEAREST);
                    tex.push((c.name.clone(), b, a));
                }
            }
            self.plan = Some((key, plan, tex));
        }
        let Some((_, plan, tex)) = &self.plan else { return };
        let plan = match plan {
            Ok(pl) => pl.clone(),
            Err(e) => {
                ui.colored_label(crate::theme::colors(ui.ctx()).error, e);
                return;
            }
        };
        let summary = match self.mode {
            Mode::Merge if plan.added == 0 => format!("El proyecto ya tiene todos los colores de {}.", s.pal.name),
            Mode::Merge => format!("Se añadirán {} colores a [palette] de game.toml; los sprites no cambian.", plan.added),
            Mode::Replace => format!("[palette] pasará a ser {} ({} colores) y se ajustarán {} sprite(s) (antes → después):", s.pal.name, plan.palette.len(), plan.sprites.len()),
        };
        ui.label(summary);
        if plan.skipped > 0 {
            ui.colored_label(crate::theme::colors(ui.ctx()).error, format!("{} colores no caben (sin caracteres libres).", plan.skipped));
        }
        if self.mode == Mode::Replace && !tex.is_empty() {
            egui::ScrollArea::horizontal().id_salt("palette_preview").show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (name, b, a) in tex {
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                for t in [b, a] {
                                    let sz = t.size_vec2();
                                    let k = (40.0 / sz.x).min(40.0 / sz.y).max(0.1);
                                    let (r, _) = ui.allocate_exact_size(Vec2::splat(42.0), Sense::hover());
                                    checkerboard(ui.painter(), r, 5.0);
                                    ui.painter().image(t.id(), Rect::from_center_size(r.center(), sz * k), Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                                }
                            });
                            ui.label(RichText::new(name).small());
                        });
                    }
                });
            });
        }
        ui.horizontal(|ui| {
            let can = !(self.mode == Mode::Merge && plan.added == 0);
            if ui.add_enabled(can, crate::theme::primary_button(ui, "Aplicar al proyecto")).clicked() {
                match apply(&p.dir, &plan, self.mode) {
                    Ok(b) => {
                        reload_game(p);
                        let msg = format!("Paleta {} aplicada ({} colores, {} sprites ajustados; copia en {})", s.pal.name, plan.palette.len(), plan.sprites.len(), b.strip_prefix(&p.dir).unwrap_or(&b).display());
                        self.status = Some((true, msg.clone()));
                        log.push(LogLine { kind: LogKind::Ok, text: msg });
                    }
                    Err(e) => {
                        self.status = Some((false, e.clone()));
                        log.push(LogLine { kind: LogKind::Error, text: e });
                    }
                }
            }
            if let Some(b) = last_backup(&p.dir) {
                if ui.button("↶ Deshacer la última aplicación").on_hover_text(format!("Restaura game.toml y los sprites desde {}", b.display())).clicked() {
                    match restore(&p.dir, &b) {
                        Ok(n) => {
                            reload_game(p);
                            let msg = format!("Paleta anterior restaurada ({n} ficheros)");
                            self.status = Some((true, msg.clone()));
                            log.push(LogLine { kind: LogKind::Ok, text: msg });
                        }
                        Err(e) => self.status = Some((false, e)),
                    }
                }
            }
        });
    }
}

/// Re-reads game.toml and the sprites after the palette changed on disk.
pub fn reload_game(p: &mut Project) {
    if let Ok(g) = std::fs::read_to_string(p.dir.join("game.toml")).map_err(|e| e.to_string()).and_then(|t| toml::from_str(&t).map_err(|e| e.to_string())) {
        p.game = g;
    }
    p.reload_sprites();
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;
    use egui_kittest::Harness;
    use modengine::palette::PEAR36 as PEAR36_COLORS;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("modeditor_palette_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn mock_fetch(url: &str) -> Result<Vec<u8>, String> {
        match url {
            "https://lospec.com/palette-list/pear36.json" => Ok(Palette::pear36().to_json().into_bytes()),
            "https://lospec.com/palette-list/tiny-3.json" => Ok(br#"{"name":"Tiny 3","author":"Test","colors":["000000","ff0000","ffffff"]}"#.to_vec()),
            "https://lospec.com/palette-list/broken.json" => Ok(b"<html>not json</html>".to_vec()),
            _ => Err("Lospec no tiene esa paleta (¿está bien escrito el nombre?)".into()),
        }
    }

    #[test]
    fn download_with_mock_and_library() {
        assert_eq!(download("pear36", mock_fetch).unwrap(), Palette::pear36());
        assert_eq!(download("tiny-3", mock_fetch).unwrap().colors, vec![0, 0xff0000, 0xffffff]);
        assert!(download("broken", mock_fetch).unwrap_err().contains("no válida"));
        assert!(download("nope", mock_fetch).unwrap_err().contains("no tiene"));

        let dir = tmp("lib");
        let mut d = PaletteDialog::new(Some(dir.clone()), mock_fetch);
        assert_eq!(d.saved.len(), 1, "only the built-in Pear36");
        d.slug = "https://lospec.com/palette-list/tiny-3".into();
        d.start_download();
        let t = std::time::Instant::now();
        while !d.poll() {
            assert!(t.elapsed().as_secs() < 5);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(dir.join("tiny-3.json").exists(), "cached in the palettes folder");
        assert_eq!(d.current().pal.name, "Tiny 3");
        // a downloaded pear36 doesn't duplicate the built-in one
        save_palette(&dir, "pear36", &Palette::pear36()).unwrap();
        let names: Vec<String> = list_saved(Some(&dir)).iter().map(|s| s.slug.clone()).collect();
        assert_eq!(names, vec!["pear36", "tiny-3"]);
        // import from a file
        std::fs::write(dir.join("mine.gpl"), "GIMP Palette\nName: Mi Paleta\n#\n10 20 30\n40 50 60\n").unwrap();
        d.file = dir.join("mine.gpl").to_string_lossy().to_string();
        d.import_file();
        assert_eq!(d.current().slug, "mi-paleta");
        assert_eq!(d.current().pal.colors, vec![0x0a141e, 0x28323c]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn game_toml_keeps_comments_and_other_keys() {
        let text = "# My game\n[game]\nname = \"x\" # the name\n\n# Characters\n[palette]\n\"k\" = \"#1a1c2c\"   # outline\n\n[display]\nwidth = 320\n";
        let mut map = BTreeMap::new();
        map.insert("k".to_string(), "#1a1c2c".to_string());
        map.insert("0".to_string(), "#5e315b".to_string());
        let merged = set_game_palette(text, &map, false, "# c").unwrap();
        assert!(merged.contains("# My game") && merged.contains("# the name") && merged.contains("# outline") && merged.contains("# Characters"), "{merged}");
        assert!(merged.contains("\"0\" = \"#5e315b\""), "{merged}");
        assert!(merged.find("[display]").unwrap() > merged.find("\"0\"").unwrap(), "new keys stay inside [palette]: {merged}");
        let g: modengine::project::GameToml = toml::from_str(&merged).unwrap();
        assert_eq!(g.palette.len(), 2);
        assert_eq!(g.display.width, 320);

        let replaced = set_game_palette(text, &Palette::pear36().char_map(), true, "# Paleta: Pear36").unwrap();
        assert!(replaced.contains("# My game") && replaced.contains("# Paleta: Pear36") && !replaced.contains("# outline"), "{replaced}");
        let g: modengine::project::GameToml = toml::from_str(&replaced).unwrap();
        assert_eq!(g.palette, Palette::pear36().char_map());
        assert_eq!(g.game.name, "x");
        assert!(replaced.find("\"0\"").unwrap() < replaced.find("\"1\"").unwrap() && replaced.find("\"9\"").unwrap() < replaced.find("\"A\"").unwrap(), "palette order");

        // no [palette] yet: appended at the end
        let added = set_game_palette("[game]\nname = \"y\"\n\n[display]\nwidth = 1\n", &Palette::pear36().char_map(), true, "# P").unwrap();
        assert!(added.find("[palette]").unwrap() > added.find("[display]").unwrap(), "{added}");
        let g: modengine::project::GameToml = toml::from_str(&added).unwrap();
        assert_eq!(g.palette.len(), 36);
    }

    #[test]
    fn replace_remaps_sprites_and_undo_restores() {
        let dir = crate::room::tests::temp_project("palette_apply");
        let mut p = Project::open(&dir).unwrap();
        let game_before = std::fs::read_to_string(dir.join("game.toml")).unwrap();
        let coin_before = std::fs::read_to_string(dir.join("sprites/coin.toml")).unwrap();
        // a sprite with its own palette entry gets folded into the project palette
        std::fs::write(dir.join("sprites/local.toml"), "# keep me\norigin = \"center\"\npalette = { \"z\" = \"#ff0000\" }\npixels = ['''\nzk\nk.\n''']\n").unwrap();
        p.reload_lists();

        let merge = plan(&p.game.palette, &pixel_sprites(&p), &Palette::pear36(), Mode::Merge).unwrap();
        assert!(merge.sprites.is_empty());
        assert_eq!(merge.palette.len(), p.game.palette.len() + merge.added);
        for (k, v) in &p.game.palette {
            assert_eq!(&merge.palette[k], v, "merge keeps '{k}'");
        }

        let pl = plan(&p.game.palette, &pixel_sprites(&p), &Palette::pear36(), Mode::Replace).unwrap();
        assert_eq!(pl.palette, Palette::pear36().char_map());
        assert!(pl.sprites.iter().any(|s| s.name == "coin") && pl.sprites.iter().any(|s| s.name == "local"));
        apply(&dir, &pl, Mode::Replace).unwrap();
        reload_game(&mut p);
        assert_eq!(p.game.palette, Palette::pear36().char_map());
        let game_after = std::fs::read_to_string(dir.join("game.toml")).unwrap();
        assert!(game_after.contains("# Coin Climber") && game_after.contains("[display]") && game_after.contains("Pear36"), "{game_after}");
        let local = std::fs::read_to_string(dir.join("sprites/local.toml")).unwrap();
        assert!(local.contains("# keep me") && local.contains("origin") && !local.contains("palette"), "{local}");
        assert!(local.contains("'''\nRK\nK.\n'''"), "z (#ff0000) -> R (#eb564b), k -> K: {local}");
        // every sprite loads with the new palette (engine loader)
        for s in &p.sprites {
            assert!(s.data.is_ok(), "{}: {:?}", s.name, s.data.as_ref().err());
        }
        let e = modengine::Engine::load(&dir).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(e.config.palette.len(), 36);

        // undo
        let b = last_backup(&dir).expect("backup");
        restore(&dir, &b).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("game.toml")).unwrap(), game_before);
        assert_eq!(std::fs::read_to_string(dir.join("sprites/coin.toml")).unwrap(), coin_before);
        assert!(last_backup(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dialog_ui_lists_and_applies() {
        let dir = crate::room::tests::temp_project("palette_ui");
        let lib = tmp("ui_lib");
        save_palette(&lib, "tiny-3", &Palette { name: "Tiny 3".into(), author: "T".into(), colors: vec![0, 0xff0000, 0xffffff] }).unwrap();
        struct St {
            p: Project,
            log: Vec<LogLine>,
        }
        let lib2 = lib.clone();
        DIALOG.with(|d| *d.borrow_mut() = Some(PaletteDialog::new(Some(lib2), mock_fetch)));
        let mut h = Harness::builder().with_size([1200.0, 900.0]).allow_missing_glyphs().with_accessibility_check(false).build_ui_state(
            |ui, s: &mut St| {
                dialog_frame(ui.ctx(), Some(&mut s.p), &mut s.log);
            },
            St { p: Project::open(&dir).unwrap(), log: vec![] },
        );
        h.run_steps(3);
        assert!(is_open());
        h.get_by_label_contains("Tiny 3");
        h.get_by_label_contains("★ Pear36");
        let b = h.get_by_label("Aplicar al proyecto");
        assert!(b.rect().max.y < 900.0, "the dialog fits: {:?}", b.rect());
        b.click();
        h.run_steps(3);
        let st = DIALOG.with(|d| d.borrow().as_ref().map(|d| format!("{:?} {:?}", d.status, d.plan.as_ref().map(|p| p.1.as_ref().map(|p| (p.added, p.palette.len())).map_err(|e| e.clone())))));
        assert!(h.state().log.iter().any(|l| l.text.contains("Pear36 aplicada")), "merge Pear36 into the example: {st:?}");
        assert!(h.state().log.iter().any(|l| l.text.contains("Pear36 aplicada")), "merge Pear36 into the example");
        assert!(h.state().p.game.palette.len() > 36);
        assert_eq!(h.state().p.game.palette["k"], "#1a1c2c", "merge keeps the example's letters");
        h.get_by_label("↶ Deshacer la última aplicación").click();
        h.run_steps(3);
        assert_eq!(h.state().p.game.palette.len(), 17);
        h.get_by_label("Cerrar").click();
        h.run_steps(2);
        assert!(!is_open());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&lib);
    }

    /// Manual: `cargo test --release -p modengine-editor real_lospec -- --ignored --nocapture`
    /// downloads pear36 and endesga-32 from lospec.com into ~/.config/modengine/palettes.
    #[test]
    #[ignore = "network"]
    fn real_lospec_download() {
        for (input, n) in [("https://lospec.com/palette-list/pear36", 36), ("endesga-32", 32)] {
            let mut d = PaletteDialog::new(palettes_dir(), curl_fetch);
            d.slug = input.into();
            d.start_download();
            let t = std::time::Instant::now();
            while !d.poll() {
                assert!(t.elapsed().as_secs() < 30, "timeout");
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            let s = d.current();
            println!("{input}: {:?} -> «{}» de {} · {} colores · {:?}", d.status, s.pal.name, s.pal.author, s.pal.colors.len(), s.path);
            assert_eq!(s.pal.colors.len(), n);
        }
        assert_eq!(download("pear36", curl_fetch).unwrap().colors, PEAR36_COLORS.to_vec(), "the built-in copy matches Lospec");
        assert!(download("no-such-palette-xyz", curl_fetch).is_err());
    }

    #[test]
    fn slugify_names() {
        assert_eq!(slugify("Mi Paleta 16!"), "mi-paleta-16");
        assert_eq!(slugify("  "), "paleta");
    }
}
