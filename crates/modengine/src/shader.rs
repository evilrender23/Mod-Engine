//! Shaders: GLSL ES 1.00 vertex/fragment pairs in `shaders/`, written like GameMaker's
//! (`in_Position`, `in_Colour`, `in_TextureCoord`, `gm_Matrices[MATRIX_WORLD_VIEW_PROJECTION]`,
//! `gm_BaseTexture`, `v_vColour`/`v_vTexcoord` varyings).
//!
//! ```text
//! shaders/sh_wave.vsh   vertex shader   (optional: a passthrough is used when missing)
//! shaders/sh_wave.fsh   fragment shader (optional too)
//! shaders/sh_wave/sh_wave.fsh   GameMaker's folder layout works as well
//! ```
//!
//! The engine can't compile GLSL without a GPU: the backend compiles every shader when the
//! game starts ([`Shaders::status`]) and [`validate`] does a static check for `modengine check`
//! and the editor. [`prepare`] turns the GameMaker-style source into what macroquad expects.

use std::path::{Path, PathBuf};

pub const DEFAULT_VSH: &str = "\
attribute vec3 in_Position;                  // (x,y,z)
attribute vec4 in_Colour;                    // (r,g,b,a)
attribute vec2 in_TextureCoord;              // (u,v)

varying vec2 v_vTexcoord;
varying vec4 v_vColour;

void main()
{
    vec4 object_space_pos = vec4(in_Position.x, in_Position.y, in_Position.z, 1.0);
    gl_Position = gm_Matrices[MATRIX_WORLD_VIEW_PROJECTION] * object_space_pos;
    v_vColour = in_Colour;
    v_vTexcoord = in_TextureCoord;
}
";

pub const DEFAULT_FSH: &str = "\
varying vec2 v_vTexcoord;
varying vec4 v_vColour;

void main()
{
    gl_FragColor = v_vColour * texture2D(gm_BaseTexture, v_vTexcoord);
}
";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stage {
    Vertex,
    Fragment,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UType {
    /// float, vec2, vec3, vec4
    Float(u8),
    /// int/bool, ivec2…
    Int(u8),
    Mat4,
    Sampler,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UniformInfo {
    pub name: String,
    pub ty: UType,
    /// Array length (1 for plain uniforms).
    pub count: usize,
}

impl UniformInfo {
    /// Number of 32-bit values it takes.
    pub fn floats(&self) -> usize {
        self.count
            * match self.ty {
                UType::Float(n) | UType::Int(n) => n as usize,
                UType::Mat4 => 16,
                UType::Sampler => 0,
            }
    }
}

#[derive(Clone, Debug)]
pub struct ShaderAsset {
    pub name: String,
    pub vsh: String,
    pub fsh: String,
    pub vsh_path: Option<PathBuf>,
    pub fsh_path: Option<PathBuf>,
    pub uniforms: Vec<UniformInfo>,
}

impl ShaderAsset {
    pub fn new(name: &str, vsh: &str, fsh: &str) -> Self {
        let mut uniforms = parse_uniforms(vsh);
        for u in parse_uniforms(fsh) {
            if !uniforms.iter().any(|x| x.name == u.name) {
                uniforms.push(u);
            }
        }
        ShaderAsset { name: name.into(), vsh: vsh.into(), fsh: fsh.into(), vsh_path: None, fsh_path: None, uniforms }
    }

    pub fn uniform(&self, name: &str) -> Option<&UniformInfo> {
        self.uniforms.iter().find(|u| u.name == name)
    }

    /// Static problems of both stages, formatted `vsh:LINE: msg` / `fsh:LINE: msg`.
    pub fn problems(&self) -> Vec<String> {
        let mut out = vec![];
        for (tag, src, st) in [("vsh", &self.vsh, Stage::Vertex), ("fsh", &self.fsh, Stage::Fragment)] {
            for d in validate(src, st) {
                out.push(format!("{tag}:{}: {}", d.line, d.msg));
            }
        }
        out.extend(link_problems(&self.vsh, &self.fsh));
        out
    }
}

/// `shaders/` of a project. A shader is any name with a `.vsh` and/or `.fsh` file, either
/// directly in `shaders/` or in `shaders/<name>/`.
pub fn load_assets(dir: &Path) -> Result<Vec<ShaderAsset>, String> {
    let root = dir.join("shaders");
    let mut found: std::collections::BTreeMap<String, (Option<PathBuf>, Option<PathBuf>)> = Default::default();
    let mut scan = |d: &Path| {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let ext = p.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
            let Some(stem) = p.file_stem().map(|s| s.to_string_lossy().to_string()) else { continue };
            let slot = found.entry(stem).or_default();
            match ext.as_str() {
                "vsh" | "vert" => slot.0 = Some(p),
                "fsh" | "frag" => slot.1 = Some(p),
                _ => {}
            }
        }
    };
    scan(&root);
    if let Ok(rd) = std::fs::read_dir(&root) {
        let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        dirs.sort();
        for d in dirs {
            scan(&d);
        }
    }
    let read = |p: &Option<PathBuf>, default: &str| -> Result<String, String> {
        match p {
            Some(p) => std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display())),
            None => Ok(default.to_string()),
        }
    };
    let mut out = vec![];
    for (name, (v, f)) in found {
        if v.is_none() && f.is_none() {
            continue;
        }
        let mut s = ShaderAsset::new(&name, &read(&v, DEFAULT_VSH)?, &read(&f, DEFAULT_FSH)?);
        s.vsh_path = v;
        s.fsh_path = f;
        out.push(s);
    }
    Ok(out)
}

