//! G633 (a fleet finding, 2026-11-02: a post check's try ended unanswered after 12 s, when its
//! adapter failed to ship the candidate to its worker, and the box waited the whole hour before
//! it tried the check again). A post check no worker answers waits ten times what its try cost,
//! from ten minutes to the hour (BACKLOG 117 kept the hour for a try that cost 383-400 s); a
//! check that holds changes still waits five minutes (G573).
//!
//! `quiet` is not answered on a tip, at once or after 3 s. On the tip after it, with the floor and
//! the wait of a check that holds changes at 0 s, the runner runs `quiet` again after the cheap
//! try, and waits after the costly one.
//!
//! Guards to mutate, in `src/bisect.rs`: in `unavailable_wait`, the wait from the cost (the
//! floor alone: the costly try runs again); in `run_at`, the cost given to `answered` (zero: the
//! same).

use crate::support;

use std::path::{Path, PathBuf};

use serde_json::Value;
use support::{TestBox, result_json};

/// A pre check `tests` that always passes and a post check `quiet` on `conf/**`.
const POLICY: &str = "revision = 1\n\n[[check]]\nid = \"tests\"\nenvironment = \"local\"\ncommand = \"scripts/bay-check\"\nadapter = [\"scripts/bay-check\"]\nexpected_minimum = 1\ntimeout_s = 60\n\n[[check]]\nid = \"quiet\"\nenvironment = \"local\"\nstage = \"post\"\ncommand = \"scripts/bay-check\"\nadapter = [\"scripts/bay-check\"]\nexpected_minimum = 1\ntimeout_s = 60\nwhen_paths = [\"conf/**\"]\n";

/// The adapter. `quiet` reads `conf/state`: `away`, no worker answers it; `slow`, no worker
/// answers it after 3 s; else (`pass`, `back`) it passes. The pre check always passes.
const ADAPTER: &str = r#"#!/bin/sh
req=$(cat)
commit=$(printf '%s' "$req" | sed -n 's/.*"candidate":{"commit":"\([0-9a-f]*\)","tree":"\([0-9a-f]*\)"[,}].*/\1/p')
tree=$(printf '%s' "$req" | sed -n 's/.*"candidate":{"commit":"\([0-9a-f]*\)","tree":"\([0-9a-f]*\)"[,}].*/\2/p')
case "$req" in
*'"plan":{"id":"quiet"'*)
  state=$(cat conf/state)
  [ "$state" = slow ] && sleep 3
  if [ "$state" = away ] || [ "$state" = slow ]; then
    printf '%s\n' '{"schema":"bay.check.result/1","unavailable":"shipping the candidate failed"}'
    exit 0
  fi
  ;;
esac
printf '{"schema":"bay.check.result/1","exercised":{"commit":"%s","tree":"%s"},"plan":{"selected":1,"completed":1},"checks":[{"id":"t1","verdict":"pass"}]}' "$commit" "$tree"
"#;

/// The records of `quiet` on the remote about `tip`.
fn records(test_box: &TestBox, remote: &Path, tip: &str) -> Vec<Value> {
    let refs = test_box.git_bare(
        remote,
        &[
            "for-each-ref",
            "--format=%(refname)",
            "refs/notes/bay/evidence/",
        ],
    );
    let mut out = Vec::new();
    for notes_ref in refs.lines() {
        let blobs = test_box.git_bare(
            remote,
            &["ls-tree", "-r", "--format=%(objectname)", notes_ref],
        );
        for blob in blobs.lines() {
            let text = test_box.git_bare(remote, &["cat-file", "blob", blob]);
            let records: Vec<Value> = serde_json::from_str(&text).unwrap();
            out.extend(records.into_iter().filter(|record| {
                record["check"]["id"] == "quiet" && record["subject"]["commit"] == tip
            }));
        }
    }
    out
}

/// Lands a change named `name` that writes `state` to `conf/state`, and returns main's new tip.
fn land(test_box: &TestBox, (repo, remote): (&Path, &Path), name: &str, state: &str) -> String {
    let started = test_box.bay_ok(repo, &["start", name, "--paths", "conf"]);
    let id = started["change"].as_str().unwrap().to_owned();
    let view = PathBuf::from(started["detail"]["view"].as_str().unwrap());
    std::fs::write(view.join("conf/state"), format!("{state}\n")).unwrap();
    let (code, merged) = result_json(test_box.bay(&view).args(["merge", "--wait"]));
    assert_eq!(code, 0, "{merged}");
    assert!(
        test_box.trunk_change_ids(remote).contains(&id),
        "{name} lands: {merged}"
    );
    test_box
        .git_bare(remote, &["rev-parse", "main"])
        .trim()
        .to_owned()
}

