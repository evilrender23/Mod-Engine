//! Tienda de assets: browse, preview and import free assets from OpenGameArt.org.
//!
//! A large modal (Archivo → Tienda de assets, Ver → Tienda, or the 🛒 toolbar button) with a
//! search bar, type / licence / order filters and a grid of results; an asset's page shows
//! its previews, licences (with what they ask of you), description and files. "Importar"
//! downloads the file into the cache and hands it to the regular Importar assets dialog
//! (sprite strip / grid / origin…); a .zip first lists its contents to pick from.
//!
//! Attribution: what is imported from here is added to the project's CREDITS.md
//! ([`credit_imported`], called by the app after the import dialog ran).
//!
//! The network side (polite rate-limited client, disk cache), the parsing of OGA pages, zips
//! and CREDITS.md live in the `modengine-store` crate (`net`, `oga`, `zip`, `credits`), shared
//! with `modengine store`, the command line the coding agent uses.

use crate::app::{LogKind, LogLine};
use crate::import::{human_size, Kind};
use crate::store_credits::Credit;
use crate::store_net::{self as net, Busy, DiskCache, Http, Job, Reply, Worker};
use crate::store_oga::{blocking_note, license_notes, ArtKind, Asset, FileLink, Hit, License, Note, Query, SearchPage, Sort};
use crate::store_zip::{self as zip, ZipItem};
use eframe::egui::{self, Color32, Rect, RichText, Sense, TextureHandle, TextureOptions, Vec2};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

// ------------------------------------------------------------------ preferences

/// A favourite asset (enough to show its card without asking OGA).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fav {
    pub slug: String,
    pub title: String,
    #[serde(default)]
    pub thumb: Option<String>,
}

/// `~/.config/modengine/store.toml`: last search and favourites.
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Prefs {
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub licenses: Vec<String>,
    #[serde(default)]
    pub sort: String,
    #[serde(default)]
    pub favorites: Vec<Fav>,
}

pub fn config_file() -> Option<PathBuf> {
    modengine::home_dir().map(|h| h.join(".config/modengine/store.toml"))
}

impl Prefs {
    pub fn load(path: Option<&Path>) -> Prefs {
        path.and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let t = toml::to_string(self).map_err(|e| e.to_string())?;
        std::fs::write(path, format!("# Tienda de assets del editor de Mod Engine: última búsqueda y favoritos\n{t}")).map_err(|e| e.to_string())
    }

    fn query(&self) -> Query {
        let mut q = Query { keys: self.query.clone(), ..Default::default() };
        if let Some(k) = ArtKind::from_key(&self.kind) {
            q.kind = k;
        }
        if let Some(s) = Sort::from_key(&self.sort) {
            q.sort = s;
        }
        let ls: BTreeSet<License> = self.licenses.iter().filter_map(|k| License::from_key(k)).collect();
        if !ls.is_empty() {
            q.licenses = ls;
        }
        q
    }
}

// ------------------------------------------------------------------ pending attributions

thread_local! {
    static DIALOG: RefCell<Option<StoreDialog>> = const { RefCell::new(None) };
    /// Files handed to the import dialog → where they came from.
    static PENDING: RefCell<HashMap<PathBuf, Credit>> = RefCell::new(HashMap::new());
}

pub fn remember_credit(path: &Path, c: Credit) {
    PENDING.with(|p| p.borrow_mut().insert(path.to_path_buf(), c));
}

/// After the import dialog ran: adds what came from the store to CREDITS.md and says so.
pub fn credit_imported(project: &Path, done: &[crate::import::Imported]) -> Vec<LogLine> {
    let mut out = vec![];
    for imp in done {
        let Some(c) = PENDING.with(|p| p.borrow_mut().remove(&imp.src)) else { continue };
        match crate::store_credits::write(project, &c, &imp.files) {
            Ok(_) => {
                out.push(LogLine { kind: LogKind::Ok, text: format!("Créditos: {} → CREDITS.md", c.one_line()) });
                let ls: Vec<License> = c.licenses.iter().map(|(n, _)| License::parse(n)).collect();
                if ls.is_empty() || ls.iter().all(|l| l.requires_credit()) {
                    out.push(LogLine { kind: LogKind::Info, text: format!("'{}' exige dar crédito a {}: muestra CREDITS.md en tu juego (pantalla de créditos, web o README).", imp.name, if c.author.is_empty() { "su autor" } else { &c.author }) });
                }
                if ls.iter().all(License::share_alike) && !ls.is_empty() {
                    out.push(LogLine { kind: LogKind::Info, text: format!("'{}' es de licencia «compartir igual»: si lo modificas, publica tu versión con la misma licencia.", imp.name) });
                }
            }
            Err(e) => out.push(LogLine { kind: LogKind::Error, text: format!("no se pudo escribir CREDITS.md: {e}") }),
        }
    }
    out
}

// ------------------------------------------------------------------ app hooks

/// Opens the store (menus, toolbar).
pub fn open_dialog() {
    DIALOG.with(|d| {
        d.borrow_mut().get_or_insert_with(StoreDialog::real);
    });
}

/// `--store <query>` / `--store-open <slug>`: open it searching / on an asset page.
pub fn open_with(query: Option<&str>, slug: Option<&str>) {
    open_dialog();
    DIALOG.with(|d| {
        if let Some(d) = d.borrow_mut().as_mut() {
            if let Some(q) = query {
                d.q.keys = q.to_string();
                d.q.page = 0;
                d.start_search = true;
            }
            if let Some(s) = slug {
                d.start_open = Some(s.to_string());
            }
        }
    });
}

/// Something is still loading (automated screenshots wait for it).
pub fn busy() -> bool {
    DIALOG.with(|d| d.borrow().as_ref().is_some_and(|d| d.loading()))
}

/// Draws the store when open. Returns files to put in the import dialog.
pub fn dialog_frame(ctx: &egui::Context, can_import: bool) -> Vec<PathBuf> {
    let Some(mut d) = DIALOG.with(|d| d.borrow_mut().take()) else { return vec![] };
    let (open, out) = d.show(ctx, can_import);
    if open {
        DIALOG.with(|slot| *slot.borrow_mut() = Some(d));
    }
    out
}

// ------------------------------------------------------------------ the dialog

#[derive(Clone, Debug, PartialEq)]
pub enum View {
    Results,
    Favorites,
    Detail(String),
}

enum Img {
    Loading,
    Ready(TextureHandle),
    Failed,
}

#[derive(Clone, Debug, PartialEq)]
enum Dl {
    Running,
    Done(PathBuf),
    Failed(String),
}

pub struct ZipPick {
    pub slug: String,
    pub file: FileLink,
    pub path: PathBuf,
    pub items: Vec<(ZipItem, bool)>,
    pub error: Option<String>,
}

