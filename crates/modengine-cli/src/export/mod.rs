//! `modengine export`: turns a project into a game players can run, like GameMaker's
//! "Create Executable".
//!
//! An exported game is a *runner* (the engine without the editor or the command line) plus
//! the project packed into one file (`game.dat`, see `modengine::pack`):
//!
//! - Linux: `<Game>/<Game>` (runner with the pack appended) + .desktop + README, and a .tar.gz
//! - Windows: `<Game>/<Game>.exe` (no console window) + README, and a .zip
//! - Android: `<Game>.apk` (arm64-v8a by default) with the pack in its assets
//!
//! Runners are compiled once per engine version and target and cached in
//! `~/.cache/modengine/runners/`, so later exports only pack the project (seconds).
//! Compiling them needs the engine source: the workspace when running from
//! `target/release`, `<install>/codigo` for installed copies, or `MODENGINE_SRC`.
//!
//! Progress lines for the editor: `PASO n/total texto`, then `LISTO <path>` or `ERROR texto`.

pub mod android;
pub mod archive;
pub mod tools;

use modengine::pack;
use modengine::project::GameToml;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tools::{human, Tools};

pub type Log<'a> = &'a dyn Fn(&str);

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const ZIG_VERSION: &str = "0.15.2";
/// Mesa for Windows (software OpenGL), from https://github.com/pal1000/mesa-dist-win.
const MESA_VERSION: &str = "26.2.6";
/// The two files a game needs for software OpenGL: Windows loads `opengl32.dll` from the game's
/// folder before the system one, and it loads llvmpipe from `libgallium_wgl.dll`.
pub const MESA_DLLS: [&str; 2] = ["opengl32.dll", "libgallium_wgl.dll"];
const CMDLINE_TOOLS: &str = "16111833";
const NDK_VERSION: &str = "27.3.13750724";
const SDK_PACKAGES: &[&str] = &["platforms;android-35", "build-tools;35.0.0", "platform-tools"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Linux,
    Windows,
    Android,
}

impl Target {
    pub const ALL: [Target; 3] = [Target::Windows, Target::Linux, Target::Android];

    pub fn parse(s: &str) -> Option<Target> {
        match s.to_lowercase().as_str() {
            "linux" => Some(Target::Linux),
            "windows" | "win" | "win64" => Some(Target::Windows),
            "android" | "apk" => Some(Target::Android),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Target::Linux => "linux",
            Target::Windows => "windows",
            Target::Android => "android",
        }
    }

    /// Rust target of the desktop runner built on this host.
    pub fn triple(self) -> &'static str {
        let host = tools::host_triple();
        match self {
            Target::Linux if host.contains("linux") => host,
            Target::Linux => "x86_64-unknown-linux-gnu",
            Target::Windows if host.contains("windows") => host,
            Target::Windows => "x86_64-pc-windows-gnu",
            Target::Android => "aarch64-linux-android",
        }
    }
}

#[derive(Debug, PartialEq, Default)]
pub struct ExportArgs {
    pub project: PathBuf,
    pub target: Option<Target>,
    pub out: Option<PathBuf>,
    /// Android: sign with the release key (environment variables) instead of the debug key.
    pub release: bool,
    /// Android ABIs (default: `[android] abis` of game.toml, else arm64-v8a).
    pub arches: Vec<String>,
    pub keystore: Option<PathBuf>,
    pub key_alias: Option<String>,
    /// Android: `adb install` the APK on the connected device.
    pub install: bool,
    /// Windows: add Mesa's software OpenGL (overrides `[export] windows_software_gl`).
    pub software_gl: bool,
}

#[derive(Debug, PartialEq)]
pub enum ExportCmd {
    Export(ExportArgs),
    Setup(Target),
    Doctor { json: bool },
}

pub const USAGE: &str = "\
usage:
  modengine export <project> --target linux|windows|android [--out DIR] [--release]
                   [--arch arm64-v8a,armeabi-v7a,x86_64] [--keystore F --key-alias A] [--install]
                   [--software-gl]   Windows: ship Mesa's software OpenGL (VMs without 3D)
  modengine export --setup linux|windows|android     install/check the toolchain (user space)
  modengine export --doctor [--json]                 which targets are ready

--release   Android: sign with your release key. The password comes from the environment
            (MODENGINE_KEYSTORE_PASS, MODENGINE_KEY_PASS); MODENGINE_KEYSTORE and
            MODENGINE_KEY_ALIAS can replace --keystore/--key-alias.
--install   Android: install the APK on the device `adb devices` shows.";

fn normalize_abi(a: &str) -> Option<String> {
    let a = match a.trim() {
        "arm64" | "aarch64" | "arm64-v8a" => "arm64-v8a",
        "arm" | "armv7" | "armeabi-v7a" => "armeabi-v7a",
        "x86_64" | "x64" => "x86_64",
        "x86" | "i686" => "x86",
        _ => return None,
    };
    Some(a.to_string())
}

pub fn parse_args(raw: &[String]) -> Result<ExportCmd, String> {
    let mut a = ExportArgs::default();
    let mut setup = None;
    let (mut doctor, mut json) = (false, false);
    let mut it = raw.iter();
    while let Some(f) = it.next() {
        let mut value = || it.next().cloned().ok_or_else(|| format!("{f} needs a value"));
        match f.as_str() {
            "--target" | "-t" => {
                let v = value()?;
                a.target = Some(Target::parse(&v).ok_or_else(|| format!("unknown target '{v}' (linux, windows or android)"))?);
            }
            "--out" | "-o" => a.out = Some(PathBuf::from(value()?)),
            "--release" => a.release = true,
            "--software-gl" => a.software_gl = true,
            "--arch" => {
                for x in value()?.split(',').filter(|s| !s.trim().is_empty()) {
                    let n = normalize_abi(x).ok_or_else(|| format!("unknown architecture '{x}' (arm64-v8a, armeabi-v7a, x86_64, x86)"))?;
                    if !a.arches.contains(&n) {
                        a.arches.push(n);
                    }
                }
            }
            "--keystore" => a.keystore = Some(PathBuf::from(value()?)),
            "--key-alias" => a.key_alias = Some(value()?),
            "--install" => a.install = true,
            "--setup" => {
                let v = value()?;
                setup = Some(Target::parse(&v).ok_or_else(|| format!("unknown target '{v}' (linux, windows or android)"))?);
            }
            "--doctor" => doctor = true,
            "--json" => json = true,
            "-h" | "--help" => return Err(USAGE.into()),
            s if s.starts_with('-') => return Err(format!("unknown option {s}\n{USAGE}")),
            s => {
                if !a.project.as_os_str().is_empty() {
                    return Err(format!("unexpected argument '{s}'"));
                }
                a.project = PathBuf::from(s);
            }
        }
    }
    if doctor {
        return Ok(ExportCmd::Doctor { json });
    }
    if let Some(t) = setup {
        return Ok(ExportCmd::Setup(t));
    }
    if a.project.as_os_str().is_empty() {
        return Err(format!("which project?\n{USAGE}"));
    }
    if a.target.is_none() {
        return Err(format!("--target is required (linux, windows or android)\n{USAGE}"));
    }
    Ok(ExportCmd::Export(a))
}

