//! The engine: owns every asset, object, room and instance, runs the game
//! loop and implements the script [`Host`].

use crate::draw::{parse_color, DrawCmd, DrawState, Rgba};
use crate::input::{Input, KEY_NAMES, MOUSE_BUTTONS};
use crate::instance::{Bv, Instance, ALARM_COUNT, BUILTIN_VARS};
use crate::natives::{self, NativeFn};
use crate::object::{parse_event, Ev, ObjectDef, ANY_KEY};
use crate::project::{DisplaySection, FontAsset, GameToml, LayerToml, Project, RoomToml, SoundAsset, Sprite};
use modscript::{
    intern, parse_file, parse_stmts, run_def, run_stmts, AssetKind, CallCx, FuncDef, Host, RtError, RtResult, Scope, Stmt,
    Sym, Value,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

// ------------------------------------------------------------------ rooms & layers

#[derive(Clone, Debug)]
pub enum LayerKind {
    Background {
        sprite: Option<u32>,
        color: Option<u32>,
        tile_x: bool,
        tile_y: bool,
        stretch: bool,
        hspeed: f64,
        vspeed: f64,
        parallax: (f64, f64),
        ox: f64,
        oy: f64,
    },
    Tiles {
        /// Sprite whose frames are the tiles.
        tileset: u32,
        /// Tile set asset (animated tiles, `tilemap_get_tileset`); None for a plain sprite.
        set: Option<u32>,
        cw: u32,
        ch: u32,
        grid: Vec<Vec<i32>>,
        ox: f64,
        oy: f64,
    },
    Instances,
}

#[derive(Clone, Debug)]
pub struct Layer {
    pub name: String,
    pub depth: f64,
    pub visible: bool,
    pub alpha: f64,
    pub kind: LayerKind,
}

#[derive(Clone)]
pub struct Spawn {
    pub object: u32,
    pub x: f64,
    pub y: f64,
    pub depth: f64,
    pub vars: Vec<(Sym, Value)>,
    /// Map-placed instances get aligned inside their cell once their sprite is known.
    pub cell: Option<[f64; 4]>,
}

pub struct RoomDef {
    pub name: String,
    pub w: f64,
    pub h: f64,
    pub speed: Option<f64>,
    pub background: Option<u32>,
    pub persistent: bool,
    pub code: Option<Rc<Vec<Stmt>>>,
    pub file: Rc<str>,
    pub layers: Vec<Layer>,
    pub spawns: Vec<Spawn>,
    pub cam_follow: Option<u32>,
    pub cam_smooth: f64,
    pub cam_start: (f64, f64),
}

struct SavedRoom {
    insts: Vec<Instance>,
    layers: Vec<Layer>,
    cam: (f64, f64),
}

#[derive(Clone, Debug)]
pub struct Camera {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub follow: Option<Value>,
    pub smooth: f64,
    pub shake: f64,
    pub shake_steps: u32,
}

#[derive(Clone, Debug)]
pub enum AudioCmd {
    Play { snd: u32, looped: bool, volume: f64 },
    Stop(u32),
    StopAll,
    Volume(u32, f64),
}

struct DynSyms {
    mouse_x: Sym,
    mouse_y: Sym,
    room: Sym,
    room_width: Sym,
    room_height: Sym,
}

pub type MeasureFn = Box<dyn Fn(Option<u32>, &str, f32) -> (f32, f32)>;

pub struct Engine {
    pub dir: PathBuf,
    pub config: GameToml,
    pub sprites: Vec<Sprite>,
    pub sounds: Vec<SoundAsset>,
    pub fonts: Vec<FontAsset>,
    pub tilesets: Vec<crate::tileset::TileSetAsset>,
    pub paths: Vec<crate::path::PathDef>,
    pub timelines: Vec<crate::timeline::TimelineDef>,
    pub curves: Vec<crate::animcurve::AnimCurve>,
    pub sequences: Vec<crate::sequence::SequenceDef>,
    /// Sequences playing in the current room.
    pub seqs: crate::sequence::SeqRuntime,
    pub objects: Vec<ObjectDef>,
    pub rooms: Vec<RoomDef>,
    pub room_order: Vec<usize>,
    pub start_room: usize,
    pub(crate) assets: HashMap<Sym, Value>,
    pub(crate) functions: HashMap<Sym, Rc<FuncDef>>,
    pub(crate) natives: HashMap<Sym, NativeFn>,
    pub(crate) consts: HashMap<Sym, Value>,
    bv: Vec<Option<Bv>>,
    dyn_syms: DynSyms,
    pub(crate) init_code: Vec<(Rc<str>, Rc<Vec<Stmt>>)>,
    startup_code: Option<Rc<Vec<Stmt>>>,

    pub globals: HashMap<Sym, Value>,
    pub insts: Vec<Instance>,
    index: HashMap<u32, usize>,
    next_id: u32,

    pub room: usize,
    pub room_w: f64,
    pub room_h: f64,
    pub room_speed: f64,
    pub room_bg: Option<u32>,
    pub layers: Vec<Layer>,
    saved_rooms: HashMap<usize, SavedRoom>,
    pub(crate) pending_room: Option<usize>,
    pub(crate) pending_restart: bool,

    pub quit: bool,
    pub input: Input,
    pub draw: DrawState,
    pub cmds: Vec<DrawCmd>,
    pub cam: Camera,
    pub audio: Vec<AudioCmd>,
    /// Recent runtime errors (also printed to stderr once each).
    pub errors: Vec<String>,
    seen_errors: HashSet<String>,
    event_stack: Vec<(Ev, u32)>,
    pub(crate) rng: u64,
    pub steps: u64,
    /// Real frames per second, set by the backend.
    pub fps: f64,
    pub debug: bool,
    /// Text measuring provided by the backend: (font, text, scale) -> (w, h).
    pub measure: MeasureFn,
    /// Particle systems (see `effects.rs`).
    pub parts: crate::effects::EngineParts,
    /// Shaders and the blend mode.
    pub shaders: crate::shader::Shaders,
    /// `extensions/` of the project.
    pub extensions: Vec<crate::extension::Extension>,
}

fn overlap(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] < b[2] && a[2] > b[0] && a[1] < b[3] && a[3] > b[1]
}

impl Engine {
    // ============================================================ loading

    pub fn load(dir: &Path) -> Result<Engine, String> {
        let project = Project::load(dir)?;
        Self::from_project(project)
    }

