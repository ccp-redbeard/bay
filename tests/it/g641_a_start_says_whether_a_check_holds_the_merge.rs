//! G641: `bay start` says whether a check will hold this change's merge.
//!
//! A change no pre-merge check covers merges in the next push (G403), but an agent learned that
//! only after it submitted, and one stray path in its declared list cost it the fast way with
//! nothing to say why. Now a start states it in `detail.pre_merge`: `at_once`, and in `checks`
//! each pre-merge check a file under the declared paths can wake, with the declared path and an
//! example file. The statement reads the globs, so it holds for a file that does not exist yet,
//! and it never promises `at_once` where the merge would wait: a false "may wait" costs a look,
//! a false "at once" costs trust.
//!
//! Guards to mutate, in `src/policy.rs`: in `pre_merge`, a check of the post list added to what it
//! reads would name it (the post case); in `can_wake`, the `globs_match` on each example (an
//! excluded directory finds its check named); `holds_policy` returning `false` (the policy case),
//! and in `src/start.rs` `say_pre_merge`, the `after` test (the case of a change after another).

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::support;

use support::{TestBox, check_files, passing_result, result_json};

/// One check `tests`, its `when_paths` set as given.
fn seeded(test_box: &TestBox, when_paths: &str) -> PathBuf {
    let repo = test_box.repo_any("repo");
    let mut files = check_files("true", &passing_result(1), 1, 60);
    let policy = files
        .iter_mut()
        .find(|(path, _)| path == ".bay/policy.toml")
        .unwrap();
    policy.1 = policy.1.replacen(
        "timeout_s",
        &format!("when_paths = {when_paths}\ntimeout_s"),
        1,
    );
    test_box.seed_with_check(
        &repo,
        &[("src/a.rs", "fn a() {}\n"), ("docs/a.md", "a\n")],
        &files,
    );
    repo
}

fn start(test_box: &TestBox, repo: &Path, intent: &str, paths: &[&str]) -> Value {
    let mut args = vec!["start", intent, "--paths"];
    args.extend_from_slice(paths);
    test_box.bay_ok(repo, &args)
}

