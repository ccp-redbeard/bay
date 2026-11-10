//! G200. The git spawn budget of a bay command is published and held: `bay status` and
//! `bay start` on a small repository spawn at most a stated number of `git` processes, read
//! from the trace a test build writes when `BAY_TEST_GIT_TRACE` names a file (G171). Each
//! command lists its repository's config once per handle it opens, never one `git config`
//! per key, so its `config` spawns stay within a handful however many keys it reads. It reads
//! the remote's default branch once, however many times it opens its repository, and an agent
//! hook also asks once where its repository is and reads its trunk's tip once. An agent hook,
//! a status and a start read each ref once between the calls of their own that may move one.

use crate::support;

use assert_cmd::Command;
use std::collections::BTreeMap;

use serde_json::Value;
use support::{TestBox, result_json, unchecked_files};

/// The most `git` spawns of `bay status` on the seeded repository, with no change open.
const STATUS_SPAWNS: usize = 80;
/// The most `git` spawns of `bay start` there.
const START_SPAWNS: usize = 45;
/// The most `git config` spawns of either command: one listing per repository handle it opens
/// (the opening check, the command, the coordination cache it writes at the end).
const CONFIG_SPAWNS: usize = 3;

/// The subcommands one run of `cmd` spawned, by count, from bay's own process only.
fn spawns(test_box: &TestBox, name: &str, cmd: &mut Command) -> (BTreeMap<String, usize>, Value) {
    let trace = test_box.path().join(format!("{name}-trace.jsonl"));
    let (code, result) = result_json(cmd.env("BAY_TEST_GIT_TRACE", &trace));
    assert_eq!(code, 0, "{result}");
    let text = std::fs::read_to_string(&trace).expect("the trace was written");
    let lines: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|error| panic!("{error}: {line}")))
        .collect();
    let pid = lines.first().map(|line| line["pid"].clone());
    let mut counts = BTreeMap::new();
    for line in lines
        .iter()
        .filter(|line| Some(&line["pid"]) == pid.as_ref())
    {
        *counts
            .entry(line["sub"].as_str().unwrap_or_default().to_owned())
            .or_insert(0) += 1;
    }
    (counts, result)
}

fn assert_budget(command: &str, counts: &BTreeMap<String, usize>, most: usize) {
    let total: usize = counts.values().sum();
    let config = counts.get("config").copied().unwrap_or(0);
    assert!(
        total <= most,
        "bay {command} spawned {total} git processes, budget {most}: {counts:?}"
    );
    assert!(
        config <= CONFIG_SPAWNS,
        "bay {command} spawned {config} git config processes, budget {CONFIG_SPAWNS}: {counts:?}"
    );
}

fn seeded(test_box: &TestBox) -> std::path::PathBuf {
    let repo = test_box.repo_any("repo");
    test_box.seed_with_check(&repo, &[("src/a.rs", "fn a() {}\n")], &unchecked_files());
    repo
}

#[test]
fn a_status_stays_within_its_git_spawn_budget() {
    let test_box = TestBox::new();
    let repo = seeded(&test_box);
    let (counts, _) = spawns(&test_box, "status", test_box.bay(&repo).arg("status"));
    assert_budget("status", &counts, STATUS_SPAWNS);
}

#[test]
fn a_start_stays_within_its_git_spawn_budget() {
    let test_box = TestBox::new();
    let repo = seeded(&test_box);
    let (counts, result) = spawns(
        &test_box,
        "start",
        test_box
            .bay(&repo)
            .args(["start", "CARD-1", "--paths", "src/"]),
    );
    assert!(result["detail"]["view"].is_string(), "{result}");
    assert_budget("start", &counts, START_SPAWNS);
}

