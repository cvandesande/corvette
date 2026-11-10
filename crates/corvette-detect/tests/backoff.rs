use std::time::Duration;

use corvette_detect::{Backoff, DEFAULT_POLICY, Policy};

const fn policy() -> Policy {
    Policy {
        initial: Duration::from_millis(100),
        max: Duration::from_millis(800),
        healthy_after: Duration::from_secs(10),
    }
}

#[test]
fn delays_double_from_the_initial_value_up_to_the_cap() {
    let mut backoff = Backoff::new(policy());
    let expected = [
        Duration::from_millis(100),
        Duration::from_millis(200),
        Duration::from_millis(400),
        Duration::from_millis(800),
        Duration::from_millis(800),
        Duration::from_millis(800),
    ];
    for want in expected {
        assert_eq!(want, backoff.after_run(Duration::ZERO));
    }
}

#[test]
fn a_healthy_run_resets_the_delay_to_the_initial_value() {
    let mut backoff = Backoff::new(policy());
    for grown in [
        Duration::from_millis(100),
        Duration::from_millis(200),
        Duration::from_millis(400),
        Duration::from_millis(800),
    ] {
        assert_eq!(grown, backoff.after_run(Duration::ZERO));
    }
    assert_eq!(
        Duration::from_millis(100),
        backoff.after_run(Duration::from_secs(10))
    );
    assert_eq!(
        Duration::from_millis(200),
        backoff.after_run(Duration::ZERO)
    );
    assert_eq!(
        Duration::from_millis(400),
        backoff.after_run(Duration::from_millis(9_999))
    );
}

#[test]
fn the_default_policy_holds_the_documented_values() {
    assert_eq!(
        Policy {
            initial: Duration::from_millis(200),
            max: Duration::from_secs(10),
            healthy_after: Duration::from_secs(30),
        },
        DEFAULT_POLICY
    );
}
