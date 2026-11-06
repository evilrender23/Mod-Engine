//! Archivo → Exportar…: makes a game players can run (Windows .exe, Linux, Android .apk).
//!
//! The work is done by `modengine export` (next to the editor, like ▶ Ejecutar) in a child
//! process: `--doctor --json` for the toolchain status of each target, `--setup <t>` to
//! install what is missing, and the export itself. Their output streams into the dialog
//! and the Salida dock, so the editor never blocks.

use crate::app::{LogKind, LogLine};
use crate::project::Project;
use eframe::egui::{self, RichText};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Target {
    Windows,
    Linux,
    Android,
}

pub const TARGETS: [Target; 3] = [Target::Windows, Target::Linux, Target::Android];

impl Target {
    pub fn id(self) -> &'static str {
        match self {
            Target::Windows => "windows",
            Target::Linux => "linux",
            Target::Android => "android",
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Target::Windows => "Windows",
            Target::Linux => "Linux",
            Target::Android => "Android",
        }
    }
    fn what(self) -> &'static str {
        match self {
            Target::Windows => ".exe en un .zip (64 bits)",
            Target::Linux => "ejecutable + .tar.gz",
            Target::Android => ".apk para el móvil",
        }
    }
    pub fn parse(s: &str) -> Option<Target> {
        TARGETS.into_iter().find(|t| t.id() == s)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub ready: bool,
    /// (name, ok, detail)
    pub checks: Vec<(String, bool, String)>,
    /// What "Instalar" downloads, with sizes.
    pub setup: String,
}

/// Parses `modengine export --doctor --json`.
pub fn parse_doctor(json: &str) -> Result<HashMap<Target, Status>, String> {
    let v: serde_json::Value = serde_json::from_str(json.trim()).map_err(|e| format!("respuesta inesperada de modengine export --doctor: {e}"))?;
    let mut out = HashMap::new();
    for t in v.as_array().ok_or("--doctor no devolvió una lista")? {
        let Some(target) = t["target"].as_str().and_then(Target::parse) else { continue };
        let checks = t["checks"]
            .as_array()
            .map(|cs| cs.iter().map(|c| (c["name"].as_str().unwrap_or("").to_string(), c["ok"].as_bool().unwrap_or(false), c["detail"].as_str().unwrap_or("").to_string())).collect())
            .unwrap_or_default();
        out.insert(target, Status { ready: t["ready"].as_bool().unwrap_or(false), checks, setup: t["setup"].as_str().unwrap_or("").to_string() });
    }
    Ok(out)
}

/// The export settings of the form, kept in game.toml (`[game]`, `[export]`, `[android]`).
#[derive(Clone, Debug, PartialEq)]
pub struct Form {
    pub name: String,
    pub version: String,
    pub icon: String,
    pub exe_name: String,
    pub out_dir: String,
    pub windows_console: bool,
    pub windows_software_gl: bool,
    pub package: String,
    pub version_code: u32,
    pub orientation: String,
    pub virtual_pad: String,
    pub buttons: String,
    pub abis: Vec<String>,
}

pub const ABIS: [(&str, &str); 3] = [("arm64-v8a", "arm64 (móviles)"), ("armeabi-v7a", "armv7 (antiguos)"), ("x86_64", "x86_64 (emulador)")];

impl Form {
    pub fn from_game(g: &modengine::project::GameToml) -> Form {
        Form {
            name: g.game.name.clone(),
            version: g.game.version.clone(),
            icon: g.game.icon.clone().unwrap_or_default(),
            exe_name: g.export.exe_name.clone().unwrap_or_default(),
            out_dir: g.export.out_dir.clone().unwrap_or_default(),
            windows_console: g.export.windows_console,
            windows_software_gl: g.export.windows_software_gl,
            package: g.android.package.clone().unwrap_or_default(),
            version_code: g.android.version_code,
            orientation: g.android.orientation.clone(),
            virtual_pad: g.android.virtual_pad.clone(),
            buttons: g.android.virtual_buttons.join(", "),
            abis: g.android.abis.clone(),
        }
    }

