//! G652: a checkout behind main is not told its hooks are missing (a fleet finding, 2026-11-04).
//!
//! A newer bay hooks more events, and main took the entries that add them. Every checkout not yet
//! on that commit read `HOOKS_MISSING`, and doctor said to land a change that had already landed,
//! though taking main is all it needs. When main's settings hold every entry this checkout lacks,
//! and this checkout carries bay's entries from before, the row says so and is not red. Settings
//! main lacks too stay red, as a person must act (G365).
//!
//! Guards to mutate, in `src/doctor.rs` `hooks_row`: `arrives_with_main` returning `false` (the
//! first case reads `HOOKS_MISSING`); returning `true` (G365's first case is no longer red).

use std::path::Path;

use crate::support;

use support::{TestBox, result_json, row};

const CLAUDE_ADAPTER: &str = "[agent]\nproduct = \"claude\"\n";

fn commit_and_push(test_box: &TestBox, repo: &Path, message: &str) {
    test_box.git_ok(repo, &["add", ".claude/settings.json"]);
    test_box.git_ok(
        repo,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "-m",
            message,
        ],
    );
    test_box.git_ok(repo, &["push", "-q", "origin", "main"]);
}

#[test]
fn a_checkout_behind_the_entries_on_main_is_told_to_take_main() {
    let test_box = TestBox::new();
    let repo = test_box.repo_any("repo");
    test_box.commit_config(&repo, CLAUDE_ADAPTER);
    test_box.git_ok(&repo, &["fetch", "-q", "origin", "main"]);
    test_box.git_ok(&repo, &["reset", "-q", "--hard", "origin/main"]);
    test_box.fake_claude(Some("2.1.302"));
    let (code, installed) = result_json(test_box.bay(&repo).arg("install"));
    assert_eq!(code, 0, "{installed}");
    let path = repo.join(".claude/settings.json");
    let complete = std::fs::read_to_string(&path).unwrap();

    // Main first holds what an older bay wrote, then what this one writes.
    let mut older: serde_json::Value = serde_json::from_str(&complete).unwrap();
    older["hooks"]["PreToolUse"]
        .as_array_mut()
        .unwrap()
        .retain(|group| !group.to_string().contains("bay hook tool.ask"));
    std::fs::write(&path, serde_json::to_string_pretty(&older).unwrap()).unwrap();
    commit_and_push(&test_box, &repo, "older entries");
    std::fs::write(&path, &complete).unwrap();
    commit_and_push(&test_box, &repo, "newer entries");
    test_box.git_ok(&repo, &["checkout", "-q", "--detach", "HEAD~1"]);

    let (_, result) = result_json(test_box.bay(&repo).arg("doctor"));

    assert_ne!(result["reason"], "HOOKS_MISSING", "{result}");
    let hooks = row(&result, "hooks");
    assert_eq!(hooks["state"], "behind_main", "{hooks}");
    assert_eq!(hooks["behind"], serde_json::json!(["tool.ask"]), "{hooks}");
    assert!(hooks.get("recovery").is_none(), "{hooks}");
    assert!(
        hooks["note"]
            .as_str()
            .is_some_and(|note| note.contains("when it takes main")),
        "{hooks}"
    );
    assert_eq!(
        row(&result, "agent-hooks")["state"],
        "behind_main",
        "{result}"
    );
}
