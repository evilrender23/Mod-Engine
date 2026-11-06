//! APK without Gradle: aapt2 (manifest + icons + assets) → javac + d8 (miniquad's activity)
//! → our zip writer (adds classes.dex and the native libraries) → zipalign → apksigner.

use super::archive::{read_zip, write_zip, ZipEntry};
use super::{run_logged, tools::Tools, Log};
use modengine::project::GameToml;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const MIN_SDK: u32 = 21;
pub const TARGET_SDK: u32 = 35;
pub const LIB_NAME: &str = "modengine_cli";

/// Android ABI → Rust target and NDK clang prefix.
pub fn abi_info(abi: &str) -> Option<(&'static str, &'static str)> {
    match abi {
        "arm64-v8a" => Some(("aarch64-linux-android", "aarch64-linux-android")),
        "armeabi-v7a" => Some(("armv7-linux-androideabi", "armv7a-linux-androideabi")),
        "x86_64" => Some(("x86_64-linux-android", "x86_64-linux-android")),
        "x86" => Some(("i686-linux-android", "i686-linux-android")),
        _ => None,
    }
}

pub const ABIS: &[&str] = &["arm64-v8a", "armeabi-v7a", "x86_64", "x86"];

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// Is this a valid Android application id (`com.example.game`)?
pub fn valid_package(p: &str) -> bool {
    let parts: Vec<&str> = p.split('.').collect();
    parts.len() >= 2
        && parts.iter().all(|s| {
            let mut c = s.chars();
            c.next().is_some_and(|f| f.is_ascii_alphabetic()) && c.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        })
}

pub fn orientation_attr(o: &str) -> &'static str {
    match o {
        "portrait" => "sensorPortrait",
        "sensor" | "any" => "fullSensor",
        _ => "sensorLandscape",
    }
}

/// `debuggable`: debug-signed APKs can be inspected with `adb shell run-as <package>`.
/// `internet`: the game uses networking (multiplayer), so it needs the INTERNET permission.
pub fn manifest(g: &GameToml, debuggable: bool, internet: bool) -> String {
    let pkg = g.android_package();
    let version = if g.game.version.trim().is_empty() { "1.0" } else { g.game.version.trim() };
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android"
    package="{pkg}"
    android:versionCode="{code}"
    android:versionName="{vname}">
    <uses-sdk android:minSdkVersion="{MIN_SDK}" android:targetSdkVersion="{TARGET_SDK}" />
    <uses-feature android:glEsVersion="0x00020000" android:required="true" />{perms}
    <application
        android:label="{label}"
        android:icon="@mipmap/ic_launcher"
        android:hasCode="true"
        android:extractNativeLibs="false"
        android:allowBackup="true"{debug}
        android:theme="@android:style/Theme.Black.NoTitleBar.Fullscreen">
        <activity
            android:name=".MainActivity"
            android:exported="true"
            android:launchMode="singleTask"
            android:screenOrientation="{orient}"
            android:configChanges="orientation|keyboardHidden|keyboard|screenSize|screenLayout|smallestScreenSize|uiMode|density|navigation|fontScale|layoutDirection|locale|touchscreen|colorMode|mcc|mnc">
            <intent-filter>
                <action android:name="android.intent.action.MAIN" />
                <category android:name="android.intent.category.LAUNCHER" />
            </intent-filter>
        </activity>
    </application>
</manifest>
"#,
        code = g.android.version_code.max(1),
        vname = xml_escape(version),
        label = xml_escape(&g.game.name),
        orient = orientation_attr(&g.android.orientation),
        perms = if internet {
            "\n    <uses-permission android:name=\"android.permission.INTERNET\" />\n    <uses-permission android:name=\"android.permission.ACCESS_NETWORK_STATE\" />"
        } else {
            ""
        },
        debug = if debuggable { "\n        android:debuggable=\"true\"" } else { "" },
    )
}

/// miniquad's Java activity for this package.
pub fn java_sources(pkg: &str) -> [(String, String); 2] {
    let fix = |s: &str| s.replace("TARGET_PACKAGE_NAME", pkg).replace("LIBRARY_NAME", LIB_NAME);
    [
        (format!("{}/MainActivity.java", pkg.replace('.', "/")), fix(include_str!("../../android/MainActivity.java"))),
        ("quad_native/QuadNative.java".into(), fix(include_str!("../../android/QuadNative.java"))),
    ]
}

