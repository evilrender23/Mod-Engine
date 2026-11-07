//! Mod Engine visual editor (`modeditor [project]`).

// a window-only program on Windows: no console window behind the editor
#![cfg_attr(windows, windows_subsystem = "windows")]

#[macro_use]
mod icons;
mod agent;
mod agent_attach;
mod agent_browser;
mod agent_history;
mod agent_markdown;
mod agent_panel;
mod agent_paste;
mod agent_slash;
mod agent_snapshot;
mod agent_workspace;
mod app;
mod dnd;
mod export;
mod extension;
mod font;
mod fx;
mod particles;
mod shader;
mod highlight;
mod curve;
mod datatable;
mod import;
mod kinds;
mod loc_editor;
mod note;
mod object;
mod objsrc;
mod motion;
mod path;
mod palette;
mod project;
mod proc;
mod projects;
mod room;
mod script;
mod sequence;
mod settings;
mod sound;
mod sprite;
mod store;
// the store's logic is shared with `modengine store` (crates/modengine-store)
use modengine_store::{credits as store_credits, net as store_net, oga as store_oga, zip as store_zip};
mod start;
mod tex;
mod theme;
mod timeline;
mod tileset;
mod typedvar;
mod update;
mod workspace;
mod tomlw;
mod zoom;

fn main() -> eframe::Result {
    attach_parent_console();
    // `modeditor --actualizar`: update an installed copy from the terminal, without a window.
    // `modeditor --version`: which commit this is.
    match std::env::args().nth(1).as_deref() {
        Some("--actualizar") => {
            if let Err(e) = update::Actualizador::default().sin_ventana() {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
            return Ok(());
        }
        Some("--version") => {
            println!("modeditor {}", update::descripcion_version());
            return Ok(());
        }
        _ => {}
    }
    // modeditor [project] [--open room:level1 ...] [--screenshot out.png]
    let mut path = None;
    let mut auto = app::Auto::default();
    let mut args = std::env::args().skip(1).peekable();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--open" => {
                if let Some(o) = args.next().and_then(|s| app::Open::parse(&s)) {
                    auto.open.push(o);
                }
            }
            "--screenshot" => auto.screenshot = args.next().map(std::path::PathBuf::from),
            "--select" => auto.select = true,
            "--fit" => auto.fit = true,
            "--agent" => auto.agent = true,
            // --run 2 : press ▶ Ejecutar (2 = Ejecutar 2 instancias) once the project is open
            "--run" => auto.run = args.next().and_then(|n| n.parse().ok()),
            // --new font: open the New dialog of a kind · --click 60,14: click there (menus)
            "--new" => auto.new = args.next().and_then(|k| kinds::parse_kind(&k)),
            "--click" => auto.click = args.next().and_then(|s| s.split_once(',').and_then(|(x, y)| Some(eframe::egui::Pos2::new(x.trim().parse().ok()?, y.trim().parse().ok()?)))),
            // --layer Mundo: select that layer in the opened rooms (screenshots of tile painting)
            "--layer" => auto.layer = args.next(),
            // --agent-demo history|queue|diff|browser|import : sample Agente state for screenshots
            "--agent-demo" => {
                auto.agent = true;
                auto.agent_demo = args.next();
            }
            // --import <file or folder>: open the import dialog with it (screenshots)
            "--import" => auto.import.extend(args.next().map(std::path::PathBuf::from)),
            "--import-run" => auto.import_run = true,
            // --new-project-dialog [platformer] : the Nuevo proyecto dialog (screenshots)
            "--new-project-dialog" => auto.new_project = Some(args.next_if(|a| modengine::scaffold::template(a).is_some()).unwrap_or_default()),
            // --open-project-dialog [folder] : the Abrir proyecto browser at that folder
            "--open-project-dialog" => {
                let dir = args.next_if(|a| std::path::Path::new(a).is_dir()).map(std::path::PathBuf::from);
                auto.open_project = dir.or_else(projects::documents_dir);
            }
            // --config-dir DIR : recent projects etc. from DIR instead of ~/.config/modengine
            "--config-dir" => auto.config_dir = args.next().map(std::path::PathBuf::from),
            // --store slime : open the Tienda de assets searching that · --store-open <slug> : on that asset
            "--store" => crate::store::open_with(args.next().as_deref(), None),
            "--store-open" => crate::store::open_with(None, args.next().as_deref()),
            // --palettes pear36[:replace] : open the Paletas dialog with that palette selected
            "--palettes" => palette::open_dialog_with(&args.next().unwrap_or_default()),
            // --export-dialog android : open Archivo → Exportar… on that target (screenshots)
            "--export-dialog" => export::open_dialog_with(args.next().as_deref().and_then(export::Target::parse), true),
            // --drag object:Coin 700,400 : screenshot of a drag held at that point
            "--drag" => {
                let what = args.next().and_then(|s| app::Open::parse(&s));
                let at = args.next().and_then(|s| s.split_once(',').and_then(|(x, y)| Some(eframe::egui::Pos2::new(x.trim().parse().ok()?, y.trim().parse().ok()?))));
                if let (Some(w), Some(p)) = (what, at) {
                    auto.drag = Some((w, p));
                }
            }
            _ => path = Some(std::path::PathBuf::from(a)),
        }
    }
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Mod Engine Editor")
            .with_inner_size(if auto.screenshot.is_some() { [1920.0, 1080.0] } else { [1400.0, 860.0] })
            .with_min_inner_size([900.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native("Mod Engine Editor", options, Box::new(move |cc| Ok(Box::new(app::EditorApp::new(cc, path, auto)))))
}

/// Started from a terminal (cmd, PowerShell): print `--version`, `--actualizar` and logs
/// there. Started from Explorer there is no parent console and nothing is shown.
#[cfg(windows)]
fn attach_parent_console() {
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    #[link(name = "kernel32")]
    extern "system" {
        fn AttachConsole(pid: u32) -> i32;
    }
    // SAFETY: plain Win32 call; failing (no parent console) leaves stdout/stderr unset
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}

#[cfg(not(windows))]
fn attach_parent_console() {}
