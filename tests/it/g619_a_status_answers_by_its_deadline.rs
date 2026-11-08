//! G619 (a fleet finding, 2026-11-01: at load 100 to 155 one sweep of a hosted remote took about
//! 34 s, and every `bay status` waited for it). A status read waits for its sweep only until its
//! deadline (`--deadline`). Past it, the read answers from the store as the last sweep left it:
//! `detail.fetched_at` names that sweep's time, `detail.stale_s` its age, and a `stale_read`
//! warning the sweep that runs on. The sweep is a process of its own, which holds no stdio of the
//! caller, so a caller that reads the output returns at the deadline; it ends by itself, settles
//! the store and records itself, so the read after it shows what it brought. A store runs one sweep
//! at a time: a read that finds one running starts none, and waits for it until its deadline.
//! `BAY_TEST_SWEEP_DELAY_MS` holds a test build's sweep before it asks the remote.
//!
//! Guards to mutate: in `src/sweeper.rs`, wait for the sweep's answer whatever the deadline, and
//! the read past it does not answer in time; give the sweep the caller's stderr, and a caller that
//! reads the output waits for the sweep's end; in `src/sweep.rs`, take the lock whoever holds it,
//! and a second read sweeps beside the first; in `src/sweeper.rs`, never see that the running
//! sweep ended, and a read that waits for it answers stale.

use crate::support;

use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::Value;
use support::{TestBox, result_json};

use crate::g617_a_listings_sweep_serves_the_reads_in_its_window::{OPEN, move_trunk, shows, store};

/// `bay status` in `dir` as the reader, with `env`: the result, how long it took, and how many of
/// its git spawns (its sweep's too) asked the remote.
fn status(
    test_box: &TestBox,
    dir: &Path,
    args: &[&str],
    env: &[(&str, &str)],
    name: &str,
) -> (Value, Duration, usize) {
    let trace = test_box.path().join(format!("{name}-trace.jsonl"));
    let mut cmd = test_box.bay(dir);
    cmd.arg("status")
        .args(args)
        .env("BAY_ACTOR", "reader")
        .env("BAY_TEST_GIT_TRACE", &trace);
    for (key, value) in env {
        cmd.env(key, value);
    }
    let started = Instant::now();
    let (code, result) = result_json(&mut cmd);
    let took = started.elapsed();
    assert_eq!(code, 0, "{result}");
    let text = std::fs::read_to_string(&trace).unwrap_or_default();
    let lines: Vec<Value> = text
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    let pid = lines.first().map(|line| line["pid"].clone());
    let asked = lines
        .iter()
        .filter(|line| Some(&line["pid"]) == pid.as_ref())
        .filter(|line| matches!(line["sub"].as_str(), Some("fetch" | "ls-remote" | "push")))
        .count();
    (result, took, asked)
}

/// The `stale_read` warning of `result`, if it names one.
fn stale_read(result: &Value) -> Option<&Value> {
    result["detail"]["warnings"]
        .as_array()?
        .iter()
        .find(|warning| warning["code"] == "stale_read")
}

