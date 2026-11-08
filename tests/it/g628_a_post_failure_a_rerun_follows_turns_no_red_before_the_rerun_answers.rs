//! G628 (a fleet finding, 2026-11-02: a post check's first failure was kept, judged red and its
//! red told to a merge three minutes before its rerun passed). A post check that fails with a
//! rerun to follow (`retry_on_failure`) keeps that failure's record at once, while the runner
//! pushes the run's notes only once its pass ends. The record says a rerun follows and when the
//! last rerun's record is due (`rerun`): until then a red judgement reads the check as not yet
//! answered, so no red opens on the first failure alone. The rerun's pass makes the check flaky,
//! and its failure opens the red. A rerun that never answers (its runner died) opens the red once
//! its record is due.
//!
//! The post check `suite` retries once. While a run's `G628_UNTIL` is set, its first run fails, and
//! its rerun waits until that Unix time and answers `G628_VERDICT` (the sandbox hides every file
//! outside the candidate, so the times and the verdict come in the check's declared env).
//!
//! Guards to mutate: in `src/check.rs`, no `stamp_rerun` (each case's red opens while the rerun
//! waits); in `src/evidence.rs`, `answer` reading no due time (the same) or never past it (the
//! third case's red never opens); in `src/red.rs`, no second read of a landing whose rerun is due
//! (`moved.trees`: the third case again). A rerun that answers nothing ends the wait (g580).
//! `Last::stands`, which keeps a runner's judgement in its process, has its own unit test.

use crate::support;

use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use support::{TestBox, result_json};

/// A pre check `tests` that always passes, and a post check `suite` on `src/**` that retries
/// once.
const POLICY: &str = "revision = 1\n\n[[check]]\nid = \"tests\"\nenvironment = \"local\"\ncommand = \"scripts/bay-check\"\nadapter = [\"scripts/bay-check\"]\nexpected_minimum = 1\ntimeout_s = 60\n\n[[check]]\nid = \"suite\"\nenvironment = \"local\"\nstage = \"post\"\ncommand = \"scripts/bay-check\"\nadapter = [\"scripts/bay-check\"]\nexpected_minimum = 1\ntimeout_s = 60\nretry_on_failure = 1\nenv = [\"G628_UNTIL\", \"G628_VERDICT\"]\nwhen_paths = [\"src/**\"]\n";

/// The adapter. While `G628_UNTIL` is set, `suite` fails a first run, and its rerun waits until
/// that Unix time, then answers `G628_VERDICT`. Every other run passes.
const ADAPTER: &str = r#"#!/bin/sh
req=$(cat)
commit=$(printf '%s' "$req" | sed -n 's/.*"candidate":{"commit":"\([0-9a-f]*\)","tree":"\([0-9a-f]*\)"[,}].*/\1/p')
tree=$(printf '%s' "$req" | sed -n 's/.*"candidate":{"commit":"\([0-9a-f]*\)","tree":"\([0-9a-f]*\)"[,}].*/\2/p')
verdict=pass
case "$req" in
*'"plan":{"id":"suite"'*)
  if [ -n "$G628_UNTIL" ]; then
    case "$req" in
    *'"retry":'[1-9]*)
      while [ "$(date +%s)" -lt "$G628_UNTIL" ]; do
        sleep 0.1
      done
      verdict=$G628_VERDICT
      ;;
    *) verdict=product_failure ;;
    esac
  fi
  ;;
esac
printf '{"schema":"bay.check.result/1","exercised":{"commit":"%s","tree":"%s"},"plan":{"selected":1,"completed":1},"checks":[{"id":"t1","verdict":"%s"}]}' "$commit" "$tree" "$verdict"
"#;

