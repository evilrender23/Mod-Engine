//! Effect assets in the editor: particle systems, shaders and extensions. Shared pieces: the
//! kinds, their file lists, creating/deleting them and the tab that wraps the three editors.

use crate::app::{Cx, Open};
use crate::project::Project;
use eframe::egui::{self, Vec2};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FxKind {
    Particles,
    Shader,
    Extension,
    /// The string tables of `lang/` (loc_editor.rs), one "asset" for the whole table.
    Language,
}

impl FxKind {
    /// Prefix used by `--open particles:name` and saved layouts.
    pub fn key(self) -> &'static str {
        match self {
            FxKind::Particles => "particles",
            FxKind::Shader => "shader",
            FxKind::Extension => "extension",
            FxKind::Language => "lang",
        }
    }
    pub fn parse(k: &str) -> Option<FxKind> {
        Some(match k {
            "particles" | "particle" | "partsys" => FxKind::Particles,
            "shader" => FxKind::Shader,
            "extension" => FxKind::Extension,
            "lang" | "language" | "idiomas" => FxKind::Language,
            _ => return None,
        })
    }
    pub fn folder(self) -> &'static str {
        match self {
            FxKind::Particles => "particles",
            FxKind::Shader => "shaders",
            FxKind::Extension => "extensions",
            FxKind::Language => "lang",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            FxKind::Particles => "Sistema de partículas",
            FxKind::Shader => "Shader",
            FxKind::Extension => "Extensión",
            FxKind::Language => "Idioma",
        }
    }
    pub fn icon(self) -> &'static str {
        match self {
            FxKind::Particles => "✨",
            FxKind::Shader => "🌈",
            FxKind::Extension => "🧩",
            FxKind::Language => "🌐",
        }
    }
    /// Asset browser section key (custom order).
    pub fn section(self) -> &'static str {
        self.folder()
    }
}

/// A shader's two files (either may not exist yet: the engine uses a passthrough then).
#[derive(Clone, Debug)]
pub struct ShaderFiles {
    pub name: String,
    pub vsh: PathBuf,
    pub fsh: PathBuf,
}

#[derive(Default)]
pub struct Lists {
    /// particles/*.toml: (name, path)
    pub particles: Vec<(String, PathBuf)>,
    pub shaders: Vec<ShaderFiles>,
    /// extensions/<name>/: (name, folder)
    pub extensions: Vec<(String, PathBuf)>,
    /// lang/*.toml: (language code, path)
    pub langs: Vec<(String, PathBuf)>,
}

impl Lists {
    pub fn load(dir: &Path) -> Lists {
        let particles = modengine::project::asset_files(&dir.join("particles"), &[]).into_iter().filter(|(_, _, t)| *t).map(|(n, p, _)| (n, p)).collect();
        let shaders = modengine::shader::load_assets(dir)
            .unwrap_or_default()
            .into_iter()
            .map(|s| {
                let base = s.fsh_path.clone().or(s.vsh_path.clone()).unwrap_or_else(|| dir.join("shaders").join(&s.name));
                ShaderFiles { vsh: s.vsh_path.unwrap_or_else(|| base.with_extension("vsh")), fsh: s.fsh_path.unwrap_or_else(|| base.with_extension("fsh")), name: s.name }
            })
            .collect();
        let extensions = match std::fs::read_dir(dir.join("extensions")) {
            Ok(rd) => {
                let mut v: Vec<(String, PathBuf)> = rd
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| p.is_dir())
                    .filter_map(|p| Some((p.file_name()?.to_string_lossy().to_string(), p)))
                    .filter(|(n, _)| !n.starts_with('.'))
                    .collect();
                v.sort();
                v
            }
            Err(_) => vec![],
        };
        Lists { particles, shaders, extensions, langs: crate::loc_editor::langs_of(dir) }
    }

    pub fn names(&self, k: FxKind) -> Vec<String> {
        match k {
            FxKind::Particles => self.particles.iter().map(|x| x.0.clone()).collect(),
            FxKind::Shader => self.shaders.iter().map(|x| x.name.clone()).collect(),
            FxKind::Extension => self.extensions.iter().map(|x| x.0.clone()).collect(),
            FxKind::Language => {
                if self.langs.is_empty() {
                    vec![]
                } else {
                    vec![crate::loc_editor::TABLE_NAME.to_string()]
                }
            }
        }
    }

    /// Names that live in the game's asset namespace (extensions don't).
    pub fn asset_names(&self) -> impl Iterator<Item = String> + '_ {
        self.names(FxKind::Particles).into_iter().chain(self.names(FxKind::Shader))
    }

    /// The asset's main file (the extension's folder, the shader's .fsh).
    pub fn path(&self, k: FxKind, name: &str) -> Option<PathBuf> {
        match k {
            FxKind::Particles => self.particles.iter().find(|x| x.0 == name).map(|x| x.1.clone()),
            FxKind::Shader => self.shaders.iter().find(|x| x.name == name).map(|x| x.fsh.clone()),
            FxKind::Extension => self.extensions.iter().find(|x| x.0 == name).map(|x| x.1.clone()),
            FxKind::Language => self.langs.first().and_then(|l| l.1.parent()).map(|d| d.to_path_buf()),
        }
    }

    pub fn shader(&self, name: &str) -> Option<&ShaderFiles> {
        self.shaders.iter().find(|s| s.name == name)
    }
}