    pub fn apply(&self, g: &mut modengine::project::GameToml, target: Target) {
        let opt = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
        g.game.name = self.name.trim().to_string();
        g.game.version = if self.version.trim().is_empty() { "1.0.0".into() } else { self.version.trim().to_string() };
        g.game.icon = opt(&self.icon);
        g.export.exe_name = opt(&self.exe_name);
        g.export.out_dir = opt(&self.out_dir);
        g.export.windows_console = self.windows_console;
        g.export.windows_software_gl = self.windows_software_gl;
        g.export.target = Some(target.id().to_string());
        g.android.package = opt(&self.package);
        g.android.version_code = self.version_code.max(1);
        g.android.orientation = self.orientation.clone();
        g.android.virtual_pad = self.virtual_pad.clone();
        g.android.virtual_buttons = self.buttons.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        g.android.abis = if self.abis.is_empty() { vec!["arm64-v8a".into()] } else { self.abis.clone() };
    }
}

enum Job {
    Setup(Target),
    Export,
}

struct Running {
    job: Job,
    child: Child,
    rx: Receiver<String>,
}

pub struct ExportDialog {
    pub target: Target,
    /// Chosen when opening (`--export-dialog android`): game.toml's last target doesn't win.
    target_fixed: bool,
    pub status: Option<Result<HashMap<Target, Status>, String>>,
    doctor_rx: Option<Receiver<Result<HashMap<Target, Status>, String>>>,
    running: Option<Running>,
    pub lines: Vec<String>,
    /// Last "PASO n/total texto".
    pub step: Option<(u32, u32, String)>,
    pub result: Option<Result<PathBuf, String>>,
    pub form: Option<Form>,
    pub release: bool,
    pub keystore: String,
    pub key_alias: String,
    /// `modengine` next to the editor (None: not found).
    pub tool: Option<PathBuf>,
}

thread_local! {
    static DIALOG: RefCell<Option<ExportDialog>> = const { RefCell::new(None) };
}

/// `modengine` next to the editor executable.
pub fn tool_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let name = if cfg!(windows) { "modengine.exe" } else { "modengine" };
    [exe.with_file_name(name), exe.parent()?.parent()?.join(name)].into_iter().find(|p| p.is_file())
}

pub fn open_dialog() {
    open_dialog_with(None, false);
}

/// `wait`: read the toolchain status before the first frame (screenshots).
pub fn open_dialog_with(target: Option<Target>, wait: bool) {
    let mut d = ExportDialog::new(tool_path());
    if let Some(t) = target {
        d.target = t;
        d.target_fixed = true;
    }
    if wait {
        if let Some(rx) = d.doctor_rx.take() {
            d.status = rx.recv().ok();
        }
    }
    DIALOG.with(|s| *s.borrow_mut() = Some(d));
}

#[cfg(test)]
pub fn is_open() -> bool {
    DIALOG.with(|d| d.borrow().is_some())
}

#[cfg(test)]
pub fn with_dialog<R>(f: impl FnOnce(&mut ExportDialog) -> R) -> Option<R> {
    DIALOG.with(|d| d.borrow_mut().as_mut().map(f))
}

#[cfg(test)]
pub fn set_dialog(d: ExportDialog) {
    DIALOG.with(|s| *s.borrow_mut() = Some(d));
}

pub fn dialog_frame(ctx: &egui::Context, project: Option<&mut Project>, log: &mut Vec<LogLine>) {
    let Some(mut d) = DIALOG.with(|d| d.borrow_mut().take()) else { return };
    let Some(p) = project else { return };
    if d.show(ctx, p, log) {
        DIALOG.with(|slot| *slot.borrow_mut() = Some(d));
    }
}

fn spawn_lines(mut cmd: Command) -> Result<(Child, Receiver<String>), String> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("no se pudo lanzar {}: {e}", cmd.get_program().to_string_lossy()))?;
    let (tx, rx) = mpsc::channel();
    if let Some(o) = child.stdout.take() {
        let tx = tx.clone();
        std::thread::spawn(move || BufReader::new(o).lines().map_while(Result::ok).for_each(|l| drop(tx.send(l))));
    }
    if let Some(e) = child.stderr.take() {
        std::thread::spawn(move || BufReader::new(e).lines().map_while(Result::ok).for_each(|l| drop(tx.send(l))));
    }
    Ok((child, rx))
}

/// Opens a folder in the file manager.
pub fn open_folder(p: &Path) {
    let dir = if p.is_dir() { p.to_path_buf() } else { p.parent().unwrap_or(p).to_path_buf() };
    let prog = if cfg!(windows) { "explorer" } else if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let _ = Command::new(prog).arg(dir).spawn();
}

