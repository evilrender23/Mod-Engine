//! Mod Engine runtime on macroquad, shared by the `modengine` command line, the
//! `modengine-runner` of exported games and the Android library (`libmodengine_cli.so`).

pub mod backend;
pub mod gpu_fx;
pub mod runner;
pub mod script_input;
pub mod touch;

#[cfg(not(target_os = "android"))]
pub mod export;
#[cfg(not(target_os = "android"))]
pub mod shader_tool;

/// Entry point miniquad's Android activity calls (see `android/MainActivity.java`).
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn quad_main() {
    runner::android_main();
}