/// The `suite` records the git directory `git_dir` holds about `tip`, in note order.
fn records(test_box: &TestBox, git_dir: &Path, tip: &str) -> Vec<Value> {
    let refs = test_box.git_bare(
        git_dir,
        &[
            "for-each-ref",
            "--format=%(refname)",
            "refs/notes/bay/evidence/",
        ],
    );
    let mut out = Vec::new();
    for notes_ref in refs.lines() {
        let blobs = test_box.git_bare(
            git_dir,
            &["ls-tree", "-r", "--format=%(objectname)", notes_ref],
        );
        for blob in blobs.lines() {
            let text = test_box.git_bare(git_dir, &["cat-file", "blob", blob]);
            let records: Vec<Value> = serde_json::from_str(&text).unwrap();
            out.extend(records.into_iter().filter(|record| {
                record["check"]["id"] == "suite" && record["subject"]["commit"] == tip
            }));
        }
    }
    out
}

fn verdicts(records: &[Value]) -> Vec<Value> {
    records
        .iter()
        .map(|record| record["verdict"].clone())
        .collect()
}

struct Line {
    test_box: TestBox,
    repo: PathBuf,
    remote: PathBuf,
    /// The repository's own git directory, where a runner keeps its records before it pushes.
    git_dir: PathBuf,
}

/// How long a rerun waits: time for a status to read the first failure while it waits.
const HOLD_S: u64 = 30;

impl Line {
    fn new() -> Self {
        let test_box = TestBox::new();
        let repo = test_box.repo_any("repo");
        let remote = test_box.path().join("repo-remote.git");
        let files = vec![
            ("scripts/bay-check".to_owned(), ADAPTER.to_owned()),
            (".bay/policy.toml".to_owned(), POLICY.to_owned()),
            (
                ".bay/config.toml".to_owned(),
                "[runner]\npost = true\nbisect_post = false\n".to_owned(),
            ),
        ];
        test_box.seed_with_check(&repo, &[("src/a.rs", "fn a() {}\n")], &files);
        let git_dir = PathBuf::from(test_box.git_ok(&repo, &["rev-parse", "--absolute-git-dir"]));
        Self {
            test_box,
            repo,
            remote,
            git_dir,
        }
    }

    /// Lands `name` on `src` and returns main's new tip. `suite` has not run on it yet.
    fn land(&self, name: &str) -> String {
        let started = self
            .test_box
            .bay_ok(&self.repo, &["start", name, "--paths", "src"]);
        let id = started["change"].as_str().unwrap().to_owned();
        let view = PathBuf::from(started["detail"]["view"].as_str().unwrap());
        std::fs::write(view.join(format!("src/{name}.rs")), "x\n").unwrap();
        let (code, merged) = result_json(self.test_box.bay(&view).args(["merge", "--wait"]));
        assert_eq!(code, 0, "{merged}");
        assert!(
            self.test_box.trunk_change_ids(&self.remote).contains(&id),
            "{name} lands: {merged}"
        );
        let tip = self
            .test_box
            .git_bare(&self.remote, &["rev-parse", "main"])
            .trim()
            .to_owned();
        assert_eq!(
            records(&self.test_box, &self.remote, &tip),
            Vec::<Value>::new()
        );
        tip
    }

