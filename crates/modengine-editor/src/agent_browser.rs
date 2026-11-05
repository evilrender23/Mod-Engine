//! "Adjuntar desde el disco": a file browser for attaching anything on the computer to the
//! agent's next message, not only project files. Places (home, its usual folders, the root,
//! mounted drives), favourites and recent folders on the left; the folder on the right with a
//! filter, hidden files, multi-select (Ctrl/Mayús+clic or the boxes) and a path field that
//! takes typed or pasted paths and `file://` URIs.

use eframe::egui::{self, Key, RichText};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// One row of the folder list.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
}

/// Every entry of `dir` (hidden ones when `hidden`), folders first, by name.
pub fn list(dir: &Path, hidden: bool) -> Result<Vec<Row>, String> {
    let rd = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut out: Vec<Row> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') && !hidden {
                return None;
            }
            let path = e.path();
            // Follows links, so a link to a folder opens as one.
            let meta = std::fs::metadata(&path).ok();
            Some(Row { is_dir: meta.as_ref().is_some_and(|m| m.is_dir()), size: meta.map(|m| m.len()).unwrap_or(0), name, path })
        })
        .collect();
    out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(out)
}

/// The usual places: home and its well-known folders, the root, and mounted drives.
pub fn places() -> Vec<(String, PathBuf)> {
    let mut out = vec![];
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from) {
        out.push(("🏠 Inicio".to_string(), home.clone()));
        for (label, names) in [
            ("🖥 Escritorio", ["Escritorio", "Desktop"]),
            ("📄 Documentos", ["Documentos", "Documents"]),
            ("⬇ Descargas", ["Descargas", "Downloads"]),
            ("🖼 Imágenes", ["Imágenes", "Pictures"]),
        ] {
            if let Some(p) = names.iter().map(|n| home.join(n)).find(|p| p.is_dir()) {
                out.push((label.to_string(), p));
            }
        }
    }
    #[cfg(windows)]
    for d in b'C'..=b'Z' {
        let p = PathBuf::from(format!("{}:\\", d as char));
        if p.is_dir() {
            out.push((format!("💽 {}:", d as char), p));
        }
    }
    #[cfg(not(windows))]
    {
        out.push(("💽 Raíz /".to_string(), PathBuf::from("/")));
        for m in mounts() {
            let name = m.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| m.display().to_string());
            out.push((format!("💾 {}", crate::agent::one_line(&name, 14)), m));
        }
    }
    out
}

/// Mounted drives a person would browse: under /mnt, /media and /run/media (and /Volumes).
#[cfg(not(windows))]
pub fn mounts() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_to_string("/proc/mounts")
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_whitespace().nth(1))
        .map(|p| PathBuf::from(p.replace("\\040", " ")))
        .filter(|p| ["/mnt/", "/media/", "/run/media/"].iter().any(|r| p.to_string_lossy().starts_with(r)))
        .collect();
    if let Ok(rd) = std::fs::read_dir("/Volumes") {
        out.extend(rd.flatten().map(|e| e.path()));
    }
    out.sort();
    out.dedup();
    out
}

/// What [`Browser::show`] returns.
#[derive(Debug, PartialEq)]
pub enum Picked {
    Nothing,
    Cancel,
    Attach(Vec<PathBuf>),
}

pub struct Browser {
    pub open: bool,
    pub dir: PathBuf,
    /// The path field (may be a pasted `file://` URI).
    pub path_text: String,
    pub filter: String,
    pub hidden: bool,
    pub selected: BTreeSet<PathBuf>,
    rows: Vec<Row>,
    listed: Option<(PathBuf, bool)>,
    pub error: Option<String>,
    /// Last clicked row, for Mayús+clic ranges.
    anchor: Option<usize>,
}

impl Default for Browser {
    fn default() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
        Browser {
            open: false,
            path_text: home.display().to_string(),
            dir: home,
            filter: String::new(),
            hidden: false,
            selected: BTreeSet::new(),
            rows: vec![],
            listed: None,
            error: None,
            anchor: None,
        }
    }
}

impl Browser {
    /// Opens at `dir` (when it exists).
    pub fn open_at(&mut self, dir: Option<&Path>) {
        self.open = true;
        self.selected.clear();
        if let Some(d) = dir.filter(|d| d.is_dir()) {
            self.go(d);
        }
    }

    pub fn go(&mut self, dir: &Path) {
        self.dir = dir.to_path_buf();
        self.path_text = dir.display().to_string();
        self.filter.clear();
        self.anchor = None;
        self.listed = None;
    }

