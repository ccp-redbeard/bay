//! G612 (a fleet finding, 2026-10-31: after its other fetches had ended, a `bay status` accepted
//! its actor's talks in three round trips one after another, about 4 s of a 15 s read). A status
//! accepts talks beside its other work with the remote, and a `bay status --since` beside its
//! other fetches; the accept fetches the actor's own talks and the ones it sent in one fetch, and
//! lists what the remote holds beside that fetch. Since G615 a status asks the remote once
//! before it accepts (the accept reads the trunk that one sweep moved), then accepts beside the
//! receipts' sync, whose push of a chain the remote lacks is a round trip too. Every round trip
//! of the reader's clone here takes at least 0.4 s (a slow `upload-pack` and `receive-pack`), so
//! two spawns that ran at once overlap in the trace a test build writes when
//! `BAY_TEST_GIT_TRACE` names a file (G171), and two that ran in turn do not.
//!
//! Guards to mutate: in `src/status.rs` `settle_swept`, accept the talks after the receipts'
//! sync, and the sync's push overlaps no listing of the talks; in `src/talk/accept.rs`
//! `fetch_talks`, fetch the actor's own talks apart, and the accept fetches talks twice; in
//! `accept`, fetch before listing, and the fetch and the listing no longer overlap; in
//! `src/watch.rs` `run`, fetch the log and the lanes in turn, and `--since` overlaps nothing.

// The slow upload-pack is a shell script, so these run on Unix only.
#![cfg(unix)]

use crate::support;

use std::path::Path;

use serde_json::Value;
use support::plain_talk::Plain;
use support::result_json;

/// Each round trip from `dir` waits 0.4 s before the remote answers.
fn slow_remote(case: &Plain, dir: &Path) {
    for (pack, key) in [
        ("upload-pack", "remote.origin.uploadpack"),
        ("receive-pack", "remote.origin.receivepack"),
    ] {
        let script = case.test_box.path().join(format!("slow-{pack}"));
        std::fs::write(
            &script,
            format!("#!/bin/sh\nsleep 0.4\nexec git {pack} \"$@\"\n"),
        )
        .unwrap();
        let mut mode = std::fs::metadata(&script).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut mode, 0o755);
        std::fs::set_permissions(&script, mode).unwrap();
        case.test_box
            .git_ok(dir, &["config", key, script.to_str().unwrap()]);
    }
}

/// `agent` merged a change of its own while the remote was away (G17): its chain here is one
/// request ahead of the remote's, and the next status's receipts sync pushes it. The chain ref.
fn a_chain_ahead(case: &Plain) -> String {
    let (code, started) = case.bay("agent", &["start", "CARD-612", "--paths", "src/"]);
    assert_eq!(code, 0, "{started}");
    let name = format!("refs/bay/changes/{}", started["change"].as_str().unwrap());
    let view = std::path::PathBuf::from(started["detail"]["view"].as_str().unwrap());
    std::fs::write(view.join("src/new.rs"), "fn new() {}\n").unwrap();
    let away = case.remote.with_extension("away");
    std::fs::rename(&case.remote, &away).unwrap();
    let (code, queued) = result_json(
        case.test_box
            .bay(&view)
            .env("BAY_ACTOR", "agent")
            .env("BAY_SESSION", "s-agent")
            .arg("merge"),
    );
    std::fs::rename(&away, &case.remote).unwrap();
    assert_eq!(code, 0, "{queued}");
    assert_eq!(queued["waiting_for"], "remote", "{queued}");
    assert_ne!(
        case.remote_tip(&name),
        Some(case.test_box.git_ok(&case.a, &["rev-parse", &name])),
        "the request reached the remote"
    );
    name
}

/// `agent` has sent a talk to `agent-b` and `agent-b` one to `agent`: the id of the second.
fn talks_both_ways(case: &Plain) -> String {
    let (code, sent) = case.talk("agent", &["agent-b", "Pick the wire format"]);
    assert_eq!(code, 0, "{sent}");
    let (code, to_me) = case.talk("agent-b", &["agent", "Pick the log format"]);
    assert_eq!(code, 0, "{to_me}");
    to_me["detail"]["talk"].as_str().unwrap().to_owned()
}

/// One git spawn of bay's own process: its subcommand, its arguments, and when it ran (from,
/// to) in microseconds since the Unix epoch.
struct Spawn {
    sub: String,
    args: Vec<String>,
    from: u64,
    to: u64,
}

impl Spawn {
    fn names(&self, part: &str) -> bool {
        self.args.iter().any(|arg| arg.contains(part))
    }

    fn overlaps(&self, other: &Self) -> bool {
        self.from < other.to && other.from < self.to
    }
}