pub struct StoreDialog {
    http: Arc<dyn Http>,
    cache: Option<DiskCache>,
    interval: Duration,
    net: Option<Worker>,
    config: Option<PathBuf>,
    pub q: Query,
    pub favorites: Vec<Fav>,
    /// The search whose results are shown / awaited.
    asked: Option<Query>,
    pub results: Option<Result<SearchPage, String>>,
    images: HashMap<String, Img>,
    pub details: HashMap<String, Result<Asset, String>>,
    pub view: View,
    downloads: HashMap<String, Dl>,
    /// Downloads to continue with once they finish: url → (slug, file).
    then_import: HashMap<String, (String, FileLink)>,
    pub zip: Option<ZipPick>,
    confirm_big: Option<String>,
    big_ok: BTreeSet<String>,
    /// Assets whose unreadable licence the user acknowledged.
    ack: BTreeSet<String>,
    pub status: Option<(bool, String)>,
    out: Vec<PathBuf>,
    start_search: bool,
    start_open: Option<String>,
    /// Jobs pushed whose reply hasn't been read yet.
    inflight: usize,
}

impl StoreDialog {
    pub fn new(http: Arc<dyn Http>, cache: Option<DiskCache>, interval: Duration, config: Option<PathBuf>) -> StoreDialog {
        let prefs = Prefs::load(config.as_deref());
        StoreDialog {
            http,
            cache,
            interval,
            net: None,
            q: prefs.query(),
            favorites: prefs.favorites,
            config,
            asked: None,
            results: None,
            images: HashMap::new(),
            details: HashMap::new(),
            view: View::Results,
            downloads: HashMap::new(),
            then_import: HashMap::new(),
            zip: None,
            confirm_big: None,
            big_ok: BTreeSet::new(),
            ack: BTreeSet::new(),
            status: None,
            out: vec![],
            start_search: false,
            start_open: None,
            inflight: 0,
        }
    }

    fn real() -> StoreDialog {
        StoreDialog::new(Arc::new(net::Curl), net::cache_dir().map(|d| DiskCache::new(d, net::CACHE_TTL)), net::MIN_INTERVAL, config_file())
    }

    fn worker(&mut self, ctx: &egui::Context) -> &Worker {
        if self.net.is_none() {
            self.net = Some(Worker::start(self.http.clone(), self.cache.clone(), self.interval, Some({
                let ctx = ctx.clone();
                Box::new(move || ctx.request_repaint())
            })));
        }
        self.net.as_ref().unwrap()
    }

    fn push(&mut self, ctx: &egui::Context, job: Job) {
        if self.worker(ctx).push(job) {
            self.inflight += 1;
        }
    }

    pub fn loading(&self) -> bool {
        self.start_search || self.start_open.is_some() || self.inflight > 0 || self.images.values().any(|i| matches!(i, Img::Loading))
    }

    fn save_prefs(&self) {
        let Some(path) = &self.config else { return };
        let p = Prefs {
            query: self.q.keys.clone(),
            kind: self.q.kind.key().into(),
            licenses: self.q.licenses.iter().map(License::key).collect(),
            sort: self.q.sort.key().into(),
            favorites: self.favorites.clone(),
        };
        let _ = p.save(path);
    }

    pub fn search(&mut self, ctx: &egui::Context) {
        if self.q.licenses.is_empty() {
            self.status = Some((false, "elige al menos una licencia".into()));
            return;
        }
        self.status = None;
        self.view = View::Results;
        self.asked = Some(self.q.clone());
        self.results = None;
        let dropped = self.worker(ctx).drop_images();
        self.inflight = self.inflight.saturating_sub(dropped);
        self.images.retain(|_, i| !matches!(i, Img::Loading));
        let job = Job::Search(self.q.clone());
        self.push(ctx, job);
        self.save_prefs();
    }

    fn want_image(&mut self, ctx: &egui::Context, url: &str) {
        if url.is_empty() || self.images.contains_key(url) {
            return;
        }
        self.images.insert(url.to_string(), Img::Loading);
        self.push(ctx, Job::Image(url.to_string()));
    }

    pub fn open_detail(&mut self, ctx: &egui::Context, slug: &str) {
        self.view = View::Detail(slug.to_string());
        self.zip = None;
        self.confirm_big = None;
        match self.details.get(slug) {
            Some(Ok(a)) => {
                let imgs: Vec<String> = a.previews.iter().take(4).map(|p| p.image.clone()).collect();
                for u in imgs {
                    self.want_image(ctx, &u);
                }
            }
            _ => {
                self.details.remove(slug);
                self.push(ctx, Job::Detail(slug.to_string()));
            }
        }
    }

    fn toggle_fav(&mut self, slug: &str, title: &str, thumb: Option<String>) {
        if let Some(i) = self.favorites.iter().position(|f| f.slug == slug) {
            self.favorites.remove(i);
        } else {
            self.favorites.push(Fav { slug: slug.into(), title: title.into(), thumb });
        }
        self.save_prefs();
    }

    fn is_fav(&self, slug: &str) -> bool {
        self.favorites.iter().any(|f| f.slug == slug)
    }

    /// Replies from the worker.
    pub fn poll(&mut self, ctx: &egui::Context) {
        let Some(n) = &self.net else { return };
        let replies: Vec<Reply> = n.rx.try_iter().collect();
        self.inflight = self.inflight.saturating_sub(replies.len());
        for r in replies {
            match r {
                Reply::Search(q, r) => {
                    if self.asked.as_ref() != Some(&q) {
                        continue; // an older search
                    }
                    if let Ok(p) = &r {
                        let thumbs: Vec<String> = p.hits.iter().filter_map(|h| h.thumb.clone()).collect();
                        for t in thumbs {
                            self.want_image(ctx, &t);
                        }
                    }
                    self.results = Some(r);
                }
                Reply::Detail(slug, r) => {
                    let imgs: Vec<String> = r.as_ref().map(|a| a.previews.iter().take(4).map(|p| p.image.clone()).collect()).unwrap_or_default();
                    self.details.insert(slug, r);
                    for u in imgs {
                        self.want_image(ctx, &u);
                    }
                }
                Reply::Image(url, r) => {
                    let img = match r {
                        Ok(rgba) => Img::Ready(ctx.load_texture(format!("store:{url}"), egui::ColorImage::from_rgba_unmultiplied(rgba.size, &rgba.pixels), TextureOptions::NEAREST)),
                        Err(_) => Img::Failed,
                    };
                    self.images.insert(url, img);
                }
                Reply::Download(url, r) => {
                    match r {
                        Ok(p) => {
                            self.downloads.insert(url.clone(), Dl::Done(p.clone()));
                            if let Some((slug, file)) = self.then_import.remove(&url) {
                                self.downloaded(&slug, &file, &p);
                            }
                        }
                        Err(e) => {
                            self.then_import.remove(&url);
                            self.status = Some((false, format!("no se pudo descargar: {e}")));
                            self.downloads.insert(url, Dl::Failed(e));
                        }
                    }
                }
            }
        }
    }

