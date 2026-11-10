//! The restart-delay policy: doubling delay, a cap, and a reset after a healthy run.

use std::time::Duration;

pub const BACKOFF_INITIAL: Duration = Duration::from_millis(200);
pub const BACKOFF_MAX: Duration = Duration::from_secs(10);
pub const HEALTHY_RUN: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    pub initial: Duration,
    pub max: Duration,
    pub healthy_after: Duration,
}

pub const DEFAULT_POLICY: Policy = Policy {
    initial: BACKOFF_INITIAL,
    max: BACKOFF_MAX,
    healthy_after: HEALTHY_RUN,
};

#[derive(Clone, Debug)]
pub struct Backoff {
    policy: Policy,
    next: Duration,
}

impl Backoff {
    #[must_use]
    pub const fn new(policy: Policy) -> Self {
        Self {
            policy,
            next: policy.initial,
        }
    }

    #[must_use]
    pub fn after_run(&mut self, ran_for: Duration) -> Duration {
        if ran_for >= self.policy.healthy_after {
            self.next = self.policy.initial;
        }
        let delay = self.next;
        self.next = self.next.saturating_mul(2).min(self.policy.max);
        delay
    }
}
