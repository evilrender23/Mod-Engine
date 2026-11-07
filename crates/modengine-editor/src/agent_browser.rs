//! "Adjuntar desde el disco": a file browser for attaching anything on the computer to the
//! agent's next message, not only project files. Places (home, its usual folders, the root,
//! mounted drives), favourites and recent folders on the left; the folder on the right with a
//! filter, hidden files, multi-select (Ctrl/Mayús+clic or the boxes) and a path field that
//! takes typed or pasted paths and `file://` URIs.
//!
//! The same browser is the editor's folder picker ([`Purpose`]): Abrir proyecto (folders
//! with a game.toml show 🎮 and the game's name; game.toml itself can be picked) and
//! Elegir carpeta (where a new project goes).

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
    /// The game's name when this is a Mod Engine project folder (it has a game.toml).
    pub project: Option<String>,
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
            let is_dir = meta.as_ref().is_some_and(|m| m.is_dir());
            let project = if is_dir { crate::projects::project_name(&path) } else { None };
            Some(Row { is_dir, size: meta.map(|m| m.len()).unwrap_or(0), name, path, project })
        })
        .collect();
    out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(out)
}

/// The usual places: home and its well-known folders, the root, and mounted drives.
pub fn places() -> Vec<(String, PathBuf)> {
    let mut out = vec![];
    if let Some(home) = modengine::home_dir() {
        out.push((concat!(fa!(house), " Inicio").to_string(), home.clone()));
        for (label, names) in [
            (concat!(fa!(desktop), " Escritorio"), ["Escritorio", "Desktop"]),
            (concat!(fa!(file), " Documentos"), ["Documentos", "Documents"]),
            (concat!(fa!(download), " Descargas"), ["Descargas", "Downloads"]),
            (concat!(fa!(image), " Imágenes"), ["Imágenes", "Pictures"]),
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
            out.push((format!(concat!(fa!(hard_drive), " {}:"), d as char), p));
        }
    }
    #[cfg(not(windows))]
    {
        out.push((concat!(fa!(hard_drive), " Raíz /").to_string(), PathBuf::from("/")));
        for m in mounts() {
            let name = m.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| m.display().to_string());
            out.push((format!(concat!(fa!(hard_drive), " {}"), crate::agent::one_line(&name, 14)), m));
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
    /// The chosen paths (one for the single-choice purposes).
    Attach(Vec<PathBuf>),
    /// "Usar el diálogo del sistema": the caller opens the native dialog instead.
    Native,
}

/// What the browser is for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Purpose {
    /// Attach any files or folders to the agent's message (multi-select).
    #[default]
    Attach,
    /// Pick a Mod Engine project: a folder with a game.toml, or the game.toml itself.
    OpenProject,
    /// Pick a folder (where a new project goes).
    PickFolder,
}

impl Purpose {
    fn title(self) -> &'static str {
        match self {
            Purpose::Attach => "Adjuntar desde el disco",
            Purpose::OpenProject => "Abrir proyecto",
            Purpose::PickFolder => "Elegir carpeta",
        }
    }

    /// Rows this purpose shows (`Attach`: everything).
    pub fn shows(self, r: &Row) -> bool {
        match self {
            Purpose::Attach => true,
            Purpose::OpenProject => r.is_dir || r.name == "game.toml",
            Purpose::PickFolder => r.is_dir,
        }
    }
}

/// The project a path stands for: a folder with a game.toml, or a game.toml file.
pub fn project_of(path: &Path) -> Option<PathBuf> {
    crate::projects::project_dir(path)
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
    pub purpose: Purpose,
    /// Show "Usar el diálogo del sistema" (a native picker is available).
    pub native: bool,
}

impl Default for Browser {
    fn default() -> Self {
        let home = modengine::home_dir().unwrap_or_else(|| PathBuf::from(if cfg!(windows) { "C:\\" } else { "/" }));
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
            purpose: Purpose::Attach,
            native: false,
        }
    }
}