    /// The "Importar" button of a file.
    pub fn import_file(&mut self, ctx: &egui::Context, slug: &str, file: &FileLink) {
        let Some(Ok(a)) = self.details.get(slug) else { return };
        if !file.is_zip() && Kind::of(Path::new(&file.name)).is_none() {
            self.status = Some((false, format!("{}: el motor no carga este formato (usa PNG, WAV, OGG, TTF u OTF)", file.name)));
            return;
        }
        if blocking_note(&a.licenses).is_some() && !self.ack.contains(slug) {
            self.status = Some((false, "confirma antes que has revisado la licencia (casilla de arriba)".into()));
            return;
        }
        if file.size.is_some_and(|s| s > net::BIG_DOWNLOAD) && !self.big_ok.contains(&file.url) {
            self.confirm_big = Some(file.url.clone());
            return;
        }
        if let Some(Dl::Done(p)) = self.downloads.get(&file.url) {
            if p.is_file() {
                let p = p.clone();
                self.downloaded(slug, file, &p);
                return;
            }
        }
        if self.downloads.get(&file.url) == Some(&Dl::Running) {
            return;
        }
        self.status = None;
        self.downloads.insert(file.url.clone(), Dl::Running);
        self.then_import.insert(file.url.clone(), (slug.to_string(), file.clone()));
        self.push(ctx, Job::Download { url: file.url.clone(), name: file.name.clone(), size: file.size });
    }

    /// A file of `slug` is on disk: a zip opens its picker, anything else goes to the
    /// import dialog with its credit remembered.
    fn downloaded(&mut self, slug: &str, file: &FileLink, path: &Path) {
        let Some(Ok(a)) = self.details.get(slug) else { return };
        if file.is_zip() {
            let r = std::fs::read(path).map_err(|e| e.to_string()).and_then(|b| zip::list(&b));
            let (items, error) = match r {
                Ok(items) => {
                    let n = items.iter().filter(|i| i.importable()).count();
                    // preselect what can be imported when it is a handful
                    (items.into_iter().map(|i| { let sel = i.importable() && n <= 12; (i, sel) }).collect(), None)
                }
                Err(e) => (vec![], Some(e)),
            };
            self.zip = Some(ZipPick { slug: slug.into(), file: file.clone(), path: path.to_path_buf(), items, error });
            return;
        }
        remember_credit(path, Credit::from_asset(a, &file.name));
        self.out.push(path.to_path_buf());
    }

    /// Extracts the chosen entries of the open zip and hands them over.
    pub fn import_zip_selection(&mut self) {
        let Some(z) = &self.zip else { return };
        let Some(Ok(a)) = self.details.get(&z.slug) else { return };
        let chosen: Vec<&ZipItem> = z.items.iter().filter(|(_, s)| *s).map(|(i, _)| i).collect();
        if chosen.is_empty() {
            return;
        }
        let dir = match &self.cache {
            Some(c) => c.dir.join("extract").join(net::hash(&z.file.url)),
            None => std::env::temp_dir().join("modengine-store").join(net::hash(&z.file.url)),
        };
        let r = std::fs::read(&z.path).map_err(|e| e.to_string()).and_then(|b| zip::extract_to(&b, &chosen, &dir));
        match r {
            Ok(paths) => {
                for (p, it) in paths.iter().zip(&chosen) {
                    remember_credit(p, Credit::from_asset(a, &format!("{} → {}", z.file.name, it.name)));
                }
                self.out.extend(paths);
                self.zip = None;
            }
            Err(e) => {
                if let Some(z) = &mut self.zip {
                    z.error = Some(e);
                }
            }
        }
    }

    // ------------------------------------------------------------ ui

