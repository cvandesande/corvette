//! Chooses the camera lifecycle line logged for a runner-state transition.

use crate::camera_runner::CameraState::{
    self, Failed, InferenceUnavailable, Running, WaitingForFrames,
};

/// Returns the `(event, detail)` line this transition logs, or `None` for no line.
///
/// All text is fixed words and integers; `down_ms` is the time spent out of `Running` before this transition, `None` before the first frame.
pub(crate) fn transition_event(
    prev: Option<CameraState>,
    next: CameraState,
    down_ms: Option<u64>,
    timeout_ms: u64,
) -> Option<(&'static str, String)> {
    match (prev, next) {
        (None | Some(WaitingForFrames | Failed), Running) => {
            let detail =
                down_ms.map_or_else(|| "first_frame".to_string(), |ms| format!("down_ms={ms}"));
            Some(("connected", detail))
        }
        (Some(InferenceUnavailable), Running) => {
            let detail = down_ms.map_or_else(|| "ok".to_string(), |ms| format!("down_ms={ms}"));
            Some(("inference-restored", detail))
        }
        (Some(Running | InferenceUnavailable), WaitingForFrames) => {
            Some(("source-gap", format!("no_frames_for_ms={timeout_ms}")))
        }
        (None | Some(Running | WaitingForFrames | Failed), InferenceUnavailable) => {
            Some(("inference-unavailable", "worker_not_answering".to_string()))
        }
        (None | Some(Running | WaitingForFrames | InferenceUnavailable), Failed) => {
            Some(("runner-failed", "reason=pipeline-error".to_string()))
        }
        (None | Some(WaitingForFrames | Failed), WaitingForFrames)
        | (Some(Running), Running)
        | (Some(InferenceUnavailable), InferenceUnavailable)
        | (Some(Failed), Failed) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TIMEOUT: u64 = 1000;

    #[test]
    fn fresh_run_logs_connected() {
        for prev in [None, Some(WaitingForFrames), Some(Failed)] {
            let (event, detail) = transition_event(prev, Running, None, TIMEOUT).expect("logged");
            assert_eq!((event, detail.as_str()), ("connected", "first_frame"));
            let (event, detail) =
                transition_event(prev, Running, Some(250), TIMEOUT).expect("logged");
            assert_eq!((event, detail.as_str()), ("connected", "down_ms=250"));
        }
    }

    #[test]
    fn inference_restored_logs_downtime_or_ok() {
        let prev = Some(InferenceUnavailable);
        let (event, detail) = transition_event(prev, Running, Some(1750), TIMEOUT).expect("logged");
        assert_eq!(
            (event, detail.as_str()),
            ("inference-restored", "down_ms=1750")
        );
        let (event, detail) = transition_event(prev, Running, None, TIMEOUT).expect("logged");
        assert_eq!((event, detail.as_str()), ("inference-restored", "ok"));
    }

    #[test]
    fn frame_wait_after_live_logs_source_gap() {
        for prev in [Some(Running), Some(InferenceUnavailable)] {
            let (event, detail) =
                transition_event(prev, WaitingForFrames, None, 12345).expect("logged");
            assert_eq!(
                (event, detail.as_str()),
                ("source-gap", "no_frames_for_ms=12345")
            );
        }
    }

    #[test]
    fn inference_unavailable_logs_worker_not_answering() {
        for prev in [None, Some(Running), Some(WaitingForFrames), Some(Failed)] {
            let (event, detail) =
                transition_event(prev, InferenceUnavailable, None, TIMEOUT).expect("logged");
            assert_eq!(
                (event, detail.as_str()),
                ("inference-unavailable", "worker_not_answering")
            );
        }
    }

    #[test]
    fn runner_failure_logs_pipeline_error() {
        for prev in [
            None,
            Some(Running),
            Some(WaitingForFrames),
            Some(InferenceUnavailable),
        ] {
            let (event, detail) = transition_event(prev, Failed, None, TIMEOUT).expect("logged");
            assert_eq!(
                (event, detail.as_str()),
                ("runner-failed", "reason=pipeline-error")
            );
        }
    }

    #[test]
    fn only_listed_pairs_produce_a_line() {
        let states = [
            None,
            Some(Running),
            Some(WaitingForFrames),
            Some(InferenceUnavailable),
            Some(Failed),
        ];
        for prev in states {
            for next in [Running, WaitingForFrames, InferenceUnavailable, Failed] {
                let listed = match next {
                    Running => !matches!(prev, Some(Running)),
                    WaitingForFrames => {
                        matches!(prev, Some(Running | InferenceUnavailable))
                    }
                    InferenceUnavailable => prev != Some(InferenceUnavailable),
                    Failed => prev != Some(Failed),
                };
                for down_ms in [None, Some(7)] {
                    let got = transition_event(prev, next, down_ms, TIMEOUT).is_some();
                    assert_eq!(got, listed, "prev={prev:?} next={next:?}");
                }
            }
        }
    }

    #[test]
    fn emitted_text_carries_no_address_or_credential() {
        let states = [
            None,
            Some(Running),
            Some(WaitingForFrames),
            Some(InferenceUnavailable),
            Some(Failed),
        ];
        for prev in states {
            for next in [Running, WaitingForFrames, InferenceUnavailable, Failed] {
                for timeout_ms in [0u64, 10000, u64::MAX] {
                    for down_ms in [
                        None,
                        Some(0),
                        Some(7),
                        Some(1750),
                        Some(9_007_199_254_740_991),
                    ] {
                        let line = transition_event(prev, next, down_ms, timeout_ms);
                        let Some((event, detail)) = line else {
                            continue;
                        };
                        for text in [event, detail.as_str()] {
                            assert!(
                                text.chars().all(|c| {
                                    c.is_ascii_lowercase()
                                        || c.is_ascii_digit()
                                        || matches!(c, '_' | '=' | ' ' | '-')
                                }),
                                "{text:?} has a character outside the fixed alphabet"
                            );
                            assert!(
                                !text.contains("rtsp")
                                    && !text.contains("://")
                                    && !text.contains('@')
                                    && !text.contains("password"),
                                "{text:?} carries address or credential text"
                            );
                        }
                    }
                }
            }
        }
    }
}
