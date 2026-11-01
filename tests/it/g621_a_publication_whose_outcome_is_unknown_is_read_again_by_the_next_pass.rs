//! G621 (a fleet finding, 2026-10-31: an operation blocked `MERGE_OUTCOME_UNKNOWN` said bay would
//! look again by itself, and no runner pass ever did; it waited for its author's `bay merge`). A
//! publication whose outcome is unknown is read again by the next executor pass: the pass reads
//! trunk first, so a candidate that landed is recorded landed and published once; one that did
//! not land is re-armed and published, once.
//!
//! Guard to mutate: in `src/publish.rs`, ask a blocked operation again only when it waits for an
//! approval or a check's answer, not when its publication's outcome is unknown, and the next pass
//! leaves the operation blocked in both cases.

// Each case runs shell scripts (adapters, a remote hook), so these run on Unix only.
#![cfg(unix)]

use crate::support;

use std::path::{Path, PathBuf};

use serde_json::Value;
use support::{TestBox, check_files, passing_result, result_json};

/// A change submitted to `repo`, whose remote runs `hook` once on a push to main: it drops the
/// connection before its report and leaves a marker, and the executor's clone reads no trunk
/// while the marker is there. The first runner pass ends `MERGE_OUTCOME_UNKNOWN`, and its
/// blocked record reaches the remote: that push, and no other (the runner's presence beats push
/// meanwhile), clears the marker (`post-receive`). The box, the remote, the executor's clone
/// and the change's id.
fn blocked_unknown(hook: &str, body: &str) -> (TestBox, PathBuf, PathBuf, String) {
    let test_box = TestBox::new();
    let repo = test_box.repo_any("repo");
    let remote = test_box.path().join("repo-remote.git");
    let marker = test_box.path().join("trunk-unreadable");
    test_box.seed_with_check(
        &repo,
        &[("src/a.rs", "fn a() {}\n")],
        &check_files("true", &passing_result(1), 1, 60),
    );
    let (id, view) = test_box.start_and_edit(&repo);
    let (code, submitted) = result_json(test_box.bay(&view).arg("merge"));
    assert_eq!(code, 0, "{submitted}");

    let hooks = remote.join("hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    let marker = marker.display();
    script(
        &hooks.join(hook),
        &format!(
            "#!/bin/sh\n{body}grep -q ' refs/heads/main$' || exit 0\nrm -f \"$0\"\n: > '{marker}'\nkill -9 $PPID\n"
        ),
    );
    script(
        &hooks.join("post-receive"),
        &format!(
            "#!/bin/sh\nwhile read old new ref; do\n  git log -1 --format=%B \"$new\" 2>/dev/null | grep -q 'publish outcome unknown' && rm -f '{marker}'\ndone\nexit 0\n"
        ),
    );

    let executor = test_box.clone_of(&remote, "executor");
    let wrapper = test_box.path().join("upload-pack");
    script(
        &wrapper,
        &format!("#!/bin/sh\n[ -e '{marker}' ] && exit 1\nexec git upload-pack \"$@\"\n"),
    );
    test_box.git_ok(
        &executor,
        &[
            "config",
            "remote.origin.uploadpack",
            wrapper.to_str().unwrap(),
        ],
    );
    let (_, ran) = result_json(test_box.bay(&executor).args(["runner", "run", "--once"]));
    let result = &ran["detail"]["ran"][0];
    assert_eq!(result["reason"], "MERGE_OUTCOME_UNKNOWN", "{ran}");
    assert_eq!(result["decision"], "blocked", "{ran}");
    assert!(
        !test_box.path().join("trunk-unreadable").exists(),
        "the blocked record never reached the remote: {ran}"
    );
    (test_box, remote, executor, id)
}

/// Writes the shell script `path` holds, executable.
fn script(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
}

/// The next runner pass in `executor`: the row of the operation it ran, and the change ids on
/// `remote`'s trunk after it.
fn next_pass(test_box: &TestBox, executor: &Path, remote: &Path) -> (Value, Vec<String>) {
    let (code, ran) = result_json(test_box.bay(executor).args(["runner", "run", "--once"]));
    assert_eq!(code, 0, "{ran}");
    (
        ran["detail"]["ran"][0].clone(),
        test_box.trunk_change_ids(remote),
    )
}

#[test]
fn a_candidate_that_landed_is_recorded_landed_by_the_next_pass() {
    // The remote commits trunk's update, then drops the connection.
    let (test_box, remote, executor, id) = blocked_unknown(
        "reference-transaction",
        "[ \"$1\" = committed ] || exit 0\n",
    );
    let landed = test_box.git_bare(&remote, &["rev-parse", "main"]);

    let (row, on_trunk) = next_pass(&test_box, &executor, &remote);
    assert_eq!(row["decision"], "merged", "the next pass left it: {row}");
    assert_eq!(row["detail"]["commit"], landed.as_str(), "{row}");
    assert_eq!(on_trunk, vec![id], "published again: {row}");
    assert_eq!(test_box.git_bare(&remote, &["rev-parse", "main"]), landed);
}

#[test]
fn a_candidate_that_did_not_land_is_published_once_by_the_next_pass() {
    // The remote drops the connection before it takes any update.
    let (test_box, remote, executor, id) = blocked_unknown("pre-receive", "");
    assert!(
        test_box.trunk_change_ids(&remote).is_empty(),
        "the refused push landed"
    );

    let (row, on_trunk) = next_pass(&test_box, &executor, &remote);
    assert_eq!(row["decision"], "merged", "the next pass left it: {row}");
    assert_eq!(on_trunk, vec![id], "{row}");
}
