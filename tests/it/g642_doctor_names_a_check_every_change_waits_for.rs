//! G642: `bay doctor` names a pre-merge check every change waits for.
//!
//! A change no pre-merge check covers merges at once (G403). One check with no `when_paths`
//! covers every path, so a repository loses that for good, and nothing said so. A repository may
//! mean it, so the row is advice and never red.
//!
//! Guards to mutate, in `src/doctor.rs` `pre_merge_row`: the `stage` test (the check after the
//! merge is named); the `wakes` test (the scoped check is named).

// Each case runs a shell script (the check's adapter), so these run on Unix only.
#![cfg(unix)]

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::support;

use support::{TestBox, check_files, passing_result};

fn seeded(test_box: &TestBox, keys: &str) -> PathBuf {
    let repo = test_box.repo_any("repo");
    let mut files = check_files("true", &passing_result(1), 1, 60);
    let policy = files
        .iter_mut()
        .find(|(path, _)| path == ".bay/policy.toml")
        .unwrap();
    policy.1 = policy
        .1
        .replacen("timeout_s", &format!("{keys}timeout_s"), 1);
    test_box.seed_with_check(&repo, &[("src/a.rs", "fn a() {}\n")], &files);
    repo
}

/// A check on `src/` alone, and a check after the merge on every path.
fn seeded_with_post(test_box: &TestBox) -> PathBuf {
    let repo = test_box.repo_any("repo");
    let mut files = check_files("true", &passing_result(1), 1, 60);
    let policy = files
        .iter_mut()
        .find(|(path, _)| path == ".bay/policy.toml")
        .unwrap();
    policy.1 = policy
        .1
        .replacen("timeout_s", "when_paths = [\"src/**\"]\ntimeout_s", 1);
    policy.1.push_str(
        "\n[[check]]\nid = \"everything\"\nenvironment = \"local\"\nstage = \"post\"\ncommand = \"scripts/bay-check\"\nadapter = [\"scripts/bay-check\"]\nexpected_minimum = 1\ntimeout_s = 60\n",
    );
    test_box.seed_with_check(&repo, &[("src/a.rs", "fn a() {}\n")], &files);
    repo
}

fn row(test_box: &TestBox, repo: &Path) -> Option<Value> {
    let (_, doctor) = support::result_json(test_box.bay(repo).arg("doctor"));
    doctor["detail"]["rows"]
        .as_array()
        .unwrap_or_else(|| panic!("no rows: {doctor}"))
        .iter()
        .find(|row| row["name"] == "pre_merge")
        .cloned()
}

#[test]
fn a_check_on_every_path_is_named_and_is_never_red() {
    for keys in ["", "when_paths = [\"**\"]\n"] {
        let test_box = TestBox::new();
        let repo = seeded(&test_box, keys);
        let row = row(&test_box, &repo).unwrap_or_else(|| panic!("no row for {keys:?}"));
        assert_eq!(row["every_change_waits_for"][0], "tests", "{row}");
        assert_eq!(row["state"], "informational", "{row}");
        assert_eq!(row["required"], false, "{row}");
    }
}

#[test]
fn a_scoped_check_and_a_check_after_the_merge_are_not_named() {
    let test_box = TestBox::new();
    let repo = seeded_with_post(&test_box);
    assert_eq!(row(&test_box, &repo), None);
}