/// `bay <args>` as `agent` on its clone: the result, and bay's own git spawns.
fn traced(case: &Plain, args: &[&str], name: &str) -> (Value, Vec<Spawn>) {
    let trace = case.test_box.path().join(format!("{name}-trace.jsonl"));
    let (code, result) = result_json(
        case.test_box
            .bay(&case.a)
            .env("BAY_ACTOR", "agent")
            .env("BAY_SESSION", "s-agent")
            .env("BAY_TEST_GIT_TRACE", &trace)
            .args(args),
    );
    assert_eq!(code, 0, "{result}");
    let text = std::fs::read_to_string(&trace).expect("the trace was written");
    let lines: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|error| panic!("{error}: {line}")))
        .collect();
    let pid = lines.first().map(|line| line["pid"].clone());
    let spawns = lines
        .iter()
        .filter(|line| Some(&line["pid"]) == pid.as_ref())
        .map(|line| {
            let to = line["end"]
                .as_u64()
                .expect("the trace names each spawn's end");
            Spawn {
                sub: line["sub"].as_str().unwrap().to_owned(),
                args: line["args"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|arg| arg.as_str().unwrap().to_owned())
                    .collect(),
                from: to - line["us"].as_u64().unwrap(),
                to,
            }
        })
        .collect();
    (result, spawns)
}

/// The accept's one fetch and its listing, after checking that it fetched talks once and
/// listed them beside that fetch.
fn accept_round_trips(spawns: &[Spawn]) -> (&Spawn, &Spawn) {
    let fetches: Vec<&Spawn> = spawns
        .iter()
        .filter(|spawn| spawn.sub == "fetch" && spawn.names("refs/bay/talk/"))
        .collect();
    assert_eq!(
        fetches.len(),
        1,
        "the accept fetched talks {} times: {:?}",
        fetches.len(),
        fetches.iter().map(|spawn| &spawn.args).collect::<Vec<_>>()
    );
    let fetch = fetches[0];
    assert!(
        fetch.names("refs/bay/talk/agent/*") && fetch.names("refs/bay/talk/agent-b/"),
        "one fetch brings the actor's own talks and the ones it sent: {:?}",
        fetch.args
    );
    let listing = spawns
        .iter()
        .find(|spawn| spawn.sub == "ls-remote" && spawn.names("refs/bay/talk/agent/"))
        .expect("the accept listed the remote's talks");
    assert!(
        fetch.overlaps(listing),
        "the talks' fetch and listing ran in turn"
    );
    (fetch, listing)
}

/// The talks were accepted after the status's one sweep ended, and listed while the receipts'
/// sync pushed `chain`.
fn after_the_sweep_beside_the_sync(spawns: &[Spawn], fetch: &Spawn, listing: &Spawn, chain: &str) {
    let sweep = spawns
        .iter()
        .find(|spawn| spawn.sub == "fetch" && spawn.names("--dry-run"))
        .expect("the status swept the remote");
    assert!(
        sweep.to <= fetch.from,
        "the talks were fetched before the sweep moved the trunk they are judged at"
    );
    let push = spawns
        .iter()
        .find(|spawn| spawn.sub == "push" && spawn.names(chain))
        .expect("the receipts' sync pushed the chain ahead of the remote's");
    assert!(
        push.overlaps(listing),
        "the talks were accepted after the receipts' sync"
    );
}

/// Some fetch that is not the talks' ran while the talks were listed.
fn beside_other_fetches(spawns: &[Spawn], listing: &Spawn) {
    assert!(
        spawns.iter().any(|spawn| spawn.sub == "fetch"
            && !spawn.names("refs/bay/talk/")
            && spawn.overlaps(listing)),
        "the talks were accepted after every other fetch ended"
    );
}

fn acked(result: &Value, id: &str) {
    let accept = &result["detail"]["talk_accept"];
    assert_eq!(accept["state"], "accepted", "{accept}");
    assert!(
        accept["acked"]
            .as_array()
            .unwrap()
            .iter()
            .any(|acked| acked == id),
        "the talk to the actor was not acked: {accept}"
    );
}

#[test]
fn a_status_accepts_talks_in_two_round_trips_beside_its_receipts_sync() {
    let case = Plain::approvals();
    let id = talks_both_ways(&case);
    let chain = a_chain_ahead(&case);
    slow_remote(&case, &case.a);
    let (result, spawns) = traced(&case, &["status"], "status");
    acked(&result, &id);
    let (fetch, listing) = accept_round_trips(&spawns);
    after_the_sweep_beside_the_sync(&spawns, fetch, listing, &chain);
}

#[test]
fn a_status_since_accepts_talks_beside_its_other_fetches() {
    let case = Plain::approvals();
    let id = talks_both_ways(&case);
    slow_remote(&case, &case.a);
    let (result, spawns) = traced(&case, &["status", "--since"], "since");
    acked(&result, &id);
    let (_, listing) = accept_round_trips(&spawns);
    beside_other_fetches(&spawns, listing);
}