/// `modengine export ...`: returns the process exit code.
pub fn main_cli(raw: &[String]) -> i32 {
    let log = |s: &str| println!("{s}");
    match parse_args(raw) {
        Err(e) => {
            eprintln!("{e}");
            2
        }
        Ok(ExportCmd::Doctor { json }) => {
            let st = doctor(&Tools::detect());
            if json {
                println!("{}", doctor_json(&st));
            } else {
                for s in &st {
                    println!("{} {}", if s.ready { "✔" } else { "⚠" }, s.target.name());
                    for c in &s.checks {
                        println!("    {} {}: {}", if c.ok { "✔" } else { "✘" }, c.name, c.detail);
                    }
                    if !s.ready {
                        println!("    → modengine export --setup {}   ({})", s.target.name(), s.setup);
                    }
                }
            }
            if st.iter().all(|s| s.ready) { 0 } else { 1 }
        }
        Ok(ExportCmd::Setup(t)) => match setup(t, &log) {
            Ok(()) => {
                println!("LISTO {}", t.name());
                0
            }
            Err(e) => {
                println!("ERROR {}", e.replace('\n', " "));
                1
            }
        },
        Ok(ExportCmd::Export(a)) => match export(&a, &log) {
            Ok(p) => {
                println!("LISTO {}", p.display());
                0
            }
            Err(e) => {
                println!("ERROR {}", e.replace('\n', " "));
                1
            }
        },
    }
}

// ================================================================== running tools

/// Runs a command, sending every output line to `log`. The error carries the last lines.
pub fn run_logged(cmd: &mut Command, log: Log) -> Result<(), String> {
    cmd.env("PATH", tools::child_path()).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let shown = format!("{} {}", cmd.get_program().to_string_lossy(), cmd.get_args().map(|a| a.to_string_lossy()).collect::<Vec<_>>().join(" "));
    log(&format!("$ {shown}"));
    let mut child = cmd.spawn().map_err(|e| format!("cannot run {}: {e}", cmd.get_program().to_string_lossy()))?;
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let mut readers = vec![];
    if let Some(o) = child.stdout.take() {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || BufReader::new(o).lines().map_while(Result::ok).for_each(|l| drop(tx.send(l)))));
    }
    if let Some(e) = child.stderr.take() {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || BufReader::new(e).lines().map_while(Result::ok).for_each(|l| drop(tx.send(l)))));
    }
    drop(tx);
    let mut tail: Vec<String> = vec![];
    for l in rx {
        log(&format!("  {l}"));
        tail.push(l);
        if tail.len() > 12 {
            tail.remove(0);
        }
    }
    readers.into_iter().for_each(|r| drop(r.join()));
    let st = child.wait().map_err(|e| e.to_string())?;
    if st.success() {
        Ok(())
    } else {
        Err(format!("{} failed ({st}):\n{}", cmd.get_program().to_string_lossy(), tail.join("\n")))
    }
}

// ================================================================== engine source & runners

/// The engine source used to compile runners.
pub fn find_source() -> Option<PathBuf> {
    let ok = |p: &Path| p.join("crates/modengine-cli/Cargo.toml").is_file() && p.join("Cargo.toml").is_file();
    if let Some(s) = std::env::var_os("MODENGINE_SRC").map(PathBuf::from).filter(|p| ok(p)) {
        return Some(s);
    }
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    if let Some(a) = exe.ancestors().skip(1).take(5).find(|a| ok(a)) {
        return Some(a.to_path_buf());
    }
    // installed: <destino>/app/modengine and <destino>/codigo
    exe.parent()?.parent().map(|d| d.join("codigo")).filter(|p| ok(p))
}

/// Fingerprint of the engine source (so cached runners follow local changes).
pub fn source_hash(src: &Path) -> String {
    fn walk(root: &Path, d: &Path, out: &mut Vec<(String, PathBuf)>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let n = e.file_name().to_string_lossy().to_string();
            // the exporter itself is not part of the runners: editing it keeps the cache
            if n.starts_with('.') || n == "target" || n == "tests" || n == "export" {
                continue;
            }
            if p.is_dir() {
                walk(root, &p, out);
            } else if p.extension().is_some_and(|x| x == "rs" || x == "toml" || x == "java") {
                out.push((p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('\\', "/"), p));
            }
        }
    }
    let mut files = vec![("Cargo.lock".to_string(), src.join("Cargo.lock")), ("Cargo.toml".to_string(), src.join("Cargo.toml"))];
    for c in ["modscript", "modengine", "modengine-cli"] {
        walk(src, &src.join("crates").join(c), &mut files);
    }
    files.sort();
    let mut buf = Vec::new();
    for (rel, p) in files {
        buf.extend_from_slice(rel.as_bytes());
        buf.extend_from_slice(&std::fs::read(&p).unwrap_or_default());
    }
    format!("{:016x}", pack::fnv1a64(&[&buf]))[..12].to_string()
}

