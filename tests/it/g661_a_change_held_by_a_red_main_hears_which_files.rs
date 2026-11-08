//! G661 (a fleet finding, 2026-11-05: a change mixing documents with code waited on a red main,
//! and its author had to work out which files held it). `bay status <change>` now names them in
//! `operation.red_holds` and the rest in `operation.red_frees`, so a mixed change knows what to
//! split out.
//!
//! Guard to mutate, in `src/publish.rs` `red_gate`: the `op.red_holds` it records.

use crate::support;

use std::path::PathBuf;

use support::{TestBox, check_files, passing_result, result_json};

const POST: &str = "\n[[check]]\nid = \"suite\"\nenvironment = \"local\"\nstage = \"post\"\ncommand = \"scripts/bay-check suite\"\nadapter = [\"scripts/bay-check\"]\nexpected_minimum = 1\ntimeout_s = 60\nwhen_paths = [\"src/**\"]\n";

#[test]
fn a_mixed_change_hears_which_of_its_files_the_red_holds() {
    let test_box = TestBox::new();
    let repo = test_box.repo_any("repo");
    let failing = passing_result(1).replace("\\\"pass\\\"", "\\\"product_failure\\\"");
    let mut files = check_files(
        &format!("[ \"$1\" = suite ] && [ -e src/bad.rs ] && {{ {failing}; exit 0; }}"),
        &passing_result(1),
        1,
        60,
    );
    for (path, text) in &mut files {
        if path == ".bay/policy.toml" {
            *text = text.replacen("timeout_s", "when_paths = [\"src/**\"]\ntimeout_s", 1);
            text.push_str(POST);
        }
    }
    files.push((
        ".bay/config.toml".to_owned(),
        "[runner]\npost = true\n".to_owned(),
    ));
    test_box.seed_with_check(
        &repo,
        &[("src/a.rs", "fn a() {}\n"), ("docs/a.md", "a\n")],
        &files,
    );
    let started = test_box.bay_ok(&repo, &["start", "BAD", "--paths", "src/"]);
    let view = PathBuf::from(started["detail"]["view"].as_str().unwrap());
    std::fs::write(view.join("src/bad.rs"), "bad\n").unwrap();
    let (code, landed) = result_json(test_box.bay(&view).args(["merge", "--wait"]));
    assert_eq!(landed["decision"], "merged", "{code} {landed}");
    let out = test_box
        .bay(&repo)
        .args(["--json", "runner", "run", "--once"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");

    let started = test_box.bay_ok(&repo, &["start", "MIXED", "--paths", "src/", "docs/"]);
    let id = started["change"].as_str().unwrap().to_owned();
    let view = PathBuf::from(started["detail"]["view"].as_str().unwrap());
    std::fs::write(view.join("src/c.rs"), "c\n").unwrap();
    std::fs::write(view.join("docs/note.md"), "note\n").unwrap();
    let (_, waiting) = result_json(test_box.bay(&view).args(["merge", "--wait"]));
    assert_eq!(waiting["reason"], "MAIN_RED", "{waiting}");

    let status = test_box.bay_ok(&repo, &["status", &id]);

    let operation = &status["detail"]["changes"][0]["operation"];
    assert_eq!(
        operation["red_holds"],
        serde_json::json!(["src/c.rs"]),
        "{status}"
    );
    assert_eq!(
        operation["red_frees"],
        serde_json::json!(["docs/note.md"]),
        "{status}"
    );
}
