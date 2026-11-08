//! G602 (a fleet finding, 2026-10-31: sessions started with `BAY_ACTOR` unset merged changes as
//! the box's user name, with `actor_source` `env`). Session start writes `BAY_ACTOR` into the
//! product's `CLAUDE_ENV_FILE` only when the environment named the actor. A `USER` or built-in
//! default is never written there, so the session's later commands still see a default (G152),
//! and `[agent] require_actor` can refuse them.
//!
//! Guard to mutate: `src/session.rs` `bind_env`: the `actor_source() == Env` test.

use crate::support;

use std::io::Write;
use std::path::Path;
use std::process::Stdio;

use serde_json::{Value, json};
use support::{TestBox, result_json};

/// `bay hook session.start` with `CLAUDE_ENV_FILE` and `envs`: the env file's exports, as the
/// product would source them before each later command.
fn session_start(
    test_box: &TestBox,
    repo: &Path,
    id: &str,
    envs: &[(&str, &str)],
) -> Vec<(String, String)> {
    let env_file = test_box.path().join(format!("{id}.sh"));
    let payload = json!({"session_id": id, "hook_event_name": "SessionStart", "cwd": repo});
    let mut command = test_box.bay_process(repo);
    command.env("CLAUDE_ENV_FILE", &env_file);
    for (key, value) in envs {
        command.env(key, value);
    }
    let mut child = command
        .args(["hook", "session.start"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .unwrap();
    assert_eq!(child.wait_with_output().unwrap().status.code(), Some(0));
    let text = std::fs::read_to_string(&env_file).unwrap();
    text.lines()
        .map(|line| {
            let (key, value) = line
                .strip_prefix("export ")
                .and_then(|rest| rest.split_once('='))
                .unwrap_or_else(|| panic!("{text}"));
            (key.to_owned(), value.trim_matches('\'').to_owned())
        })
        .collect()
}

fn start_source(
    test_box: &TestBox,
    repo: &Path,
    intent: &str,
    envs: &[(&str, &str)],
    exports: &[(String, String)],
) -> Value {
    let mut command = test_box.bay(repo);
    for (key, value) in envs {
        command.env(key, value);
    }
    for (key, value) in exports {
        command.env(key, value);
    }
    let (code, result) = result_json(command.args(["start", intent, "--paths", "src/"]));
    assert_eq!(code, 0, "{result}");
    result["detail"]["actor_source"].clone()
}

#[test]
fn a_default_actor_stays_a_default_after_session_start() {
    let test_box = TestBox::new();
    let repo = test_box.repo_any("repo");
    test_box.seed_trunk(&repo, &[("src/a.rs", "fn a() {}\n")]);

    let user = [("USER", "box-login")];
    let exports = session_start(&test_box, &repo, "s-user", &user);
    let keys: Vec<&str> = exports.iter().map(|(key, _)| key.as_str()).collect();
    assert_eq!(keys, ["BAY_SESSION"], "{exports:?}");
    let source = start_source(&test_box, &repo, "CARD-1", &user, &exports);
    assert_eq!(source, "user-default");

    let exports = session_start(&test_box, &repo, "s-bare", &[]);
    let keys: Vec<&str> = exports.iter().map(|(key, _)| key.as_str()).collect();
    assert_eq!(keys, ["BAY_SESSION"], "{exports:?}");
}

#[test]
fn a_named_actor_is_written_for_the_session() {
    let test_box = TestBox::new();
    let repo = test_box.repo_any("repo");
    test_box.seed_trunk(&repo, &[("src/a.rs", "fn a() {}\n")]);

    let named = [("BAY_ACTOR", "seat-a"), ("USER", "box-login")];
    let exports = session_start(&test_box, &repo, "s-named", &named);
    assert!(
        exports.contains(&("BAY_ACTOR".to_owned(), "seat-a".to_owned())),
        "{exports:?}"
    );
    // Only the env file names the actor now: the session's later command keeps the name.
    let user = [("USER", "box-login")];
    let source = start_source(&test_box, &repo, "CARD-1", &user, &exports);
    assert_eq!(source, "env");
}
