//! `modengine-runner`: plays the game packed into it (or `game.dat` next to it).
//! `modengine export` copies it as `<Game>` / `<Game>.exe` with the game appended.

// exported games open only their window, no console (export can switch it back on)
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    modengine_cli::runner::desktop_main();
}