fn runners_dir() -> PathBuf {
    tools::cache_dir().join("runners")
}

/// A runner already compiled for this engine version and target (any source hash), newest
/// first: what installs without source fall back to.
fn any_cached(triple: &str, file: &str) -> Option<PathBuf> {
    let mut v: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(runners_dir())
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let n = p.file_name().unwrap_or_default().to_string_lossy().to_string();
            n.starts_with(&format!("{VERSION}-")) && n.ends_with(&format!("-{triple}")) && p.join(file).is_file()
        })
        .filter_map(|p| Some((std::fs::metadata(p.join(file)).ok()?.modified().ok()?, p.join(file))))
        .collect();
    v.sort();
    v.pop().map(|(_, p)| p)
}

/// The desktop runner executable for `target`, compiling it if needed.
pub fn desktop_runner(target: Target, t: &Tools, log: Log) -> Result<PathBuf, String> {
    let triple = target.triple();
    let file = if triple.contains("windows") { "modengine-runner.exe" } else { "modengine-runner" };
    if triple == tools::host_triple() {
        if let Some(sib) = std::env::current_exe().ok().and_then(|e| e.canonicalize().ok()).and_then(|e| e.parent().map(|d| d.join(file))).filter(|p| p.is_file()) {
            log(&format!("runner: {}", sib.display()));
            return Ok(sib);
        }
    }
    let Some(src) = find_source() else {
        return any_cached(triple, file).ok_or_else(|| no_source_msg(target));
    };
    let cached = runners_dir().join(format!("{VERSION}-{}-{triple}", source_hash(&src))).join(file);
    if cached.is_file() {
        log(&format!("runner en caché: {}", cached.display()));
        return Ok(cached);
    }
    let cargo = t.cargo.clone().ok_or("no se encuentra cargo (instala Rust: https://rustup.rs)")?;
    log(&format!("compilando el runner para {triple} (solo la primera vez; tarda unos minutos)"));
    let cross = triple != tools::host_triple();
    let mut c = Command::new(&cargo);
    if cross {
        if !t.has_target(triple) {
            return Err(format!("falta el target de Rust {triple}: modengine export --setup {}", target.name()));
        }
        if t.zig.is_none() || t.zigbuild.is_none() {
            return Err(format!("compilar para {} desde aquí necesita zig y cargo-zigbuild: modengine export --setup {}", target.name(), target.name()));
        }
        c.arg("zigbuild");
    } else {
        c.arg("build");
    }
    c.args(["--release", "-p", "modengine-cli", "--bin", "modengine-runner", "--target", triple, "--config", "profile.release.strip=\"symbols\""]);
    c.current_dir(&src).env("CARGO_TARGET_DIR", tools::cache_dir().join("build"));
    if let Some(z) = &t.zig {
        c.env("CARGO_ZIGBUILD_ZIG_PATH", z);
    }
    run_logged(&mut c, log)?;
    let built = tools::cache_dir().join("build").join(triple).join("release").join(file);
    store_runner(&built, &cached, triple)?;
    Ok(cached)
}

/// Copies a freshly built runner into the cache and drops older builds of the same target.
fn store_runner(built: &Path, cached: &Path, triple: &str) -> Result<(), String> {
    std::fs::create_dir_all(cached.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::copy(built, cached).map_err(|e| format!("{}: {e}", built.display()))?;
    let keep = cached.parent().unwrap().to_path_buf();
    if let Ok(rd) = std::fs::read_dir(runners_dir()) {
        for d in rd.flatten().map(|e| e.path()) {
            if d != keep && d.file_name().is_some_and(|n| n.to_string_lossy().ends_with(&format!("-{triple}"))) {
                let _ = std::fs::remove_dir_all(d);
            }
        }
    }
    Ok(())
}

fn no_source_msg(target: Target) -> String {
    format!(
        "para exportar a {} hay que compilar el runner y no se encuentra el código del motor. \
         Instala Mod Engine con instalar.sh/instalar.ps1 (deja el código en <instalación>/codigo) \
         o indica una copia del repositorio con la variable MODENGINE_SRC.",
        target.name()
    )
}

/// `libmodengine_cli.so` for an Android ABI, compiling it if needed.
pub fn android_lib(abi: &str, t: &Tools, log: Log) -> Result<PathBuf, String> {
    let (triple, clang) = android::abi_info(abi).ok_or_else(|| format!("unknown ABI {abi}"))?;
    let file = format!("lib{}.so", android::LIB_NAME);
    let Some(src) = find_source() else {
        return any_cached(triple, &file).ok_or_else(|| no_source_msg(Target::Android));
    };
    let cached = runners_dir().join(format!("{VERSION}-{}-{triple}", source_hash(&src))).join(&file);
    if cached.is_file() {
        log(&format!("biblioteca {abi} en caché: {}", cached.display()));
        return Ok(cached);
    }
    let cargo = t.cargo.clone().ok_or("no se encuentra cargo (instala Rust: https://rustup.rs)")?;
    if !t.has_target(triple) {
        return Err(format!("falta el target de Rust {triple}: modengine export --setup android"));
    }
    let linker = t.ndk_clang(clang, android::MIN_SDK).ok_or("falta el NDK de Android: modengine export --setup android")?;
    let ar = t.ndk_tool("llvm-ar").ok_or("el NDK no tiene llvm-ar")?;
    log(&format!("compilando el motor para Android {abi} (solo la primera vez; tarda unos minutos)"));
    let up = triple.to_uppercase().replace('-', "_");
    let mut c = Command::new(&cargo);
    c.args(["rustc", "--release", "-p", "modengine-cli", "--lib", "--crate-type", "cdylib", "--target", triple, "--config", "profile.release.strip=\"symbols\""]);
    c.current_dir(&src).env("CARGO_TARGET_DIR", tools::cache_dir().join("build"));
    c.env(format!("CARGO_TARGET_{up}_LINKER"), &linker);
    // 16 KB pages (Android 15+ devices)
    c.env(format!("CARGO_TARGET_{up}_RUSTFLAGS"), "-C link-arg=-Wl,-z,max-page-size=16384");
    c.env(format!("CC_{}", triple.replace('-', "_")), &linker).env(format!("AR_{}", triple.replace('-', "_")), &ar);
    run_logged(&mut c, log)?;
    let built = tools::cache_dir().join("build").join(triple).join("release").join(&file);
    store_runner(&built, &cached, triple)?;
    Ok(cached)
}

// ================================================================== doctor

#[derive(Debug, Clone)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct TargetStatus {
    pub target: Target,
    pub ready: bool,
    pub checks: Vec<Check>,
    /// What `--setup` would install, with sizes.
    pub setup: String,
}

