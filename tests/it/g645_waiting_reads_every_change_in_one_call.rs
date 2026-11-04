//! G645: `bay waiting` reads every change in one git call (a fleet finding, 2026-11-04).
//!
//! On a store with 3,701 changes the one command a person needs ran for minutes, one git call per
//! change for a record it mostly set aside. Its git calls now do not grow with the changes.
//!
//! The guard to mutate, in `src/approve.rs` `records`: one read per record in place of
//! `records_at` (the calls grow).

// Each case runs a shell script (the check's adapter), so these run on Unix only.
#![cfg(unix)]

use std::path::{Path, PathBuf};

use crate::support;

use support::{TestBox, result_json, unchecked_files};

fn git_calls(test_box: &TestBox, repo: &Path, name: &str) -> usize {
    let trace = test_box.path().join(name);
    let (code, read) = result_json(
        test_box
            .bay(repo)
            .arg("waiting")
            .env("BAY_TEST_GIT_TRACE", &trace),
    );
    assert_eq!(code, 0, "{read}");
    std::fs::read_to_string(&trace)
        .unwrap_or_default()
        .lines()
        .count()
}

#[test]
fn merged_changes_add_no_git_calls_to_waiting() {
    let test_box = TestBox::new();
    let repo = test_box.repo_any("repo");
    test_box.seed_with_check(&repo, &[("src/a.rs", "fn a() {}\n")], &unchecked_files());
    let merge = |n: usize| {
        let intent = format!("CARD-{n}");
        let started = test_box.bay_ok(&repo, &["start", &intent, "--paths", "src/"]);
        let view = PathBuf::from(started["detail"]["view"].as_str().unwrap());
        std::fs::write(view.join(format!("src/f{n}.rs")), "fn f() {}\n").unwrap();
        let (code, landed) = result_json(test_box.bay(&view).args(["merge", "--wait"]));
        assert_eq!(code, 0, "{landed}");
    };
    merge(0);
    let before = git_calls(&test_box, &repo, "before.trace");

    for n in 1..5 {
        merge(n);
    }

    let after = git_calls(&test_box, &repo, "after.trace");
    assert!(after <= before, "{before} git calls grew to {after}");
}
