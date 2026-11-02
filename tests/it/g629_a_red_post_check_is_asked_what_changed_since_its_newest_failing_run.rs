//! G629 (a fleet finding, 2026-11-02: after its red, edge-py-fast ran 2,073 units on each new
//! tip, in 627-645 s). A red post check whose newest failing run was asked its failed units again
//! and answered each one is asked what changed since that run's landing, and runs its failed
//! units.
//!
//! G495 asks a red check what changed since its newest pass, so a suite that picks its tests by
//! changed paths runs the failing tests again. G540 then named the failed units in `rerun`. The
//! culprit's paths stayed in the range of every later tip, so each run picked the culprit's whole
//! selection again, though the failing run before it had already tested that range.
//!
//! Guards to mutate: in `src/bisect.rs` `tips`, the red case of `since` (since the newest pass
//! always: the third run tests t2 again); `failing`'s `answered_rerun` (true whenever the run was
//! asked units again: after the half run, the range is the half run's landing and t2 is not
//! tested again).

#![cfg(unix)]

use crate::support;

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use support::{TestBox, result_json};

/// A pre check `tests` that always passes, and a post check `suite` on `src/**`.
const POLICY: &str = "revision = 1\n\n[[check]]\nid = \"tests\"\nenvironment = \"local\"\ncommand = \"scripts/bay-check\"\nadapter = [\"scripts/bay-check\"]\nexpected_minimum = 1\ntimeout_s = 60\n\n[[check]]\nid = \"suite\"\nenvironment = \"local\"\nstage = \"post\"\ncommand = \"scripts/bay-check\"\nadapter = [\"scripts/bay-check\"]\nexpected_minimum = 1\ntimeout_s = 60\nwhen_paths = [\"src/**\"]\n";

/// The adapter. `suite`'s units are the files under `units/`; a unit fails when its file says
/// `fail`. A change under `src/` reaches every unit; the request's `rerun` adds the units it
/// names, and one the tree no longer has is named in `plan.gone`. While `conf/half` exists the
/// adapter ignores `rerun` and runs t1 alone, so its run fails without answering t3.
const ADAPTER: &str = r#"#!/bin/sh
req=$(cat)
commit=$(printf '%s' "$req" | sed -n 's/.*"candidate":{"commit":"\([0-9a-f]*\)","tree":"\([0-9a-f]*\)"[,}].*/\1/p')
tree=$(printf '%s' "$req" | sed -n 's/.*"candidate":{"commit":"\([0-9a-f]*\)","tree":"\([0-9a-f]*\)"[,}].*/\2/p')
case "$req" in
*'"plan":{"id":"suite"'*) ;;
*) printf '{"schema":"bay.check.result/1","exercised":{"commit":"%s","tree":"%s"},"plan":{"selected":1,"completed":1},"checks":[{"id":"t0","verdict":"pass"}]}' "$commit" "$tree"; exit 0 ;;
esac
changed=$(printf '%s' "$req" | sed -n 's/.*"changed_paths":\[\([^]]*\)\].*/\1/p')
rerun=$(printf '%s' "$req" | sed -n 's/.*"rerun":\[\([^]]*\)\].*/\1/p' | tr -d '"' | tr ',' ' ')
wanted=
case "$changed" in *'"src/'*) wanted=$(ls units) ;; esac
if [ -f conf/half ]; then wanted=t1; rerun=; fi
units=
gone=
n=0
seen=" "
for u in $wanted $rerun; do
  case "$seen" in *" $u "*) continue ;; esac
  seen="$seen$u "
  if [ -f "units/$u" ]; then
    verdict=pass
    [ "$(cat "units/$u")" = fail ] && verdict=product_failure
    units="$units${units:+,}{\"id\":\"$u\",\"verdict\":\"$verdict\"}"
    n=$((n + 1))
  else
    gone="$gone${gone:+,}\"$u\""
  fi
done
printf '{"schema":"bay.check.result/1","exercised":{"commit":"%s","tree":"%s"},"plan":{"selected":%s,"completed":%s,"gone":[%s]},"checks":[%s]}' "$commit" "$tree" "$n" "$n" "$gone" "$units"
"#;

/// The `suite` records on the remote about `tip`.
fn suite_records(test_box: &TestBox, remote: &Path, tip: &str) -> Vec<Value> {
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
                record["check"]["id"] == "suite" && record["subject"]["commit"] == tip
            }));
        }
    }
    out
}

fn unit_ids(record: &Value) -> Vec<String> {
    record["units"]
        .as_array()
        .unwrap()
        .iter()
        .map(|unit| unit["id"].as_str().unwrap().to_owned())
        .collect()
}

/// `suite`'s repository and its remote: units t1 and t3 fail, and t2 passes.
fn seeded(test_box: &TestBox) -> (PathBuf, PathBuf) {
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
    test_box.seed_with_check(
        &repo,
        &[
            ("src/a.rs", "fn a() {}\n"),
            ("conf/readme", "conf\n"),
            ("units/t1", "fail\n"),
            ("units/t2", "pass\n"),
            ("units/t3", "fail\n"),
        ],
        &files,
    );
    (repo, remote)
}

/// The suite: lands changes, runs one runner pass, and reads `suite`'s record on the tip.
struct Suite {
    test_box: TestBox,
    repo: PathBuf,
    remote: PathBuf,
}

impl Suite {
    fn new() -> Self {
        let test_box = TestBox::new();
        let (repo, remote) = seeded(&test_box);
        Self {
            test_box,
            repo,
            remote,
        }
    }

