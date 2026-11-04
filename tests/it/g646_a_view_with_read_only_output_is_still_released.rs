//! G646: a view holding read-only build output is still released.
//!
//! Some build tools leave read-only directories. `git clean` cannot empty them, and `git worktree
//! remove --force` fails on them yet unregisters the view anyway, leaving a folder with a dangling
//! `.git`. bay then kept that folder at every later start, and reported nothing.
//!
//! Guards to mutate, in `src/view.rs`: in `remove_worktree`, the `remove_tree` call (the released
//! view's folder stays); in `make_writable`, the `set_permissions` call (both cases fail); in
//! `make_writable`, the `is_dir` test, so it follows a link (the outside directory turns writable).

// Each case runs a shell script (the check's adapter), so these run on Unix only.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use crate::support;

use support::{TestBox, check_files, passing_result, result_json};

fn seeded(test_box: &TestBox) -> PathBuf {
    let repo = test_box.repo_any("repo");
    let files = check_files("true", &passing_result(1), 1, 60);
    test_box.seed_with_check(
        &repo,
        &[("src/a.rs", "fn a() {}\n"), (".gitignore", "cache/\n")],
        &files,
    );
    repo
}

fn read_only(directory: &Path) {
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o555)).unwrap();
}

/// A merged change's view holding `cache/pkg/file`, its two directories read-only.
fn landed_with_output(test_box: &TestBox, repo: &Path) -> PathBuf {
    let started = test_box.bay_ok(repo, &["start", "CARD-1", "--paths", "src/", ".gitignore"]);
    let view = PathBuf::from(started["detail"]["view"].as_str().unwrap());
    std::fs::write(view.join("src/b.rs"), "fn b() {}\n").unwrap();
    let (code, landed) = result_json(test_box.bay(&view).args(["merge", "--wait"]));
    assert_eq!(code, 0, "{landed}");
    if view.exists() {
        add_output(&view);
    }
    view
}

fn add_output(view: &Path) {
    std::fs::create_dir_all(view.join("cache/pkg")).unwrap();
    std::fs::write(view.join("cache/pkg/file"), "built\n").unwrap();
    read_only(&view.join("cache/pkg"));
    read_only(&view.join("cache"));
}

#[test]
fn a_released_view_with_read_only_output_goes_whole() {
    let test_box = TestBox::new();
    let repo = seeded(&test_box);
    let view = landed_with_output(&test_box, &repo);
    assert!(view.join("cache/pkg/file").exists());
    // Outside the view, and read-only: removing the view must not change it through the link.
    let outside = test_box.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    read_only(&outside);
    std::fs::set_permissions(view.join("cache"), std::fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::symlink(&outside, view.join("cache/link")).unwrap();
    read_only(&view.join("cache"));

    let started = test_box.bay_ok(&repo, &["start", "CARD-2", "--paths", "src/"]);

    assert!(!view.exists(), "the view stayed: {started}");
    let mode = std::fs::metadata(&outside).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o555, "a link was followed out of the view");
    let listed = test_box.git_ok(&repo, &["worktree", "list", "--porcelain"]);
    assert!(
        !listed.contains(view.to_str().unwrap()),
        "it is still registered: {listed}"
    );
}

#[test]
fn a_kept_view_gives_up_read_only_output_too() {
    let test_box = TestBox::new();
    let repo = seeded(&test_box);
    let view = landed_with_output(&test_box, &repo);
    std::fs::write(view.join("src/a.rs"), "fn a() { later() }\n").unwrap();

    test_box.bay_ok(&repo, &["start", "CARD-2", "--paths", "src/"]);

    assert!(!view.join("cache").exists(), "the output stayed");
    assert_eq!(
        std::fs::read_to_string(view.join("src/a.rs")).unwrap(),
        "fn a() { later() }\n",
        "the edit is work"
    );
}