/// What the sweep lock of the store at `dir` says, once `done` holds of it, waiting up to 60 s.
fn lock_once(dir: &Path, done: impl Fn(&Value) -> bool) -> Value {
    let lock = dir.join(".git").join("bay").join("sweep.lock");
    let until = Instant::now() + Duration::from_secs(60);
    loop {
        let held: Value = std::fs::read_to_string(&lock)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(Value::Null);
        if done(&held) {
            return held;
        }
        assert!(
            Instant::now() < until,
            "the sweep lock never said so: {held}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
#[cfg_attr(
    windows,
    ignore = "Windows: a timing bound this machine's process starts miss; under investigation"
)]
fn a_read_past_its_deadline_answers_from_the_store_and_names_its_age() {
    let (test_box, _, remote, reader, id) = store();
    let (first, _, asked) = status(&test_box, &reader, &[], &[], "first");
    assert_eq!(asked, 1, "the first read did not sweep: {first}");
    assert!(stale_read(&first).is_none(), "{first}");
    assert!(
        first["detail"]["stale_s"].is_u64(),
        "a read must name its age: {first}"
    );
    let main = first["detail"]["main"].clone();
    let next = move_trunk(&test_box, &remote);

    // The sweep waits 5 s before it asks the remote; the read waits 1 s, and a caller that reads
    // its output (as `$(bay status)` does) has it back then.
    let (late, took, _) = status(
        &test_box,
        &reader,
        &["--deadline", "1"],
        &[("BAY_TEST_SWEEP_DELAY_MS", "5000")],
        "late",
    );
    assert!(
        took < Duration::from_secs(4),
        "the read took {took:?}: {late}"
    );
    let warning = stale_read(&late).unwrap_or_else(|| panic!("no stale_read warning: {late}"));
    assert_eq!(warning["sweep"]["state"], "started", "{warning}");
    assert!(warning["sweep"]["by"]["pid"].is_u64(), "{warning}");
    assert_eq!(
        late["detail"]["main"], main,
        "the stale read is not the store: {late}"
    );
    assert_eq!(
        late["detail"]["fetched_at"], first["freshness"]["fetched_at"],
        "the stale read did not name the last sweep's time"
    );
    assert_eq!(
        late["freshness"]["fetched_at"],
        late["detail"]["fetched_at"]
    );
    assert!(late["detail"]["stale_s"].is_u64(), "{late}");
    assert!(shows(&late, &id), "{late}");

    // The sweep runs on to its end and settles the store: the read in its window shows the trunk
    // it brought and asks the remote nothing.
    lock_once(&reader, |held| held["ended"].is_u64());
    let (after, _, asked) = status(
        &test_box,
        &reader,
        &[],
        &[("BAY_TEST_SWEPT_S", OPEN)],
        "after",
    );
    assert_eq!(
        asked, 0,
        "the sweep the late read started did not serve the next: {after}"
    );
    assert_eq!(after["detail"]["main"], next.as_str(), "{after}");
    assert!(stale_read(&after).is_none(), "{after}");
}

#[test]
#[cfg_attr(
    windows,
    ignore = "Windows: a timing bound this machine's process starts miss; under investigation"
)]
fn a_read_while_a_sweep_runs_starts_none_and_names_it() {
    let (test_box, _, remote, reader, _) = store();
    let next = move_trunk(&test_box, &remote);
    let mut first = test_box
        .bay_process(&reader)
        .args(["--json", "status"])
        .env("BAY_ACTOR", "reader")
        .env("BAY_TEST_SWEEP_DELAY_MS", "6000")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let running = lock_once(&reader, |held| {
        held["since"].is_u64() && held["ended"].is_null()
    });

    let (stale, took, asked) = status(&test_box, &reader, &["--deadline", "1"], &[], "stale");
    assert_eq!(
        asked, 0,
        "a read beside a running sweep asked the remote: {stale}"
    );
    assert!(
        took < Duration::from_secs(4),
        "the read took {took:?}: {stale}"
    );
    let warning = stale_read(&stale).unwrap_or_else(|| panic!("no stale_read warning: {stale}"));
    assert_eq!(warning["sweep"]["state"], "running", "{warning}");
    assert_eq!(warning["sweep"]["by"]["pid"], running["pid"], "{warning}");

    // A read that waits long enough is served by the running sweep, and starts none either.
    let (waited, _, asked) = status(&test_box, &reader, &[], &[], "waited");
    assert_eq!(
        asked, 0,
        "a read that waited for the running sweep asked the remote"
    );
    assert!(stale_read(&waited).is_none(), "{waited}");
    assert_eq!(waited["detail"]["main"], next.as_str(), "{waited}");
    assert!(first.wait().unwrap().success());
}