/// Launcher icon sizes per density.
pub const MIPMAPS: &[(&str, u32)] = &[("mdpi", 48), ("hdpi", 72), ("xhdpi", 96), ("xxhdpi", 144), ("xxxhdpi", 192)];

/// Signing: the auto-generated debug keystore, or a release one from the environment
/// (`MODENGINE_KEYSTORE`, `MODENGINE_KEY_ALIAS`, `MODENGINE_KEYSTORE_PASS`, optional
/// `MODENGINE_KEY_PASS`). Passwords never go into the project.
#[derive(Debug)]
pub enum Signing {
    Debug(PathBuf),
    Release { keystore: PathBuf, alias: String },
}

pub fn signing(release: bool, keystore: Option<&Path>, alias: Option<&str>, env: &dyn Fn(&str) -> Option<String>) -> Result<Signing, String> {
    if !release {
        return Ok(Signing::Debug(super::tools::config_dir().join("android/debug.keystore")));
    }
    let ks = keystore.map(Path::to_path_buf).or_else(|| env("MODENGINE_KEYSTORE").map(PathBuf::from)).ok_or(
        "firmar en modo release necesita un almacén de claves: --keystore FICHERO.jks o la variable MODENGINE_KEYSTORE",
    )?;
    let alias = alias.map(str::to_string).or_else(|| env("MODENGINE_KEY_ALIAS")).ok_or("falta el alias de la clave: --key-alias A o MODENGINE_KEY_ALIAS")?;
    if env("MODENGINE_KEYSTORE_PASS").is_none() {
        return Err("falta la contraseña del almacén: define la variable de entorno MODENGINE_KEYSTORE_PASS (nunca se guarda en el proyecto)".into());
    }
    Ok(Signing::Release { keystore: ks, alias })
}

fn ensure_debug_keystore(t: &Tools, ks: &Path, log: Log) -> Result<(), String> {
    if ks.is_file() {
        return Ok(());
    }
    std::fs::create_dir_all(ks.parent().unwrap()).map_err(|e| e.to_string())?;
    log(&format!("creando el almacén de claves de depuración {}", ks.display()));
    let keytool = t.java_tool("keytool").ok_or("no se encuentra keytool (viene con Java)")?;
    let mut c = Command::new(keytool);
    c.args(["-genkeypair", "-noprompt", "-keystore"]).arg(ks);
    c.args(["-storepass", "android", "-keypass", "android", "-alias", "androiddebugkey", "-keyalg", "RSA", "-keysize", "2048", "-validity", "10000"]);
    c.args(["-dname", "CN=Android Debug,O=Android,C=US"]);
    run_logged(&mut c, log)
}

pub struct ApkInput<'a> {
    pub game: &'a GameToml,
    pub pack: &'a [u8],
    /// (abi, path of libmodengine_cli.so)
    pub libs: Vec<(String, PathBuf)>,
    /// 512×512 RGBA icon.
    pub icon: image::RgbaImage,
    pub signing: Signing,
    /// Add the INTERNET permission (the game uses networking).
    pub internet: bool,
    pub work: PathBuf,
    pub out: PathBuf,
}