impl ExportDialog {
    pub fn new(tool: Option<PathBuf>) -> ExportDialog {
        let mut d = ExportDialog {
            target: Target::Windows,
            target_fixed: false,
            status: None,
            doctor_rx: None,
            running: None,
            lines: vec![],
            step: None,
            result: None,
            form: None,
            release: false,
            keystore: String::new(),
            key_alias: String::new(),
            tool,
        };
        d.refresh();
        d
    }

    /// Asks `modengine export --doctor --json` again (in the background).
    pub fn refresh(&mut self) {
        let Some(tool) = self.tool.clone() else {
            self.status = Some(Err("no encuentro el ejecutable `modengine` junto al editor (cargo build --release -p modengine-cli)".into()));
            return;
        };
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut c = crate::proc::command(tool);
            c.args(["export", "--doctor", "--json"]);
            let r = c.output().map_err(|e| e.to_string()).and_then(|o| parse_doctor(&String::from_utf8_lossy(&o.stdout)));
            let _ = tx.send(r);
        });
        self.doctor_rx = Some(rx);
    }

    pub fn busy(&self) -> bool {
        self.running.is_some()
    }

    fn status_of(&self, t: Target) -> Option<&Status> {
        self.status.as_ref().and_then(|s| s.as_ref().ok()).and_then(|m| m.get(&t))
    }

    fn start(&mut self, job: Job, args: Vec<String>, log: &mut Vec<LogLine>) {
        let Some(tool) = self.tool.clone() else {
            self.result = Some(Err("no encuentro `modengine` junto al editor".into()));
            return;
        };
        let mut c = crate::proc::command(tool);
        c.args(&args);
        self.lines.clear();
        self.step = None;
        self.result = None;
        log.push(LogLine { kind: LogKind::Info, text: format!("modengine {}", args.join(" ")) });
        match spawn_lines(c) {
            Ok((child, rx)) => self.running = Some(Running { job, child, rx }),
            Err(e) => self.result = Some(Err(e)),
        }
    }

    /// Reads new output lines; true when something changed.
    pub fn poll(&mut self, log: &mut Vec<LogLine>) -> bool {
        let mut changed = false;
        if let Some(rx) = &self.doctor_rx {
            if let Ok(r) = rx.try_recv() {
                self.status = Some(r);
                self.doctor_rx = None;
                changed = true;
            }
        }
        let Some(run) = &mut self.running else { return changed };
        while let Ok(l) = run.rx.try_recv() {
            changed = true;
            if let Some(rest) = l.strip_prefix("PASO ") {
                if let Some((nm, text)) = rest.split_once(' ') {
                    if let Some((n, m)) = nm.split_once('/') {
                        self.step = Some((n.parse().unwrap_or(0), m.parse().unwrap_or(1), text.to_string()));
                    }
                }
            }
            if let Some(p) = l.strip_prefix("LISTO ") {
                self.result = Some(Ok(PathBuf::from(p)));
                log.push(LogLine { kind: LogKind::Ok, text: format!("✔ exportado: {p}") });
            } else if let Some(e) = l.strip_prefix("ERROR ") {
                self.result = Some(Err(e.to_string()));
                log.push(LogLine { kind: LogKind::Error, text: format!("exportar: {e}") });
            } else if !l.starts_with("  ") && !l.starts_with('$') {
                // our own messages; tool output stays in the dialog
                log.push(LogLine { kind: LogKind::Info, text: l.clone() });
            }
            self.lines.push(l);
            if self.lines.len() > 400 {
                self.lines.drain(..100);
            }
        }
        if let Ok(Some(st)) = run.child.try_wait() {
            // drain what is left
            while let Ok(l) = run.rx.try_recv() {
                self.lines.push(l);
            }
            let setup = matches!(run.job, Job::Setup(_));
            self.running = None;
            if self.result.is_none() {
                self.result = Some(if st.success() { Ok(PathBuf::new()) } else { Err(format!("modengine terminó con {st}")) });
            }
            if setup {
                if let Some(Ok(_)) = &self.result {
                    self.result = None;
                    self.lines.push("✔ instalado".into());
                }
                self.refresh();
            }
            changed = true;
        }
        changed
    }

    fn cancel(&mut self) {
        if let Some(mut r) = self.running.take() {
            let _ = r.child.kill();
            let _ = r.child.wait();
            self.result = Some(Err("cancelado".into()));
        }
    }

    /// Saves the form into game.toml and starts `modengine export`.
    fn export(&mut self, p: &mut Project, log: &mut Vec<LogLine>) {
        let Some(form) = &self.form else { return };
        form.apply(&mut p.game, self.target);
        let path = p.dir.join("game.toml");
        if let Err(e) = std::fs::write(&path, crate::settings::write_game_toml(&p.game)) {
            self.result = Some(Err(format!("{}: {e}", path.display())));
            return;
        }
        // open Ajustes del juego editors without changes pick up the new values
        crate::palette::bump();
        let mut args = vec!["export".to_string(), p.dir.to_string_lossy().to_string(), "--target".into(), self.target.id().into()];
        if self.target == Target::Android && self.release {
            args.push("--release".into());
            if !self.keystore.trim().is_empty() {
                args.extend(["--keystore".to_string(), self.keystore.trim().to_string()]);
            }
            if !self.key_alias.trim().is_empty() {
                args.extend(["--key-alias".to_string(), self.key_alias.trim().to_string()]);
            }
        }
        self.start(Job::Export, args, log);
    }

    /// Draws the dialog; false when it was closed.
    pub fn show(&mut self, ctx: &egui::Context, p: &mut Project, log: &mut Vec<LogLine>) -> bool {
        if self.poll(log) {
            ctx.request_repaint();
        }
        if self.running.is_some() || self.doctor_rx.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        if self.form.is_none() {
            self.form = Some(Form::from_game(&p.game));
            if let Some(t) = p.game.export.target.as_deref().and_then(Target::parse).filter(|_| !self.target_fixed) {
                self.target = t;
            }
        }
        let mut open = true;
        let screen = ctx.content_rect();
        egui::Modal::new(egui::Id::unique("export_dialog")).show(ctx, |ui| {
            ui.set_width(820.0_f32.min(screen.width() - 60.0));
            ui.horizontal(|ui| {
                ui.heading("📦 Exportar juego");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_enabled(!self.busy(), egui::Button::new("⟳ Comprobar de nuevo")).on_hover_text("Vuelve a buscar los compiladores y SDK instalados").clicked() {
                        self.refresh();
                    }
                });
            });
            ui.label(RichText::new("Crea una versión del juego que se puede jugar sin Mod Engine.").weak());
            ui.add_space(6.0);
            self.cards(ui, log);
            ui.add_space(6.0);
            ui.separator();
            let max_h = (screen.height() - 470.0).clamp(150.0, 420.0);
            egui::ScrollArea::vertical().id_salt("export_form").max_height(max_h).show(ui, |ui| self.form_ui(ui, p));
            ui.separator();
            self.progress(ui);
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let ready = self.status_of(self.target).is_some_and(|s| s.ready);
                let can = !self.busy() && ready && self.form.as_ref().is_some_and(|f| !f.name.trim().is_empty());
                let label = format!("Exportar a {}", self.target.title());
                if ui.add_enabled(can, crate::theme::primary_button(ui, &label)).on_disabled_hover_text("Instala primero lo que falta para esta plataforma").clicked() {
                    self.export(p, log);
                }
                if self.busy() && ui.button("Cancelar").clicked() {
                    self.cancel();
                }
                if let Some(Ok(path)) = &self.result {
                    if !path.as_os_str().is_empty() && ui.button("📂 Abrir carpeta").clicked() {
                        open_folder(path);
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_enabled(!self.busy(), egui::Button::new("Cerrar")).clicked() {
                        open = false;
                    }
                });
            });
        });
        open
    }

    fn cards(&mut self, ui: &mut egui::Ui, log: &mut Vec<LogLine>) {
        let pal = crate::theme::colors(ui.ctx());
        let mut install = None;
        ui.columns(3, |cols| {
            for (i, t) in TARGETS.into_iter().enumerate() {
                let ui = &mut cols[i];
                let selected = self.target == t;
                let stroke = if selected { ui.visuals().selection.stroke } else { ui.visuals().widgets.noninteractive.bg_stroke };
                let r = egui::Frame::group(ui.style())
                    .stroke(egui::Stroke::new(if selected { 2.0 } else { 1.0 }, stroke.color))
                    .fill(if selected { ui.visuals().selection.bg_fill.gamma_multiply(0.25) } else { egui::Color32::TRANSPARENT })
                    .show(ui, |ui| {
                        ui.set_min_height(86.0);
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new(t.title()).heading().strong());
                        ui.label(RichText::new(t.what()).weak());
                        match (&self.status, self.status_of(t)) {
                            (None, _) => {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label("comprobando…");
                                });
                            }
                            (Some(Err(_)), _) => {
                                ui.colored_label(pal.error, "⚠ no disponible");
                            }
                            (_, Some(s)) if s.ready => {
                                ui.colored_label(pal.ok, "✔ listo");
                            }
                            (_, Some(s)) => {
                                let missing: Vec<&str> = s.checks.iter().filter(|c| !c.1 && !c.0.contains("opcional") && c.0 != "runner" && !c.0.starts_with("motor para")).map(|c| c.0.as_str()).collect();
                                ui.colored_label(pal.error, format!("⚠ falta {}", missing.first().copied().unwrap_or("algo"))).on_hover_text(missing.join(", "));
                                let installing = matches!(&self.running, Some(Running { job: Job::Setup(x), .. }) if *x == t);
                                if installing {
                                    ui.horizontal(|ui| {
                                        ui.spinner();
                                        ui.label("instalando…");
                                    });
                                } else if ui.add_enabled(!self.busy(), egui::Button::new("⬇ Instalar")).on_hover_text(format!("Descarga e instala en tu carpeta de usuario: {}", s.setup)).clicked() {
                                    install = Some(t);
                                }
                            }
                            (_, None) => {
                                ui.label("?");
                            }
                        }
                    })
                    .response;
                let r = ui.interact(r.rect, ui.scope_id().with(("export_card", i)), egui::Sense::click());
                if r.clicked() && !self.busy() {
                    self.target = t;
                }
            }
        });
        if let Some(Err(e)) = &self.status {
            ui.colored_label(pal.error, e);
        }
        if let Some(t) = install {
            self.target = t;
            self.start(Job::Setup(t), vec!["export".into(), "--setup".into(), t.id().into()], log);
        }
    }

    fn form_ui(&mut self, ui: &mut egui::Ui, p: &Project) {
        let target = self.target;
        let Some(f) = &mut self.form else { return };
        let enabled = self.running.is_none();
        ui.add_enabled_ui(enabled, |ui| {
            egui::Grid::new("export_game").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.label("Nombre");
                ui.text_edit_singleline(&mut f.name);
                ui.end_row();
                ui.label("Versión");
                ui.add(egui::TextEdit::singleline(&mut f.version).desired_width(120.0));
                ui.end_row();
                ui.label("Icono (PNG)");
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("export_icon").selected_text(if f.icon.is_empty() { "(icono por defecto)".to_string() } else { f.icon.clone() }).show_ui(ui, |ui| {
                        ui.selectable_value(&mut f.icon, String::new(), "(icono por defecto)");
                        for s in &p.sprites {
                            // sprites drawn from a PNG file (inline pixel art has none)
                            let png = if s.is_toml { s.path.with_extension("png") } else { s.path.clone() };
                            if let (true, Ok(rel)) = (png.is_file(), png.strip_prefix(&p.dir)) {
                                let rel = rel.to_string_lossy().replace('\\', "/");
                                ui.selectable_value(&mut f.icon, rel.clone(), rel);
                            }
                        }
                    });
                })
                .response
                .on_hover_text("Se usa para la ventana del juego, el lanzador de Linux y el icono del APK");
                ui.end_row();
                ui.label("Ejecutable");
                ui.add(egui::TextEdit::singleline(&mut f.exe_name).hint_text(f.name.clone()));
                ui.end_row();
                ui.label("Carpeta de salida");
                ui.add(egui::TextEdit::singleline(&mut f.out_dir).hint_text(format!("export (→ export/{})", target.id())))
                    .on_hover_text("Relativa al proyecto o absoluta; se añade una subcarpeta por plataforma");
                ui.end_row();
            });
            match target {
                Target::Windows => {
                    ui.add_space(4.0);
                    ui.checkbox(&mut f.windows_console, "Mostrar una consola junto al juego (para depurar)");
                    ui.checkbox(&mut f.windows_software_gl, "Incluir OpenGL por software (Mesa, unos 80 MB)").on_hover_text(
                        "Para equipos sin OpenGL 3: máquinas virtuales sin aceleración 3D o sin drivers de gráfica. \
                         Con estos ficheros el juego dibuja siempre con el procesador, también en equipos con gráfica, \
                         así que va más lento: déjalo apagado si tus jugadores tienen drivers normales. La primera vez descarga Mesa (56 MB).",
                    );
                }
                Target::Linux => {}
                Target::Android => {
                    ui.add_space(6.0);
                    ui.strong("Android");
                    egui::Grid::new("export_android").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                        ui.label("Paquete");
                        let mut g = p.game.clone();
                        g.game.name = f.name.clone();
                        g.android.package = None;
                        ui.add(egui::TextEdit::singleline(&mut f.package).hint_text(g.android_package())).on_hover_text("Identificador único de la app, p. ej. com.miestudio.mijuego");
                        ui.end_row();
                        ui.label("Código de versión");
                        ui.add(egui::DragValue::new(&mut f.version_code).range(1..=2_100_000_000)).on_hover_text("Súbelo en cada versión que publiques");
                        ui.end_row();
                        ui.label("Orientación");
                        egui::ComboBox::from_id_salt("export_orient").selected_text(orientation_label(&f.orientation)).show_ui(ui, |ui| {
                            for o in ["landscape", "portrait", "sensor"] {
                                ui.selectable_value(&mut f.orientation, o.to_string(), orientation_label(o));
                            }
                        });
                        ui.end_row();
                        ui.label("Mando táctil");
                        egui::ComboBox::from_id_salt("export_pad").selected_text(pad_label(&f.virtual_pad)).show_ui(ui, |ui| {
                            for o in ["none", "arrows", "dpad"] {
                                ui.selectable_value(&mut f.virtual_pad, o.to_string(), pad_label(o));
                            }
                        });
                        ui.end_row();
                        ui.label("Botones táctiles");
                        ui.add(egui::TextEdit::singleline(&mut f.buttons).hint_text("space:A, enter:OK")).on_hover_text("Teclas que pulsan los botones redondos (tecla o tecla:etiqueta), separados por comas");
                        ui.end_row();
                        ui.label("Arquitecturas");
                        ui.horizontal(|ui| {
                            for (abi, label) in ABIS {
                                let mut on = f.abis.iter().any(|a| a == abi);
                                if ui.checkbox(&mut on, label).changed() {
                                    f.abis.retain(|a| a != abi);
                                    if on {
                                        f.abis.push(abi.to_string());
                                    }
                                }
                            }
                        });
                        ui.end_row();
                        ui.label("Firma");
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut self.release, false, "Depuración (para probar)");
                            ui.radio_value(&mut self.release, true, "Publicación");
                        });
                        ui.end_row();
                        if self.release {
                            ui.label("Almacén de claves");
                            ui.add(egui::TextEdit::singleline(&mut self.keystore).hint_text("MODENGINE_KEYSTORE o ruta al .jks"));
                            ui.end_row();
                            ui.label("Alias");
                            ui.add(egui::TextEdit::singleline(&mut self.key_alias).hint_text("MODENGINE_KEY_ALIAS"));
                            ui.end_row();
                        }
                    });
                    if self.release {
                        ui.label(RichText::new("La contraseña se lee de la variable de entorno MODENGINE_KEYSTORE_PASS (y MODENGINE_KEY_PASS): nunca se guarda en el proyecto.").weak());
                    }
                }
            }
        });
    }

    fn progress(&mut self, ui: &mut egui::Ui) {
        let pal = crate::theme::colors(ui.ctx());
        if let Some((n, m, text)) = &self.step {
            let frac = if self.busy() { (*n as f32 - 0.5).max(0.0) / (*m).max(1) as f32 } else { 1.0 };
            ui.add(egui::ProgressBar::new(frac).text(format!("{n}/{m} {text}")).animate(self.busy()));
        } else if self.busy() {
            ui.add(egui::ProgressBar::new(0.0).text("empezando…").animate(true));
        }
        match &self.result {
            Some(Ok(p)) if !p.as_os_str().is_empty() => {
                ui.colored_label(pal.ok, format!("✔ Listo: {}", p.display()));
            }
            Some(Err(e)) => {
                ui.colored_label(pal.error, format!("✘ {e}"));
            }
            _ => {}
        }
        if !self.lines.is_empty() {
            egui::ScrollArea::vertical().id_salt("export_log").max_height(110.0).auto_shrink([false, true]).stick_to_bottom(true).show(ui, |ui| {
                for l in &self.lines {
                    ui.label(RichText::new(l).monospace().small().color(pal.info));
                }
            });
        }
    }
}

