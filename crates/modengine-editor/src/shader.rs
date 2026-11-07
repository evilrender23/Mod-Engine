//! Shader editor: the vertex and fragment sources in two code tabs with GLSL highlighting,
//! a static check on every keystroke, a real GPU compile (`modengine shader --check`), a
//! rendered preview and a live preview window that reloads on save.

use crate::app::{Cx, LogKind, LogLine};
use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{self, Color32, FontId, RichText, TextureHandle, Vec2};
use modengine::shader::{link_problems, parse_uniforms, validate, Stage, UType, DEFAULT_FSH, DEFAULT_VSH};
use std::collections::HashSet;
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::sync::mpsc::{self, Receiver};

const KEYWORDS: &[&str] = &[
    "attribute", "uniform", "varying", "const", "void", "if", "else", "for", "while", "do", "return", "discard", "precision", "lowp", "mediump",
    "highp", "in", "out", "inout", "struct", "break", "continue", "true", "false",
];
const TYPES: &[&str] = &["float", "int", "bool", "vec2", "vec3", "vec4", "ivec2", "ivec3", "ivec4", "bvec2", "bvec3", "bvec4", "mat2", "mat3", "mat4", "sampler2D", "samplerCube"];
const FUNCS: &[&str] = &[
    "texture2D", "texture2DLod", "textureCube", "radians", "degrees", "sin", "cos", "tan", "asin", "acos", "atan", "pow", "exp", "log", "exp2", "log2", "sqrt",
    "inversesqrt", "abs", "sign", "floor", "ceil", "fract", "mod", "min", "max", "clamp", "mix", "step", "smoothstep", "length", "distance", "dot", "cross",
    "normalize", "reflect", "refract", "faceforward", "matrixCompMult", "lessThan", "greaterThan", "equal", "any", "all", "not",
];
const BUILTIN_VARS: &[&str] = &[
    "gl_Position", "gl_FragColor", "gl_FragCoord", "gl_FragData", "gl_PointSize", "gl_FrontFacing", "gm_Matrices", "gm_BaseTexture", "in_Position",
    "in_Colour", "in_Color", "in_TextureCoord", "in_Normal", "v_vTexcoord", "v_vColour", "MATRIX_VIEW", "MATRIX_PROJECTION", "MATRIX_WORLD",
    "MATRIX_WORLD_VIEW", "MATRIX_WORLD_VIEW_PROJECTION",
];

/// GLSL highlighting; lines in `bad` get a red background.
pub fn highlight_glsl(src: &str, font: FontId, dark: bool, bad: &HashSet<u32>) -> LayoutJob {
    let mut job = LayoutJob::default();
    let p = crate::theme::code_palette(dark);
    let err_bg = if dark { Color32::from_rgba_unmultiplied(200, 60, 60, 60) } else { Color32::from_rgba_unmultiplied(255, 120, 120, 70) };
    let b = src.as_bytes();
    let mut i = 0;
    let mut line = 1u32;
    while i < b.len() {
        let c = b[i];
        let start = i;
        let col;
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            col = p.comment;
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                i += 1;
            }
            i = (i + 2).min(b.len());
            col = p.comment;
        } else if c == b'#' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            col = p.value;
        } else if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'.') {
                i += 1;
            }
            col = p.number;
        } else if c.is_ascii_alphabetic() || c == b'_' {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            let w = &src[start..i];
            col = if KEYWORDS.contains(&w) {
                p.keyword
            } else if TYPES.contains(&w) {
                p.string
            } else if FUNCS.contains(&w) {
                p.builtin
            } else if BUILTIN_VARS.contains(&w) {
                p.var
            } else {
                p.text
            };
        } else {
            i += src[i..].chars().next().map(|ch| ch.len_utf8()).unwrap_or(1);
            col = p.text;
        }
        // split the token at newlines so each line gets its own background
        for part in src[start..i].split_inclusive('\n') {
            let fmt = TextFormat { font_id: font.clone(), color: col, background: if bad.contains(&line) { err_bg } else { Color32::TRANSPARENT }, ..Default::default() };
            job.append(part, 0.0, fmt);
            if part.ends_with('\n') {
                line += 1;
            }
        }
    }
    job
}