fn check(name: &str, ok: bool, detail: impl Into<String>) -> Check {
    Check { name: name.into(), ok, detail: detail.into() }
}

fn shown(p: &Option<PathBuf>) -> String {
    p.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "no encontrado".into())
}

pub fn doctor(t: &Tools) -> Vec<TargetStatus> {
    let src = find_source();
    let hash = src.as_ref().map(|s| source_hash(s));
    let cached = |triple: &str, file: &str| match &hash {
        Some(h) => runners_dir().join(format!("{VERSION}-{h}-{triple}")).join(file).is_file(),
        None => any_cached(triple, file).is_some(),
    };
    let src_check = check("código del motor", src.is_some(), src.as_ref().map(|s| s.display().to_string()).unwrap_or_else(|| "no encontrado (MODENGINE_SRC o <instalación>/codigo)".into()));
    let cargo_check = check("cargo", t.cargo.is_some(), shown(&t.cargo));
    let mut out = vec![];
    for target in Target::ALL {
        let mut checks = vec![];
        let ready;
        let setup;
        match target {
            Target::Linux | Target::Windows => {
                let triple = target.triple();
                let file = if triple.contains("windows") { "modengine-runner.exe" } else { "modengine-runner" };
                let sibling = triple == tools::host_triple()
                    && std::env::current_exe().ok().and_then(|e| e.canonicalize().ok()).and_then(|e| e.parent().map(|d| d.join(file))).is_some_and(|p| p.is_file());
                let have_runner = sibling || cached(triple, file);
                checks.push(check("runner", have_runner, if sibling { "junto a modengine".to_string() } else if have_runner { "compilado en caché".into() } else { "se compilará en la primera exportación".into() }));
                let cross = triple != tools::host_triple();
                let mut can_build = src.is_some() && t.cargo.is_some();
                if !have_runner {
                    checks.push(src_check.clone());
                    checks.push(cargo_check.clone());
                }
                if cross && !have_runner {
                    let tgt = t.has_target(triple);
                    checks.push(check(&format!("target de Rust {triple}"), tgt, if tgt { "instalado" } else { "falta (rustup target add)" }));
                    checks.push(check("zig", t.zig.is_some(), shown(&t.zig)));
                    checks.push(check("cargo-zigbuild", t.zigbuild.is_some(), shown(&t.zigbuild)));
                    can_build &= tgt && t.zig.is_some() && t.zigbuild.is_some();
                }
                ready = have_runner || can_build;
                setup = if cross { format!("target de Rust (~60 MB), zig {ZIG_VERSION} (~55 MB de descarga, ~375 MB instalado), cargo-zigbuild") } else { "nada: compila con el Rust de este equipo".into() };
            }
            Target::Android => {
                let bt = t.build_tools();
                let jar = t.android_jar();
                checks.push(check("SDK de Android", t.android_sdk.is_some(), shown(&t.android_sdk)));
                checks.push(check("build-tools (aapt2, d8, apksigner)", bt.is_some(), shown(&bt)));
                checks.push(check("plataforma (android.jar)", jar.is_some(), shown(&jar)));
                checks.push(check("Java (javac)", t.javac.is_some(), shown(&t.javac)));
                let lib = cached("aarch64-linux-android", &format!("lib{}.so", android::LIB_NAME));
                checks.push(check("motor para arm64-v8a", lib, if lib { "compilado en caché" } else { "se compilará en la primera exportación" }));
                let mut can_build = true;
                if !lib {
                    checks.push(src_check.clone());
                    checks.push(cargo_check.clone());
                    let tgt = t.has_target("aarch64-linux-android");
                    checks.push(check("target de Rust aarch64-linux-android", tgt, if tgt { "instalado" } else { "falta" }));
                    checks.push(check("NDK", t.ndk.is_some(), shown(&t.ndk)));
                    can_build = src.is_some() && t.cargo.is_some() && tgt && t.ndk.is_some();
                }
                let adb = t.adb();
                checks.push(check("adb (opcional, para instalar en el móvil)", adb.is_some(), shown(&adb)));
                ready = bt.is_some() && jar.is_some() && t.javac.is_some() && (lib || can_build);
                setup = format!("command-line tools (~170 MB), plataforma 35 + build-tools 35 (~120 MB), platform-tools (~8 MB), NDK {NDK_VERSION} (~630 MB de descarga, ~2 GB instalado); necesita Java 17+");
            }
        }
        // a runner that can be compiled on the first export is not a problem
        for c in checks.iter_mut().filter(|c| c.name == "runner" || c.name.starts_with("motor para")) {
            c.ok |= ready;
        }
        out.push(TargetStatus { target, ready, checks, setup });
    }
    out
}

pub fn doctor_json(st: &[TargetStatus]) -> String {
    let v: Vec<serde_json::Value> = st
        .iter()
        .map(|s| {
            serde_json::json!({
                "target": s.target.name(),
                "ready": s.ready,
                "setup": s.setup,
                "checks": s.checks.iter().map(|c| serde_json::json!({"name": c.name, "ok": c.ok, "detail": c.detail})).collect::<Vec<_>>(),
            })
        })
        .collect();
    serde_json::Value::Array(v).to_string()
}

// ================================================================== setup

fn download(url: &str, to: &Path, log: Log) -> Result<(), String> {
    std::fs::create_dir_all(to.parent().unwrap()).map_err(|e| e.to_string())?;
    let curl = tools::which("curl").ok_or("hace falta curl para descargar")?;
    log(&format!("descargando {url}"));
    run_logged(Command::new(curl).args(["-fL", "--retry", "3", "-sS", "-o"]).arg(to).arg(url), log)?;
    log(&format!("  {} descargados", human(std::fs::metadata(to).map(|m| m.len()).unwrap_or(0))));
    Ok(())
}

