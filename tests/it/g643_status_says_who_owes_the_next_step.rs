//! G643: `bay status` says who owes the next step (a fleet finding, 2026-11-04).
//!
//! A person read "Done; nothing else is needed." with three of their changes refused and a fourth
//! out of tries. A listing fell through to the last sentence an `ok` has. Now its sentence counts
//! the reader's own open changes by who owes the next step, and names those that need someone:
//! "Nothing needs you" only when that is so, and the old sentence only when nothing is open.
//!
//! Guards to mutate, in `src/result.rs` `BayResult::say`: `status_say` back to `main_say` (every
//! case here reads the old sentence); in `owes`, the `op.is_null()` arm (the unsubmitted changes
//! count as on the way); in `listing_say`, `no_live_runner` returning `false` (the no-runner case
//! says nothing needs you; the unit test in `src/result.rs` holds the live-runner sentence).

// Each case runs a shell script (the check's adapter), so these run on Unix only.
#![cfg(unix)]

use std::path::{Path, PathBuf};

use crate::support;

use support::{TestBox, check_files, passing_result, result_json};

/// A trunk whose one check fails a candidate holding `src/refused.rs`.
fn seeded(test_box: &TestBox) -> PathBuf {
    let repo = test_box.repo_any("repo");
    let pass = passing_result(1);
    let fail = pass.replace("\\\"pass\\\"", "\\\"product_failure\\\"");
    let result = format!("if [ -e src/refused.rs ]; then {fail}; else {pass}; fi");
    let files = check_files("true", &result, 1, 60);
    test_box.seed_with_check(&repo, &[("src/a.rs", "fn a() {}\n")], &files);
    repo
}

/// Starts change `name` with `src/<name>.rs` written; its id and view.
fn started(test_box: &TestBox, repo: &Path, name: &str) -> (String, PathBuf) {
    let intent = format!("CARD-{}", name.to_uppercase());
    let started = test_box.bay_ok(repo, &["start", &intent, "--paths", "src/"]);
    let view = PathBuf::from(started["detail"]["view"].as_str().unwrap());
    std::fs::write(view.join(format!("src/{name}.rs")), "fn x() {}\n").unwrap();
    (started["change"].as_str().unwrap().to_owned(), view)
}

fn said(test_box: &TestBox, repo: &Path) -> String {
    let read = test_box.bay_ok(repo, &["status"]);
    read["say"].as_str().unwrap().to_owned()
}

#[test]
fn a_refused_change_is_named_and_the_sentence_says_someone_is_needed() {
    let test_box = TestBox::new();
    let repo = seeded(&test_box);
    let (refused, view) = started(&test_box, &repo, "refused");
    let (code, blocked) = result_json(test_box.bay(&view).args(["merge", "--wait"]));
    assert_eq!(code, 10, "{blocked}");
    started(&test_box, &repo, "draft");

    let said = said(&test_box, &repo);

    assert!(!said.contains("nothing else is needed"), "{said}");
    assert!(!said.contains("Nothing needs you"), "{said}");
    assert!(
        said.contains(&format!("1 needs a fix ({refused})")),
        "{said}"
    );
    assert!(said.contains("1 is not submitted"), "{said}");
}

#[test]
fn drafts_alone_say_nothing_needs_you() {
    let test_box = TestBox::new();
    let repo = seeded(&test_box);
    started(&test_box, &repo, "one");
    started(&test_box, &repo, "two");

    let said = said(&test_box, &repo);

    assert!(said.starts_with("Nothing needs you"), "{said}");
    assert!(said.contains("2 are not submitted"), "{said}");
}

#[test]
fn changes_in_line_with_no_runner_never_say_nothing_needs_you() {
    let test_box = TestBox::new();
    let repo = seeded(&test_box);
    for name in ["one", "two"] {
        let (_, view) = started(&test_box, &repo, name);
        let (code, queued) = result_json(test_box.bay(&view).arg("merge"));
        assert_eq!(code, 0, "{queued}");
    }

    let said = said(&test_box, &repo);

    assert!(!said.contains("Nothing needs you"), "{said}");
    assert!(said.contains("2 wait for a runner"), "{said}");
}

#[test]
fn nothing_open_keeps_the_plain_sentence() {
    let test_box = TestBox::new();
    let repo = seeded(&test_box);

    assert_eq!(said(&test_box, &repo), "Done; nothing else is needed.");
}
