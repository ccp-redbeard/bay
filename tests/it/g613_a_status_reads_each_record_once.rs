//! G613 (a fleet finding, 2026-10-31: one `bay status` read and parsed all 3,052 change
//! records five times, about 2 s). One process reads the record at each chain head once: a
//! later read of every chain lists the chains again and batches only the heads that moved. Read
//! from the trace a test build writes when `BAY_TEST_GIT_TRACE` names a file (G171): a read of
//! every chain is one `for-each-ref` of the change refs, and a `cat-file --batch` is one batch
//! of records. A status reads on several threads at once, so the spawn that starts after a read
//! may be another thread's: the test counts the batches of the whole status, not the spawn that
//! follows each read.
//!
//! Guard to mutate: in `src/change.rs` `held_records`, hand back no record kept, and every
//! read of every chain batches every record again.

use crate::support;

use serde_json::Value;
use support::plain_talk::Plain;
use support::result_json;

const LISTING: &str = "--format=%(objectname) %(refname)";

/// One git spawn of bay's own process: its subcommand and its arguments.
struct Spawn {
    sub: String,
    args: Vec<String>,
}

impl Spawn {
    fn lists_every_chain(&self) -> bool {
        self.sub == "for-each-ref"
            && self.args.iter().any(|arg| arg == LISTING)
            && self
                .args
                .last()
                .is_some_and(|arg| arg == "refs/bay/changes/")
    }

    fn batch(&self) -> bool {
        self.sub == "cat-file" && self.args.iter().any(|arg| arg == "--batch")
    }
}

/// `bay status` as `agent` on its clone: bay's own git spawns.
fn traced(case: &Plain) -> Vec<Spawn> {
    let trace = case.test_box.path().join("status-trace.jsonl");
    let (code, result) = result_json(
        case.test_box
            .bay(&case.a)
            .env("BAY_ACTOR", "agent")
            .env("BAY_SESSION", "s-agent")
            .env("BAY_TEST_GIT_TRACE", &trace)
            .arg("status"),
    );
    assert_eq!(code, 0, "{result}");
    let text = std::fs::read_to_string(&trace).expect("the trace was written");
    let lines: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|error| panic!("{error}: {line}")))
        .collect();
    let pid = lines.first().map(|line| line["pid"].clone());
    lines
        .iter()
        .filter(|line| Some(&line["pid"]) == pid.as_ref())
        .map(|line| Spawn {
            sub: line["sub"].as_str().unwrap().to_owned(),
            args: line["args"]
                .as_array()
                .unwrap()
                .iter()
                .map(|arg| arg.as_str().unwrap().to_owned())
                .collect(),
        })
        .collect()
}

#[test]
fn a_status_batches_each_chain_record_once() {
    let case = Plain::approvals();
    for (actor, card) in [
        ("agent", "CARD-1"),
        ("agent", "CARD-2"),
        ("agent-b", "CARD-3"),
    ] {
        let (code, started) = case.bay(actor, &["start", card, "--paths", "src/"]);
        assert_eq!(code, 0, "{started}");
    }
    let spawns = traced(&case);
    let reads = spawns
        .iter()
        .filter(|spawn| spawn.lists_every_chain())
        .count();
    let batches = spawns.iter().filter(|spawn| spawn.batch()).count();
    assert!(reads >= 3, "a status read every chain {reads} times");
    assert!(
        batches <= 2,
        "{batches} batches of records for {reads} reads of every chain"
    );
}