/// W5 (a 139 s `bay status` on a queue of 87 changes): status reads the executor logs once,
/// in one batch, however many live changes it lists, never every log once per change, and every
/// chain in one batch, never one read per change or per state. With two changes open here and
/// the merges made in another clone, its `cat-file` spawns are the same after four as after two.
#[test]
fn a_status_reads_the_executor_logs_once_however_many_changes_it_lists() {
    let test_box = TestBox::new();
    let repo = seeded(&test_box);
    for n in 1..=2 {
        test_box.bay_ok(
            &repo,
            &[
                "start",
                &format!("OPEN-{n}"),
                "--paths",
                &format!("open{n}/"),
            ],
        );
    }
    // The merges come from another clone, so this one's views stay the same while the logs grow.
    let other = test_box.clone_of(&test_box.path().join("repo-remote.git"), "other");
    let merge = |n: usize| {
        let started = test_box.bay_ok(&other, &["start", &format!("CARD-{n}"), "--paths", "src/"]);
        let view = std::path::PathBuf::from(started["detail"]["view"].as_str().unwrap());
        std::fs::write(view.join(format!("src/n{n}.rs")), "fn n() {}\n").unwrap();
        let (code, merged) = result_json(test_box.bay(&view).args(["merge", "--wait"]));
        assert_eq!(code, 0, "{merged}");
    };
    let cat_files = |name: &str| {
        let (counts, listed) = spawns(&test_box, name, test_box.bay(&repo).arg("status"));
        assert!(listed.to_string().contains("OPEN-2"), "{listed}");
        counts.get("cat-file").copied().unwrap_or(0)
    };
    merge(1);
    merge(2);
    let two = cat_files("two");
    merge(3);
    merge(4);
    let four = cat_files("four");
    assert!(
        four <= two,
        "bay status read more objects one by one as the logs grew: {two} cat-file spawns after two merges, {four} after four"
    );
}

/// W5: a command pushes only the local chains the remote lacks or is behind on, never a stale
/// copy of a chain the remote has moved on: that push could only be refused (14 such pushes in
/// one 139 s `bay status`).
#[test]
fn a_status_never_pushes_a_chain_the_remote_moved_past() {
    let test_box = TestBox::new();
    let repo = seeded(&test_box);
    let started = test_box.bay_ok(&repo, &["start", "CARD-1", "--paths", "src/"]);
    let view = std::path::PathBuf::from(started["detail"]["view"].as_str().unwrap());
    std::fs::write(view.join("src/n.rs"), "fn n() {}\n").unwrap();
    let remote = test_box.path().join("repo-remote.git");
    let other = test_box.clone_of(&remote, "other");
    test_box.git_ok(
        &other,
        &[
            "fetch",
            "-q",
            "origin",
            "+refs/bay/changes/*:refs/bay/changes/*",
        ],
    );
    let (code, merged) = result_json(test_box.bay(&view).args(["merge", "--wait"]));
    assert_eq!(code, 0, "{merged}");
    let (counts, _) = spawns(&test_box, "stale", test_box.bay(&other).arg("status"));
    assert_eq!(
        counts.get("push").copied().unwrap_or(0),
        0,
        "bay status pushed a stale chain: {counts:?}"
    );
}