    /// Returns `(still open, files to import)`.
    pub fn show(&mut self, ctx: &egui::Context, can_import: bool) -> (bool, Vec<PathBuf>) {
        if std::mem::take(&mut self.start_search) {
            self.search(ctx);
        }
        if let Some(s) = self.start_open.take() {
            self.open_detail(ctx, &s);
        }
        self.poll(ctx);
        if self.inflight > 0 {
            ctx.request_repaint_after(Duration::from_millis(150));
        }
        let mut close = false;
        let screen = ctx.content_rect();
        let width = (screen.width() - 60.0).clamp(640.0, 1320.0);
        let body_h = (screen.height() - 260.0).clamp(260.0, 900.0);
        let resp = egui::Modal::new(egui::Id::unique("store_dialog")).show(ctx, |ui| {
            ui.set_width(width);
            ui.horizontal(|ui| {
                ui.heading(concat!(fa!(cart_shopping), " Tienda de assets"));
                ui.label(RichText::new("· assets gratuitos de OpenGameArt.org").weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Cerrar").clicked() {
                        close = true;
                    }
                });
            });
            self.search_bar(ui);
            self.status_line(ui);
            ui.separator();
            let view = self.view.clone();
            match view {
                View::Results | View::Favorites => self.grid(ui, body_h),
                View::Detail(slug) => self.detail(ui, &slug, body_h, can_import),
            }
            ui.separator();
            ui.label(RichText::new("Cada petición la lanzas tú: máx. 1 cada 2 s y caché de 24 h (~/.cache/modengine/store). Comprueba siempre la licencia en la página del asset.").small().weak());
        });
        let open = !(close || resp.should_close());
        (open, std::mem::take(&mut self.out))
    }

    fn search_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let mut go = false;
        ui.horizontal(|ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut self.q.keys).hint_text("Buscar (slime, coin, jump, forest…)").desired_width(260.0).id_salt("store_keys"));
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                go = true;
            }
            egui::ComboBox::from_id_salt("store_kind").selected_text(self.q.kind.label()).show_ui(ui, |ui| {
                for k in ArtKind::ALL {
                    if ui.selectable_value(&mut self.q.kind, k, k.label()).clicked() {
                        go = true;
                    }
                }
            });
            egui::ComboBox::from_id_salt("store_sort").selected_text(self.q.sort.label()).show_ui(ui, |ui| {
                for s in Sort::ALL {
                    if ui.selectable_value(&mut self.q.sort, s, s.label()).clicked() {
                        go = true;
                    }
                }
            });
            if ui.add(crate::theme::primary_button(ui, concat!(fa!(magnifying_glass), " Buscar"))).clicked() {
                go = true;
            }
            let favs = format!(concat!(fa!(star), " Favoritos ({})"), self.favorites.len());
            if ui.selectable_label(self.view == View::Favorites, favs).clicked() {
                self.view = if self.view == View::Favorites { View::Results } else { View::Favorites };
                let thumbs: Vec<String> = self.favorites.iter().filter_map(|f| f.thumb.clone()).collect();
                for t in thumbs {
                    self.want_image(&ctx, &t);
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Licencias:");
            for l in License::FILTERS {
                let mut on = self.q.licenses.contains(&l);
                let text = if l.share_alike() { format!(concat!(fa!(triangle_exclamation), " {}"), l.name()) } else { l.name() };
                let hover = match l.level() {
                    0 => "Dominio público: sin obligaciones",
                    1 => "Exige dar crédito al autor (se apunta en CREDITS.md)",
                    _ => "Crédito + «compartir igual»: lo que modifiques debe publicarse con la misma licencia (GPL: además, con el texto de la licencia y las fuentes)",
                };
                if ui.checkbox(&mut on, text).on_hover_text(hover).changed() {
                    if on {
                        self.q.licenses.insert(l);
                    } else {
                        self.q.licenses.remove(&l);
                    }
                }
            }
        });
        if go {
            self.q.page = 0;
            self.search(&ctx);
        }
    }

    fn status_line(&mut self, ui: &mut egui::Ui) {
        let pal = crate::theme::colors(ui.ctx());
        ui.horizontal(|ui| {
            let busy = self.net.as_ref().map(Worker::busy).unwrap_or_default();
            let queued = self.net.as_ref().map(Worker::queued).unwrap_or(0);
            match &busy {
                Busy::Idle => {}
                Busy::Waiting(_) => {
                    ui.spinner();
                    ui.label(RichText::new(format!("esperando turno (máx. 1 petición cada {} s)… {queued} en cola", self.interval.as_secs().max(1))).weak());
                }
                Busy::Running(j) => {
                    ui.spinner();
                    let what = match j {
                        Job::Search(_) => "buscando en OpenGameArt…".to_string(),
                        Job::Detail(_) => "cargando la ficha del asset…".to_string(),
                        Job::Image(_) => format!("cargando vistas previas… ({queued} en cola)"),
                        Job::Download { name, .. } => format!("descargando {name}…"),
                    };
                    ui.label(RichText::new(what).weak());
                }
            }
            if let Some((ok, msg)) = &self.status {
                ui.colored_label(if *ok { pal.ok } else { pal.error }, msg);
            }
        });
    }

    fn grid(&mut self, ui: &mut egui::Ui, h: f32) {
        let ctx = ui.ctx().clone();
        let pal = crate::theme::colors(&ctx);
        let hits: Vec<Hit> = if self.view == View::Favorites {
            self.favorites.iter().map(|f| Hit { slug: f.slug.clone(), title: f.title.clone(), thumb: f.thumb.clone() }).collect()
        } else {
            match &self.results {
                Some(Ok(p)) => p.hits.clone(),
                Some(Err(e)) => {
                    let e = e.clone();
                    ui.allocate_ui(Vec2::new(ui.available_width(), h), |ui| {
                        ui.add_space(40.0);
                        ui.vertical_centered(|ui| {
                            ui.colored_label(pal.error, format!(concat!(fa!(xmark), " {e}"), e = e));
                            ui.add_space(6.0);
                            if ui.button("Reintentar").clicked() {
                                self.search(&ctx);
                            }
                        });
                    });
                    return;
                }
                None => vec![],
            }
        };
        let mut open = None;
        let mut fav = None;
        egui::ScrollArea::vertical().id_salt("store_grid").max_height(h).min_scrolled_height(h).auto_shrink([false, false]).show(ui, |ui| {
            if hits.is_empty() {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    let msg = match (&self.view, &self.results, &self.asked) {
                        (View::Favorites, _, _) => "Aún no tienes favoritos: marca la estrella en un asset para guardarlo aquí.",
                        (_, Some(Ok(_)), _) => "No hay resultados con esos filtros. Prueba otra palabra, otro tipo o más licencias.",
                        (_, None, Some(_)) => "Buscando…",
                        _ => "Escribe qué buscas (en inglés funciona mejor: slime, coin, explosion…) y pulsa Buscar.",
                    };
                    ui.label(RichText::new(msg).weak());
                });
                return;
            }
            let card_w = 176.0;
            let cols = ((ui.available_width() + 8.0) / (card_w + 8.0)).floor().max(1.0) as usize;
            for row in hits.chunks(cols) {
                ui.horizontal_top(|ui| {
                    for hit in row {
                        let (o, f) = self.card(ui, hit, card_w);
                        if o {
                            open = Some(hit.slug.clone());
                        }
                        if f {
                            fav = Some(hit.clone());
                        }
                    }
                });
                ui.add_space(4.0);
            }
        });
        if let Some(h) = fav {
            self.toggle_fav(&h.slug, &h.title, h.thumb);
        }
        if let Some(s) = open {
            self.open_detail(&ctx, &s);
        }
        if self.view == View::Results {
            if let Some(Ok(p)) = &self.results {
                let (page, last) = (p.page, p.last_page);
                ui.horizontal(|ui| {
                    if ui.add_enabled(page > 0, egui::Button::new(concat!(fa!(caret_left), " Anterior"))).clicked() {
                        self.q = self.asked.clone().unwrap_or_else(|| self.q.clone());
                        self.q.page = page - 1;
                        self.search(&ctx);
                    }
                    ui.label(format!("Página {} de {}", page + 1, last + 1));
                    if ui.add_enabled(page < last, egui::Button::new(concat!("Siguiente ", fa!(caret_right)))).clicked() {
                        self.q = self.asked.clone().unwrap_or_else(|| self.q.clone());
                        self.q.page = page + 1;
                        self.search(&ctx);
                    }
                });
            }
        }
    }

    /// One result: `(open it, toggle favourite)`.
    fn card(&mut self, ui: &mut egui::Ui, hit: &Hit, w: f32) -> (bool, bool) {
        let mut open = false;
        let mut fav = false;
        egui::Frame::group(ui.style()).inner_margin(6).show(ui, |ui| {
            ui.set_width(w - 14.0);
            ui.vertical(|ui| {
                let (rect, r) = ui.allocate_exact_size(Vec2::new(w - 14.0, 116.0), Sense::click());
                self.paint_image(ui, rect, hit.thumb.as_deref());
                if r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("ver la ficha").clicked() {
                    open = true;
                }
                ui.horizontal(|ui| {
                    let star = if self.is_fav(&hit.slug) { RichText::new(fa!(star)) } else { crate::icons::regular(fa!(star)) };
                    if ui.small_button(star).on_hover_text("favorito").clicked() {
                        fav = true;
                    }
                    let t = ui.add(egui::Label::new(RichText::new(&hit.title).strong()).truncate().sense(Sense::click()));
                    if t.on_hover_text(&hit.title).clicked() {
                        open = true;
                    }
                });
                // author and licences come from the asset page: known once it was opened
                match self.details.get(&hit.slug) {
                    Some(Ok(a)) => {
                        ui.label(RichText::new(format!("por {}", a.author)).small().weak());
                        ui.horizontal_wrapped(|ui| {
                            for l in &a.licenses {
                                badge(ui, l);
                            }
                        });
                    }
                    _ if self.view == View::Results && self.q.licenses.len() == 1 => {
                        ui.label(RichText::new(" ").small());
                        ui.horizontal(|ui| badge(ui, self.q.licenses.iter().next().unwrap()));
                    }
                    _ => {
                        ui.label(RichText::new(" ").small());
                        ui.label(RichText::new("licencia: abre la ficha").small().weak());
                    }
                }
            });
        });
        (open, fav)
    }

    fn paint_image(&self, ui: &egui::Ui, rect: Rect, url: Option<&str>) {
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 4.0, if ui.visuals().dark_mode { Color32::from_gray(28) } else { Color32::from_gray(225) });
        match url.and_then(|u| self.images.get(u)) {
            Some(Img::Ready(t)) => {
                let s = t.size_vec2();
                let k = (rect.width() / s.x).min(rect.height() / s.y);
                // pixel art: whole-number zoom when it fits
                let k = if k >= 1.0 { k.floor() } else { k };
                let r = Rect::from_center_size(rect.center(), s * k);
                p.image(t.id(), r, Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0)), Color32::WHITE);
            }
            Some(Img::Loading) => {
                p.text(rect.center(), egui::Align2::CENTER_CENTER, "…", egui::FontId::proportional(20.0), ui.visuals().weak_text_color());
            }
            _ => {
                p.text(rect.center(), egui::Align2::CENTER_CENTER, "sin vista previa", egui::FontId::proportional(12.0), ui.visuals().weak_text_color());
            }
        }
    }

    fn detail(&mut self, ui: &mut egui::Ui, slug: &str, h: f32, can_import: bool) {
        let ctx = ui.ctx().clone();
        let pal = crate::theme::colors(&ctx);
        let warn = ui.visuals().warn_fg_color;
        let mut back = false;
        let a = match self.details.get(slug) {
            Some(Ok(a)) => a.clone(),
            other => {
                let err = match other {
                    Some(Err(e)) => Some(e.clone()),
                    _ => None,
                };
                ui.horizontal(|ui| {
                    if ui.button(concat!(fa!(caret_left), " Volver")).clicked() {
                        back = true;
                    }
                });
                ui.allocate_ui(Vec2::new(ui.available_width(), h - 30.0), |ui| {
                    ui.add_space(40.0);
                    ui.vertical_centered(|ui| match &err {
                        Some(e) => {
                            ui.colored_label(pal.error, format!(concat!(fa!(xmark), " {e}"), e = e));
                            if ui.button("Reintentar").clicked() {
                                self.open_detail(&ctx, slug);
                            }
                        }
                        None => {
                            ui.spinner();
                            ui.label(RichText::new("cargando la ficha…").weak());
                        }
                    });
                });
                if back {
                    self.view = View::Results;
                }
                return;
            }
        };
        ui.horizontal(|ui| {
            if ui.button(concat!(fa!(caret_left), " Volver a los resultados")).clicked() {
                back = true;
            }
            let star = if self.is_fav(slug) { RichText::new(concat!(fa!(star), " En favoritos")) } else { crate::icons::regular(concat!(fa!(star), " Añadir a favoritos")) };
            if ui.button(star).clicked() {
                let thumb = a.previews.first().map(|p| p.image.clone()).filter(|s| !s.is_empty());
                self.toggle_fav(slug, &a.title, thumb);
            }
            if ui.button(concat!(fa!(globe), " Abrir en el navegador")).on_hover_text(a.url()).clicked() {
                if let Err(e) = net::open_in_browser(&a.url()) {
                    self.status = Some((false, e));
                }
            }
        });
        if back {
            self.view = if self.favorites.iter().any(|f| f.slug == slug) && self.results.is_none() { View::Favorites } else { View::Results };
            self.zip = None;
            return;
        }
        egui::ScrollArea::vertical().id_salt(("store_detail", slug)).max_height(h - 30.0).min_scrolled_height(h - 30.0).auto_shrink([false, false]).show(ui, |ui| {
            ui.heading(&a.title);
            let mut by = format!("por {}", if a.author.is_empty() { "autor desconocido" } else { &a.author });
            if !a.collaborators.is_empty() {
                by += &format!(" (con {})", a.collaborators.join(", "));
            }
            if !a.date.is_empty() {
                by += &format!(" · {}", a.date);
            }
            if !a.art_types.is_empty() {
                by += &format!(" · {}", a.art_types.join(", "));
            }
            ui.label(RichText::new(by).weak());
            ui.add_space(6.0);
            ui.horizontal_top(|ui| {
                // previews
                ui.vertical(|ui| {
                    ui.set_width(360.0);
                    let imgs: Vec<String> = a.previews.iter().map(|p| p.image.clone()).filter(|s| !s.is_empty()).take(4).collect();
                    if let Some(first) = imgs.first() {
                        let (r, _) = ui.allocate_exact_size(Vec2::new(360.0, 260.0), Sense::hover());
                        self.paint_image(ui, r, Some(first));
                    } else {
                        let (r, _) = ui.allocate_exact_size(Vec2::new(360.0, 120.0), Sense::hover());
                        self.paint_image(ui, r, None);
                    }
                    if imgs.len() > 1 {
                        ui.horizontal(|ui| {
                            for u in &imgs[1..] {
                                let (r, _) = ui.allocate_exact_size(Vec2::new(114.0, 84.0), Sense::hover());
                                self.paint_image(ui, r, Some(u));
                            }
                        });
                    }
                });
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("Licencia:").strong());
                        if a.licenses.is_empty() {
                            ui.colored_label(pal.error, "desconocida");
                        }
                        for l in &a.licenses {
                            let r = badge(ui, l);
                            if let Some(u) = l.url() {
                                r.on_hover_text(u);
                            }
                        }
                    });
                    for (n, text) in license_notes(&a.licenses) {
                        let c = match n {
                            Note::Info => ui.visuals().weak_text_color(),
                            Note::Warn => warn,
                            Note::Block => pal.error,
                        };
                        let icon = match n {
                            Note::Info => fa!(circle_info),
                            Note::Warn => fa!(triangle_exclamation),
                            Note::Block => fa!(xmark),
                        };
                        ui.colored_label(c, format!("{icon} {text}"));
                    }
                    if blocking_note(&a.licenses).is_some() {
                        let mut ack = self.ack.contains(slug);
                        if ui.checkbox(&mut ack, "He revisado la licencia en la página y puedo usar este asset").changed() {
                            if ack {
                                self.ack.insert(slug.to_string());
                            } else {
                                self.ack.remove(slug);
                            }
                        }
                    }
                    if !a.notice.trim().is_empty() {
                        ui.add_space(4.0);
                        ui.label(RichText::new("Atribución que pide el autor:").strong());
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.label(RichText::new(&a.notice).italics());
                        });
                    }
                    if !a.tags.is_empty() {
                        ui.add_space(4.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new("Etiquetas:").strong());
                            for t in &a.tags {
                                ui.label(RichText::new(format!("#{t}")).small().weak());
                            }
                        });
                    }
                    if !a.description.is_empty() {
                        ui.add_space(4.0);
                        ui.label(RichText::new("Descripción").strong());
                        egui::ScrollArea::vertical().id_salt("store_desc").max_height(150.0).show(ui, |ui| {
                            ui.label(&a.description);
                        });
                    }
                });
            });
            ui.add_space(8.0);
            ui.label(RichText::new("Ficheros").strong());
            self.files_ui(ui, &a, can_import);
            if self.zip.as_ref().is_some_and(|z| z.slug == slug) {
                ui.add_space(8.0);
                self.zip_ui(ui, can_import);
            }
        });
    }

    fn files_ui(&mut self, ui: &mut egui::Ui, a: &Asset, can_import: bool) {
        let ctx = ui.ctx().clone();
        let pal = crate::theme::colors(&ctx);
        if a.files.is_empty() {
            ui.label(RichText::new("La página no tiene ficheros descargables.").weak());
            return;
        }
        let progress = self.net.as_ref().and_then(Worker::progress);
        let mut act = None;
        let mut big = None;
        egui::Grid::new(("store_files", &a.slug)).num_columns(4).striped(true).spacing([14.0, 6.0]).show(ui, |ui| {
            for f in &a.files {
                let ext = f.ext();
                let kind = Kind::of(Path::new(&f.name));
                ui.label(format!("{} {}", kind.map(crate::import::kind_icon).unwrap_or(if f.is_zip() { fa!(file_zipper) } else { "·" }), f.name));
                ui.label(RichText::new(if ext.is_empty() { "?".to_string() } else { ext.to_uppercase() }).weak());
                ui.label(RichText::new(f.size.map(human_size).unwrap_or_else(|| "¿?".into())).weak());
                ui.horizontal(|ui| {
                    let state = self.downloads.get(&f.url).cloned();
                    let label = if f.is_zip() { concat!(fa!(download), " Ver contenido…") } else { concat!(fa!(download), " Importar…") };
                    let supported = f.is_zip() || kind.is_some();
                    if !supported {
                        let why = if ext == "mp3" { "MP3: el motor usa OGG o WAV (conviértelo)" } else { "formato que el motor no carga" };
                        ui.label(RichText::new(why).small().weak());
                    } else if state == Some(Dl::Running) {
                        match progress.as_ref().filter(|p| p.0 == f.url) {
                            Some((_, done, Some(total))) if *total > 0 => {
                                ui.add(egui::ProgressBar::new(*done as f32 / *total as f32).desired_width(160.0).text(format!("{} / {}", human_size(*done), human_size(*total))));
                            }
                            Some((_, done, _)) => {
                                ui.spinner();
                                ui.label(human_size(*done));
                            }
                            None => {
                                ui.spinner();
                                ui.label(RichText::new("en cola…").weak());
                            }
                        }
                    } else {
                        let r = ui.add_enabled(can_import, crate::theme::primary_button(ui, label)).on_disabled_hover_text("Abre un proyecto para importar");
                        if r.clicked() {
                            act = Some(f.clone());
                        }
                        if let Some(Dl::Failed(e)) = &state {
                            ui.colored_label(pal.error, fa!(xmark)).on_hover_text(e);
                        }
                        if self.confirm_big.as_deref() == Some(&f.url) {
                            big = Some(f.clone());
                        }
                    }
                });
                ui.end_row();
            }
        });
        if let Some(f) = big {
            let warn = ui.visuals().warn_fg_color;
            ui.horizontal(|ui| {
                ui.colored_label(warn, format!(concat!(fa!(triangle_exclamation), " {} mide {} (más de 50 MB). ¿Descargar igualmente?"), f.name, f.size.map(human_size).unwrap_or_default()));
                if ui.button("Descargar").clicked() {
                    self.big_ok.insert(f.url.clone());
                    self.confirm_big = None;
                    act = Some(f.clone());
                }
                if ui.button("Cancelar").clicked() {
                    self.confirm_big = None;
                }
            });
        }
        if let Some(f) = act {
            self.import_file(&ctx, &a.slug, &f);
        }
    }

    fn zip_ui(&mut self, ui: &mut egui::Ui, can_import: bool) {
        let pal = crate::theme::colors(ui.ctx());
        let mut go = false;
        let mut close = false;
        let Some(z) = &mut self.zip else { return };
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!(concat!(fa!(file_zipper), " Contenido de {}"), z.file.name)).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button(fa!(xmark)).on_hover_text("cerrar").clicked() {
                        close = true;
                    }
                });
            });
            if let Some(e) = &z.error {
                ui.colored_label(pal.error, format!(concat!(fa!(xmark), " {e}"), e = e));
            }
            let n_imp = z.items.iter().filter(|(i, _)| i.importable()).count();
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{} ficheros, {n_imp} importables (PNG, WAV, OGG, TTF, OTF)", z.items.len())).weak());
                if ui.small_button("Todos los importables").clicked() {
                    for (i, s) in &mut z.items {
                        *s = i.importable();
                    }
                }
                if ui.small_button("Ninguno").clicked() {
                    for (_, s) in &mut z.items {
                        *s = false;
                    }
                }
            });
            egui::ScrollArea::vertical().id_salt("store_zip").max_height(220.0).show(ui, |ui| {
                for (it, sel) in &mut z.items {
                    let kind = Kind::of(Path::new(it.file_name()));
                    let text = format!("{} {}  ({})", kind.map(crate::import::kind_icon).unwrap_or("·"), it.name, human_size(it.size));
                    if kind.is_some() {
                        ui.checkbox(sel, text);
                    } else {
                        ui.add_enabled(false, egui::Checkbox::new(&mut false, RichText::new(text).weak()));
                    }
                }
            });
            let n = z.items.iter().filter(|(_, s)| *s).count();
            if ui.add_enabled(n > 0 && can_import, crate::theme::primary_button(ui, &format!("Importar seleccionados ({n})…"))).clicked() {
                go = true;
            }
        });
        if close {
            self.zip = None;
        } else if go {
            self.import_zip_selection();
        }
    }
}

