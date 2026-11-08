//! The integration tests' stand-in for a `#!/bin/sh` program on Windows. The tests put fake
//! programs on `PATH` as shell scripts (`git`, `claude`, `sudo`, ...), and Windows starts only
//! a program with an extension: a copy of this shim, `<name>.exe`, runs the script `<name>`
//! beside it through Git for Windows' `sh.exe`, with the same arguments, streams and exit
//! status. A script that only hands every call to one program
//! (`#!/bin/sh\nexec "<program>" "$@"\n`) starts that program directly, so the box's plain
//! `git` costs no shell per call. On Unix the tests run the scripts themselves.
//!
//! `sh.exe` is `BAY_TEST_SH`, else the `sh.path` file beside the shim, else `sh` on `PATH`.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let shim = match std::env::current_exe() {
        Ok(shim) => shim,
        Err(error) => return fail(&format!("sh_shim: cannot find itself: {error}")),
    };
    let script = shim.with_extension("");
    let text = match std::fs::read_to_string(&script) {
        Ok(text) => text,
        Err(error) => return fail(&format!("sh_shim: {}: {error}", script.display())),
    };
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let mut cmd = if let Some(program) = handed_to(&text) {
        Command::new(program)
    } else {
        let Some(sh) = sh(&shim) else {
            return fail("sh_shim: no sh.exe: set BAY_TEST_SH or put sh on PATH");
        };
        let mut cmd = Command::new(sh);
        cmd.arg(&script);
        // sh.exe expands globs and braces in the command line a Windows program hands it, so
        // `main^{commit}` would reach the script as `main^commit`: `noglob` passes every
        // argument as given.
        let msys = std::env::var("MSYS").unwrap_or_default();
        cmd.env("MSYS", format!("{msys} noglob").trim_start());
        cmd
    };
    match cmd.args(args).status() {
        Ok(status) => status
            .code()
            .and_then(|code| u8::try_from(code).ok())
            .map_or(ExitCode::FAILURE, ExitCode::from),
        Err(error) => fail(&format!("sh_shim: {}: {error}", script.display())),
    }
}

/// The one program a pass-through script hands every call to.
fn handed_to(text: &str) -> Option<&str> {
    text.strip_prefix("#!/bin/sh\nexec \"")?
        .strip_suffix("\" \"$@\"\n")
        .filter(|program| !program.contains(['"', '$', '`', '\\']))
}

fn sh(shim: &Path) -> Option<PathBuf> {
    std::env::var_os("BAY_TEST_SH")
        .map(PathBuf::from)
        .or_else(|| {
            let beside = shim.parent()?.join("sh.path");
            let text = std::fs::read_to_string(beside).ok()?;
            Some(PathBuf::from(text.trim()))
        })
        .or_else(|| {
            bay::platform::resolve(
                "sh",
                std::env::var_os("PATH").as_deref(),
                std::env::var_os("PATHEXT").as_deref(),
            )
        })
}

fn fail(message: &str) -> ExitCode {
    eprintln!("{message}");
    ExitCode::from(127)
}