// ------------------------------------------------------------------ source handling

/// Source with comments replaced by spaces (newlines kept, so line numbers stay right).
pub fn strip_comments(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                out.push(' ');
                i += 1;
            }
        } else if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
            out.push_str("  ");
            i += 2;
            while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                out.push(if b[i] == b'\n' { '\n' } else { ' ' });
                i += 1;
            }
            if i < b.len() {
                out.push_str("  ");
                i += 2;
            }
        } else {
            let ch = src[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

fn words(line: &str) -> Vec<&str> {
    line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '[' || c == ']')).filter(|w| !w.is_empty()).collect()
}

/// Uniforms declared in a source (`uniform vec2 u_pos;`, `uniform float a[4];`,
/// `uniform highp float t, k;`). The built-ins the engine provides are left out.
pub fn parse_uniforms(src: &str) -> Vec<UniformInfo> {
    let mut out = vec![];
    let clean = strip_comments(src);
    for stmt in clean.split(';') {
        let s = stmt.trim();
        let Some(rest) = s.strip_prefix("uniform") else { continue };
        if !rest.starts_with(char::is_whitespace) {
            continue;
        }
        let mut toks: Vec<&str> = rest.split_whitespace().collect();
        toks.retain(|t| !matches!(*t, "lowp" | "mediump" | "highp"));
        if toks.len() < 2 {
            continue;
        }
        let ty = match toks[0] {
            "float" => UType::Float(1),
            "vec2" => UType::Float(2),
            "vec3" => UType::Float(3),
            "vec4" => UType::Float(4),
            "int" | "bool" => UType::Int(1),
            "ivec2" | "bvec2" => UType::Int(2),
            "ivec3" | "bvec3" => UType::Int(3),
            "ivec4" | "bvec4" => UType::Int(4),
            "mat4" => UType::Mat4,
            "sampler2D" => UType::Sampler,
            _ => continue,
        };
        for decl in toks[1..].join(" ").split(',') {
            let d = decl.trim();
            let (name, count) = match d.split_once('[') {
                Some((n, c)) => (n.trim(), c.trim_end_matches(']').trim().parse::<usize>().unwrap_or(1)),
                None => (d, 1),
            };
            if name.is_empty() || BUILTIN_UNIFORMS.contains(&name) || out.iter().any(|u: &UniformInfo| u.name == name) {
                continue;
            }
            out.push(UniformInfo { name: name.to_string(), ty, count: count.max(1) });
        }
    }
    out
}

const BUILTIN_UNIFORMS: &[&str] = &["Model", "Projection", "_Time", "Texture", "gm_BaseTexture", "gm_Matrices"];
const BUILTIN_ATTRIBUTES: &[&str] = &["position", "texcoord", "color0", "normal", "in_Position", "in_Colour", "in_Color", "in_TextureCoord", "in_Normal"];

