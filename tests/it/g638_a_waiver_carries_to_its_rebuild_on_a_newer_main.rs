//! G638 (a fleet finding, 2026-11-02: a change blocked on a waivable check waited for its
//! approver; every landing on main before the approver came would have voided the waiver, since
//! the author's retry rebuilt the change on the new main, and N-9 names one candidate): a
//! waiver carries to a rebuilt candidate of the same change on another tree, when the change's
//! patch is the same and the check fails there the same way. The landing names the waiver with
//! `carried_from`, the candidate the approver waived, and the real verdict. A check the waived
//! failure ended the run before is run on the rebuilt candidate before it lands; when it fails,
//! `bay waiting` asks the approver only about it. A change refreshed onto the newer main
//! (`bay start <change>`) takes the waiver too: its merge reads the waiver from the remote. A
//! failure that reads differently on the new main, a new revision of the change, or a new
//! candidate on the waived one's tree carries nothing.
//!
//! Guards to mutate (each makes a case here fail):
//! - `src/policy.rs` `Policy::waiver`: drop the `.or_else(..)` over `carried` (the rebuilt
//!   candidate stays `CHECK_FAILED`: the first case).
//! - `src/throughput.rs` `carried_waivers`: drop the compare of the two outcomes (the failure
//!   whose reason changed lands: the second case).
//! - `src/throughput.rs` `carried_waivers`: drop the compare of the two patch ids (the new
//!   revision lands: the third case).
//! - `src/throughput.rs` `carried_waivers`: drop `candidate.tree != tree` (a waiver of an
//!   earlier candidate on the same tree lands a new one: the fourth case).
//! - `src/publish.rs` `run_policy`: drop the G638 second pass (`lint` never runs on the rebuilt
//!   candidate, and it does not land: the first case).
//! - `src/publish.rs` `waiver_gaps`: drop the `carried_from` line (the first case).
//! - `src/publish.rs` `waivable_gaps`: pass no `carried` to `Policy::gaps` (`bay waiting` asks
//!   the approver to waive `tests` again: the fifth case).
//! - `src/approve.rs` `waivable`: pass no earlier candidates to `waivable_gaps` (the same: the
//!   fifth case).
//! - `src/publish.rs` `fetch_for_carry`: drop the fetch (the refreshed change's merge never sees
//!   the waiver, and it stays `CHECK_FAILED`: the sixth case).

// Each case runs shell scripts (adapters, a wrapper on PATH), so these run on Unix only.
#![cfg(unix)]

use crate::support;

use std::path::PathBuf;

use serde_json::Value;
use support::UNSANDBOXED_CHOICE;
use support::approval::{APPROVER, Trunk, trunk_before};

/// The `tests` check's own key that makes it waivable, then a second run check, `lint`, after
/// it: a `tests` failure ends the run before `lint` starts.
const WAIVABLE_THEN_LINT: &str = "waivable = true\n\n[[check]]\nid = \"lint\"\nenvironment = \"local\"\ncommand = \"scripts/bay-check\"\nadapter = [\"scripts/bay-check\"]\nexpected_minimum = 1\ntimeout_s = 60\n";

/// The script's answer for `lint`: it passes every candidate.
fn lint_passes() -> String {
    format!(
        "case \"$req\" in *'\"plan\":{{\"id\":\"lint\"'*) {}; exit 0;; esac\n",
        support::passing_result(1)
    )
}

/// The script's answer for a candidate that holds `src/fail-moving`: `tests` fails unit `t1`
/// with a reason that counts the files under `src/`, so it reads differently on a newer main.
fn fails_with_the_count() -> String {
    let failing = "printf '%s' \"{\\\"schema\\\":\\\"bay.check.result/1\\\",\\\"exercised\\\":{\\\"commit\\\":\\\"$commit\\\",\\\"tree\\\":\\\"$tree\\\"},\\\"plan\\\":{\\\"discovered\\\":1,\\\"selected\\\":1,\\\"completed\\\":1},\\\"checks\\\":[{\\\"id\\\":\\\"t1\\\",\\\"verdict\\\":\\\"product_failure\\\",\\\"reason\\\":\\\"wrong with $n files\\\"}]}\"";
    format!("n=$(ls src | wc -l | tr -d ' ')\n[ -e src/fail-moving ] && {{ {failing}; exit 0; }}\n")
}

