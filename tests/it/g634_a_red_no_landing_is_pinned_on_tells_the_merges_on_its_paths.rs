//! G634 (a fleet finding, 2026-11-02: a red with no culprit was told to its first failure, a
//! merge of three documents no path of the check names, and not to the one merge on its paths
//! since it last passed). A red post check the evidence pins on no landing is told to the merges
//! on the check's paths since it last passed, up to its first failure, once each.
//!
//! A post check runs on main's tip, not on each landing, so a failure on a tip whose first parent
//! never ran pins no culprit. Before G634 its owner was that first failure, on the check's paths
//! or not.
//!
//! Guards to mutate, in `src/red_owner.rs`: in `told`, the range owners (`red_owner(red)` for
//! `range_owners`: the first failure is told); `on_paths`' `red.covers` filter (every landing of
//! the range is told: the one off the check's paths too).

#![cfg(unix)]

use crate::g630_a_red_that_fails_on_a_new_unit_tells_the_merge_on_its_paths::Suite;

use std::collections::BTreeSet;

/// `suite` passes on `base`; then `cause` breaks t1 on src/, `more` (when given) lands on src/
/// too, and `doc` lands off suite's paths. suite runs on the tip alone, `doc`, and fails: no
/// culprit is pinned. Returns the suite, then the ids of `cause`, `more` and `doc`.
fn red_on_a_doc(more: bool) -> (Suite, String, Option<String>, String) {
    let suite = Suite::new();
    suite.land(&["base", "--paths", "src"], &|view| {
        std::fs::write(view.join("src/base.rs"), "x\n").unwrap();
    });
    let ran = suite.run();
    assert_eq!(suite.red(), serde_json::json!([]), "suite passes: {ran}");

    let cause = suite.land(&["cause", "--paths", "src", "units"], &|view| {
        std::fs::write(view.join("src/cause.rs"), "x\n").unwrap();
        std::fs::write(view.join("units/t1"), "fail\n").unwrap();
    });
    let more = more.then(|| {
        suite.land(&["more", "--paths", "src"], &|view| {
            std::fs::write(view.join("src/more.rs"), "x\n").unwrap();
        })
    });
    let doc = suite.land(&["doc", "--paths", "conf"], &|view| {
        std::fs::write(view.join("conf/doc"), "x\n").unwrap();
    });
    let ran = suite.run();
    let red = suite.red();
    assert_eq!(red[0]["check"], "suite", "suite is red: {ran}");
    assert!(red[0]["culprit"].is_null(), "no culprit is pinned: {red}");
    (suite, cause, more, doc)
}

#[test]
fn a_red_no_landing_is_pinned_on_tells_the_one_merge_on_its_paths() {
    let (suite, cause, _, doc) = red_on_a_doc(false);

    // THEN `cause`, the one merge on suite's paths since it passed, is told, once; `doc`, the
    // first failure, is off the paths and is not.
    let wakes = suite.red_wakes();
    assert_eq!(wakes.len(), 1, "{wakes:?}");
    assert_eq!(wakes[0]["change"], cause.as_str(), "{wakes:?}");
    let line = wakes[0]["line"].as_str().unwrap();
    assert!(
        line.contains("is the one merge on its paths since it last passed, not a proven cause"),
        "{line}"
    );
    assert!(
        wakes.iter().all(|wake| wake["change"] != doc.as_str()),
        "{wakes:?}"
    );
    suite.run();
    assert_eq!(suite.red_wakes().len(), 1, "a range owner is told once");
}

#[test]
fn a_red_no_landing_is_pinned_on_tells_each_merge_on_its_paths() {
    let (suite, cause, more, doc) = red_on_a_doc(true);
    let more = more.unwrap();

    // THEN both merges on suite's paths since it passed are told, each once, and `doc` is not.
    let wakes = suite.red_wakes();
    let told: BTreeSet<&str> = wakes
        .iter()
        .map(|wake| wake["change"].as_str().unwrap())
        .collect();
    assert_eq!(
        told,
        BTreeSet::from([cause.as_str(), more.as_str()]),
        "{wakes:?}"
    );
    assert_eq!(wakes.len(), 2, "{wakes:?}");
    for wake in &wakes {
        let line = wake["line"].as_str().unwrap();
        assert!(
            line.contains(
                "is one of 2 merges on its paths since it last passed, not a proven cause"
            ),
            "{line}"
        );
    }
    assert!(!told.contains(doc.as_str()), "{wakes:?}");
}
