//! Networking of the asset store: an honest, polite HTTP client for OpenGameArt.
//!
//! * `curl` does the requests (like the Paletas dialog: no HTTP/TLS crates), with a
//!   User-Agent that says who we are.
//! * Every request is started by the user (a search, opening an asset, the thumbnails of the
//!   page they are looking at, a download). Nothing is crawled in the background.
//! * One worker thread runs a queue; network requests are at least [`MIN_INTERVAL`] apart
//!   (thumbnails too). Pages and thumbnails are cached on disk for [`CACHE_TTL`]
//!   (`~/.cache/modengine/store/`), so going back and forth costs nothing.
//! * Only URLs that pass [`allowed_url`] are fetched.

use crate::store_oga::{allowed_url, parse_detail, parse_search, Asset, Query, SearchPage};
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime};

pub const MIN_INTERVAL: Duration = Duration::from_secs(2);
pub const CACHE_TTL: Duration = Duration::from_secs(24 * 3600);
/// Downloads above this ask first.
pub const BIG_DOWNLOAD: u64 = 50 * 1024 * 1024;

pub fn user_agent() -> String {
    format!("ModEngine-Editor/{} (+https://github.com/evilrender23/Mod-Engine)", env!("CARGO_PKG_VERSION"))
}

#[derive(Clone, Debug, PartialEq)]
pub enum NetError {
    Offline,
    Timeout,
    /// 429 / 503: the site asks us to slow down.
    RateLimited,
    Http(u16),
    Refused(String),
    Other(String),
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetError::Offline => write!(f, "sin conexión con opengameart.org (¿hay Internet?)"),
            NetError::Timeout => write!(f, "opengameart.org no responde (tiempo agotado)"),
            NetError::RateLimited => write!(f, "OpenGameArt pide que esperemos (demasiadas peticiones): prueba en un minuto"),
            NetError::Http(404) => write!(f, "no encontrado en OpenGameArt (404)"),
            NetError::Http(c) => write!(f, "OpenGameArt respondió con un error HTTP {c}"),
            NetError::Refused(u) => write!(f, "dirección no permitida para la tienda: {u}"),
            NetError::Other(e) => write!(f, "{e}"),
        }
    }
}

/// Something that can GET a URL (curl for real, fixtures in tests).
pub trait Http: Send + Sync {
    fn get(&self, url: &str) -> Result<Vec<u8>, NetError>;
    /// GET into a file (default: [`Http::get`] then write). `curl` writes as it downloads so
    /// the UI can show progress from the file size.
    fn download(&self, url: &str, to: &Path) -> Result<(), NetError> {
        let b = self.get(url)?;
        fs::write(to, b).map_err(|e| NetError::Other(format!("{}: {e}", to.display())))
    }
}

pub struct Curl;

impl Curl {
    fn run(&self, url: &str, out: Option<&Path>, max_time: u32) -> Result<Vec<u8>, NetError> {
        if !allowed_url(url) {
            return Err(NetError::Refused(url.into()));
        }
        let mut cmd = std::process::Command::new("curl");
        cmd.args(["-fsSL", "--proto", "=https", "--proto-redir", "=https", "--max-redirs", "3", "--connect-timeout", "15", "--max-time"]);
        cmd.arg(max_time.to_string()).arg("-A").arg(user_agent());
        if let Some(p) = out {
            cmd.arg("-o").arg(p);
        }
        cmd.arg(url);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let o = cmd.output().map_err(|e| NetError::Other(format!("no se pudo ejecutar curl: {e}")))?;
        if o.status.success() {
            return Ok(o.stdout);
        }
        let err = String::from_utf8_lossy(&o.stderr).to_string();
        Err(curl_error(o.status.code(), &err))
    }
}

