//! Running exported games: finds the game pack (appended to the executable, or `game.dat`
//! next to it, or in the APK assets on Android) and opens the game window.

use crate::backend::{self, RunOpts};
use modengine::Engine;
use std::path::{Path, PathBuf};

/// Name of the pack file next to the runner and inside the APK assets.
pub const PACK_FILE: &str = "game.dat";

/// Where an exported runner finds its game: the executable itself when a pack is appended
/// to it, otherwise `game.dat` (or `<exe name>.dat`) in the same folder.
pub fn find_pack(exe: &Path) -> Option<PathBuf> {
    if let Ok(mut f) = std::fs::File::open(exe) {
        if matches!(modengine::pack::appended(&mut f), Ok(Some(_))) {
            return Some(exe.to_path_buf());
        }
    }
    let dir = exe.parent()?;
    let stem = exe.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    [PACK_FILE.to_string(), format!("{stem}.dat")].into_iter().map(|n| dir.join(n)).find(|p| p.is_file())
}

/// The window/app icon in the three sizes miniquad wants (16, 32 and 64 pixels).
pub fn icon_for(e: &Engine) -> Option<macroquad::miniquad::conf::Icon> {
    let rel = e.config.game.icon.as_deref()?;
    let bytes = modengine::vfs::read(&e.dir.join(rel)).ok()?;
    let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let px = |n: u32| image::imageops::resize(&img, n, n, image::imageops::FilterType::Triangle).into_raw();
    let mut icon = macroquad::miniquad::conf::Icon { small: [0; 16 * 16 * 4], medium: [0; 32 * 32 * 4], big: [0; 64 * 64 * 4] };
    icon.small.copy_from_slice(&px(16));
    icon.medium.copy_from_slice(&px(32));
    icon.big.copy_from_slice(&px(64));
    Some(icon)
}

/// Window settings from game.toml.
pub fn window_conf(e: &Engine) -> macroquad::window::Conf {
    let d = &e.config.display;
    macroquad::window::Conf {
        window_title: e.config.game.name.clone(),
        window_width: (d.width * d.scale.max(1)) as i32,
        window_height: (d.height * d.scale.max(1)) as i32,
        fullscreen: d.fullscreen,
        window_resizable: true,
        high_dpi: false,
        icon: icon_for(e),
        ..Default::default()
    }
}

/// Opens the window and plays `e` until it quits.
pub fn play(e: Engine, opts: RunOpts) {
    let mut conf = window_conf(&e);
    if let Some(n) = opts.instance {
        conf.window_title = format!("{} [{n}]", conf.window_title);
    }
    if opts.test.as_ref().is_some_and(|t| t.hidden) {
        crate::test_run::prepare_hidden(&mut conf);
    }
    if !crate::display::available() {
        eprintln!("cannot open the game window: no X server or Wayland compositor found (DISPLAY and WAYLAND_DISPLAY are not set)");
        std::process::exit(1);
    }
    crate::display::prepare(&mut conf);
    macroquad::Window::from_config(conf, backend::run(e, opts));
}

/// Flags an exported game accepts (for testing and support): `--screenshot F.png`,
/// `--frames N`, `--touch`, `--input SPEC`, `--pack FILE`, `--headless` (dedicated server,
/// no window), `--steps N` (with --headless), `--instance N` (window N of several).
#[derive(Default, Debug, PartialEq)]
pub struct RunnerArgs {
    pub screenshot: Option<String>,
    pub frames: Option<u64>,
    pub touch: bool,
    pub input: Option<String>,
    pub pack: Option<PathBuf>,
    pub headless: bool,
    pub steps: Option<u64>,
    pub instance: Option<u32>,
}

pub fn parse_runner_args(raw: &[String]) -> Result<RunnerArgs, String> {
    let mut a = RunnerArgs::default();
    let mut it = raw.iter();
    while let Some(f) = it.next() {
        let mut value = || it.next().cloned().ok_or_else(|| format!("{f} needs a value"));
        match f.as_str() {
            "--screenshot" => a.screenshot = Some(value()?),
            "--frames" => a.frames = Some(value()?.parse().map_err(|_| "--frames needs a number".to_string())?),
            "--touch" => a.touch = true,
            "--input" => a.input = Some(value()?),
            "--pack" => a.pack = Some(PathBuf::from(value()?)),
            "--headless" | "--server" => a.headless = true,
            "--steps" => a.steps = Some(value()?.parse().map_err(|_| "--steps needs a number".to_string())?),
            "--instance" => a.instance = Some(value()?.parse().map_err(|_| "--instance needs a number".to_string())?),
            // macOS adds -psn_... when started from Finder; ignore unknown flags
            _ => {}
        }
    }
    Ok(a)
}