/// Mesa's x64 software OpenGL DLLs, downloaded once into the toolchains folder.
fn mesa_dlls(log: Log) -> Result<Vec<(String, Vec<u8>)>, String> {
    let dir = tools::toolchains_dir().join(format!("mesa-{MESA_VERSION}"));
    if !MESA_DLLS.iter().all(|f| dir.join(f).is_file()) {
        let archive = tools::toolchains_dir().join("downloads").join(format!("mesa-{MESA_VERSION}.7z"));
        download(&format!("https://github.com/pal1000/mesa-dist-win/releases/download/{MESA_VERSION}/mesa3d-{MESA_VERSION}-release-mingw.7z"), &archive, log)?;
        let tmp = dir.with_extension("tmp");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
        // bsdtar (the `tar` of Windows 10+, macOS and most Linux) reads 7z; GNU tar does not.
        let tar = tools::which("bsdtar").or_else(|| tools::which("tar")).ok_or("hace falta bsdtar o tar para abrir el paquete de Mesa")?;
        let names: Vec<String> = MESA_DLLS.iter().map(|f| format!("x64/{f}")).collect();
        run_logged(Command::new(tar).arg("-xf").arg(&archive).arg("-C").arg(&tmp).args(&names), log)
            .map_err(|e| format!("no se pudo abrir {} ({e}); instala bsdtar (libarchive)", archive.display()))?;
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::rename(tmp.join("x64"), &dir).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::remove_file(&archive);
    }
    MESA_DLLS.iter().map(|f| std::fs::read(dir.join(f)).map(|b| (f.to_string(), b)).map_err(|e| format!("{}: {e}", dir.join(f).display()))).collect()
}

fn unzip(zip: &Path, to: &Path, log: Log) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    match tools::which("unzip") {
        Some(u) if !cfg!(windows) => run_logged(Command::new(u).arg("-q").arg("-o").arg(zip).arg("-d").arg(to), log),
        _ => run_logged(Command::new("tar").arg("-xf").arg(zip).arg("-C").arg(to), log),
    }
}

fn rustup_targets(t: &Tools, targets: &[&str], log: Log) -> Result<(), String> {
    let missing: Vec<&str> = targets.iter().copied().filter(|x| !t.has_target(x)).collect();
    if missing.is_empty() {
        log(&format!("targets de Rust ya instalados: {}", targets.join(", ")));
        return Ok(());
    }
    let rustup = t.rustup.clone().ok_or("no se encuentra rustup (instala Rust con https://rustup.rs)")?;
    run_logged(Command::new(rustup).args(["target", "add"]).args(&missing), log)
}

fn setup_zig(t: &Tools, log: Log) -> Result<(), String> {
    let dir = tools::toolchains_dir();
    if t.zig.is_none() {
        let host = match (std::env::consts::OS, std::env::consts::ARCH) {
            ("linux", a) => format!("{a}-linux"),
            ("macos", a) => format!("{a}-macos"),
            ("windows", a) => format!("{a}-windows"),
            (o, a) => return Err(format!("no hay zig para {o}/{a}")),
        };
        let ext = if cfg!(windows) { "zip" } else { "tar.xz" };
        let archive = dir.join("downloads").join(format!("zig.{ext}"));
        download(&format!("https://ziglang.org/download/{ZIG_VERSION}/zig-{host}-{ZIG_VERSION}.{ext}"), &archive, log)?;
        let tmp = dir.join("zig.tmp");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
        if cfg!(windows) {
            unzip(&archive, &tmp, log)?;
        } else {
            run_logged(Command::new("tar").arg("-xJf").arg(&archive).arg("-C").arg(&tmp), log)?;
        }
        let inner = std::fs::read_dir(&tmp).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).find(|p| p.is_dir()).ok_or("el archivo de zig está vacío")?;
        let _ = std::fs::remove_dir_all(dir.join("zig"));
        std::fs::rename(inner, dir.join("zig")).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::remove_file(&archive);
        log(&format!("zig {ZIG_VERSION} en {} ({})", dir.join("zig").display(), human(tools::dir_size(&dir.join("zig")))));
    } else {
        log(&format!("zig ya está: {}", shown(&t.zig)));
    }
    if t.zigbuild.is_none() {
        let cargo = t.cargo.clone().ok_or("no se encuentra cargo")?;
        run_logged(Command::new(cargo).args(["install", "--locked", "cargo-zigbuild"]), log)?;
    } else {
        log(&format!("cargo-zigbuild ya está: {}", shown(&t.zigbuild)));
    }
    Ok(())
}

