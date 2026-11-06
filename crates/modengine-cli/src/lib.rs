//! Mod Engine runtime on macroquad, shared by the `modengine` command line, the
//! `modengine-runner` of exported games and the Android library (`libmodengine_cli.so`).

pub mod backend;
pub mod canvas;
pub mod gpu_fx;
pub mod headless;
pub mod runner;
pub mod scissor;
pub mod script_input;
pub mod sprite_draw;
pub mod test_run;
pub mod touch;

#[cfg(not(target_os = "android"))]
pub mod export;
#[cfg(not(target_os = "android"))]
pub mod import_cli;
#[cfg(not(target_os = "android"))]
pub mod pixel_check;
#[cfg(not(target_os = "android"))]
pub mod shader_tool;
#[cfg(not(target_os = "android"))]
pub mod sprite_bake;
#[cfg(not(target_os = "android"))]
pub mod store_cli;

/// Entry point miniquad's Android activity calls (see `android/MainActivity.java`).
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn quad_main() {
    runner::android_main();
}
