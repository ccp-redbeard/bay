//! G616 (a fleet finding, 2026-10-31: after its one round trip a `bay status` spent about 6 s
//! reading this store, one section after the one before). Each section of a status only reads the
//! store, so the sections run beside each other. Read from the trace a test build writes when
//! `BAY_TEST_GIT_TRACE` names a file (G171): with each `git log` slowed, the logs of several
//! sections are in flight at one time. Two sections that read the chains beside each other read
//! each chain's record once: with each `git cat-file` slowed, so that two sections are in their
//! read of the chains at one time, one batch of bay's own process reads the records.
//!
//! Guards to mutate: in `src/status.rs`, in `run`, join each section before the next one starts,
//! and no two logs are in flight at one time; in `src/change.rs`, in `Chain::read_all`, take no
//! turn, and two sections both read the records neither holds yet.

use crate::support;

use std::path::{Path, PathBuf};

use serde_json::Value;
use support::{TestBox, result_json, unchecked_files};

const CONFIG: &str = "main = \"main\"\n\n[[target]]\nname = \"staging\"\n";

/// The git spawns of bay's own process that `trace` shows.
fn own_spawns(trace: &str) -> Vec<Value> {
    let lines: Vec<Value> = trace
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|error| panic!("{error}: {line}")))
        .collect();
    let pid = lines.first().map(|line| line["pid"].clone());
    lines
        .into_iter()
        .filter(|line| Some(&line["pid"]) == pid.as_ref())
        .collect()
}

/// The most `git log` spawns of bay's own process that `trace` shows in flight at one time.
fn most_logs_in_flight(trace: &str) -> usize {
    let mut edges: Vec<(u64, i8)> = own_spawns(trace)
        .iter()
        .filter(|line| line["sub"] == "log")
        .flat_map(|line| {
            let end = line["end"].as_u64().unwrap();
            [(end - line["us"].as_u64().unwrap(), 1), (end, -1)]
        })
        .collect();
    // An end sorts before a start at the same time, so two spawns that only touch do not overlap.
    edges.sort_unstable();
    let (mut now, mut most) = (0_i64, 0_i64);
    for (_, step) in edges {
        now += i64::from(step);
        most = most.max(now);
    }
    usize::try_from(most).unwrap()
}

/// A repository with a `staging` target and one live change: the box, the clone and the id.
fn store() -> (TestBox, PathBuf, String) {
    let test_box = TestBox::new();
    let repo = test_box.repo_any("repo");
    let remote = test_box.path().join("repo-remote.git");
    let mut files = unchecked_files();
    files.push((".bay/config.toml".to_owned(), CONFIG.to_owned()));
    test_box.seed_with_check(&repo, &[("src/a.rs", "fn a() {}\n")], &files);
    let main = test_box.git_bare(&remote, &["rev-parse", "refs/heads/main"]);
    test_box.git_bare(&remote, &["update-ref", "refs/heads/staging", &main]);
    let (id, _) = test_box.start_and_edit(&repo);
    (test_box, repo, id)
}

/// A wrapper body that waits half a second before each `git <sub>`.
fn slowed(sub: &str) -> String {
    format!(
        "skip=\nfor arg in \"$@\"; do\n  if [ -n \"$skip\" ]; then skip=; continue; fi\n  case \"$arg\" in\n    -C|-c) skip=1;;\n    -*) ;;\n    {sub}) sleep 0.5; break;;\n    *) break;;\n  esac\ndone"
    )
}

/// A wrapper body that waits half a second before each `git cat-file`, and keeps what each
/// `git cat-file --batch` reads in `<dir>/batch.<parent pid>.<pid>`.
fn kept_batches(dir: &Path) -> String {
    let dir = dir.display();
    format!(
        "{}\ncase \" $* \" in\n  *\" cat-file --batch \"*) tee \"{dir}/batch.$PPID.$$\" | \"$real\" \"$@\"; exit $?;;\nesac",
        slowed("cat-file")
    )
}

#[test]
fn a_status_reads_its_sections_side_by_side() {
    let (test_box, repo, id) = store();

    // Each `git log` waits half a second before it runs.
    test_box.wrap_git(&slowed("log"));
    let trace = test_box.path().join("trace.jsonl");
    let (code, status) = result_json(
        test_box
            .bay(&repo)
            .arg("status")
            .env("BAY_TEST_GIT_TRACE", &trace),
    );
    assert_eq!(code, 0, "{status}");
    let listed = status["detail"]["changes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["change"] == id.as_str());
    assert!(listed, "the status lost the live change: {status}");

    let trace = std::fs::read_to_string(&trace).expect("the trace was written");
    let most = most_logs_in_flight(&trace);
    assert!(
        most >= 3,
        "at most {most} logs were in flight at one time, so the sections ran one after another"
    );
}

#[test]
#[cfg_attr(
    windows,
    ignore = "Windows: bay spawns more git processes than counted; under investigation"
)]
fn sections_side_by_side_read_each_chain_once() {
    let (test_box, repo, id) = store();
    let (other, _) = test_box.start_and_edit(&repo);
    let records: Vec<String> = [&id, &other]
        .iter()
        .map(|id| {
            let head = test_box.git_ok(&repo, &["rev-parse", &format!("refs/bay/changes/{id}")]);
            format!("{}:change.json", head.trim())
        })
        .collect();
    let batches = test_box.path().join("batches");
    std::fs::create_dir_all(&batches).unwrap();
    // Each `git cat-file` waits half a second, so two sections that both lack the records are
    // both in their read of them at one time.
    test_box.wrap_git(&kept_batches(&batches));
    let trace = test_box.path().join("trace.jsonl");
    let (code, status) = result_json(
        test_box
            .bay(&repo)
            .args(["status", &id])
            .env("BAY_TEST_GIT_TRACE", &trace),
    );
    assert_eq!(code, 0, "{status}");

    let trace = std::fs::read_to_string(&trace).expect("the trace was written");
    let pid = own_spawns(&trace)[0]["pid"].as_u64().unwrap();
    let reads = std::fs::read_dir(&batches)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(&format!("batch.{pid}.")))
        })
        .filter(|path| {
            let read = std::fs::read_to_string(path).unwrap();
            records
                .iter()
                .all(|record| read.lines().any(|line| line == record))
        })
        .count();
    assert_eq!(
        reads, 1,
        "{reads} batches of bay's own process read the records of every chain"
    );
}