    pub fn from_project(p: Project) -> Result<Engine, String> {
        let mut errors: Vec<String> = Vec::new();
        let mut assets: HashMap<Sym, Value> = HashMap::new();
        let mut register = |name: &str, v: Value, errors: &mut Vec<String>| {
            let s = intern(name);
            if let Some(prev) = assets.get(&s) {
                errors.push(format!("duplicate asset name '{name}' ({} and {})", prev.type_name(), v.type_name()));
            }
            assets.insert(s, v);
        };
        for (i, s) in p.sprites.iter().enumerate() {
            register(&s.name, Value::Asset(AssetKind::Sprite, i as u32), &mut errors);
        }
        for (i, s) in p.sounds.iter().enumerate() {
            register(&s.name, Value::Asset(AssetKind::Sound, i as u32), &mut errors);
        }
        for (i, f) in p.fonts.iter().enumerate() {
            register(&f.name, Value::Asset(AssetKind::Font, i as u32), &mut errors);
        }
        for (i, t) in p.tilesets.iter().enumerate() {
            register(&t.name, Value::Asset(AssetKind::TileSet, i as u32), &mut errors);
        }
        for (i, (name, _, _)) in p.rooms.iter().enumerate() {
            register(name, Value::Asset(AssetKind::Room, i as u32), &mut errors);
        }
        let motion = crate::anim_assets::load(&p.dir, &mut errors);
        for (name, v) in crate::anim_assets::names(&motion) {
            register(&name, v, &mut errors);
        }

        // ---- scripts
        let mut functions: HashMap<Sym, Rc<FuncDef>> = HashMap::new();
        let mut decls = Vec::new();
        let mut init_code = Vec::new();
        for (file, src) in &p.scripts {
            match parse_file(file, src) {
                Ok(ast) => {
                    for f in ast.functions {
                        if functions.contains_key(&f.name) {
                            errors.push(format!("{file}:{}: function '{}' is defined twice", f.line, f.name));
                        }
                        functions.insert(f.name, f);
                    }
                    if !ast.init.is_empty() {
                        init_code.push((Rc::from(file.as_str()), Rc::new(ast.init)));
                    }
                    decls.extend(ast.objects);
                }
                Err(e) => errors.push(e.to_string()),
            }
        }
        for (i, d) in decls.iter().enumerate() {
            register(&d.name.name(), Value::Asset(AssetKind::Object, i as u32), &mut errors);
        }
        if !errors.is_empty() {
            return Err(errors.join("\n"));
        }

        let objects: Vec<ObjectDef> = decls
            .iter()
            .map(|d| ObjectDef {
                name: d.name.name().to_string(),
                sym: d.name,
                parent: None,
                props: Rc::new(vec![]),
                vars: d.vars.clone(),
                file: d.file.clone(),
                own_events: HashMap::new(),
                events: HashMap::new(),
                ancestors: vec![],
                collision_targets: Rc::new(vec![]),
                input_events: Rc::new(vec![]),
                bounds_events: false,
            })
            .collect();

        let mut bv = Vec::new();
        for (name, v) in BUILTIN_VARS {
            let s = intern(name).index();
            if bv.len() <= s {
                bv.resize(s + 1, None);
            }
            bv[s] = Some(*v);
        }

        let display = p.config.display.clone();
        let mut e = Engine {
            dir: p.dir.clone(),
            config: p.config.clone(),
            sprites: p.sprites,
            sounds: p.sounds,
            fonts: p.fonts,
            tilesets: vec![],
            paths: motion.paths,
            timelines: motion.timelines,
            curves: motion.curves,
            sequences: vec![],
            seqs: Default::default(),
            objects,
            rooms: vec![],
            room_order: vec![],
            start_room: 0,
            assets,
            functions,
            natives: natives::registry(),
            consts: natives::constants(),
            bv,
            dyn_syms: DynSyms {
                mouse_x: intern("mouse_x"),
                mouse_y: intern("mouse_y"),
                room: intern("room"),
                room_width: intern("room_width"),
                room_height: intern("room_height"),
            },
            init_code,
            startup_code: None,
            globals: HashMap::new(),
            insts: vec![],
            index: HashMap::new(),
            next_id: 100_000,
            room: 0,
            room_w: display.width as f64,
            room_h: display.height as f64,
            room_speed: display.step_rate,
            room_bg: None,
            layers: vec![],
            saved_rooms: HashMap::new(),
            pending_room: None,
            pending_restart: false,
            quit: false,
            input: Input::default(),
            draw: DrawState::default(),
            cmds: vec![],
            cam: Camera {
                x: 0.0,
                y: 0.0,
                w: display.width as f64,
                h: display.height as f64,
                follow: None,
                smooth: 1.0,
                shake: 0.0,
                shake_steps: 0,
            },
            audio: vec![],
            errors: vec![],
            seen_errors: HashSet::new(),
            event_stack: vec![],
            rng: 0x9E37_79B9_7F4A_7C15,
            steps: 0,
            fps: 0.0,
            debug: false,
            measure: Box::new(|_, text, scale| {
                let lines = text.split('\n').count() as f32;
                let longest = text.split('\n').map(|l| l.chars().count()).max().unwrap_or(0) as f32;
                (longest * 8.0 * scale, lines * 16.0 * scale)
            }),
            parts: Default::default(),
            shaders: Default::default(),
            extensions: vec![],
        };
        // particle systems, shaders and extensions
        crate::effects::link(&mut e, &mut errors);
        // fonts measure with their baked atlases, also without a window
        e.measure = crate::font::headless_measure(&e.fonts);
        // tile sets: their tiles become frames of a sprite drawn like any other
        for mut t in p.tilesets {
            t.sprite_index = e.sprites.len() as u32;
            let s = t.take_sprite();
            e.sprites.push(s);
            e.tilesets.push(t);
        }

        // ---- objects: parents, events
        for (i, d) in decls.into_iter().enumerate() {
            if let Some(ps) = d.parent {
                match e.assets.get(&ps) {
                    Some(Value::Asset(AssetKind::Object, pi)) => e.objects[i].parent = Some(*pi),
                    _ => errors.push(format!("{}:{}: parent object '{ps}' of '{}' does not exist", d.file, d.line, d.name)),
                }
            }
            for ev in &d.events {
                let arg = match &ev.arg {
                    Some(expr) => match modscript::eval_expr(&mut e, expr, &d.file, CallCx::global()) {
                        Ok(v) => Some(v),
                        Err(err) => {
                            errors.push(format!("{}:{}: in event argument: {}", d.file, ev.line, err.msg));
                            continue;
                        }
                    },
                    None => None,
                };
                match parse_event(&ev.name.name(), arg) {
                    Ok(k) => {
                        if e.objects[i].own_events.insert(k, ev.body.clone()).is_some() {
                            errors.push(format!("{}:{}: event 'on {}' is defined twice in '{}'", d.file, ev.line, ev.name, d.name));
                        }
                    }
                    Err(msg) => errors.push(format!("{}:{}: {msg}", d.file, ev.line)),
                }
            }
            e.objects[i].props = Rc::new(d.props);
        }
        let n = e.objects.len();
        for i in 0..n {
            let mut chain = vec![i as u32];
            let mut cur = e.objects[i].parent;
            while let Some(p) = cur {
                if chain.contains(&p) {
                    errors.push(format!("object '{}' has a circular parent chain", e.objects[i].name));
                    break;
                }
                chain.push(p);
                cur = e.objects[p as usize].parent;
            }
            let mut events = HashMap::new();
            for &a in chain.iter().rev() {
                for (k, def) in &e.objects[a as usize].own_events {
                    events.insert(*k, (a, def.clone()));
                }
            }
            let mut coll: Vec<u32> = events.keys().filter_map(|k| if let Ev::Collision(t) = k { Some(*t) } else { None }).collect();
            coll.sort();
            let mut input: Vec<Ev> = events.keys().copied().filter(|k| k.is_input()).collect();
            input.sort_by_key(|k| format!("{k:?}"));
            let o = &mut e.objects[i];
            o.bounds_events = events.contains_key(&Ev::OutsideRoom) || events.contains_key(&Ev::Boundary);
            o.ancestors = chain;
            o.events = events;
            o.collision_targets = Rc::new(coll);
            o.input_events = Rc::new(input);
        }

        // ---- typed variables: a child may not redeclare a parent's variable with another type
        for i in 0..n {
            for v in &e.objects[i].vars {
                if BUILTIN_VARS.iter().any(|(b, _)| *b == &*v.name.name()) {
                    errors.push(format!("{}:{}: '{}' is a built-in variable and can't be given a type", e.objects[i].file, v.line, v.name));
                }
                for &a in e.objects[i].ancestors.iter().skip(1) {
                    let pa = &e.objects[a as usize];
                    if let Some(pv) = pa.vars.iter().find(|p| p.name == v.name) {
                        if pv.ty != v.ty {
                            errors.push(format!(
                                "{}:{}: '{}' declares '{}: {}' but its parent '{}' declares it as {}",
                                e.objects[i].file,
                                v.line,
                                e.objects[i].name,
                                v.name,
                                v.ty.to_source(),
                                pa.name,
                                pv.ty.to_source()
                            ));
                        }
                        break;
                    }
                }
            }
        }

        crate::anim_assets::resolve_sequences(&mut e, motion.sequences, &mut errors);

        // ---- rooms
        for (name, path, rt) in &p.rooms {
            match e.build_room(name, path, rt) {
                Ok(r) => e.rooms.push(r),
                Err(msg) => errors.push(format!("{}: {msg}", path.display())),
            }
        }
        let names: Vec<String> = e.rooms.iter().map(|r| r.name.clone()).collect();
        let find = |n: &str| names.iter().position(|r| r == n);
        if e.config.game.rooms.is_empty() {
            e.room_order = (0..e.rooms.len()).collect();
        } else {
            for rn in &e.config.game.rooms {
                match find(rn) {
                    Some(i) => e.room_order.push(i),
                    None => errors.push(format!("game.toml: room '{rn}' listed in [game].rooms does not exist")),
                }
            }
            for i in 0..e.rooms.len() {
                if !e.room_order.contains(&i) {
                    e.room_order.push(i);
                }
            }
        }
        e.start_room = match &e.config.game.start_room {
            Some(n) => match find(n) {
                Some(i) => i,
                None => {
                    errors.push(format!("game.toml: start_room '{n}' does not exist"));
                    0
                }
            },
            None => e.room_order.first().copied().unwrap_or(0),
        };

        if !errors.is_empty() {
            return Err(errors.join("\n"));
        }
        Ok(e)
    }