/// The script's answer for `lint` on a candidate that holds both `src/fail` and `src/other.rs`
/// (the change rebuilt on the main [`main_moves`] made): it fails unit `t1`.
fn lint_fails_on_the_newer_main() -> String {
    let failing = support::passing_result(1).replace("\\\"pass\\\"", "\\\"product_failure\\\"");
    format!(
        "case \"$req\" in *'\"plan\":{{\"id\":\"lint\"'*) [ -e src/fail ] && [ -e src/other.rs ] && {{ {failing}; exit 0; }};; esac\n"
    )
}

/// The trunk: `tests` waivable and `lint` after it, both run by a script that first runs
/// `before`. On a box whose global config names no producer, so the author's and the
/// approver's clones each take their own name (G297).
fn trunk_running(before: &str) -> Trunk {
    trunk_of(WAIVABLE_THEN_LINT, &format!("{}{before}", lint_passes()))
}

/// [`trunk_running`], with `tests` as the `tests` check's own keys and `lint` after it, and
/// `before` as the whole start of the script.
fn trunk_of(tests: &str, before: &str) -> Trunk {
    let trunk = trunk_before(tests, before);
    std::fs::write(&trunk.test_box.global_config, UNSANDBOXED_CHOICE).unwrap();
    trunk
}

/// A change on `src/` that adds `file`, which `tests` fails, landed once: it is blocked
/// `CHECK_FAILED` and `lint` did not run. Its id and view.
fn failed_change(trunk: &Trunk, file: &str) -> (String, PathBuf) {
    let (id, view) = trunk.start("CARD-fail", &["src/"], &[(file, "1\n")]);
    let (code, blocked) = trunk.land(&view);
    assert_eq!(code, 10, "{blocked}");
    assert_eq!(blocked["reason"], "CHECK_FAILED", "{blocked}");
    let rows = blocked["detail"]["checks"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        rows.iter()
            .any(|row| row["check"] == "lint" && row["verdict"] == "not_run"),
        "lint ran after the failed tests: {blocked}"
    );
    (id, view)
}

/// The approver waives `tests` on the pending candidate of `id`: that candidate and the
/// waiver's record.
fn waive(trunk: &Trunk, id: &str) -> (String, String) {
    let (code, said) = trunk.approve(
        &trunk.approver_box,
        id,
        &["--waive", "tests", "--why", "a known failure"],
    );
    assert_eq!(code, 0, "{said}");
    let candidate = said["detail"]["commit"].as_str().unwrap().to_owned();
    let record = said["detail"]["records"][0].as_str().unwrap().to_owned();
    (candidate, record)
}

/// Another change, on a file of its own under `src/`, lands: main moves.
fn main_moves(trunk: &Trunk) {
    let (_, view) = trunk.start(
        "CARD-other",
        &["src/"],
        &[("src/other.rs", "fn other() {}\n")],
    );
    let (code, landed) = trunk.land(&view);
    assert_eq!(code, 0, "{landed}");
    assert_eq!(landed["decision"], "merged", "{landed}");
}

/// `bay merge --wait` in `view`, refused `CHECK_FAILED`.
fn still_failed(trunk: &Trunk, view: &std::path::Path) -> Value {
    let (code, refused) = trunk.land(view);
    assert_eq!(code, 10, "a waiver carried: {refused}");
    assert_eq!(refused["reason"], "CHECK_FAILED", "{refused}");
    refused
}

