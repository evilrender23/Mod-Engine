//! Finding (and installing, `--setup`) what each export target needs: Rust targets, zig +
//! cargo-zigbuild (Windows from Linux/Mac), the Android SDK/NDK and Java.

use std::path::{Path, PathBuf};
use std::process::Command;

pub fn exe(name: &str) -> String {
    if cfg!(windows) { format!("{name}.exe") } else { name.to_string() }
}

fn home() -> PathBuf {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}

fn env_dir(k: &str) -> Option<PathBuf> {
    std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// `~/.cache/modengine` (runners, build folder, APK work folder).
pub fn cache_dir() -> PathBuf {
    if let Some(d) = env_dir("MODENGINE_CACHE") {
        return d;
    }
    if cfg!(windows) {
        return env_dir("LOCALAPPDATA").unwrap_or_else(home).join("modengine").join("cache");
    }
    env_dir("XDG_CACHE_HOME").unwrap_or_else(|| home().join(".cache")).join("modengine")
}

/// `~/.config/modengine` (the Android debug keystore).
pub fn config_dir() -> PathBuf {
    if cfg!(windows) {
        return env_dir("APPDATA").unwrap_or_else(home).join("modengine");
    }
    env_dir("XDG_CONFIG_HOME").unwrap_or_else(|| home().join(".config")).join("modengine")
}

/// `~/.local/share/modengine-toolchains` (zig, downloads, logs).
pub fn toolchains_dir() -> PathBuf {
    if let Some(d) = env_dir("MODENGINE_TOOLCHAINS") {
        return d;
    }
    if cfg!(windows) {
        return env_dir("LOCALAPPDATA").unwrap_or_else(home).join("modengine-toolchains");
    }
    env_dir("XDG_DATA_HOME").unwrap_or_else(|| home().join(".local/share")).join("modengine-toolchains")
}

pub fn default_android_sdk() -> PathBuf {
    if cfg!(windows) {
        env_dir("LOCALAPPDATA").unwrap_or_else(home).join("Android").join("Sdk")
    } else if cfg!(target_os = "macos") {
        home().join("Library/Android/sdk")
    } else {
        home().join("Android/Sdk")
    }
}

/// Searches PATH (plus ~/.cargo/bin, which GUI sessions often lack).
pub fn which(name: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.push(home().join(".cargo/bin"));
    let names: Vec<String> = if cfg!(windows) { vec![format!("{name}.exe"), format!("{name}.cmd"), format!("{name}.bat")] } else { vec![name.to_string()] };
    dirs.iter().flat_map(|d| names.iter().map(move |n| d.join(n))).find(|p| p.is_file())
}

/// PATH for child processes: ~/.cargo/bin and our zig first.
pub fn child_path() -> std::ffi::OsString {
    let mut dirs: Vec<PathBuf> = vec![home().join(".cargo/bin"), toolchains_dir().join("zig")];
    if let Some(p) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&p));
    }
    std::env::join_paths(dirs).unwrap_or_default()
}

/// The host as Rust names it.
pub fn host_triple() -> &'static str {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "aarch64-unknown-linux-gnu"
    } else if cfg!(all(windows, target_env = "msvc")) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(windows) {
        "x86_64-pc-windows-gnu"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else {
        "x86_64-apple-darwin"
    }
}

/// Everything found on this machine.
#[derive(Debug, Default, Clone)]
pub struct Tools {
    pub cargo: Option<PathBuf>,
    pub rustup: Option<PathBuf>,
    pub rust_targets: Vec<String>,
    pub zig: Option<PathBuf>,
    pub zigbuild: Option<PathBuf>,
    pub android_sdk: Option<PathBuf>,
    pub ndk: Option<PathBuf>,
    pub java_home: Option<PathBuf>,
    pub javac: Option<PathBuf>,
}