    fn sprite_by_name(&self, n: &str) -> Result<u32, String> {
        match self.assets.get(&intern(n)) {
            Some(Value::Asset(AssetKind::Sprite, i)) => Ok(*i),
            _ => Err(format!("sprite '{n}' does not exist")),
        }
    }

    /// Declared type of variable `name` for `object` (its own or the nearest ancestor's),
    /// with the object that declares it.
    pub fn var_type(&self, object: u32, name: Sym) -> Option<(modscript::VarType, u32)> {
        let o = self.objects.get(object as usize)?;
        let chain: Vec<u32> = if o.ancestors.is_empty() { vec![object] } else { o.ancestors.clone() };
        chain.iter().find_map(|&a| self.objects[a as usize].vars.iter().find(|v| v.name == name).map(|v| (v.ty.clone(), a)))
    }

    /// Converts a TOML value from a room file to a value of type `ty`: asset types take the
    /// asset name as a string and colours take `"#rrggbb"`.
    pub fn typed_value(&self, ty: &modscript::VarType, v: &toml::Value) -> Result<Value, String> {
        use modscript::VarType;
        let val = match (ty, v) {
            (VarType::Asset(k), toml::Value::String(s)) => match self.assets.get(&intern(s)) {
                Some(a @ Value::Asset(k2, _)) if k2 == k => a.clone(),
                _ => return Err(format!("{} '{s}' does not exist", k.name())),
            },
            (VarType::Color, toml::Value::String(s)) => Value::Num(crate::draw::parse_color(s).ok_or_else(|| format!("bad colour '{s}'"))? as f64),
            _ => toml_to_value(v)?,
        };
        ty.check(&val)?;
        Ok(val)
    }

    /// Checks every typed variable of instance `id` against its declaration.
    fn check_typed_vars(&mut self, id: u32) {
        let Some(i) = self.idx(id) else { return };
        let obj = self.insts[i].object as usize;
        let mut errs = Vec::new();
        for &a in &self.objects[obj].ancestors {
            let def = &self.objects[a as usize];
            for v in &def.vars {
                let val = self.insts[i].vars.get(&v.name).cloned().unwrap_or(Value::Null);
                if let Err(m) = v.ty.check(&val) {
                    let mut e = RtError::new(format!("{}.{}: {m}", self.objects[obj].name, v.name));
                    e.file = def.file.clone();
                    e.line = v.line;
                    errs.push(e);
                }
            }
        }
        for e in errs {
            self.report(e);
        }
    }

    fn object_by_name(&self, n: &str) -> Result<u32, String> {
        match self.assets.get(&intern(n)) {
            Some(Value::Asset(AssetKind::Object, i)) => Ok(*i),
            _ => Err(format!("object '{n}' does not exist")),
        }
    }

    fn build_room(&self, name: &str, path: &Path, rt: &RoomToml) -> Result<RoomDef, String> {
        let disp: &DisplaySection = &self.config.display;
        let mut layers = Vec::new();
        let mut spawns = Vec::new();
        let (mut max_w, mut max_h) = (0.0f64, 0.0f64);
        for (li, l) in rt.layers.iter().enumerate() {
            let lname = if l.name.is_empty() { format!("{}{li}", l.kind) } else { l.name.clone() };
            let kind = match l.kind.as_str() {
                "background" => LayerKind::Background {
                    sprite: l.sprite.as_deref().map(|s| self.sprite_by_name(s)).transpose()?,
                    color: match &l.color {
                        Some(c) => Some(parse_color(c).ok_or_else(|| format!("bad colour '{c}'"))?),
                        None => None,
                    },
                    tile_x: l.tile_x,
                    tile_y: l.tile_y,
                    stretch: l.stretch,
                    hspeed: l.hspeed,
                    vspeed: l.vspeed,
                    parallax: parse_parallax(l)?,
                    ox: l.x,
                    oy: l.y,
                },
                "tiles" => {
                    let ts = l.tileset.as_deref().ok_or("tile layers need `tileset = \"tile set or sprite\"`")?;
                    // a tile set asset, or (older projects) a sprite whose frames are the tiles
                    let set = self.tilesets.iter().position(|t| t.name == ts).map(|i| i as u32);
                    let tileset = match set {
                        Some(i) => self.tilesets[i as usize].sprite_index,
                        None => self.sprite_by_name(ts).map_err(|_| format!("tile set or sprite '{ts}' does not exist"))?,
                    };
                    let spr = &self.sprites[tileset as usize];
                    let [cw, ch] = l.cell.unwrap_or([spr.fw, spr.fh]);
                    let mut grid = Vec::new();
                    if let Some(data) = &l.data {
                        grid = crate::natives::tilemap::parse_data(data).map_err(|e| format!("layer '{lname}': {e}"))?;
                    }
                    let map = match (&l.map, &l.data) {
                        (Some(m), _) => m.as_str(),
                        (None, Some(_)) => "",
                        (None, None) => return Err("tile layers need a `map` or `data`".into()),
                    };
                    for row in map_rows(map) {
                        let mut r = Vec::new();
                        for c in row.chars() {
                            let v = match l.legend.get(&c.to_string()) {
                                Some(toml::Value::Integer(i)) => *i as i32,
                                Some(other) => return Err(format!("tile legend '{c}' must be a tile number, got {other}")),
                                None if c == '.' || c == ' ' => -1,
                                None => return Err(format!("map character '{c}' is not in the tile legend")),
                            };
                            r.push(v);
                        }
                        grid.push(r);
                    }
                    max_w = max_w.max(grid.iter().map(|r| r.len()).max().unwrap_or(0) as f64 * cw as f64 + l.x);
                    max_h = max_h.max(grid.len() as f64 * ch as f64 + l.y);
                    LayerKind::Tiles { tileset, set, cw, ch, grid, ox: l.x, oy: l.y }
                }
                "instances" => {
                    for it in &l.instances {
                        let object = self.object_by_name(&it.object)?;
                        let vars = it
                            .vars
                            .iter()
                            .map(|(k, v)| {
                                let sym = intern(k);
                                let val = match self.var_type(object, sym) {
                                    Some((ty, _)) => self.typed_value(&ty, v).map_err(|m| format!("instance of {} at {}, {}: {k}: {m}", it.object, it.x, it.y))?,
                                    None => toml_to_value(v)?,
                                };
                                Ok((sym, val))
                            })
                            .collect::<Result<Vec<_>, String>>()?;
                        spawns.push(Spawn { object, x: it.x, y: it.y, depth: l.depth, vars, cell: None });
                    }
                    if let Some(map) = &l.map {
                        let [cw, ch] = l.cell.unwrap_or([16, 16]);
                        let rows = map_rows(map);
                        for (ry, row) in rows.iter().enumerate() {
                            for (cx, c) in row.chars().enumerate() {
                                if c == '.' || c == ' ' {
                                    continue;
                                }
                                let names: Vec<String> = match l.legend.get(&c.to_string()) {
                                    Some(toml::Value::String(s)) => vec![s.clone()],
                                    Some(toml::Value::Array(a)) => a.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
                                    Some(other) => return Err(format!("legend '{c}' must be an object name, got {other}")),
                                    None => return Err(format!("map character '{c}' is not in the legend of layer '{lname}'")),
                                };
                                let (x, y) = (l.x + cx as f64 * cw as f64, l.y + ry as f64 * ch as f64);
                                for n in names {
                                    let object = self.object_by_name(&n)?;
                                    spawns.push(Spawn {
                                        object,
                                        x,
                                        y,
                                        depth: l.depth,
                                        vars: vec![],
                                        cell: Some([x, y, cw as f64, ch as f64]),
                                    });
                                }
                            }
                        }
                        max_w = max_w.max(rows.iter().map(|r| r.chars().count()).max().unwrap_or(0) as f64 * cw as f64 + l.x);
                        max_h = max_h.max(rows.len() as f64 * ch as f64 + l.y);
                    }
                    LayerKind::Instances
                }
                other => return Err(format!("unknown layer type '{other}' (use instances, tiles or background)")),
            };
            layers.push(Layer { name: lname, depth: l.depth, visible: l.visible, alpha: l.alpha, kind });
        }
        let file: Rc<str> = Rc::from(path.display().to_string().as_str());
        let code = match &rt.code {
            Some(src) => Some(Rc::new(parse_stmts(&file, src).map_err(|e| format!("room code: {}", e.msg))?)),
            None => None,
        };
        let cam_follow = rt.camera.follow.as_deref().map(|n| self.object_by_name(n)).transpose()?;
        Ok(RoomDef {
            name: name.to_string(),
            // without an explicit size a room covers its maps, and at least the screen
            w: rt.width.map(|w| w as f64).unwrap_or(max_w.max(disp.width as f64)),
            h: rt.height.map(|h| h as f64).unwrap_or(max_h.max(disp.height as f64)),
            speed: rt.speed,
            background: match &rt.background {
                Some(c) => Some(parse_color(c).ok_or_else(|| format!("bad background colour '{c}'"))?),
                None => None,
            },
            persistent: rt.persistent,
            code,
            file,
            layers,
            spawns,
            cam_follow,
            cam_smooth: rt.camera.smooth.clamp(0.01, 1.0),
            cam_start: (rt.camera.x, rt.camera.y),
        })
    }