/// curl exit code + stderr → what went wrong.
pub fn curl_error(code: Option<i32>, stderr: &str) -> NetError {
    match code {
        Some(6) | Some(7) => NetError::Offline,
        Some(28) => NetError::Timeout,
        Some(22) => {
            let status = stderr.rsplit("error:").next().and_then(|s| s.trim().split_whitespace().next()).and_then(|s| s.parse::<u16>().ok()).unwrap_or(0);
            match status {
                429 | 503 => NetError::RateLimited,
                s => NetError::Http(s),
            }
        }
        _ => NetError::Other(format!("curl: {}", stderr.trim())),
    }
}

impl Http for Curl {
    fn get(&self, url: &str) -> Result<Vec<u8>, NetError> {
        self.run(url, None, 30)
    }

    fn download(&self, url: &str, to: &Path) -> Result<(), NetError> {
        self.run(url, Some(to), 900).map(|_| ())
    }
}

// ------------------------------------------------------------------ rate limiting

/// At most one request per `interval`.
pub struct RateLimiter {
    pub interval: Duration,
    last: Option<Instant>,
}

impl RateLimiter {
    pub fn new(interval: Duration) -> Self {
        RateLimiter { interval, last: None }
    }

    /// How long to wait before a request at `now`.
    pub fn wait(&self, now: Instant) -> Duration {
        match self.last {
            Some(l) => (l + self.interval).saturating_duration_since(now),
            None => Duration::ZERO,
        }
    }

    /// A request went out at `now`.
    pub fn mark(&mut self, now: Instant) {
        self.last = Some(now);
    }
}

// ------------------------------------------------------------------ disk cache

/// `~/.cache/modengine/store` (or `$XDG_CACHE_HOME/modengine/store`).
pub fn cache_dir() -> Option<PathBuf> {
    if let Some(x) = std::env::var_os("XDG_CACHE_HOME").filter(|x| !x.is_empty()) {
        return Some(PathBuf::from(x).join("modengine/store"));
    }
    std::env::var_os("HOME").or_else(|| std::env::var_os("LOCALAPPDATA")).map(|h| PathBuf::from(h).join(".cache/modengine/store"))
}

/// FNV-1a: stable file names for URLs.
pub fn hash(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

#[derive(Clone)]
pub struct DiskCache {
    pub dir: PathBuf,
    pub ttl: Duration,
}

impl DiskCache {
    pub fn new(dir: PathBuf, ttl: Duration) -> Self {
        DiskCache { dir, ttl }
    }

    fn path(&self, url: &str) -> PathBuf {
        self.dir.join("http").join(hash(url))
    }

    /// Fresh entry (younger than the TTL).
    pub fn get(&self, url: &str) -> Option<Vec<u8>> {
        let p = self.path(url);
        let age = fs::metadata(&p).ok()?.modified().ok().and_then(|m| SystemTime::now().duration_since(m).ok()).unwrap_or(Duration::ZERO);
        (age < self.ttl).then(|| fs::read(&p).ok()).flatten()
    }

    /// Any entry, however old (used when offline).
    pub fn get_stale(&self, url: &str) -> Option<Vec<u8>> {
        fs::read(self.path(url)).ok()
    }

    pub fn put(&self, url: &str, bytes: &[u8]) {
        let p = self.path(url);
        if let Some(d) = p.parent() {
            let _ = fs::create_dir_all(d);
        }
        let tmp = p.with_extension("part");
        if fs::write(&tmp, bytes).is_ok() {
            let _ = fs::rename(&tmp, &p);
        }
    }

    /// Where a downloaded file of the asset store is kept: `files/<hash of url>/<name>`.
    pub fn file_path(&self, url: &str, name: &str) -> PathBuf {
        self.dir.join("files").join(hash(url)).join(safe_file_name(name))
    }
}

/// A file name safe to create anywhere (no separators, no leading dots).
pub fn safe_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let s: String = base.chars().map(|c| if c.is_control() || ":*?\"<>|".contains(c) { '_' } else { c }).collect();
    let s = s.trim().trim_start_matches('.').to_string();
    if s.is_empty() { "fichero".into() } else { s }
}

