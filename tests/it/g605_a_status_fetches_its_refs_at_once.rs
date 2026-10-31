//! G605 (a fleet finding, 2026-10-31: one `bay status` on a busy repository fetched nine times
//! one after another, 16.5 s of its 42 s). After the trunk fetch, a status read fetches each set
//! of refs that no other of its fetches writes at the same time: the receipts' sync and the
//! chains, the fleet, the log and the evidence. Its answer is the one it gave before.
//!
//! Guard to mutate: `src/status.rs` `fetch_all`: join each fetch as soon as it is spawned, and
//! no two fetches are ever in flight at once.

// The case puts a shell wrapper for `git` on PATH, so it runs on Unix only.
#![cfg(unix)]

use crate::support;

use support::{TestBox, check_files, passing_result, result_json};

#[test]
fn a_status_fetches_its_independent_refs_at_once() {
    let test_box = TestBox::new();
    let repo = test_box.repo_any("repo");
    let check = check_files("# adapter v1", &passing_result(1), 1, 60);
    test_box.seed_with_check(&repo, &[("src/a.rs", "fn a() {}\n")], &check);
    let started = test_box.bay_ok(&repo, &["start", "CARD-6051", "--paths", "src/"]);
    let id = started["change"].as_str().unwrap().to_owned();

    // Each fetch and ls-remote marks itself in flight, waits a second, and logs how many are
    // in flight then.
    let flight = test_box.path().join("in-flight");
    std::fs::create_dir_all(&flight).unwrap();
    let seen = test_box.path().join("seen.log");
    test_box.wrap_git(&format!(
        "for arg in \"$@\"; do\n  case \"$arg\" in\n    fetch|ls-remote)\n      : > '{flight}/'$$\n      sleep 1\n      echo \"$arg $(ls '{flight}' | wc -l)\" >> '{seen}'\n      \"$real\" \"$@\"\n      code=$?\n      rm -f '{flight}/'$$\n      exit $code;;\n  esac\ndone",
        flight = flight.display(),
        seen = seen.display(),
    ));

    let (code, status) = result_json(test_box.bay(&repo).arg("status"));
    assert_eq!(code, 0, "{status}");
    let listed = status["detail"]["changes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["change"] == id.as_str());
    assert!(listed, "the status lost the live change: {status}");
    assert!(
        !status["freshness"]["fetched_at"]
            .as_str()
            .unwrap_or_default()
            .starts_with("not fetched"),
        "{status}"
    );

    let log = std::fs::read_to_string(&seen).unwrap_or_default();
    let most = log
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1)?.parse::<usize>().ok())
        .max()
        .unwrap_or(0);
    assert!(
        most >= 3,
        "the status fetched one set of refs after another: at most {most} in flight\n{log}"
    );
}
