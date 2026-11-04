//! G640: `bay start` frees what bay may free, and does so first when the disk is low.
//!
//! G595 removes a day-old half-written pack in a runner's pass only, so a clone no runner works in
//! kept them: one held 14 GB. And G265 refused a start below the disk floor before G97 released a
//! single ended view, so the moment space mattered most was the one moment nothing was freed.
//! Now every start removes this clone's day-old half-written packs, and a start below the floor
//! frees first, measures again, and refuses only if it is still below.
//!
//! Guards to mutate, in `src/start.rs`: in `reclaim`, the `sweep_tmp_packs` call (the first test
//! finds the pack); in `start`, the `reclaim` before the floor is measured again (the second test
//! finds the pack and the view).

// Each case runs a shell script (the check's adapter), so these run on Unix only.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::support;

use support::{TestBox, check_files, passing_result, result_json};

const DAY: Duration = Duration::from_hours(24);

/// A half-written pack `name` in the clone's pack directory, read-only as Git leaves it, last
/// touched `ago`.
fn left(repo: &Path, name: &str, ago: Duration) -> PathBuf {
    let file = repo.join(".git/objects/pack").join(name);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "PACK").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_modified(SystemTime::now() - ago)
        .unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o444)).unwrap();
    file
}

fn seeded(test_box: &TestBox) -> PathBuf {
    let repo = test_box.repo_any("repo");
    let files = check_files("true", &passing_result(1), 1, 60);
    test_box.seed_with_check(&repo, &[("src/a.rs", "fn a() {}\n")], &files);
    repo
}

#[test]
fn a_start_removes_this_clones_day_old_half_written_packs() {
    let test_box = TestBox::new();
    let repo = seeded(&test_box);
    let stale = left(&repo, "tmp_pack_aB3dE9", DAY * 2);
    let index = left(&repo, "tmp_idx_aB3dE9", DAY * 2);
    let recent = left(&repo, "tmp_pack_fetching", Duration::from_hours(1));

    test_box.bay_ok(&repo, &["start", "CARD-1", "--paths", "src/"]);

    assert!(!stale.exists(), "a day-old half-written pack stayed");
    assert!(!index.exists(), "a day-old half-written index stayed");
    assert!(recent.exists(), "a pack a fetch may be writing was removed");
    test_box.git_ok(&repo, &["fsck", "--no-progress"]);
}

#[test]
fn a_start_below_the_floor_frees_first_and_names_what_it_freed() {
    let test_box = TestBox::new();
    let repo = seeded(&test_box);
    let started = test_box.bay_ok(&repo, &["start", "CARD-1", "--paths", "src/"]);
    let id = started["change"].as_str().unwrap().to_owned();
    let view = PathBuf::from(started["detail"]["view"].as_str().unwrap());
    std::fs::write(view.join("src/b.rs"), "fn b() {}\n").unwrap();
    let (code, landed) = result_json(test_box.bay(&view).args(["merge", "--wait"]));
    assert_eq!(code, 0, "{landed}");
    let stale = left(&repo, "tmp_pack_aB3dE9", DAY * 2);
    test_box.git_ok(&repo, &["config", "bay.minFreeMiB", "999999999999"]);

    let (code, refused) = result_json(
        test_box
            .bay(&repo)
            .args(["start", "CARD-2", "--paths", "src/"]),
    );

    assert_eq!(code, 10, "{refused}");
    assert_eq!(refused["reason"], "DISK_LOW", "{refused}");
    assert!(!stale.exists(), "the refusal freed no pack: {refused}");
    assert!(!view.exists(), "the refusal freed no view: {refused}");
    assert_eq!(
        refused["detail"]["released_views"],
        serde_json::json!([id]),
        "{refused}"
    );
}
