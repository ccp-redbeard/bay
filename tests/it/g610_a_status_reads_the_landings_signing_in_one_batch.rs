//! G610 (a fleet finding, 2026-10-31: under `[policy] signing = "approvals"` each `bay status`
//! read the policy at the parent of each of 49 landings apart, to judge each one landed through
//! bay, about 2 s). A status reads those policies in one batch, each distinct text once, and
//! still counts every landing through bay. Four landings cost no more single policy reads than
//! one, read from the trace a test build writes when `BAY_TEST_GIT_TRACE` names a file (G171).
//!
//! Guards to mutate: in `src/unattributed.rs` `scan`, drop the batch (`if signatures {` to
//! `if false {`), and the single reads grow with the landings; in `src/policy.rs`
//! `signing_stated_many`, keep `Signing::All` for each batched text, and the landings no longer
//! count through bay.

// Each landing runs a shell check adapter, so these run on Unix only.
#![cfg(unix)]

use crate::support;

use std::path::{Path, PathBuf};

use serde_json::Value;
use support::{AGENT, REGISTRAR, TestBox, check_files, key_line, passing_result, result_json};

const APPROVALS_ONLY: &str = "\n[policy]\nsigning = \"approvals\"\n";

/// A trunk on a bay host under a key registry with `[policy] signing = "approvals"`, and
/// `landings` changes landed on it through bay.
fn trunk(landings: usize) -> (TestBox, PathBuf) {
    let test_box = TestBox::new();
    let host = test_box.path().join("host.git");
    test_box.host_init(&host);
    let repo = test_box.repo_without_remote_any("repo");
    test_box.add_origin(&repo, &host);
    let mut check = check_files("true", &passing_result(1), 1, 60);
    for (path, text) in &mut check {
        if path == ".bay/policy.toml" {
            text.push_str(APPROVALS_ONLY);
        }
    }
    test_box.seed_with_check(&repo, &[("src/a.rs", "fn a() {}\n")], &check);
    let keys = test_box.keys();
    test_box.enroll_files(
        &repo,
        &keys,
        &[
            ("agent", key_line(AGENT, &keys.agent)),
            ("registry", key_line(REGISTRAR, &keys.registry)),
        ],
    );
    for n in 0..landings {
        let started = test_box.bay_ok(&repo, &["start", &format!("CARD-{n}"), "--paths", "src/"]);
        let view = PathBuf::from(started["detail"]["view"].as_str().unwrap());
        std::fs::write(
            view.join(format!("src/n{n}.rs")),
            format!("fn n{n}() {{}}\n"),
        )
        .unwrap();
        let (code, landed) = result_json(test_box.bay(&view).args(["merge", "--wait"]));
        assert_eq!(landed["decision"], "merged", "{code}: {landed}");
    }
    (test_box, repo)
}

/// One `bay status` in `repo`: its `unattributed` rows, and how many of bay's own git spawns
/// read the policy file of one commit alone.
fn status(test_box: &TestBox, repo: &Path, name: &str) -> (Value, usize) {
    let trace = test_box.path().join(format!("{name}-trace.jsonl"));
    let (code, result) = result_json(
        test_box
            .bay(repo)
            .arg("status")
            .env("BAY_TEST_GIT_TRACE", &trace),
    );
    assert_eq!(code, 0, "{result}");
    let text = std::fs::read_to_string(&trace).expect("the trace was written");
    let lines: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|error| panic!("{error}: {line}")))
        .collect();
    let pid = lines.first().map(|line| line["pid"].clone());
    let single = lines
        .iter()
        .filter(|line| Some(&line["pid"]) == pid.as_ref())
        .filter(|line| {
            line["args"].as_array().is_some_and(|args| {
                args.iter().any(|arg| {
                    arg.as_str()
                        .is_some_and(|arg| arg.ends_with(":.bay/policy.toml"))
                })
            })
        })
        .count();
    (result["detail"]["unattributed"].clone(), single)
}

#[test]
fn four_landings_cost_no_more_single_policy_reads_than_one() {
    let (one_box, one) = trunk(1);
    let (four_box, four) = trunk(4);
    let (_, with_one) = status(&one_box, &one, "one");
    let (rows, with_four) = status(&four_box, &four, "four");
    assert!(
        with_four <= with_one,
        "four landings read {with_four} policies alone, one read {with_one}"
    );
    assert!(rows["through_bay"].as_u64().unwrap() >= 4, "{rows}");
    assert_eq!(
        rows["count"], 0,
        "a landing through bay counted around it: {rows}"
    );
}
