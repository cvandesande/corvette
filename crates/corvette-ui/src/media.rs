//! Retained recording ranges, the media URLs addressing them, and range presets.

use corvette_api::RecordingSegment;

use crate::local_time::{local_day_start, recent_calendar_days};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RecordingRange {
    pub(crate) camera: String,
    pub(crate) start_time: f64,
    pub(crate) end_time: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RecordingClip {
    pub(crate) range: RecordingRange,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RecordingMedia {
    pub(crate) clips: Vec<RecordingClip>,
    pub(crate) motion_ranges: Vec<RecordingRange>,
}

impl RecordingRange {
    pub(crate) fn clip_url(&self) -> String {
        let camera = js_sys::encode_uri_component(&self.camera);
        format!(
            "/api/{camera}/start/{}/end/{}/clip.mp4",
            self.start_time, self.end_time
        )
    }

    pub(crate) fn poster_url(&self) -> String {
        let camera = js_sys::encode_uri_component(&self.camera);
        let frame_time = self.start_time.midpoint(self.end_time);
        format!("/api/{camera}/recordings/{frame_time}/snapshot.jpg?height=720")
    }
}

pub(crate) fn recording_media(
    selection: &RecordingRange,
    mut segments: Vec<RecordingSegment>,
) -> RecordingMedia {
    const MAX_SEGMENT_GAP_SECONDS: f64 = 1.0;

    segments.sort_by(|left, right| left.start_time.total_cmp(&right.start_time));
    let mut clip_ranges = Vec::<RecordingRange>::new();
    let mut motion_ranges = Vec::<RecordingRange>::new();
    for segment in segments {
        let start_time = segment.start_time.max(selection.start_time);
        let end_time = segment.end_time.min(selection.end_time);
        let has_motion = segment.motion.is_some_and(|motion| motion > 0.0);
        if start_time >= end_time {
            continue;
        }

        if has_motion {
            push_contiguous_range(
                &mut motion_ranges,
                selection,
                start_time,
                end_time,
                MAX_SEGMENT_GAP_SECONDS,
            );
        }
        push_contiguous_range(
            &mut clip_ranges,
            selection,
            start_time,
            end_time,
            MAX_SEGMENT_GAP_SECONDS,
        );
    }
    let mut clips = clip_ranges
        .into_iter()
        .map(|range| RecordingClip { range })
        .collect::<Vec<_>>();
    clips.reverse();
    RecordingMedia {
        clips,
        motion_ranges,
    }
}

fn push_contiguous_range(
    ranges: &mut Vec<RecordingRange>,
    selection: &RecordingRange,
    start_time: f64,
    end_time: f64,
    maximum_gap: f64,
) {
    if let Some(range) = ranges.last_mut()
        && start_time <= range.end_time + maximum_gap
    {
        range.end_time = range.end_time.max(end_time);
        return;
    }
    ranges.push(RecordingRange {
        camera: selection.camera.clone(),
        start_time,
        end_time,
    });
}

pub(crate) const RECORDING_PRESETS: [RecordingPreset; 5] = [
    RecordingPreset::Today,
    RecordingPreset::Yesterday,
    RecordingPreset::LastTenMinutes,
    RecordingPreset::LastHour,
    RecordingPreset::LastSevenDays,
];

#[derive(Clone, Copy)]
pub(crate) enum RecordingPreset {
    Today,
    Yesterday,
    LastTenMinutes,
    LastHour,
    LastSevenDays,
}

impl RecordingPreset {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Today => "Today",
            Self::Yesterday => "Yesterday",
            Self::LastTenMinutes => "Last 10 minutes",
            Self::LastHour => "Last hour",
            Self::LastSevenDays => "Last 7 days",
        }
    }

    pub(crate) fn range(self) -> (f64, f64) {
        let now = js_sys::Date::new_0();
        let end_time = now.get_time() / 1_000.0;
        let today = local_day_start(&now);
        match self {
            Self::Today => (today, end_time),
            Self::Yesterday => {
                let yesterday = recent_calendar_days(2)[0].start_time;
                (yesterday, today)
            }
            Self::LastTenMinutes => (end_time - 600.0, end_time),
            Self::LastHour => (end_time - 3_600.0, end_time),
            Self::LastSevenDays => (recent_calendar_days(7)[0].start_time, end_time),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contiguous_segments_become_newest_first_recordings() {
        let selection = RecordingRange {
            camera: "front".to_owned(),
            start_time: 100.0,
            end_time: 200.0,
        };
        let segments = vec![
            RecordingSegment {
                start_time: 150.0,
                end_time: 160.0,
                motion: Some(0.0),
            },
            RecordingSegment {
                start_time: 90.0,
                end_time: 110.0,
                motion: Some(2.0),
            },
            RecordingSegment {
                start_time: 160.5,
                end_time: 170.0,
                motion: Some(3.0),
            },
            RecordingSegment {
                start_time: 205.0,
                end_time: 215.0,
                motion: None,
            },
        ];

        assert_eq!(
            recording_media(&selection, segments),
            RecordingMedia {
                clips: vec![
                    RecordingClip {
                        range: RecordingRange {
                            camera: "front".to_owned(),
                            start_time: 150.0,
                            end_time: 170.0,
                        },
                    },
                    RecordingClip {
                        range: RecordingRange {
                            camera: "front".to_owned(),
                            start_time: 100.0,
                            end_time: 110.0,
                        },
                    },
                ],
                motion_ranges: vec![
                    RecordingRange {
                        camera: "front".to_owned(),
                        start_time: 100.0,
                        end_time: 110.0,
                    },
                    RecordingRange {
                        camera: "front".to_owned(),
                        start_time: 160.5,
                        end_time: 170.0,
                    },
                ],
            }
        );
    }
}
