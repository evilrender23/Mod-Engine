//! Hooks for automated test runs from the terminal (`modengine check|run` flags, see
//! docs/pruebas.md): a fixed random seed, code run at given steps, assertions, test
//! scripts whose objects are created in every room, and a per-event profiler.
//!
//! Backends drive a test run with [`Engine::test_tick`] / [`Engine::test_render`] instead of
//! `tick()` / `render()`, and call [`Engine::test_finish`] at the end for the exit code.

use crate::engine::Engine;
use crate::object::Ev;
use modscript::{eval_expr, parse_expr, parse_stmts, run_stmts, CallCx, Expr, Stmt};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Exit code when an `--assert` failed (runtime errors give 1, bad flags 2).
pub const EXIT_ASSERT: i32 = 3;

/// When an `--assert` is checked.
#[derive(Clone, Debug, PartialEq)]
pub enum AssertWhen {
    /// At the end of the run (`--assert EXPR`).
    End,
    /// After logic step N (`--assert N:EXPR`).
    Step(u64),
    /// After every step (`--assert *:EXPR`).
    Every,
}

pub struct Assert {
    pub when: AssertWhen,
    pub src: String,
    expr: Rc<Expr>,
    checked: bool,
}

/// Time spent in one event of one object.
#[derive(Clone, Copy, Default, Debug)]
pub struct EventStat {
    pub calls: u64,
    /// Time inside the event itself (events it triggers are counted on their own).
    pub total: Duration,
    pub max: Duration,
}

#[derive(Default)]
pub struct Profile {
    pub events: HashMap<(u32, Ev), EventStat>,
    /// Child time of the events running now (to count each event's own time).
    stack: Vec<Duration>,
    pub tick: Duration,
    pub render: Duration,
    pub ticks: u64,
    pub renders: u64,
    /// When profiling started (the table compares with the time since then).
    pub since: Option<Instant>,
}

/// State of the test hooks (all off by default: a normal game is unaffected).
#[derive(Default)]
pub struct Testing {
    /// `--seed N`: the random generators start from N, and `randomize()` stays reproducible.
    pub seed: Option<u64>,
    randomize_calls: u64,
    /// Objects defined in `--test-script` files, created in every room that lacks them.
    pub spawn: Vec<u32>,
    /// `--exec-at STEP:CODE`, run after logic step STEP.
    exec_at: Vec<(u64, Rc<Vec<Stmt>>)>,
    pub asserts: Vec<Assert>,
    /// Messages of the failed assertions.
    pub failures: Vec<String>,
    /// `--profile`.
    pub profile: Option<Profile>,
}

fn splitmix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Splits `N:REST` into (Some(N), REST); anything else is (None, whole).
fn step_prefix(s: &str) -> (Option<&str>, &str) {
    match s.split_once(':') {
        Some((a, b)) if !a.trim().is_empty() && (a.trim() == "*" || a.trim().chars().all(|c| c.is_ascii_digit())) => (Some(a.trim()), b),
        _ => (None, s),
    }
}

/// Reads `--test-script` files: (name shown in errors, source).
pub fn read_test_scripts(files: &[PathBuf]) -> Result<Vec<(String, String)>, String> {
    files
        .iter()
        .map(|f| {
            let src = std::fs::read_to_string(f).map_err(|e| format!("--test-script {}: {e}", f.display()))?;
            Ok((f.display().to_string(), src))
        })
        .collect()
}