fn orientation_label(o: &str) -> &'static str {
    match o {
        "portrait" => "Vertical",
        "sensor" => "Girar con el móvil",
        _ => "Horizontal",
    }
}

fn pad_label(o: &str) -> &'static str {
    match o {
        "arrows" => "Flechas ← →",
        "dpad" => "Cruceta (4 direcciones)",
        _ => "Ninguno",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCTOR: &str = r#"[{"target":"windows","ready":false,"setup":"zig (~55 MB)","checks":[{"name":"runner","ok":false,"detail":"x"},{"name":"zig","ok":false,"detail":"no encontrado"}]},
        {"target":"linux","ready":true,"setup":"nada","checks":[{"name":"runner","ok":true,"detail":"junto a modengine"}]},
        {"target":"android","ready":true,"setup":"NDK","checks":[]}]"#;

    #[test]
    fn doctor_json_is_read() {
        let m = parse_doctor(DOCTOR).unwrap();
        assert!(!m[&Target::Windows].ready && m[&Target::Linux].ready);
        assert_eq!(m[&Target::Windows].checks[1], ("zig".into(), false, "no encontrado".into()));
        assert!(parse_doctor("nope").is_err());
    }

    #[test]
    fn form_round_trips_through_game_toml() {
        let mut g: modengine::project::GameToml = toml::from_str("[game]\nname = \"Coin\"\n").unwrap();
        let mut f = Form::from_game(&g);
        assert_eq!((f.version.as_str(), f.orientation.as_str(), f.virtual_pad.as_str()), ("1.0.0", "landscape", "none"));
        f.version = "2.0".into();
        f.package = "org.ana.coin".into();
        f.virtual_pad = "arrows".into();
        f.buttons = "space:A,  z ,".into();
        f.abis = vec!["arm64-v8a".into(), "x86_64".into()];
        f.windows_console = true;
        f.apply(&mut g, Target::Android);
        assert_eq!(g.android.virtual_buttons, vec!["space:A", "z"]);
        assert_eq!(g.export.target.as_deref(), Some("android"));
        let back: modengine::project::GameToml = toml::from_str(&crate::settings::write_game_toml(&g)).unwrap();
        assert_eq!(Form::from_game(&back), Form { buttons: "space:A, z".into(), ..f });
    }

    #[test]
    fn dialog_renders_cards_form_and_progress() {
        use egui_kittest::kittest::Queryable;
        use egui_kittest::Harness;
        let dir = crate::room::tests::temp_project("export_ui");
        struct St {
            p: Project,
            log: Vec<LogLine>,
        }
        let mut d = ExportDialog::new(None);
        d.status = Some(Ok(parse_doctor(DOCTOR).unwrap()));
        set_dialog(d);
        let mut h = Harness::builder().with_size([1200.0, 900.0]).allow_missing_glyphs().with_accessibility_check(false).build_ui_state(
            |ui, s: &mut St| dialog_frame(ui.ctx(), Some(&mut s.p), &mut s.log),
            St { p: Project::open(&dir).unwrap(), log: vec![] },
        );
        h.run_steps(3);
        assert!(is_open());
        for t in ["Windows", "Linux", "Android", "⬇ Instalar", "Nombre", "Versión"] {
            h.get_by_label(t);
        }
        h.get_by_label_contains("falta zig");
        // the platformer configures touch controls for Android
        h.get_by_label("Android").click();
        h.run_steps(3);
        assert_eq!(with_dialog(|d| d.target), Some(Target::Android));
        h.get_by_label("Mando táctil");
        assert_eq!(with_dialog(|d| d.form.as_ref().map(|f| (f.virtual_pad.clone(), f.buttons.clone()))).flatten(), Some(("arrows".into(), "space:A, enter:OK".into())));
        h.get_by_label("Exportar a Android");
        // fake progress from `modengine export`
        with_dialog(|d| {
            d.step = Some((3, 4, "Preparando el motor para Android".into()));
            d.result = Some(Ok(PathBuf::from("/tmp/Coin Climber.apk")));
            d.lines = vec!["PASO 3/4 Preparando el motor para Android".into()];
        });
        h.run_steps(2);
        h.get_by_label_contains("Listo: /tmp/Coin Climber.apk");
        h.get_by_label("📂 Abrir carpeta");
        h.get_by_label("Cerrar").click();
        h.run_steps(2);
        assert!(!is_open());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
