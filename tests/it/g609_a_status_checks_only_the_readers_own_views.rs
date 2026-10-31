//! G609 (a fleet finding, 2026-10-31: on a box shared by 34 agents, each `bay status` checked
//! the 20 views other agents' ended changes left, about 12 s a status). A listing checks the
//! reader's own views; another actor's ended views are named in `more_leftovers`, never
//! checked, and `bay status <change>` checks one.
//!
//! Guard to mutate, in `src/status.rs` `leftovers`: list the views of every ended change again,
//! and the other actor's status checks the abandoned view.

// The case puts a shell wrapper for `git` on PATH, so it runs on Unix only.
#![cfg(unix)]

use crate::support;

use std::path::{Path, PathBuf};

use serde_json::Value;
use support::{TestBox, result_json};

/// `bay <args>` as `actor`, which must succeed.
fn bay_as(test_box: &TestBox, repo: &Path, actor: &str, args: &[&str]) -> Value {
    let (code, result) = result_json(test_box.bay(repo).args(args).env("BAY_ACTOR", actor));
    assert_eq!(code, 0, "bay {args:?} as {actor}: {result}");
    result
}

/// The leftover row of `id` in `status`, if the status checked that view.
fn leftover<'a>(status: &'a Value, id: &str) -> Option<&'a Value> {
    status["detail"]["leftovers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["change"] == id)
}

#[test]
fn a_status_names_another_actors_ended_views_and_checks_only_its_own() {
    let test_box = TestBox::new();
    let repo = test_box.repo_any("repo");
    test_box.seed_trunk(&repo, &[("src/a.rs", "fn a() {}\n")]);

    // Actor `a` leaves a view with an unsaved edit and abandons its change.
    let started = bay_as(
        &test_box,
        &repo,
        "a",
        &["start", "CARD-1", "--paths", "src/"],
    );
    let id = started["change"].as_str().unwrap().to_owned();
    let view = PathBuf::from(started["detail"]["view"].as_str().unwrap());
    std::fs::write(view.join("src/a.rs"), "fn a() { edited }\n").unwrap();
    bay_as(
        &test_box,
        &repo,
        "a",
        &["abandon", &id, "--why", "moved on"],
    );
    assert!(
        view.is_dir(),
        "the abandon removed a view with an unsaved edit"
    );

    // Every git spawn that names the view is kept.
    let spawns = test_box.path().join("spawns");
    test_box.wrap_git(&format!(
        "case \"$*\" in *{view}*) echo \"$*\" >> '{spawns}' ;; esac",
        view = view.display(),
        spawns = spawns.display(),
    ));
    let checked = || {
        let text = std::fs::read_to_string(&spawns).unwrap_or_default();
        let _ = std::fs::remove_file(&spawns);
        text.lines().count()
    };

    // Actor `b`'s listing names the view and runs no git in it.
    let listing = bay_as(&test_box, &repo, "b", &["status"]);
    assert!(
        leftover(&listing, &id).is_none(),
        "b's listing checked a's view: {listing}"
    );
    let more = &listing["detail"]["more_leftovers"];
    assert_eq!(more["count"], 1, "{listing}");
    assert_eq!(more["ids"][0], id.as_str(), "{listing}");
    assert_eq!(checked(), 0, "b's listing ran git in a's view");

    // Read by name, the view is checked, and its unsaved edit is reported.
    let named = bay_as(&test_box, &repo, "b", &["status", &id]);
    let row = leftover(&named, &id).unwrap_or_else(|| panic!("not checked by name: {named}"));
    assert_eq!(row["uncheckpointed"], true, "{row}");
    assert!(checked() > 0, "a status by name ran no git in the view");

    // Its own actor's listing checks it.
    let own = bay_as(&test_box, &repo, "a", &["status"]);
    let row = leftover(&own, &id).unwrap_or_else(|| panic!("a's listing skipped it: {own}"));
    assert_eq!(row["state"], "abandoned", "{row}");
    assert_eq!(row["uncheckpointed"], true, "{row}");
    assert!(own["detail"]["more_leftovers"].is_null(), "{own}");
}