/// Loads a project folder or an exported game (like [`Engine::load`] / [`Engine::load_pack`])
/// plus extra ModScript files (`--test-script`). The objects those files define are
/// created automatically in every room (except names starting with `_`).
pub fn load_with_tests(path: &Path, test_scripts: &[PathBuf]) -> Result<Engine, String> {
    if test_scripts.is_empty() {
        return if path.is_file() { Engine::load_pack(path) } else { Engine::load(path) };
    }
    // a test script inside the project folder is already loaded with the project: it only
    // needs its objects created in every room
    let root = (!path.is_file()).then(|| path.canonicalize().ok()).flatten();
    let mut inside: Vec<String> = Vec::new();
    let mut outside: Vec<PathBuf> = Vec::new();
    for f in test_scripts {
        let rel = root.as_ref().and_then(|r| f.canonicalize().ok().and_then(|c| c.strip_prefix(r).ok().map(|p| p.to_string_lossy().replace('\\', "/"))));
        match rel {
            Some(rel) => inside.push(rel),
            None => outside.push(f.clone()),
        }
    }
    let extra = read_test_scripts(&outside)?;
    let build = |dir: &Path| -> Result<Engine, String> {
        let mut p = crate::project::Project::load(dir)?;
        p.scripts.extend(extra.iter().cloned());
        Engine::from_project(p)
    };
    let mut e = if path.is_file() {
        let src = crate::vfs::mount(path, crate::pack::Pack::open(path)?);
        let mut e = build(src.root())?;
        // same per-user save folder as Engine::load_source
        let pkg = e.config.android_package();
        let env = |k: &str| std::env::var(k).ok();
        e.save_dir = crate::vfs::user_data_dir(&e.config.game.name, Some(&pkg), &env).join("saves");
        e
    } else {
        build(path)?
    };
    let files: Vec<&str> = extra.iter().map(|(f, _)| f.as_str()).chain(inside.iter().map(String::as_str)).collect();
    e.testing.spawn = e.objects.iter().enumerate().filter(|(_, o)| files.contains(&&*o.file) && !o.name.starts_with('_')).map(|(i, _)| i as u32).collect();
    Ok(e)
}

impl Engine {
    /// `--seed N`: every random generator of the game starts from N (rand, irand, choose,
    /// shuffle, chance, particles, camera shake). `randomize()` then picks seeds derived
    /// from N instead of the clock, so runs repeat exactly.
    pub fn set_seed(&mut self, n: u64) {
        self.testing.seed = Some(n);
        self.testing.randomize_calls = 0;
        self.rng = splitmix(n) | 1;
        self.parts.world.rng = splitmix(n ^ 0x5DEE_CE66_D1CE_4E5B) | 1;
    }

    /// What `randomize()` does under `--seed`: a new seed, but always the same sequence.
    /// Returns false without a fixed seed (then the clock is used).
    pub(crate) fn seeded_randomize(&mut self) -> bool {
        let Some(n) = self.testing.seed else { return false };
        self.testing.randomize_calls += 1;
        self.rng = splitmix(n.wrapping_add(self.testing.randomize_calls.wrapping_mul(0xA24B_AED4_963E_E407))) | 1;
        true
    }

    /// `--exec-at STEP:CODE`: run CODE (global scope) right after logic step STEP, before it
    /// is drawn. Step 0 is right after the first room starts (like `--exec`).
    pub fn add_exec_at(&mut self, spec: &str) -> Result<(), String> {
        let (step, code) = step_prefix(spec);
        let step = step.filter(|s| *s != "*").ok_or_else(|| format!("--exec-at '{spec}': se esperaba PASO:CÓDIGO, p. ej. 120:with Jugador x = 50"))?;
        let step: u64 = step.parse().map_err(|_| format!("--exec-at '{spec}': paso no válido"))?;
        let stmts = parse_stmts("<exec-at>", code).map_err(|e| format!("--exec-at '{spec}': {e}"))?;
        self.testing.exec_at.push((step, Rc::new(stmts)));
        Ok(())
    }

    /// `--assert EXPR` (checked at the end), `--assert N:EXPR` (after step N) or
    /// `--assert *:EXPR` (after every step). A false value or an error is a failure.
    pub fn add_assert(&mut self, spec: &str) -> Result<(), String> {
        let (step, src) = step_prefix(spec);
        let when = match step {
            None => AssertWhen::End,
            Some("*") => AssertWhen::Every,
            Some(n) => AssertWhen::Step(n.parse().map_err(|_| format!("--assert '{spec}': paso no válido"))?),
        };
        let expr = parse_expr("<assert>", src.trim()).map_err(|e| format!("--assert '{spec}': {e}"))?;
        self.testing.asserts.push(Assert { when, src: src.trim().to_string(), expr: Rc::new(expr), checked: false });
        Ok(())
    }

