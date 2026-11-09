//! G632 (BACKLOG 218, a fleet finding, 2026-11-02: "`UserPromptSubmit` hook [... `bay hook
//! turn.start`] timed out after 30s — output discarded", too often). A session's hook polled its
//! talks and fetched the fleet's presence itself, and each git call had the network bound of its
//! own (180 s), so one slow remote held the hook past the product's timeout. Now:
//!
//! - a prompt's hook polls in a child (`bay live poll`) it waits for a short while, and exits 0
//!   with what it has; the talk the child accepts is told at a later hook;
//! - what stays in a hook (the lock a lockable edit takes, G336) ends its wait for the remote by
//!   the hook's budget, and is refused `REMOTE_UNREACHABLE` once the budget is spent: unknown is
//!   never free.
//!
//! Guards to mutate: in `src/hook_poll.rs`, the join waits for the child to end; in
//! `src/notice.rs`, `no_executor` fetches a stale presence record itself; in `src/git.rs`, a
//! call to the remote keeps its own bound whatever the deadline.

use crate::support;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use support::talk::{TalkCase, from_now, hosted_talk};
use support::{TestBox, UNCHECKED_POLICY};

/// How long the slow remote takes to answer each call.
const SLOW_S: u64 = 8;

/// Makes each call of `clone` to its remote wait [`SLOW_S`] seconds before the remote answers.
fn slow(test_box: &TestBox, clone: &Path) {
    test_box.git_ok(
        clone,
        &[
            "config",
            "remote.origin.uploadpack",
            &format!("sleep {SLOW_S}; git upload-pack"),
        ],
    );
}

/// `bay hook <event>` in `cwd` with `payload` and `env`: (exit, stdout and stderr, time taken).
fn hook(
    test_box: &TestBox,
    cwd: &Path,
    event: &str,
    payload: &Value,
    env: &[(&str, &str)],
) -> (Option<i32>, String, Duration) {
    let started = Instant::now();
    let mut command = test_box.bay_process(cwd);
    command
        .args(["hook", event])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in env {
        command.env(key, value);
    }
    let mut child = command.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    (
        output.status.code(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
        started.elapsed(),
    )
}

/// The lock of the poll a hook started in `clone`.
fn poll_lock(test_box: &TestBox, clone: &Path) -> PathBuf {
    let common = test_box.git_ok(
        clone,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    );
    PathBuf::from(common.trim())
        .join("bay")
        .join("hook-poll.lock")
}

/// Waits until no poll runs in `clone`.
fn poll_ended(test_box: &TestBox, clone: &Path) {
    let lock = poll_lock(test_box, clone);
    let until = Instant::now() + Duration::from_secs(SLOW_S * 12);
    while lock.exists() {
        assert!(
            Instant::now() < until,
            "the poll in {} did not end",
            clone.display()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// `bay hook turn.start` as `agent-b` on its clone, for session `s-b`, waiting at most 1 s for
/// its poll.
fn prompt(case: &TalkCase) -> (String, Duration) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/claude-code-2.1.302/02-UserPromptSubmit.json");
    let mut payload: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    payload["session_id"] = Value::from("s-b");
    payload["cwd"] = Value::from(case.b.display().to_string());
    let (code, said, took) = hook(
        &case.test_box,
        &case.b,
        "turn.start",
        &payload,
        &[("BAY_ACTOR", "agent-b"), ("BAY_TEST_POLL_JOIN_S", "1")],
    );
    assert_eq!(code, Some(0), "{said}");
    (said, took)
}

#[test]
fn a_prompt_is_not_held_by_a_slow_remote_and_its_talk_is_told_at_a_later_hook() {
    let case = hosted_talk();
    let follow_up = from_now(3600);
    let (code, sent) = case.talk(
        "agent",
        &[
            "agent-b",
            "Review the parser change",
            "--kind",
            "delivery",
            "--change",
            "abcd2345",
            "--follow-up",
            &follow_up,
        ],
    );
    assert_eq!(code, 0, "{sent}");
    let id = sent["detail"]["talk"].as_str().unwrap().to_owned();
    slow(&case.test_box, &case.b);

    let (first, took) = prompt(&case);
    assert!(
        took < Duration::from_secs(6),
        "the prompt waited {took:?} for a remote that answers in {SLOW_S} s: {first}"
    );
    assert!(!first.contains(&id), "{first}");

    // The poll goes on after the hook, and the next hook tells what it accepted.
    poll_ended(&case.test_box, &case.b);
    let (second, _) = prompt(&case);
    assert!(
        second.contains(&format!(
            "bay: talk {id} (delivery) from agent arrived for agent-b"
        )),
        "the talk the poll accepted was not told at the next hook: {second}"
    );
    poll_ended(&case.test_box, &case.b);
}

#[test]
fn a_lockable_edit_ends_its_wait_for_a_slow_remote_by_the_budget() {
    let test_box = TestBox::new();
    let first = test_box.repo_any("repo");
    test_box.seed_with_check(
        &first,
        &[("art/logo.bin", "logo\n"), ("src/a.rs", "fn a() {}\n")],
        &[
            (".bay/policy.toml".to_owned(), UNCHECKED_POLICY.to_owned()),
            (".gitattributes".to_owned(), "art/** lockable\n".to_owned()),
            (
                ".bay/config.toml".to_owned(),
                "[runner]\nlease_s = 30\nheartbeat_s = 1\n".to_owned(),
            ),
        ],
    );
    let remote = test_box.path().join("repo-remote.git");
    let second = test_box.clone_of(&remote, "second");
    slow(&test_box, &second);

    let payload = json!({
        "session_id": "s-second",
        "hook_event_name": "PreToolUse",
        "tool_name": "Write",
        "tool_input": {"file_path": second.join("art/logo.bin"), "content": "x"},
        "cwd": second,
    });
    let (code, said, took) = hook(
        &test_box,
        &second,
        "tool.before",
        &payload,
        &[("BAY_TEST_HOOK_BUDGET_S", "2")],
    );
    assert_eq!(code, Some(2), "{said}");
    assert!(said.contains("refused REMOTE_UNREACHABLE"), "{said}");
    assert!(
        took < Duration::from_secs(6),
        "the edit waited {took:?} for a remote that answers in {SLOW_S} s: {said}"
    );
}
