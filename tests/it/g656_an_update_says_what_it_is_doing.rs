//! G656 (owner, 2026-11-05: "it felt like a very long process, as there was no feedback on what
//! was going on"). `bay update` on a terminal shows the step it is on with the seconds it has
//! taken, on one line it clears at the end. Only a terminal sees it: stdout stays the one result,
//! and a command with no terminal, such as the background update, hears nothing. A test build asks
//! for the steps as plain lines, as it has no terminal.
//!
//! Guard to mutate, in `src/update.rs`: `step`, what it records (no step is said); `Ticker::start`,
//! the terminal test (a command with no terminal hears the steps).

// Each case runs shell scripts (the fixture's binaries), so these run on Unix only.
#![cfg(unix)]

use crate::release_fixture;

use release_fixture::Fixture;
use release_fixture::RetryBusy as _;

const NEWER: &str = "9.1.0";

fn update(fixture: &Fixture, env: &[(&str, &str)]) -> (serde_json::Value, String) {
    let mut cmd = fixture.bay(fixture.path());
    cmd.envs(env.iter().copied()).arg("update");
    let out = cmd.output_busy();
    let text = String::from_utf8(out.stdout).unwrap();
    let value = serde_json::from_str(&text).unwrap_or_else(|_| panic!("one result: {text}"));
    (value, String::from_utf8(out.stderr).unwrap())
}

#[test]
fn each_step_of_an_update_is_said_in_order() {
    let fixture = Fixture::new();
    fixture.release(NEWER, NEWER, None);
    fixture.list(&[(NEWER, false)]);

    let (result, stderr) = update(&fixture, &[("BAY_TEST_UPDATE_STEPS", "1")]);

    assert_eq!(result["detail"]["updated"], true, "{result}");
    let steps: Vec<&str> = stderr
        .lines()
        .filter_map(|line| line.strip_prefix("step: "))
        .collect();
    assert_eq!(
        steps,
        [
            "Looking for the newest release",
            "Checking the signature of bay 9.1.0",
            "Downloading bay 9.1.0",
            "Installing bay 9.1.0",
        ],
        "{stderr}"
    );
}

#[test]
fn a_command_with_no_terminal_hears_no_steps() {
    let fixture = Fixture::new();
    fixture.release(NEWER, NEWER, None);
    fixture.list(&[(NEWER, false)]);

    let (result, stderr) = update(&fixture, &[]);

    assert_eq!(result["detail"]["updated"], true, "{result}");
    assert!(!stderr.contains("Downloading"), "{stderr}");
}