    // ============================================================ lifecycle

    /// Runs top-level script code and enters the first room.
    pub fn start(&mut self) {
        for (file, code) in self.init_code.clone() {
            if let Err(e) = run_stmts(self, &code, &file, CallCx::global()) {
                self.report(e);
            }
        }
        self.enter_room(self.start_room, true);
        if let Some(code) = self.startup_code.clone() {
            if let Err(e) = run_stmts(self, &code, &Rc::from("<exec>"), CallCx::global()) {
                self.report(e);
            }
        }
    }

    /// Extra code run once right after the first room starts (debugging / tests).
    pub fn set_startup_code(&mut self, src: &str) -> Result<(), String> {
        let stmts = parse_stmts("<exec>", src).map_err(|e| e.to_string())?;
        self.startup_code = Some(Rc::new(stmts));
        Ok(())
    }

    /// Call when the window closes.
    pub fn shutdown(&mut self) {
        let ids = self.ids();
        self.dispatch(&ids, Ev::GameEnd);
        self.extensions_final();
    }

    pub fn step_rate(&self) -> f64 {
        self.room_speed.max(1.0)
    }

    fn enter_room(&mut self, r: usize, first: bool) {
        self.room = r;
        let (w, h, speed, bg, follow, smooth, start, layers) = {
            let d = &self.rooms[r];
            (d.w, d.h, d.speed, d.background, d.cam_follow, d.cam_smooth, d.cam_start, d.layers.clone())
        };
        self.room_w = w;
        self.room_h = h;
        self.room_speed = speed.unwrap_or(self.config.display.step_rate);
        self.room_bg = bg;
        self.cam.follow = follow.map(|o| Value::Asset(AssetKind::Object, o));
        self.cam.smooth = smooth;
        self.cam.x = start.0;
        self.cam.y = start.1;
        self.cam.shake_steps = 0;

        if let Some(saved) = self.saved_rooms.remove(&r) {
            self.layers = saved.layers;
            self.cam.x = saved.cam.0;
            self.cam.y = saved.cam.1;
            for inst in saved.insts {
                self.index.insert(inst.id, self.insts.len());
                self.insts.push(inst);
            }
        } else {
            self.layers = layers;
            let spawns = self.rooms[r].spawns.clone();
            for s in spawns {
                let id = self.create_instance(s.object, s.x, s.y, s.depth);
                self.init_instance(id, &s.vars, s.cell, Scope::Global);
            }
            if let Some(code) = self.rooms[r].code.clone() {
                let file = self.rooms[r].file.clone();
                if let Err(e) = run_stmts(self, &code, &file, CallCx::global()) {
                    self.report(e);
                }
            }
        }
        let ids = self.ids();
        if first {
            self.dispatch(&ids, Ev::GameStart);
        }
        self.dispatch(&ids, Ev::RoomStart);
        self.update_camera(true);
    }

    fn leave_room(&mut self) {
        let ids = self.ids();
        self.dispatch(&ids, Ev::RoomEnd);
        self.seqs.list.clear();
        let all = std::mem::take(&mut self.insts);
        let (keep, gone): (Vec<Instance>, Vec<Instance>) = all.into_iter().filter(|i| i.alive).partition(|i| i.persistent);
        if self.rooms[self.room].persistent {
            self.saved_rooms.insert(self.room, SavedRoom { insts: gone, layers: self.layers.clone(), cam: (self.cam.x, self.cam.y) });
        }
        self.insts = keep;
        self.rebuild_index();
    }

    fn restart_game(&mut self) {
        self.audio.push(AudioCmd::StopAll);
        self.insts.clear();
        self.index.clear();
        self.seqs.list.clear();
        self.parts.world.types.clear();
        self.parts.world.systems.clear();
        self.globals.clear();
        self.saved_rooms.clear();
        self.start();
    }

    fn rebuild_index(&mut self) {
        self.index.clear();
        for (i, inst) in self.insts.iter().enumerate() {
            self.index.insert(inst.id, i);
        }
    }

    // ============================================================ instances

