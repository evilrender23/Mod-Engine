//! Terminal testing flags shared by `modengine run` and `check` (docs/pruebas.md):
//! `--seed`, `--exec-at`, `--assert`, `--save-dir` / `--no-save`, `--test-script`,
//! `--profile`, and screenshots at exact logic steps (`--screenshot-at`, `--crop`) taken
//! in a window that is hidden from the desktop.

use macroquad::prelude::*;
use modengine::Engine;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Mutex;

/// Test flags of the command line. All off by default.
#[derive(Default, Debug, Clone)]
pub struct TestArgs {
    pub seed: Option<u64>,
    pub exec_at: Vec<String>,
    pub asserts: Vec<String>,
    pub save_dir: Option<PathBuf>,
    pub no_save: bool,
    pub test_scripts: Vec<PathBuf>,
    pub profile: bool,
    /// `--screenshot-at 200,500,900`: logic steps to capture.
    pub shots: Vec<u64>,
    /// `--crop x,y,w,h` in game pixels.
    pub crop: Option<[u32; 4]>,
    /// `--show`: a visible window even when taking screenshots.
    pub show: bool,
    /// `--hidden`: no window on the desktop.
    pub hidden: bool,
    /// `check --render`: also draw on the GPU (in a hidden window).
    pub render: bool,
}

/// `Some(true)` when `flag` is a test flag that takes a value, `Some(false)` for a switch.
pub fn flag_kind(flag: &str) -> Option<bool> {
    match flag {
        "--seed" | "--exec-at" | "--assert" | "--save-dir" | "--test-script" | "--screenshot-at" | "--crop" => Some(true),
        "--no-save" | "--profile" | "--show" | "--hidden" | "--render" => Some(false),
        _ => None,
    }
}

fn numbers(v: &str, what: &str) -> Result<Vec<u64>, String> {
    v.split([',', ' ']).filter(|s| !s.trim().is_empty()).map(|s| s.trim().parse::<u64>().map_err(|_| format!("{what}: '{s}' no es un número"))).collect()
}

impl TestArgs {
    /// Stores one test flag (see [`flag_kind`]).
    pub fn set(&mut self, flag: &str, value: Option<String>) -> Result<(), String> {
        let v = value.unwrap_or_default();
        match flag {
            "--seed" => self.seed = Some(v.trim().parse().map_err(|_| format!("--seed: '{v}' no es un número entero"))?),
            "--exec-at" => self.exec_at.push(v),
            "--assert" => self.asserts.push(v),
            "--save-dir" => self.save_dir = Some(PathBuf::from(v)),
            "--test-script" => self.test_scripts.push(PathBuf::from(v)),
            "--screenshot-at" => self.shots.extend(numbers(&v, "--screenshot-at")?),
            "--crop" => {
                let n = numbers(&v, "--crop")?;
                if n.len() != 4 || n[2] == 0 || n[3] == 0 {
                    return Err(format!("--crop: se esperaba x,y,ancho,alto (p. ej. 0,0,320,180), no '{v}'"));
                }
                self.crop = Some([n[0] as u32, n[1] as u32, n[2] as u32, n[3] as u32]);
            }
            "--no-save" => self.no_save = true,
            "--profile" => self.profile = true,
            "--show" => self.show = true,
            "--hidden" => self.hidden = true,
            "--render" => self.render = true,
            _ => return Err(format!("unknown test flag {flag}")),
        }
        Ok(())
    }

    /// The last step named by `--exec-at N:` / `--assert N:` (a run with screenshots lasts
    /// at least until then).
    pub fn last_named_step(&self) -> Option<u64> {
        self.exec_at.iter().chain(&self.asserts).filter_map(|s| s.split_once(':')).filter_map(|(n, _)| n.trim().parse::<u64>().ok()).max()
    }

    /// Applies the flags to a loaded engine (before `start()`).
    pub fn apply(&self, e: &mut Engine) -> Result<(), String> {
        if let Some(n) = self.seed {
            e.set_seed(n);
        }
        if let Some(d) = &self.save_dir {
            e.save_dir = d.clone();
        } else if self.no_save {
            e.save_dir = no_save_dir()?;
        }
        for s in &self.exec_at {
            e.add_exec_at(s)?;
        }
        for s in &self.asserts {
            e.add_assert(s)?;
        }
        if self.profile {
            e.enable_profile();
        }
        Ok(())
    }
}

