//! Mod Engine visual editor (`modeditor [project]`).

mod app;
mod highlight;
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
mod workspace;
mod tomlw;

fn main() -> eframe::Result {
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