    /// Opens `name` on `paths`, edits its view, and waits until it lands.
    fn land(&self, name: &str, paths: &[&str], edit: &dyn Fn(&Path)) {
        let mut args = vec!["start", name, "--paths"];
        args.extend(paths);
        let started = self.test_box.bay_ok(&self.repo, &args);
        let id = started["change"].as_str().unwrap().to_owned();
        let view = PathBuf::from(started["detail"]["view"].as_str().unwrap());
        edit(&view);
        let (code, merged) = result_json(self.test_box.bay(&view).args(["merge", "--wait"]));
        assert_eq!(code, 0, "{merged}");
        assert!(
            self.test_box.trunk_change_ids(&self.remote).contains(&id),
            "{name} lands: {merged}"
        );
    }

    /// One runner pass, then `suite`'s one record on the tip.
    fn run(&self) -> Value {
        let ran = self
            .test_box
            .bay_ok(&self.repo, &["runner", "run", "--once"]);
        let tip = self
            .test_box
            .git_bare(&self.remote, &["rev-parse", "main"])
            .trim()
            .to_owned();
        let mut records = suite_records(&self.test_box, &self.remote, &tip);
        assert_eq!(records.len(), 1, "suite ran on the tip: {ran}");
        records.remove(0)
    }

    fn red(&self) -> Value {
        self.test_box.bay_ok(&self.repo, &["status"])["detail"]["red"].clone()
    }
}

#[test]
fn a_red_check_runs_its_failed_units_and_what_changed_since_its_newest_failing_run() {
    let suite = Suite::new();

    // `one` lands on src/: every unit runs, and t1 and t3 fail. `suite` is red.
    suite.land("one", &["src"], &|view| {
        std::fs::write(view.join("src/one.rs"), "x\n").unwrap();
    });
    let first = suite.run();
    assert_eq!(unit_ids(&first), ["t1", "t2", "t3"], "{first}");
    assert_eq!(suite.red()[0]["check"], "suite", "suite is red: {first}");

    // `two` lands off suite's paths. The failing run was asked no unit again, so the range is
    // since suite's newest pass (G495): it holds `one`'s src/, and every unit runs again.
    suite.land("two", &["conf"], &|view| {
        std::fs::write(view.join("conf/two"), "x\n").unwrap();
    });
    let second = suite.run();
    assert_eq!(second["post"]["rerun"], json!(["t1", "t3"]), "{second}");
    assert_eq!(unit_ids(&second), ["t1", "t2", "t3"], "{second}");
    assert_eq!(second["verdict"], "product_failure", "{second}");

    // `three` lands off suite's paths. The newest failing run answered t1 and t3, so the range is
    // since its landing: nothing there wakes a unit, and only t1 and t3 run.
    suite.land("three", &["conf"], &|view| {
        std::fs::write(view.join("conf/three"), "x\n").unwrap();
    });
    let third = suite.run();
    assert_eq!(third["post"]["rerun"], json!(["t1", "t3"]), "{third}");
    assert_eq!(
        unit_ids(&third),
        ["t1", "t3"],
        "the range before the failing run is not tested again: {third}"
    );
    assert_eq!(third["verdict"], "product_failure", "{third}");
    assert_eq!(suite.red()[0]["check"], "suite", "suite is still red");

    // The fix lands off suite's paths: t1 passes, and t3 is deleted. The run asks for both and
    // passes, and main is green.
    suite.land("fix", &["units"], &|view| {
        std::fs::write(view.join("units/t1"), "pass\n").unwrap();
        std::fs::remove_file(view.join("units/t3")).unwrap();
    });
    let fixed = suite.run();
    assert_eq!(unit_ids(&fixed), ["t1"], "{fixed}");
    assert_eq!(fixed["gone"], json!(["t3"]), "{fixed}");
    assert_eq!(fixed["verdict"], "pass", "{fixed}");
    assert_eq!(suite.red(), json!([]), "the fix clears the red");
}

#[test]
fn a_failing_run_that_left_a_failed_unit_unanswered_keeps_the_range_since_the_pass() {
    let suite = Suite::new();
    suite.land("one", &["src"], &|view| {
        std::fs::write(view.join("src/one.rs"), "x\n").unwrap();
    });
    let first = suite.run();
    assert_eq!(suite.red()[0]["check"], "suite", "suite is red: {first}");

    // `half` lands off suite's paths, and its run reads no `rerun`: t1 runs alone and fails, and
    // t3 goes unanswered.
    suite.land("half", &["conf"], &|view| {
        std::fs::write(view.join("conf/half"), "x\n").unwrap();
    });
    let half = suite.run();
    assert_eq!(half["post"]["rerun"], json!(["t1", "t3"]), "{half}");
    assert_eq!(unit_ids(&half), ["t1"], "{half}");
    assert_eq!(half["verdict"], "product_failure", "{half}");

    // The newest failing run did not answer t3, so it proves nothing about the range before it:
    // the next run is asked since suite's newest pass again, and every unit runs. (`whole` writes
    // a file of its own: a tree that already has a record is not run again.)
    suite.land("whole", &["conf"], &|view| {
        std::fs::remove_file(view.join("conf/half")).unwrap();
        std::fs::write(view.join("conf/whole"), "x\n").unwrap();
    });
    let whole = suite.run();
    assert_eq!(whole["post"]["rerun"], json!(["t1"]), "{whole}");
    assert_eq!(
        unit_ids(&whole),
        ["t1", "t2", "t3"],
        "a run that left a unit unanswered narrows nothing: {whole}"
    );
}