pub fn setup(target: Target, log: Log) -> Result<(), String> {
    let t = Tools::detect();
    log(&format!("PASO 1/2 Preparando la exportación a {}", target.name()));
    match target {
        Target::Linux | Target::Windows => {
            let triple = target.triple();
            if triple == tools::host_triple() {
                log("compila con el Rust de este equipo: no hay nada que instalar");
                if t.cargo.is_none() {
                    return Err("no se encuentra cargo (instala Rust: https://rustup.rs)".into());
                }
            } else {
                rustup_targets(&t, &[triple], log)?;
                setup_zig(&Tools::detect(), log)?;
            }
        }
        Target::Android => {
            if t.javac.is_none() {
                return Err("hace falta Java 17 o más nuevo (JDK con javac). Instálalo con el gestor de paquetes (p. ej. jdk-openjdk / openjdk-17-jdk) y vuelve a intentarlo.".into());
            }
            rustup_targets(&t, &["aarch64-linux-android", "armv7-linux-androideabi", "x86_64-linux-android"], log)?;
            let sdk = t.android_sdk.clone().unwrap_or_else(tools::default_android_sdk);
            if t.sdkmanager().is_none() {
                let os = if cfg!(windows) { "win" } else if cfg!(target_os = "macos") { "mac" } else { "linux" };
                let zip = tools::toolchains_dir().join("downloads/cmdline-tools.zip");
                download(&format!("https://dl.google.com/android/repository/commandlinetools-{os}-{CMDLINE_TOOLS}_latest.zip"), &zip, log)?;
                let tmp = sdk.join("cmdline-tools/tmp");
                let _ = std::fs::remove_dir_all(&tmp);
                unzip(&zip, &tmp, log)?;
                let _ = std::fs::remove_dir_all(sdk.join("cmdline-tools/latest"));
                std::fs::rename(tmp.join("cmdline-tools"), sdk.join("cmdline-tools/latest")).map_err(|e| e.to_string())?;
                let _ = std::fs::remove_dir_all(&tmp);
                let _ = std::fs::remove_file(&zip);
            }
            let t2 = Tools { android_sdk: Some(sdk.clone()), ..Tools::detect() };
            let sm = t2.sdkmanager().ok_or("no se pudo instalar sdkmanager")?;
            log("aceptando las licencias del SDK de Android");
            let mut lic = Command::new(&sm);
            lic.arg(format!("--sdk_root={}", sdk.display())).arg("--licenses").env("PATH", tools::child_path()).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null());
            if let Some(jh) = &t.java_home {
                lic.env("JAVA_HOME", jh);
            }
            if let Ok(mut ch) = lic.spawn() {
                if let Some(mut i) = ch.stdin.take() {
                    use std::io::Write;
                    let _ = i.write_all("y\n".repeat(30).as_bytes());
                }
                let _ = ch.wait();
            }
            let mut pkgs: Vec<String> = SDK_PACKAGES.iter().map(|s| s.to_string()).collect();
            if t2.ndk.is_none() {
                pkgs.push(format!("ndk;{NDK_VERSION}"));
            }
            log(&format!("instalando {} (el NDK son ~630 MB de descarga)", pkgs.join(", ")));
            let mut c = Command::new(&sm);
            c.arg(format!("--sdk_root={}", sdk.display())).args(&pkgs);
            if let Some(jh) = &t.java_home {
                c.env("JAVA_HOME", jh);
            }
            run_logged(&mut c, log)?;
            log(&format!("SDK de Android en {} ({})", sdk.display(), human(tools::dir_size(&sdk))));
        }
    }
    log("PASO 2/2 Comprobando");
    let st = doctor(&Tools::detect()).into_iter().find(|s| s.target == target).unwrap();
    for c in &st.checks {
        log(&format!("{} {}: {}", if c.ok { "✔" } else { "✘" }, c.name, c.detail));
    }
    if st.ready { Ok(()) } else { Err(format!("todavía falta algo para exportar a {}", target.name())) }
}

// ================================================================== export

/// Name usable as a file name on every system.
pub fn file_name(s: &str) -> String {
    let n: String = s.trim().chars().map(|c| if "/\\:*?\"<>|".contains(c) || c.is_control() { '_' } else { c }).collect();
    let n = n.trim_matches(|c| c == '.' || c == ' ').to_string();
    if n.is_empty() { "Game".into() } else { n }
}

/// The game's icon (512×512): the PNG of `[game] icon`, or the default one.
pub fn game_icon(dir: &Path, g: &GameToml) -> Result<image::RgbaImage, String> {
    if let Some(rel) = &g.game.icon {
        let p = dir.join(rel);
        let bytes = std::fs::read(&p).map_err(|e| format!("icono {}: {e}", p.display()))?;
        let img = image::load_from_memory(&bytes).map_err(|e| format!("icono {}: {e}", p.display()))?.to_rgba8();
        // keep pixel art crisp when scaling up
        let filter = if img.width() < 128 { image::imageops::FilterType::Nearest } else { image::imageops::FilterType::Lanczos3 };
        return Ok(image::imageops::resize(&img, 512, 512, filter));
    }
    Ok(default_icon())
}

/// A rounded purple square with a white "play" triangle.
pub fn default_icon() -> image::RgbaImage {
    let n = 512u32;
    image::RgbaImage::from_fn(n, n, |x, y| {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let r = 96.0;
        let (cx, cy) = (fx.clamp(r + 16.0, n as f32 - r - 16.0), fy.clamp(r + 16.0, n as f32 - r - 16.0));
        let d = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt() - r;
        if d > 0.0 {
            return image::Rgba([0, 0, 0, 0]);
        }
        let t = fy / n as f32;
        let bg = [(112.0 - 40.0 * t) as u8, (76.0 - 30.0 * t) as u8, (230.0 - 60.0 * t) as u8];
        // play triangle
        let (ax, ay, bx, by, qx, qy) = (190.0, 140.0, 190.0, 372.0, 384.0, 256.0);
        let s = |px: f32, py: f32, x1: f32, y1: f32, x2: f32, y2: f32| (px - x2) * (y1 - y2) - (x1 - x2) * (py - y2);
        let (d1, d2, d3) = (s(fx, fy, ax, ay, bx, by), s(fx, fy, bx, by, qx, qy), s(fx, fy, qx, qy, ax, ay));
        let inside = !((d1 < 0.0 || d2 < 0.0 || d3 < 0.0) && (d1 > 0.0 || d2 > 0.0 || d3 > 0.0));
        let alpha = ((-d).min(1.5) / 1.5 * 255.0) as u8;
        if inside { image::Rgba([255, 255, 255, alpha]) } else { image::Rgba([bg[0], bg[1], bg[2], alpha]) }
    })
}

fn png_bytes(img: &image::RgbaImage) -> Vec<u8> {
    let mut v = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut v), image::ImageFormat::Png).unwrap();
    v
}

/// Windows executables: 2 = no console window, 3 = console.
pub fn pe_subsystem(exe: &[u8]) -> Option<u16> {
    let pe = u32::from_le_bytes(exe.get(0x3c..0x40)?.try_into().ok()?) as usize;
    if exe.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let at = pe + 24 + 68;
    Some(u16::from_le_bytes(exe.get(at..at + 2)?.try_into().ok()?))
}

