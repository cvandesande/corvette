use std::time::{Duration, SystemTime, UNIX_EPOCH};

use corvette_detect::{Decode, FrameClock, FrameSpec};

#[test]
fn a_replay_clock_adds_the_index_over_the_rate_to_the_start() {
    let spec = FrameSpec::new(4, 2, 5, Decode::Cpu).expect("valid spec");
    let clock = FrameClock::replay(Duration::from_secs(1_000), spec);
    assert_eq!(clock.stamp(0), Duration::from_secs(1_000));
    assert_eq!(clock.stamp(1), Duration::from_millis(1_000_200));
    assert_eq!(clock.stamp(2), Duration::from_millis(1_000_400));
    assert_eq!(clock.stamp(5), Duration::from_secs(1_001));
}

#[test]
fn replay_times_are_exact_integer_fractions_of_a_second() {
    let spec = FrameSpec::new(4, 2, 3, Decode::Cpu).expect("valid spec");
    let clock = FrameClock::replay(Duration::ZERO, spec);
    assert_eq!(clock.stamp(1), Duration::from_nanos(333_333_333));
    assert_eq!(clock.stamp(2), Duration::from_nanos(666_666_666));
    assert_eq!(clock.stamp(3), Duration::from_secs(1));
    assert_eq!(clock.stamp(4), Duration::from_nanos(1_333_333_333));
}

#[test]
fn a_replay_clock_does_not_overflow() {
    let spec = FrameSpec::new(4, 2, 3, Decode::Cpu).expect("valid spec");
    let clock = FrameClock::replay(Duration::ZERO, spec);
    assert!(clock.stamp(u64::MAX) >= Duration::from_secs(6_148_914_691_236_517_204));
    let clock = FrameClock::replay(Duration::MAX, spec);
    assert_eq!(clock.stamp(1), Duration::MAX);
}

#[test]
fn a_live_clock_stamps_with_the_wall_clock() {
    let clock = FrameClock::live();
    let before = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after the epoch");
    let first = clock.stamp(0);
    let second = clock.stamp(7);
    let after = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after the epoch");
    assert!(first >= before && first <= after);
    assert!(second >= before && second <= after);
}