pub struct ShaderEditor {
    pub name: String,
    vsh_path: PathBuf,
    fsh_path: PathBuf,
    vsh: String,
    fsh: String,
    saved: (String, String),
    vsh_existed: bool,
    stage: Stage,
    sprite: Option<String>,
    gpu: Option<Receiver<(bool, Vec<String>)>>,
    gpu_out: Option<(bool, Vec<String>)>,
    shot: Option<Receiver<Result<egui::ColorImage, String>>>,
    preview: Option<TextureHandle>,
    preview_err: Option<String>,
    live: Option<Child>,
}

impl ShaderEditor {
    pub fn open(s: &crate::fx::ShaderFiles) -> Result<Self, String> {
        let read = |p: &PathBuf, d: &str| if p.exists() { std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display())) } else { Ok(d.to_string()) };
        let vsh = read(&s.vsh, DEFAULT_VSH)?;
        let fsh = read(&s.fsh, DEFAULT_FSH)?;
        Ok(ShaderEditor {
            name: s.name.clone(),
            vsh_path: s.vsh.clone(),
            fsh_path: s.fsh.clone(),
            saved: (vsh.clone(), fsh.clone()),
            vsh_existed: s.vsh.exists(),
            vsh,
            fsh,
            stage: Stage::Fragment,
            sprite: None,
            gpu: None,
            gpu_out: None,
            shot: None,
            preview: None,
            preview_err: None,
            live: None,
        })
    }

    pub fn dirty(&self) -> bool {
        (self.vsh.as_str(), self.fsh.as_str()) != (self.saved.0.as_str(), self.saved.1.as_str())
    }

    pub fn save(&mut self) -> Result<(), String> {
        if let Some(d) = self.fsh_path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        std::fs::write(&self.fsh_path, &self.fsh).map_err(|e| format!("{}: {e}", self.fsh_path.display()))?;
        // the vertex shader is optional: only written when it exists or isn't the default one
        if self.vsh_existed || self.vsh != DEFAULT_VSH {
            std::fs::write(&self.vsh_path, &self.vsh).map_err(|e| format!("{}: {e}", self.vsh_path.display()))?;
            self.vsh_existed = true;
        }
        self.saved = (self.vsh.clone(), self.fsh.clone());
        Ok(())
    }

    /// Static problems of the current text: (stage, line, message).
    pub fn problems(&self) -> Vec<(Stage, u32, String)> {
        let mut out: Vec<(Stage, u32, String)> = vec![];
        for (st, src) in [(Stage::Vertex, &self.vsh), (Stage::Fragment, &self.fsh)] {
            out.extend(validate(src, st).into_iter().map(|d| (st, d.line, d.msg)));
        }
        out.extend(link_problems(&self.vsh, &self.fsh).into_iter().map(|m| (Stage::Fragment, 0, m)));
        out
    }

    fn run_tool(cx: &Cx, args: Vec<String>) -> Result<Receiver<(bool, Vec<String>)>, String> {
        let exe = crate::fx::modengine_exe().ok_or("no encuentro el ejecutable `modengine` junto al editor (cargo build -p modengine-cli)")?;
        let dir = cx.project.dir.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let out = crate::proc::command(exe).arg("shader").arg(&dir).args(&args).stdin(Stdio::null()).output();
            let r = match out {
                Ok(o) => {
                    let mut lines: Vec<String> = String::from_utf8_lossy(&o.stdout).lines().map(String::from).collect();
                    lines.extend(String::from_utf8_lossy(&o.stderr).lines().filter(|l| !l.trim().is_empty()).map(String::from));
                    (o.status.success(), lines)
                }
                Err(e) => (false, vec![format!("no se pudo lanzar modengine: {e}")]),
            };
            let _ = tx.send(r);
        });
        Ok(rx)
    }

    fn compile_gpu(&mut self, cx: &mut Cx) {
        if let Err(e) = self.save() {
            return cx.error(e);
        }
        match Self::run_tool(cx, vec![self.name.clone(), "--check".into()]) {
            Ok(rx) => {
                self.gpu = Some(rx);
                self.gpu_out = None;
            }
            Err(e) => cx.error(e),
        }
    }

    fn render_preview(&mut self, cx: &mut Cx) {
        if let Err(e) = self.save() {
            return cx.error(e);
        }
        let Some(exe) = crate::fx::modengine_exe() else {
            return cx.error("no encuentro el ejecutable `modengine` junto al editor");
        };
        let dir = cx.project.dir.clone();
        let out = std::env::temp_dir().join(format!("modeditor_shader_{}_{}.png", self.name, std::process::id()));
        let mut args = vec!["shader".to_string(), dir.display().to_string(), self.name.clone(), "--screenshot".into(), out.display().to_string(), "--frames".into(), "20".into()];
        if let Some(s) = &self.sprite {
            args.extend(["--sprite".into(), s.clone()]);
        }
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let r = crate::proc::command(exe).args(&args).stdin(Stdio::null()).output().map_err(|e| e.to_string()).and_then(|o| {
                let text = String::from_utf8_lossy(&o.stdout).to_string();
                if text.contains("error") && !out.exists() {
                    return Err(text);
                }
                let img = image::open(&out).map_err(|e| format!("{e}\n{text}"))?.to_rgba8();
                let _ = std::fs::remove_file(&out);
                Ok(egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw()))
            });
            let _ = tx.send(r);
        });
        self.shot = Some(rx);
    }

    fn live_preview(&mut self, cx: &mut Cx) {
        if let Err(e) = self.save() {
            return cx.error(e);
        }
        if let Some(mut c) = self.live.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        let Some(exe) = crate::fx::modengine_exe() else {
            return cx.error("no encuentro el ejecutable `modengine` junto al editor");
        };
        let mut cmd = crate::proc::command(exe);
        cmd.arg("shader").arg(&cx.project.dir).arg(&self.name).stdin(Stdio::null()).stdout(Stdio::null());
        if let Some(s) = &self.sprite {
            cmd.args(["--sprite", s]);
        }
        match cmd.spawn() {
            Ok(c) => {
                self.live = Some(c);
                cx.log.push(LogLine { kind: LogKind::Info, text: format!("vista previa en vivo de '{}': se recarga cada vez que guardas", self.name) });
            }
            Err(e) => cx.error(format!("no se pudo lanzar la vista previa: {e}")),
        }
    }

    fn poll(&mut self, ctx: &egui::Context, cx: &mut Cx) {
        if let Some(rx) = &self.gpu {
            match rx.try_recv() {
                Ok((ok, lines)) => {
                    for l in &lines {
                        let kind = if l.starts_with("ok ") {
                            LogKind::Ok
                        } else if l.contains("error") {
                            LogKind::Error
                        } else {
                            LogKind::Info
                        };
                        cx.log.push(LogLine { kind, text: format!("shader {}: {l}", self.name) });
                    }
                    self.gpu_out = Some((ok, lines));
                    self.gpu = None;
                }
                Err(_) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
            }
        }
        if let Some(rx) = &self.shot {
            match rx.try_recv() {
                Ok(Ok(img)) => {
                    self.preview = Some(ctx.load_texture(format!("shader_preview_{}", self.name), img, egui::TextureOptions::LINEAR));
                    self.preview_err = None;
                    self.shot = None;
                }
                Ok(Err(e)) => {
                    self.preview_err = Some(e);
                    self.shot = None;
                }
                Err(_) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
            }
        }
        if let Some(c) = &mut self.live {
            if let Ok(Some(_)) = c.try_wait() {
                self.live = None;
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        self.poll(ui.ctx(), cx);
        let pal = crate::theme::colors(ui.ctx());
        let problems = self.problems();
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.stage, Stage::Fragment, RichText::new("Fragment (.fsh)").strong());
            ui.selectable_value(&mut self.stage, Stage::Vertex, RichText::new("Vertex (.vsh)").strong());
            ui.separator();
            let busy = self.gpu.is_some();
            if ui.add_enabled(!busy, egui::Button::new(if busy { concat!(fa!(hourglass_half), " Compilando…") } else { concat!(fa!(gear), " Compilar en GPU") })).on_hover_text("guarda y compila el shader de verdad con OpenGL (modengine shader --check)").clicked() {
                self.compile_gpu(cx);
            }
            if ui.add_enabled(self.shot.is_none(), egui::Button::new(concat!(fa!(camera), " Vista previa"))).on_hover_text("guarda y renderiza el shader sobre el sprite elegido").clicked() {
                self.render_preview(cx);
            }
            let live = self.live.is_some();
            if ui.button(if live { concat!(fa!(arrows_rotate), " Reiniciar vista en vivo") } else { concat!(fa!(play), " Vista en vivo") }).on_hover_text("abre una ventana animada que se recarga cada vez que guardas").clicked() {
                self.live_preview(cx);
            }
            ui.label("sprite");
            let sel = self.sprite.clone().unwrap_or_else(|| "(imagen de prueba)".into());
            egui::ComboBox::from_id_salt(("sh_sprite", &self.name)).selected_text(sel).show_ui(ui, |ui| {
                ui.selectable_value(&mut self.sprite, None, "(imagen de prueba)");
                for n in cx.project.sprite_names() {
                    ui.selectable_value(&mut self.sprite, Some(n.clone()), n);
                }
            });
        });
        egui::Panel::right(("sh_side", &self.name)).resizable(true).default_size(300.0).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.strong("Comprobación");
                if problems.is_empty() {
                    ui.colored_label(pal.ok, concat!(fa!(check), " sin errores de sintaxis"));
                }
                for (st, line, msg) in &problems {
                    let tag = if *st == Stage::Vertex { "vsh" } else { "fsh" };
                    let r = ui.colored_label(pal.error, if *line > 0 { format!("{tag}:{line}: {msg}") } else { msg.clone() });
                    if r.clicked() {
                        self.stage = *st;
                    }
                }
                if let Some((ok, lines)) = &self.gpu_out {
                    ui.add_space(4.0);
                    ui.strong("GPU");
                    if *ok {
                        ui.colored_label(pal.ok, concat!(fa!(check), " compila en la GPU"));
                    }
                    for l in lines.iter().filter(|l| !l.starts_with("ok ")) {
                        ui.colored_label(if l.contains("error") { pal.error } else { pal.info }, RichText::new(l).monospace().size(11.5));
                    }
                }
                ui.add_space(8.0);
                ui.strong("Vista previa");
                if let Some(t) = &self.preview {
                    let w = ui.available_width().min(300.0);
                    let s = t.size_vec2();
                    ui.image(egui::load::SizedTexture::new(t.id(), Vec2::new(w, w * s.y / s.x.max(1.0))));
                } else if self.shot.is_some() {
                    ui.label("renderizando…");
                } else {
                    ui.label(RichText::new(concat!("Pulsa ", fa!(camera), " Vista previa para renderizar el shader (o ", fa!(play), " Vista en vivo para verlo animado).")).weak());
                }
                if let Some(e) = &self.preview_err {
                    ui.colored_label(pal.error, e);
                }
                ui.add_space(8.0);
                ui.strong("Uniforms");
                let mut all = parse_uniforms(&self.vsh);
                for u in parse_uniforms(&self.fsh) {
                    if !all.iter().any(|x| x.name == u.name) {
                        all.push(u);
                    }
                }
                if all.is_empty() {
                    ui.label(RichText::new("ninguno (declara `uniform float u_time;`)").weak());
                }
                for u in &all {
                    let (ty, setter) = match u.ty {
                        UType::Float(n) => (if n == 1 { "float".to_string() } else { format!("vec{n}") }, Some("shader_set_uniform_f")),
                        UType::Int(n) => (if n == 1 { "int".to_string() } else { format!("ivec{n}") }, Some("shader_set_uniform_i")),
                        UType::Mat4 => ("mat4".into(), Some("shader_set_uniform_matrix")),
                        UType::Sampler => ("sampler2D".into(), None),
                    };
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("{} {}{}", ty, u.name, if u.count > 1 { format!("[{}]", u.count) } else { String::new() })).monospace());
                        let code = match setter {
                            Some(f) => {
                                let args = (0..u.floats()).map(|k| format!("v{k}")).collect::<Vec<_>>().join(", ");
                                format!("{f}(shader_get_uniform({}, \"{}\"), {args})", self.name, u.name)
                            }
                            None => format!("texture_set_stage(shader_get_sampler_index({}, \"{}\"), sprite)", self.name, u.name),
                        };
                        if ui.small_button(fa!(copy)).on_hover_text(format!("copiar: {code}")).clicked() {
                            ui.ctx().copy_text(code);
                        }
                    });
                }
                ui.add_space(8.0);
                ui.label(
                    RichText::new(format!(
                        "GLSL ES 1.00 al estilo GameMaker: in_Position, in_Colour, in_TextureCoord, gm_Matrices[MATRIX_WORLD_VIEW_PROJECTION], gm_BaseTexture.\n\nEn ModScript:\nshader_set({0})\n…dibujar…\nshader_reset()",
                        self.name
                    ))
                    .weak()
                    .size(11.5),
                );
            });
        });
        egui::CentralPanel::default().show(ui, |ui| {
            let bad: HashSet<u32> = problems.iter().filter(|p| p.0 == self.stage).map(|p| p.1).collect();
            let text = match self.stage {
                Stage::Vertex => &mut self.vsh,
                Stage::Fragment => &mut self.fsh,
            };
            let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
                let mut job = highlight_glsl(buf.as_str(), FontId::monospace(13.5), ui.visuals().dark_mode, &bad);
                job.wrap.max_width = wrap;
                ui.ctx().fonts_mut(|f| f.layout_job(job))
            };
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(text)
                        .id_salt(("shader_code", &self.name, self.stage == Stage::Vertex))
                        .code_editor()
                        .desired_rows(30)
                        .desired_width(f32::INFINITY)
                        .lock_focus(true)
                        .layouter(&mut layouter),
                );
            });
        });
    }

    /// The shader as the engine would load it from the editor's current text.
    #[cfg(test)]
    pub fn asset(&self) -> modengine::shader::ShaderAsset {
        modengine::shader::ShaderAsset::new(&self.name, &self.vsh, &self.fsh)
    }
}

