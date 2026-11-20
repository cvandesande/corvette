use corvette_detect::{RateWindow, Rates};
use std::time::{Duration, Instant};

#[test]
fn a_full_second_of_totals_publishes_three_hundred_and_one_hundred_tenths() {
    let start = Instant::now();
    let mut window = RateWindow::new(start);
    window.observe(start + Duration::from_secs(1), 10, 30);
    assert_eq!(
        window.rates(),
        Rates {
            fps_in_tenths: 300,
            fps_processed_tenths: 100,
        }
    );
}

#[test]
fn two_seconds_of_the_same_totals_publishes_half_the_tenths() {
    let start = Instant::now();
    let mut window = RateWindow::new(start);
    window.observe(start + Duration::from_secs(2), 10, 30);
    assert_eq!(
        window.rates(),
        Rates {
            fps_in_tenths: 150,
            fps_processed_tenths: 50,
        }
    );
}

#[test]
fn an_observation_before_the_second_leaves_the_published_rates_alone() {
    let start = Instant::now();
    let mut window = RateWindow::new(start);
    window.observe(start + Duration::from_millis(900), 27, 9);
    assert_eq!(window.rates(), Rates::default());
    window.observe(start + Duration::from_secs(1), 10, 30);
    window.observe(start + Duration::from_millis(1_500), 90, 200);
    assert_eq!(
        window.rates(),
        Rates {
            fps_in_tenths: 300,
            fps_processed_tenths: 100,
        },
        "an observation half a second into the new window must not publish yet"
    );
}

#[test]
fn a_restarted_counter_publishes_zero_instead_of_a_negative_rate() {
    let start = Instant::now();
    let mut window = RateWindow::new(start);
    window.observe(start + Duration::from_secs(1), 100, 100);
    assert_eq!(
        window.rates(),
        Rates {
            fps_in_tenths: 1000,
            fps_processed_tenths: 1000,
        }
    );
    window.observe(start + Duration::from_secs(2), 5, 5);
    assert_eq!(window.rates(), Rates::default());
}

#[test]
fn a_long_idle_gap_publishes_zero_rates() {
    let start = Instant::now();
    let mut window = RateWindow::new(start);
    window.observe(start + Duration::from_secs(1), 30, 30);
    assert_ne!(window.rates(), Rates::default());
    window.observe(start + Duration::from_secs(11), 30, 30);
    assert_eq!(window.rates(), Rates::default());
}

#[test]
fn five_frames_over_three_seconds_round_to_seventeen_tenths() {
    let start = Instant::now();
    let mut window = RateWindow::new(start);
    window.observe(start + Duration::from_secs(3), 0, 5);
    assert_eq!(
        window.rates(),
        Rates {
            fps_in_tenths: 17,
            fps_processed_tenths: 0,
        }
    );
}

#[test]
fn the_window_restarts_at_the_observation_that_completes_it() {
    let start = Instant::now();
    let mut window = RateWindow::new(start);
    window.observe(start + Duration::from_secs(1), 10, 30);
    window.observe(start + Duration::from_secs(2), 30, 60);
    assert_eq!(
        window.rates(),
        Rates {
            fps_in_tenths: 300,
            fps_processed_tenths: 200,
        }
    );
}