    /// Typed or pasted text in the path field: a folder opens; a file (or several, one per
    /// line, e.g. pasted `file://` URIs) is selected in its folder.
    pub fn enter_path(&mut self, text: &str) -> Vec<PathBuf> {
        let paths: Vec<PathBuf> = text.lines().map(str::trim).filter(|l| !l.is_empty()).map(crate::import::expand).collect();
        let existing: Vec<PathBuf> = paths.iter().filter(|p| p.exists()).cloned().collect();
        if existing.is_empty() {
            self.error = Some(format!("No existe: {}", text.trim()));
            return vec![];
        }
        self.error = None;
        if existing.len() == 1 && existing[0].is_dir() {
            self.go(&existing[0]);
            return vec![];
        }
        if let Some(parent) = existing[0].parent() {
            self.go(parent);
        }
        self.selected = existing.iter().cloned().collect();
        existing
    }

    pub fn rows(&mut self) -> &[Row] {
        if self.listed.as_ref() != Some(&(self.dir.clone(), self.hidden)) {
            match list(&self.dir, self.hidden) {
                Ok(r) => {
                    self.rows = r;
                    self.error = None;
                }
                Err(e) => {
                    self.rows.clear();
                    self.error = Some(e);
                }
            }
            self.listed = Some((self.dir.clone(), self.hidden));
        }
        &self.rows
    }

