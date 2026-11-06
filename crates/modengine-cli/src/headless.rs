//! Running a game without a window: `modengine server` / `modengine run --headless`
//! (dedicated multiplayer servers, tests). Only the logic runs — no draw events, no audio —
//! in real time at the room's step rate.

use crate::script_input::ScriptedInput;
use modengine::Engine;
use std::time::{Duration, Instant};

pub struct HeadlessOpts {
    pub input: Option<ScriptedInput>,
    /// Stop after this many steps (None: until the game calls quit()).
    pub steps: Option<u64>,
    /// Print where the instances of these objects are, every 30 steps.
    pub watch: Vec<String>,
    /// Sleep between steps to keep the game's speed (false: as fast as possible).
    pub realtime: bool,
}

/// `step N room R Obj: (x, y) ...` lines for `--watch`.
pub fn watch_lines(e: &Engine, watch: &[String]) -> Vec<String> {
    watch
        .iter()
        .map(|w| {
            let found: Vec<String> = e.insts.iter().filter(|i| i.alive && e.objects[i.object as usize].name == *w).map(|i| format!("({:.1}, {:.1})", i.x, i.y)).collect();
            format!("step {:>5} room {:<10} {w}: {}", e.steps, e.rooms[e.room].name, found.join(" "))
        })
        .collect()
}

/// Plays the game without a window until it quits (or `steps` run). Returns the exit code.
pub fn run(mut e: Engine, o: HeadlessOpts) -> i32 {
    println!("[headless] '{}' sin ventana, room '{}', {} pasos/s", e.config.game.name, e.rooms.get(e.start_room).map(|r| r.name.as_str()).unwrap_or("?"), e.step_rate());
    e.test_starting();
    e.start();
    e.test_started();
    let mut next = Instant::now();
    loop {
        if o.steps.is_some_and(|n| e.steps >= n) {
            break;
        }
        e.input.down.iter_mut().for_each(|d| *d = false);
        e.input.mouse_down = [false; 3];
        if let Some(si) = &o.input {
            si.apply_to(e.steps + 1, &mut e);
        }
        e.test_tick();
        e.audio.clear();
        if !o.watch.is_empty() && e.steps % 30 == 0 {
            for l in watch_lines(&e, &o.watch) {
                println!("{l}");
            }
        }
        if e.quit {
            println!("[headless] quit() en el paso {}", e.steps);
            break;
        }
        if o.realtime {
            next += Duration::from_secs_f64(1.0 / e.step_rate());
            let now = Instant::now();
            if next > now {
                std::thread::sleep(next - now);
            } else if now - next > Duration::from_secs(1) {
                // far behind (machine asleep, debugger): don't try to catch up
                next = now;
            }
        }
    }
    e.shutdown();
    println!("[headless] fin tras {} pasos", e.steps);
    let code = e.test_finish();
    if e.error_count() > 0 {
        eprintln!("{} runtime error(s)", e.error_count());
    }
    code
}