    pub fn create_instance(&mut self, obj: u32, x: f64, y: f64, depth: f64) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.index.insert(id, self.insts.len());
        self.insts.push(Instance::new(id, obj, x, y, depth));
        id
    }

    /// Runs property initializers (parents first), applies extra variables and fires `create`.
    pub fn init_instance(&mut self, id: u32, vars: &[(Sym, Value)], cell: Option<[f64; 4]>, creator: Scope) {
        let Some(i) = self.idx(id) else { return };
        let obj = self.insts[i].object as usize;
        let chain = self.objects[obj].ancestors.clone();
        for &a in chain.iter().rev() {
            let props = self.objects[a as usize].props.clone();
            if props.is_empty() {
                continue;
            }
            let file = self.objects[a as usize].file.clone();
            if let Err(e) = run_stmts(self, &props, &file, CallCx { self_: Scope::Inst(id), other: creator.clone() }) {
                self.report(e);
            }
        }
        if let Some([cx, cy, cw, ch]) = cell {
            let inst = &self.insts[i];
            if let Some(s) = inst.sprite.map(|s| &self.sprites[s as usize]) {
                let (w, h) = (s.fw as f64 * inst.scale_x.abs(), s.fh as f64 * inst.scale_y.abs());
                let (ox, oy) = (s.origin.0 * inst.scale_x.abs(), s.origin.1 * inst.scale_y.abs());
                let x = cx + (cw - w) / 2.0 + ox;
                let y = if h > ch { cy + ch - h + oy } else { cy + (ch - h) / 2.0 + oy };
                let inst = &mut self.insts[i];
                inst.x = x;
                inst.y = y;
                inst.xstart = x;
                inst.ystart = y;
                inst.xprev = x;
                inst.yprev = y;
            }
        }
        for (k, v) in vars {
            if let Err(e) = self.inst_set(id, *k, v.clone()) {
                self.report(e);
            }
        }
        if self.objects[obj].ancestors.iter().any(|&a| !self.objects[a as usize].vars.is_empty()) {
            self.check_typed_vars(id);
        }
        self.run_event(id, Ev::Create, creator);
    }

    pub fn destroy(&mut self, id: u32, run_event: bool) {
        let Some(i) = self.idx(id) else { return };
        if !self.insts[i].alive {
            return;
        }
        self.insts[i].alive = false;
        if run_event {
            self.run_event(id, Ev::Destroy, Scope::Global);
        }
    }

    pub fn idx(&self, id: u32) -> Option<usize> {
        self.index.get(&id).copied()
    }

    pub fn alive(&self, id: u32) -> bool {
        self.idx(id).map(|i| self.insts[i].alive).unwrap_or(false)
    }

    /// Ids of all live instances, in creation order.
    pub fn ids(&self) -> Vec<u32> {
        self.insts.iter().filter(|i| i.alive).map(|i| i.id).collect()
    }

    pub fn is_a(&self, obj: u32, target: u32) -> bool {
        self.objects[obj as usize].ancestors.contains(&target)
    }

    pub fn matches(&self, inst: &Instance, target: &Value) -> bool {
        match target {
            Value::Inst(id) => inst.id == *id,
            Value::Asset(AssetKind::Object, o) => self.is_a(inst.object, *o),
            Value::All => true,
            _ => false,
        }
    }

    /// Live instances selected by an instance id, an object (including children) or `all`.
    pub fn resolve(&self, v: &Value) -> Vec<u32> {
        match v {
            Value::Inst(id) => {
                if self.alive(*id) {
                    vec![*id]
                } else {
                    vec![]
                }
            }
            Value::Asset(AssetKind::Object, _) | Value::All => {
                self.insts.iter().filter(|i| i.alive && self.matches(i, v)).map(|i| i.id).collect()
            }
            _ => vec![],
        }
    }

    // ============================================================ collision helpers

    /// Collision box of instance `i` if it stood at (x, y). None without a sprite/mask.
    pub fn bbox_at(&self, i: usize, x: f64, y: f64) -> Option<[f64; 4]> {
        let inst = &self.insts[i];
        let spr = &self.sprites[inst.mask.or(inst.sprite)? as usize];
        let (ox, oy) = spr.origin;
        let b = spr.bbox;
        let x1 = x + (b[0] - ox) * inst.scale_x;
        let x2 = x + (b[2] - ox) * inst.scale_x;
        let y1 = y + (b[1] - oy) * inst.scale_y;
        let y2 = y + (b[3] - oy) * inst.scale_y;
        Some([x1.min(x2), y1.min(y2), x1.max(x2), y1.max(y2)])
    }

    pub fn bbox(&self, i: usize) -> Option<[f64; 4]> {
        self.bbox_at(i, self.insts[i].x, self.insts[i].y)
    }

    /// Instances matching `target` whose box overlaps `rect` (excluding `except`).
    pub fn hits_rect(&self, rect: [f64; 4], target: &Value, except: Option<u32>, first: bool) -> Vec<u32> {
        let mut out = Vec::new();
        for (j, o) in self.insts.iter().enumerate() {
            if !o.alive || Some(o.id) == except || !self.matches(o, target) {
                continue;
            }
            if let Some(ob) = self.bbox(j) {
                if overlap(rect, ob) {
                    out.push(o.id);
                    if first {
                        break;
                    }
                }
            }
        }
        out
    }

    /// Would instance `i` overlap `target` if it stood at (x, y)?
    pub fn hits_at(&self, i: usize, x: f64, y: f64, target: &Value, first: bool) -> Vec<u32> {
        match self.bbox_at(i, x, y) {
            Some(bb) => self.hits_rect(bb, target, Some(self.insts[i].id), first),
            None => vec![],
        }
    }

    pub fn solid_at(&self, i: usize, x: f64, y: f64) -> bool {
        let Some(bb) = self.bbox_at(i, x, y) else { return false };
        let me = self.insts[i].id;
        self.insts.iter().enumerate().any(|(j, o)| {
            o.alive && o.solid && o.id != me && self.bbox(j).map(|ob| overlap(bb, ob)).unwrap_or(false)
        })
    }

    // ============================================================ events

    pub fn run_event(&mut self, id: u32, ev: Ev, other: Scope) -> bool {
        let Some(i) = self.idx(id) else { return false };
        if !self.insts[i].alive && ev != Ev::Destroy {
            return false;
        }
        let obj = self.insts[i].object as usize;
        let Some((owner, def)) = self.objects[obj].events.get(&ev).cloned() else { return false };
        self.exec_event(id, ev, owner, &def, other);
        true
    }

    fn exec_event(&mut self, id: u32, ev: Ev, owner: u32, def: &Rc<FuncDef>, other: Scope) {
        self.event_stack.push((ev, owner));
        let r = run_def(self, def, Scope::Inst(id), other, vec![]);
        self.event_stack.pop();
        if let Err(e) = r {
            self.report(e);
        }
    }

    /// Runs the parent's version of the event that is currently executing.
    pub fn run_inherited(&mut self, cx: &CallCx) -> RtResult<()> {
        let Some(&(ev, owner)) = self.event_stack.last() else {
            return Err(RtError::new("inherited() can only be used inside an event"));
        };
        let Scope::Inst(id) = cx.self_ else { return Err(RtError::new("inherited() needs an instance")) };
        let mut cur = self.objects[owner as usize].parent;
        while let Some(p) = cur {
            if let Some(def) = self.objects[p as usize].own_events.get(&ev).cloned() {
                self.exec_event(id, ev, p, &def, cx.other.clone());
                return Ok(());
            }
            cur = self.objects[p as usize].parent;
        }
        Ok(())
    }

    fn dispatch(&mut self, ids: &[u32], ev: Ev) {
        for &id in ids {
            self.run_event(id, ev, Scope::Global);
        }
    }

    pub fn report(&mut self, e: RtError) {
        let msg = e.to_string();
        if self.seen_errors.insert(msg.clone()) {
            eprintln!("[error] {msg}");
            self.errors.push(msg);
            if self.errors.len() > 8 {
                self.errors.remove(0);
            }
        }
    }

    // ============================================================ the step

    /// One logic step (frame of game time).
    pub fn tick(&mut self) {
        self.steps += 1;
        for inst in self.insts.iter_mut().filter(|i| i.alive) {
            inst.xprev = inst.x;
            inst.yprev = inst.y;
        }
        let ids = self.ids();

        self.dispatch(&ids, Ev::StepBegin);
        crate::timeline::step(self, &ids);
        self.alarms(&ids);
        self.input_events(&ids);
        self.dispatch(&ids, Ev::Step);
        self.motion();
        crate::anim_assets::step_paths(self, &ids);
        self.collisions(&ids);
        self.bounds(&ids);
        self.dispatch(&ids, Ev::StepEnd);
        crate::sequence::step(self);
        self.step_particles();

        for l in &mut self.layers {
            if let LayerKind::Background { hspeed, vspeed, ox, oy, .. } = &mut l.kind {
                *ox += *hspeed;
                *oy += *vspeed;
            }
        }
        self.update_camera(false);
        if self.insts.iter().any(|i| !i.alive) {
            self.insts.retain(|i| i.alive);
            self.rebuild_index();
        }
        self.input.clear_edges();

        if self.pending_restart {
            self.pending_restart = false;
            self.pending_room = None;
            self.restart_game();
        } else if let Some(r) = self.pending_room.take() {
            self.leave_room();
            self.enter_room(r, false);
        }
    }

    fn alarms(&mut self, ids: &[u32]) {
        for &id in ids {
            let Some(i) = self.idx(id) else { continue };
            if !self.insts[i].alive {
                continue;
            }
            let alarms = self.insts[i].alarms.clone();
            for a in 0..ALARM_COUNT {
                let fire = {
                    let mut arr = alarms.borrow_mut();
                    match arr.get(a).and_then(|v| v.as_num()) {
                        Some(v) if v > 0.0 => {
                            let nv = (v - 1.0).max(0.0);
                            if nv <= 0.0 {
                                arr[a] = Value::Num(-1.0);
                                true
                            } else {
                                arr[a] = Value::Num(nv);
                                false
                            }
                        }
                        _ => false,
                    }
                };
                if fire && self.alive(id) {
                    self.run_event(id, Ev::Alarm(a as u8), Scope::Global);
                }
            }
        }
    }

    fn mouse_room(&self) -> (f64, f64) {
        (self.input.mouse_x + self.cam.x, self.input.mouse_y + self.cam.y)
    }

    fn input_events(&mut self, ids: &[u32]) {
        let (mx, my) = self.mouse_room();
        for &id in ids {
            let Some(i) = self.idx(id) else { continue };
            if !self.insts[i].alive {
                continue;
            }
            let evs = self.objects[self.insts[i].object as usize].input_events.clone();
            if evs.is_empty() {
                continue;
            }
            let over = self.bbox(i).map(|b| mx >= b[0] && mx < b[2] && my >= b[1] && my < b[3]).unwrap_or(false);
            let was_over = std::mem::replace(&mut self.insts[i].mouse_over, over);
            for ev in evs.iter() {
                let inp = &self.input;
                let key = |k: u16, v: &Vec<bool>| if k == ANY_KEY { v.iter().any(|b| *b) } else { v[k as usize] };
                let fire = match *ev {
                    Ev::Key(k) => key(k, &inp.down),
                    Ev::KeyPressed(k) => key(k, &inp.pressed),
                    Ev::KeyReleased(k) => key(k, &inp.released),
                    Ev::GlobalMouseDown(b) => inp.mouse_down[b as usize],
                    Ev::GlobalMousePressed(b) => inp.mouse_pressed[b as usize],
                    Ev::GlobalMouseReleased(b) => inp.mouse_released[b as usize],
                    Ev::MouseDown(b) => over && inp.mouse_down[b as usize],
                    Ev::MousePressed(b) => over && inp.mouse_pressed[b as usize],
                    Ev::MouseReleased(b) => over && inp.mouse_released[b as usize],
                    Ev::MouseEnter => over && !was_over,
                    Ev::MouseLeave => !over && was_over,
                    _ => false,
                };
                if fire {
                    self.run_event(id, *ev, Scope::Global);
                    if !self.alive(id) {
                        break;
                    }
                }
            }
        }
    }

    fn motion(&mut self) {
        let mut ended = Vec::new();
        let rate = self.step_rate();
        let sprites = &self.sprites;
        for inst in self.insts.iter_mut().filter(|i| i.alive) {
            if inst.friction != 0.0 {
                let sp = inst.speed();
                if sp > 0.0 {
                    let ns = (sp - inst.friction).max(0.0);
                    let k = ns / sp;
                    inst.hspeed *= k;
                    inst.vspeed *= k;
                }
            }
            if inst.gravity != 0.0 {
                let r = inst.gravity_dir.to_radians();
                let (h, v) = (inst.hspeed + inst.gravity * r.cos(), inst.vspeed - inst.gravity * r.sin());
                inst.set_hv(h, v);
            }
            inst.x += inst.hspeed;
            inst.y += inst.vspeed;
            if let Some(s) = inst.sprite {
                let spr = &sprites[s as usize];
                let n = spr.frames.len() as f64;
                if spr.fps > 0.0 && inst.anim_speed != 0.0 {
                    inst.frame += spr.fps * inst.anim_speed / rate;
                    if inst.frame >= n || inst.frame < 0.0 {
                        inst.frame = inst.frame.rem_euclid(n);
                        ended.push(inst.id);
                    }
                }
            }
        }
        for id in ended {
            self.run_event(id, Ev::AnimationEnd, Scope::Global);
        }
    }

    fn collisions(&mut self, ids: &[u32]) {
        for &id in ids {
            let Some(i) = self.idx(id) else { continue };
            if !self.insts[i].alive {
                continue;
            }
            let targets = self.objects[self.insts[i].object as usize].collision_targets.clone();
            for &t in targets.iter() {
                let target = Value::Asset(AssetKind::Object, t);
                let hits = self.hits_at(i, self.insts[i].x, self.insts[i].y, &target, false);
                for oid in hits {
                    if !self.alive(id) {
                        break;
                    }
                    let Some(oi) = self.idx(oid) else { continue };
                    if !self.insts[oi].alive {
                        continue;
                    }
                    let still = match (self.bbox(i), self.bbox(oi)) {
                        (Some(a), Some(b)) => overlap(a, b),
                        _ => false,
                    };
                    if !still {
                        continue;
                    }
                    if self.insts[oi].solid {
                        let me = &mut self.insts[i];
                        me.x = me.xprev;
                        me.y = me.yprev;
                    }
                    self.run_event(id, Ev::Collision(t), Scope::Inst(oid));
                }
            }
        }
    }

    fn bounds(&mut self, ids: &[u32]) {
        for &id in ids {
            let Some(i) = self.idx(id) else { continue };
            let inst = &self.insts[i];
            if !inst.alive || !self.objects[inst.object as usize].bounds_events {
                continue;
            }
            let b = self.bbox(i).unwrap_or([inst.x, inst.y, inst.x, inst.y]);
            let outside = b[2] < 0.0 || b[0] > self.room_w || b[3] < 0.0 || b[1] > self.room_h;
            let boundary = !outside && (b[0] < 0.0 || b[2] > self.room_w || b[1] < 0.0 || b[3] > self.room_h);
            let was_out = std::mem::replace(&mut self.insts[i].outside, outside);
            let was_b = std::mem::replace(&mut self.insts[i].at_boundary, boundary);
            if outside && !was_out {
                self.run_event(id, Ev::OutsideRoom, Scope::Global);
            }
            if boundary && !was_b {
                self.run_event(id, Ev::Boundary, Scope::Global);
            }
        }
    }

    pub fn update_camera(&mut self, snap: bool) {
        if let Some(t) = self.cam.follow.clone() {
            if let Some(i) = self.resolve(&t).first().and_then(|id| self.idx(*id)) {
                let (tx, ty) = (self.insts[i].x - self.cam.w / 2.0, self.insts[i].y - self.cam.h / 2.0);
                let k = if snap { 1.0 } else { self.cam.smooth };
                self.cam.x += (tx - self.cam.x) * k;
                self.cam.y += (ty - self.cam.y) * k;
            }
        }
        self.cam.x = if self.room_w > self.cam.w { self.cam.x.clamp(0.0, self.room_w - self.cam.w) } else { (self.room_w - self.cam.w) / 2.0 };
        self.cam.y = if self.room_h > self.cam.h { self.cam.y.clamp(0.0, self.room_h - self.cam.h) } else { (self.room_h - self.cam.h) / 2.0 };
        if self.cam.shake_steps > 0 {
            self.cam.shake_steps -= 1;
        }
    }

    // ============================================================ drawing

    /// Runs the draw events and fills `self.cmds`.
    pub fn render(&mut self) {
        self.cmds.clear();
        self.draw = DrawState::default();
        // the backend starts every frame with the default shader and blend mode
        self.shaders.current = None;
        self.shaders.blend = 0;
        let (mut vx, mut vy) = (self.cam.x, self.cam.y);
        if self.cam.shake_steps > 0 {
            vx += (natives::rand01(&mut self.rng) * 2.0 - 1.0) * self.cam.shake;
            vy += (natives::rand01(&mut self.rng) * 2.0 - 1.0) * self.cam.shake;
        }
        if self.config.display.pixel_perfect {
            vx = vx.round();
            vy = vy.round();
        }
        let view = DrawCmd::View { x: vx as f32, y: vy as f32, w: self.cam.w as f32, h: self.cam.h as f32 };
        self.cmds.push(view.clone());
        self.cmds.push(DrawCmd::Clear(Rgba::from_rgb(self.room_bg.unwrap_or(0), 1.0)));

        // instances sorted by depth: higher depth = further back = drawn first
        let mut order: Vec<(f64, usize, u32)> =
            self.insts.iter().enumerate().filter(|(_, i)| i.alive && i.visible).map(|(n, i)| (i.depth, n, i.id)).collect();
        order.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal).then(a.1.cmp(&b.1)));
        let inst_ids: Vec<u32> = order.iter().map(|o| o.2).collect();

        self.dispatch_visible(&inst_ids, Ev::DrawBegin);

        // layers and instances interleaved by depth (layers first on ties)
        enum Item {
            Layer(usize),
            Inst(u32),
            Seq(u32),
            Parts(usize),
        }
        let mut items: Vec<(f64, Item)> = Vec::new();
        for (li, l) in self.layers.iter().enumerate() {
            if l.visible && !matches!(l.kind, LayerKind::Instances) {
                items.push((l.depth, Item::Layer(li)));
            }
        }
        for o in &order {
            items.push((o.0, Item::Inst(o.2)));
        }
        for s in &self.seqs.list {
            items.push((s.depth, Item::Seq(s.id)));
        }
        for (d, ps) in self.particle_draw_items() {
            items.push((d, Item::Parts(ps)));
        }
        items.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        for (_, item) in items {
            match item {
                Item::Layer(li) => self.draw_layer(li, vx, vy),
                Item::Seq(id) => crate::sequence::draw(self, id),
                Item::Parts(ps) => self.draw_particles(ps),
                Item::Inst(id) => {
                    let Some(i) = self.idx(id) else { continue };
                    if !self.insts[i].alive || !self.insts[i].visible {
                        continue;
                    }
                    if !self.run_event(id, Ev::Draw, Scope::Global) {
                        self.draw_inst(i);
                    }
                }
            }
        }

        self.dispatch_visible(&inst_ids, Ev::DrawEnd);

        if self.debug {
            for i in 0..self.insts.len() {
                if let (true, Some(b)) = (self.insts[i].alive, self.bbox(i)) {
                    self.cmds.push(DrawCmd::Rect {
                        x: b[0] as f32,
                        y: b[1] as f32,
                        w: (b[2] - b[0]) as f32,
                        h: (b[3] - b[1]) as f32,
                        color: Rgba { r: 1.0, g: 0.2, b: 0.2, a: 0.8 },
                        outline: true,
                    });
                }
            }
        }

        self.cmds.push(DrawCmd::Gui);
        self.draw = DrawState::default();
        self.dispatch_visible(&inst_ids, Ev::DrawGui);
    }

    fn dispatch_visible(&mut self, ids: &[u32], ev: Ev) {
        for &id in ids {
            if let Some(i) = self.idx(id) {
                if self.insts[i].alive && self.insts[i].visible {
                    self.run_event(id, ev, Scope::Global);
                }
            }
        }
    }

    /// Draws instance `i` with its own sprite, frame, scale, angle, tint and alpha.
    pub fn draw_inst(&mut self, i: usize) {
        let inst = &self.insts[i];
        let Some(s) = inst.sprite else { return };
        let n = self.sprites[s as usize].frames.len() as i64;
        let frame = (inst.frame.floor() as i64).rem_euclid(n.max(1)) as u32;
        self.cmds.push(DrawCmd::Sprite {
            spr: s,
            frame,
            x: inst.x as f32,
            y: inst.y as f32,
            sx: inst.scale_x as f32,
            sy: inst.scale_y as f32,
            angle: inst.angle as f32,
            color: Rgba::from_rgb(inst.tint, inst.alpha),
        });
    }

    fn draw_layer(&mut self, li: usize, vx: f64, vy: f64) {
        let (vw, vh) = (self.cam.w, self.cam.h);
        let alpha = self.layers[li].alpha;
        // tile layers draw straight from the grid (no per-frame copy of the map)
        if let LayerKind::Tiles { ox, oy, .. } = self.layers[li].kind {
            let col = Rgba::from_rgb(0xFFFFFF, alpha);
            return crate::natives::tilemap::draw_tiles(self, li, ox, oy, (vx, vy, vw, vh), col);
        }
        match self.layers[li].kind.clone() {
            LayerKind::Background { sprite, color, tile_x, tile_y, stretch, parallax, ox, oy, .. } => {
                if let Some(c) = color {
                    self.cmds.push(DrawCmd::Rect {
                        x: vx as f32,
                        y: vy as f32,
                        w: vw as f32,
                        h: vh as f32,
                        color: Rgba::from_rgb(c, alpha),
                        outline: false,
                    });
                }
                let Some(s) = sprite else { return };
                let spr = &self.sprites[s as usize];
                let (fw, fh) = (spr.fw as f64, spr.fh as f64);
                let col = Rgba::from_rgb(0xFFFFFF, alpha);
                if stretch {
                    self.cmds.push(DrawCmd::Stretched { spr: s, frame: 0, x: vx as f32, y: vy as f32, w: vw as f32, h: vh as f32, color: col });
                    return;
                }
                let bx = ox + vx * (1.0 - parallax.0);
                let by = oy + vy * (1.0 - parallax.1);
                let xs: Vec<f64> = if tile_x {
                    let start = bx + ((vx - bx) / fw).floor() * fw;
                    (0..).map(|k| start + k as f64 * fw).take_while(|x| *x < vx + vw).collect()
                } else {
                    vec![bx]
                };
                let ys: Vec<f64> = if tile_y {
                    let start = by + ((vy - by) / fh).floor() * fh;
                    (0..).map(|k| start + k as f64 * fh).take_while(|y| *y < vy + vh).collect()
                } else {
                    vec![by]
                };
                let (gx, gy) = spr.origin;
                for &y in &ys {
                    for &x in &xs {
                        self.cmds.push(DrawCmd::Sprite {
                            spr: s,
                            frame: 0,
                            x: (x + gx) as f32,
                            y: (y + gy) as f32,
                            sx: 1.0,
                            sy: 1.0,
                            angle: 0.0,
                            color: col,
                        });
                    }
                }
            }
            LayerKind::Tiles { .. } => {}
            LayerKind::Instances => {}
        }
    }

    // ============================================================ builtin variables

    fn builtin(&self, s: Sym) -> Option<Bv> {
        self.bv.get(s.index()).copied().flatten()
    }

    fn get_bv(&self, i: usize, bv: Bv) -> Value {
        let inst = &self.insts[i];
        let n = Value::Num;
        let spr = |s: Option<u32>| match s {
            Some(s) => Value::Asset(AssetKind::Sprite, s),
            None => Value::Null,
        };
        match bv {
            Bv::X => n(inst.x),
            Bv::Y => n(inst.y),
            Bv::Xprev => n(inst.xprev),
            Bv::Yprev => n(inst.yprev),
            Bv::Xstart => n(inst.xstart),
            Bv::Ystart => n(inst.ystart),
            Bv::Hspeed => n(inst.hspeed),
            Bv::Vspeed => n(inst.vspeed),
            Bv::Speed => n(inst.speed()),
            Bv::Direction => n(inst.dir),
            Bv::Gravity => n(inst.gravity),
            Bv::GravityDir => n(inst.gravity_dir),
            Bv::Friction => n(inst.friction),
            Bv::Sprite => spr(inst.sprite),
            Bv::Mask => spr(inst.mask),
            Bv::Frame => n(inst.frame),
            Bv::FrameCount => n(inst.sprite.map(|s| self.sprites[s as usize].frames.len() as f64).unwrap_or(0.0)),
            Bv::AnimSpeed => n(inst.anim_speed),
            Bv::ScaleX => n(inst.scale_x),
            Bv::ScaleY => n(inst.scale_y),
            Bv::Angle => n(inst.angle),
            Bv::Alpha => n(inst.alpha),
            Bv::Tint => n(inst.tint as f64),
            Bv::Depth => n(inst.depth),
            Bv::Visible => Value::Bool(inst.visible),
            Bv::Solid => Value::Bool(inst.solid),
            Bv::Persistent => Value::Bool(inst.persistent),
            Bv::Alarm => Value::Array(inst.alarms.clone()),
            Bv::Id => Value::Inst(inst.id),
            Bv::Object => Value::Asset(AssetKind::Object, inst.object),
            Bv::BboxLeft | Bv::BboxRight | Bv::BboxTop | Bv::BboxBottom => {
                let b = self.bbox(i).unwrap_or([inst.x, inst.y, inst.x, inst.y]);
                n(match bv {
                    Bv::BboxLeft => b[0],
                    Bv::BboxTop => b[1],
                    Bv::BboxRight => b[2],
                    _ => b[3],
                })
            }
            Bv::SpriteWidth => n(inst.sprite.map(|s| self.sprites[s as usize].fw as f64 * inst.scale_x.abs()).unwrap_or(0.0)),
            Bv::SpriteHeight => n(inst.sprite.map(|s| self.sprites[s as usize].fh as f64 * inst.scale_y.abs()).unwrap_or(0.0)),
            Bv::Path(v) => crate::path::get_var(inst, v),
            Bv::Timeline(v) => crate::timeline::get_var(inst, v),
        }
    }

    fn set_bv(&mut self, i: usize, bv: Bv, name: Sym, v: Value) -> RtResult<()> {
        let num = |v: &Value| v.as_num().ok_or_else(|| RtError::new(format!("'{name}' must be a number, got {}", v.type_name())));
        let sprite = |v: &Value| match v {
            Value::Null => Ok(None),
            Value::Asset(AssetKind::Sprite, s) => Ok(Some(*s)),
            other => Err(RtError::new(format!("'{name}' must be a sprite or null, got {}", other.type_name()))),
        };
        let inst = &mut self.insts[i];
        match bv {
            Bv::X => inst.x = num(&v)?,
            Bv::Y => inst.y = num(&v)?,
            Bv::Xprev => inst.xprev = num(&v)?,
            Bv::Yprev => inst.yprev = num(&v)?,
            Bv::Xstart => inst.xstart = num(&v)?,
            Bv::Ystart => inst.ystart = num(&v)?,
            Bv::Hspeed => {
                let h = num(&v)?;
                let vs = inst.vspeed;
                inst.set_hv(h, vs)
            }
            Bv::Vspeed => {
                let vs = num(&v)?;
                let h = inst.hspeed;
                inst.set_hv(h, vs)
            }
            Bv::Speed => inst.set_speed(num(&v)?),
            Bv::Direction => inst.set_dir(num(&v)?),
            Bv::Gravity => inst.gravity = num(&v)?,
            Bv::GravityDir => inst.gravity_dir = num(&v)?,
            Bv::Friction => inst.friction = num(&v)?,
            Bv::Sprite => inst.sprite = sprite(&v)?,
            Bv::Mask => inst.mask = sprite(&v)?,
            Bv::Frame => inst.frame = num(&v)?,
            Bv::AnimSpeed => inst.anim_speed = num(&v)?,
            Bv::ScaleX => inst.scale_x = num(&v)?,
            Bv::ScaleY => inst.scale_y = num(&v)?,
            Bv::Angle => inst.angle = num(&v)?,
            Bv::Alpha => inst.alpha = num(&v)?,
            Bv::Tint => inst.tint = num(&v)? as u32,
            Bv::Depth => inst.depth = num(&v)?,
            Bv::Visible => inst.visible = v.truthy(),
            Bv::Solid => inst.solid = v.truthy(),
            Bv::Persistent => inst.persistent = v.truthy(),
            Bv::Alarm => match v {
                Value::Array(a) => inst.alarms = a,
                _ => return Err(RtError::new("alarm must stay an array; use alarm[0] = steps")),
            },
            Bv::Path(pv) => return crate::path::set_var(inst, pv, v),
            Bv::Timeline(tv) => return crate::timeline::set_var(inst, tv, v),
            _ => return Err(RtError::new(format!("'{name}' is read-only"))),
        }
        Ok(())
    }

    // ============================================================ misc helpers used by natives

    pub fn asset_name(&self, kind: AssetKind, i: u32) -> String {
        match kind {
            AssetKind::Object => self.objects[i as usize].name.clone(),
            AssetKind::Sprite => self.sprites[i as usize].name.clone(),
            AssetKind::Sound => self.sounds[i as usize].name.clone(),
            AssetKind::Room => self.rooms[i as usize].name.clone(),
            AssetKind::Font => self.fonts[i as usize].name.clone(),
            AssetKind::TileSet => self.tilesets[i as usize].name.clone(),
            AssetKind::Path | AssetKind::Timeline | AssetKind::AnimCurve | AssetKind::Sequence => crate::anim_assets::asset_name(self, kind, i),
            AssetKind::ParticleSystem => self.parts.world.assets[i as usize].0.clone(),
            AssetKind::Shader => self.shaders.list[i as usize].name.clone(),
        }
    }

    pub fn key_struct_names() -> Vec<(Sym, Value)> {
        let mut v: Vec<(Sym, Value)> = KEY_NAMES.iter().enumerate().map(|(i, k)| (intern(k), Value::Num(i as f64))).collect();
        v.extend(KEY_NAMES.iter().enumerate().filter(|(_, k)| k.len() == 1 && k.chars().all(|c| c.is_ascii_digit())).map(|(i, k)| (intern(&format!("n{k}")), Value::Num(i as f64))));
        v
    }

    pub fn mouse_struct_names() -> Vec<(Sym, Value)> {
        MOUSE_BUTTONS.iter().enumerate().map(|(i, k)| (intern(k), Value::Num(i as f64))).collect()
    }

    pub fn layer_index(&self, name: Option<&str>) -> Option<usize> {
        match name {
            Some(n) => self.layers.iter().position(|l| l.name == n),
            None => self.layers.iter().position(|l| matches!(l.kind, LayerKind::Tiles { .. })),
        }
    }
}

