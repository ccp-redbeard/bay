//! G647: releasing a view leaves every other worktree's registration alone.
//!
//! bay ran a plain `git worktree prune` after each release, which drops the registration of any
//! worktree whose folder is absent at that moment, such as one on an unmounted disk. When the
//! folder came back it was no longer a worktree, and its index and any operation in progress
//! were gone.
//!
//! The guard to mutate, in `src/view.rs`: `unregister`, a plain `git worktree prune` (the sibling
//! is gone from the list).

use std::path::PathBuf;

use crate::support;

use support::{TestBox, check_files, passing_result, result_json};

#[test]
fn a_siblings_folder_absent_at_a_release_still_works_when_it_returns() {
    let test_box = TestBox::new();
    let repo = test_box.repo_any("repo");
    let files = check_files("true", &passing_result(1), 1, 60);
    test_box.seed_with_check(&repo, &[("src/a.rs", "fn a() {}\n")], &files);
    let sibling = test_box.path().join("sibling");
    test_box.git_ok(
        &repo,
        &["worktree", "add", "--detach", sibling.to_str().unwrap()],
    );
    let started = test_box.bay_ok(&repo, &["start", "CARD-1", "--paths", "src/"]);
    let view = PathBuf::from(started["detail"]["view"].as_str().unwrap());
    std::fs::write(view.join("src/b.rs"), "fn b() {}\n").unwrap();
    let (code, landed) = result_json(test_box.bay(&view).args(["merge", "--wait"]));
    assert_eq!(code, 0, "{landed}");
    let away = test_box.path().join("away");
    std::fs::rename(&sibling, &away).unwrap();

    let next = test_box.bay_ok(&repo, &["start", "CARD-2", "--paths", "src/"]);
    assert!(!view.exists(), "the merged view stayed: {next}");
    assert!(
        !test_box
            .git_ok(&repo, &["worktree", "list", "--porcelain"])
            .contains(view.to_str().unwrap()),
        "the released view is still registered"
    );

    std::fs::rename(&away, &sibling).unwrap();
    test_box.git_ok(&sibling, &["status", "--short"]);
}