/// A coloured licence label.
fn badge(ui: &mut egui::Ui, l: &License) -> egui::Response {
    let (bg, fg) = match l.level() {
        0 => (Color32::from_rgb(46, 125, 50), Color32::WHITE),
        1 => (Color32::from_rgb(25, 101, 175), Color32::WHITE),
        2 => (Color32::from_rgb(214, 120, 0), Color32::BLACK),
        _ => (Color32::from_rgb(150, 40, 40), Color32::WHITE),
    };
    egui::Frame::new().fill(bg).corner_radius(4).inner_margin(egui::Margin::symmetric(5, 1)).show(ui, |ui| ui.label(RichText::new(l.name()).small().color(fg))).response
}

#[cfg(test)]
mod tests {
    use super::*;
    use modengine_store::testing::{tmp, Mock};
    use crate::store_net::NetError;
    use egui_kittest::{kittest::Queryable, Harness};

    fn fixture(name: &str) -> Vec<u8> {
        modengine_store::testing::fixture(name)
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut b = std::io::Cursor::new(vec![]);
        image::RgbaImage::from_fn(w, h, |x, _| image::Rgba([(x * 8) as u8, 200, 90, 255])).write_to(&mut b, image::ImageFormat::Png).unwrap();
        b.into_inner()
    }

