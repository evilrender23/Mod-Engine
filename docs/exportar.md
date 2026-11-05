# Exportar el juego (Windows, Linux, Android)

Como «Create Executable» de GameMaker: convierte el proyecto en un juego que se juega **sin Mod
Engine instalado**.

- Editor: **Archivo → 📦 Exportar…** (o el botón **📦 Exportar** de la barra).
- Terminal: `modengine export <proyecto> --target windows|linux|android`.

| Plataforma | Qué sale | Dónde (por defecto) |
|---|---|---|
| Windows (64 bits) | `<Juego>/<Juego>.exe` + `README.txt`, y `<Juego>-windows-x86_64.zip` | `<proyecto>/export/windows/` |
| Linux (x86_64) | `<Juego>/<Juego>` + `.desktop` + `icon.png` + `README.txt`, y `<Juego>-linux-x86_64.tar.gz` | `<proyecto>/export/linux/` |
| Android | `<Juego>.apk` (arm64-v8a; opcionalmente armeabi-v7a y x86_64) | `<proyecto>/export/android/` |

La carpeta `export/` no entra nunca en el juego exportado (tampoco `notes/`, `saves/` ni las carpetas
ocultas como `.modengine/`).

## Cómo funciona

Un juego exportado es un **runner** (el motor sin editor ni línea de comandos) más el proyecto
empaquetado en un único fichero, `game.dat`:

- En Windows y Linux, `game.dat` va **pegado al final del ejecutable** (un solo fichero). El runner
  también acepta un `game.dat` (o `<Juego>.dat`) a su lado.
- En Android, va dentro del APK como `assets/game.dat`.

`game.dat` (formato en `crates/modengine/src/pack.rs`) tiene cabecera con versión, índice de
ficheros y suma de comprobación; el mismo proyecto da siempre el mismo fichero. El motor lee el
proyecto igual desde la carpeta que desde el pack (`crates/modengine/src/vfs.rs`), y los tests
comprueban con todos los ejemplos que se juega exactamente igual.

**Partidas guardadas** (`save_data`/`load_data`): un juego exportado no puede escribir dentro de sí
mismo, así que guarda en la carpeta del usuario:

| Sistema | Carpeta |
|---|---|
| Linux | `~/.local/share/<juego>/saves` (o `$XDG_DATA_HOME/<juego>/saves`) |
| Windows | `%APPDATA%\<juego>\saves` |
| Android | almacenamiento interno de la app (`/data/data/<paquete>/files/saves`) |

`<juego>` es el nombre del juego en minúsculas con `_` (`Coin Climber` → `coin_climber`). Con F12
los juegos exportados guardan capturas en la carpeta de al lado de `saves`.

`modengine run juego.dat` (o `modengine run "Coin Climber"`, el ejecutable exportado) juega un pack
con el motor de desarrollo; útil para comprobar una exportación.

### Runners y caché

La primera exportación a cada plataforma **compila el runner** (de 15 s a unos minutos). Se guarda
en `~/.cache/modengine/runners/<versión>-<huella del código>-<target>/` y las siguientes
exportaciones solo empaquetan el proyecto (menos de un segundo). Si cambias el código del motor, la
huella cambia y se recompila solo lo necesario (el compilado intermedio vive en
`~/.cache/modengine/build`, ~750 MB).

Para compilar hace falta el **código del motor**, que se busca en este orden:

1. la variable `MODENGINE_SRC`;
2. el workspace, si `modengine` se ejecuta desde `target/release` (desarrollo);
3. `<instalación>/codigo`, el clon que deja `instalar.sh` / `instalar.ps1`.

Exportar para el mismo sistema en el que estás (Linux desde Linux, Windows desde Windows) no
compila nada: usa el `modengine-runner` que hay junto a `modengine` (los instaladores lo copian).

## Requisitos por plataforma

`modengine export --doctor` dice qué plataformas están listas y qué falta;
`modengine export --setup <plataforma>` lo instala **en tu carpeta de usuario** (sin sudo). En el
editor, cada tarjeta tiene un botón **⬇ Instalar** que hace lo mismo en segundo plano.

### Linux

