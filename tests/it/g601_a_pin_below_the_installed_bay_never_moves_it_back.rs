//! G601 (a fleet finding, 2026-10-31: five boxes kept `bay.updateVersion` at 0.5.35 or 0.5.44
//! while they ran 0.6.16 or 0.6.17). A pin is the oldest version a box runs (G393), so a pin
//! below the installed bay holds nothing back, and it is never the target: with a release list
//! that cannot be read, or a pin newer than every listed release but older than this bay,
//! `bay update` moves nothing.
//! A box goes back only when asked by `--version`, to a pin set at that version.
//!
//! Guard to mutate: `src/update.rs` `resolve`: the `on_pin` filter on a pin older than `BAY`.

// Each case runs shell scripts (adapters, a wrapper on PATH), so these run on Unix only.
#![cfg(unix)]

use crate::release_fixture;

use release_fixture::Fixture;

/// Two releases older than any bay this test runs.
const OLDER: &str = "0.0.1";
const OLD: &str = "0.0.2";

#[test]
fn a_pin_below_the_installed_bay_is_never_the_target() {
    let fixture = Fixture::new();
    fixture.release(OLDER, OLDER, None);
    fixture.release(OLD, OLD, None);
    fixture.box_config("bay.updateVersion", OLD);

    // No release list can be read: the old pin is not taken in its place.
    for args in [&["--check"][..], &[]] {
        let (code, result) = fixture.update(fixture.path(), args);
        assert_eq!(code, 10, "{result}");
        assert_eq!(result["reason"], "UPDATE_UNAVAILABLE", "{result}");
    }
    fixture.assert_untouched();

    // The pin is newer than every listed release, and older than this bay.
    fixture.list(&[(OLDER, false)]);
    let (code, result) = fixture.update(fixture.path(), &[]);
    assert_eq!(code, 10, "{result}");
    assert_eq!(result["reason"], "UPDATE_PINNED", "{result}");
    assert_eq!(result["detail"]["target"], OLDER, "{result}");
    fixture.assert_untouched();
}

#[test]
fn only_a_version_asked_for_at_the_pin_moves_a_box_back() {
    let fixture = Fixture::new();
    fixture.release(OLD, OLD, None);
    fixture.list(&[(OLD, false)]);
    fixture.box_config("bay.updateVersion", OLD);

    let (code, result) = fixture.update(fixture.path(), &["--version", OLD]);
    assert_eq!(code, 0, "{result}");
    assert_eq!(result["detail"]["updated"], true, "{result}");
    assert_eq!(result["detail"]["target"], OLD, "{result}");
    let say = result["say"].as_str().unwrap();
    assert!(say.starts_with("Updated bay from "), "{say}");
    assert!(say.contains(&format!(" to {OLD}.")), "{say}");
}
