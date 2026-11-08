//! G622 (a fleet finding, 2026-10-31: a check's fix was named the culprit of the red it fixed).
//! A post run judges a landing with its first parent's check (G91). A landing that changes the
//! check itself, right above a pass, fails under the old check, and no run proves it the cause:
//! its red names no culprit (`suspect.judged_old`), and its `say` says the next landing on the
//! check's paths judges the new check. That landing passes, and the red clears.
//!
//! The post check `suite` fails a tree with `src/bad.rs` while its adapter file
//! `scripts/suite-mode` says `strict`. `good` passes; `both` adds `src/bad.rs` and makes the
//! mode `lenient` in one change, so its own run, under the strict mode, fails.
//!
//! Guards to mutate, in `src/red.rs`: `judged_old` always `false` (`both` is named culprit), or
//! always `true` (a landing that did not change the check is never named culprit).

use crate::support;

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use support::{TestBox, result_json};

/// A pre check `tests` that always passes, and a post check `suite` on `src/**` whose adapter
/// holds its mode file.
fn policy(revision: u32) -> String {
    format!(
        "revision = {revision}\n\n[[check]]\nid = \"tests\"\nenvironment = \"local\"\ncommand = \"scripts/bay-check\"\nadapter = [\"scripts/bay-check\"]\nexpected_minimum = 1\ntimeout_s = 60\n\n[[check]]\nid = \"suite\"\nenvironment = \"local\"\nstage = \"post\"\ncommand = \"scripts/bay-check\"\nadapter = [\"scripts/bay-check\", \"scripts/suite-mode\"]\nexpected_minimum = 1\ntimeout_s = 60\nwhen_paths = [\"src/**\"]\n"
    )
}

/// The adapter. `suite` fails a tree with `src/bad.rs` while the mode file of its own adapter
/// says `strict`; every other run passes.
const ADAPTER: &str = r#"#!/bin/sh
req=$(cat)
commit=$(printf '%s' "$req" | sed -n 's/.*"candidate":{"commit":"\([0-9a-f]*\)","tree":"\([0-9a-f]*\)"[,}].*/\1/p')
tree=$(printf '%s' "$req" | sed -n 's/.*"candidate":{"commit":"\([0-9a-f]*\)","tree":"\([0-9a-f]*\)"[,}].*/\2/p')
verdict=pass
case "$req" in
*'"plan":{"id":"suite"'*)
  if [ -f src/bad.rs ] && [ "$(cat "$BAY_ADAPTER_DIR/scripts/suite-mode")" = strict ]; then
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
}

impl Line {
    fn new() -> Self {
        let test_box = TestBox::new();
        let repo = test_box.repo_any("repo");
        let remote = test_box.path().join("repo-remote.git");
        let files = vec![
            ("scripts/bay-check".to_owned(), ADAPTER.to_owned()),
            ("scripts/suite-mode".to_owned(), "strict\n".to_owned()),
            (".bay/policy.toml".to_owned(), policy(1)),
            (
                ".bay/config.toml".to_owned(),
                "[runner]\npost = true\nbisect_post = false\n".to_owned(),
            ),
        ];
        test_box.seed_with_check(&repo, &[("src/a.rs", "fn a() {}\n")], &files);
        Self {
            test_box,
            repo,
            remote,
        }
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
        let mut ran = Vec::new();
        for _ in 0..3 {
            if !records(&self.test_box, &self.remote, "suite", &tip).is_empty() {
                return tip;
            }
            let (code, pass) = result_json(
                self.test_box
                    .bay(&self.repo)
                    .args(["runner", "run", "--once"]),
            );
            assert_eq!(code, 0, "{pass}");
            ran.push(pass);
        }
        panic!("no run of suite on {name}'s tip: {ran:?}");
    }

    /// `status`'s `detail.red`.
    fn red(&self) -> Value {
        self.test_box.bay_ok(&self.repo, &["status"])["detail"]["red"].clone()
    }
}

#[test]
fn a_landing_that_changed_its_check_is_judged_with_the_old_one_and_named_no_culprit() {
    // GIVEN `good` on src/, which passes `suite`.
    let line = Line::new();
    let good = line.land("good", &["src/"], &[("src/b.rs", "fn b() {}\n")], None);
    assert_eq!(
        records(&line.test_box, &line.remote, "suite", &good)[0]["verdict"],
        "pass"
    );

    // WHEN `both` adds src/bad.rs and makes the check's mode lenient in one change.
    let revised = policy(2);
    let both = line.land(
        "both",
        &["src/", ".bay/policy.toml", "scripts/suite-mode"],
        &[
            ("src/bad.rs", "x\n"),
            (".bay/policy.toml", revised.as_str()),
            ("scripts/suite-mode", "lenient\n"),
        ],
        None,
    );

    // THEN its own run judged it with the strict mode of `good` and failed, and its red names
    // no culprit: the pass sits right below it, but the run judged it with the old check.
    assert_eq!(
        records(&line.test_box, &line.remote, "suite", &both)[0]["verdict"],
        "product_failure"
    );
    let red = line.red();
    assert_eq!(red[0]["check"], "suite", "{red}");
    assert_eq!(red[0]["culprit"], Value::Null, "{red}");
    assert_eq!(
        red[0]["suspect"]["judged_old"]["merge"],
        both.as_str(),
        "{red}"
    );
    assert_eq!(
        red[0]["suspect"]["last_passed"]["merge"],
        good.as_str(),
        "{red}"
    );
    let said = red[0]["say"].as_str().unwrap();
    assert!(
        said.contains("changed the check itself")
            && said.contains("named no cause")
            && said.contains("--fixes suite")
            && !said.contains("broke it"),
        "{said}"
    );

    // AND the next landing on the check's paths runs the lenient mode, and its pass clears it.
    let next = line.land("next", &["src/"], &[("src/next.rs", "x\n")], Some("suite"));
    assert_eq!(
        records(&line.test_box, &line.remote, "suite", &next)[0]["verdict"],
        "pass"
    );
    assert_eq!(line.red(), json!([]));
}

#[test]
fn a_landing_that_did_not_change_its_check_is_still_named_its_culprit() {
    // GIVEN `good` on src/, which passes `suite`.
    let line = Line::new();
    let good = line.land("good", &["src/"], &[("src/b.rs", "fn b() {}\n")], None);

    // WHEN `bad` adds src/bad.rs and leaves the check as it is.
    let bad = line.land("bad", &["src/"], &[("src/bad.rs", "x\n")], None);

    // THEN the pass sits right below it, and the evidence names it the culprit.
    let red = line.red();
    assert_eq!(red[0]["culprit"]["merge"], bad.as_str(), "{red}");
    assert_eq!(red[0]["suspect"]["judged_old"], Value::Null, "{red}");
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