    /// `--profile`: time every event of every object from now on.
    pub fn enable_profile(&mut self) {
        self.testing.profile = Some(Profile { since: Some(Instant::now()), ..Default::default() });
    }

    /// Total number of different runtime errors so far (`errors` only keeps the last few).
    pub fn error_count(&self) -> usize {
        self.seen_errors.len()
    }

    /// Called when a room has been entered (after its instances and creation code, before
    /// `room_start`): creates the test objects the room does not have yet. Persistent ones
    /// carried over from the previous room are not duplicated.
    pub(crate) fn test_room_entered(&mut self) {
        for o in self.testing.spawn.clone() {
            if self.insts.iter().any(|i| i.alive && i.object == o) {
                continue;
            }
            let id = self.create_instance(o, 0.0, 0.0, 0.0);
            self.init_instance(id, &[], None, modscript::Scope::Global);
        }
    }

    /// One logic step plus the test hooks: `--exec-at` and `--assert` of this step, and the
    /// `--profile` timing. Use it instead of `tick()` in test drivers.
    pub fn test_tick(&mut self) {
        let t0 = Instant::now();
        self.tick();
        if let Some(p) = &mut self.testing.profile {
            p.tick += t0.elapsed();
            p.ticks += 1;
        }
        self.after_step();
    }

    /// `render()` plus `--profile` timing.
    pub fn test_render(&mut self) {
        let t0 = Instant::now();
        self.render();
        if let Some(p) = &mut self.testing.profile {
            p.render += t0.elapsed();
            p.renders += 1;
        }
    }

    /// Call right before `start()`: the `--profile` clock starts here (after loading assets).
    pub fn test_starting(&mut self) {
        if let Some(p) = &mut self.testing.profile {
            p.since = Some(Instant::now());
        }
    }

    /// Runs the `--exec-at 0:` code and step-0 asserts (call right after `start()`).
    pub fn test_started(&mut self) {
        self.after_step();
    }

    fn after_step(&mut self) {
        let step = self.steps;
        let due: Vec<Rc<Vec<Stmt>>> = self.testing.exec_at.iter().filter(|(s, _)| *s == step).map(|(_, c)| c.clone()).collect();
        for code in due {
            if let Err(e) = run_stmts(self, &code, &Rc::from("<exec-at>"), CallCx::global()) {
                self.report(e);
            }
        }
        for i in 0..self.testing.asserts.len() {
            let due = match self.testing.asserts[i].when {
                AssertWhen::Step(n) => n == step,
                AssertWhen::Every => !self.testing.asserts[i].checked,
                AssertWhen::End => false,
            };
            if due {
                self.check_assert(i, &format!("en el paso {step}"));
            }
        }
    }

    fn check_assert(&mut self, i: usize, at: &str) {
        let (expr, src) = (self.testing.asserts[i].expr.clone(), self.testing.asserts[i].src.clone());
        let failed = match eval_expr(self, &expr, &Rc::from("<assert>"), CallCx::global()) {
            Ok(v) if v.truthy() => None,
            Ok(v) => Some(format!("vale {}", v.to_string())),
            Err(e) => Some(format!("error: {e}")),
        };
        let a = &mut self.testing.asserts[i];
        match failed {
            Some(why) => {
                // `*:` asserts report only their first failure
                a.checked = true;
                let msg = format!("--assert fallido {at} (room '{}'): {src}  ({why})", self.rooms[self.room].name);
                eprintln!("[assert] {msg}");
                self.testing.failures.push(msg);
            }
            None => {
                if a.when != AssertWhen::Every {
                    a.checked = true;
                }
            }
        }
    }