// ------------------------------------------------------------------ the worker

#[derive(Clone, Debug, PartialEq)]
pub enum Job {
    Search(Query),
    Detail(String),
    /// Decode an image (thumbnail / preview) for the UI.
    Image(String),
    /// Download a file of an asset into the cache: url, original name, size the page says.
    Download { url: String, name: String, size: Option<u64> },
}

impl Job {
    fn url(&self) -> String {
        match self {
            Job::Search(q) => q.url(),
            Job::Detail(s) => crate::store_oga::content_url(s),
            Job::Image(u) | Job::Download { url: u, .. } => u.clone(),
        }
    }
}

/// Decoded RGBA image.
#[derive(Clone, Debug, PartialEq)]
pub struct Rgba {
    pub size: [usize; 2],
    pub pixels: Vec<u8>,
}

#[derive(Debug)]
pub enum Reply {
    Search(Query, Result<SearchPage, String>),
    Detail(String, Result<Asset, String>),
    Image(String, Result<Rgba, String>),
    Download(String, Result<PathBuf, String>),
}

/// What the worker is doing (shown in the UI).
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Busy {
    #[default]
    Idle,
    /// Waiting for the rate limiter before this job.
    Waiting(Job),
    Running(Job),
}

struct Shared {
    queue: Mutex<(VecDeque<Job>, bool)>,
    cv: Condvar,
    busy: Mutex<Busy>,
    /// File being downloaded right now (for the progress bar).
    downloading: Mutex<Option<(String, PathBuf, Option<u64>)>>,
}

pub struct Worker {
    shared: Arc<Shared>,
    pub rx: Receiver<Reply>,
}

impl Worker {
    /// Starts the worker thread. `repaint` wakes the UI when a reply arrives.
    pub fn start(http: Arc<dyn Http>, cache: Option<DiskCache>, interval: Duration, repaint: Option<eframe::egui::Context>) -> Worker {
        let shared = Arc::new(Shared { queue: Mutex::new((VecDeque::new(), false)), cv: Condvar::new(), busy: Mutex::new(Busy::Idle), downloading: Mutex::new(None) });
        let (tx, rx) = mpsc::channel();
        let s = shared.clone();
        std::thread::Builder::new().name("store-net".into()).spawn(move || run(s, http, cache, interval, tx, repaint)).expect("store thread");
        Worker { shared, rx }
    }

    /// Queues a job: user actions go first, images to the back. A job already queued is not
    /// queued twice.
    pub fn push(&self, job: Job) -> bool {
        let mut q = self.shared.queue.lock().unwrap();
        if q.0.contains(&job) {
            return false;
        }
        if matches!(job, Job::Image(_)) {
            q.0.push_back(job);
        } else {
            let at = q.0.iter().position(|j| matches!(j, Job::Image(_))).unwrap_or(q.0.len());
            q.0.insert(at, job);
        }
        self.shared.cv.notify_all();
        true
    }

    /// Drops queued image jobs (a new search makes the old thumbnails pointless).
    pub fn drop_images(&self) -> usize {
        let mut q = self.shared.queue.lock().unwrap();
        let before = q.0.len();
        q.0.retain(|j| !matches!(j, Job::Image(_)));
        before - q.0.len()
    }

    pub fn queued(&self) -> usize {
        self.shared.queue.lock().unwrap().0.len()
    }

    pub fn busy(&self) -> Busy {
        self.shared.busy.lock().unwrap().clone()
    }

    /// `(url, bytes so far, total)` of the running download.
    pub fn progress(&self) -> Option<(String, u64, Option<u64>)> {
        let d = self.shared.downloading.lock().unwrap();
        let (url, path, total) = d.as_ref()?;
        let part = path.with_extension("part");
        let done = fs::metadata(&part).or_else(|_| fs::metadata(path)).map(|m| m.len()).unwrap_or(0);
        Some((url.clone(), done, *total))
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let mut q = self.shared.queue.lock().unwrap();
        q.1 = true;
        q.0.clear();
        self.shared.cv.notify_all();
    }
}

