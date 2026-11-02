//! G627 (fleet findings, 2026-11-02: a canary merge and a card merge were each named the
//! culprit of a flaky test's failure). A post check with a footprint reads only its read set.
//! When the same check judged the pass and the first failure right above it, and the read set
//! holds the same bytes in both trees, the check answered two ways on one input: that merge is
//! no cause. Its red names no culprit (`suspect.same_read`), and its `say` says the check is
//! flaky or its environment failed. The next pass clears the red.
//!
//! The post check `suite` reads `src/` (its `paths`) and wakes on `src/**` and `notes/**`. It
//! fails a tree with `src/bad.rs`, and every run while the box sets `G627_SUITE=flaky`.
//!
//! With no footprint and `[runner] bisect_post`, a range of one landing whose culprit stands on
//! one record takes one rerun at the suspect (`crate::bisect`): a pass there makes it flaky and
//! clears the red, a second failure confirms it, and it is never run a third time.
//!
//! Guards to mutate: in `src/red.rs`, `same_read` always `false` (the notes merge is named
//! culprit), always `true` (a merge that changed the read set is never named culprit), or with
//! the read-set digests not compared (the same); in `src/bisect.rs`, `unconfirmed` always
//! `false` (no rerun: the flaky red stays) or with no record count (a third run).

// Each case runs shell scripts (the adapter), so these run on Unix only.
#![cfg(unix)]

use crate::support;

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use support::{TestBox, result_json};

/// A pre check `tests` that always passes, and a post check `suite` that wakes on `src/**` and
/// `notes/**`; with `footprint`, it reads only `src/`.
fn policy(footprint: bool) -> String {
    let read = if footprint {
        "footprint = true\npaths = [\"src/\"]\n"
    } else {
        ""
    };
    format!(
        "revision = 1\n\n[[check]]\nid = \"tests\"\nenvironment = \"local\"\ncommand = \"scripts/bay-check\"\nadapter = [\"scripts/bay-check\"]\nexpected_minimum = 1\ntimeout_s = 60\n\n[[check]]\nid = \"suite\"\nenvironment = \"local\"\nstage = \"post\"\ncommand = \"scripts/bay-check\"\nadapter = [\"scripts/bay-check\"]\nexpected_minimum = 1\ntimeout_s = 60\nenv = [\"G627_SUITE\"]\n{read}when_paths = [\"src/**\", \"notes/**\"]\n"
    )
}

/// The adapter. `suite` fails a tree with `src/bad.rs`, and every run while the box sets
/// `G627_SUITE=flaky`; every other run passes.
const ADAPTER: &str = r#"#!/bin/sh
req=$(cat)
commit=$(printf '%s' "$req" | sed -n 's/.*"candidate":{"commit":"\([0-9a-f]*\)","tree":"\([0-9a-f]*\)"[,}].*/\1/p')
tree=$(printf '%s' "$req" | sed -n 's/.*"candidate":{"commit":"\([0-9a-f]*\)","tree":"\([0-9a-f]*\)"[,}].*/\2/p')
verdict=pass
case "$req" in
*'"plan":{"id":"suite"'*)
  if [ -f src/bad.rs ] || [ "$G627_SUITE" = flaky ]; then
    verdict=product_failure
  fi
  ;;
esac
printf '{"schema":"bay.check.result/1","exercised":{"commit":"%s","tree":"%s"},"plan":{"selected":1,"completed":1},"checks":[{"id":"t1","verdict":"%s"}]}' "$commit" "$tree" "$verdict"
"#;

/// The records of post check `check` on the remote about `tip`.
fn records(test_box: &TestBox, remote: &Path, check: &str, tip: &str) -> Vec<Value> {
    let refs = test_box.git_bare(
        remote,
        &[
            "for-each-ref",
            "--format=%(refname)",
            "refs/notes/bay/evidence/",
        ],
    );
    let mut out = Vec::new();
    for notes_ref in refs.lines() {
        let blobs = test_box.git_bare(
            remote,
            &["ls-tree", "-r", "--format=%(objectname)", notes_ref],
        );
        for blob in blobs.lines() {
            let text = test_box.git_bare(remote, &["cat-file", "blob", blob]);
            let records: Vec<Value> = serde_json::from_str(&text).unwrap();
            out.extend(records.into_iter().filter(|record| {
                record["check"]["id"] == check && record["subject"]["commit"] == tip
            }));
        }
    }
    out
}