/// A fleet finding (2026-10-26: 43 views, a named status took 5 minutes): `bay status <change>`
/// reads that change's own view and newest draft, never every view's chain state by state, so
/// its git spawns are the same with two other views open as with six.
#[test]
fn a_named_status_costs_the_same_however_many_other_views_are_open() {
    let test_box = TestBox::new();
    let repo = seeded(&test_box);
    let open = |n: usize| {
        test_box.bay_ok(
            &repo,
            &["start", &format!("VIEW-{n}"), "--paths", &format!("v{n}/")],
        )["change"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let named = open(0);
    let (one, two) = (open(1), open(2));
    let total = |name: &str| {
        let (counts, _) = spawns(
            &test_box,
            name,
            test_box.bay(&repo).args(["status", &named]),
        );
        counts.values().sum::<usize>()
    };
    let with_two = total("two");
    for n in 3..=6 {
        open(n);
    }
    let with_six = total("six");
    assert!(
        with_six <= with_two,
        "bay status {named} spawned more git processes as other views opened ({one}, {two}, …): {with_two} with two, {with_six} with six"
    );
}

/// The `git` calls `bay hook <event>` made itself, in a seeded repository, given the recorded
/// payload `fixture` with this repository as its `cwd` and in place of `<repo>`. The repository
/// is named as the operating system names it, so every path the hook derives from the payload
/// spells it the same way.
fn hook_calls(event: &str, fixture: &str) -> Vec<Value> {
    use std::io::Write as _;
    use std::process::Stdio;

    let test_box = TestBox::new();
    let repo = std::fs::canonicalize(seeded(&test_box)).unwrap();
    // Windows puts `\\?\` before a canonical path, which no agent product sends.
    let repo = std::path::PathBuf::from(repo.to_str().unwrap().trim_start_matches(r"\\?\"));
    let recorded = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/claude-code-2.1.302")
        .join(fixture);
    let slashed = repo.to_str().unwrap().replace('\\', "/");
    let text = std::fs::read_to_string(recorded)
        .unwrap()
        .replace("<repo>", &slashed);
    let mut payload: Value = serde_json::from_str(&text).unwrap();
    payload["session_id"] = serde_json::json!("s-g200");
    payload["cwd"] = serde_json::json!(repo);
    let trace = test_box.path().join("hook-trace.jsonl");
    let mut child = test_box
        .bay_process(&repo)
        .args(["hook", event])
        .env("BAY_TEST_GIT_TRACE", &trace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let pid = child.id();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    std::fs::read_to_string(&trace)
        .expect("the trace was written")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|line| line["pid"] == pid)
        .collect()
}

/// The calls among `calls` with an argument that `matches`.
fn with_arg(calls: &[Value], matches: impl Fn(&str) -> bool) -> Vec<&Value> {
    calls
        .iter()
        .filter(|call| {
            call["args"]
                .as_array()
                .is_some_and(|args| args.iter().filter_map(Value::as_str).any(&matches))
        })
        .collect()
}

/// A Windows trace (a git spawn there costs tens of milliseconds): one hook opened its
/// repository five times and read the remote's default branch, `refs/remotes/origin/HEAD`, at
/// each. A process reads it once, and again only once a ref may have moved: after a call of
/// its own that may move one, or after a hook's wait for its poll. The hook at a turn's end
/// starts no poll.
///
/// Guard to mutate: `src/git.rs` `Git::remote_head`: read on every call.
#[test]
fn a_hook_reads_the_remotes_default_branch_once() {
    let calls = hook_calls("turn.end", "09-Stop.json");
    let reads = with_arg(&calls, |arg| arg == "refs/remotes/origin/HEAD");
    assert_eq!(
        reads.len(),
        1,
        "the hook read the remote's default branch {} times: {reads:#?}",
        reads.len()
    );
}

/// The same trace: the hook before a tool call made 14 `git` calls. Six read the tip of the
/// trunk, one at each open of the repository, and three asked where the repository is, once
/// for each spelling of its folder (`.`, the payload's `cwd`, the written file's folder). The
/// hook runs before every tool call, so half its run was reads it had just made. A hook reads
/// the tip once and asks once.
///
/// Guards to mutate: `src/main.rs` `keeps_ref_reads`: false for a hook; `src/git.rs`
/// `Git::common_dir`: key by the path as given.
#[test]
fn a_tool_hook_reads_its_trunk_once_and_asks_once_where_its_repository_is() {
    let calls = hook_calls("tool.before", "03-PreToolUse.json");
    let tips = with_arg(&calls, |arg| {
        arg.starts_with("refs/remotes/origin/main^{commit}")
    });
    assert_eq!(
        tips.len(),
        1,
        "the hook read the trunk's tip {} times: {tips:#?}",
        tips.len()
    );
    let asked = with_arg(&calls, |arg| arg == "--git-common-dir");
    assert_eq!(
        asked.len(),
        1,
        "the hook asked {} times where its repository is: {asked:#?}",
        asked.len()
    );
}

/// The calls of bay's own process, in the order they started, from the trace `cmd` writes.
fn traced(test_box: &TestBox, name: &str, cmd: &mut Command) -> (Vec<Value>, Value) {
    let trace = test_box.path().join(format!("{name}-trace.jsonl"));
    let (code, result) = result_json(cmd.env("BAY_TEST_GIT_TRACE", &trace));
    assert_eq!(code, 0, "{result}");
    let lines: Vec<Value> = std::fs::read_to_string(&trace)
        .expect("the trace was written")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let pid = lines.first().map(|line| line["pid"].clone());
    let start = |line: &Value| line["end"].as_u64().unwrap() - line["us"].as_u64().unwrap();
    let mut own: Vec<Value> = lines
        .into_iter()
        .filter(|line| Some(&line["pid"]) == pid.as_ref())
        .collect();
    own.sort_by_key(start);
    (own, result)
}

/// The ref reads among `calls` that read again what was read with no call between that may
/// move a ref: the one form of each read a command keeps (`rev-parse --verify -q <ref>^{object}`
/// and the `for-each-ref` listing of each ref's object). A call that only reads, or writes
/// objects, the index or config, moves no ref; any other may.
fn rereads(calls: &[Value]) -> Vec<String> {
    const READS: &[&str] = &[
        "--version",
        "cat-file",
        "check-attr",
        "cherry",
        "commit-tree",
        "config",
        "diff",
        "diff-tree",
        "for-each-ref",
        "hash-object",
        "log",
        "ls-files",
        "ls-remote",
        "ls-tree",
        "merge-base",
        "merge-tree",
        "mktree",
        "patch-id",
        "read-tree",
        "rev-list",
        "rev-parse",
        "show",
        "show-ref",
        "status",
        "update-index",
        "write-tree",
    ];
    let mut seen = std::collections::BTreeSet::new();
    let mut again = Vec::new();
    for call in calls {
        let args: Vec<&str> = call["args"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        let sub = call["sub"].as_str().unwrap_or_default();
        let reads = READS.contains(&sub)
            || (sub == "remote" && args.contains(&"get-url"))
            || (sub == "fetch" && args.contains(&"--dry-run"))
            || (sub == "symbolic-ref" && args.iter().filter(|a| !a.starts_with('-')).count() <= 3);
        if !reads {
            seen.clear();
            continue;
        }
        let kept = (sub == "rev-parse"
            && args.windows(3).any(|w| w == ["--verify", "-q", w[2]])
            && args.last().is_some_and(|last| last.ends_with("^{object}")))
            || (sub == "for-each-ref" && args.contains(&"--format=%(objectname) %(refname)"));
        if kept && !seen.insert(args.join(" ")) {
            again.push(args.join(" "));
        }
    }
    again
}

/// A Windows trace: a second `bay status`, served by the sweep the first one made, read the key
/// registry six times and listed the change chains four times; a status in a view read the
/// registry eight times. Nothing it does between moves a ref. It reads each ref once.
///
/// Guard to mutate: `src/main.rs` `keeps_ref_reads`: false for a status.
#[test]
fn a_status_reads_each_ref_once_between_moves() {
    let test_box = TestBox::new();
    let repo = seeded(&test_box);
    // The second read is served by the first one's sweep, however slow the machine.
    let status = || {
        let mut cmd = test_box.bay(&repo);
        cmd.arg("status").env("BAY_TEST_SWEPT_S", "600");
        cmd
    };
    let (code, first) = result_json(&mut status());
    assert_eq!(code, 0, "{first}");
    let (calls, _) = traced(&test_box, "status", &mut status());
    assert!(
        calls.iter().all(|call| call["sub"] != "fetch"),
        "the second status fetched; the first one's sweep should serve it: {calls:#?}"
    );
    let again = rereads(&calls);
    assert!(again.is_empty(), "the status read again: {again:#?}");
}

/// The same trace: `bay start` read the trunk's tip four times and the registry twice. Between
/// its fetches and its writes, it reads each ref once.
///
/// Guard to mutate: `src/main.rs` `keeps_ref_reads`: false for a start.
#[test]
fn a_start_reads_each_ref_once_between_moves() {
    let test_box = TestBox::new();
    let repo = seeded(&test_box);
    let (calls, result) = traced(
        &test_box,
        "start",
        test_box
            .bay(&repo)
            .args(["start", "CARD-1", "--paths", "src/"]),
    );
    assert!(result["detail"]["view"].is_string(), "{result}");
    let again = rereads(&calls);
    assert!(again.is_empty(), "the start read again: {again:#?}");
}
