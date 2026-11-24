//! Chooses the object lines logged from one frame: a confirm line when an id first reports confirmed, an ended line when a confirmed id retires.

use std::collections::HashMap;

use crate::object_stage::{FrameObjects, TrackedObject};

/// The object lines' per-runner memory: the ids already logged confirmed and the newest frame's timings each ended line quotes.
#[derive(Debug, Default)]
pub(crate) struct ObjectLog {
    live: HashMap<String, ConfirmedObject>,
}

#[derive(Clone, Debug)]
struct ConfirmedObject {
    label: String,
    first_seen: f64,
    last_seen: f64,
}

impl ConfirmedObject {
    fn from_frame(object: &TrackedObject) -> Self {
        Self {
            label: object.label.clone(),
            first_seen: object.first_seen,
            last_seen: object.last_seen,
        }
    }

    fn refresh(&mut self, object: &TrackedObject) {
        self.label.clone_from(&object.label);
        self.last_seen = object.last_seen;
    }

    fn seconds(&self) -> f64 {
        (self.last_seen - self.first_seen).max(0.0).round()
    }
}

impl ObjectLog {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Returns the `(event, detail)` pairs this frame logs, the way `transition_event` does for lifecycle lines: `object-confirmed` on an id's first confirmed frame, `object-ended` when a remembered id lists as ended.
    pub(crate) fn observe(&mut self, frame: &FrameObjects) -> Vec<(&'static str, String)> {
        let mut lines = Vec::new();
        for object in &frame.objects {
            if object.id.parse::<u64>().is_err() {
                continue;
            }
            if let Some(known) = self.live.get_mut(&object.id) {
                known.refresh(object);
                continue;
            }
            if !object.confirmed {
                continue;
            }
            let score_pct = (object.score * 100.0).round();
            lines.push((
                "object-confirmed",
                format!(
                    "id={} label={} score_pct={score_pct:.0} zones={}",
                    object.id,
                    sanitized_label(&object.label),
                    object.zones_now.len()
                ),
            ));
            self.live
                .insert(object.id.clone(), ConfirmedObject::from_frame(object));
        }
        for id in &frame.ended {
            let Some(known) = self.live.remove(id) else {
                continue;
            };
            lines.push((
                "object-ended",
                format!(
                    "id={id} label={} seconds={:.0}",
                    sanitized_label(&known.label),
                    known.seconds()
                ),
            ));
        }
        lines
    }
}

fn sanitized_label(label: &str) -> &str {
    if !label.is_empty()
        && label
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        label
    } else {
        "other"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(
        id: &str,
        label: &str,
        confirmed: bool,
        first_seen: f64,
        last_seen: f64,
    ) -> TrackedObject {
        TrackedObject {
            id: id.to_owned(),
            label: label.to_owned(),
            score: 0.9,
            confirmed,
            bounds: [0.0; 4],
            edges: [0, 0, 10, 10],
            first_seen,
            last_seen,
            zones_now: Vec::new(),
            zones_entered: Vec::new(),
            stationary: false,
        }
    }

    fn frame(objects: Vec<TrackedObject>, ended: &[&str]) -> FrameObjects {
        FrameObjects {
            frame_time: 0.0,
            objects,
            ended: ended.iter().map(|id| (*id).to_owned()).collect(),
        }
    }

    #[test]
    fn confirmed_object_logs_one_line_across_three_frames() {
        let mut log = ObjectLog::new();
        assert_eq!(
            log.observe(&frame(vec![object("1", "person", true, 0.0, 1.0)], &[])),
            vec![(
                "object-confirmed",
                "id=1 label=person score_pct=90 zones=0".to_owned()
            )]
        );
        assert!(
            log.observe(&frame(vec![object("1", "person", true, 0.0, 2.0)], &[]))
                .is_empty(),
            "the confirm line must not repeat"
        );
        assert!(
            log.observe(&frame(vec![object("1", "person", true, 0.0, 3.0)], &[]))
                .is_empty(),
            "the confirm line must not repeat"
        );
    }

    #[test]
    fn unconfirmed_object_logs_no_line() {
        let mut log = ObjectLog::new();
        assert!(
            log.observe(&frame(vec![object("1", "person", false, 0.0, 1.0)], &[]))
                .is_empty(),
            "an unconfirmed object must not log"
        );
        assert!(
            log.observe(&frame(vec![object("1", "person", false, 0.0, 2.0)], &[]))
                .is_empty(),
            "an unconfirmed object must not log"
        );
    }

    #[test]
    fn ended_object_logs_seconds_between_first_and_last_seen() {
        let mut log = ObjectLog::new();
        log.observe(&frame(vec![object("1", "person", true, 10.0, 10.0)], &[]));
        log.observe(&frame(vec![object("1", "person", true, 10.0, 12.4)], &[]));
        assert_eq!(
            log.observe(&frame(vec![], &["1"])),
            vec![("object-ended", "id=1 label=person seconds=2".to_owned())]
        );
        assert!(
            log.observe(&frame(vec![], &["1"])).is_empty(),
            "the ended entry is removed after its line"
        );
    }

    #[test]
    fn ended_object_that_never_confirmed_logs_nothing() {
        let mut log = ObjectLog::new();
        assert!(
            log.observe(&frame(vec![object("1", "person", false, 0.0, 1.0)], &[]))
                .is_empty()
        );
        assert!(
            log.observe(&frame(vec![], &["1"])).is_empty(),
            "an id that never confirmed has no ended line"
        );
    }

    #[test]
    fn hostile_label_logs_the_word_other() {
        let mut log = ObjectLog::new();
        let hostile = "pe\"rson\nsüda";
        assert_eq!(
            log.observe(&frame(vec![object("1", hostile, true, 0.0, 0.0)], &[])),
            vec![(
                "object-confirmed",
                "id=1 label=other score_pct=90 zones=0".to_owned()
            )]
        );
    }

    #[test]
    fn only_the_fixed_label_alphabet_is_written() {
        for (label, want) in [
            ("person", "person"),
            ("Car_9-x", "Car_9-x"),
            ("", "other"),
            ("a b", "other"),
            ("a\"b", "other"),
            ("a\nb", "other"),
            ("pérson", "other"),
        ] {
            let mut log = ObjectLog::new();
            let lines = log.observe(&frame(vec![object("1", label, true, 0.0, 0.0)], &[]));
            assert_eq!(
                lines[0].1,
                format!("id=1 label={want} score_pct=90 zones=0"),
                "label {label:?}"
            );
        }
    }

    #[test]
    fn non_numeric_id_logs_no_line() {
        let mut log = ObjectLog::new();
        assert!(
            log.observe(&frame(vec![object("h/ost", "person", true, 0.0, 0.0)], &[]))
                .is_empty(),
            "a non-numeric id must not log"
        );
        assert!(
            log.observe(&frame(vec![], &["h/ost"])).is_empty(),
            "a non-numeric id must not log"
        );
    }

    #[test]
    fn confirmed_object_reports_its_zone_count() {
        let mut tracked = object("7", "person", true, 0.0, 0.0);
        tracked.zones_now = vec!["door".to_owned(), "yard".to_owned()];
        let mut log = ObjectLog::new();
        assert_eq!(
            log.observe(&frame(vec![tracked], &[])),
            vec![(
                "object-confirmed",
                "id=7 label=person score_pct=90 zones=2".to_owned()
            )]
        );
    }
}
