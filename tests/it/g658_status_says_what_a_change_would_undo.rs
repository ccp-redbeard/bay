//! G658 (a fleet finding, 2026-11-05: a seat read `UNDOES_MERGED` in `bay status` with no paths,
//! and could not tell what to keep). The refusal named the file and the change, but only to the
//! command that ran the merge; a session reading it later saw the code alone. Now the operation
//! keeps them, and `bay status <change>` shows `operation.undoes`.
//!
//! Guard to mutate, in `src/publish.rs` `undoes_merged`: the `op.undoes` it records.

use crate::support;

use support::{TestBox, UNCHECKED_POLICY, result_json};

const A: &str = "one\ntwo\n";

#[test]
fn status_names_the_file_and_the_change_it_would_undo() {
    let test_box = TestBox::new();
    let repo = test_box.repo_any("repo");
    test_box.seed_with_check(
        &repo,
        &[("src/a.rs", A)],
        &[(".bay/policy.toml".to_owned(), UNCHECKED_POLICY.to_owned())],
    );
    let remote = test_box.path().join("repo-remote.git");
    let other = test_box.clone_of(&remote, "other");
    let started = test_box.bay_ok(&repo, &["start", "ADD", "--paths", "src/"]);
    let id = started["change"].as_str().unwrap().to_owned();
    let view = std::path::PathBuf::from(started["detail"]["view"].as_str().unwrap());
    std::fs::write(other.join("src/a.rs"), "ONE\ntwo\n").unwrap();
    let (code, saved) = result_json(test_box.bay(&other).args(["save", "SHOUT"]));
    assert_eq!(code, 0, "{saved}");
    let theirs = saved["change"].as_str().unwrap().to_owned();
    test_box.bay_ok(&repo, &["start", &id]);
    std::fs::write(view.join("src/a.rs"), "one\ntwo\nthree\n").unwrap();
    let (code, refused) = result_json(test_box.bay(&view).args(["merge", "--wait"]));
    assert_eq!(code, 10, "{refused}");
    assert_eq!(refused["reason"], "UNDOES_MERGED", "{refused}");

    let status = test_box.bay_ok(&repo, &["status", &id]);

    let undoes = &status["detail"]["changes"][0]["operation"]["undoes"];
    assert_eq!(undoes["paths"][0], "src/a.rs", "{status}");
    assert_eq!(undoes["change"], theirs.as_str(), "{status}");
    assert!(undoes["commit"].is_string(), "{status}");
}