/// Shows a fatal message where the player can see it: stderr, `error.log`, and on Windows a
/// native message box (exported games have no console, and a GL window may be the very thing
/// that failed); elsewhere a small window.
pub fn fatal(msg: &str) -> ! {
    eprintln!("{msg}");
    let log = write_error_log(msg);
    #[cfg(windows)]
    {
        let extra = log.map(|p| format!("\n\nGuardado en: {}", p.display())).unwrap_or_default();
        message_box("Mod Engine", &format!("{msg}{extra}"));
        std::process::exit(1)
    }
    #[cfg(not(windows))]
    fatal_window(msg, log)
}

#[cfg(not(windows))]
fn fatal_window(msg: &str, _log: Option<PathBuf>) -> ! {
    let text = msg.to_string();
    if !crate::display::available() {
        std::process::exit(1);
    }
    let mut conf = macroquad::window::Conf { window_title: "Mod Engine".into(), window_width: 720, window_height: 240, ..Default::default() };
    crate::display::prepare(&mut conf);
    macroquad::Window::from_config(
        conf,
        async move {
            use macroquad::prelude::*;
            loop {
                clear_background(Color::new(0.12, 0.05, 0.05, 1.0));
                for (i, l) in text.lines().take(10).enumerate() {
                    draw_text(l, 16.0, 32.0 + i as f32 * 22.0, 20.0, WHITE);
                }
                if is_key_pressed(KeyCode::Escape) || is_quit_requested() {
                    break;
                }
                next_frame().await;
            }
        },
    );
    std::process::exit(1)
}

/// `error.log` in the game's user folder (or next to the executable): what went wrong when the
/// game could not even open its window.
fn error_log_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let name = exe.file_stem()?.to_string_lossy().to_string();
    let env = |k: &str| std::env::var(k).ok();
    let dir = modengine::vfs::user_data_dir(&name, None, &env);
    if std::fs::create_dir_all(&dir).is_ok() {
        return Some(dir.join("error.log"));
    }
    exe.parent().map(|d| d.join("error.log"))
}

fn write_error_log(msg: &str) -> Option<PathBuf> {
    let p = error_log_path()?;
    std::fs::write(&p, format!("{msg}\n")).ok()?;
    Some(p)
}

/// A native Windows message box (user32 is always there; no extra crate).
#[cfg(windows)]
pub fn message_box(title: &str, text: &str) {
    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(hwnd: *mut core::ffi::c_void, text: *const u16, caption: *const u16, kind: u32) -> i32;
    }
    let w = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (t, c) = (w(text), w(title));
    // MB_OK | MB_ICONERROR
    // SAFETY: both buffers are NUL-terminated UTF-16 and outlive the call.
    unsafe {
        MessageBoxW(core::ptr::null_mut(), t.as_ptr(), c.as_ptr(), 0x10);
    }
}

/// What to tell the player when the window/graphics layer panics (no usable OpenGL 3).
fn explain_panic(raw: &str) -> String {
    // miniquad panics in WGL setup, or unwraps a missing GL function in native/gl.rs when the
    // driver only has OpenGL 1.1 (Windows without a GPU driver, VMs without 3D).
    let gl = ["WGL", "OpenGL", "GL_", "pixel format", "context", "native/gl.rs", "native\\gl.rs"].iter().any(|k| raw.contains(k));
    if gl {
        format!(
            "No se pudo iniciar el juego: este equipo no tiene OpenGL 3.\n\n\
             Pasa en máquinas virtuales sin aceleración 3D o con drivers de gráfica viejos. \
             Instala los drivers de la tarjeta gráfica o activa la aceleración 3D de la máquina virtual \
             (en QEMU: virtio-gpu con virgl, o pasar la GPU). También sirve poner junto al juego el \
             opengl32.dll de Mesa (OpenGL por software).\n\nDetalle: {raw}"
        )
    } else {
        format!("El juego se cerró por un error:\n\n{raw}")
    }
}

