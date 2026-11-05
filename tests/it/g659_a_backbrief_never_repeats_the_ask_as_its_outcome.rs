//! G659 (a fleet finding, 2026-11-05: a reply to a rejected ask read as agreeing to it). A
//! backbrief with no `--outcome` wrote the talk's own ask as its outcome, so a reply that said
//! only a status appeared to restate what was asked. Now its outcome is the status word.
//!
//! Guard to mutate, in `src/talk/backbrief.rs`: the outcome's default (the talk's `what` again).

use crate::support;

use support::talk::{from_now, hosted_talk};

const ASK: &str = "Change the segment start";

fn reply(extra: &[&str]) -> serde_json::Value {
    let case = hosted_talk();
    let follow_up = from_now(3600);
    let (code, sent) = case.talk(
        "agent",
        &[
            "agent-b",
            ASK,
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
    let mut args = vec!["--backbrief", id.as_str(), "done"];
    args.extend_from_slice(extra);
    let (code, replied) = case.talk("agent-b", &args);
    assert_eq!(code, 0, "{replied}");
    let (code, shown) = case.talk("agent-b", &["--show", &id]);
    assert_eq!(code, 0, "{shown}");
    shown["detail"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .rfind(|entry| entry["kind"] == "backbrief")
        .unwrap_or_else(|| panic!("no backbrief: {shown}"))
        .clone()
}

#[test]
fn a_reply_with_no_outcome_does_not_restate_the_ask() {
    let last = reply(&[]);
    assert_eq!(last["outcome"], "done", "{last}");
}

#[test]
fn an_outcome_given_is_kept() {
    let last = reply(&["--outcome", "Kept the start as it is"]);
    assert_eq!(last["outcome"], "Kept the start as it is", "{last}");
}