pub fn set_pe_subsystem(exe: &mut [u8], v: u16) -> bool {
    if pe_subsystem(exe).is_none() {
        return false;
    }
    let pe = u32::from_le_bytes(exe[0x3c..0x40].try_into().unwrap()) as usize;
    exe[pe + 24 + 68..pe + 24 + 70].copy_from_slice(&v.to_le_bytes());
    true
}

fn readme(g: &GameToml, target: Target, exe_name: &str) -> String {
    let run = match target {
        Target::Linux => format!("Ejecuta «{exe_name}» (doble clic o ./\"{exe_name}\" desde una terminal).\nRun \"{exe_name}\" (double click or ./\"{exe_name}\" from a terminal)."),
        _ => format!("Abre «{exe_name}».\nOpen \"{exe_name}\"."),
    };
    let saves = match target {
        Target::Linux => format!("~/.local/share/{}/saves", modengine::vfs::safe_id(&g.game.name)),
        _ => format!("%APPDATA%\\{}\\saves", modengine::vfs::safe_id(&g.game.name)),
    };
    format!(
        "{} {}\n\n{run}\n\nPartidas guardadas / saved games: {saves}\nF4: pantalla completa / fullscreen\n\nHecho con Mod Engine.\n",
        g.game.name, g.game.version
    )
}

fn desktop_entry(g: &GameToml, exe_name: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName={}\nComment={} {}\nExec=sh -c 'cd \"$(dirname \"$1\")\" && exec ./\"{}\"' sh %k\nIcon=applications-games\nTerminal=false\nCategories=Game;\n",
        g.game.name, g.game.name, g.game.version, exe_name
    )
}

fn write(p: &Path, b: &[u8]) -> Result<(), String> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    std::fs::write(p, b).map_err(|e| format!("{}: {e}", p.display()))
}

#[cfg(unix)]
fn make_executable(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755));
}
#[cfg(not(unix))]
fn make_executable(_: &Path) {}

