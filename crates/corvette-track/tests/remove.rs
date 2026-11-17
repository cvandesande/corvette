use corvette_track::{IdSource, Observation, Sighting, TrackConfig, Tracker};

const fn tracker() -> Tracker<u64> {
    let config = TrackConfig::new(25, 2, 4, 3.4, 0.1).expect("test config");
    Tracker::new(config, 2.5)
}

// Feeds one frame of "car" detections, boxes moved two pixels a frame like the tracked scenarios.
fn cars(tracker: &mut Tracker<u64>, ids: &mut IdSource, dets: &[(u64, i32)]) {
    let sightings: Vec<Sighting<u64>> = dets
        .iter()
        .map(|(tag, x)| Sighting {
            label: "car".to_string(),
            observation: Observation {
                edges: [*x, 100, x + 40, 180],
                data: *tag,
            },
        })
        .collect();
    tracker.update(ids, sightings).expect("car frame");
}

fn summary(tracker: &Tracker<u64>) -> Vec<(Option<u64>, i64)> {
    tracker
        .tracks()
        .iter()
        .map(|track| (track.id(), track.hit_counter()))
        .collect()
}

// Three real tracks born one frame apart near x 100, 300 and 500, so the ids follow list order.
fn three_real_tracks() -> (Tracker<u64>, IdSource) {
    let mut tracker = tracker();
    let mut ids = IdSource::default();
    cars(&mut tracker, &mut ids, &[(1, 100)]);
    cars(&mut tracker, &mut ids, &[(2, 102), (3, 300)]);
    cars(&mut tracker, &mut ids, &[(4, 104), (5, 302), (6, 500)]);
    cars(&mut tracker, &mut ids, &[(7, 106), (8, 304), (9, 502)]);
    cars(&mut tracker, &mut ids, &[(10, 108), (11, 306), (12, 504)]);
    assert_eq!(
        summary(&tracker),
        vec![(Some(1), 5), (Some(2), 4), (Some(3), 3)]
    );
    (tracker, ids)
}

// Two live tracks of one label near x 100 and 300, the second born one frame after the first.
fn two_real_tracks() -> (Tracker<u64>, IdSource) {
    let mut tracker = tracker();
    let mut ids = IdSource::default();
    cars(&mut tracker, &mut ids, &[(1, 100)]);
    cars(&mut tracker, &mut ids, &[(2, 102), (3, 300)]);
    cars(&mut tracker, &mut ids, &[(4, 104), (5, 302)]);
    cars(&mut tracker, &mut ids, &[(6, 106), (7, 304)]);
    assert_eq!(summary(&tracker), vec![(Some(1), 4), (Some(2), 3)]);
    (tracker, ids)
}

#[test]
fn removes_only_the_named_track() {
    let (mut tracker, _) = three_real_tracks();
    assert!(tracker.remove(2));
    assert_eq!(summary(&tracker), vec![(Some(1), 5), (Some(3), 3)]);
    assert_eq!(tracker.tracks().len(), 2);
}

#[test]
fn unknown_id_changes_nothing() {
    let (mut tracker, _) = three_real_tracks();
    assert!(!tracker.remove(4242));
    assert_eq!(
        summary(&tracker),
        vec![(Some(1), 5), (Some(2), 4), (Some(3), 3)]
    );
    assert!(!tracker.remove(0));
    assert_eq!(
        summary(&tracker),
        vec![(Some(1), 5), (Some(2), 4), (Some(3), 3)]
    );
}

#[test]
fn removal_keeps_the_order_of_the_rest() {
    let (mut tracker, _) = three_real_tracks();
    assert!(tracker.remove(1));
    assert_eq!(summary(&tracker), vec![(Some(2), 4), (Some(3), 3)]);
}

#[test]
fn unknown_id_returns_false_on_an_empty_tracker() {
    let mut tracker = tracker();
    assert!(!tracker.remove(1));
    assert!(tracker.tracks().is_empty());
}

#[test]
fn initializing_track_is_not_removable() {
    let mut tracker = tracker();
    let mut ids = IdSource::default();
    cars(&mut tracker, &mut ids, &[(1, 100)]);
    assert!(tracker.tracks()[0].is_initializing());
    assert_eq!(tracker.tracks()[0].id(), None);
    assert_eq!(tracker.tracks()[0].initializing_id(), 1);
    assert!(!tracker.remove(1));
    assert!(!tracker.remove(0));
    assert_eq!(summary(&tracker), vec![(None, 1)]);
}

#[test]
fn removes_an_expired_listed_track() {
    let mut tracker = tracker();
    let mut ids = IdSource::default();
    cars(&mut tracker, &mut ids, &[(1, 100)]);
    cars(&mut tracker, &mut ids, &[(2, 102)]);
    cars(&mut tracker, &mut ids, &[(3, 104)]);
    assert_eq!(summary(&tracker), vec![(Some(1), 3)]);
    for _ in 0..4 {
        cars(&mut tracker, &mut ids, &[]);
    }
    assert_eq!(summary(&tracker), vec![(Some(1), -1)]);
    assert!(!tracker.tracks()[0].is_active());
    assert!(tracker.remove(1));
    assert!(tracker.tracks().is_empty());
    assert!(!tracker.remove(1));
}

#[test]
fn matching_continues_after_removal() {
    let (mut tracker, mut ids) = two_real_tracks();
    assert!(tracker.remove(1));
    assert_eq!(summary(&tracker), vec![(Some(2), 3)]);
    cars(&mut tracker, &mut ids, &[(8, 306)]);
    assert_eq!(summary(&tracker), vec![(Some(2), 4)]);
    assert_eq!(tracker.tracks()[0].last_detection().data, 8);
    // The empty place of the removed track starts a fresh track only once a detection appears there.
    cars(&mut tracker, &mut ids, &[(9, 308), (10, 100)]);
    assert_eq!(summary(&tracker), vec![(Some(2), 5), (None, 1)]);
}

#[test]
fn removal_keeps_other_live_tracks_of_the_same_label() {
    let (mut tracker, _) = two_real_tracks();
    assert!(tracker.remove(1));
    assert_eq!(summary(&tracker), vec![(Some(2), 3)]);
    assert_eq!(tracker.active_tracks().count(), 1);
    assert!(tracker.tracks()[0].is_active());
    assert_eq!(tracker.tracks()[0].label(), "car");
}
