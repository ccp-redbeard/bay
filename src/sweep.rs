//! G615 (a fleet finding, 2026-10-31: on a hosted remote a `bay status` spent about 7 s of 12 to
//! 19 s on the network: the trunk fetch alone first, then eight fetches and listings beside
//! each other, three of its sets two round trips one after another). A status read now asks the
//! remote once: one `git fetch --dry-run --porcelain` of every namespace the read judges. Git
//! lists each namespace by its prefix, brings the objects of every ref that moved, writes no
//! ref, and names each update it would make. bay then makes only the updates that each
//! namespace's own rule allows, in one ref transaction. When nothing moved, no object comes and
//! no ref is written.
//!
//! G617 (a fleet finding, 2026-10-31: the sweep alone took 3.2 to 4.0 s of each status on a
//! hosted remote, however few refs had moved): a listing's sweep serves every status read of this
//! store for [`BAY_SWEPT_S`] after it. Inside that window a read asks the remote nothing, reads
//! the store as that sweep left it, and names that sweep's time as its `fetched_at`. A push from
//! this store ends the window of every sweep that began before the push ended, so the read after
//! a write asks the remote again.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::git::{Git, GitError};

/// How long, in seconds, a listing's sweep serves the status reads of its store after it.
pub const BAY_SWEPT_S: u64 = 15;

const TEST_HOOKS: bool = option_env!("BAY_RELEASE_VERSION").is_none();

/// One update the fetch would make to the local ref it names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Move {
    /// Git's flag: ' ' a fast-forward, '+' a forced update, '*' a new ref, '-' a prune (the
    /// remote lacks the ref).
    pub flag: char,
    /// The local ref before; `None` when it does not exist.
    pub old: Option<String>,
    /// The remote's; `None` for a prune.
    pub new: Option<String>,
}

/// One ref update [`apply`] makes: the ref, its value now, and its new value (`None` deletes).
pub type Update = (String, Option<String>, Option<String>);

/// What one sweep found.
#[derive(Debug, Default)]
pub struct Swept {
    /// Each local ref that would move, by name.
    pub moves: BTreeMap<String, Move>,
    /// When the remote was asked: taken before the sweep, so it never overstates (G80).
    pub at: String,
    /// The same time, in Unix seconds.
    pub unix: u64,
}

impl Swept {
    /// The remote's value of local ref `name` in a swept namespace, where `local` is this
    /// store's: the value it would move to, else `local`.
    #[must_use]
    pub fn remote_of<'a>(&'a self, name: &str, local: Option<&'a str>) -> Option<&'a str> {
        match self.moves.get(name) {
            Some(found) => found.new.as_deref(),
            None => local,
        }
    }

    /// The moves of the local refs under `prefix`, by name.
    pub fn under<'a>(&'a self, prefix: &'a str) -> impl Iterator<Item = (&'a str, &'a Move)> {
        self.moves
            .range(prefix.to_owned()..)
            .take_while(move |(name, _)| name.starts_with(prefix))
            .map(|(name, found)| (name.as_str(), found))
    }

    /// The updates of the moves under `prefix` that `take` allows, as [`apply`] makes them.
    #[must_use]
    pub fn taken(&self, prefix: &str, take: impl Fn(&str, &Move) -> bool) -> Vec<Update> {
        self.under(prefix)
            .filter(|(name, found)| take(name, found))
            .map(|(name, found)| (name.to_owned(), found.old.clone(), found.new.clone()))
            .collect()
    }
}

/// Asks `remote` once for every ref `refspecs` name, pruned to what it holds: the objects of
/// each ref that moved come, and no ref is written.
///
/// # Errors
///
/// The remote cannot be read, or lacks a ref a refspec names exactly.
pub fn sweep(git: &Git, remote: &str, refspecs: &[String]) -> Result<Swept, GitError> {
    let unix = crate::live::unix_now();
    let at = crate::clock::rfc3339_of(unix);
    let mut args = vec![
        "fetch",
        "--dry-run",
        "--porcelain",
        "--prune",
        "--no-tags",
        remote,
    ];
    args.extend(refspecs.iter().map(String::as_str));
    let listed = git.run(&args)?;
    Ok(Swept {
        moves: parse(&listed),
        at,
        unix,
    })
}

/// The window, in seconds: [`BAY_SWEPT_S`], or in a test build `BAY_TEST_SWEPT_S`.
fn window_s() -> u64 {
    std::env::var("BAY_TEST_SWEPT_S")
        .ok()
        .filter(|_| TEST_HOOKS)
        .and_then(|text| text.parse().ok())
        .unwrap_or(BAY_SWEPT_S)
}

/// The record of the last listing's sweep of the store whose common directory is `common`.
fn record(common: &Path) -> PathBuf {
    common.join("bay").join("swept.json")
}

/// When the store last pushed, in Unix seconds ([`pushed`]).
fn push_mark(common: &Path) -> PathBuf {
    common.join("bay").join("swept-pushed")
}

