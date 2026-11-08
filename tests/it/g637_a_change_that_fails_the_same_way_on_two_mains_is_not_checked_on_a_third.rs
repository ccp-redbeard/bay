//! G637 (a fleet finding, 2026-11-02: two changes whose own task cards failed a records check
//! were checked again on each new main, and failed the same way each time, two lost candidates
//! and a runner's time on each): a change whose last two candidates, each on a different main,
//! failed the same units for the same reasons, each unit naming a file the change changes, is
//! refused as before when main moves again, with no third run. A new revision re-arms it. A
//! failure that names no file of the change, or whose reason changed between the two mains, may
//! be one a newer main fixes, so it is checked again.
//!
//! Guards to mutate (each makes a case here fail):
//! - `src/publish.rs` `retry_blocked`: drop the G637 arm (a third run: the first case).
//! - `src/publish.rs` `fails_as_before`: `own` as `true` (the failure that names no file of the
//!   change is refused unrun: the second case).
//! - `src/publish.rs` `fails_as_before`: drop the compare of the two candidates' failing units
//!   (the failure whose reason changed is refused unrun: the third case).

use crate::support;

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use support::{TestBox, check_files, passing_result, result_json};

/// The file the change adds and the trunk's `tests` fails a candidate for.
const OWN: &str = "src/fail-own";

/// A result with one passing unit and one failing unit `t2` whose reason is `reason`.
fn failing_result(reason: &str) -> String {
    format!(
        "printf '%s' \"{{\\\"schema\\\":\\\"bay.check.result/1\\\",\\\"exercised\\\":{{\\\"commit\\\":\\\"$commit\\\",\\\"tree\\\":\\\"$tree\\\"}},\\\"plan\\\":{{\\\"discovered\\\":2,\\\"selected\\\":2,\\\"completed\\\":2}},\\\"checks\\\":[{{\\\"id\\\":\\\"t1\\\",\\\"verdict\\\":\\\"pass\\\"}},{{\\\"id\\\":\\\"t2\\\",\\\"verdict\\\":\\\"product_failure\\\",\\\"reason\\\":\\\"{reason}\\\"}}]}}\""
    )
}

/// A trunk whose `tests` fails a candidate that holds `src/fail-own` with `reason` (`$n` is the
/// count of files under `src/`), and the change that adds it, refused once. Its repository, id
/// and view.
fn failed_once(test_box: &TestBox, reason: &str) -> (PathBuf, String, PathBuf) {
    let repo = test_box.repo_any("repo");
    let before = format!(
        "n=$(ls src | wc -l | tr -d ' ')\n[ -e {OWN} ] && {{ {}; exit 0; }}",
        failing_result(reason)
    );
    test_box.seed_with_check(
        &repo,
        &[("src/a.rs", "fn a() {}\n")],
        &check_files(&before, &passing_result(2), 2, 60),
    );
    let keys = test_box.keys();
    test_box.enroll(&repo, &keys);
    test_box.use_keys(&repo, &keys);
    let started = test_box.bay_ok(&repo, &["start", "CARD-own", "--paths", "src/"]);
    let id = started["change"].as_str().unwrap().to_owned();
    let view = PathBuf::from(started["detail"]["view"].as_str().unwrap());
    std::fs::write(view.join(OWN), "1\n").unwrap();
    let refused = merge(test_box, &view);
    assert!(refused["detail"]["fails_as_before"].is_null(), "{refused}");
    (repo, id, view)
}

/// `bay merge --wait` in `view`, refused `CHECK_FAILED`.
fn merge(test_box: &TestBox, view: &Path) -> Value {
    let (code, refused) = result_json(test_box.bay(view).args(["merge", "--wait"]));
    assert_eq!(code, 10, "{refused}");
    assert_eq!(refused["reason"], "CHECK_FAILED", "{refused}");
    refused
}

/// Another change, on a file of its own under `src/`, lands: main moves.
fn main_moves(test_box: &TestBox, repo: &Path, n: u32) {
    let intent = format!("CARD-other-{n}");
    let started = test_box.bay_ok(repo, &["start", &intent, "--paths", "src/"]);
    let view = PathBuf::from(started["detail"]["view"].as_str().unwrap());
    std::fs::write(view.join(format!("src/other{n}.rs")), "fn other() {}\n").unwrap();
    let landed = test_box.bay_ok(&view, &["merge", "--wait"]);
    assert_eq!(landed["decision"], "merged", "{landed}");
}

/// The candidates change `id` has had.
fn attempts(test_box: &TestBox, repo: &Path, id: &str) -> u64 {
    let status = test_box.bay_ok(repo, &["status", id]);
    status["detail"]["changes"][0]["operation"]["attempts"]
        .as_u64()
        .unwrap_or_else(|| panic!("no attempts: {status}"))
}

/// The change of [`failed_once`], refused again on a second main and then merged again on a
/// third: the refusal of that third merge.
fn third_merge(test_box: &TestBox, reason: &str) -> (PathBuf, String, PathBuf, Value) {
    let (repo, id, view) = failed_once(test_box, reason);
    main_moves(test_box, &repo, 1);
    let again = merge(test_box, &view);
    assert!(again["detail"]["fails_as_before"].is_null(), "{again}");
    assert_eq!(attempts(test_box, &repo, &id), 2);
    main_moves(test_box, &repo, 2);
    let third = merge(test_box, &view);
    (repo, id, view, third)
}

#[test]
fn a_change_that_failed_the_same_way_on_two_mains_is_refused_unrun_on_a_third() {
    let test_box = TestBox::new();
    let (repo, id, view, third) = third_merge(&test_box, "src/fail-own: wrong");

    assert_eq!(
        third["detail"]["fails_as_before"],
        json!([{"check": "tests", "unit": "t2", "reason": "src/fail-own: wrong"}]),
        "{third}"
    );
    assert_eq!(third["detail"]["unchanged"], true, "{third}");
    assert_eq!(attempts(&test_box, &repo, &id), 2, "no third candidate");

    // WHEN the author fixes it in place, THEN the new revision is checked, and lands.
    std::fs::remove_file(view.join(OWN)).unwrap();
    std::fs::write(view.join("src/fixed.rs"), "fn fixed() {}\n").unwrap();
    let landed = test_box.bay_ok(&view, &["merge", "--wait"]);
    assert_eq!(landed["decision"], "merged", "{landed}");
}

#[test]
fn a_failure_that_names_no_file_of_the_change_is_checked_again_on_a_third_main() {
    let test_box = TestBox::new();
    let (repo, id, _, third) = third_merge(&test_box, "the build box is full");

    assert!(third["detail"]["fails_as_before"].is_null(), "{third}");
    assert_eq!(attempts(&test_box, &repo, &id), 3, "a third candidate ran");
}

#[test]
fn a_failure_whose_reason_changed_between_two_mains_is_checked_again_on_a_third() {
    let test_box = TestBox::new();
    let (repo, id, _, third) = third_merge(&test_box, "src/fail-own: wrong with $n files");

    assert!(third["detail"]["fails_as_before"].is_null(), "{third}");
    assert_eq!(attempts(&test_box, &repo, &id), 3, "a third candidate ran");
}
