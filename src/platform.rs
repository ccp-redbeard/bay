//! What differs between Unix and Windows where bay starts a process or names a path (`BACKLOG`
//! Launch requirements, plan W1). On Unix each of these is what bay did before it built for
//! Windows: the home directory is `$HOME`, a program is found by the system's own lookup, and a
//! detached process leads a process group of its own.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

/// This user's home directory: `HOME`, and on Windows, where `HOME` is usually unset,
/// `USERPROFILE`. `None` when neither is set, so a caller names what it cannot find.
#[must_use]
pub fn home() -> Option<OsString> {
    let home = std::env::var_os("HOME");
    if cfg!(windows) {
        home.filter(|home| !home.is_empty())
            .or_else(|| std::env::var_os("USERPROFILE").filter(|home| !home.is_empty()))
    } else {
        home
    }
}

/// The program to start for `name`. On Windows a bare name is looked up on `PATH` with each
/// extension of `PATHEXT` (so `claude` finds `claude.cmd`, which `Command` alone does not);
/// a name not found is returned as it is, so starting it fails as not found. On Unix the name
/// is returned as it is.
#[must_use]
pub fn program(name: &str) -> PathBuf {
    if cfg!(windows) {
        resolve(
            name,
            std::env::var_os("PATH").as_deref(),
            std::env::var_os("PATHEXT").as_deref(),
        )
        .unwrap_or_else(|| PathBuf::from(name))
    } else {
        PathBuf::from(name)
    }
}

/// Keeps this process's standard handles to itself on Windows. A handle a Windows process
/// was given is inheritable, and every process bay starts inherits each inheritable handle,
/// whatever stdio it is given. A detached process bay leaves running (a checkpoint, a poll)
/// then holds the caller's pipe open, and a caller reading bay's output to its end, as an
/// agent reads a hook's, waits until that process ends. A child given bay's stdio still gets
/// it: `Command` passes it a copy of its own.
#[allow(unsafe_code)]
pub fn keep_std_handles() {
    #[cfg(windows)]
    {
        use std::os::windows::io::{AsRawHandle as _, RawHandle};
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn SetHandleInformation(handle: RawHandle, mask: u32, flags: u32) -> i32;
        }
        const HANDLE_FLAG_INHERIT: u32 = 1;
        for handle in [
            std::io::stdin().as_raw_handle(),
            std::io::stdout().as_raw_handle(),
            std::io::stderr().as_raw_handle(),
        ] {
            if !handle.is_null() {
                // SAFETY: the handle is this process's own standard handle, open for its
                // lifetime; only its inherit flag changes. A failure leaves it as it was.
                unsafe {
                    SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0);
                }
            }
        }
    }
}

/// A `git` command with no arguments yet. On Unix, and for any `git` but Git for Windows'
/// launcher, `git` started by name. Git for Windows puts its launcher, `<root>\cmd\git.exe`, on
/// `PATH`; the launcher starts `<root>\<arch>\bin\git.exe` with `MSYSTEM` set and Git's own
/// directories first on `PATH`. Each call then costs two process starts, and a process start
/// is slow on Windows. bay starts that git itself, with the environment the launcher gives it.
#[must_use]
pub fn git() -> Command {
    let Some(direct) = git_direct() else {
        return Command::new("git");
    };
    let mut cmd = Command::new(&direct.program);
    let mut path = direct.path_prefix.clone();
    if let Some(rest) = std::env::var_os("PATH") {
        path.push(rest);
    }
    cmd.env("PATH", path);
    if std::env::var_os("MSYSTEM").is_none() {
        cmd.env("MSYSTEM", direct.msystem);
    }
    cmd
}

/// Whether `program` is the `git` that [`git`] starts.
#[must_use]
pub fn is_git(program: &OsStr) -> bool {
    program == "git" || git_direct().is_some_and(|direct| direct.program == program)
}