/// The time of the last listing's sweep of `remote` by `refspecs` in this store, while its
/// window is open at `now`; `None` when no such sweep is that recent, or the store pushed after
/// it began.
#[must_use]
pub fn window(common: &Path, remote: &str, refspecs: &[String], now: u64) -> Option<String> {
    let text = std::fs::read_to_string(record(common)).ok()?;
    let held: Value = serde_json::from_str(&text).ok()?;
    let unix = held["unix"].as_u64()?;
    let pushed = std::fs::read_to_string(push_mark(common))
        .ok()
        .and_then(|text| text.trim().parse::<u64>().ok());
    let open = held["remote"] == remote
        && held["refspecs"] == json!(refspecs)
        && pushed.is_none_or(|pushed| pushed < unix)
        && unix <= now
        && now < unix.saturating_add(window_s());
    open.then(|| held["at"].as_str().map(str::to_owned))?
}

/// Records that a listing's sweep of `remote` by `refspecs` settled this store, so the reads in
/// its window are served by it. Best effort: a record that cannot be written opens no window.
pub fn remember(common: &Path, remote: &str, refspecs: &[String], swept: &Swept) {
    let path = record(common);
    let held = json!({"remote": remote, "refspecs": refspecs, "at": swept.at, "unix": swept.unix});
    replace(&path, &held.to_string());
}

/// Records that the store whose common directory is `common` pushed at `now`: a push may have
/// changed what the remote holds, so no sweep that began at or before it serves a read. A push
/// that ends inside the second a sweep began closes that sweep's window too.
pub fn pushed(common: &Path, now: u64) {
    replace(&push_mark(common), &now.to_string());
}

/// Writes `text` at `path` whole, through a file of this process beside it. Best effort.
fn replace(path: &Path, text: &str) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    if std::fs::write(&tmp, text).is_ok() {
        let _ = crate::durable::rename(&tmp, path);
    }
}

/// The moves of `git fetch --porcelain`: lines of a flag, the old and new object ids, and the
/// local ref.
fn parse(listed: &str) -> BTreeMap<String, Move> {
    let id = |oid: &str| (!oid.bytes().all(|byte| byte == b'0')).then(|| oid.to_owned());
    listed
        .lines()
        .filter_map(|line| {
            let flag = line.chars().next()?;
            let mut words = line.get(1..)?.split_whitespace();
            let (old, new, name) = (words.next()?, words.next()?, words.next()?);
            Some((
                name.to_owned(),
                Move {
                    flag,
                    old: id(old),
                    new: id(new),
                },
            ))
        })
        .collect()
}

/// Makes each move of `moves` (a ref, its value now, its new value; `None` deletes) in one
/// transaction, each compared with its value now. When the transaction fails (a ref another
/// process moved meanwhile), each is made alone, and one that moved meanwhile is left.
pub fn apply(git: &Git, moves: &[Update]) {
    let line = |text: &mut String, (name, old, new): &Update| {
        let _ = match (old, new) {
            (Some(old), Some(new)) => writeln!(text, "update {name} {new} {old}"),
            (None, Some(new)) => writeln!(text, "create {name} {new}"),
            (Some(old), None) => writeln!(text, "delete {name} {old}"),
            (None, None) => Ok(()),
        };
    };
    let mut all = String::new();
    for found in moves {
        line(&mut all, found);
    }
    if all.is_empty()
        || git
            .run_with(&["update-ref", "--stdin"], &[], Some(all.as_bytes()))
            .is_ok()
    {
        return;
    }
    for found in moves {
        let mut one = String::new();
        line(&mut one, found);
        let _ = git.run_with(&["update-ref", "--stdin"], &[], Some(one.as_bytes()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_porcelain_listing_names_each_move_with_its_flag() {
        let zero = "0".repeat(40);
        let (a, b) = ("a".repeat(40), "b".repeat(40));
        let listed = format!(
            "  {a} {b} refs/bay/changes/c1\n+ {a} {b} refs/bay/log/own\n* {zero} {b} refs/remotes/origin/dev\n- {a} {zero} refs/bay/fleet/gone"
        );
        let moves = parse(&listed);
        assert_eq!(moves["refs/bay/changes/c1"].flag, ' ');
        assert_eq!(moves["refs/bay/log/own"].flag, '+');
        assert_eq!(moves["refs/remotes/origin/dev"].old, None);
        assert_eq!(moves["refs/bay/fleet/gone"].new, None);
        let swept = Swept {
            moves,
            at: String::new(),
            unix: 0,
        };
        assert_eq!(swept.remote_of("refs/bay/fleet/gone", Some(&a)), None);
        assert_eq!(
            swept.remote_of("refs/bay/changes/c2", Some(&a)),
            Some(a.as_str())
        );
        assert_eq!(swept.under("refs/bay/fleet/").count(), 1);
        assert_eq!(
            swept.taken("refs/bay/", |_, found| found.flag != '-'),
            vec![
                (
                    "refs/bay/changes/c1".to_owned(),
                    Some(a.clone()),
                    Some(b.clone())
                ),
                (
                    "refs/bay/log/own".to_owned(),
                    Some(a.clone()),
                    Some(b.clone())
                )
            ]
        );
    }
}