/// The item of `bay waiting` in the approver's clone for change `id`.
fn waiting_for(trunk: &Trunk, id: &str) -> Value {
    let listed = trunk.waiting(&trunk.approver_box);
    listed["detail"]["waiting"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["change"] == id)
        .cloned()
        .unwrap_or_else(|| panic!("bay waiting lists no waivable check of {id}: {listed}"))
}

/// The checks the item asks the approver to waive.
fn asked(item: &Value) -> Vec<String> {
    item["waivable"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["check"].as_str().unwrap().to_owned())
        .collect()
}

/// The coverage gaps of `landed` that name a waiver.
fn waived_gaps(landed: &Value) -> Vec<Value> {
    landed["detail"]["coverage_gaps"]
        .as_array()
        .unwrap_or_else(|| panic!("no coverage_gaps: {landed}"))
        .iter()
        .filter(|gap| gap.get("waived_by").is_some())
        .cloned()
        .collect()
}

#[test]
fn a_waiver_carries_to_the_rebuild_on_a_newer_main_and_the_landing_names_it() {
    let trunk = trunk_running("");
    let (id, view) = failed_change(&trunk, "src/fail");
    let (waived, record) = waive(&trunk, &id);
    main_moves(&trunk);

    // The author's retry rebuilds the change on the new main: `tests` fails there as before,
    // the waiver carries, `lint` runs, and the rebuilt candidate lands.
    let (code, landed) = trunk.land(&view);
    assert_eq!(code, 0, "{landed}");
    assert_eq!(landed["decision"], "merged", "{landed}");
    assert_ne!(trunk.trunk_tip(), waived, "the rebuilt candidate landed");
    let gaps: Vec<&Value> = landed["detail"]["coverage_gaps"]
        .as_array()
        .unwrap_or_else(|| panic!("no coverage_gaps: {landed}"))
        .iter()
        .filter(|gap| gap.get("waived_by").is_some())
        .collect();
    assert_eq!(gaps.len(), 1, "{landed}");
    let gap = gaps[0];
    assert_eq!(gap["check"], "tests", "{landed}");
    assert_eq!(gap["waived_by"], APPROVER, "{landed}");
    assert_eq!(gap["record"], record.as_str(), "{landed}");
    assert_eq!(gap["carried_from"], waived.as_str(), "{landed}");
    assert_eq!(gap["verdict"], "product_failure", "{landed}");
    let say = landed["say"].as_str().unwrap_or_default();
    assert!(say.contains("waived"), "{landed}");
}

#[test]
fn a_failure_that_reads_differently_on_the_newer_main_takes_no_carried_waiver() {
    let trunk = trunk_running(&fails_with_the_count());
    let (id, view) = failed_change(&trunk, "src/fail-moving");
    waive(&trunk, &id);
    main_moves(&trunk);

    still_failed(&trunk, &view);
}

#[test]
fn a_new_revision_of_the_change_takes_no_carried_waiver() {
    let trunk = trunk_running("");
    let (id, view) = failed_change(&trunk, "src/fail");
    waive(&trunk, &id);

    // The author changes the change: its failure reads as before, but the patch the approver
    // weighed is not this one.
    std::fs::write(view.join("src/more.rs"), "fn more() {}\n").unwrap();
    still_failed(&trunk, &view);
}