fn map_rows(map: &str) -> Vec<String> {
    let lines: Vec<&str> = map.lines().collect();
    let start = lines.iter().position(|l| !l.trim().is_empty()).unwrap_or(lines.len());
    let end = lines.iter().rposition(|l| !l.trim().is_empty()).map(|e| e + 1).unwrap_or(start);
    lines[start..end].iter().map(|l| l.trim_end().to_string()).collect()
}

fn parse_parallax(l: &LayerToml) -> Result<(f64, f64), String> {
    let f = |v: &toml::Value| v.as_float().or_else(|| v.as_integer().map(|i| i as f64));
    Ok(match &l.parallax {
        None => (1.0, 1.0),
        Some(toml::Value::Array(a)) if a.len() == 2 => match (f(&a[0]), f(&a[1])) {
            (Some(x), Some(y)) => (x, y),
            _ => return Err("parallax must be a number or [x, y]".into()),
        },
        Some(v) => match f(v) {
            Some(p) => (p, p),
            None => return Err("parallax must be a number or [x, y]".into()),
        },
    })
}

pub fn toml_to_value(v: &toml::Value) -> Result<Value, String> {
    Ok(match v {
        toml::Value::String(s) => Value::str(s),
        toml::Value::Integer(i) => Value::Num(*i as f64),
        toml::Value::Float(f) => Value::Num(*f),
        toml::Value::Boolean(b) => Value::Bool(*b),
        toml::Value::Array(a) => Value::array(a.iter().map(toml_to_value).collect::<Result<_, _>>()?),
        toml::Value::Table(t) => Value::new_struct(t.iter().map(|(k, v)| Ok((intern(k), toml_to_value(v)?))).collect::<Result<Vec<_>, String>>()?),
        toml::Value::Datetime(d) => Value::string(d.to_string()),
    })
}