pub fn build_apk(t: &Tools, a: ApkInput, log: Log) -> Result<PathBuf, String> {
    let sdk = t.android_sdk.clone().ok_or("no se encuentra el SDK de Android (modengine export --setup android)")?;
    let bt = t.build_tools().ok_or("faltan las build-tools del SDK de Android (modengine export --setup android)")?;
    let platform = t.android_jar().ok_or("falta una plataforma del SDK (platforms;android-XX)")?;
    let tool = |n: &str| {
        let p = bt.join(if cfg!(windows) && (n == "d8" || n == "apksigner") { format!("{n}.bat") } else { super::tools::exe(n) });
        if p.is_file() { Ok(p) } else { Err(format!("falta {} en {}", n, bt.display())) }
    };
    let pkg = a.game.android_package();
    if !valid_package(&pkg) {
        return Err(format!("'{pkg}' no es un identificador de paquete válido (ejemplo: com.miestudio.mijuego)"));
    }
    let w = &a.work;
    let _ = std::fs::remove_dir_all(w);
    for d in ["res", "assets", "java", "classes", "dex"] {
        std::fs::create_dir_all(w.join(d)).map_err(|e| e.to_string())?;
    }
    let write = |p: PathBuf, b: &[u8]| -> Result<(), String> {
        std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&p, b).map_err(|e| format!("{}: {e}", p.display()))
    };
    log(&format!("APK {pkg} (SDK {}, build-tools {})", sdk.display(), bt.file_name().unwrap_or_default().to_string_lossy()));

    // ---- resources: manifest, launcher icons, assets/game.dat
    write(w.join("AndroidManifest.xml"), manifest(a.game, matches!(a.signing, Signing::Debug(_)), a.internet).as_bytes())?;
    for (dpi, n) in MIPMAPS {
        let img = image::imageops::resize(&a.icon, *n, *n, image::imageops::FilterType::Lanczos3);
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| e.to_string())?;
        write(w.join(format!("res/mipmap-{dpi}/ic_launcher.png")), &png)?;
    }
    write(w.join("assets").join(crate::runner::PACK_FILE), a.pack)?;
    let aapt2 = tool("aapt2")?;
    run_logged(Command::new(&aapt2).args(["compile", "--dir"]).arg(w.join("res")).arg("-o").arg(w.join("res.zip")), log)?;
    let mut link = Command::new(&aapt2);
    link.arg("link").arg("-o").arg(w.join("base.apk")).arg("-I").arg(&platform);
    link.arg("--manifest").arg(w.join("AndroidManifest.xml")).arg("-A").arg(w.join("assets")).arg(w.join("res.zip"));
    link.args(["--auto-add-overlay", "--min-sdk-version", &MIN_SDK.to_string(), "--target-sdk-version", &TARGET_SDK.to_string()]);
    run_logged(&mut link, log)?;

    // ---- code: the activity
    let mut javac = Command::new(t.java_tool("javac").ok_or("no se encuentra javac (instala un JDK 17 o más nuevo)")?);
    javac.args(["-nowarn", "-Xlint:-options", "-source", "8", "-target", "8", "-encoding", "UTF-8", "-classpath"]).arg(&platform).arg("-d").arg(w.join("classes"));
    for (rel, text) in java_sources(&pkg) {
        let p = w.join("java").join(rel);
        write(p.clone(), text.as_bytes())?;
        javac.arg(p);
    }
    run_logged(&mut javac, log)?;
    let mut classes = vec![];
    collect(&w.join("classes"), &mut classes);
    let mut d8 = Command::new(tool("d8")?);
    d8.args(["--release", "--min-api", &MIN_SDK.to_string(), "--lib"]).arg(&platform).arg("--output").arg(w.join("dex")).args(&classes);
    run_logged(&mut d8, log)?;

    // ---- put it together
    let base = std::fs::read(w.join("base.apk")).map_err(|e| e.to_string())?;
    let mut entries = read_zip(&base)?;
    let dex = std::fs::read(w.join("dex/classes.dex")).map_err(|e| format!("d8 no generó classes.dex: {e}"))?;
    entries.push(ZipEntry::new("classes.dex", &dex, true, 0));
    for (abi, so) in &a.libs {
        let bytes = std::fs::read(so).map_err(|e| format!("{}: {e}", so.display()))?;
        // stored, so Android maps it straight from the APK (extractNativeLibs=false)
        entries.push(ZipEntry::new(&format!("lib/{abi}/lib{LIB_NAME}.so"), &bytes, false, 0));
    }
    std::fs::write(w.join("unaligned.apk"), write_zip(&entries)).map_err(|e| e.to_string())?;
    run_logged(Command::new(tool("zipalign")?).args(["-P", "16", "-f", "4"]).arg(w.join("unaligned.apk")).arg(w.join("aligned.apk")), log)?;

    // ---- sign
    std::fs::create_dir_all(a.out.parent().unwrap()).map_err(|e| e.to_string())?;
    let mut sign = Command::new(tool("apksigner")?);
    sign.arg("sign");
    match &a.signing {
        Signing::Debug(ks) => {
            ensure_debug_keystore(t, ks, log)?;
            sign.arg("--ks").arg(ks).args(["--ks-pass", "pass:android", "--key-pass", "pass:android", "--ks-key-alias", "androiddebugkey"]);
            log("firmado con la clave de depuración (para probar; usa --release para publicar)");
        }
        Signing::Release { keystore, alias } => {
            sign.arg("--ks").arg(keystore).args(["--ks-key-alias", alias, "--ks-pass", "env:MODENGINE_KEYSTORE_PASS"]);
            if std::env::var_os("MODENGINE_KEY_PASS").is_some() {
                sign.args(["--key-pass", "env:MODENGINE_KEY_PASS"]);
            }
            log(&format!("firmado con {} ({alias})", keystore.display()));
        }
    }
    if let Some(jh) = &t.java_home {
        sign.env("JAVA_HOME", jh);
    }
    sign.arg("--out").arg(&a.out).arg(w.join("aligned.apk"));
    run_logged(&mut sign, log)?;
    run_logged(Command::new(tool("apksigner")?).args(["verify", "--min-sdk-version", &MIN_SDK.to_string()]).arg(&a.out), log)?;
    let _ = std::fs::remove_file(a.out.with_extension("apk.idsig"));
    Ok(a.out)
}