impl Browser {
    pub fn with_purpose(purpose: Purpose) -> Browser {
        Browser { purpose, ..Default::default() }
    }

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
        let purpose = self.purpose;
        let multi = purpose == Purpose::Attach;
        let id = match purpose {
            Purpose::Attach => "agent_disk_browser",
            Purpose::OpenProject => "open_project_browser",
            Purpose::PickFolder => "pick_folder_browser",
        };
        // Opened from a modal dialog (Nuevo proyecto → 📁 Elegir…): it must sit above it, or it
        // is drawn underneath and the modal blocks its clicks. It has no popups of its own.
        let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::unique(id));
        egui::Window::new(purpose.title())
            .id(layer.id)
            .order(egui::Order::Foreground)
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([720.0, 520.0])
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let up = self.dir.parent().map(Path::to_path_buf);
                    if ui.add_enabled(up.is_some(), egui::Button::new(fa!(arrow_up))).on_hover_text("Carpeta de arriba").clicked() {
                        go_to = up;
                    }
                    let fav = favorites.contains(&self.dir);
                    if ui.selectable_label(fav, if fav { RichText::new(fa!(star)) } else { crate::icons::regular(fa!(star)) }).on_hover_text("Añadir o quitar de favoritos").clicked() {
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
                            .id(egui::Id::unique(id).with("path")),
                    );
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    if ui.button("Ir").clicked() || enter {
                        let t = self.path_text.clone();
                        let files = self.enter_path(&t);
                        // a pasted project folder or game.toml opens straight away
                        if purpose == Purpose::OpenProject {
                            if let Some(p) = files.first().and_then(|f| project_of(f)).or_else(|| files.is_empty().then(|| project_of(&self.dir)).flatten()) {
                                picked = Picked::Attach(vec![p]);
                            }
                        }
                    }
                });
                if purpose == Purpose::OpenProject {
                    if let Some(name) = crate::projects::project_name(&self.dir) {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!(concat!(fa!(gamepad), " Esta carpeta es el proyecto «{}»"), name)).color(crate::theme::colors(ui.ctx()).ok));
                            if ui.small_button("Abrir").clicked() {
                                picked = Picked::Attach(vec![self.dir.clone()]);
                            }
                        });
                    }
                }
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
                                    if ui.selectable_label(self.dir == *p, format!(concat!(fa!(star), " {}"), crate::agent::one_line(&name, 20))).on_hover_text(p.display().to_string()).clicked() {
                                        go_to = Some(p.clone());
                                    }
                                }
                            }
                            if !recent.is_empty() {
                                ui.add_space(4.0);
                                ui.label(RichText::new("Recientes").weak().small());
                                for p in recent.iter().take(8) {
                                    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.display().to_string());
                                    if ui.selectable_label(self.dir == *p, format!(concat!(fa!(clock), " {}"), crate::agent::one_line(&name, 20))).on_hover_text(p.display().to_string()).clicked() {
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
                            let hint = match purpose {
                                Purpose::Attach => "Ctrl/Mayús+clic: varios · doble clic: abrir carpeta",
                                Purpose::OpenProject => concat!(fa!(gamepad), " = proyecto de Mod Engine · doble clic: abrir"),
                                Purpose::PickFolder => "doble clic: entrar en la carpeta",
                            };
                            ui.label(RichText::new(hint).weak().small());
                        });
                        let filter = self.filter.to_lowercase();
                        let rows: Vec<Row> = self.rows().iter().filter(|r| purpose.shows(r) && (filter.is_empty() || r.name.to_lowercase().contains(&filter))).cloned().collect();
                        egui::ScrollArea::vertical().id_salt("agent_disk_rows").max_height(h - 26.0).auto_shrink([false, false]).show(ui, |ui| {
                            if rows.is_empty() {
                                ui.label(RichText::new("(vacía)").weak());
                            }
                            let mods = ui.input(|i| i.modifiers);
                            for (i, r) in rows.iter().enumerate().take(3000) {
                                let sel = self.selected.contains(&r.path);
                                ui.horizontal(|ui| {
                                    let mut on = sel;
                                    if multi && ui.checkbox(&mut on, "").changed() {
                                        if on {
                                            self.selected.insert(r.path.clone());
                                        } else {
                                            self.selected.remove(&r.path);
                                        }
                                        self.anchor = Some(i);
                                    }
                                    let icon = if r.project.is_some() {
                                        fa!(gamepad)
                                    } else if r.is_dir {
                                        fa!(folder)
                                    } else if crate::agent::IMAGE_EXTS.iter().any(|e| r.name.to_lowercase().ends_with(&format!(".{e}"))) {
                                        fa!(image)
                                    } else {
                                        fa!(file)
                                    };
                                    let size = if r.is_dir { String::new() } else { format!("  {}", crate::import::human_size(r.size)) };
                                    let text = match &r.project {
                                        Some(game) if game != &r.name => format!("{icon} {}  —  {game}", r.name),
                                        _ => format!("{icon} {}{size}", r.name),
                                    };
                                    let resp = ui.selectable_label(sel, text);
                                    let project = (purpose == Purpose::OpenProject).then(|| project_of(&r.path)).flatten();
                                    if resp.double_clicked() && project.is_some() {
                                        picked = Picked::Attach(vec![project.clone().unwrap_or_default()]);
                                    } else if resp.double_clicked() && r.is_dir {
                                        go_to = Some(r.path.clone());
                                    } else if resp.clicked() {
                                        if !multi {
                                            self.selected.clear();
                                            self.selected.insert(r.path.clone());
                                        } else if mods.shift {
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
                if purpose != Purpose::Attach {
                    ui.horizontal(|ui| {
                        let sel = self.selected.iter().next().cloned();
                        if purpose == Purpose::OpenProject {
                            let target = sel.as_deref().and_then(project_of);
                            if ui.add_enabled(target.is_some(), crate::theme::primary_button(ui, "Abrir proyecto")).on_disabled_hover_text(concat!("Elige una carpeta con ", fa!(gamepad), " o un game.toml")).clicked() {
                                picked = Picked::Attach(target.into_iter().collect());
                            }
                        } else {
                            let target = sel.filter(|p| p.is_dir()).unwrap_or_else(|| self.dir.clone());
                            let name = target.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| target.display().to_string());
                            if ui.add(crate::theme::primary_button(ui, &format!("Elegir «{name}»"))).on_hover_text(target.display().to_string()).clicked() {
                                picked = Picked::Attach(vec![target]);
                            }
                        }
                        if ui.button("Cancelar").clicked() {
                            picked = Picked::Cancel;
                        }
                        if self.native && ui.button(concat!(fa!(window_restore), " Usar el diálogo del sistema")).on_hover_text("El selector de carpetas de tu escritorio (KDE, GNOME, Windows, macOS)").clicked() {
                            picked = Picked::Native;
                        }
                    });
                } else {
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
                }
            });
        ctx.move_to_top(layer);
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
