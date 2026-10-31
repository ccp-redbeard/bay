//! G611 (a fleet finding, 2026-10-31: a status that refreshed 48 live changes ran three git
//! processes a change to settle them, about 2 s). A status reads the chains it fetched and this
//! store's own in one read each, deletes the fetched refs in one transaction, and settles only
//! a chain the remote moved. Six changes cost no more of those processes than one, read from
//! the trace a test build writes when `BAY_TEST_GIT_TRACE` names a file (G171).
//!
//! Guards to mutate, in `src/repo.rs` `settle_fetched`: settle every change, and the processes
//! grow with the changes; settle none, and a chain the remote moved stays behind.

use crate::support;

use std::path::{Path, PathBuf};

use serde_json::Value;
use support::{TestBox, result_json, unchecked_files};

/// A repository with `changes` live changes, and a second clone that holds their chains, as a
/// store shared by many agents does: the box, the first clone, the reader and the change ids.
fn store(changes: usize) -> (TestBox, PathBuf, PathBuf, Vec<String>) {
    let test_box = TestBox::new();
    let repo = test_box.repo_any("repo");
    let remote = test_box.path().join("repo-remote.git");
    test_box.seed_with_check(&repo, &[("src/a.rs", "fn a() {}\n")], &unchecked_files());
    let ids = (0..changes)
        .map(|_| test_box.start_and_edit(&repo).0)
        .collect();
    let reader = test_box.clone_of(&remote, "reader");
    test_box.git_ok(
        &reader,
        &[
            "fetch",
            "-q",
            "origin",
            "+refs/bay/changes/*:refs/bay/changes/*",
        ],
    );
    (test_box, repo, reader, ids)
}

/// The `rev-parse` and `update-ref` spawns of one `bay status` in `dir` read as actor
/// `reader`, from bay's own process only.
fn settles(test_box: &TestBox, dir: &Path, name: &str) -> usize {
    let trace = test_box.path().join(format!("{name}-trace.jsonl"));
    let (code, result) = result_json(
        test_box
            .bay(dir)
            .arg("status")
            .env("BAY_ACTOR", "reader")
            .env("BAY_TEST_GIT_TRACE", &trace),
    );
    assert_eq!(code, 0, "{result}");
    let text = std::fs::read_to_string(&trace).expect("the trace was written");
    let lines: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|error| panic!("{error}: {line}")))
        .collect();
    let pid = lines.first().map(|line| line["pid"].clone());
    lines
        .iter()
        .filter(|line| Some(&line["pid"]) == pid.as_ref())
        .filter(|line| line["sub"] == "rev-parse" || line["sub"] == "update-ref")
        .count()
}

#[test]
fn six_fetched_chains_cost_no_more_settling_than_one() {
    let (one_box, _, one, _) = store(1);
    let (six_box, _, six, _) = store(6);
    let with_one = settles(&one_box, &one, "one");
    let with_six = settles(&six_box, &six, "six");
    assert!(
        with_six <= with_one,
        "settling six chains cost {with_six} processes, one cost {with_one}"
    );
}

#[test]
fn a_status_settles_a_chain_the_remote_moved_and_leaves_no_fetched_ref() {
    let (test_box, repo, reader, ids) = store(2);
    let moved = &ids[0];
    let name = format!("refs/bay/changes/{moved}");
    let before = test_box.git_ok(&reader, &["rev-parse", &name]);

    // The change moves on the remote: its own clone abandons it.
    let (code, result) = result_json(test_box.bay(&repo).args(["abandon", moved]));
    assert_eq!(code, 0, "{result}");
    let remote = test_box.git_ok(&repo, &["ls-remote", "origin", &name]);
    let remote = remote.split_whitespace().next().unwrap().to_owned();
    assert_ne!(remote, before.trim(), "the abandon did not move the chain");

    let _ = settles(&test_box, &reader, "moved");
    let after = test_box.git_ok(&reader, &["rev-parse", &name]);
    assert_eq!(after.trim(), remote, "the status left the chain behind");
    let other = format!("refs/bay/changes/{}", ids[1]);
    assert_eq!(
        test_box.git_ok(&reader, &["rev-parse", &other]),
        test_box.git_ok(&repo, &["rev-parse", &other]),
        "the status moved a chain the remote did not"
    );
    let left = test_box.git_ok(
        &reader,
        &["for-each-ref", "--format=%(refname)", "refs/bay/fetch/"],
    );
    assert!(left.trim().is_empty(), "fetched refs were left: {left}");
}
