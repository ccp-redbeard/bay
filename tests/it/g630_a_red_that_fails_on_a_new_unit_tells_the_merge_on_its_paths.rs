//! G630 (a fleet finding, 2026-11-02: a red came back on another unit after a fix landed, and the
//! session of the merge that broke it was told nothing). A red post check that fails on a unit its
//! first failure did not is told to the merges on the check's paths since the failing run before,
//! once each.
//!
//! G571 tells a red to the change that owns it: its first failure, which stays the owner while the
//! red stands. A fix that mends that unit and breaks another leaves the red standing under the
//! same owner, so before G630 no one was told of the new unit.
//!
//! Guards to mutate: in `src/red_owner.rs` `told`, the unit owners (`owners.extend(more)`: the fix
//! is never told); `unit_owners`' `red.covers` filter (every landing since the failing run before
//! is told: the landing off the check's paths too).

#![cfg(unix)]

use crate::support;

use std::path::{Path, PathBuf};

use serde_json::Value;
use support::{TestBox, result_json, write_executable};

/// A pre check `tests` that always passes, and a post check `suite` on `src/**`.
const POLICY: &str = "revision = 1\n\n[[check]]\nid = \"tests\"\nenvironment = \"local\"\ncommand = \"scripts/bay-check\"\nadapter = [\"scripts/bay-check\"]\nexpected_minimum = 1\ntimeout_s = 60\n\n[[check]]\nid = \"suite\"\nenvironment = \"local\"\nstage = \"post\"\ncommand = \"scripts/bay-check\"\nadapter = [\"scripts/bay-check\"]\nexpected_minimum = 1\ntimeout_s = 60\nwhen_paths = [\"src/**\"]\n";

/// The adapter. `suite`'s units are the files under `units/`; a unit fails when its file says
/// `fail`. A change under `src/` reaches every unit, and the request's `rerun` adds the units it
/// names.
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
units=
n=0
seen=" "
for u in $wanted $rerun; do
  case "$seen" in *" $u "*) continue ;; esac
  seen="$seen$u "
  [ -f "units/$u" ] || continue
  verdict=pass
  [ "$(cat "units/$u")" = fail ] && verdict=product_failure
  units="$units${units:+,}{\"id\":\"$u\",\"verdict\":\"$verdict\"}"
  n=$((n + 1))
done
printf '{"schema":"bay.check.result/1","exercised":{"commit":"%s","tree":"%s"},"plan":{"selected":%s,"completed":%s},"checks":[%s]}' "$commit" "$tree" "$n" "$n" "$units"
"#;

/// The trunk: `suite`'s units t1 and t2 pass. Its wake adapter `scripts/bay-wake` appends each
/// request to `<root>/woken`.
struct Suite {
    test_box: TestBox,
    repo: PathBuf,
    remote: PathBuf,
    woken: PathBuf,
}

impl Suite {
    fn new() -> Self {
        let test_box = TestBox::new();
        let repo = test_box.repo_any("repo");
        let remote = test_box.path().join("repo-remote.git");
        let woken = test_box.path().join("woken");
        std::fs::create_dir_all(repo.join("scripts")).unwrap();
        write_executable(
            &repo.join("scripts/bay-wake"),
            &format!(
                "#!/bin/sh\ncat >> '{}'\necho >> '{}'\nprintf '{{\"schema\":\"bay.wake-result/1\",\"result\":\"woken\",\"reason\":null}}'\n",
                woken.display(),
                woken.display()
            ),
        );
        let files = vec![
            ("scripts/bay-check".to_owned(), ADAPTER.to_owned()),
            (".bay/policy.toml".to_owned(), POLICY.to_owned()),
            (
                ".bay/config.toml".to_owned(),
                "[runner]\npost = true\nbisect_post = false\n\n[plugin.wake]\ncommand = \"scripts/bay-wake\"\n"
                    .to_owned(),
            ),
        ];
        test_box.seed_with_check(
            &repo,
            &[
                ("src/a.rs", "fn a() {}\n"),
                ("conf/readme", "conf\n"),
                ("units/t1", "pass\n"),
                ("units/t2", "pass\n"),
            ],
            &files,
        );
        Self {
            test_box,
            repo,
            remote,
            woken,
        }
    }