/// Git for Windows' git, which its launcher would start, and what the launcher puts first on
/// `PATH`.
struct GitDirect {
    program: PathBuf,
    /// `<root>\<arch>\bin;<root>\usr\bin;<home>\bin;`, as the launcher puts it.
    path_prefix: OsString,
    msystem: &'static str,
}

/// [`GitDirect`] when the `git` on `PATH` is Git for Windows' launcher, found once: bay never
/// changes its own `PATH`.
fn git_direct() -> Option<&'static GitDirect> {
    static DIRECT: std::sync::OnceLock<Option<GitDirect>> = std::sync::OnceLock::new();
    DIRECT
        .get_or_init(|| {
            if !cfg!(windows) {
                return None;
            }
            let launcher = resolve(
                "git",
                std::env::var_os("PATH").as_deref(),
                std::env::var_os("PATHEXT").as_deref(),
            )?;
            git_behind_launcher(&launcher, home().as_deref())
        })
        .as_ref()
}

/// The git that `launcher` starts, when `launcher` is Git for Windows' `<root>\cmd\git.exe`
/// and that git is installed beside it.
fn git_behind_launcher(launcher: &Path, home: Option<&OsStr>) -> Option<GitDirect> {
    let named = |path: &Path, name: &str| {
        path.file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|file| file.eq_ignore_ascii_case(name))
    };
    let cmd_dir = launcher.parent()?;
    if !named(launcher, "git.exe") || !named(cmd_dir, "cmd") {
        return None;
    }
    let root = cmd_dir.parent()?;
    [
        ("mingw64", "MINGW64"),
        ("clangarm64", "CLANGARM64"),
        ("mingw32", "MINGW32"),
    ]
    .into_iter()
    .find_map(|(arch, msystem)| {
        let bin = root.join(arch).join("bin");
        let program = bin.join("git.exe");
        if !program.is_file() {
            return None;
        }
        let mut path_prefix = OsString::new();
        let mut dirs = vec![bin, root.join("usr").join("bin")];
        dirs.extend(home.map(|home| Path::new(home).join("bin")));
        for dir in dirs {
            path_prefix.push(dir.as_os_str());
            path_prefix.push(";");
        }
        Some(GitDirect {
            program,
            path_prefix,
            msystem,
        })
    })
}

/// The Windows lookup of `name` on `path` with the extensions of `pathext` (`;`-separated,
/// `.COM;.EXE;.BAT;.CMD` when unset): in each directory of `path` in order, the name as given
/// when it already has an extension of `pathext`, else the name with each extension in turn.
/// A name with a directory in it is not looked up.
#[must_use]
pub fn resolve(name: &str, path: Option<&OsStr>, pathext: Option<&OsStr>) -> Option<PathBuf> {
    if name.is_empty() || name.contains(['/', '\\']) {
        return None;
    }
    let pathext = pathext
        .and_then(OsStr::to_str)
        .filter(|text| !text.trim().is_empty())
        .unwrap_or(".COM;.EXE;.BAT;.CMD");
    let extensions: Vec<&str> = pathext
        .split(';')
        .map(str::trim)
        .filter(|ext| ext.starts_with('.') && ext.len() > 1)
        .collect();
    let has_extension = Path::new(name)
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|ext| {
            extensions
                .iter()
                .any(|known| known[1..].eq_ignore_ascii_case(ext))
        });
    let candidates: Vec<String> = if has_extension {
        vec![name.to_owned()]
    } else {
        extensions
            .iter()
            .map(|ext| format!("{name}{}", ext.to_ascii_lowercase()))
            .collect()
    };
    std::env::split_paths(path?)
        .filter(|dir| !dir.as_os_str().is_empty())
        .find_map(|dir| {
            candidates
                .iter()
                .map(|candidate| dir.join(candidate))
                .find(|candidate| candidate.is_file())
        })
}