fn rows(p: &Project, k: FxKind) -> Vec<crate::kinds::Row> {
    p.fx.names(k).into_iter().map(|n| crate::kinds::Row::plain(&n, Open::Fx(k, n.clone()))).collect()
}
pub fn particle_rows(p: &Project) -> Vec<crate::kinds::Row> {
    rows(p, FxKind::Particles)
}
pub fn shader_rows(p: &Project) -> Vec<crate::kinds::Row> {
    rows(p, FxKind::Shader)
}
pub fn extension_rows(p: &Project) -> Vec<crate::kinds::Row> {
    rows(p, FxKind::Extension)
}
/// One row per language, all opening the same keys × languages table.
pub fn language_rows(p: &Project) -> Vec<crate::kinds::Row> {
    let what = Open::Fx(FxKind::Language, crate::loc_editor::TABLE_NAME.into());
    p.fx.langs.iter().map(|(code, _)| crate::kinds::Row { name: code.clone(), label: code.clone(), sprite: None, what: what.clone() }).collect()
}

/// The `modengine` executable next to the editor's.
pub fn modengine_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.with_file_name(if cfg!(windows) { "modengine.exe" } else { "modengine" });
    exe.exists().then_some(exe)
}

/// Inspector details.
pub fn inspector_rows(p: &Project, what: &Open) -> Vec<(&'static str, String)> {
    let Open::Fx(k, n) = what else { return vec![] };
    let rel = |path: &Path| path.strip_prefix(&p.dir).unwrap_or(path).display().to_string();
    let mut out = vec![];
    match k {
        FxKind::Particles => {
            let Some(path) = p.fx.path(*k, n) else { return out };
            out.push(("fichero", rel(&path)));
            match modengine::particles::PartSysToml::parse(&std::fs::read_to_string(&path).unwrap_or_default()) {
                Ok(t) => {
                    out.push(("emisores", t.emitters.len().to_string()));
                    out.push(("tipos", t.types.iter().map(|t| t.name.clone()).collect::<Vec<_>>().join(", ")));
                    out.push(("profundidad", modscript::fmt_num(t.depth)));
                }
                Err(e) => out.push(("error", e)),
            }
        }
        FxKind::Shader => {
            let Some(s) = p.fx.shader(n) else { return out };
            for (label, f) in [("vertex", &s.vsh), ("fragment", &s.fsh)] {
                out.push((label, if f.exists() { rel(f) } else { "(por defecto)".into() }));
            }
            let a = modengine::shader::ShaderAsset::new(
                n,
                &std::fs::read_to_string(&s.vsh).unwrap_or_else(|_| modengine::shader::DEFAULT_VSH.into()),
                &std::fs::read_to_string(&s.fsh).unwrap_or_else(|_| modengine::shader::DEFAULT_FSH.into()),
            );
            out.push(("uniforms", if a.uniforms.is_empty() { "—".into() } else { a.uniforms.iter().map(|u| u.name.clone()).collect::<Vec<_>>().join(", ") }));
            let probs = a.problems();
            out.push(("comprobación", if probs.is_empty() { "✔ sin errores".into() } else { probs[0].clone() }));
        }
        FxKind::Language => out = crate::loc_editor::inspector_rows(&p.dir),
        FxKind::Extension => {
            let Some(dir) = p.fx.path(*k, n) else { return out };
            match modengine::extension::load_one(&p.dir, &dir) {
                Ok(x) => {
                    out.push(("carpeta", rel(&dir)));
                    out.push(("versión", x.meta.version.clone()));
                    if !x.meta.author.is_empty() {
                        out.push(("autor", x.meta.author.clone()));
                    }
                    out.push(("activa", if x.meta.enabled { "sí" } else { "no" }.into()));
                    out.push(("ficheros", x.scripts.len().to_string()));
                    out.push(("funciones", x.meta.functions.len().to_string()));
                }
                Err(e) => out.push(("error", e)),
            }
        }
    }
    out
}