fn run(s: Arc<Shared>, http: Arc<dyn Http>, cache: Option<DiskCache>, interval: Duration, tx: Sender<Reply>, repaint: Option<eframe::egui::Context>) {
    let mut limiter = RateLimiter::new(interval);
    loop {
        let job = {
            let mut q = s.queue.lock().unwrap();
            while q.0.is_empty() && !q.1 {
                q = s.cv.wait(q).unwrap();
            }
            if q.1 {
                return;
            }
            q.0.pop_front().unwrap()
        };
        let reply = handle(&s, &*http, cache.as_ref(), &mut limiter, job);
        *s.busy.lock().unwrap() = Busy::Idle;
        if tx.send(reply).is_err() {
            return;
        }
        if let Some(c) = &repaint {
            c.request_repaint();
        }
    }
}

/// GET through the cache and the rate limiter.
fn fetch(s: &Shared, http: &dyn Http, cache: Option<&DiskCache>, limiter: &mut RateLimiter, job: &Job) -> Result<Vec<u8>, NetError> {
    let url = job.url();
    if let Some(b) = cache.and_then(|c| c.get(&url)) {
        return Ok(b);
    }
    wait_turn(s, limiter, job);
    let r = http.get(&url);
    limiter.mark(Instant::now());
    match r {
        Ok(b) => {
            if let Some(c) = cache {
                c.put(&url, &b);
            }
            Ok(b)
        }
        // offline: an old copy is better than nothing
        Err(NetError::Offline | NetError::Timeout) if cache.and_then(|c| c.get_stale(&url)).is_some() => Ok(cache.and_then(|c| c.get_stale(&url)).unwrap()),
        Err(e) => Err(e),
    }
}

fn wait_turn(s: &Shared, limiter: &RateLimiter, job: &Job) {
    let w = limiter.wait(Instant::now());
    if !w.is_zero() {
        *s.busy.lock().unwrap() = Busy::Waiting(job.clone());
        std::thread::sleep(w);
    }
    *s.busy.lock().unwrap() = Busy::Running(job.clone());
}

