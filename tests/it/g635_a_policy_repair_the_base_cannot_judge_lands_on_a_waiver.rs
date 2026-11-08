//! G635 (a fleet finding, 2026-11-02: a policy fix that copied a file every TypeScript check
//! needed was judged by the policy it fixed too, whose checks could not install without that
//! file, so they answered unknown on each of its candidates, and no signature or approval check
//! could let it through). Under `signing = "approvals"`, a candidate that changes only the
//! policy and adapter paths is judged by the base's checks with each one waivable: an enrolled
//! approver may waive one on that exact candidate, and it lands named as a gap. Its own new
//! policy still runs every check, and none of those is waivable. A candidate that changes any
//! other path is no repair, and its base's checks stay as the policy marks them.
//!
//! The repair here lists `src/fail` (or `src/unknown`) as an adapter path of its own policy
//! and rewrites the adapter so it no longer fails (or crashes) on that file: main's adapter, run
//! on the candidate, fails (or gives no clear answer), as main's policy could not judge the fix
//! that adds the file it lacked.
//!
//! Guards to mutate (each makes a case here fail):
//! - `src/policy.rs` `Policy::with_repair_waivers`: drop the loop (`bay waiting` lists
//!   nothing: the first case).
//! - `src/policy.rs` `repairs`: drop the `only_policy_paths` test, as `Ok(true)` (the change
//!   that also edits `src/b.rs` takes a waiver: the fourth case).
//! - `src/publish.rs` `judge`: the G635 arm returns `Some(base)` (the waived repair stays
//!   `CHECK_FAILED`: the first case).
//! - `src/publish.rs` `waivable_policy` and `waivable_gaps`, `src/approve.rs` `waive_in`,
//!   `src/outside.rs` `approval_policies`: each `load_base_of` to `load_at` (no waiver is
//!   listed, written or read: the first case).
//! - `src/publish.rs` `coverage_gaps`: `load_base_of` to `load_at` (the landing names no
//!   waiver: the first case).
//! - `src/publish.rs` `finished_checks`: drop `waived` (the base's check that gave no clear
//!   answer runs again on the waived candidate, and its record collides: the second case).

use crate::support;

use std::path::{Path, PathBuf};

use serde_json::Value;
use support::UNSANDBOXED_CHOICE;
use support::approval::{APPROVER, Trunk, trunk_without_approval_check};

/// The base's adapter entry; a repair adds the file its adapter fails on to it.
const ADAPTER: &str = "adapter = [\"scripts/bay-check\"]";
/// The base's `tests` fails a candidate that holds `src/fail`.
const FAILS: (&str, &str) = ("fail", "CHECK_FAILED");
/// The base's `tests` crashes on a candidate that holds `src/unknown`: no clear answer.
const CRASHES: (&str, &str) = ("unknown", "CHECK_UNKNOWN");

/// [`trunk_without_approval_check`], as a repository may ask no approval, on a box whose global
/// config names no producer, as a person's machine has none (G297): the approver reads the
/// author's evidence as another writer's.
fn unnamed() -> Trunk {
    let trunk = trunk_without_approval_check();
    std::fs::write(&trunk.test_box.global_config, UNSANDBOXED_CHOICE).unwrap();
    trunk
}

/// A change that raises the policy's revision, lists `src/<on>` as an adapter path and adds
/// it, and, when `fixed`, rewrites the adapter so it no longer acts on `src/<on>`; `more` adds
/// a file no policy names. Landed once, it is blocked with `reason`. Its id and view.
fn repair(trunk: &Trunk, (on, reason): (&str, &str), fixed: bool, more: bool) -> (String, PathBuf) {
    let (id, view) = trunk.start("CARD-repair", &[".bay/", "scripts/", "src/"], &[]);
    let policy = std::fs::read_to_string(view.join(".bay/policy.toml")).unwrap();
    assert!(
        policy.contains("revision = 1\n") && policy.contains(ADAPTER),
        "{policy}"
    );
    let policy = policy.replace("revision = 1\n", "revision = 2\n").replace(
        ADAPTER,
        &format!("adapter = [\"scripts/bay-check\", \"src/{on}\"]"),
    );
    std::fs::write(view.join(".bay/policy.toml"), policy).unwrap();
    if fixed {
        let script = std::fs::read_to_string(view.join("scripts/bay-check")).unwrap();
        let acts = format!("[ -e src/{on} ] && ");
        assert!(script.contains(&acts), "{script}");
        std::fs::write(
            view.join("scripts/bay-check"),
            script.replace(&acts, "[ -e src/never ] && "),
        )
        .unwrap();
    }
    std::fs::write(view.join("src").join(on), "1\n").unwrap();
    if more {
        std::fs::write(view.join("src/b.rs"), "fn b() {}\n").unwrap();
    }
    let (code, blocked) = trunk.land(&view);
    assert_eq!(code, 10, "{blocked}");
    assert_eq!(blocked["reason"], reason, "{blocked}");
    (id, view)
}

