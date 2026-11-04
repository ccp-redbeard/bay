//! G639: a view of an ended change that is kept for its work gives up its build output.
//!
//! G97 keeps a view with an edit or an untracked file, and the whole view stayed: one stray edit
//! after a merge kept gigabytes of installed dependencies on the disk for good, and nothing else
//! ever removed them. The files the view's ignore rules match are not work (G97 already removes
//! them with a released view), and the change that needed them has ended. So the next `bay start`
//! on the machine removes them from each kept view, and leaves the edit, the untracked file and
//! the view itself. A view whose change has not ended keeps its build output.
//!
//! Only directories go. A loose ignored file, such as an `.env`, stays: bay cannot tell it from
//! work, and this view was kept because someone may come back to it.
//!
//! Guards to mutate, in `src/view.rs`: in `release`, the `remove_ignored_directories` call (the
//! first test finds the build output still there); `remove_ignored_directories`, a plain `git
//! clean -fdXq` (the third test finds the `.env` gone).

// Each case runs a shell script (the check's adapter), so these run on Unix only.
#![cfg(unix)]

use std::path::{Path, PathBuf};

use crate::support;

use support::{TestBox, check_files, passing_result, result_json};

/// A trunk with one check that fails a candidate holding `src/refused.rs`, whose ignore rules
/// name `node_modules/`.
fn seeded(test_box: &TestBox) -> PathBuf {
    let repo = test_box.repo_any("repo");
    let pass = passing_result(1);
    let fail = pass.replace("\\\"pass\\\"", "\\\"product_failure\\\"");
    let result = format!("if [ -e src/refused.rs ]; then {fail}; else {pass}; fi");
    let files = check_files("true", &result, 1, 60);
    test_box.seed_with_check(
        &repo,
        &[
            ("src/a.rs", "fn a() {}\n"),
            (".gitignore", "node_modules/\n.env\n"),
        ],
        &files,
    );
    repo
}

/// Starts change `name` and writes `src/<name>.rs` and some build output in its view.
fn started(test_box: &TestBox, repo: &Path, name: &str) -> PathBuf {
    let intent = format!("CARD-{}", name.to_uppercase());
    let started = test_box.bay_ok(repo, &["start", &intent, "--paths", "src/", ".gitignore"]);
    let view = PathBuf::from(started["detail"]["view"].as_str().unwrap());
    std::fs::write(
        view.join(format!("src/{name}.rs")),
        format!("fn {name}() {{}}\n"),
    )
    .unwrap();
    std::fs::create_dir_all(view.join("node_modules/pkg")).unwrap();
    std::fs::write(view.join("node_modules/pkg/index.js"), "built\n").unwrap();
    view
}

#[test]
fn a_view_kept_for_its_edit_loses_its_build_output_and_keeps_the_edit() {
    let test_box = TestBox::new();
    let repo = seeded(&test_box);
    let edited = started(&test_box, &repo, "edited");
    let untracked = started(&test_box, &repo, "untracked");
    for view in [&edited, &untracked] {
        let (code, landed) = result_json(test_box.bay(view).args(["merge", "--wait"]));
        assert_eq!(code, 0, "{landed}");
    }
    std::fs::write(edited.join("src/a.rs"), "fn a() { later() }\n").unwrap();
    std::fs::write(untracked.join("src/more.rs"), "fn more() {}\n").unwrap();

    test_box.bay_ok(&repo, &["start", "CARD-NEXT", "--paths", "src/"]);

    assert!(!edited.join("node_modules").exists(), "the output stayed");
    assert!(
        !untracked.join("node_modules").exists(),
        "the output stayed"
    );
    assert_eq!(
        std::fs::read_to_string(edited.join("src/a.rs")).unwrap(),
        "fn a() { later() }\n",
        "the edit is work"
    );
    assert!(untracked.join("src/more.rs").exists(), "the file is work");
}

#[test]
fn a_view_whose_change_has_not_ended_keeps_its_build_output() {
    let test_box = TestBox::new();
    let repo = seeded(&test_box);
    let open = started(&test_box, &repo, "open");
    let refused = started(&test_box, &repo, "refused");
    let (code, blocked) = result_json(test_box.bay(&refused).args(["merge", "--wait"]));
    assert_eq!(code, 10, "{blocked}");

    test_box.bay_ok(&repo, &["start", "CARD-NEXT", "--paths", "src/"]);

    for view in [&open, &refused] {
        assert!(
            view.join("node_modules/pkg/index.js").exists(),
            "{} lost its output while its change is open",
            view.display()
        );
    }
}

#[test]
fn a_loose_ignored_file_in_a_kept_view_stays() {
    let test_box = TestBox::new();
    let repo = seeded(&test_box);
    let view = started(&test_box, &repo, "edited");
    std::fs::write(view.join(".env"), "TOKEN=kept\n").unwrap();
    let (code, landed) = result_json(test_box.bay(&view).args(["merge", "--wait"]));
    assert_eq!(code, 0, "{landed}");
    std::fs::write(view.join("src/a.rs"), "fn a() { later() }\n").unwrap();

    test_box.bay_ok(&repo, &["start", "CARD-NEXT", "--paths", "src/"]);

    assert!(!view.join("node_modules").exists(), "the output stayed");
    assert_eq!(
        std::fs::read_to_string(view.join(".env")).unwrap(),
        "TOKEN=kept\n",
        "bay cannot tell a loose ignored file from work"
    );
}