static NO_SAVE_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// `--no-save`: an empty temporary save folder, deleted by [`cleanup`].
fn no_save_dir() -> Result<PathBuf, String> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    let d = std::env::temp_dir().join(format!("modengine-nosave-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&d).map_err(|e| format!("--no-save: {e}"))?;
    *NO_SAVE_DIR.lock().unwrap() = Some(d.clone());
    Ok(d)
}

/// Removes the `--no-save` folder. Call before exiting.
pub fn cleanup() {
    if let Some(d) = NO_SAVE_DIR.lock().unwrap().take() {
        let _ = std::fs::remove_dir_all(d);
    }
}

static EXIT: AtomicI32 = AtomicI32::new(0);

/// Exit code of a windowed run (set by the backend when the game loop ends).
pub fn exit_code() -> i32 {
    EXIT.load(Ordering::SeqCst)
}

pub fn set_exit_code(c: i32) {
    EXIT.store(c, Ordering::SeqCst);
}

/// File name of the screenshot of `step`: `%d` / `%04d`-style or `{step}` in the pattern
/// become the step; without either, `_0200` (4 digits) goes before the extension.
pub fn shot_path(pattern: &str, step: u64) -> String {
    if let Some(i) = pattern.find('%') {
        let rest = &pattern[i + 1..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if rest[digits.len()..].starts_with('d') {
            let width: usize = digits.trim_start_matches('0').parse().unwrap_or(0);
            let num = if digits.starts_with('0') { format!("{step:0width$}") } else { format!("{step:width$}") };
            return format!("{}{}{}", &pattern[..i], num, &rest[digits.len() + 1..]);
        }
    }
    if pattern.contains("{step}") {
        return pattern.replace("{step}", &step.to_string());
    }
    let p = Path::new(pattern);
    let ext = p.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_else(|| "png".into());
    let stem = p.with_extension("");
    format!("{}_{step:04}.{ext}", stem.display())
}

/// What a windowed test run does (`run --screenshot-at / --steps / --hidden`, `check --render`).
pub struct WindowTest {
    /// Steps to capture, ascending.
    pub shots: Vec<u64>,
    pub pattern: String,
    pub crop: Option<[u32; 4]>,
    /// Quit after this step (the last screenshot or `--steps`).
    pub stop_at: Option<u64>,
    /// Exactly one logic step per rendered frame, whatever the real time (vsync off).
    pub lockstep: bool,
    pub hidden: bool,
    last: Option<u64>,
}

impl WindowTest {
    pub fn new(shots: &[u64], pattern: Option<String>, crop: Option<[u32; 4]>, steps: Option<u64>, hidden: bool) -> Self {
        let mut shots = shots.to_vec();
        shots.sort_unstable();
        shots.dedup();
        let stop_at = match (shots.last(), steps) {
            (Some(&s), Some(n)) => Some(s.max(n)),
            (Some(&s), None) => Some(s),
            (None, n) => n,
        };
        WindowTest { shots, pattern: pattern.unwrap_or_else(|| "shot_%04d.png".into()), crop, stop_at, lockstep: true, hidden, last: None }
    }

    /// After a frame was drawn into the game canvas `rt` (and flushed): saves the screenshot
    /// of this step if one is due. Returns true when the run is over.
    pub fn after_frame(&mut self, e: &Engine, rt: &Texture2D) -> bool {
        if self.last == Some(e.steps) {
            return false;
        }
        self.last = Some(e.steps);
        if self.shots.binary_search(&e.steps).is_ok() {
            let path = shot_path(&self.pattern, e.steps);
            match save_canvas(rt, &path, self.crop) {
                Ok(()) => println!("captura del paso {}: {path}", e.steps),
                Err(err) => {
                    eprintln!("--screenshot-at: no se pudo guardar {path}: {err}");
                    set_exit_code(1);
                }
            }
        }
        self.stop_at.is_some_and(|s| e.steps >= s)
    }
}

/// Saves the game's final image (what the window shows, before scaling; no debug overlay)
/// as PNG at the game's resolution. Opaque, like the window shows it.
pub fn save_canvas(rt: &Texture2D, path: &str, crop: Option<[u32; 4]>) -> Result<(), String> {
    let img = rt.get_texture_data();
    let (w, h) = (img.width as u32, img.height as u32);
    let rgba = canvas_pixels(img.bytes);
    let mut out = image::RgbaImage::from_raw(w, h, rgba).ok_or("bad canvas size")?;
    if let Some([x, y, cw, ch]) = crop {
        if x >= w || y >= h {
            return Err(format!("--crop {x},{y},{cw},{ch} queda fuera del lienzo de {w}x{h}"));
        }
        out = image::imageops::crop_imm(&out, x, y, cw.min(w - x), ch.min(h - y)).to_image();
    }
    if let Some(dir) = Path::new(path).parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    out.save(path).map_err(|e| e.to_string())
}

/// The window shows the canvas opaque (its alpha is ignored): the PNG does too. Render
/// targets are stored top row first (see canvas::camera_for), so no flip is needed.
pub fn canvas_pixels(mut bytes: Vec<u8>) -> Vec<u8> {
    bytes.chunks_exact_mut(4).for_each(|p| p[3] = 255);
    bytes
}

/// End of a windowed run: asserts, `--profile`, exit code for `main`.
pub fn finish_window(e: &mut Engine) {
    let code = e.test_finish();
    if e.error_count() > 0 {
        eprintln!("{} runtime error(s)", e.error_count());
    }
    set_exit_code(exit_code().max(code));
}

/// Window settings for a hidden test window: a unique title (to find and hide it) and no
/// vsync (the run goes as fast as the machine can draw).
pub fn prepare_hidden(conf: &mut macroquad::window::Conf) {
    conf.window_title = format!("{} [modengine-test {}]", conf.window_title, std::process::id());
    *HIDDEN_TITLE.lock().unwrap() = Some(conf.window_title.clone());
    conf.platform.swap_interval = Some(0);
    conf.fullscreen = false;
}

static HIDDEN_TITLE: Mutex<Option<String>> = Mutex::new(None);

/// Takes the test window off the desktop. On Linux/X11 (also XWayland) the window is
/// withdrawn (unmapped): rendering goes on into the game canvas, which is what the
/// screenshots read. Elsewhere it is moved far outside the screen.
/// Returns false when the window could not be withdrawn (then it was only moved away).
pub fn hide_window() -> bool {
    let title = HIDDEN_TITLE.lock().unwrap().clone().unwrap_or_default();
    #[cfg(target_os = "linux")]
    if x11::withdraw(&title) {
        return true;
    }
    let _ = title;
    macroquad::miniquad::window::set_window_position(30000, 30000);
    false
}

#[cfg(target_os = "linux")]
mod x11 {
    use macroquad::miniquad::native::module::linux::Module;
    use std::ffi::{c_char, c_int, c_uint, c_ulong, c_void, CStr};

    type Display = c_void;
    type Window = c_ulong;

    /// Finds the top-level window with this exact title and withdraws it.
    pub fn withdraw(title: &str) -> bool {
        let Ok(lib) = Module::load("libX11.so.6").or_else(|_| Module::load("libX11.so")) else { return false };
        type Open = unsafe extern "C" fn(*const c_char) -> *mut Display;
        type Root = unsafe extern "C" fn(*mut Display) -> Window;
        type Screen = unsafe extern "C" fn(*mut Display) -> c_int;
        type Query = unsafe extern "C" fn(*mut Display, Window, *mut Window, *mut Window, *mut *mut Window, *mut c_uint) -> c_int;
        type Fetch = unsafe extern "C" fn(*mut Display, Window, *mut *mut c_char) -> c_int;
        type Free = unsafe extern "C" fn(*mut c_void) -> c_int;
        type Withdraw = unsafe extern "C" fn(*mut Display, Window, c_int) -> c_int;
        type Sync = unsafe extern "C" fn(*mut Display, c_int) -> c_int;
        type Close = unsafe extern "C" fn(*mut Display) -> c_int;
        let (Ok(open), Ok(root), Ok(screen), Ok(query), Ok(fetch), Ok(free), Ok(withdraw), Ok(sync), Ok(close)) = (
            lib.get_symbol::<Open>("XOpenDisplay"),
            lib.get_symbol::<Root>("XDefaultRootWindow"),
            lib.get_symbol::<Screen>("XDefaultScreen"),
            lib.get_symbol::<Query>("XQueryTree"),
            lib.get_symbol::<Fetch>("XFetchName"),
            lib.get_symbol::<Free>("XFree"),
            lib.get_symbol::<Withdraw>("XWithdrawWindow"),
            lib.get_symbol::<Sync>("XSync"),
            lib.get_symbol::<Close>("XCloseDisplay"),
        ) else {
            return false;
        };
        // SAFETY: plain Xlib calls on our own display connection; every pointer Xlib returns
        // is checked and freed with XFree.
        unsafe {
            let d = open(std::ptr::null());
            if d.is_null() {
                return false;
            }
            let mut stack = vec![root(d)];
            let mut found = None;
            while let Some(w) = stack.pop() {
                let mut name: *mut c_char = std::ptr::null_mut();
                if fetch(d, w, &mut name) != 0 && !name.is_null() {
                    let same = CStr::from_ptr(name).to_string_lossy() == title;
                    free(name as *mut c_void);
                    if same {
                        found = Some(w);
                        break;
                    }
                }
                let (mut r, mut p, mut kids, mut n): (Window, Window, *mut Window, c_uint) = (0, 0, std::ptr::null_mut(), 0);
                if query(d, w, &mut r, &mut p, &mut kids, &mut n) != 0 && !kids.is_null() {
                    stack.extend_from_slice(std::slice::from_raw_parts(kids, n as usize));
                    free(kids as *mut c_void);
                }
            }
            if let Some(w) = found {
                withdraw(d, w, screen(d));
                sync(d, 0);
            }
            close(d);
            found.is_some()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shot_names() {
        assert_eq!(shot_path("shot_%04d.png", 200), "shot_0200.png");
        assert_eq!(shot_path("out/s%d.png", 7), "out/s7.png");
        assert_eq!(shot_path("a_{step}.png", 12), "a_12.png");
        assert_eq!(shot_path("capturas/menu.png", 90), "capturas/menu_0090.png");
        assert_eq!(shot_path("x", 3), "x_0003.png");
    }

    #[test]
    fn flags() {
        let mut t = TestArgs::default();
        t.set("--screenshot-at", Some("900,200, 500".into())).unwrap();
        t.set("--crop", Some("10,20,30,40".into())).unwrap();
        t.set("--seed", Some("42".into())).unwrap();
        assert_eq!((t.shots.clone(), t.crop, t.seed), (vec![900, 200, 500], Some([10, 20, 30, 40]), Some(42)));
        assert!(t.set("--crop", Some("1,2,3".into())).is_err());
        assert!(t.set("--seed", Some("x".into())).is_err());
        assert_eq!((flag_kind("--profile"), flag_kind("--assert"), flag_kind("--room")), (Some(false), Some(true), None));
        let w = WindowTest::new(&t.shots, None, None, Some(100), true);
        assert_eq!((w.shots.clone(), w.stop_at), (vec![200, 500, 900], Some(900)));
        assert_eq!(WindowTest::new(&[], None, None, Some(100), true).stop_at, Some(100));
        t.set("--assert", Some("300:x > 1".into())).unwrap();
        t.set("--assert", Some("a ? b : c".into())).unwrap();
        t.set("--exec-at", Some("120:foo()".into())).unwrap();
        assert_eq!(t.last_named_step(), Some(300));
    }

    #[test]
    fn canvas_is_opaque() {
        assert_eq!(canvas_pixels(vec![255, 0, 0, 255, 10, 20, 30, 128]), vec![255, 0, 0, 255, 10, 20, 30, 255]);
    }
}