    /// A runner pass in the background whose `suite` rerun on `tip` answers `verdict` after
    /// [`HOLD_S`], once its first run failed: that run's record is kept here, not yet pushed.
    fn waiting_pass(&self, tip: &str, verdict: &str) -> Child {
        let until = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + HOLD_S;
        let mut child = self
            .test_box
            .bay_process(&self.repo)
            .env("G628_UNTIL", until.to_string())
            .env("G628_VERDICT", verdict)
            .args(["--json", "runner", "run", "--once"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let start = Instant::now();
        let kept = loop {
            let kept = records(&self.test_box, &self.git_dir, tip);
            if !kept.is_empty() {
                break kept;
            }
            assert!(
                child.try_wait().unwrap().is_none(),
                "the pass ended before suite ran: {}",
                support::queued::finished(child)
            );
            assert!(
                start.elapsed() < Duration::from_secs(120),
                "suite never ran"
            );
            std::thread::sleep(Duration::from_millis(200));
        };
        assert_eq!(verdicts(&kept), [json!("product_failure")], "{kept:?}");
        assert_eq!(kept[0]["rerun"]["left"], 1, "{}", kept[0]);
        assert!(kept[0]["rerun"]["until"].is_string(), "{}", kept[0]);
        assert_eq!(
            records(&self.test_box, &self.remote, tip),
            Vec::<Value>::new()
        );
        child
    }

    /// Asserts that `tip`'s rerun still waits: this box holds only the first run's record.
    fn still_waits(&self, tip: &str) {
        let kept = records(&self.test_box, &self.git_dir, tip);
        assert_eq!(
            verdicts(&kept),
            [json!("product_failure")],
            "the rerun answered before the status read: raise HOLD_S"
        );
    }

    /// `status`'s `detail.red`, read by a reader whose clock is `skew_s` seconds ahead.
    fn red(&self, skew_s: u64) -> Value {
        let (code, status) = result_json(
            self.test_box
                .bay(&self.repo)
                .env("BAY_TEST_RERUN_CLOCK_SKEW_S", skew_s.to_string())
                .args(["status"]),
        );
        assert_eq!(code, 0, "{status}");
        status["detail"]["red"].clone()
    }
}

#[test]
fn a_post_failure_a_rerun_follows_turns_no_red_before_the_rerun_passes() {
    // GIVEN `one` on src/, and a runner pass whose `suite` failed once and reruns.
    let line = Line::new();
    let tip = line.land("one");
    let pass = line.waiting_pass(&tip, "pass");

    // WHEN the rerun has not answered, THEN the kept failure opens no red.
    let red = line.red(0);
    line.still_waits(&tip);
    assert_eq!(red, json!([]), "{red}");

    // WHEN the rerun passes, THEN the check is flaky on the tip, and main stays green.
    let ran = support::queued::finished(pass);
    let pushed = records(&line.test_box, &line.remote, &tip);
    assert_eq!(
        verdicts(&pushed),
        [json!("product_failure"), json!("pass")],
        "{ran}"
    );
    assert_eq!(pushed[1]["rerun"], Value::Null, "{}", pushed[1]);
    let red = line.red(0);
    assert_eq!(red, json!([]), "{red}");
}

#[test]
fn a_rerun_that_fails_again_opens_the_red() {
    // GIVEN `one` on src/, and a runner pass whose `suite` failed once and reruns.
    let line = Line::new();
    let tip = line.land("one");
    let pass = line.waiting_pass(&tip, "product_failure");
    let red = line.red(0);
    line.still_waits(&tip);
    assert_eq!(red, json!([]), "{red}");

    // WHEN the rerun fails too, THEN the last record says no rerun follows, and suite is red.
    let ran = support::queued::finished(pass);
    let pushed = records(&line.test_box, &line.remote, &tip);
    assert_eq!(
        verdicts(&pushed),
        [json!("product_failure"), json!("product_failure")],
        "{ran}"
    );
    assert_eq!(pushed[1]["rerun"], Value::Null, "{}", pushed[1]);
    let red = line.red(0);
    assert_eq!(red[0]["check"], "suite", "{red}");
}

#[test]
fn a_rerun_that_never_answers_opens_the_red_once_its_record_is_due() {
    // GIVEN `one` on src/, a runner pass whose `suite` failed once and reruns, and a status
    // that read it then.
    let line = Line::new();
    let tip = line.land("one");
    let mut pass = line.waiting_pass(&tip, "pass");
    let red = line.red(0);
    line.still_waits(&tip);
    assert_eq!(red, json!([]), "{red}");

    // WHEN the runner dies mid-rerun.
    pass.kill().unwrap();
    pass.wait().unwrap();

    // THEN no red opens before the rerun's record is due: one rerun's bounds, 60 s and the
    // default 600 s to materialize, and 300 s to reach another box.
    let red = line.red(0);
    assert_eq!(red, json!([]), "{red}");
    let red = line.red(900);
    assert_eq!(red, json!([]), "{red}");

    // AND past it the kept failure answers, and suite is red.
    let red = line.red(1000);
    assert_eq!(red[0]["check"], "suite", "{red}");
}
