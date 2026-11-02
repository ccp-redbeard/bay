//! G631 (BACKLOG 214 and 216, fleet findings, 2026-11-02). A change's own run (`bay merge
//! --wait`) that this machine cannot finish leaves the operation to the runners at once and
//! writes no refusal:
//!
//! - main's policy does not load here (ftltlvo6: a bay older than the policy refused the change
//!   with `POLICY_INVALID`, a refusal no session was told of, and a runner merged it minutes
//!   later). With a live runner the run leaves the operation, unclaimed and in its place, and
//!   says to update bay. With no live runner it judges by the last valid policy as before (G400).
//! - the host refuses this machine's push for permission (cotffvzj: a client's run was refused
//!   with GH013 four times, retried until its retries ran out, and a runner merged it). The run
//!   gives the operation to the runners with no retry counted.
//!
//! Guards to mutate: in `src/publish.rs`, `policy_unread_here` returning `None` (the client
//! merges the change itself); the `refused_for_permission` branch of the publishing push (the
//! client retries the push until the operation is blocked).

#![cfg(unix)]

use crate::support;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use support::{TestBox, check_files, passing_result, result_json};

const POLICY: &str = ".bay/policy.toml";

/// A runner clone of `remote` named `name`.
fn runner(test_box: &TestBox, remote: &Path, name: &str) -> PathBuf {
    let clone = test_box.clone_of(remote, name);
    test_box.git_ok(&clone, &["config", "bay.runnerName", name]);
    test_box.git_ok(
        &clone,
        &["config", "bay.producerName", &format!("{name}-checks")],
    );
    clone
}

/// Makes the presence of the runner `exec` fresh: it makes a pass, and stops before it claims.
fn seen(test_box: &TestBox, exec: &Path) {
    let _ = test_box
        .bay_process(exec)
        .args(["--json", "runner", "run", "--once"])
        .env("BAY_TEST_PAUSE_AT", "before-claim")
        .env("BAY_TEST_PAUSE_FILE", test_box.path().join("never"))
        .env("BAY_TEST_PAUSE_TIMEOUT_S", "2")
        .output();
}

/// Runs `exec` until change `id` is on main.
fn merged_by(test_box: &TestBox, exec: &Path, remote: &Path, id: &str) {
    for _ in 0..3 {
        test_box.bay_ok(exec, &["runner", "run", "--once"]);
        if test_box.trunk_change_ids(remote).contains(&id.to_owned()) {
            return;
        }
    }
    panic!(
        "the runner did not merge {id}: {}",
        test_box.git_bare(
            remote,
            &["log", "--format=%s", &bay::change::change_ref(id)]
        )
    );
}

fn chain(test_box: &TestBox, remote: &Path, id: &str) -> String {
    test_box.git_bare(
        remote,
        &["log", "--format=%s", &bay::change::change_ref(id)],
    )
}

#[test]
fn a_run_that_cannot_read_mains_policy_leaves_the_change_to_a_live_runner() {
    let test_box = TestBox::new();
    let repo = test_box.repo_any("repo");
    let remote = test_box.path().join("repo-remote.git");
    let files = check_files("true", &passing_result(1), 1, 60);
    test_box.seed_with_check(&repo, &[("src/a.rs", "fn a() {}\n")], &files);
    let valid = files
        .iter()
        .find(|(path, _)| path == POLICY)
        .map(|(_, text)| text.clone())
        .unwrap();
    // Main's newest policy does not load here (pushed outside bay, as a newer bay's key would be
    // to an older bay).
    let outside = test_box.clone_of(&remote, "outside");
    std::fs::write(
        outside.join(POLICY),
        valid.replacen("revision = 1\n", "revision = 2\nunchecked = true\n", 1),
    )
    .unwrap();
    test_box.git_ok(&outside, &["commit", "-q", "-am", "policy"]);
    test_box.git_ok(&outside, &["push", "-q", "origin", "HEAD:main"]);

    let (id, view) = test_box.start_and_edit(&repo);
    let (code, queued) = result_json(test_box.bay(&view).arg("merge"));
    assert_eq!(code, 0, "{queued}");
    let exec = runner(&test_box, &remote, "exec");
    seen(&test_box, &exec);

    let (code, waited) = result_json(test_box.bay(&view).args(["merge", "--wait"]));
    assert_eq!(code, 0, "{waited}");
    assert_eq!(waited["decision"], "queued", "{waited}");
    assert_eq!(waited["detail"]["yielded"], true, "{waited}");
    assert!(
        waited["detail"]["recovery"]
            .as_str()
            .is_some_and(|text| text.contains("run bay update")),
        "{waited}"
    );
    assert!(
        waited["detail"]["why"]
            .as_str()
            .is_some_and(|why| why.contains("unchecked")),
        "the result names the rule this bay could not read: {waited}"
    );
    let told = chain(&test_box, &remote, &id);
    assert!(
        !told.contains("by local") && !told.contains("blocked"),
        "the client claimed or refused the operation: {told}"
    );

    merged_by(&test_box, &exec, &remote, &id);
}

#[test]
fn a_run_whose_push_the_host_refuses_for_permission_leaves_the_change_to_the_runners() {
    let test_box = TestBox::new();
    let repo = test_box.repo_any("repo");
    let remote = test_box.path().join("repo-remote.git");
    let files = check_files("true", &passing_result(1), 1, 60);
    test_box.seed_with_check(&repo, &[("src/a.rs", "fn a() {}\n")], &files);
    // The host lets no push from this client update main.
    let hook = remote.join("hooks/pre-receive");
    assert!(!hook.exists(), "the remote has a pre-receive hook already");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(
        &hook,
        "#!/bin/sh\nwhile read old new ref; do\n  if [ \"$ref\" = refs/heads/main ] && [ -n \"$CLIENT_MAY_NOT_PUSH\" ]; then\n    echo 'error: GH013: Repository rule violations found for refs/heads/main.' >&2\n    echo '- Cannot update this protected ref.' >&2\n    exit 1\n  fi\ndone\nexit 0\n",
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();

    let (id, view) = test_box.start_and_edit(&repo);
    let (code, waited) = result_json(
        test_box
            .bay(&view)
            .args(["merge", "--wait"])
            .env("CLIENT_MAY_NOT_PUSH", "1"),
    );
    assert_eq!(code, 0, "{waited}");
    assert_eq!(waited["decision"], "queued", "{waited}");
    assert_eq!(waited["detail"]["yielded"], true, "{waited}");
    assert!(
        waited["detail"]["host_said"]
            .as_str()
            .is_some_and(|said| said.contains("GH013")),
        "{waited}"
    );
    let told = chain(&test_box, &remote, &id);
    assert!(
        !told.contains("retry") && !told.contains("blocked"),
        "the client retried or refused the push: {told}"
    );
    assert_eq!(test_box.trunk_change_ids(&remote), Vec::<String>::new());

    let exec = runner(&test_box, &remote, "exec");
    merged_by(&test_box, &exec, &remote, &id);
}