/// `modengine-runner` (an exported game) on desktop.
pub fn desktop_main() {
    // A panic in an exported game must be seen: there is no console on Windows.
    std::panic::set_hook(Box::new(|info| {
        let raw = match (info.payload().downcast_ref::<&str>(), info.payload().downcast_ref::<String>()) {
            (Some(s), _) => s.to_string(),
            (_, Some(s)) => s.clone(),
            _ => "panic".to_string(),
        };
        let at = info.location().map(|l| format!(" ({}:{})", l.file(), l.line())).unwrap_or_default();
        let msg = explain_panic(&format!("{raw}{at}"));
        eprintln!("{msg}");
        let log = write_error_log(&msg);
        #[cfg(windows)]
        message_box("Mod Engine", &format!("{msg}{}", log.map(|p| format!("\n\nGuardado en: {}", p.display())).unwrap_or_default()));
        #[cfg(not(windows))]
        let _ = log;
    }));
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let args = parse_runner_args(&raw).unwrap_or_else(|e| fatal(&e));
    let exe = std::env::current_exe().unwrap_or_else(|e| fatal(&format!("cannot find the game executable: {e}")));
    let Some(pack) = args.pack.clone().or_else(|| find_pack(&exe)) else {
        fatal(&format!("No game data found.\nThis runner needs a game pack appended to it or a {PACK_FILE} file next to it:\n{}", exe.display()))
    };
    let e = Engine::load_pack(&pack).unwrap_or_else(|m| fatal(&format!("Could not load the game:\n{m}")));
    let input = args.input.as_ref().map(|s| crate::script_input::ScriptedInput::parse(s).unwrap_or_else(|m| fatal(&m)));
    if args.headless {
        // a dedicated server: no window (and no message boxes; errors go to the console)
        let code = crate::headless::run(e, crate::headless::HeadlessOpts { input, steps: args.steps, watch: vec![], realtime: true });
        std::process::exit(code);
    }
    let opts = RunOpts { screenshot: args.screenshot.map(|p| (args.frames.unwrap_or(120), p)), input, touch: args.touch.then_some(true), instance: args.instance, test: None };
    play(e, opts);
}

/// Android: the pack is `assets/game.dat` inside the APK.
#[cfg(target_os = "android")]
pub fn android_main() {
    let conf = macroquad::window::Conf { window_title: "Mod Engine".into(), fullscreen: true, high_dpi: true, ..Default::default() };
    macroquad::Window::from_config(conf, async {
        let bytes = match macroquad::file::load_file(PACK_FILE).await {
            Ok(b) => b,
            Err(e) => return android_error(format!("cannot read {PACK_FILE}: {e}")).await,
        };
        let pack = match modengine::pack::Pack::parse(bytes) {
            Ok(p) => p,
            Err(e) => return android_error(e).await,
        };
        let src = modengine::vfs::mount(Path::new(PACK_FILE), pack);
        match Engine::load_source(&src) {
            Ok(e) => backend::run(e, RunOpts { screenshot: None, input: None, touch: Some(true), instance: None, test: None }).await,
            Err(e) => android_error(e).await,
        }
    });
}

#[cfg(target_os = "android")]
async fn android_error(msg: String) {
    use macroquad::prelude::*;
    macroquad::miniquad::error!("{}", msg);
    loop {
        clear_background(Color::new(0.12, 0.05, 0.05, 1.0));
        for (i, l) in msg.lines().take(12).enumerate() {
            draw_text(l, 16.0, 40.0 + i as f32 * 30.0, 28.0, WHITE);
        }
        next_frame().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graphics_panics_are_explained() {
        assert!(explain_panic("WGL: ARB_create_context required!").contains("OpenGL 3"));
        assert!(explain_panic("WGL_ARB_pixel_format is required").contains("máquinas virtuales"));
        assert!(explain_panic("called `Option::unwrap()` on a `None` value (miniquad-0.4.11/src/native/gl.rs:339)").contains("OpenGL 3"));
        assert!(explain_panic("index out of bounds").starts_with("El juego se cerró"));
    }

    #[test]
    fn runner_flags() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(parse_runner_args(&[]).unwrap(), RunnerArgs::default());
        let a = parse_runner_args(&s(&["--screenshot", "a.png", "--frames", "90", "--touch", "-psn_0_1"])).unwrap();
        assert_eq!((a.screenshot.as_deref(), a.frames, a.touch), (Some("a.png"), Some(90), true));
        assert!(parse_runner_args(&s(&["--frames", "x"])).is_err());
        assert!(parse_runner_args(&s(&["--pack"])).is_err());
        let a = parse_runner_args(&s(&["--headless", "--steps", "600", "--instance", "2"])).unwrap();
        assert_eq!((a.headless, a.steps, a.instance), (true, Some(600), Some(2)));
    }

    #[test]
    fn finds_the_pack_appended_or_next_to_the_runner() {
        let dir = std::env::temp_dir().join(format!("me_find_pack_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let pack = modengine::pack::write(&[("game.toml".into(), b"[game]".to_vec())]);
        let exe = dir.join("Game");
        std::fs::write(&exe, modengine::pack::append_to(b"runner", &pack)).unwrap();
        assert_eq!(find_pack(&exe), Some(exe.clone()));
        let plain = dir.join("Other");
        std::fs::write(&plain, b"runner").unwrap();
        assert_eq!(find_pack(&plain), None);
        std::fs::write(dir.join("Other.dat"), &pack).unwrap();
        assert_eq!(find_pack(&plain), Some(dir.join("Other.dat")));
        std::fs::write(dir.join(PACK_FILE), &pack).unwrap();
        assert_eq!(find_pack(&plain), Some(dir.join(PACK_FILE)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