#[test]
fn a_waiver_of_an_earlier_candidate_on_the_same_tree_answers_nothing() {
    // `tests` runs on each candidate commit, so a new candidate on the waived one's tree has
    // a failure of its own that reads as the waived one.
    let trunk = trunk_of(
        &format!("inputs = [\"commit\"]\n{WAIVABLE_THEN_LINT}"),
        &lint_passes(),
    );
    let (id, view) = failed_change(&trunk, "src/fail");
    let (waived, _) = waive(&trunk, &id);
    let before = trunk.trunk_tip();

    // The author changes the change, then changes it back: the last revision's candidate is
    // on the waived one's tree, with the same patch, on the same main. N-9 names the waived
    // candidate, and on its tree no other candidate takes its waiver.
    std::fs::write(view.join("src/more.rs"), "fn more() {}\n").unwrap();
    still_failed(&trunk, &view);
    std::fs::remove_file(view.join("src/more.rs")).unwrap();
    still_failed(&trunk, &view);
    let pending = waiting_for(&trunk, &id)["commit"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(pending, waived, "the last revision checks a new candidate");
    let tree = |commit: &str| {
        trunk.test_box.git_ok(
            &trunk.approver_box,
            &["rev-parse", &format!("{commit}^{{tree}}")],
        )
    };
    assert_eq!(
        tree(&pending),
        tree(&waived),
        "the candidates share one tree"
    );
    assert_eq!(trunk.trunk_tip(), before, "trunk did not move");
}

#[test]
fn a_check_that_fails_only_on_the_rebuild_is_the_one_the_approver_is_asked_about() {
    // `lint` is waivable too, and fails only on the main `main_moves` makes.
    let trunk = trunk_of(
        &format!("{WAIVABLE_THEN_LINT}waivable = true\n"),
        &format!("{}{}", lint_fails_on_the_newer_main(), lint_passes()),
    );
    let (id, view) = failed_change(&trunk, "src/fail");
    let (waived, record) = waive(&trunk, &id);
    main_moves(&trunk);

    // On the new main the waiver carries for `tests`, and `lint` then fails: the approver is
    // asked about `lint` only.
    let refused = still_failed(&trunk, &view);
    let rows = refused["detail"]["checks"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        rows.iter()
            .any(|row| row["check"] == "lint" && row["verdict"] == "product_failure"),
        "{refused}"
    );
    assert_eq!(asked(&waiting_for(&trunk, &id)), ["lint"]);

    // Waived too, the rebuilt candidate lands, naming both waivers.
    let (code, said) = trunk.approve(
        &trunk.approver_box,
        &id,
        &["--waive", "lint", "--why", "a known lint"],
    );
    assert_eq!(code, 0, "{said}");
    let rebuilt = said["detail"]["commit"].as_str().unwrap().to_owned();
    assert_ne!(rebuilt, waived, "{said}");
    let (code, landed) = trunk.land(&view);
    assert_eq!(code, 0, "{landed}");
    assert_eq!(landed["decision"], "merged", "{landed}");
    let gaps = waived_gaps(&landed);
    assert_eq!(gaps.len(), 2, "{landed}");
    let tests = gaps.iter().find(|gap| gap["check"] == "tests").unwrap();
    assert_eq!(tests["record"], record.as_str(), "{landed}");
    assert_eq!(tests["carried_from"], waived.as_str(), "{landed}");
    let lint = gaps.iter().find(|gap| gap["check"] == "lint").unwrap();
    assert!(lint.get("carried_from").is_none(), "{landed}");
}

#[test]
fn a_refreshed_change_takes_the_waiver_of_its_candidate_before_the_refresh() {
    let trunk = trunk_running("");
    let (id, view) = failed_change(&trunk, "src/fail");
    let (waived, record) = waive(&trunk, &id);
    main_moves(&trunk);

    // The author brings the newer main into the change (`bay start <change>`), then merges:
    // the patch is the one the approver weighed, so the waiver carries.
    let refreshed = trunk.test_box.bay_ok(&trunk.repo, &["start", &id]);
    let view = refreshed["detail"]["view"]
        .as_str()
        .map_or(view, PathBuf::from);
    let (code, landed) = trunk.land(&view);
    assert_eq!(code, 0, "{landed}");
    assert_eq!(landed["decision"], "merged", "{landed}");
    let gaps = waived_gaps(&landed);
    assert_eq!(gaps.len(), 1, "{landed}");
    assert_eq!(gaps[0]["record"], record.as_str(), "{landed}");
    assert_eq!(gaps[0]["carried_from"], waived.as_str(), "{landed}");
}
