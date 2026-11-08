//! G603 (a fleet finding, 2026-10-31: `bay status` of a change whose adapter edit kept the
//! policy's revision read "Done; nothing else is needed." while its operation was blocked). A
//! fault stops an operation with no reason code, so no outcome says why: the status row of that
//! operation carries its receipt's words (`fault`), and its sentence says it stopped, why, and
//! what to do.
//!
//! Guard to mutate: `src/result.rs` `change_say`: the `no_outcome_say` arm.

use crate::support;

use support::{TestBox, check_files, passing_result, result_json};

#[test]
fn a_status_of_a_change_a_fault_stopped_says_why_and_what_to_do() {
    let test_box = TestBox::new();
    let repo = test_box.repo_any("repo");
    let check = check_files("# adapter v1", &passing_result(1), 1, 60);
    test_box.seed_with_check(&repo, &[("src/a.rs", "fn a() {}\n")], &check);
    let started = test_box.bay_ok(&repo, &["start", "CARD-1", "--paths", "src/", "scripts/"]);
    let id = started["change"].as_str().unwrap().to_owned();
    let view = std::path::PathBuf::from(started["detail"]["view"].as_str().unwrap());
    let adapter = check_files("# adapter v2", &passing_result(1), 1, 60);
    std::fs::write(view.join("scripts/bay-check"), &adapter[0].1).unwrap();
    let (code, refused) = result_json(test_box.bay(&view).args(["merge", "--wait"]));
    assert_eq!(code, 20, "{refused}");

    let (code, status) = result_json(test_box.bay(&repo).args(["status", &id]));
    assert_eq!(code, 0, "{status}");
    let operation = &status["detail"]["changes"][0]["operation"];
    assert_eq!(operation["state"], "blocked", "{status}");
    let why = operation["fault"]["why"].as_str().unwrap_or_default();
    assert!(why.contains("must raise revision above 1"), "{status}");
    let say = status["say"].as_str().unwrap();
    assert!(say.contains("stopped and is not in line: "), "{say}");
    assert!(say.contains(why), "{say}");
    assert!(!say.contains("nothing else is needed"), "{say}");
}