Nada desde Linux: usa el runner de al lado de `modengine`. El ejecutable necesita glibc 2.35 o más
nueva (Ubuntu 22.04, Debian 12, Fedora 36…) y `libasound.so.2` (ALSA, presente en cualquier
escritorio); la ventana y OpenGL se cargan al arrancar, sin dependencias de enlace.

Para que aparezca en el menú de aplicaciones, copia el `.desktop` a `~/.local/share/applications/`
(abre el ejecutable que tiene al lado).

### Windows

- **Desde Windows**: nada (usa `modengine-runner.exe`, target `x86_64-pc-windows-msvc`).
- **Desde Linux o Mac**: compilación cruzada a `x86_64-pc-windows-gnu` con
  [zig](https://ziglang.org) + [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild).
  `--setup windows` instala:
  - el target de Rust `x86_64-pc-windows-gnu` (~140 MB),
  - zig 0.15.2 en `~/.local/share/modengine-toolchains/zig` (~55 MB de descarga, ~375 MB),
  - `cargo install cargo-zigbuild` (~10 MB).

El `.exe` es un único fichero de ~10 MB (~4 MB en el .zip), sin consola (subsistema GUI) y sin DLL
aparte: solo usa las de Windows 10/11 (o Windows 7/8 con el runtime UCRT). Con
`[export] windows_console = true` abre también una consola, útil para ver `show_debug_message`.

#### Equipos sin OpenGL 3 (máquinas virtuales)

Mod Engine dibuja con OpenGL 3. Un Windows sin drivers de gráfica, o una máquina virtual sin
aceleración 3D (QEMU/VirtualBox/Hyper-V con el adaptador básico de Microsoft), solo trae OpenGL 1.1:
el juego no puede abrir su ventana y muestra «No se pudo iniciar el juego: este equipo no tiene
OpenGL 3» (también en `%APPDATA%\<juego>\error.log`).

Para esos equipos, marca **Incluir OpenGL por software** en el diálogo de exportar
(`[export] windows_software_gl = true`, o `modengine export … --target windows --software-gl`).
Junto al `.exe` van `opengl32.dll` y `libgallium_wgl.dll` de
[Mesa para Windows](https://github.com/pal1000/mesa-dist-win) (llvmpipe): el juego dibuja con el
procesador y arranca en cualquier Windows x64. La primera vez descarga Mesa (53 MB) y lo guarda en
`modengine-toolchains/mesa-<versión>`; el `.zip` del juego pasa de ~4 MB a ~32 MB.

Con esos ficheros Windows usa siempre Mesa, también en equipos con gráfica, que van más lentos: no
lo actives si tus jugadores tienen drivers normales. Para quitarlo en un equipo concreto basta con
borrar las dos DLL.

### Android

`--setup android` instala, en `~/Android/Sdk` (o en `$ANDROID_HOME` si ya tienes un SDK):

| Pieza | Descarga | Instalado |
|---|---|---|
| command-line tools (sdkmanager) | ~170 MB | ~170 MB |
| `platforms;android-35` | ~60 MB | ~130 MB |
| `build-tools;35.0.0` (aapt2, d8, zipalign, apksigner) | ~60 MB | ~150 MB |
| `platform-tools` (adb) | ~8 MB | ~22 MB |
| NDK r27d (`ndk;27.3.13750724`) | ~630 MB | ~2 GB |
| targets de Rust aarch64/armv7/x86_64-linux-android | ~150 MB c/u | |

Necesita **Java 17 o más nuevo** (un JDK con `javac` y `keytool`); `--setup` no lo instala: usa el
gestor de paquetes (`jdk-openjdk`, `openjdk-17-jdk`, …).

El APK se monta sin Gradle: `aapt2` (manifiesto, iconos, `assets/game.dat`) → `javac` + `d8` (la
actividad de [miniquad](https://github.com/not-fl3/miniquad), en `crates/modengine-cli/android/`)
→ bibliotecas nativas sin comprimir (`extractNativeLibs=false`, alineadas a 16 KB para Android 15)
→ `zipalign` → `apksigner`. minSdk 21 (Android 5), targetSdk 35.

Ajustes en `game.toml` (`[android]`, ver [proyecto.md](proyecto.md#gametoml)) o en el diálogo:
paquete, código de versión, orientación (horizontal, vertical o girar con el móvil), mando táctil y
arquitecturas. `arm64-v8a` sirve para casi todos los móviles; `x86_64` es para el emulador.

#### Controles táctiles

Un toque mueve el ratón (`mouse_pressed`, `gui_mouse_x`…) y `touch_count()`, `touch_x(i)`,
`touch_y(i)` dan todos los dedos. Para juegos de teclado hay un **mando en pantalla**:

```toml
[android]
virtual_pad = "arrows"                     # none, arrows (← →) o dpad (4 direcciones)
virtual_buttons = ["space:A", "enter:OK"]  # botones redondos: tecla[:etiqueta]
```

Cada botón mantiene pulsada su tecla mientras un dedo está encima, así que el juego no cambia. Para
probarlo en el ordenador: `modengine run <proyecto> --touch` (el ratón hace de dedo); el ejecutable
exportado acepta también `--touch`.

#### Firma

- **Depuración** (por defecto): una clave que se crea sola en
  `~/.config/modengine/android/debug.keystore`. Sirve para instalar y probar, no para publicar.
- **Publicación** (`--release` o «Firma: Publicación» en el diálogo): tu almacén de claves.

```bash
# crear uno (una vez; guárdalo bien: sin él no podrás actualizar la app)
keytool -genkeypair -keystore mi-juego.jks -alias juego -keyalg RSA -keysize 2048 -validity 10000

export MODENGINE_KEYSTORE_PASS='…'   # contraseña del almacén
export MODENGINE_KEY_PASS='…'        # de la clave, si es distinta
modengine export mijuego --target android --release --keystore mi-juego.jks --key-alias juego
```

`MODENGINE_KEYSTORE` y `MODENGINE_KEY_ALIAS` sustituyen a `--keystore`/`--key-alias`. Las
contraseñas solo se leen de variables de entorno: **nunca se guardan en el proyecto**. Para que el
editor las vea, defínelas antes de abrirlo.

#### Instalar en el móvil

Activa la depuración USB en el móvil, conéctalo y:

```bash
modengine export mijuego --target android --install     # o: adb install -r Juego.apk
```

## Línea de comandos

```text
modengine export <proyecto> --target linux|windows|android [--out CARPETA] [--release]
                 [--arch arm64-v8a,armeabi-v7a,x86_64] [--keystore F --key-alias A] [--install]
modengine export --setup linux|windows|android
modengine export --doctor [--json]
```

La salida tiene líneas `PASO n/total …` y acaba en `LISTO <ruta>` o `ERROR <motivo>` (el editor
las usa para la barra de progreso).

## Problemas frecuentes

| Mensaje | Solución |
|---|---|
| `no se encuentra el código del motor` | Instala con `instalar.sh`/`instalar.ps1` (deja `codigo/`) o define `MODENGINE_SRC=/ruta/al/repo`. Solo hace falta para compilar runners que no estén en caché. |
| `falta el target de Rust …` / `zig` | `modengine export --setup windows` (o `android`). |
| `hace falta Java 17` | Instala un JDK; si tienes varios, `JAVA_HOME` elige cuál. |
| `firmar en modo release necesita…` | Define `MODENGINE_KEYSTORE_PASS` (y el almacén/alias) antes de exportar. |
| El APK no instala: «firmas distintas» | Ya hay una versión firmada con otra clave: desinstálala (`adb uninstall <paquete>`). |
| El APK no instala: «versión anterior» | Sube `[android] version_code`. |
| Linux: `GLIBC_2.35 not found` | El sistema de destino es demasiado antiguo para el runner compilado aquí. |
| Windows SmartScreen avisa | Los .exe sin firma de código lo provocan; «Más información → Ejecutar de todas formas». |
| El juego exportado no arranca | Ejecútalo desde una terminal (o con `windows_console = true`) para ver el error; `modengine check <proyecto>` encuentra los fallos de script antes de exportar. |

## Lo que no hace (todavía)

- macOS e iOS no son destinos de exportación. Web (WebAssembly) tampoco.
- El icono del `.exe` en el Explorador de Windows es el genérico (la ventana sí usa `[game] icon`).
- Linux: no genera AppImage ni Flatpak; el runner sigue la versión de glibc del equipo que compila.
- Android: no genera App Bundles (`.aab`) para Google Play, solo APK.