    /// OGA as the fixtures say: the CC0 slime search, the three asset pages, a PNG for every
    /// image, the slime sheet and a zip.
    fn mock() -> Mock {
        let mut m = Mock::default();
        let q = Query { keys: "slime".into(), licenses: [License::Cc0].into_iter().collect(), ..Default::default() };
        m.pages.insert(q.url(), Ok(fixture("search_slime_cc0.html")));
        for slug in ["animated-slime", "pixel-art-animated-slime", "tiny-creatures"] {
            m.pages.insert(crate::store_oga::content_url(slug), Ok(fixture(&format!("{slug}.html"))));
        }
        let page = crate::store_oga::parse_search(&String::from_utf8(fixture("search_slime_cc0.html")).unwrap()).unwrap();
        for h in &page.hits {
            m.pages.insert(h.thumb.clone().unwrap(), Ok(png(32, 16)));
        }
        let a = crate::store_oga::parse_detail(&String::from_utf8(fixture("animated-slime.html")).unwrap(), "animated-slime").unwrap();
        for p in &a.previews {
            m.pages.insert(p.image.clone(), Ok(png(64, 32)));
        }
        let sheet = png(128, 32);
        // the page says 12041 bytes: pad the PNG (trailing bytes after IEND are ignored)
        let mut padded = sheet.clone();
        padded.resize(12041, 0);
        m.pages.insert(a.files[0].url.clone(), Ok(padded));
        let mut z = modengine_store::testing::make_zip(&[("Slime/slime_idle.png", &sheet, true), ("Slime/notes.txt", b"hi", false)]);
        z.resize(12061, 0); // the size the page announces

        m.pages.insert("https://opengameart.org/sites/default/files/Slime_0.zip".into(), Ok(z));
        m
    }