    /// End of a test run: checks the end-of-run asserts and the step asserts that never
    /// ran, prints the `--profile` table, and returns the exit code: 0 all good,
    /// [`EXIT_ASSERT`] (3) an assert failed, 1 runtime errors.
    pub fn test_finish(&mut self) -> i32 {
        for i in 0..self.testing.asserts.len() {
            match self.testing.asserts[i].when {
                AssertWhen::End => self.check_assert(i, &format!("al final (paso {})", self.steps)),
                AssertWhen::Step(n) if !self.testing.asserts[i].checked => {
                    let msg = format!("--assert {n}:{} no se comprobó: la ejecución terminó en el paso {}", self.testing.asserts[i].src, self.steps);
                    eprintln!("[assert] {msg}");
                    self.testing.failures.push(msg);
                }
                _ => {}
            }
        }
        if let Some(table) = self.profile_report() {
            println!("{table}");
        }
        let passed = self.testing.asserts.len() - self.testing.failures.len().min(self.testing.asserts.len());
        if !self.testing.asserts.is_empty() {
            if self.testing.failures.is_empty() {
                println!("asserts: {} de {} correctos", passed, self.testing.asserts.len());
            } else {
                eprintln!("asserts: {} fallido(s) de {}", self.testing.failures.len(), self.testing.asserts.len());
            }
        }
        if !self.testing.failures.is_empty() {
            EXIT_ASSERT
        } else if self.error_count() > 0 {
            1
        } else {
            0
        }
    }

    /// The `--profile` tables (None when profiling is off).
    pub fn profile_report(&self) -> Option<String> {
        let p = self.testing.profile.as_ref()?;
        Some(format_profile(p, &|o| self.objects.get(o as usize).map(|d| d.name.clone()).unwrap_or_else(|| format!("#{o}")), &|ev| self.event_label(ev)))
    }

    fn event_label(&self, ev: Ev) -> String {
        let key = |k: u16| if k == crate::object::ANY_KEY { "any".to_string() } else { crate::input::KEY_NAMES.get(k as usize).unwrap_or(&"?").to_string() };
        let btn = |b: u8| crate::input::MOUSE_BUTTONS.get(b as usize).unwrap_or(&"?").to_string();
        match ev {
            Ev::Alarm(n) => format!("alarm({n})"),
            Ev::Collision(o) => format!("collision({})", self.objects.get(o as usize).map(|d| d.name.as_str()).unwrap_or("?")),
            Ev::Key(k) => format!("key({})", key(k)),
            Ev::KeyPressed(k) => format!("key_pressed({})", key(k)),
            Ev::KeyReleased(k) => format!("key_released({})", key(k)),
            Ev::MouseDown(b) => format!("mouse_down({})", btn(b)),
            Ev::MousePressed(b) => format!("mouse_pressed({})", btn(b)),
            Ev::MouseReleased(b) => format!("mouse_released({})", btn(b)),
            Ev::GlobalMouseDown(b) => format!("global_mouse_down({})", btn(b)),
            Ev::GlobalMousePressed(b) => format!("global_mouse_pressed({})", btn(b)),
            Ev::GlobalMouseReleased(b) => format!("global_mouse_released({})", btn(b)),
            Ev::User(n) => format!("user({n})"),
            other => {
                // "StepBegin" -> "step_begin", the name used in `on step_begin`
                let dbg = format!("{other:?}");
                let snake: String = dbg.chars().enumerate().flat_map(|(i, c)| if c.is_uppercase() && i > 0 { vec!['_', c.to_ascii_lowercase()] } else { vec![c.to_ascii_lowercase()] }).collect();
                snake
            }
        }
    }
}

/// Start of an event for `--profile` (None when profiling is off).
pub(crate) fn prof_begin(e: &mut Engine, id: u32) -> Option<(Instant, u32)> {
    let p = e.testing.profile.as_mut()?;
    p.stack.push(Duration::ZERO);
    let obj = e.idx(id).map(|i| e.insts[i].object)?;
    Some((Instant::now(), obj))
}