fn varyings(src: &str) -> Vec<String> {
    let clean = strip_comments(src);
    let mut out = vec![];
    for stmt in clean.split(';') {
        let s = stmt.trim();
        let Some(rest) = s.strip_prefix("varying") else { continue };
        let toks: Vec<&str> = rest.split_whitespace().filter(|t| !matches!(*t, "lowp" | "mediump" | "highp")).collect();
        if toks.len() >= 2 {
            for n in toks[1..].join(" ").split(',') {
                out.push(n.trim().split('[').next().unwrap_or("").trim().to_string());
            }
        }
    }
    out
}

/// Turns a GameMaker-style source into the GLSL macroquad compiles. A `#line 1` keeps the
/// driver's error line numbers equal to the user's.
pub fn prepare(src: &str, stage: Stage) -> String {
    let clean = strip_comments(src);
    // drop the declarations the prelude provides, keeping one (empty) line for each
    let mut body = String::new();
    for (orig, c) in src.lines().zip(clean.lines().chain(std::iter::repeat(""))) {
        let w = words(c);
        let drop = match w.first().copied() {
            Some("attribute") => w.iter().any(|x| BUILTIN_ATTRIBUTES.contains(x)),
            Some("uniform") => w.iter().any(|x| BUILTIN_UNIFORMS.contains(x)),
            _ => c.trim_start().starts_with("#version"),
        };
        if drop {
            body.push('\n');
        } else {
            body.push_str(orig);
            body.push('\n');
        }
    }
    match stage {
        Stage::Vertex => format!(
            "#version 100
attribute vec3 position;
attribute vec2 texcoord;
attribute vec4 color0;
attribute vec4 normal;
uniform mat4 Model;
uniform mat4 Projection;
#define in_Position position
#define in_TextureCoord texcoord
#define in_Colour (color0 / 255.0)
#define in_Color (color0 / 255.0)
#define in_Normal (normal.xyz)
#define MATRIX_VIEW 0
#define MATRIX_PROJECTION 1
#define MATRIX_WORLD 2
#define MATRIX_WORLD_VIEW 3
#define MATRIX_WORLD_VIEW_PROJECTION 4
mat4 gm_Matrices[5];
#define main gm_user_main
#line 1
{body}
#undef main
void main() {{
    gm_Matrices[0] = mat4(1.0);
    gm_Matrices[1] = Projection;
    gm_Matrices[2] = Model;
    gm_Matrices[3] = Model;
    gm_Matrices[4] = Projection * Model;
    gm_user_main();
}}
"
        ),
        Stage::Fragment => format!(
            "#version 100
precision mediump float;
uniform sampler2D Texture;
#define gm_BaseTexture Texture
#line 1
{body}"
        ),
    }
}

// ------------------------------------------------------------------ static validation

#[derive(Clone, Debug, PartialEq)]
pub struct Diag {
    pub line: u32,
    pub msg: String,
}

fn line_of(src: &str, byte: usize) -> u32 {
    src[..byte.min(src.len())].matches('\n').count() as u32 + 1
}