impl Drop for ShaderEditor {
    fn drop(&mut self) {
        if let Some(mut c) = self.live.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlights_and_marks_bad_lines() {
        let bad: HashSet<u32> = [2].into_iter().collect();
        let job = highlight_glsl("uniform float t;\nvoid main() { gl_FragColor = vec4(t) }\n", FontId::monospace(12.0), true, &bad);
        let slice = |r: &std::ops::Range<eframe::egui::epaint::text::ByteIndex>| &job.text[r.start.0..r.end.0];
        let text: String = job.sections.iter().map(|s| slice(&s.byte_range)).collect();
        assert!(text.starts_with("uniform float t;"));
        let red = job.sections.iter().filter(|s| s.format.background != Color32::TRANSPARENT).map(|s| slice(&s.byte_range)).collect::<String>();
        assert!(red.contains("gl_FragColor") && !red.contains("uniform"), "{red}");
    }

    #[test]
    fn creates_edits_checks_and_saves() {
        let dir = crate::room::tests::temp_project("shader_ed");
        let mut p = crate::project::Project::open(&dir).unwrap();
        crate::fx::create(&mut p, crate::fx::FxKind::Shader, "sh_test").unwrap();
        let files = p.fx.shader("sh_test").unwrap().clone();
        let mut ed = ShaderEditor::open(&files).unwrap();
        assert!(ed.problems().is_empty(), "{:?}", ed.problems());
        ed.fsh = "varying vec2 v_vTexcoord;\nvarying vec4 v_vColour;\nuniform float u_k;\nvoid main() {\n    gl_FragColor = v_vColour * u_k\n}\n".into();
        let probs = ed.problems();
        assert_eq!(probs.len(), 1);
        assert_eq!((probs[0].0, probs[0].1), (Stage::Fragment, 5));
        assert!(ed.dirty());
        ed.fsh = ed.fsh.replace("u_k\n", "u_k;\n");
        ed.save().unwrap();
        assert!(!ed.dirty());
        assert_eq!(ed.asset().uniforms[0].name, "u_k");
        let loaded = modengine::shader::load_assets(&dir).unwrap();
        assert!(loaded.iter().any(|s| s.name == "sh_test" && s.fsh.contains("u_k;")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