    /// The window. `favorites` and `recent` are remembered by the panel; ★ adds or removes the
    /// current folder from the favourites.
    pub fn show(&mut self, ctx: &egui::Context, favorites: &mut Vec<PathBuf>, recent: &[PathBuf]) -> Picked {
        if !self.open {
            return Picked::Nothing;
        }
        let mut picked = Picked::Nothing;
        let mut open = true;
        let mut go_to: Option<PathBuf> = None;
        egui::Window::new("Adjuntar desde el disco")
            .id(egui::Id::unique("agent_disk_browser"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([720.0, 520.0])
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let up = self.dir.parent().map(Path::to_path_buf);
                    if ui.add_enabled(up.is_some(), egui::Button::new("⬆")).on_hover_text("Carpeta de arriba").clicked() {
                        go_to = up;
                    }
                    let fav = favorites.contains(&self.dir);
                    if ui.selectable_label(fav, if fav { "★" } else { "☆" }).on_hover_text("Añadir o quitar de favoritos").clicked() {
                        if fav {
                            favorites.retain(|f| *f != self.dir);
                        } else {
                            favorites.push(self.dir.clone());
                        }
                    }
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut self.path_text)
                            .hint_text("Ruta, ~/…, o pega file://…")
                            .desired_width(ui.available_width() - 70.0)
                            .id(egui::Id::unique("agent_disk_path")),
                    );
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    if ui.button("Ir").clicked() || enter {
                        let t = self.path_text.clone();
                        self.enter_path(&t);
                    }
                });
                if let Some(e) = &self.error {
                    ui.label(RichText::new(e).color(crate::theme::colors(ui.ctx()).error));
                }
                ui.separator();
                // Fixed list height, so the window never outgrows the screen.
                let h = (ctx.content_rect().height() - 260.0).clamp(160.0, 420.0);
                ui.horizontal_top(|ui| {
                    // Places.
                    ui.allocate_ui_with_layout(egui::vec2(170.0, h), egui::Layout::top_down(egui::Align::Min), |ui| {
                        egui::ScrollArea::vertical().id_salt("agent_disk_places").max_height(h).show(ui, |ui| {
                            ui.set_width(165.0);
                            ui.label(RichText::new("Lugares").weak().small());
                            for (label, p) in places() {
                                if ui.selectable_label(self.dir == p, label).on_hover_text(p.display().to_string()).clicked() {
                                    go_to = Some(p);
                                }
                            }
                            if !favorites.is_empty() {
                                ui.add_space(4.0);
                                ui.label(RichText::new("Favoritos").weak().small());
                                for p in favorites.iter() {
                                    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.display().to_string());
                                    if ui.selectable_label(self.dir == *p, format!("★ {}", crate::agent::one_line(&name, 20))).on_hover_text(p.display().to_string()).clicked() {
                                        go_to = Some(p.clone());
                                    }
                                }
                            }
                            if !recent.is_empty() {
                                ui.add_space(4.0);
                                ui.label(RichText::new("Recientes").weak().small());
                                for p in recent.iter().take(8) {
                                    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.display().to_string());
                                    if ui.selectable_label(self.dir == *p, format!("🕘 {}", crate::agent::one_line(&name, 20))).on_hover_text(p.display().to_string()).clicked() {
                                        go_to = Some(p.clone());
                                    }
                                }
                            }
                        });
                    });
                    ui.add_sized([6.0, h], egui::Separator::default().vertical());
                    // The folder.
                    ui.vertical(|ui| {
                        ui.set_max_height(h);
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("Filtrar…").desired_width(180.0));
                            if ui.checkbox(&mut self.hidden, "Ocultos").changed() {
                                self.listed = None;
                            }
                            ui.label(RichText::new("Ctrl/Mayús+clic: varios · doble clic: abrir carpeta").weak().small());
                        });
                        let filter = self.filter.to_lowercase();
                        let rows: Vec<Row> = self.rows().iter().filter(|r| filter.is_empty() || r.name.to_lowercase().contains(&filter)).cloned().collect();
                        egui::ScrollArea::vertical().id_salt("agent_disk_rows").max_height(h - 26.0).auto_shrink([false, false]).show(ui, |ui| {
                            if rows.is_empty() {
                                ui.label(RichText::new("(vacía)").weak());
                            }
                            let mods = ui.input(|i| i.modifiers);
                            for (i, r) in rows.iter().enumerate().take(3000) {
                                let sel = self.selected.contains(&r.path);
                                ui.horizontal(|ui| {
                                    let mut on = sel;
                                    if ui.checkbox(&mut on, "").changed() {
                                        if on {
                                            self.selected.insert(r.path.clone());
                                        } else {
                                            self.selected.remove(&r.path);
                                        }
                                        self.anchor = Some(i);
                                    }
                                    let icon = if r.is_dir {
                                        "📁"
                                    } else if crate::agent::IMAGE_EXTS.iter().any(|e| r.name.to_lowercase().ends_with(&format!(".{e}"))) {
                                        "🖼"
                                    } else {
                                        "📄"
                                    };
                                    let size = if r.is_dir { String::new() } else { format!("  {}", crate::import::human_size(r.size)) };
                                    let resp = ui.selectable_label(sel, format!("{icon} {}{size}", r.name));
                                    if resp.double_clicked() && r.is_dir {
                                        go_to = Some(r.path.clone());
                                    } else if resp.clicked() {
                                        if mods.shift {
                                            if let Some(a) = self.anchor {
                                                let (lo, hi) = (a.min(i), a.max(i));
                                                for row in &rows[lo..=hi] {
                                                    self.selected.insert(row.path.clone());
                                                }
                                            }
                                        } else if mods.command || mods.ctrl {
                                            if sel {
                                                self.selected.remove(&r.path);
                                            } else {
                                                self.selected.insert(r.path.clone());
                                            }
                                        } else {
                                            self.selected.clear();
                                            self.selected.insert(r.path.clone());
                                        }
                                        self.anchor = Some(i);
                                    }
                                });
                            }
                        });
                    });
                });
                ui.separator();
                ui.horizontal(|ui| {
                    let n = self.selected.len();
                    let label = match n {
                        0 => "Adjuntar".to_string(),
                        1 => "Adjuntar 1 elemento".to_string(),
                        n => format!("Adjuntar {n} elementos"),
                    };
                    if ui.add_enabled(n > 0, crate::theme::primary_button(ui, &label)).clicked() {
                        picked = Picked::Attach(self.selected.iter().cloned().collect());
                    }
                    if ui.button("Adjuntar esta carpeta").on_hover_text(self.dir.display().to_string()).clicked() {
                        picked = Picked::Attach(vec![self.dir.clone()]);
                    }
                    if ui.button("Cancelar").clicked() {
                        picked = Picked::Cancel;
                    }
                    ui.label(RichText::new("Las carpetas llegan como ruta, con permiso de lectura para todo lo de dentro").weak().small());
                });
            });
        if let Some(d) = go_to {
            self.go(&d);
        }
        if !open {
            picked = Picked::Cancel;
        }
        if picked != Picked::Nothing {
            self.open = false;
            self.selected.clear();
        }
        picked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_and_enters_paths() {
        let d = std::env::temp_dir().join(format!("modeditor_disk_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("Sub")).unwrap();
        std::fs::write(d.join("b.txt"), "x").unwrap();
        std::fs::write(d.join("a.png"), "x").unwrap();
        std::fs::write(d.join(".oculto"), "x").unwrap();
        let names = |h| list(&d, h).unwrap().into_iter().map(|r| r.name).collect::<Vec<_>>();
        assert_eq!(names(false), ["Sub", "a.png", "b.txt"]);
        assert_eq!(names(true), ["Sub", ".oculto", "a.png", "b.txt"]);

        let mut b = Browser::default();
        // A folder opens; pasted file:// URIs select their files.
        assert!(b.enter_path(&d.join("Sub").display().to_string()).is_empty());
        assert_eq!(b.dir, d.join("Sub"));
        let uris = format!("file://{}\nfile://{}", d.join("a.png").display(), d.join("b.txt").display());
        assert_eq!(b.enter_path(&uris), [d.join("a.png"), d.join("b.txt")]);
        assert_eq!(b.dir, d);
        assert_eq!(b.selected.len(), 2);
        assert!(b.enter_path("/no/such/thing").is_empty() && b.error.is_some());
        assert!(places().iter().any(|(l, _)| l.contains("Inicio")));
        let _ = std::fs::remove_dir_all(&d);
    }
}