/// End of an event started with [`prof_begin`].
pub(crate) fn prof_end(e: &mut Engine, started: Option<(Instant, u32)>, ev: Ev) {
    let Some(p) = e.testing.profile.as_mut() else { return };
    let children = p.stack.pop().unwrap_or_default();
    let Some((t0, obj)) = started else { return };
    let total = t0.elapsed();
    let own = total.saturating_sub(children);
    if let Some(parent) = p.stack.last_mut() {
        *parent += total;
    }
    let s = p.events.entry((obj, ev)).or_default();
    s.calls += 1;
    s.total += own;
    s.max = s.max.max(own);
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn format_profile(p: &Profile, obj_name: &dyn Fn(u32) -> String, ev_name: &dyn Fn(Ev) -> String) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let events_total: Duration = p.events.values().map(|s| s.total).sum();
    let wall = p.since.map(|t| t.elapsed()).unwrap_or(p.tick + p.render).max(p.tick + p.render);
    let _ = writeln!(out, "\n=== perfil (--profile) ===");
    let _ = writeln!(
        out,
        "pasos lógicos: {}  ({:.2} ms en total, {:.3} ms/paso)   dibujado: {} ({:.2} ms, {:.3} ms/frame)",
        p.ticks,
        ms(p.tick),
        if p.ticks > 0 { ms(p.tick) / p.ticks as f64 } else { 0.0 },
        p.renders,
        ms(p.render),
        if p.renders > 0 { ms(p.render) / p.renders as f64 } else { 0.0 }
    );
    let _ = writeln!(out, "tiempo total: {:.2} ms (desde el arranque: carga de la room, pasos, dibujado y, con ventana, la GPU)", ms(wall));
    let _ = writeln!(
        out,
        "en eventos: {:.2} ms ({:.0}% del total); el resto es el motor (movimiento, colisiones, partículas, cámara, GPU...)",
        ms(events_total),
        if wall.is_zero() { 0.0 } else { 100.0 * events_total.as_secs_f64() / wall.as_secs_f64() }
    );
    let mut rows: Vec<(&(u32, Ev), &EventStat)> = p.events.iter().collect();
    rows.sort_by(|a, b| b.1.total.cmp(&a.1.total).then_with(|| a.0 .0.cmp(&b.0 .0)));
    let pct = |d: Duration| if events_total.is_zero() { 0.0 } else { 100.0 * d.as_secs_f64() / events_total.as_secs_f64() };
    let _ = writeln!(out, "\npor evento (tiempo propio, sin los eventos que dispara):");
    let _ = writeln!(out, "{:<24} {:<26} {:>9} {:>11} {:>10} {:>10} {:>6}", "objeto", "evento", "llamadas", "total ms", "media µs", "máx µs", "%");
    for ((o, ev), s) in &rows {
        let _ = writeln!(
            out,
            "{:<24} {:<26} {:>9} {:>11.3} {:>10.2} {:>10.1} {:>6.1}",
            obj_name(*o),
            ev_name(*ev),
            s.calls,
            ms(s.total),
            s.total.as_secs_f64() * 1e6 / s.calls.max(1) as f64,
            s.max.as_secs_f64() * 1e6,
            pct(s.total)
        );
    }
    let mut per_obj: HashMap<u32, (u64, Duration)> = HashMap::new();
    for ((o, _), s) in &rows {
        let e = per_obj.entry(*o).or_default();
        e.0 += s.calls;
        e.1 += s.total;
    }
    let mut objs: Vec<(u32, (u64, Duration))> = per_obj.into_iter().collect();
    objs.sort_by(|a, b| b.1 .1.cmp(&a.1 .1).then_with(|| a.0.cmp(&b.0)));
    let _ = writeln!(out, "\npor objeto:");
    let _ = writeln!(out, "{:<24} {:>9} {:>11} {:>6}", "objeto", "llamadas", "total ms", "%");
    for (o, (calls, total)) in objs {
        let _ = writeln!(out, "{:<24} {:>9} {:>11.3} {:>6.1}", obj_name(o), calls, ms(total), pct(total));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_prefixes() {
        assert_eq!(step_prefix("120:x = 1"), (Some("120"), "x = 1"));
        assert_eq!(step_prefix("*:a > 0"), (Some("*"), "a > 0"));
        assert_eq!(step_prefix("a ? b : c"), (None, "a ? b : c"));
        assert_eq!(step_prefix("{a: 1}.a == 1"), (None, "{a: 1}.a == 1"));
    }
}