/// Lands a tip on which no worker answers `quiet` (`state`: `away` or `slow`), then one on which
/// it passes, with a runner pass after each: both short waits are 0 s on the second. The second
/// tip's `conf/state` is new (`back`), so its range wakes `quiet` (G439). Returns the cost the box
/// noted, whether `quiet` ran on the second tip, and the second tip's passes.
fn unanswered_then_back(state: &str) -> (u64, bool, String) {
    let test_box = TestBox::new();
    let repo = test_box.repo_any("repo");
    let remote = test_box.path().join("repo-remote.git");
    let files = vec![
        ("scripts/bay-check".to_owned(), ADAPTER.to_owned()),
        (".bay/policy.toml".to_owned(), POLICY.to_owned()),
        (
            ".bay/config.toml".to_owned(),
            "[runner]\npost = true\nbisect_post = false\n".to_owned(),
        ),
    ];
    test_box.seed_with_check(
        &repo,
        &[("src/a.rs", "fn a() {}\n"), ("conf/state", "pass\n")],
        &files,
    );
    let pass = |env: &[(&str, &str)]| {
        let mut cmd = test_box.bay(&repo);
        for (key, value) in env {
            cmd.env(key, value);
        }
        let (code, ran) = result_json(cmd.args(["runner", "run", "--once"]));
        assert_eq!(code, 0, "{ran}");
        ran
    };
    let noted = || -> Option<Value> {
        let text = std::fs::read_to_string(repo.join(".git/bay/post-unavailable.json")).ok()?;
        serde_json::from_str::<Value>(&text)
            .ok()?
            .get("quiet@local")
            .cloned()
    };

    // GIVEN a tip on which no worker answers `quiet`: the box notes what the try cost.
    let away = land(&test_box, (&repo, &remote), "away", state);
    let mut ran = Vec::new();
    for _ in 0..3 {
        if noted().is_some() {
            break;
        }
        ran.push(pass(&[]));
    }
    let note = noted().unwrap_or_else(|| panic!("quiet was tried and noted: {ran:?}"));
    assert!(records(&test_box, &remote, &away).is_empty(), "{ran:?}");

    // WHEN workers answer again on the next tip, with both short waits at 0 s (`run_at` checks
    // the wait of a check that holds changes again).
    let back = land(&test_box, (&repo, &remote), "back", "back");
    let short = [
        ("BAY_TEST_UNAVAILABLE_FLOOR_S", "0"),
        ("BAY_TEST_HOLDING_UNAVAILABLE_WAIT_S", "0"),
    ];
    let ran = [pass(&short), pass(&short)];
    let records = records(&test_box, &remote, &back);
    if let Some(record) = records.first() {
        assert_eq!(record["verdict"], "pass", "{record} {ran:?}");
    }
    (
        note["cost_s"].as_u64().unwrap(),
        !records.is_empty(),
        format!("{ran:?}"),
    )
}

#[test]
#[cfg_attr(
    windows,
    ignore = "Windows: a timing bound this machine's process starts miss; under investigation"
)]
fn a_cheap_try_no_worker_answered_is_tried_again_after_the_floor() {
    let (cost, ran, passes) = unanswered_then_back("away");
    // THEN the try cost under a second, and `quiet` ran again on the next tip.
    assert_eq!(cost, 0, "{passes}");
    assert!(
        ran,
        "a try that cost nothing waits only the floor: {passes}"
    );
}

#[test]
#[cfg_attr(
    windows,
    ignore = "Windows: a timing bound this machine's process starts miss; under investigation"
)]
fn a_costly_try_no_worker_answered_waits_ten_times_its_cost() {
    let (cost, ran, passes) = unanswered_then_back("slow");
    // THEN the try cost 3 s, and `quiet` still waits its 30 s on the next tip.
    assert!(cost >= 3, "{cost} {passes}");
    assert!(
        !ran,
        "a try that cost {cost} s waits {} s: {passes}",
        cost * 10
    );
}