// ================================================================ Host

impl Host for Engine {
    fn user_function(&self, name: Sym) -> Option<Rc<FuncDef>> {
        self.functions.get(&name).cloned()
    }

    fn has_native(&self, name: Sym) -> bool {
        self.natives.contains_key(&name)
    }

    fn call_native(&mut self, name: Sym, cx: &CallCx, args: Vec<Value>) -> RtResult<Value> {
        let f = *self.natives.get(&name).ok_or_else(|| RtError::new(format!("unknown function '{name}'")))?;
        f(self, cx, &args).map_err(|mut e| {
            if e.line == 0 && e.trace.is_empty() {
                e.msg = format!("{name}(): {}", e.msg);
            }
            e
        })
    }

    fn global_get(&mut self, name: Sym) -> Option<Value> {
        self.globals.get(&name).cloned()
    }

    fn global_set(&mut self, name: Sym, v: Value) {
        self.globals.insert(name, v);
    }

    fn inst_get(&mut self, id: u32, name: Sym) -> RtResult<Option<Value>> {
        let Some(i) = self.idx(id) else { return Err(RtError::new(format!("instance {id} does not exist (destroyed?)"))) };
        if let Some(bv) = self.builtin(name) {
            return Ok(Some(self.get_bv(i, bv)));
        }
        Ok(self.insts[i].vars.get(&name).cloned())
    }

