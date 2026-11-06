//! Child processes. The editor is a window-only program on Windows, so a console program it
//! starts (modengine, git, curl, cargo, an agent CLI…) would open a console window of its own:
//! every child goes through [`command`], which asks Windows not to.

use std::ffi::OsStr;
use std::process::Command;

#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// `Command::new(program)` that never opens a console window.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}
