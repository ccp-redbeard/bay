//! G620 (a fleet finding, 2026-11-01: at load 100 to 155 a sweep of a hosted remote took about
//! 34 s, so its window, 15 s from its start, had closed before it ended, and every read swept
//! again). A sweep's window counts from its end, and it is twice as long as the sweep took, from
//! 15 s to 60 s. Read with the window a release build has (no `BAY_TEST_SWEPT_S`);
//! `BAY_TEST_SWEEP_HOLD_MS` makes a test build's sweep take that long after it asks the remote.
//!
//! Guards to mutate: in `src/sweep.rs`, count the window from the sweep's start, and the read
//! after a slow sweep's end asks the remote again; make it as long as the sweep took, not twice,
//! and the same read asks again.

use crate::support;

use std::path::Path;
use std::time::Duration;

use serde_json::Value;
use support::{TestBox, result_json};

use crate::g617_a_listings_sweep_serves_the_reads_in_its_window::{move_trunk, store};

/// `bay status` in `dir` as the reader with the release build's window and `env`: how many of
/// its git spawns (its sweep's too) asked the remote, and the result.
fn status(test_box: &TestBox, dir: &Path, env: &[(&str, &str)], name: &str) -> (usize, Value) {
    let trace = test_box.path().join(format!("{name}-trace.jsonl"));
    let mut cmd = test_box.bay(dir);
    cmd.arg("status")
        .env("BAY_ACTOR", "reader")
        .env_remove("BAY_TEST_SWEPT_S")
        .env("BAY_TEST_GIT_TRACE", &trace);
    for (key, value) in env {
        cmd.env(key, value);
    }
    let (code, result) = result_json(&mut cmd);
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
    (asked, result)
}

/// Waits until the Unix time `at`.
fn until(at: u64) {
    let at = std::time::UNIX_EPOCH + Duration::from_secs(at);
    if let Ok(left) = at.duration_since(std::time::SystemTime::now()) {
        std::thread::sleep(left);
    }
}

#[test]
fn a_slow_sweep_serves_the_reads_after_its_end_twice_as_long_as_it_took() {
    let (test_box, _, remote, reader, _) = store();
    // The sweep takes 12 s from its start to its end: its window is 24 s after its end. Counted
    // from its start, or as long as the sweep took, it closes 12 s after the end, before the read
    // 18 s after it.
    let (asked, first) = status(
        &test_box,
        &reader,
        &[("BAY_TEST_SWEEP_HOLD_MS", "12000")],
        "first",
    );
    assert_eq!(asked, 1, "the first read did not sweep: {first}");
    let record: Value = serde_json::from_str(
        &std::fs::read_to_string(reader.join(".git").join("bay").join("swept.json")).unwrap(),
    )
    .unwrap();
    let (start, end) = (
        record["unix"].as_u64().unwrap(),
        record["ended"].as_u64().unwrap(),
    );
    let took = end - start;
    assert!(
        (12..20).contains(&took),
        "the sweep took {took} s: {record}"
    );
    let main = first["detail"]["main"].clone();
    let next = move_trunk(&test_box, &remote);

    until(end + 18);
    let (asked, served) = status(&test_box, &reader, &[], "served");
    assert_eq!(
        asked, 0,
        "18 s after a {took} s sweep the window was closed: {served}"
    );
    assert_eq!(served["detail"]["main"], main, "{served}");
    assert_eq!(
        served["detail"]["fetched_at"],
        first["freshness"]["fetched_at"]
    );

    until(end + 2 * took + 1);
    let (asked, after) = status(&test_box, &reader, &[], "after");
    assert_eq!(asked, 1, "the window outlived twice the sweep: {after}");
    assert_eq!(after["detail"]["main"], next.as_str(), "{after}");
}