pub fn export(a: &ExportArgs, log: Log) -> Result<PathBuf, String> {
    let target = a.target.ok_or("falta --target")?;
    let dir = &a.project;
    let total = 4;
    log(&format!("PASO 1/{total} Comprobando el proyecto"));
    let engine = modengine::Engine::load(dir).map_err(|e| format!("el proyecto no carga:\n{e}"))?;
    let g = engine.config.clone();
    drop(engine);
    let name = file_name(g.export.exe_name.as_deref().filter(|s| !s.trim().is_empty()).unwrap_or(&g.game.name));
    let out = match (&a.out, &g.export.out_dir) {
        (Some(o), _) => o.clone(),
        (None, Some(o)) if !o.trim().is_empty() => dir.join(o).join(target.name()),
        _ => dir.join("export").join(target.name()),
    };
    log(&format!("{} {} → {}", g.game.name, g.game.version, out.display()));

    log(&format!("PASO 2/{total} Empaquetando los datos del juego"));
    let pack_bytes = pack::pack_dir(dir)?;
    let files = pack::Pack::parse(pack_bytes.clone())?.len();
    log(&format!("game.dat: {files} ficheros, {}", human(pack_bytes.len() as u64)));
    let icon = game_icon(dir, &g)?;
    let t = Tools::detect();

    let result = match target {
        Target::Linux | Target::Windows => {
            log(&format!("PASO 3/{total} Preparando el runner para {}", target.name()));
            let runner = desktop_runner(target, &t, log)?;
            log(&format!("PASO 4/{total} Creando el ejecutable"));
            let mut exe_bytes = std::fs::read(&runner).map_err(|e| format!("{}: {e}", runner.display()))?;
            let windows = target == Target::Windows;
            if windows && !set_pe_subsystem(&mut exe_bytes, if g.export.windows_console { 3 } else { 2 }) {
                return Err(format!("{} no es un ejecutable de Windows", runner.display()));
            }
            let exe_name = if windows { format!("{name}.exe") } else { name.clone() };
            let game = pack::append_to(&exe_bytes, &pack_bytes);
            let folder = out.join(&name);
            let exe_path = folder.join(&exe_name);
            write(&exe_path, &game)?;
            make_executable(&exe_path);
            let readme_name = "README.txt";
            let readme = readme(&g, target, &exe_name);
            write(&folder.join(readme_name), readme.as_bytes())?;
            let icon_png = png_bytes(&icon);
            let mut entries: Vec<(String, Vec<u8>, u32)> = vec![(format!("{name}/{exe_name}"), game, 0o755), (format!("{name}/{readme_name}"), readme.into_bytes(), 0o644)];
            if windows {
                // Software OpenGL next to the .exe, or none: a stale copy from an earlier export
                // would silently keep the game on the slow path.
                for f in MESA_DLLS {
                    let _ = std::fs::remove_file(folder.join(f));
                }
                if g.export.windows_software_gl || a.software_gl {
                    log("OpenGL por software (Mesa llvmpipe) junto al ejecutable");
                    for (f, bytes) in mesa_dlls(log)? {
                        write(&folder.join(&f), &bytes)?;
                        entries.push((format!("{name}/{f}"), bytes, 0o644));
                    }
                }
                let zip: Vec<archive::ZipEntry> = entries.iter().map(|(n, d, m)| archive::ZipEntry::new(n, d, true, *m)).collect();
                let zp = out.join(format!("{name}-windows-x86_64.zip"));
                write(&zp, &archive::write_zip(&zip))?;
                log(&format!("{} ({})", zp.display(), human(std::fs::metadata(&zp).map(|m| m.len()).unwrap_or(0))));
            } else {
                let desktop = desktop_entry(&g, &exe_name);
                write(&folder.join(format!("{name}.desktop")), desktop.as_bytes())?;
                make_executable(&folder.join(format!("{name}.desktop")));
                write(&folder.join("icon.png"), &icon_png)?;
                entries.push((format!("{name}/{name}.desktop"), desktop.into_bytes(), 0o755));
                entries.push((format!("{name}/icon.png"), icon_png, 0o644));
                let tp = out.join(format!("{name}-linux-{}.tar.gz", std::env::consts::ARCH));
                write(&tp, &archive::write_tar_gz(&entries))?;
                log(&format!("{} ({})", tp.display(), human(std::fs::metadata(&tp).map(|m| m.len()).unwrap_or(0))));
            }
            log(&format!("{} ({})", exe_path.display(), human(std::fs::metadata(&exe_path).map(|m| m.len()).unwrap_or(0))));
            folder
        }
        Target::Android => {
            let abis: Vec<String> = if !a.arches.is_empty() {
                a.arches.clone()
            } else if !g.android.abis.is_empty() {
                g.android.abis.iter().filter_map(|x| normalize_abi(x)).collect()
            } else {
                vec!["arm64-v8a".into()]
            };
            let signing = android::signing(a.release, a.keystore.as_deref(), a.key_alias.as_deref(), &|k| std::env::var(k).ok())?;
            log(&format!("PASO 3/{total} Preparando el motor para Android ({})", abis.join(", ")));
            let mut libs = vec![];
            for abi in &abis {
                libs.push((abi.clone(), android_lib(abi, &t, log)?));
            }
            log(&format!("PASO 4/{total} Creando el APK"));
            let apk = out.join(format!("{name}.apk"));
            let work = tools::cache_dir().join("apk").join(g.android_package());
            // multiplayer games need the INTERNET permission (game.toml [android] internet overrides)
            let internet = g.android.internet.unwrap_or_else(|| {
                pack::Pack::parse(pack_bytes.clone())
                    .map(|p| p.files().any(|(n, d)| (n.ends_with(".ms") || n.ends_with(".toml")) && modengine::net::source_uses_network(&String::from_utf8_lossy(d))))
                    .unwrap_or(false)
            });
            if internet {
                log("permiso INTERNET (el juego usa la red)");
            }
            let apk = android::build_apk(&t, android::ApkInput { game: &g, pack: &pack_bytes, libs, icon, signing, internet, work, out: apk }, log)?;
            log(&format!("{} ({})", apk.display(), human(std::fs::metadata(&apk).map(|m| m.len()).unwrap_or(0))));
            if a.install {
                let adb = t.adb().ok_or("no se encuentra adb (platform-tools)")?;
                run_logged(Command::new(adb).args(["install", "-r"]).arg(&apk), log)?;
            }
            apk
        }
    };
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn export_arguments() {
        let ExportCmd::Export(a) = parse_args(&s(&["games/plat", "--target", "windows", "--out", "/tmp/o"])).unwrap() else { panic!() };
        assert_eq!((a.project, a.target, a.out, a.release), (PathBuf::from("games/plat"), Some(Target::Windows), Some(PathBuf::from("/tmp/o")), false));
        let ExportCmd::Export(a) = parse_args(&s(&["p", "-t", "apk", "--release", "--arch", "arm64,armv7,arm64-v8a", "--install"])).unwrap() else { panic!() };
        assert_eq!((a.target, a.release, a.install), (Some(Target::Android), true, true));
        assert_eq!(a.arches, vec!["arm64-v8a", "armeabi-v7a"]);
        let ExportCmd::Export(a) = parse_args(&s(&["p", "-t", "windows", "--software-gl"])).unwrap() else { panic!() };
        assert!(a.software_gl);
        assert_eq!(parse_args(&s(&["--setup", "android"])).unwrap(), ExportCmd::Setup(Target::Android));
        assert_eq!(parse_args(&s(&["--doctor", "--json"])).unwrap(), ExportCmd::Doctor { json: true });
        assert!(parse_args(&s(&["p"])).unwrap_err().contains("--target"));
        assert!(parse_args(&s(&["--target", "linux"])).unwrap_err().contains("project"));
        assert!(parse_args(&s(&["p", "--target", "mac"])).unwrap_err().contains("unknown target"));
        assert!(parse_args(&s(&["p", "--target", "android", "--arch", "mips"])).is_err());
        assert!(parse_args(&s(&["p", "--bogus"])).is_err());
    }

    #[test]
    fn names_icons_and_pe_patching() {
        assert_eq!(file_name("Coin: Climber?"), "Coin_ Climber_");
        assert_eq!(file_name("  ..  "), "Game");
        let i = default_icon();
        assert_eq!(i.dimensions(), (512, 512));
        assert_eq!(i.get_pixel(0, 0)[3], 0, "transparent corner");
        assert_eq!(i.get_pixel(256, 256).0, [255, 255, 255, 255], "white triangle");
        // a tiny fake PE: "MZ", e_lfanew = 0x40, "PE\0\0", subsystem at 0x40 + 24 + 68
        let mut pe = vec![0u8; 0x40 + 24 + 70];
        pe[0..2].copy_from_slice(b"MZ");
        pe[0x3c] = 0x40;
        pe[0x40..0x44].copy_from_slice(b"PE\0\0");
        pe[0x40 + 24 + 68] = 3;
        assert_eq!(pe_subsystem(&pe), Some(3));
        assert!(set_pe_subsystem(&mut pe, 2));
        assert_eq!(pe_subsystem(&pe), Some(2));
        assert!(!set_pe_subsystem(&mut [0u8; 10], 2));
    }

    #[test]
    fn exports_and_runner_triples() {
        assert_eq!(Target::Android.triple(), "aarch64-linux-android");
        if cfg!(target_os = "linux") {
            assert_eq!(Target::Windows.triple(), "x86_64-pc-windows-gnu");
            assert_eq!(Target::Linux.triple(), tools::host_triple());
        }
        let g: GameToml = toml::from_str("[game]\nname = \"Coin Climber\"\nversion = \"2.0\"\n").unwrap();
        let d = desktop_entry(&g, "Coin Climber");
        assert!(d.contains("Name=Coin Climber") && d.contains("exec ./\"Coin Climber\"") && d.contains("Categories=Game;"));
        assert!(readme(&g, Target::Windows, "Coin Climber.exe").contains("%APPDATA%\\coin_climber\\saves"));
    }

    #[test]
    fn source_hash_is_stable() {
        if let Some(src) = find_source() {
            assert_eq!(source_hash(&src), source_hash(&src));
            assert_eq!(source_hash(&src).len(), 12);
        }
    }
}
