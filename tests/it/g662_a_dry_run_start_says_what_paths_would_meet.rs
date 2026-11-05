//! G662 (owner, 2026-11-05: which lane should a piece of work go in, and how do we know
//! beforehand?). To learn whether a check would hold some paths, an agent had to open a change or
//! read the policy by hand. `bay start <intent> --paths <paths> --dry-run` says, from the policy on
//! main and the live changes: `detail.pre_merge` and `detail.overlaps`, and it writes nothing.
//!
//! Guards to mutate, in `src/main.rs`: the `dry_run` test (a change opens); in `src/start.rs`
//! `preview`: the overlaps it lists.

// Each case runs a shell script (the check's adapter), so these run on Unix only.
#![cfg(unix)]

use crate::support;

use std::path::PathBuf;

use support::{TestBox, check_files, passing_result};

fn seeded() -> (TestBox, PathBuf) {
    let test_box = TestBox::new();
    let repo = test_box.repo_any("repo");
    let mut files = check_files("true", &passing_result(1), 1, 60);
    for (path, text) in &mut files {
        if path == ".bay/policy.toml" {
            *text = text.replacen("timeout_s", "when_paths = [\"src/**\"]\ntimeout_s", 1);
        }
    }
    test_box.seed_with_check(
        &repo,
        &[("src/a.rs", "fn a() {}\n"), ("docs/a.md", "a\n")],
        &files,
    );
    (test_box, repo)
}

fn changes(test_box: &TestBox, repo: &std::path::Path) -> String {
    test_box.git_ok(repo, &["for-each-ref", "refs/bay/changes"])
}

#[test]
fn a_path_no_check_reads_says_it_waits_for_none_and_writes_nothing() {
    let (test_box, repo) = seeded();

    let said = test_box.bay_ok(&repo, &["start", "NOTE", "--paths", "docs/", "--dry-run"]);

    assert_eq!(said["detail"]["pre_merge"]["at_once"], true, "{said}");
    assert!(said["change"].is_null(), "{said}");
    assert!(said["detail"].get("view").is_none(), "{said}");
    assert_eq!(changes(&test_box, &repo), "", "a dry run opened a change");
    assert!(
        said["say"]
            .as_str()
            .unwrap()
            .starts_with("Nothing was started"),
        "{said}"
    );
}

#[test]
fn a_path_a_check_reads_names_the_check() {
    let (test_box, repo) = seeded();

    let said = test_box.bay_ok(&repo, &["start", "CODE", "--paths", "src/", "--dry-run"]);

    assert_eq!(said["detail"]["pre_merge"]["at_once"], false, "{said}");
    assert_eq!(
        said["detail"]["pre_merge"]["checks"][0]["check"], "tests",
        "{said}"
    );
    assert!(said["say"].as_str().unwrap().contains("tests"), "{said}");
}

#[test]
fn a_live_change_on_the_same_paths_is_named() {
    let (test_box, repo) = seeded();
    let live = test_box.bay_ok(&repo, &["start", "LIVE", "--paths", "docs/"]);

    let said = test_box.bay_ok(&repo, &["start", "NOTE", "--paths", "docs/", "--dry-run"]);

    assert_eq!(
        said["detail"]["overlaps"][0]["change"], live["change"],
        "{said}"
    );
}
