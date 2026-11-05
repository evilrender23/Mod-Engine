//! Mod Engine visual editor (`modeditor [project]`).

mod agent;
mod agent_attach;
mod agent_panel;
mod app;
mod dnd;
mod highlight;
mod import;
mod object;
mod objsrc;
mod project;
mod room;
mod script;
mod settings;
mod sound;
mod sprite;
mod tex;
mod theme;
mod typedvar;
mod update;
mod workspace;
mod tomlw;
mod zoom;

fn main() -> eframe::Result {
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
    let mut args = std::env::args().skip(1);
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
            // --import <file or folder>: open the import dialog with it (screenshots)
            "--import" => auto.import.extend(args.next().map(std::path::PathBuf::from)),
            "--import-run" => auto.import_run = true,
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