fn handle(s: &Shared, http: &dyn Http, cache: Option<&DiskCache>, limiter: &mut RateLimiter, job: Job) -> Reply {
    *s.busy.lock().unwrap() = Busy::Running(job.clone());
    match job.clone() {
        Job::Search(q) => {
            let r = fetch(s, http, cache, limiter, &job).map_err(|e| e.to_string()).and_then(|b| parse_search(&String::from_utf8_lossy(&b)));
            Reply::Search(q, r)
        }
        Job::Detail(slug) => {
            let r = fetch(s, http, cache, limiter, &job).map_err(|e| e.to_string()).and_then(|b| parse_detail(&String::from_utf8_lossy(&b), &slug));
            Reply::Detail(slug, r)
        }
        Job::Image(url) => {
            let r = fetch(s, http, cache, limiter, &job).map_err(|e| e.to_string()).and_then(|b| decode_thumb(&b, 320));
            Reply::Image(url, r)
        }
        Job::Download { url, name, size } => {
            let r = download(s, http, cache, limiter, &job, &url, &name, size);
            *s.downloading.lock().unwrap() = None;
            Reply::Download(url, r)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn download(s: &Shared, http: &dyn Http, cache: Option<&DiskCache>, limiter: &mut RateLimiter, job: &Job, url: &str, name: &str, size: Option<u64>) -> Result<PathBuf, String> {
    let Some(cache) = cache else { return Err("no hay carpeta de caché ($HOME)".into()) };
    let dst = cache.file_path(url, name);
    // already downloaded with the right size: reuse it
    if let Ok(m) = fs::metadata(&dst) {
        if size.is_none_or(|n| n == m.len()) && m.len() > 0 {
            return Ok(dst);
        }
    }
    if let Some(d) = dst.parent() {
        fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let part = dst.with_extension("part");
    *s.downloading.lock().unwrap() = Some((url.to_string(), dst.clone(), size));
    wait_turn(s, limiter, job);
    let r = http.download(url, &part);
    limiter.mark(Instant::now());
    if let Err(e) = r {
        let _ = fs::remove_file(&part);
        return Err(e.to_string());
    }
    let got = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    check_size(got, size).inspect_err(|_| {
        let _ = fs::remove_file(&part);
    })?;
    fs::rename(&part, &dst).map_err(|e| format!("{}: {e}", dst.display()))?;
    Ok(dst)
}

/// The downloaded size must match what the page announced (the only integrity data OGA
/// publishes); empty files are always wrong.
pub fn check_size(got: u64, expected: Option<u64>) -> Result<(), String> {
    if got == 0 {
        return Err("la descarga está vacía".into());
    }
    match expected {
        Some(n) if n != got => Err(format!("la descarga mide {got} bytes y la página dice {n}: incompleta o cambiada, no se usa")),
        _ => Ok(()),
    }
}

/// Decodes a PNG/GIF/JPEG (first frame) and scales it down to fit `max` px.
pub fn decode_thumb(bytes: &[u8], max: u32) -> Result<Rgba, String> {
    let img = image::load_from_memory(bytes).map_err(|e| format!("imagen ilegible: {e}"))?.to_rgba8();
    let (w, h) = img.dimensions();
    let img = if w.max(h) > max {
        let k = max as f32 / w.max(h) as f32;
        image::imageops::resize(&img, ((w as f32 * k) as u32).max(1), ((h as f32 * k) as u32).max(1), image::imageops::FilterType::Nearest)
    } else {
        img
    };
    let (w, h) = img.dimensions();
    Ok(Rgba { size: [w as usize, h as usize], pixels: img.into_raw() })
}

/// Opens a URL in the system browser.
pub fn open_in_browser(url: &str) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("solo se abren direcciones https://".into());
    }
    #[cfg(target_os = "windows")]
    let r = {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("cmd").args(["/C", "start", "", url]).creation_flags(0x0800_0000).spawn()
    };
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(url).spawn();
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(url).spawn();
    r.map(|_| ()).map_err(|e| format!("no se pudo abrir el navegador: {e}"))
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::collections::HashMap;

    pub fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("modeditor_store_{tag}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
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

    pub fn wait_reply(w: &Worker) -> Reply {
        w.rx.recv_timeout(Duration::from_secs(10)).expect("reply")
    }

    #[test]
    fn rate_limiter_spaces_requests() {
        let t0 = Instant::now();
        let mut r = RateLimiter::new(Duration::from_secs(2));
        assert_eq!(r.wait(t0), Duration::ZERO);
        r.mark(t0);
        assert_eq!(r.wait(t0 + Duration::from_millis(500)), Duration::from_millis(1500));
        assert_eq!(r.wait(t0 + Duration::from_secs(3)), Duration::ZERO);
    }

    #[test]
    fn worker_respects_the_interval_and_caches() {
        let dir = tmp("worker");
        let png = {
            let mut b = std::io::Cursor::new(vec![]);
            image::RgbaImage::from_pixel(4, 2, image::Rgba([255, 0, 0, 255])).write_to(&mut b, image::ImageFormat::Png).unwrap();
            b.into_inner()
        };
        let mut m = Mock::default();
        let a = "https://opengameart.org/sites/default/files/a.png".to_string();
        let b = "https://opengameart.org/sites/default/files/b.png".to_string();
        m.pages.insert(a.clone(), Ok(png.clone()));
        m.pages.insert(b.clone(), Ok(png));
        let m = Arc::new(m);
        let cache = DiskCache::new(dir.clone(), CACHE_TTL);
        let w = Worker::start(m.clone(), Some(cache.clone()), Duration::from_millis(300), None);
        let t = Instant::now();
        w.push(Job::Image(a.clone()));
        w.push(Job::Image(b.clone()));
        for _ in 0..2 {
            match wait_reply(&w) {
                Reply::Image(_, Ok(img)) => assert_eq!(img.size, [4, 2]),
                r => panic!("{r:?}"),
            }
        }
        assert!(t.elapsed() >= Duration::from_millis(300), "second request waited for its turn");
        assert_eq!(m.hits.lock().unwrap().len(), 2);
        // cached now: no more network
        w.push(Job::Image(a.clone()));
        assert!(matches!(wait_reply(&w), Reply::Image(_, Ok(_))));
        assert_eq!(m.hits.lock().unwrap().len(), 2);
        assert!(cache.get(&a).is_some());
        drop(w);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cache_ttl() {
        let dir = tmp("ttl");
        let c = DiskCache::new(dir.clone(), Duration::from_secs(3600));
        let u = "https://opengameart.org/content/x";
        assert!(c.get(u).is_none());
        c.put(u, b"hola");
        assert_eq!(c.get(u).as_deref(), Some(&b"hola"[..]));
        // two hours old: expired, but still there for offline use
        let f = fs::File::options().write(true).open(c.path(u)).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(7200)).unwrap();
        assert!(c.get(u).is_none());
        assert_eq!(c.get_stale(u).as_deref(), Some(&b"hola"[..]));
        assert_ne!(hash("a"), hash("b"));
        assert_eq!(safe_file_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_file_name("..hidden"), "hidden");
        assert_eq!(safe_file_name("a:b?.png"), "a_b_.png");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn errors_are_explained() {
        assert_eq!(curl_error(Some(6), ""), NetError::Offline);
        assert_eq!(curl_error(Some(28), ""), NetError::Timeout);
        assert_eq!(curl_error(Some(22), "curl: (22) The requested URL returned error: 429"), NetError::RateLimited);
        assert_eq!(curl_error(Some(22), "curl: (22) The requested URL returned error: 404"), NetError::Http(404));
        assert!(NetError::RateLimited.to_string().contains("esperemos"));
        assert!(Curl.get("https://example.com/").unwrap_err() == NetError::Refused("https://example.com/".into()), "never leaves the allow-list");
        assert!(check_size(10, Some(10)).is_ok() && check_size(10, None).is_ok());
        assert!(check_size(9, Some(10)).is_err() && check_size(0, None).is_err());
    }

    #[test]
    fn downloads_check_the_size() {
        let dir = tmp("dl");
        let mut m = Mock::default();
        let u = "https://opengameart.org/sites/default/files/x_0.png".to_string();
        m.pages.insert(u.clone(), Ok(vec![1, 2, 3]));
        let m = Arc::new(m);
        let w = Worker::start(m.clone(), Some(DiskCache::new(dir.clone(), CACHE_TTL)), Duration::ZERO, None);
        w.push(Job::Download { url: u.clone(), name: "x.png".into(), size: Some(4) });
        match wait_reply(&w) {
            Reply::Download(_, Err(e)) => assert!(e.contains("página dice 4"), "{e}"),
            r => panic!("{r:?}"),
        }
        w.push(Job::Download { url: u.clone(), name: "x.png".into(), size: Some(3) });
        let p = match wait_reply(&w) {
            Reply::Download(_, Ok(p)) => p,
            r => panic!("{r:?}"),
        };
        assert_eq!(p.file_name().unwrap(), "x.png", "keeps the original name");
        assert_eq!(fs::read(&p).unwrap(), vec![1, 2, 3]);
        // a second time comes from the cache
        w.push(Job::Download { url: u.clone(), name: "x.png".into(), size: Some(3) });
        assert!(matches!(wait_reply(&w), Reply::Download(_, Ok(_))));
        assert_eq!(m.hits.lock().unwrap().len(), 2);
        drop(w);
        let _ = fs::remove_dir_all(&dir);
    }
}