/// Detaches `cmd` from the caller as a process of its own that outlives it: on Unix the leader
/// of a new process group, on Windows a process in a new process group with a console of its
/// own that has no window.
pub fn detach(cmd: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        cmd.process_group(0);
    }
    detach_windows(cmd);
}

/// [`detach`] for a process bay started detached on Windows only: on Unix nothing changes.
///
/// The process gets a console with no window (`CREATE_NO_WINDOW`), not none at all
/// (`DETACHED_PROCESS`): a process with no console gives every console program it starts, each
/// `git` and `sh`, a new console window of its own, which flashes open and shut on the screen.
/// Under a windowless console its children share that console and nothing shows. The new
/// process group keeps the caller's Ctrl+C from reaching it.
pub fn detach_windows(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

/// Waits for `child` on a thread of its own, so a long-running parent keeps no exited child
/// (G575, a fleet finding, 2026-10-29: a runner held 32 zombies, one for each hourly
/// `bay update` it started). A short-lived parent exits first and leaves the child to init.
pub fn reap(mut child: Child) {
    std::thread::spawn(move || {
        let _ = child.wait();
    });
}

/// A search path from `dirs`, joined with this platform's separator: `;` on Windows, where a
/// directory holds a `:` after its drive letter, `:` elsewhere.
///
/// On Windows a script runs under Git for Windows' `sh.exe`, whose `/usr/bin` holds the POSIX
/// tools but not `git.exe`; Git keeps that in `/mingw64/bin`, and its own `sh` launcher puts
/// that first. So `/usr/bin` there brings `/mingw64/bin` just before it, and a policy's system
/// directories hold Git as they do on Unix.
#[must_use]
pub fn search_path(dirs: &[String]) -> String {
    if !cfg!(windows) {
        return dirs.join(":");
    }
    let mut out: Vec<&str> = Vec::with_capacity(dirs.len() + 1);
    for dir in dirs {
        if dir == "/usr/bin" && !dirs.iter().any(|named| named == MINGW_BIN) {
            out.push(MINGW_BIN);
        }
        out.push(dir);
    }
    out.join(";")
}

/// Where Git for Windows keeps `git.exe`, as its `sh.exe` names the folder.
const MINGW_BIN: &str = "/mingw64/bin";

/// How an adapter's program starts, read from its first line (plan W6).
#[derive(Debug, PartialEq, Eq)]
pub enum Launch {
    /// No `#!` line: the program starts by itself.
    Direct,
    /// A `#!` line naming `sh` or `bash`, with the one argument the line passes, if any: on
    /// Windows the script runs through Git for Windows' `sh.exe`.
    Sh(Option<String>),
    /// A `#!` line naming another interpreter, as written.
    Other(String),
}

/// The [`Launch`] of a program whose first bytes are `head`. A `#!` line is the interpreter
/// path and at most one argument, as Unix reads it; `/usr/bin/env sh` names `sh`.
#[must_use]
pub fn launch_of(head: &[u8]) -> Launch {
    let Some(line) = head.strip_prefix(b"#!") else {
        return Launch::Direct;
    };
    let line = line.split(|byte| *byte == b'\n').next().unwrap_or_default();
    let line = String::from_utf8_lossy(line);
    let line = line.trim();
    let (interpreter, arg) = line
        .split_once([' ', '\t'])
        .map_or((line, None), |(interpreter, arg)| {
            (interpreter, Some(arg.trim()).filter(|arg| !arg.is_empty()))
        });
    let name = |path: &str| {
        path.rsplit(['/', '\\'])
            .next()
            .unwrap_or_default()
            .to_owned()
    };
    let shell = |name: &str| matches!(name, "sh" | "bash");
    match (name(interpreter).as_str(), arg) {
        ("env", Some(program)) if shell(program) => Launch::Sh(None),
        (program, arg) if shell(program) => Launch::Sh(arg.map(str::to_owned)),
        _ => Launch::Other(line.to_owned()),
    }
}

/// Where Git for Windows keeps `sh.exe`, given Git's exec path (`git --exec-path`, as
/// `C:/Program Files/Git/mingw64/libexec/git-core`): `usr/bin/sh.exe`, then `bin/sh.exe`, in
/// each directory above the exec path, nearest first. `usr/bin/sh.exe` is the shell itself;
/// `bin/sh.exe` is a launcher that puts Git's own directories (and `~/bin`) ahead of `PATH`,
/// so a check would not run under the `PATH` its policy names.
#[must_use]
pub fn sh_candidates(exec_path: &Path) -> Vec<PathBuf> {
    exec_path
        .ancestors()
        .skip(1)
        .take(3)
        .flat_map(|dir| {
            [
                dir.join("usr").join("bin").join("sh.exe"),
                dir.join("bin").join("sh.exe"),
            ]
        })
        .collect()
}

/// Why a shebang adapter does not start on Windows when no `sh.exe` is found.
pub const NO_SH: &str = "the plugin is a #! script, and Windows runs one only through \
     Git for Windows' sh.exe, which was not found above git --exec-path";

/// The command that starts the adapter program at `program`. On Unix the program itself. On
/// Windows a `#!` script naming `sh` or `bash` runs through Git for Windows' `sh.exe` (plan W6);
/// a script naming another interpreter, or one with no `sh.exe` to run it, is refused by name.
/// A program that cannot be read starts as it is, so starting it fails as it would.
///
/// # Errors
///
/// On Windows only: a `#!` script that has no interpreter bay can start.
pub fn script(program: &Path) -> Result<Command, String> {
    if !cfg!(windows) {
        return Ok(Command::new(program));
    }
    let mut head = [0u8; 256];
    let read = std::fs::File::open(program)
        .and_then(|mut file| std::io::Read::read(&mut file, &mut head))
        .unwrap_or(0);
    match launch_of(&head[..read]) {
        Launch::Direct => Ok(Command::new(program)),
        Launch::Sh(arg) => {
            let sh = git_sh().ok_or_else(|| format!("{}: {NO_SH}", program.display()))?;
            let mut cmd = Command::new(sh);
            cmd.args(arg).arg(sh_word(program));
            Ok(cmd)
        }
        Launch::Other(line) => Err(format!(
            "{}: the plugin's #! line names {line}, and on Windows bay runs only a sh or bash \
             script (through Git for Windows' sh.exe) or a program that starts by itself",
            program.display()
        )),
    }
}

/// `path` as Git for Windows' `sh.exe` reads a script's path: with no verbatim `\\?\` prefix,
/// which it cannot open, and `/` for `\`, which it reads as escapes. `C:/dir/script` names the
/// same file to it as `C:\dir\script` does to Windows.
#[must_use]
pub fn sh_word(path: &Path) -> String {
    let text = path.to_string_lossy();
    let text = text
        .strip_prefix(r"\\?\UNC\")
        .map(|rest| format!(r"\\{rest}"))
        .or_else(|| text.strip_prefix(r"\\?\").map(str::to_owned))
        .unwrap_or_else(|| text.into_owned());
    text.replace('\\', "/")
}

/// An argument for a plugin, which may be a script `sh.exe` runs: on Windows a verbatim path
/// (`\\?\C:\...`, as `canonicalize` gives) as [`sh_word`] spells it, which sh and Windows
/// programs alike open; any other argument as it is.
#[must_use]
pub fn plain_arg(arg: &OsStr) -> OsString {
    match arg.to_str() {
        Some(text) if cfg!(windows) && text.starts_with(r"\\?\") => sh_word(Path::new(text)).into(),
        _ => arg.to_owned(),
    }
}

/// Git for Windows' `sh.exe`, found from `git --exec-path` once it is found: a lookup that
/// fails (a `git` that timed out, say) is tried again by the next caller, so a long-running
/// runner is never left without a shell until it restarts.
fn git_sh() -> Option<PathBuf> {
    static SH: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    if let Some(sh) = SH.get() {
        return Some(sh.clone());
    }
    let mut cmd = git();
    cmd.arg("--exec-path");
    let output = crate::git::run_bounded(cmd, "git --exec-path").ok()?;
    let exec_path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !output.status.success() || exec_path.is_empty() {
        return None;
    }
    let sh = sh_candidates(Path::new(&exec_path))
        .into_iter()
        .find(|candidate| candidate.is_file())?;
    Some(SH.get_or_init(|| sh).clone())
}

/// Has `cmd`, when it starts Git for Windows' `sh.exe`, pass its arguments as given. sh.exe
/// glob- and brace-expands the command line a Windows program hands it, against its working
/// directory: a check's `scripts/check tests/*` would be expanded by the candidate's own file
/// names, where Unix passes it as written. Called after the caller has set the environment,
/// which may have been cleared. Any other program is left alone.
pub fn sh_args_as_given(cmd: &mut Command) {
    if !cfg!(windows) {
        return;
    }
    let is_sh = Path::new(cmd.get_program())
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("sh.exe"));
    if is_sh {
        let msys = cmd
            .get_envs()
            .find(|(key, _)| key.eq_ignore_ascii_case("MSYS"))
            .and_then(|(_, value)| value)
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();
        cmd.env("MSYS", format!("{msys} noglob").trim_start());
    }
}

/// One of the POSIX tools Git for Windows keeps in its `usr\bin` (`df`, say), found from the
/// `sh.exe` bay runs scripts through: in that directory, or in `usr\bin` beside it when sh came
/// from Git's `bin`. `None` when there is no such sh or no such tool.
#[must_use]
pub fn git_tool(name: &str) -> Option<PathBuf> {
    let sh = git_sh()?;
    let dir = sh.parent()?;
    let file = format!("{name}.exe");
    [
        dir.join(&file),
        dir.parent()?.join("usr").join("bin").join(&file),
    ]
    .into_iter()
    .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::{Launch, git_behind_launcher, launch_of, resolve, sh_candidates, sh_word};
    use std::ffi::OsStr;

    #[test]
    fn git_for_windows_launcher_is_skipped_for_the_git_beside_it() {
        let root = tempfile::tempdir().unwrap();
        let file = |path: std::path::PathBuf| {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "").unwrap();
            path
        };
        let launcher = file(root.path().join("cmd").join("git.exe"));
        // No git beside the launcher: it is started as it is.
        assert!(git_behind_launcher(&launcher, None).is_none());
        let git = file(root.path().join("mingw64").join("bin").join("git.exe"));
        let direct = git_behind_launcher(&launcher, Some(OsStr::new("H"))).unwrap();
        assert_eq!(direct.program, git);
        assert_eq!(direct.msystem, "MINGW64");
        let bin = root.path().join("mingw64").join("bin");
        let usr = root.path().join("usr").join("bin");
        let home = std::path::Path::new("H").join("bin");
        assert_eq!(
            direct.path_prefix,
            OsStr::new(&format!(
                "{};{};{};",
                bin.display(),
                usr.display(),
                home.display()
            ))
        );
        // A git.exe in any other directory is not a launcher.
        let other = file(root.path().join("bin").join("git.exe"));
        assert!(git_behind_launcher(&other, None).is_none());
    }

    fn dirs() -> (tempfile::TempDir, tempfile::TempDir) {
        (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap())
    }

    fn joined(dirs: &[&std::path::Path]) -> std::ffi::OsString {
        std::env::join_paths(dirs).unwrap()
    }

    #[test]
    fn only_sh_is_told_to_take_its_arguments_as_given() {
        let msys = |cmd: &std::process::Command| {
            cmd.get_envs()
                .find(|(key, _)| *key == "MSYS")
                .and_then(|(_, value)| value)
                .map(|value| value.to_string_lossy().into_owned())
        };
        let mut sh = std::process::Command::new(r"C:\Git\usr\bin\sh.exe");
        sh.env_clear().env("MSYS", "winsymlinks");
        super::sh_args_as_given(&mut sh);
        let mut other = std::process::Command::new("checker.exe");
        super::sh_args_as_given(&mut other);
        assert_eq!(msys(&other), None);
        if cfg!(windows) {
            assert_eq!(msys(&sh).as_deref(), Some("winsymlinks noglob"));
        } else {
            assert_eq!(msys(&sh).as_deref(), Some("winsymlinks"));
        }
    }

    #[test]
    fn a_script_path_reaches_sh_with_slashes_and_no_verbatim_prefix() {
        let path = std::path::Path::new;
        assert_eq!(sh_word(path(r"\\?\C:\a\b\script")), "C:/a/b/script");
        assert_eq!(sh_word(path(r"C:\a\script")), "C:/a/script");
        assert_eq!(sh_word(path(r"\\?\UNC\host\share\s")), "//host/share/s");
        assert_eq!(sh_word(path("/usr/bin/script")), "/usr/bin/script");
    }

    #[test]
    fn a_bare_name_is_found_through_pathext_in_path_order() {
        let (first, second) = dirs();
        std::fs::write(second.path().join("claude.cmd"), "").unwrap();
        let path = joined(&[first.path(), second.path()]);
        assert_eq!(
            resolve(
                "claude",
                Some(&path),
                Some(OsStr::new(".COM;.EXE;.BAT;.CMD"))
            ),
            Some(second.path().join("claude.cmd"))
        );
        // An earlier directory wins, and within it the earlier extension.
        std::fs::write(first.path().join("claude.bat"), "").unwrap();
        std::fs::write(first.path().join("claude.exe"), "").unwrap();
        assert_eq!(
            resolve(
                "claude",
                Some(&path),
                Some(OsStr::new(".COM;.EXE;.BAT;.CMD"))
            ),
            Some(first.path().join("claude.exe"))
        );
    }

    #[test]
    fn an_unset_pathext_takes_the_windows_default() {
        let (dir, _) = dirs();
        std::fs::write(dir.path().join("claude.cmd"), "").unwrap();
        let path = joined(&[dir.path()]);
        assert_eq!(
            resolve("claude", Some(&path), None),
            Some(dir.path().join("claude.cmd"))
        );
    }

    #[test]
    fn a_name_with_a_known_extension_is_looked_up_as_given() {
        let (dir, _) = dirs();
        std::fs::write(dir.path().join("git.exe"), "").unwrap();
        let path = joined(&[dir.path()]);
        let pathext = Some(OsStr::new(".EXE;.CMD"));
        assert_eq!(
            resolve("git.exe", Some(&path), pathext),
            Some(dir.path().join("git.exe"))
        );
        assert_eq!(resolve("git.cmd", Some(&path), pathext), None);
    }

    #[test]
    fn nothing_found_is_none_never_a_guess() {
        let (dir, _) = dirs();
        // A bare file with no extension is not what Windows would run.
        std::fs::write(dir.path().join("claude"), "").unwrap();
        // A directory is not a program.
        std::fs::create_dir(dir.path().join("claude.cmd")).unwrap();
        let path = joined(&[dir.path()]);
        assert_eq!(resolve("claude", Some(&path), None), None);
        assert_eq!(resolve("claude", None, None), None);
        assert_eq!(resolve("", Some(&path), None), None);
        assert_eq!(resolve("bin/claude", Some(&path), None), None);
    }

    #[test]
    fn a_shebang_names_how_the_adapter_starts() {
        assert_eq!(launch_of(b"\x7fELF\x02"), Launch::Direct);
        assert_eq!(launch_of(b"MZ\x90\x00"), Launch::Direct);
        assert_eq!(launch_of(b""), Launch::Direct);
        assert_eq!(launch_of(b"#!/bin/sh\nexit 0\n"), Launch::Sh(None));
        assert_eq!(launch_of(b"#!/bin/bash\r\n"), Launch::Sh(None));
        assert_eq!(
            launch_of(b"#! /bin/sh -eu\nset -x\n"),
            Launch::Sh(Some("-eu".to_owned()))
        );
        assert_eq!(launch_of(b"#!/usr/bin/env sh\n"), Launch::Sh(None));
        assert_eq!(launch_of(b"#!/usr/bin/env bash\n"), Launch::Sh(None));
        assert_eq!(
            launch_of(b"#!/usr/bin/env python3\n"),
            Launch::Other("/usr/bin/env python3".to_owned())
        );
        assert_eq!(
            launch_of(b"#!/usr/bin/zsh\n"),
            Launch::Other("/usr/bin/zsh".to_owned())
        );
        // A first line cut short by the read still names its interpreter.
        assert_eq!(launch_of(b"#!/bin/sh"), Launch::Sh(None));
    }

    #[test]
    fn sh_exe_is_looked_for_above_git_s_exec_path() {
        let at = |parts: &[&str]| parts.iter().collect::<std::path::PathBuf>();
        // Git for Windows: the exec path is `<root>/mingw64/libexec/git-core`, and its shell
        // is `<root>/bin/sh.exe`, with `<root>/usr/bin/sh.exe` beside it.
        let found = sh_candidates(&at(&["Git", "mingw64", "libexec", "git-core"]));
        assert_eq!(
            found,
            [
                at(&["Git", "mingw64", "libexec", "usr", "bin", "sh.exe"]),
                at(&["Git", "mingw64", "libexec", "bin", "sh.exe"]),
                at(&["Git", "mingw64", "usr", "bin", "sh.exe"]),
                at(&["Git", "mingw64", "bin", "sh.exe"]),
                at(&["Git", "usr", "bin", "sh.exe"]),
                at(&["Git", "bin", "sh.exe"]),
            ]
        );
    }

    #[test]
    fn a_search_path_takes_this_platform_s_separator() {
        let dirs = ["/usr/bin".to_owned(), "/bin".to_owned()];
        let joined = super::search_path(&dirs);
        let split: Vec<_> = std::env::split_paths(&joined).collect();
        let want: &[&str] = if cfg!(windows) {
            &["/mingw64/bin", "/usr/bin", "/bin"]
        } else {
            &["/usr/bin", "/bin"]
        };
        let want: Vec<_> = want.iter().map(std::path::Path::new).collect();
        assert_eq!(split, want);
        // A policy that names Git's folder itself keeps it where it put it, once.
        let named = ["/usr/bin".to_owned(), "/mingw64/bin".to_owned()];
        assert_eq!(
            super::search_path(&named),
            if cfg!(windows) {
                "/usr/bin;/mingw64/bin"
            } else {
                "/usr/bin:/mingw64/bin"
            }
        );
    }

    /// On Windows a `#!/bin/sh` adapter starts through `sh.exe` from Git for Windows.
    #[cfg(windows)]
    #[test]
    fn a_sh_script_starts_through_git_s_sh_exe_on_windows() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("adapter");
        std::fs::write(&script, "#!/bin/sh\necho ok\n").unwrap();
        let mut cmd = super::script(&script).unwrap();
        assert!(
            std::path::Path::new(cmd.get_program())
                .file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("sh.exe"))
        );
        let output = cmd.output().unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "ok");
        std::fs::write(&script, "#!/usr/bin/env python3\n").unwrap();
        assert!(super::script(&script).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_reaped_child_leaves_no_zombie() {
        // G575: an exited child bay never waits for stays in the process table.
        let child = std::process::Command::new("true").spawn().unwrap();
        let entry = std::path::PathBuf::from(format!("/proc/{}", child.id()));
        super::reap(child);
        let started = std::time::Instant::now();
        while entry.exists() && started.elapsed() < std::time::Duration::from_secs(10) {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            !entry.exists(),
            "{} is still in the process table",
            entry.display()
        );
    }
}