struct Line {
    test_box: TestBox,
    repo: PathBuf,
    remote: PathBuf,
    /// `G627_SUITE`, which every bay command that may run `suite` sets: while it is `flaky`,
    /// every run of `suite` fails, whatever it reads.
    suite: std::cell::Cell<&'static str>,
}

impl Line {
    /// A line whose `suite` has a footprint or not, with `[runner] bisect_post` or not.
    fn new(footprint: bool, bisect: bool) -> Self {
        let test_box = TestBox::new();
        let repo = test_box.repo_any("repo");
        let remote = test_box.path().join("repo-remote.git");
        let files = vec![
            ("scripts/bay-check".to_owned(), ADAPTER.to_owned()),
            (".bay/policy.toml".to_owned(), policy(footprint)),
            (
                ".bay/config.toml".to_owned(),
                format!("[runner]\npost = true\nbisect_post = {bisect}\n"),
            ),
        ];
        test_box.seed_with_check(
            &repo,
            &[
                ("src/a.rs", "fn a() {}\n"),
                ("notes/a.md", "a\n"),
                ("docs/a.md", "a\n"),
            ],
            &files,
        );
        Self {
            test_box,
            repo,
            remote,
            suite: std::cell::Cell::new("steady"),
        }
    }

    /// Makes every run of `suite` fail from now on (`true`), or pass again (`false`).
    fn flaky(&self, on: bool) {
        self.suite.set(if on { "flaky" } else { "steady" });
    }

    /// Lands change `name` on `paths`, writing `written` in its view (with `--fixes fixes` when
    /// it names one), runs passes until `suite` has a record on the new tip, and returns it.
    fn land(
        &self,
        name: &str,
        paths: &[&str],
        written: &[(&str, &str)],
        fixes: Option<&str>,
    ) -> String {
        let mut args = vec!["start", name, "--paths"];
        args.extend(paths);
        if let Some(fixes) = fixes {
            args.extend(["--fixes", fixes]);
        }
        let started = self.test_box.bay_ok(&self.repo, &args);
        let id = started["change"].as_str().unwrap().to_owned();
        let view = PathBuf::from(started["detail"]["view"].as_str().unwrap());
        for (path, text) in written {
            std::fs::write(view.join(path), text).unwrap();
        }
        let (code, merged) = result_json(
            self.test_box
                .bay(&view)
                .env("G627_SUITE", self.suite.get())
                .args(["merge", "--wait"]),
        );
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
        let mut ran = Vec::new();
        for _ in 0..3 {
            if !records(&self.test_box, &self.remote, "suite", &tip).is_empty() {
                return tip;
            }
            let (code, pass) = result_json(
                self.test_box
                    .bay(&self.repo)
                    .env("G627_SUITE", self.suite.get())
                    .args(["runner", "run", "--once"]),
            );
            assert_eq!(code, 0, "{pass}");
            ran.push(pass);
        }
        panic!("no run of suite on {name}'s tip: {ran:?}");
    }

    /// Runs up to `passes` runner passes, until `suite` has `records` records on `tip`, and
    /// returns their verdicts, in note order.
    fn until(&self, tip: &str, records: usize, passes: usize) -> Vec<Value> {
        for _ in 0..passes {
            if self::records(&self.test_box, &self.remote, "suite", tip).len() >= records {
                break;
            }
            let (code, pass) = result_json(
                self.test_box
                    .bay(&self.repo)
                    .env("G627_SUITE", self.suite.get())
                    .args(["runner", "run", "--once"]),
            );
            assert_eq!(code, 0, "{pass}");
        }
        self::records(&self.test_box, &self.remote, "suite", tip)
            .iter()
            .map(|record| record["verdict"].clone())
            .collect()
    }

    /// `status`'s `detail.red`.
    fn red(&self) -> Value {
        self.test_box.bay_ok(&self.repo, &["status"])["detail"]["red"].clone()
    }
}

