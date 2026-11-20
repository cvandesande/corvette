//! The two bodies one camera asks the detection service for: its objects answer and one status entry.

use crate::camera_runner::{CameraState, CameraStatus};
use crate::object_stage::FrameObjects;
use crate::record::{RecordError, json_string, object_entries};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ObjectsAnswer {
    Objects(String),
    Unknown,
}

pub const OFF_STATE: &str = "off";

/// Turns the last published frame into its objects body; no frame ever published is `Unknown`.
/// # Errors
pub fn objects_answer(published: Option<&FrameObjects>) -> Result<ObjectsAnswer, RecordError> {
    let Some(frame) = published else {
        return Ok(ObjectsAnswer::Unknown);
    };
    let entries = object_entries(frame)?;
    Ok(ObjectsAnswer::Objects(format!(
        "{{\"objects\":[{entries}]}}"
    )))
}

#[must_use]
pub const fn state_name(state: CameraState) -> &'static str {
    match state {
        CameraState::Running => "running",
        CameraState::WaitingForFrames => "waiting_for_frames",
        CameraState::InferenceUnavailable => "inference_unavailable",
        CameraState::Failed => "failed",
    }
}

#[must_use]
pub fn status_entry(camera: &str, status: &CameraStatus) -> String {
    format!(
        "{{\"camera\":{camera},\"state\":{state},\"frames_in\":{frames_in},\"frames_tracked\":{frames_tracked},\"frames_inference_failed\":{frames_inference_failed},\"source_gaps\":{source_gaps}}}",
        camera = json_string(camera),
        state = json_string(state_name(status.state)),
        frames_in = status.counters.frames_in,
        frames_tracked = status.counters.frames_tracked,
        frames_inference_failed = status.counters.frames_inference_failed,
        source_gaps = status.source_gaps,
    )
}

#[must_use]
pub fn off_entry(camera: &str) -> String {
    format!(
        "{{\"camera\":{},\"state\":\"{OFF_STATE}\"}}",
        json_string(camera)
    )
}
