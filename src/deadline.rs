//! BACKLOG 218 (a fleet finding, 2026-11-02: "`UserPromptSubmit` hook [... `bay hook
//! turn.start`] timed out after 30s — output discarded", too often): a process ends its wait for
//! the remote by one moment. A product ends a hook at its entry's timeout and drops what the hook
//! says. `bay hook` sets the deadline ([`crate::agent_hook::budget`]): a git call to a remote is
//! then bounded by the time left and is not started after it, and the hook waits for its poll
//! ([`crate::hook_poll`]) no longer than it. Every other command has no deadline, and each call
//! keeps its own bound.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

static UNTIL: OnceLock<Instant> = OnceLock::new();

/// Ends this process's slow work at `until`. Set once: a later call keeps the first.
pub fn set(until: Instant) {
    let _ = UNTIL.set(until);
}

/// `bound`, cut to the time left before this process's deadline; `None` once it has passed.
/// With no deadline, `bound` as it is.
#[must_use]
pub fn bounded(bound: Duration) -> Option<Duration> {
    within(bound, UNTIL.get().copied(), Instant::now())
}

fn within(bound: Duration, until: Option<Instant>, now: Instant) -> Option<Duration> {
    let Some(until) = until else {
        return Some(bound);
    };
    let left = until.saturating_duration_since(now);
    (!left.is_zero()).then(|| bound.min(left))
}

#[cfg(test)]
mod tests {
    use super::within;
    use std::time::{Duration, Instant};

    #[test]
    fn a_bound_is_cut_to_the_time_left_and_none_is_left_after_the_deadline() {
        let now = Instant::now();
        let bound = Duration::from_secs(180);
        assert_eq!(within(bound, None, now), Some(bound));
        let until = now + Duration::from_secs(4);
        assert_eq!(
            within(bound, Some(until), now),
            Some(Duration::from_secs(4))
        );
        let short = Duration::from_secs(1);
        assert_eq!(within(short, Some(until), now), Some(short));
        assert_eq!(within(bound, Some(until), until), None);
        assert_eq!(
            within(bound, Some(until), until + Duration::from_secs(1)),
            None
        );
    }
}