fn write_new(path: &Path, text: &str) -> Result<(), String> {
    if path.exists() {
        return Err(format!("{} ya existe", path.display()));
    }
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// A small but complete starting system: sparks rising from an ellipse.
pub fn particle_template() -> modengine::particles::PartSysToml {
    use modengine::particles::{EmitterToml, PartSysToml, PartTypeToml, Range2};
    PartSysToml {
        types: vec![PartTypeToml {
            name: "chispa".into(),
            shape: "flare".into(),
            size: Range2([0.1, 0.25]),
            size_incr: -0.002,
            speed: Range2([0.6, 1.4]),
            direction: Range2([70.0, 110.0]),
            colors: vec!["#ffcd75".into(), "#ef7d57".into(), "#b13e53".into()],
            alphas: vec![1.0, 0.8, 0.0],
            additive: true,
            life: Range2([30.0, 60.0]),
            ..Default::default()
        }],
        emitters: vec![EmitterToml { name: "base".into(), ty: "chispa".into(), shape: "ellipse".into(), region: [-12.0, -4.0, 12.0, 4.0], count: 2, ..Default::default() }],
        ..Default::default()
    }
}

pub fn create(p: &mut Project, k: FxKind, name: &str) -> Result<(), String> {
    let dir = p.dir.clone();
    match k {
        FxKind::Particles => write_new(&dir.join("particles").join(format!("{name}.toml")), &particle_template().to_toml())?,
        FxKind::Shader => {
            write_new(&dir.join("shaders").join(format!("{name}.vsh")), modengine::shader::DEFAULT_VSH)?;
            write_new(&dir.join("shaders").join(format!("{name}.fsh")), modengine::shader::DEFAULT_FSH)?;
        }
        FxKind::Language => {
            // name = language code: an empty table (every key shows as missing)
            if !crate::loc_editor::valid_lang(name) {
                return Err(format!("'{name}' no vale como código de idioma (es, en, pt-BR...)"));
            }
            write_new(&dir.join("lang").join(format!("{name}.toml")), &format!("# Textos en '{name}': clave = \"texto\" (se editan en Idiomas)\n"))?;
        }
        FxKind::Extension => {
            let folder = dir.join("extensions").join(name);
            if folder.exists() {
                return Err(format!("{} ya existe", folder.display()));
            }
            let meta = modengine::extension::ExtensionToml {
                name: name.into(),
                description: "Funciones reutilizables".into(),
                files: vec![format!("{name}.ms")],
                functions: vec![modengine::extension::ExtFunction { name: format!("{name}_hello"), args: "who".into(), help: "devuelve un saludo".into() }],
                ..Default::default()
            };
            write_new(&folder.join(modengine::extension::META_FILE), &meta.to_toml())?;
            let rel = format!("extensions/{name}/{name}.ms");
            let i = p.add_script(&rel, format!("// Funciones de la extensión {name}: se cargan antes que los scripts del proyecto.\nfunction {name}_hello(who) {{\n    return \"hola, \" + who\n}}\n"));
            p.save_script(i)?;
        }
    }
    p.fx = Lists::load(&dir);
    Ok(())
}

/// Moves the asset's files to `.trash` and reloads the lists.
pub fn delete(p: &mut Project, k: FxKind, name: &str) -> Result<(), String> {
    match k {
        FxKind::Shader => {
            if let Some(s) = p.fx.shader(name).cloned() {
                for f in [s.vsh, s.fsh] {
                    if f.exists() {
                        p.trash(&f)?;
                    }
                }
            }
        }
        _ => {
            if let Some(path) = p.fx.path(k, name) {
                p.trash(&path)?;
            }
            if k == FxKind::Extension {
                // its scripts are gone from disk too
                let prefix = format!("extensions/{name}/");
                p.scripts.retain(|s| !s.rel.starts_with(&prefix));
                p.rescan_objects();
            }
        }
    }
    p.fx = Lists::load(&p.dir);
    Ok(())
}

// ------------------------------------------------------------------ the tab

pub enum FxTab {
    Particles(crate::particles::ParticleEditor),
    Shader(crate::shader::ShaderEditor),
    Extension(crate::extension::ExtensionEditor),
    Language(crate::loc_editor::LangEditor),
}

impl FxTab {
    pub fn open(p: &Project, k: FxKind, name: &str) -> Result<FxTab, Option<String>> {
        Ok(match k {
            FxKind::Particles => {
                let path = p.fx.path(k, name).ok_or(None)?;
                FxTab::Particles(crate::particles::ParticleEditor::open(name, &path).map_err(Some)?)
            }
            FxKind::Shader => {
                let s = p.fx.shader(name).ok_or(None)?;
                FxTab::Shader(crate::shader::ShaderEditor::open(s).map_err(Some)?)
            }
            FxKind::Extension => {
                let dir = p.fx.path(k, name).ok_or(None)?;
                FxTab::Extension(crate::extension::ExtensionEditor::open(name, &dir).map_err(Some)?)
            }
            FxKind::Language => FxTab::Language(crate::loc_editor::LangEditor::open(&p.dir)),
        })
    }
    pub fn kind(&self) -> FxKind {
        match self {
            FxTab::Particles(_) => FxKind::Particles,
            FxTab::Shader(_) => FxKind::Shader,
            FxTab::Extension(_) => FxKind::Extension,
            FxTab::Language(_) => FxKind::Language,
        }
    }
    pub fn name(&self) -> &str {
        match self {
            FxTab::Particles(e) => &e.name,
            FxTab::Shader(e) => &e.name,
            FxTab::Extension(e) => &e.name,
            FxTab::Language(e) => &e.name,
        }
    }
    pub fn title(&self) -> String {
        let k = self.kind();
        format!("{} {}: {}", k.icon(), k.label(), self.name())
    }
    pub fn dirty(&self) -> bool {
        match self {
            FxTab::Particles(e) => e.dirty(),
            FxTab::Shader(e) => e.dirty(),
            FxTab::Extension(e) => e.dirty(),
            FxTab::Language(e) => e.dirty,
        }
    }
    pub fn save(&mut self, cx: &mut Cx) -> Result<(), String> {
        let r = match self {
            FxTab::Particles(e) => e.save(),
            FxTab::Shader(e) => e.save(),
            FxTab::Extension(e) => e.save(),
            FxTab::Language(e) => e.save(cx),
        };
        cx.project.fx = Lists::load(&cx.project.dir);
        r
    }
    pub fn undo(&mut self) {
        if let FxTab::Particles(e) = self {
            e.undo()
        }
    }
    pub fn redo(&mut self) {
        if let FxTab::Particles(e) = self {
            e.redo()
        }
    }
    pub fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        match self {
            FxTab::Particles(e) => e.ui(ui, cx),
            FxTab::Shader(e) => e.ui(ui, cx),
            FxTab::Extension(e) => e.ui(ui, cx),
            FxTab::Language(e) => e.ui(ui, cx),
        }
    }
}

pub fn window_size(k: FxKind) -> Vec2 {
    match k {
        FxKind::Particles => Vec2::new(1000.0, 640.0),
        FxKind::Shader => Vec2::new(980.0, 600.0),
        FxKind::Extension => Vec2::new(860.0, 600.0),
        FxKind::Language => Vec2::new(900.0, 560.0),
    }
}