/// Checks a GLSL ES 1.00 source without a GPU: brackets, `main`, GLSL 3 syntax that GL ES 2
/// rejects, and what each stage must write. Real compilation happens when the game starts.
pub fn validate(src: &str, stage: Stage) -> Vec<Diag> {
    let mut out = vec![];
    let clean = strip_comments(src);
    let mut stack: Vec<(char, usize)> = vec![];
    for (i, ch) in clean.char_indices() {
        match ch {
            '(' | '{' | '[' => stack.push((ch, i)),
            ')' | '}' | ']' => {
                let want = match ch {
                    ')' => '(',
                    '}' => '{',
                    _ => '[',
                };
                match stack.pop() {
                    Some((o, _)) if o == want => {}
                    Some((o, at)) => {
                        out.push(Diag { line: line_of(&clean, i), msg: format!("'{ch}' cierra un '{o}' abierto en la línea {}", line_of(&clean, at)) });
                        return out;
                    }
                    None => {
                        out.push(Diag { line: line_of(&clean, i), msg: format!("'{ch}' sin abrir") });
                        return out;
                    }
                }
            }
            _ => {}
        }
    }
    if let Some((o, at)) = stack.pop() {
        out.push(Diag { line: line_of(&clean, at), msg: format!("'{o}' sin cerrar") });
        return out;
    }
    let has = |w: &str| {
        clean.match_indices(w).find(|(i, _)| {
            let before = clean[..*i].chars().last().is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'));
            let after = clean[i + w.len()..].chars().next().is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'));
            before && after
        })
    };
    match has("main") {
        Some(_) if clean.contains("void main") || clean.split_whitespace().collect::<Vec<_>>().windows(2).any(|w| w[0] == "void" && w[1].starts_with("main")) => {}
        _ => out.push(Diag { line: 1, msg: "falta la función `void main()`".into() }),
    }
    // GLSL 3 globals (`in vec2 uv;`, `out vec4 color;`); `in`/`out` stay valid as parameter qualifiers
    for (n, l) in clean.lines().enumerate() {
        let t = l.trim_start();
        for (q, msg) in [
            ("out ", "GLSL ES 1.00 no tiene variables `out`: usa `varying` (y gl_FragColor en el fragment shader)"),
            ("in ", "GLSL ES 1.00 no tiene variables `in`: usa `attribute` o `varying`"),
        ] {
            if t.starts_with(q) && t.trim_end().ends_with(';') && !t.contains('(') {
                out.push(Diag { line: n as u32 + 1, msg: msg.into() });
            }
        }
    }
    for (word, msg) in [("texture", "GLSL ES 1.00 usa texture2D(), no texture()"), ("layout", "GLSL ES 1.00 no admite `layout(...)`")] {
        if let Some((i, _)) = has(word) {
            // `texture` is only wrong as a call
            if word == "texture" && !clean[i + word.len()..].trim_start().starts_with('(') {
                continue;
            }
            out.push(Diag { line: line_of(&clean, i), msg: msg.into() });
        }
    }
    match stage {
        Stage::Vertex => {
            if has("gl_Position").is_none() {
                out.push(Diag { line: 1, msg: "el vertex shader debe escribir gl_Position".into() });
            }
            if let Some((i, _)) = has("gl_FragColor") {
                out.push(Diag { line: line_of(&clean, i), msg: "gl_FragColor solo existe en el fragment shader".into() });
            }
        }
        Stage::Fragment => {
            if has("gl_FragColor").is_none() && has("gl_FragData").is_none() {
                out.push(Diag { line: 1, msg: "el fragment shader debe escribir gl_FragColor".into() });
            }
            if let Some((i, _)) = has("attribute") {
                out.push(Diag { line: line_of(&clean, i), msg: "`attribute` solo se puede usar en el vertex shader".into() });
            }
        }
    }
    // statements must end with ';' before a closing brace (on the same line or the previous one)
    let lines: Vec<&str> = clean.lines().collect();
    for (n, l) in lines.iter().enumerate() {
        let t = l.trim();
        if !t.ends_with('}') {
            continue;
        }
        let before = t[..t.len() - 1].trim_end();
        let (prev, at) = if before.is_empty() {
            match lines[..n].iter().enumerate().rev().find(|(_, x)| !x.trim().is_empty()) {
                Some((k, x)) => (x.trim(), k),
                None => continue,
            }
        } else {
            (before, n)
        };
        if !prev.ends_with([';', '{', '}']) && !prev.starts_with('#') {
            out.push(Diag { line: at as u32 + 1, msg: "falta ';' al final de la instrucción".into() });
        }
    }
    out
}

/// A varying read by the fragment shader must be written by the vertex shader.
pub fn link_problems(vsh: &str, fsh: &str) -> Vec<String> {
    let v = varyings(vsh);
    varyings(fsh).into_iter().filter(|n| !n.is_empty() && !v.contains(n)).map(|n| format!("fsh: el varying '{n}' no está declarado en el vertex shader")).collect()
}

// ------------------------------------------------------------------ engine state

/// Shaders of a running game.
#[derive(Clone, Debug, Default)]
pub struct Shaders {
    pub list: Vec<ShaderAsset>,
    /// Set by the backend after compiling: `Some(Err(msg))` when the GPU rejected it. `None`
    /// without a GPU (headless runs), where only the static check counts.
    pub status: Vec<Option<Result<(), String>>>,
    /// Uniform handles returned by `shader_get_uniform`: (shader, uniform name).
    pub handles: Vec<(u32, String)>,
    /// Shader set by `shader_set` (None = default).
    pub current: Option<u32>,
    /// Blend mode set by `gpu_set_blendmode` (0 normal, 1 add, 2 subtract, 3 max).
    pub blend: u8,
}

