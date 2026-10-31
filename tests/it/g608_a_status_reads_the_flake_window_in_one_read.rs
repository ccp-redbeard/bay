//! G608 (a fleet finding, 2026-10-31: a status counted each required check's flaky landings
//! with one records read a landing, about 60 reads of the evidence refs on a window of 60). A
//! status reads the records of the whole window at once: it lists the evidence refs as often
//! with six landings as with two, and counts every landing.
//!
//! Guard to mutate, in `src/status.rs` `flakes`: read each landing's records alone, and a
//! status lists the evidence refs once more for each landing.

// The landing runs a shell check adapter, and the case puts a shell wrapper for `git` on PATH,
// so it runs on Unix only.
#![cfg(unix)]

use crate::g606_a_status_judges_red_from_the_last_status::land;
use crate::support;

use std::path::Path;

use bay::evidence::EVIDENCE_PREFIX;
use support::{TestBox, check_files, passing_result};

/// One `bay status`: how many times it listed the evidence refs, and the landings its flake
/// rows counted.
fn status(test_box: &TestBox, repo: &Path) -> (usize, u64) {
    let listed = test_box.path().join("listed");
    let _ = std::fs::remove_file(&listed);
    let status = test_box.bay_ok(repo, &["status"]);
    let lists = std::fs::read_to_string(&listed)
        .unwrap_or_default()
        .lines()
        .count();
    let landings = status["detail"]["flakes"][0]["merges"]
        .as_u64()
        .unwrap_or_else(|| panic!("no flake row: {status}"));
    (lists, landings)
}

#[test]
fn a_status_lists_the_evidence_refs_as_often_with_six_landings_as_with_two() {
    let test_box = TestBox::new();
    let repo = test_box.repo_any("repo");
    let files = check_files("true", &passing_result(1), 1, 60);
    test_box.seed_with_check(&repo, &[("src/a.rs", "fn a() {}\n")], &files);
    for n in 0..2 {
        land(&test_box, &repo, &format!("src/first{n}.rs"));
    }

    // Each `git for-each-ref` of the evidence refs leaves one line.
    test_box.wrap_git(&format!(
        "case \"$*\" in\n  *for-each-ref*{EVIDENCE_PREFIX}*) echo listed >> '{listed}' ;;\nesac",
        listed = test_box.path().join("listed").display(),
    ));
    let (two, read) = status(&test_box, &repo);
    assert!(read >= 2, "the flake rows counted {read} landings");

    for n in 0..4 {
        land(&test_box, &repo, &format!("src/more{n}.rs"));
    }
    let (six, more) = status(&test_box, &repo);
    assert_eq!(more, read + 4, "the flake rows lost a landing");
    assert_eq!(
        six, two,
        "a status listed the evidence refs {six} times with {more} landings, {two} with {read}"
    );
}
