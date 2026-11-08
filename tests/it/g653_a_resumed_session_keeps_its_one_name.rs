//! G653 (a fleet finding, 2026-11-05: a session resumed with no `BAY_ACTOR` was refused
//! `VIEW_NOT_YOURS` from the view it had opened). The product's env file still gave its shell the
//! name it started with, and its hooks fell back to the user name, so one session was two actors,
//! and its take-over of its own change sat out the takeover window.
//!
//! Now a session has one name. Session start keeps a name the environment gave, and every later
//! hook of that session reads it when its own process has none. A session whose env file holds a
//! name from before this bay gets it from there at its next start. A process that names an actor
//! still decides, and a default is never kept (G602).
//!
//! Guards to mutate, in `src/session.rs`: `keep_actor`, the write (the resumed cases are refused);
//! `kept_actor`, the read from the env file (the older session is refused); in `act_as_kept`
//! with `src/change.rs` `named_actor`, the environment read first and no kept name read when it
//! names one (both mutated, the renamed session stays the old name); in `src/hook_poll.rs` `start`,
//! the name given to the child (the talk is never told).

use crate::support;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde_json::{Value, json};
use support::{TestBox, unchecked_files};

const LOGIN: (&str, &str) = ("USER", "box-login");

fn run_hook(
    test_box: &TestBox,
    dir: &Path,
    event: &str,
    payload: &Value,
    envs: &[(&str, &str)],
) -> (Option<i32>, String) {
    let mut command = test_box.bay_process(dir);
    command.envs(envs.iter().copied());
    let mut child = command
        .args(["hook", event])
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
    let out = child.wait_with_output().unwrap();
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// `SessionStart` of session `id`, in the product's env file for it.
fn session_start(test_box: &TestBox, repo: &Path, id: &str, envs: &[(&str, &str)]) -> PathBuf {
    let env_file = test_box.path().join(format!("{id}.sh"));
    let file = env_file.to_str().unwrap();
    let mut with_file = vec![("CLAUDE_ENV_FILE", file)];
    with_file.extend_from_slice(envs);
    let payload = json!({"session_id": id, "hook_event_name": "SessionStart", "cwd": repo});
    let (code, stderr) = run_hook(test_box, repo, "session.start", &payload, &with_file);
    assert_eq!(code, Some(0), "{stderr}");
    env_file
}

/// A write in `view` by session `id`, as the product's hook process runs it: only the user name.
fn write_in(test_box: &TestBox, view: &Path, id: &str) -> (Option<i32>, String) {
    let payload = json!({
        "session_id": id,
        "hook_event_name": "PreToolUse",
        "tool_name": "Write",
        "tool_input": {"file_path": view.join("src/a.rs"), "content": "x"},
        "cwd": view,
    });
    run_hook(test_box, view, "tool.before", &payload, &[LOGIN])
}

/// A repository with a view of a change `seat-a` opened.
fn opened() -> (TestBox, PathBuf, PathBuf) {
    let test_box = TestBox::new();
    let repo = test_box.repo_any("repo");
    test_box.seed_with_check(&repo, &[("src/a.rs", "fn a() {}\n")], &unchecked_files());
    let started = test_box
        .bay(&repo)
        .env("BAY_ACTOR", "seat-a")
        .args(["start", "CARD-1", "--paths", "src/"])
        .output()
        .unwrap();
    let started: Value = serde_json::from_slice(&started.stdout).unwrap();
    let view = PathBuf::from(started["detail"]["view"].as_str().unwrap());
    (test_box, repo, view)
}

#[test]
fn a_session_writes_in_its_view_when_its_hooks_have_no_name() {
    let (test_box, repo, view) = opened();
    session_start(&test_box, &repo, "s-a", &[("BAY_ACTOR", "seat-a"), LOGIN]);

    let (code, stderr) = write_in(&test_box, &view, "s-a");

    assert_eq!(code, Some(0), "{stderr}");
}

#[test]
fn a_resume_with_no_name_keeps_the_name_for_hook_and_shell() {
    let (test_box, repo, view) = opened();
    session_start(&test_box, &repo, "s-a", &[("BAY_ACTOR", "seat-a"), LOGIN]);

    let env_file = session_start(&test_box, &repo, "s-a", &[LOGIN]);

    let (code, stderr) = write_in(&test_box, &view, "s-a");
    assert_eq!(code, Some(0), "{stderr}");
    let exports = std::fs::read_to_string(env_file).unwrap();
    assert_eq!(
        exports.lines().rfind(|line| line.contains("BAY_ACTOR")),
        Some("export BAY_ACTOR='seat-a'"),
        "{exports}"
    );
}

#[test]
fn a_session_older_than_this_bay_gets_its_name_from_its_env_file() {
    let (test_box, repo, view) = opened();
    std::fs::write(
        test_box.path().join("s-old.sh"),
        "export BAY_SESSION='s-old'\nexport BAY_ACTOR='seat-a'\nexport BAY_SESSION='s-old'\n",
    )
    .unwrap();

    session_start(&test_box, &repo, "s-old", &[LOGIN]);

    let (code, stderr) = write_in(&test_box, &view, "s-old");
    assert_eq!(code, Some(0), "{stderr}");
}

#[test]
fn another_session_on_the_account_is_still_refused() {
    let (test_box, repo, view) = opened();
    session_start(&test_box, &repo, "s-a", &[("BAY_ACTOR", "seat-a"), LOGIN]);
    session_start(&test_box, &repo, "s-b", &[LOGIN]);

    let (code, stderr) = write_in(&test_box, &view, "s-b");

    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("VIEW_NOT_YOURS"), "{stderr}");
    assert!(stderr.contains("box-login"), "{stderr}");
}

#[test]
fn a_process_that_names_another_actor_decides() {
    let (test_box, repo, view) = opened();
    session_start(&test_box, &repo, "s-a", &[("BAY_ACTOR", "seat-a"), LOGIN]);

    session_start(&test_box, &repo, "s-a", &[("BAY_ACTOR", "seat-b"), LOGIN]);

    let (code, stderr) = write_in(&test_box, &view, "s-a");
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("seat-b"), "{stderr}");
}

#[test]
fn a_resumed_session_hears_the_talks_of_its_name() {
    let case = support::talk::hosted_talk();
    let named = [("BAY_ACTOR", "agent-b"), LOGIN];
    session_start(&case.test_box, &case.b, "s-b", &named);
    let follow_up = support::talk::from_now(3600);
    let (code, sent) = case.talk(
        "agent",
        &[
            "agent-b",
            "Read this",
            "--kind",
            "delivery",
            "--change",
            "abcd2345",
            "--follow-up",
            &follow_up,
        ],
    );
    assert_eq!(code, 0, "{sent}");
    let id = sent["detail"]["talk"].as_str().unwrap();

    // The hook's own poll accepts the talk, in a child of a process that names no actor.
    let step = json!({"session_id": "s-b", "hook_event_name": "PostToolBatch", "tool_calls": [], "cwd": case.b});
    let (code, told) = run_hook(&case.test_box, &case.b, "step.end", &step, &[LOGIN]);

    assert_eq!(code, Some(0), "{told}");
    assert!(
        told.contains(id),
        "never told of the talk to its name: {told}"
    );
    assert!(told.contains("arrived for agent-b"), "{told}");
}