    fn wait(h: &mut Harness<'_, St>, what: impl Fn(&St) -> bool) {
        for _ in 0..400 {
            h.step();
            if what(h.state()) {
                h.run_steps(2);
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let d = &h.state().d;
        panic!("timed out: view {:?}, status {:?}, out {}, zip {}, downloads {:?}", d.view, d.status, h.state().out.len(), d.zip.is_some(), d.downloads);
    }

    struct St {
        d: StoreDialog,
        out: Vec<PathBuf>,
    }

    #[test]
    fn prefs_round_trip() {
        let dir = tmp("prefs");
        let f = dir.join("store.toml");
        assert_eq!(Prefs::load(Some(&f)), Prefs::default());
        assert_eq!(Prefs::default().query(), Query::default(), "defaults: 2D, no SA/GPL");
        let p = Prefs { query: "coin".into(), kind: "sfx".into(), licenses: vec!["cc0".into(), "cc-by-sa-4.0".into(), "bogus".into()], sort: "created".into(), favorites: vec![Fav { slug: "a".into(), title: "A".into(), thumb: None }] };
        p.save(&f).unwrap();
        let back = Prefs::load(Some(&f));
        assert_eq!(back, p);
        let q = back.query();
        assert_eq!((q.keys.as_str(), q.kind, q.sort), ("coin", ArtKind::Sfx, Sort::Newest));
        assert_eq!(q.licenses, [License::Cc0, License::CcBySa4].into_iter().collect());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn store_ui_searches_opens_and_imports() {
        let dir = tmp("ui");
        let cfg = dir.join("cfg/store.toml");
        let m = Arc::new(mock());
        let mut d = StoreDialog::new(m.clone(), Some(DiskCache::new(dir.join("cache"), net::CACHE_TTL)), Duration::ZERO, Some(cfg.clone()));
        d.q.keys = "slime".into();
        d.q.licenses = [License::Cc0].into_iter().collect();
        d.start_search = true;
        let mut h = Harness::builder().with_size([1400.0, 900.0]).allow_missing_glyphs().with_accessibility_check(false).build_ui_state(
            |ui, s: &mut St| {
                let (_, out) = s.d.show(ui.ctx(), true);
                s.out.extend(out);
            },
            St { d, out: vec![] },
        );
        wait(&mut h, |s| matches!(&s.d.results, Some(Ok(_))) && !s.d.loading());
        h.get_by_label("Pixel art animated Slime");
        h.get_by_label("Página 1 de 4");
        assert!(h.state().d.images.values().all(|i| matches!(i, Img::Ready(_))), "thumbnails loaded");
        assert!(std::fs::read_to_string(&cfg).unwrap().contains("query = \"slime\""), "last search saved");

        // open the CC-BY slime: licence, warning, notice and its file
        h.state_mut().d.start_open = Some("animated-slime".into());
        wait(&mut h, |s| matches!(s.d.details.get("animated-slime"), Some(Ok(_))) && !s.d.loading());
        h.get_by_label("Animated Slime");
        assert_eq!(h.query_all_by_label("CC-BY 3.0").count(), 2, "the filter checkbox and the asset's badge");
        h.get_by_label_contains("Exige dar crédito");
        h.get_by_label_contains("slime spritesheet calciumtrice.png");
        h.get_by_label("Atribución que pide el autor:");
        // favourite it
        h.get_by_label(concat!(fa!(star), " Añadir a favoritos")).click();
        h.run_steps(2);
        assert_eq!(h.state().d.favorites.len(), 1);
        assert!(std::fs::read_to_string(&cfg).unwrap().contains("animated-slime"));
        // import the PNG: downloaded (size checked) and handed over with its credit
        h.get_by_label(concat!(fa!(download), " Importar…")).click();
        wait(&mut h, |s| !s.out.is_empty());
        let got = h.state().out[0].clone();
        assert_eq!(got.file_name().unwrap(), "slime spritesheet calciumtrice.png");
        assert_eq!(std::fs::metadata(&got).unwrap().len(), 12041);
        assert!(PENDING.with(|p| p.borrow().get(&got).is_some_and(|c| c.author == "Calciumtrice")));

        // a zip: list, pick, extract
        h.state_mut().d.start_open = Some("pixel-art-animated-slime".into());
        wait(&mut h, |s| matches!(s.d.details.get("pixel-art-animated-slime"), Some(Ok(_))) && !s.d.loading());
        h.get_by_label(concat!(fa!(download), " Ver contenido…")).click();
        wait(&mut h, |s| s.d.zip.is_some());
        h.get_by_label_contains("Contenido de Slime.zip");
        h.get_by_label_contains("Slime/slime_idle.png");
        h.get_by_label("Importar seleccionados (1)…").click();
        wait(&mut h, |s| s.out.len() == 2);
        let extracted = h.state().out[1].clone();
        assert_eq!(extracted.file_name().unwrap(), "slime_idle.png");
        assert!(PENDING.with(|p| p.borrow().get(&extracted).is_some_and(|c| c.source_file == "Slime.zip → Slime/slime_idle.png" && c.licenses[0].0 == "CC0")));

        // now through the real import + credits into a project
        let project = crate::room::tests::temp_project("store_import");
        let imp = crate::import::import(&project, Kind::Sprite, &got, "oga_slime", &crate::import::SpriteOpts { frames: crate::import::Frames::Strip(4), ..Default::default() }, false).unwrap();
        let lines = credit_imported(&project, &[imp]);
        assert!(lines.iter().any(|l| l.text.contains("CREDITS.md")) && lines.iter().any(|l| l.text.contains("exige dar crédito")));
        let md = std::fs::read_to_string(project.join("CREDITS.md")).unwrap();
        assert!(md.contains("## Animated Slime — Calciumtrice") && md.contains("`sprites/oga_slime.png`") && md.contains("`sprites/oga_slime.toml`") && md.contains("CC-BY 3.0"));
        assert!(std::fs::read_to_string(project.join("sprites/oga_slime.toml")).unwrap().starts_with("# Origen:"));
        modengine::Engine::load(&project).expect("the project still loads");
        // only allowed URLs, and never the same one twice (disk cache)
        let hits = m.hits.lock().unwrap().clone();
        let uniq: BTreeSet<&String> = hits.iter().collect();
        assert_eq!(uniq.len(), hits.len(), "{hits:?}");
        let _ = std::fs::remove_dir_all(&project);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn errors_show_nicely() {
        let mut m = Mock::default();
        let q = Query { keys: "x".into(), ..Default::default() };
        m.pages.insert(q.url(), Err(NetError::Offline));
        let mut d = StoreDialog::new(Arc::new(m), None, Duration::ZERO, None);
        d.q = q;
        d.start_search = true;
        let mut h = Harness::builder().with_size([1200.0, 800.0]).allow_missing_glyphs().with_accessibility_check(false).build_ui_state(
            |ui, s: &mut St| {
                s.d.show(ui.ctx(), false);
            },
            St { d, out: vec![] },
        );
        wait(&mut h, |s| matches!(&s.d.results, Some(Err(_))));
        h.get_by_label_contains("sin conexión");
        h.get_by_label("Reintentar");
        // an asset with an unknown licence can't be imported until acknowledged
        let d = &mut h.state_mut().d;
        let a = Asset { slug: "u".into(), title: "U".into(), files: vec![FileLink { url: "https://opengameart.org/sites/default/files/u.png".into(), name: "u.png".into(), mime: None, size: Some(3) }], ..Default::default() };
        d.details.insert("u".into(), Ok(a.clone()));
        let ctx = egui::Context::default();
        d.import_file(&ctx, "u", &a.files[0]);
        assert!(d.status.as_ref().unwrap().1.contains("licencia"));
        assert!(d.downloads.is_empty());
        // mp3 is refused with a reason
        let mp3 = FileLink { url: "https://opengameart.org/sites/default/files/a.mp3".into(), name: "a.mp3".into(), mime: None, size: None };
        d.ack.insert("u".into());
        d.import_file(&ctx, "u", &mp3);
        assert!(d.status.as_ref().unwrap().1.contains("formato"));
        // > 50 MB asks first
        let big = FileLink { url: "https://opengameart.org/sites/default/files/big.png".into(), name: "big.png".into(), mime: None, size: Some(net::BIG_DOWNLOAD + 1) };
        d.import_file(&ctx, "u", &big);
        assert_eq!(d.confirm_big.as_deref(), Some(big.url.as_str()));
        assert!(d.downloads.is_empty());
    }

    /// Real OpenGameArt, a handful of requests: `cargo test -p modengine-editor --release
    /// real_opengameart -- --ignored --nocapture`. Searches "slime" (2D, CC0), opens results
    /// until one has a PNG, downloads it and imports it into a copy of examples/platformer,
    /// then runs `modengine check` on it. Keeps the copy (path printed).
    #[test]
    #[ignore]
    fn real_opengameart_import() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let cache = DiskCache::new(net::cache_dir().unwrap(), net::CACHE_TTL);
        let w = Worker::start(Arc::new(net::Curl), Some(cache.clone()), net::MIN_INTERVAL, None);
        let q = Query { keys: "slime".into(), licenses: [License::Cc0].into_iter().collect(), ..Default::default() };
        println!("GET {}", q.url());
        w.push(Job::Search(q));
        let page = match w.rx.recv_timeout(Duration::from_secs(60)).unwrap() {
            Reply::Search(_, r) => r.unwrap(),
            r => panic!("{r:?}"),
        };
        println!("{} results, {} pages", page.hits.len(), page.last_page + 1);
        assert!(!page.hits.is_empty());
        let mut found = None;
        // the "slime"-titled ones first: more likely a single sheet than a tileset zip
        let mut hits = page.hits.clone();
        hits.sort_by_key(|h| !h.title.to_lowercase().contains("slime"));
        for hit in hits.iter().take(4) {
            w.push(Job::Detail(hit.slug.clone()));
            let a = match w.rx.recv_timeout(Duration::from_secs(60)).unwrap() {
                Reply::Detail(_, r) => r.unwrap(),
                r => panic!("{r:?}"),
            };
            println!("  {} by {} {:?}: {:?}", a.title, a.author, a.licenses.iter().map(License::name).collect::<Vec<_>>(), a.files.iter().map(|f| &f.name).collect::<Vec<_>>());
            if let Some(f) = a.files.iter().find(|f| f.ext() == "png").cloned() {
                found = Some((a, f));
                break;
            }
        }
        let (a, f) = found.expect("a CC0 slime with a PNG among the first results");
        assert_eq!(a.licenses, vec![License::Cc0]);
        w.push(Job::Download { url: f.url.clone(), name: f.name.clone(), size: f.size });
        let file = match w.rx.recv_timeout(Duration::from_secs(300)).unwrap() {
            Reply::Download(_, r) => r.unwrap(),
            r => panic!("{r:?}"),
        };
        println!("downloaded {} ({} bytes)", file.display(), std::fs::metadata(&file).unwrap().len());
        let tmp = crate::room::tests::temp_project("store_real");
        let name = crate::import::sanitize(Path::new(&f.name).file_stem().unwrap().to_str().unwrap());
        let imp = crate::import::import(&tmp, Kind::Sprite, &file, &name, &crate::import::SpriteOpts::default(), false).unwrap();
        remember_credit(&file, Credit::from_asset(&a, &f.name));
        let lines = credit_imported(&tmp, &[imp]);
        for l in &lines {
            println!("  log: {}", l.text);
        }
        let md = std::fs::read_to_string(tmp.join("CREDITS.md")).unwrap();
        println!("--- CREDITS.md\n{md}---");
        assert!(md.contains(&a.url()) && md.contains("CC0"));
        let bin = root.join("target/release").join(format!("modengine{}", std::env::consts::EXE_SUFFIX));
        let out = std::process::Command::new(&bin).arg("check").arg(&tmp).output().expect("build modengine first (cargo build --release)");
        println!("modengine check: {}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success());
        println!("project kept at {}", tmp.display());
    }
}