    /// Opens a change with `start` (its intent and flags), edits its view, and waits until it
    /// lands. Returns its id.
    fn land(&self, start: &[&str], edit: &dyn Fn(&Path)) -> String {
        let mut args = vec!["start"];
        args.extend(start);
        let started = self.test_box.bay_ok(&self.repo, &args);
        let id = started["change"].as_str().unwrap().to_owned();
        let view = PathBuf::from(started["detail"]["view"].as_str().unwrap());
        edit(&view);
        let (code, merged) = result_json(self.test_box.bay(&view).args(["merge", "--wait"]));
        assert_eq!(code, 0, "{merged}");
        assert!(
            self.test_box.trunk_change_ids(&self.remote).contains(&id),
            "{start:?} lands: {merged}"
        );
        id
    }

    /// One runner pass, then a second, which tells what the first one's records judge.
    fn run(&self) -> Value {
        let ran = self
            .test_box
            .bay_ok(&self.repo, &["runner", "run", "--once"]);
        self.test_box
            .bay_ok(&self.repo, &["runner", "run", "--once"]);
        ran
    }

    /// The `red` wake requests the adapter was given, oldest first.
    fn red_wakes(&self) -> Vec<Value> {
        std::fs::read_to_string(&self.woken)
            .unwrap_or_default()
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|request| request["cause"] == "red")
            .collect()
    }

    fn red(&self) -> Value {
        self.test_box.bay_ok(&self.repo, &["status"])["detail"]["red"].clone()
    }
}

#[test]
fn a_red_that_fails_on_a_new_unit_tells_the_merge_on_its_paths_once() {
    let suite = Suite::new();

    // `one` breaks t1 on src/: suite is red, and `one` is told so.
    let one = suite.land(&["one", "--paths", "src", "units"], &|view| {
        std::fs::write(view.join("src/one.rs"), "x\n").unwrap();
        std::fs::write(view.join("units/t1"), "fail\n").unwrap();
    });
    let ran = suite.run();
    assert_eq!(suite.red()[0]["check"], "suite", "suite is red: {ran}");
    let wakes = suite.red_wakes();
    assert_eq!(wakes.len(), 1, "{wakes:?}");
    assert_eq!(wakes[0]["change"], one.as_str(), "{wakes:?}");

    // `other` lands off suite's paths, and `fix` mends t1 and breaks t2 on src/. suite runs on
    // the tip alone: t1 passes and t2 fails, so the red stands, owned by `one`.
    let other = suite.land(&["other", "--paths", "conf"], &|view| {
        std::fs::write(view.join("conf/other"), "x\n").unwrap();
    });
    let fix = suite.land(
        &["fix", "--paths", "src", "units", "--fixes", &one],
        &|view| {
            std::fs::write(view.join("src/fix.rs"), "x\n").unwrap();
            std::fs::write(view.join("units/t1"), "pass\n").unwrap();
            std::fs::write(view.join("units/t2"), "fail\n").unwrap();
        },
    );
    let ran = suite.run();
    assert_eq!(
        suite.red()[0]["check"],
        "suite",
        "suite is still red: {ran}"
    );

    // `fix` is the one merge on suite's paths since the failing run before: it is told of t2,
    // once. `other` is off the paths and `one` was told already.
    let wakes = suite.red_wakes();
    assert_eq!(wakes.len(), 2, "{wakes:?}");
    assert_eq!(wakes[1]["change"], fix.as_str(), "{wakes:?}");
    let line = wakes[1]["line"].as_str().unwrap();
    assert!(
        line.contains(
            "is the one merge on its paths since the failing run before: it now fails on 1 more \
             unit, not a proven cause"
        ),
        "{line}"
    );
    assert!(
        wakes.iter().all(|wake| wake["change"] != other.as_str()),
        "{wakes:?}"
    );
    suite.run();
    assert_eq!(suite.red_wakes().len(), 2, "a unit owner is told once");

    // t2 is mended off suite's paths: the run asks for it again and passes, and no one is told.
    suite.land(&["mend", "--paths", "units"], &|view| {
        std::fs::write(view.join("units/t2"), "pass\n").unwrap();
    });
    suite.run();
    assert_eq!(
        suite.red(),
        serde_json::json!([]),
        "the mend clears the red"
    );
    assert_eq!(suite.red_wakes().len(), 2, "a green main tells no one");
}