impl Shaders {
    pub fn new(list: Vec<ShaderAsset>) -> Self {
        let n = list.len();
        Shaders { list, status: vec![None; n], handles: vec![], current: None, blend: 0 }
    }

    pub fn is_compiled(&self, i: u32) -> bool {
        match self.status.get(i as usize) {
            Some(Some(r)) => r.is_ok(),
            Some(None) => self.list[i as usize].problems().is_empty(),
            None => false,
        }
    }

    /// Handle for a uniform (or a sampler), or -1 when the shader doesn't declare it.
    pub fn handle(&mut self, sh: u32, name: &str) -> i64 {
        let Some(s) = self.list.get(sh as usize) else { return -1 };
        if s.uniform(name).is_none() {
            return -1;
        }
        if let Some(i) = self.handles.iter().position(|(a, b)| *a == sh && b == name) {
            return i as i64;
        }
        self.handles.push((sh, name.to_string()));
        self.handles.len() as i64 - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        assert!(validate(DEFAULT_VSH, Stage::Vertex).is_empty(), "{:?}", validate(DEFAULT_VSH, Stage::Vertex));
        assert!(validate(DEFAULT_FSH, Stage::Fragment).is_empty());
        assert!(link_problems(DEFAULT_VSH, DEFAULT_FSH).is_empty());
    }

    #[test]
    fn finds_uniforms() {
        let u = parse_uniforms("uniform float u_time; // t\nuniform highp vec2 u_res, u_mouse;\nuniform float k[3];\nuniform sampler2D u_pal;\nuniform sampler2D gm_BaseTexture;");
        let names: Vec<&str> = u.iter().map(|u| u.name.as_str()).collect();
        assert_eq!(names, ["u_time", "u_res", "u_mouse", "k", "u_pal"]);
        assert_eq!(u[1].ty, UType::Float(2));
        assert_eq!(u[3].count, 3);
        assert_eq!(u[3].floats(), 3);
        assert_eq!(u[4].ty, UType::Sampler);
    }

    #[test]
    fn reports_errors_with_lines() {
        let d = validate("void main() {\n  gl_FragColor = vec4(1.0)\n}\n", Stage::Fragment);
        assert_eq!(d, vec![Diag { line: 2, msg: "falta ';' al final de la instrucción".into() }]);
        let d = validate("void main() { gl_FragColor = vec4(1.0) }", Stage::Fragment);
        assert_eq!(d.len(), 1);
        let d = validate("void main() {\n  gl_FragColor = texture(gm_BaseTexture, v);\n", Stage::Fragment);
        assert_eq!(d[0].msg, "'{' sin cerrar");
        let d = validate("out vec4 c;\nvoid main() { c = texture(t, v); }", Stage::Fragment);
        assert!(d.iter().any(|d| d.line == 1 && d.msg.contains("out")));
        assert!(d.iter().any(|d| d.line == 2 && d.msg.contains("texture2D")));
        assert!(d.iter().any(|d| d.msg.contains("gl_FragColor")));
        assert!(link_problems(DEFAULT_VSH, "varying vec2 v_other;\nvoid main(){gl_FragColor=vec4(1.0);}")[0].contains("v_other"));
    }

    #[test]
    fn prepare_keeps_line_numbers() {
        let p = prepare(DEFAULT_VSH, Stage::Vertex);
        assert!(p.starts_with("#version 100\n"));
        assert!(p.contains("#line 1\n"));
        let after = p.split("#line 1\n").nth(1).unwrap();
        // the attribute lines are blanked, not removed
        assert_eq!(after.lines().nth(4).unwrap(), "varying vec2 v_vTexcoord;");
        assert!(!after.contains("attribute vec3 in_Position"));
        let f = prepare("#version 100\nuniform sampler2D gm_BaseTexture;\nvoid main(){}", Stage::Fragment);
        assert_eq!(f.matches("#version").count(), 1);
        assert_eq!(f.matches("uniform sampler2D").count(), 1);
    }
}