fn collect(d: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(d) {
        let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        v.sort();
        for p in v {
            if p.is_dir() {
                collect(&p, out);
            } else if p.extension().is_some_and(|e| e == "class") {
                out.push(p);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_from_game_toml() {
        let mut g: GameToml = toml::from_str("[game]\nname = \"Coin <Climber> & co\"\nversion = \"1.2.3\"\n[android]\norientation = \"portrait\"\nversion_code = 7\n").unwrap();
        let m = manifest(&g, true, false);
        assert!(m.contains("package=\"com.modengine.coin_climber_co\""), "{m}");
        assert!(!m.contains("android.permission.INTERNET"), "no network, no permission");
        assert!(m.contains("android:debuggable=\"true\""));
        assert!(m.contains("android:versionCode=\"7\"") && m.contains("android:versionName=\"1.2.3\""));
        assert!(m.contains("android:label=\"Coin &lt;Climber&gt; &amp; co\""));
        assert!(m.contains("android:screenOrientation=\"sensorPortrait\""));
        assert!(m.contains("android:name=\".MainActivity\"") && m.contains("android.intent.category.LAUNCHER"));
        assert!(m.contains(&format!("android:targetSdkVersion=\"{TARGET_SDK}\"")));
        g.android.package = Some("org.ana.juego".into());
        g.android.orientation = "landscape".into();
        let m = manifest(&g, false, true);
        assert!(m.contains("package=\"org.ana.juego\"") && m.contains("sensorLandscape"));
        assert!(m.contains("<uses-permission android:name=\"android.permission.INTERNET\" />"), "{m}");
        assert!(!m.contains("debuggable"), "release APKs are not debuggable");
    }

    #[test]
    fn package_ids_and_java() {
        assert!(valid_package("com.modengine.coin_climber") && valid_package("org.a1.b"));
        assert!(!valid_package("juego") && !valid_package("com.1abc.x") && !valid_package("com..x") && !valid_package("com.a-b.c"));
        let g: GameToml = toml::from_str("[game]\nname = \"123\"\n").unwrap();
        assert!(valid_package(&g.android_package()), "{}", g.android_package());
        let [(p, act), (q, native)] = java_sources("org.ana.juego");
        assert_eq!(p, "org/ana/juego/MainActivity.java");
        assert!(act.contains("package org.ana.juego;") && act.contains("System.loadLibrary(\"modengine_cli\")"));
        assert_eq!(q, "quad_native/QuadNative.java");
        assert!(native.contains("public native static void surfaceOnTouch"));
    }

    #[test]
    fn release_signing_needs_env_not_project_secrets() {
        let none = |_: &str| None;
        assert!(matches!(signing(false, None, None, &none), Ok(Signing::Debug(p)) if p.ends_with("android/debug.keystore")));
        assert!(signing(true, None, None, &none).unwrap_err().contains("MODENGINE_KEYSTORE"));
        let some = |k: &str| match k {
            "MODENGINE_KEYSTORE" => Some("/k/rel.jks".to_string()),
            "MODENGINE_KEY_ALIAS" => Some("juego".to_string()),
            _ => None,
        };
        assert!(signing(true, None, None, &some).unwrap_err().contains("MODENGINE_KEYSTORE_PASS"));
        let all = |k: &str| some(k).or_else(|| (k == "MODENGINE_KEYSTORE_PASS").then(|| "x".to_string()));
        assert!(matches!(signing(true, None, None, &all), Ok(Signing::Release { alias, .. }) if alias == "juego"));
    }
}