fn checks(started: &Value) -> Vec<String> {
    started["detail"]["pre_merge"]["checks"]
        .as_array()
        .unwrap_or_else(|| panic!("no pre_merge.checks: {started}"))
        .iter()
        .map(|row| row["check"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn a_change_outside_every_check_says_at_once_and_merges_so() {
    let test_box = TestBox::new();
    let repo = seeded(&test_box, r#"["src/**"]"#);

    let started = start(&test_box, &repo, "NOTE", &["docs/"]);

    assert_eq!(started["detail"]["pre_merge"]["at_once"], true, "{started}");
    assert_eq!(started["detail"]["pre_merge"]["checks"], json!([]));
    // What it promised is what the merge does: nothing ran, and it is on main.
    let view = PathBuf::from(started["detail"]["view"].as_str().unwrap());
    std::fs::write(view.join("docs/note.md"), "a note\n").unwrap();
    let (code, landed) = result_json(test_box.bay(&view).args(["merge", "--wait"]));
    assert_eq!(code, 0, "{landed}");
    assert_eq!(landed["decision"], "merged", "{landed}");
}

#[test]
fn a_path_a_check_can_read_names_the_check_the_path_and_an_example() {
    let test_box = TestBox::new();
    let repo = seeded(&test_box, r#"["src/**/*.rs"]"#);

    // `src/new/` has no file yet: the globs still say what a file there can wake.
    let started = start(&test_box, &repo, "CODE", &["docs/", "src/new/"]);

    let pre_merge = &started["detail"]["pre_merge"];
    assert_eq!(pre_merge["at_once"], false, "{started}");
    assert_eq!(
        pre_merge["checks"].as_array().unwrap().len(),
        1,
        "{started}"
    );
    let row = &pre_merge["checks"][0];
    assert_eq!(row["path"], "src/new/", "{started}");
    let example = row["example"].as_str().unwrap();
    assert!(
        example == "src/new/example.rs",
        "not a file the globs take: {started}"
    );
}

#[test]
fn a_glob_that_excludes_the_directory_wakes_nothing_there() {
    let test_box = TestBox::new();
    // As a repository says "every AGENTS.md, but not the ones under notes/".
    let repo = seeded(&test_box, r#"["**/AGENTS.md", "!notes/**"]"#);

    let notes = start(&test_box, &repo, "NOTES", &["notes/"]);
    let docs = start(&test_box, &repo, "DOCS", &["docs/"]);

    assert_eq!(notes["detail"]["pre_merge"]["at_once"], true, "{notes}");
    assert_eq!(docs["detail"]["pre_merge"]["at_once"], false, "{docs}");
    assert_eq!(
        docs["detail"]["pre_merge"]["checks"][0]["example"], "docs/AGENTS.md",
        "{docs}"
    );
}

#[test]
fn a_named_file_answers_for_itself_alone() {
    let test_box = TestBox::new();
    let repo = seeded(&test_box, r#"["**/AGENTS.md"]"#);

    // `docs/a.md` is a file on main, so no `docs/a.md/AGENTS.md` can exist.
    let started = start(&test_box, &repo, "ONE", &["docs/a.md"]);

    assert_eq!(started["detail"]["pre_merge"]["at_once"], true, "{started}");
}

#[test]
fn a_change_to_the_policy_never_says_at_once() {
    let test_box = TestBox::new();
    let repo = seeded(&test_box, r#"["src/**"]"#);

    let started = start(&test_box, &repo, "POLICY", &[".bay/"]);

    assert_eq!(
        started["detail"]["pre_merge"]["at_once"], false,
        "{started}"
    );
    assert_eq!(
        checks(&started),
        ["tests"],
        "a policy change wakes every check"
    );
}

#[test]
fn a_change_after_another_never_says_at_once() {
    let test_box = TestBox::new();
    let repo = seeded(&test_box, r#"["src/**"]"#);
    let first = start(&test_box, &repo, "FIRST", &["src/"]);
    let id = first["change"].as_str().unwrap();

    let started = test_box.bay_ok(
        &repo,
        &["start", "SECOND", "--paths", "docs/", "--after", id],
    );

    assert_eq!(
        started["detail"]["pre_merge"]["at_once"], false,
        "{started}"
    );
    assert_eq!(started["detail"]["pre_merge"]["checks"], json!([]));
}

#[test]
fn a_check_after_the_merge_holds_nothing() {
    let test_box = TestBox::new();
    let repo = test_box.repo_any("repo");
    let mut files = check_files("true", &passing_result(1), 1, 60);
    let policy = files
        .iter_mut()
        .find(|(path, _)| path == ".bay/policy.toml")
        .unwrap();
    policy.1 = policy
        .1
        .replacen("timeout_s", "when_paths = [\"src/**\"]\ntimeout_s", 1);
    policy.1.push_str(
        "\n[[check]]\nid = \"everything\"\nenvironment = \"local\"\nstage = \"post\"\ncommand = \"scripts/bay-check\"\nadapter = [\"scripts/bay-check\"]\nexpected_minimum = 1\ntimeout_s = 60\n",
    );
    test_box.seed_with_check(&repo, &[("docs/a.md", "a\n")], &files);

    let started = start(&test_box, &repo, "NOTE", &["docs/"]);

    assert_eq!(started["detail"]["pre_merge"]["at_once"], true, "{started}");
    assert_eq!(started["detail"]["pre_merge"]["checks"], json!([]));
}

#[test]
fn a_repository_without_checks_is_not_told() {
    let test_box = TestBox::new();
    let repo = test_box.repo_any("repo");
    test_box.seed_with_check(&repo, &[("docs/a.md", "a\n")], &support::unchecked_files());

    let started = start(&test_box, &repo, "NOTE", &["docs/"]);

    assert!(started["detail"].get("pre_merge").is_none(), "{started}");
}