    fn inst_set(&mut self, id: u32, name: Sym, v: Value) -> RtResult<()> {
        let Some(i) = self.idx(id) else { return Err(RtError::new(format!("instance {id} does not exist (destroyed?)"))) };
        if let Some(bv) = self.builtin(name) {
            return self.set_bv(i, bv, name, v);
        }
        self.insts[i].vars.insert(name, v);
        Ok(())
    }

    fn inst_alive(&self, id: u32) -> bool {
        self.alive(id)
    }

    fn instances_of(&mut self, v: &Value) -> Vec<u32> {
        self.resolve(v)
    }

    fn constant(&mut self, name: Sym) -> Option<Value> {
        if let Some(v) = self.assets.get(&name) {
            return Some(v.clone());
        }
        if let Some(v) = self.consts.get(&name) {
            return Some(v.clone());
        }
        let d = &self.dyn_syms;
        if name == d.mouse_x {
            Some(Value::Num(self.mouse_room().0))
        } else if name == d.mouse_y {
            Some(Value::Num(self.mouse_room().1))
        } else if name == d.room {
            Some(Value::Asset(AssetKind::Room, self.room as u32))
        } else if name == d.room_width {
            Some(Value::Num(self.room_w))
        } else if name == d.room_height {
            Some(Value::Num(self.room_h))
        } else {
            None
        }
    }
}