#[test]
fn a_failure_on_the_bytes_the_pass_read_names_no_culprit() {
    // GIVEN `good` on src/, which passes `suite`.
    let line = Line::new(true, false);
    let good = line.land("good", &["src/"], &[("src/b.rs", "fn b() {}\n")], None);
    assert_eq!(
        records(&line.test_box, &line.remote, "suite", &good)[0]["verdict"],
        "pass"
    );

    // WHEN `suite` turns flaky, and `card` lands on notes/ only, outside what the check reads.
    line.flaky(true);
    let card = line.land("card", &["notes/"], &[("notes/card.md", "c\n")], None);

    // THEN its run failed, and its red names no culprit: the pass sits right below it, but the
    // same check read the same bytes in both trees.
    assert_eq!(
        records(&line.test_box, &line.remote, "suite", &card)[0]["verdict"],
        "product_failure"
    );
    let red = line.red();
    assert_eq!(red[0]["check"], "suite", "{red}");
    assert_eq!(red[0]["culprit"], Value::Null, "{red}");
    assert_eq!(
        red[0]["suspect"]["same_read"]["merge"],
        card.as_str(),
        "{red}"
    );
    assert_eq!(
        red[0]["suspect"]["last_passed"]["merge"],
        good.as_str(),
        "{red}"
    );
    let said = red[0]["say"].as_str().unwrap();
    assert!(
        said.contains("same files")
            && said.contains("flaky")
            && said.contains("no cause")
            && !said.contains("broke it"),
        "{said}"
    );

    // AND once the check is steady again, its run on the next tip passes and the red clears.
    // `next` lands off the check's paths: a change on them waits for main while it is red, and a
    // red check runs again on each new tip (G492).
    line.flaky(false);
    let next = line.land("next", &["docs/"], &[("docs/next.md", "n\n")], None);
    assert_eq!(
        records(&line.test_box, &line.remote, "suite", &next)[0]["verdict"],
        "pass"
    );
    assert_eq!(line.red(), json!([]));
}

#[test]
fn a_failure_on_bytes_the_pass_did_not_read_still_names_its_culprit() {
    // GIVEN `good` on src/, which passes `suite`.
    let line = Line::new(true, false);
    let good = line.land("good", &["src/"], &[("src/b.rs", "fn b() {}\n")], None);

    // WHEN `bad` adds src/bad.rs, inside what the check reads.
    let bad = line.land("bad", &["src/"], &[("src/bad.rs", "x\n")], None);

    // THEN the pass sits right below it, the check read other bytes, and the evidence names it
    // the culprit.
    let red = line.red();
    assert_eq!(red[0]["culprit"]["merge"], bad.as_str(), "{red}");
    assert_eq!(red[0]["suspect"]["same_read"], Value::Null, "{red}");
    assert_eq!(
        red[0]["suspect"]["last_passed"]["merge"],
        good.as_str(),
        "{red}"
    );
    assert!(
        red[0]["say"].as_str().unwrap().contains("broke it"),
        "{red}"
    );
}

#[test]
fn a_one_landing_culprit_on_one_record_is_run_again_and_a_pass_there_clears_the_red() {
    // GIVEN `suite` with no footprint, `[runner] bisect_post`, and `good`, which passes it.
    let line = Line::new(false, true);
    let good = line.land("good", &["src/"], &[("src/b.rs", "fn b() {}\n")], None);
    assert_eq!(line.until(&good, 1, 0), vec![json!("pass")]);

    // WHEN `suite` fails once, on `card`'s run.
    line.flaky(true);
    let card = line.land("card", &["notes/"], &[("notes/card.md", "c\n")], None);
    line.flaky(false);
    let red = line.red();
    assert_eq!(
        red[0]["culprit"]["merge"],
        card.as_str(),
        "one record: {red}"
    );

    // THEN a later pass runs the check at `card` once more; it passes, so `card` is flaky,
    // which decides nothing, and the pass below is main's newest answer: no red.
    let mut verdicts = line.until(&card, 2, 4);
    verdicts.sort_by_key(ToString::to_string);
    assert_eq!(verdicts, vec![json!("pass"), json!("product_failure")]);
    assert_eq!(line.red(), json!([]));
}

#[test]
fn a_one_landing_culprit_that_fails_again_is_confirmed_and_never_run_a_third_time() {
    // GIVEN `suite` with no footprint, `[runner] bisect_post`, and `good`, which passes it.
    let line = Line::new(false, true);
    let good = line.land("good", &["src/"], &[("src/b.rs", "fn b() {}\n")], None);

    // WHEN `bad` adds src/bad.rs, so every run of it fails.
    let bad = line.land("bad", &["src/"], &[("src/bad.rs", "x\n")], None);

    // THEN the rerun at `bad` fails too, the evidence names it the culprit, and later passes
    // never run it a third time.
    assert_eq!(
        line.until(&bad, 2, 4),
        vec![json!("product_failure"), json!("product_failure")]
    );
    assert_eq!(line.until(&bad, 3, 2).len(), 2);
    let red = line.red();
    assert_eq!(red[0]["culprit"]["merge"], bad.as_str(), "{red}");
    assert_eq!(
        red[0]["suspect"]["last_passed"]["merge"],
        good.as_str(),
        "{red}"
    );
}
