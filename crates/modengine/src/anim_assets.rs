//! Loading of the motion assets (paths, timelines, animation curves and sequences)
//! and the per-step update of instances that follow a path.

use crate::animcurve::AnimCurve;
use crate::engine::Engine;
use crate::object::Ev;
use crate::path::PathDef;
use crate::project::asset_files;
use crate::sequence::{SequenceDef, SequenceToml};
use crate::timeline::TimelineDef;
use modscript::{AssetKind, Scope, Value};
use std::path::Path;

/// Folders of each motion asset type inside a project.
pub const FOLDERS: [(&str, AssetKind); 4] = [("paths", AssetKind::Path), ("timelines", AssetKind::Timeline), ("curves", AssetKind::AnimCurve), ("sequences", AssetKind::Sequence)];

#[derive(Default)]
pub struct Loaded {
    pub paths: Vec<PathDef>,
    pub timelines: Vec<TimelineDef>,
    pub curves: Vec<AnimCurve>,
    /// Sequences are resolved later, once objects exist: (name, file, data).
    pub sequences: Vec<(String, String, SequenceToml)>,
}

fn tomls(dir: &Path, folder: &str) -> Vec<(String, std::path::PathBuf, String)> {
    asset_files(&dir.join(folder), &[]).into_iter().filter(|(_, _, is_toml)| *is_toml).map(|(n, p, _)| (n.clone(), p, format!("{folder}/{n}.toml"))).collect()
}

/// Reads `paths/`, `timelines/`, `curves/` and `sequences/` (all optional).
pub fn load(dir: &Path, errors: &mut Vec<String>) -> Loaded {
    let mut l = Loaded::default();
    for (name, path, _) in tomls(dir, "paths") {
        match PathDef::load(&name, &path) {
            Ok(p) => l.paths.push(p),
            Err(e) => errors.push(e),
        }
    }
    for (name, path, rel) in tomls(dir, "timelines") {
        match TimelineDef::load(&name, &path, &rel) {
            Ok(t) => l.timelines.push(t),
            Err(e) => errors.push(e),
        }
    }
    for (name, path, _) in tomls(dir, "curves") {
        match AnimCurve::load(&name, &path) {
            Ok(c) => l.curves.push(c),
            Err(e) => errors.push(e),
        }
    }
    for (name, path, rel) in tomls(dir, "sequences") {
        match SequenceDef::load(&name, &path, &rel) {
            Ok((n, t)) => l.sequences.push((n, rel, t)),
            Err(e) => errors.push(e),
        }
    }
    l
}

/// Every asset name with its script value, for the engine's asset table.
pub fn names(l: &Loaded) -> Vec<(String, Value)> {
    let mut v = vec![];
    v.extend(l.paths.iter().enumerate().map(|(i, p)| (p.name.clone(), Value::Asset(AssetKind::Path, i as u32))));
    v.extend(l.timelines.iter().enumerate().map(|(i, t)| (t.name.clone(), Value::Asset(AssetKind::Timeline, i as u32))));
    v.extend(l.curves.iter().enumerate().map(|(i, c)| (c.name.clone(), Value::Asset(AssetKind::AnimCurve, i as u32))));
    v.extend(l.sequences.iter().enumerate().map(|(i, s)| (s.0.clone(), Value::Asset(AssetKind::Sequence, i as u32))));
    v
}

/// Resolves the tracks of the loaded sequences (needs sprites, objects and sounds).
pub fn resolve_sequences(e: &mut Engine, raw: Vec<(String, String, SequenceToml)>, errors: &mut Vec<String>) {
    for (name, rel, t) in raw {
        match SequenceDef::resolve(e, &name, &rel, t) {
            Ok(s) => e.sequences.push(s),
            Err(err) => errors.push(err),
        }
    }
}

/// Name of a motion asset (for `asset_name`).
pub fn asset_name(e: &Engine, kind: AssetKind, i: u32) -> String {
    let i = i as usize;
    match kind {
        AssetKind::Path => e.paths.get(i).map(|p| p.name.clone()),
        AssetKind::Timeline => e.timelines.get(i).map(|t| t.name.clone()),
        AssetKind::AnimCurve => e.curves.get(i).map(|c| c.name.clone()),
        AssetKind::Sequence => e.sequences.get(i).map(|s| s.name.clone()),
        _ => None,
    }
    .unwrap_or_default()
}

/// Moves the instances that follow a path (after the normal motion), firing `path_ended`.
pub fn step_paths(e: &mut Engine, ids: &[u32]) {
    let mut ended = vec![];
    for &id in ids {
        let Some(i) = e.idx(id) else { continue };
        let Some(pi) = e.insts[i].path.index.filter(|_| e.insts[i].alive) else { continue };
        let Some(p) = e.paths.get(pi as usize) else { continue };
        if crate::path::advance(&mut e.insts[i], p) {
            ended.push(id);
        }
    }
    for id in ended {
        e.run_event(id, Ev::PathEnded, Scope::Global);
    }
}
