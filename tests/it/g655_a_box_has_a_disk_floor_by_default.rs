//! G655 (owner ruling, 2026-11-05; a machine at 96% with no floor set was never told). A box with
//! no `bay.minFreeMiB` has a floor of 5 GiB, because the safe behaviour should need no step nobody
//! is prompted to take. A start below it frees what it can first (G640) and refuses `DISK_LOW` only
//! if still low. `0` turns the floor off.
//!
//! Guard to mutate, in `src/disk.rs` `floor`: unset reading `DEFAULT_FLOOR_MIB` (make it 0: the
//! first case starts); the first case's `floor_mib` holds the number.

// The floor is not judged on Windows.
#![cfg(unix)]

use crate::support;

use std::path::PathBuf;

use support::{TestBox, result_json, unchecked_files};

fn seeded(test_box: &TestBox) -> PathBuf {
    let repo = test_box.repo_any("repo");
    test_box.seed_with_check(&repo, &[("src/a.rs", "fn a() {}\n")], &unchecked_files());
    // Every other test runs with the floor off, whatever disk the machine has.
    test_box.git_ok(&repo, &["config", "--global", "--unset", "bay.minFreeMiB"]);
    repo
}

fn start(test_box: &TestBox, repo: &std::path::Path, free_mib: &str) -> (i32, serde_json::Value) {
    result_json(
        test_box
            .bay(repo)
            .env("BAY_TEST_FREE_MIB", free_mib)
            .args(["start", "CARD-1", "--paths", "src/"]),
    )
}

#[test]
fn a_box_that_set_nothing_refuses_below_five_gibibytes() {
    let test_box = TestBox::new();
    let repo = seeded(&test_box);

    let (code, refused) = start(&test_box, &repo, "5119");

    assert_eq!(code, 10, "{refused}");
    assert_eq!(refused["reason"], "DISK_LOW", "{refused}");
    assert_eq!(refused["detail"]["floor_mib"], 5120, "{refused}");
}

#[test]
fn a_box_that_set_nothing_starts_at_the_floor() {
    let test_box = TestBox::new();
    let repo = seeded(&test_box);

    let (code, started) = start(&test_box, &repo, "5120");

    assert_eq!(code, 0, "{started}");
}

#[test]
fn zero_turns_the_floor_off() {
    let test_box = TestBox::new();
    let repo = seeded(&test_box);
    test_box.git_ok(&repo, &["config", "bay.minFreeMiB", "0"]);

    let (code, started) = start(&test_box, &repo, "1");

    assert_eq!(code, 0, "{started}");
}