impl Tools {
    pub fn detect() -> Tools {
        let rustup = which("rustup");
        let rust_targets = rustup
            .as_ref()
            .and_then(|r| Command::new(r).args(["target", "list", "--installed"]).output().ok())
            .map(|o| String::from_utf8_lossy(&o.stdout).lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
            .unwrap_or_default();
        let zig_local = toolchains_dir().join("zig").join(exe("zig"));
        let zig = if zig_local.is_file() { Some(zig_local) } else { which("zig") };
        let sdk = env_dir("ANDROID_HOME").or_else(|| env_dir("ANDROID_SDK_ROOT")).or_else(|| Some(default_android_sdk())).filter(|p| p.is_dir());
        let ndk = env_dir("ANDROID_NDK_HOME").or_else(|| env_dir("ANDROID_NDK_ROOT")).filter(|p| p.is_dir()).or_else(|| {
            let d = sdk.as_ref()?.join("ndk");
            let mut v: Vec<PathBuf> = std::fs::read_dir(d).ok()?.flatten().map(|e| e.path()).filter(|p| p.join("toolchains/llvm/prebuilt").is_dir()).collect();
            v.sort_by_key(|p| version_key(&p.file_name().unwrap_or_default().to_string_lossy()));
            v.pop()
        });
        let java_home = env_dir("JAVA_HOME").filter(|p| p.join("bin").join(exe("javac")).is_file());
        let javac = java_home.as_ref().map(|h| h.join("bin").join(exe("javac"))).or_else(|| which("javac"));
        Tools { cargo: which("cargo"), rustup, rust_targets, zig, zigbuild: which("cargo-zigbuild"), android_sdk: sdk, ndk, java_home, javac }
    }

    pub fn has_target(&self, t: &str) -> bool {
        self.rust_targets.iter().any(|x| x == t)
    }

    pub fn java_tool(&self, name: &str) -> Option<PathBuf> {
        let from_home = self.java_home.as_ref().map(|h| h.join("bin").join(exe(name))).filter(|p| p.is_file());
        from_home.or_else(|| self.javac.as_ref().and_then(|j| j.parent()).map(|d| d.join(exe(name))).filter(|p| p.is_file())).or_else(|| which(name))
    }

    /// Newest `build-tools/<v>` with aapt2.
    pub fn build_tools(&self) -> Option<PathBuf> {
        let d = self.android_sdk.as_ref()?.join("build-tools");
        let mut v: Vec<PathBuf> = std::fs::read_dir(d).ok()?.flatten().map(|e| e.path()).filter(|p| p.join(exe("aapt2")).is_file()).collect();
        v.sort_by_key(|p| version_key(&p.file_name().unwrap_or_default().to_string_lossy()));
        v.pop()
    }

    /// Newest `platforms/android-N/android.jar`.
    pub fn android_jar(&self) -> Option<PathBuf> {
        let d = self.android_sdk.as_ref()?.join("platforms");
        let mut v: Vec<PathBuf> = std::fs::read_dir(d).ok()?.flatten().map(|e| e.path().join("android.jar")).filter(|p| p.is_file()).collect();
        v.sort_by_key(|p| version_key(&p.parent().unwrap().file_name().unwrap_or_default().to_string_lossy()));
        v.pop()
    }

    pub fn sdkmanager(&self) -> Option<PathBuf> {
        let p = self.android_sdk.as_ref().unwrap_or(&default_android_sdk()).join("cmdline-tools/latest/bin").join(if cfg!(windows) { "sdkmanager.bat" } else { "sdkmanager" });
        p.is_file().then_some(p)
    }

    pub fn adb(&self) -> Option<PathBuf> {
        self.android_sdk.as_ref().map(|s| s.join("platform-tools").join(exe("adb"))).filter(|p| p.is_file()).or_else(|| which("adb"))
    }

    /// NDK clang for an ABI, e.g. `aarch64-linux-android21-clang`.
    pub fn ndk_clang(&self, clang_prefix: &str, api: u32) -> Option<PathBuf> {
        let host = if cfg!(windows) { "windows-x86_64" } else if cfg!(target_os = "macos") { "darwin-x86_64" } else { "linux-x86_64" };
        let bin = self.ndk.as_ref()?.join("toolchains/llvm/prebuilt").join(host).join("bin");
        let name = format!("{clang_prefix}{api}-clang{}", if cfg!(windows) { ".cmd" } else { "" });
        let p = bin.join(name);
        p.is_file().then_some(p)
    }

    pub fn ndk_tool(&self, name: &str) -> Option<PathBuf> {
        let host = if cfg!(windows) { "windows-x86_64" } else if cfg!(target_os = "macos") { "darwin-x86_64" } else { "linux-x86_64" };
        let p = self.ndk.as_ref()?.join("toolchains/llvm/prebuilt").join(host).join("bin").join(exe(name));
        p.is_file().then_some(p)
    }
}

/// "27.3.13750724" / "android-35" / "35.0.0-rc1" → sortable numbers.
pub fn version_key(s: &str) -> Vec<u64> {
    s.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).map(|p| p.parse().unwrap_or(0)).collect()
}

/// Folder size in bytes (for the setup report).
pub fn dir_size(p: &Path) -> u64 {
    let Ok(m) = std::fs::symlink_metadata(p) else { return 0 };
    if m.is_file() {
        return m.len();
    }
    if !m.is_dir() {
        return 0;
    }
    std::fs::read_dir(p).map(|rd| rd.flatten().map(|e| dir_size(&e.path())).sum()).unwrap_or(0)
}

pub fn human(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 30 => format!("{:.1} GB", b as f64 / (1u64 << 30) as f64),
        b if b >= 1 << 20 => format!("{:.0} MB", b as f64 / (1u64 << 20) as f64),
        b => format!("{} KB", b.div_ceil(1024)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_sort_numerically() {
        let mut v = vec!["9.0.0", "27.3.13750724", "35.0.0", "android-9", "android-35"];
        v.sort_by_key(|s| version_key(s));
        assert_eq!(v, vec!["android-9", "9.0.0", "27.3.13750724", "android-35", "35.0.0"]);
        assert_eq!(human(3 << 20), "3 MB");
        assert_eq!(human(2_700_000_000), "2.5 GB");
    }
}