/// The item of `bay waiting` in `dir` for change `id`.
fn waiting_item(trunk: &Trunk, dir: &Path, id: &str) -> Option<Value> {
    let listed = trunk.waiting(dir);
    listed["detail"]["waiting"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["change"] == id)
        .cloned()
}

/// The approver's waiver of `tests` on change `id`.
fn waive(trunk: &Trunk, id: &str) -> (i32, Value) {
    trunk.approve(
        &trunk.approver_box,
        id,
        &["--waive", "tests", "--why", "main cannot install the fix"],
    )
}

/// A repair the base's `tests` blocks as `kind` says, with `verdict`: the approver's waiver lands
/// it, named as a gap with that verdict.
fn lands_on_a_waiver(kind: (&str, &str), verdict: &str) {
    let trunk = unnamed();
    let (id, view) = repair(&trunk, kind, true, false);

    // THEN `bay waiting` lists the base's `tests`, which the policy never marked waivable.
    let item = waiting_item(&trunk, &trunk.approver_box, &id)
        .unwrap_or_else(|| panic!("bay waiting lists no waivable check of the repair {id}"));
    let row = &item["waivable"][0];
    assert_eq!(row["check"], "tests", "{item}");
    assert_eq!(row["verdict"], verdict, "{item}");
    assert_eq!(
        row["waive"],
        format!("bay approve {id} --waive tests --why <text>"),
        "{item}"
    );
    let candidate = item["commit"].as_str().unwrap().to_owned();

    let (code, said) = waive(&trunk, &id);
    assert_eq!(code, 0, "{said}");
    assert_eq!(said["detail"]["waived"], "tests", "{said}");
    assert_eq!(said["detail"]["commit"], candidate.as_str(), "{said}");

    // WHEN the author merges again, THEN the same candidate lands; its own policy's `tests`
    // ran and passed, and the base's is named as a waiver with its real verdict.
    let (code, landed) = trunk.land(&view);
    assert_eq!(code, 0, "{landed}");
    assert_eq!(landed["decision"], "merged", "{landed}");
    let gaps = landed["detail"]["coverage_gaps"]
        .as_array()
        .unwrap_or_else(|| panic!("no coverage_gaps: {landed}"));
    let gap = gaps
        .iter()
        .find(|gap| gap["check"] == "tests" && gap.get("waived_by").is_some())
        .unwrap_or_else(|| panic!("no waiver gap: {landed}"));
    assert_eq!(gap["waived_by"], APPROVER, "{landed}");
    assert_eq!(gap["why"], "main cannot install the fix", "{landed}");
    assert_eq!(gap["verdict"], verdict, "{landed}");
    let say = landed["say"].as_str().unwrap_or_default();
    assert!(say.contains("waived"), "{landed}");
}

#[test]
fn a_repair_the_base_fails_lands_on_a_waiver_named_as_a_gap() {
    lands_on_a_waiver(FAILS, "product_failure");
}

#[test]
fn a_repair_the_base_gives_no_clear_answer_lands_on_a_waiver_named_as_a_gap() {
    lands_on_a_waiver(CRASHES, "unknown");
}

#[test]
fn a_repair_whose_own_policy_fails_takes_no_waiver_of_its_own_check() {
    let trunk = unnamed();
    // The adapter still fails on `src/fail`, so the repair's own policy fails it too.
    let (id, view) = repair(&trunk, FAILS, false, false);
    let (code, said) = waive(&trunk, &id);
    assert_eq!(code, 0, "the base's check takes a waiver: {said}");

    // THEN the own policy's `tests` runs and fails, and no waiver answers it.
    let before = trunk.trunk_tip();
    let (code, still) = trunk.land(&view);
    assert_eq!(code, 10, "{still}");
    assert_eq!(still["reason"], "CHECK_FAILED", "{still}");
    assert_eq!(trunk.trunk_tip(), before, "{still}");
    let rows = waiting_item(&trunk, &trunk.approver_box, &id)
        .map(|item| item["waivable"].as_array().cloned().unwrap_or_default())
        .unwrap_or_default();
    assert!(
        rows.is_empty(),
        "the own policy's check is never waivable: {rows:?}"
    );
}

#[test]
fn a_change_to_another_path_is_no_repair_and_takes_no_waiver() {
    let trunk = unnamed();
    let (id, _) = repair(&trunk, FAILS, true, true);

    // THEN its base's `tests` stays as the policy marks it: not waivable.
    assert!(
        waiting_item(&trunk, &trunk.approver_box, &id).is_none(),
        "a change to src/b.rs is no repair"
    );
    let (code, refused) = waive(&trunk, &id);
    assert_eq!(code, 20, "{refused}");
    assert!(
        refused["detail"]["why"]
            .as_str()
            .is_some_and(|why| why.contains("tests is not waivable")),
        "{refused}"
    );
}
