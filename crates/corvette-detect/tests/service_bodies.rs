use corvette_detect::{
    CameraState, CameraStatus, FrameObjects, OFF_STATE, ObjectsAnswer, PipelineCounters,
    RecordError, RecordHeader, RecordWriter, TrackedObject, objects_answer, off_entry, state_name,
    status_entry,
};
use serde_json::Value;

fn header() -> RecordHeader {
    RecordHeader {
        camera: "yard".to_owned(),
        segment: "2026-11-18-00-00-00.mp4".to_owned(),
        frame_width: 640,
        frame_height: 480,
        detect_fps: 5.0,
    }
}

fn object(id: &str, stationary: bool) -> TrackedObject {
    TrackedObject {
        id: id.to_owned(),
        label: "person".to_owned(),
        score: 0.9,
        confirmed: true,
        bounds: [0.1, 0.2, 0.3, 0.4],
        edges: [10, 20, 30, 40],
        first_seen: 5.0,
        last_seen: 6.0,
        zones_now: if stationary {
            vec!["gate".to_owned()]
        } else {
            Vec::new()
        },
        zones_entered: vec!["gate".to_owned()],
        stationary,
    }
}

const fn frame_of(objects: Vec<TrackedObject>) -> FrameObjects {
    FrameObjects {
        frame_time: 1.0,
        objects,
        ended: Vec::new(),
    }
}

fn body(frame: Option<&FrameObjects>) -> String {
    match objects_answer(frame).expect("answer builds") {
        ObjectsAnswer::Objects(body) => body,
        ObjectsAnswer::Unknown => panic!("expected an objects body"),
    }
}

#[test]
fn two_objects_parse_in_order() {
    let frame = frame_of(vec![object("7", true), object("8", false)]);
    let text = body(Some(&frame));
    let value: Value = serde_json::from_str(&text).expect("body parses");
    let objects = value["objects"].as_array().expect("objects array");
    assert_eq!(objects.len(), 2);
    let first = &objects[0];
    assert_eq!(first["id"].as_str().expect("id string"), "7");
    assert_eq!(first["label"].as_str().expect("label string"), "person");
    assert_eq!(first["score"].to_string(), "0.9");
    assert_eq!(first["confirmed"], Value::Bool(true));
    let edges = first["box"].as_array().expect("box array");
    assert_eq!(edges.len(), 4);
    for edge in edges {
        assert!(edge.is_number());
    }
    assert!(first["first_seen"].is_number());
    assert!(first["last_seen"].is_number());
    assert_eq!(first["zones_now"].as_array().expect("zones_now").len(), 1);
    assert!(first["zones_entered"].is_array());
    assert_eq!(first["state"].as_str().expect("state string"), "stationary");
    assert_eq!(objects[1]["id"].as_str().expect("second id"), "8");
    assert_eq!(
        objects[1]["state"].as_str().expect("second state"),
        "moving"
    );
}

#[test]
fn camera_b_ids_never_enter_camera_a_body() {
    let frame_a = frame_of(vec![object("a7", false)]);
    let frame_b = frame_of(vec![object("b7", false)]);
    let text_a = body(Some(&frame_a));
    assert!(text_a.contains("a7"));
    assert!(!text_a.contains("b7"));
    let text_b = body(Some(&frame_b));
    assert!(text_b.contains("b7"));
    assert!(!text_b.contains("a7"));
}

#[test]
fn no_frame_is_unknown_and_empty_frame_is_seen() {
    assert_eq!(
        objects_answer(None).expect("none builds"),
        ObjectsAnswer::Unknown
    );
    let empty = frame_of(Vec::new());
    assert_eq!(body(Some(&empty)), "{\"objects\":[]}");
    assert_ne!(
        objects_answer(Some(&empty)).expect("empty builds"),
        ObjectsAnswer::Unknown
    );
}

#[test]
fn entry_matches_the_record_frame_line() {
    let frame = frame_of(vec![object("7", true)]);
    let mut writer = RecordWriter::new(Vec::new(), &header()).expect("record header");
    writer.write_frame(&frame).expect("record frame");
    let line = String::from_utf8(writer.finish()).expect("utf8 line");
    let start = line.find("\"objects\":[").expect("objects start") + "\"objects\":[".len();
    let end = line.find("],\"ended\"").expect("objects end");
    assert!(start < end);
    assert_eq!(
        body(Some(&frame)),
        format!("{{\"objects\":[{}]}}", &line[start..end])
    );
}

#[test]
fn non_finite_score_is_an_error() {
    let mut nan_object = object("7", false);
    nan_object.score = f64::NAN;
    let frame = frame_of(vec![nan_object]);
    assert!(matches!(
        objects_answer(Some(&frame)),
        Err(RecordError::NonFinite(_))
    ));
}

const fn status(state: CameraState) -> CameraStatus {
    CameraStatus {
        state,
        counters: PipelineCounters {
            frames_in: 11,
            frames_tracked: 10,
            frames_inference_failed: 2,
        },
        source_gaps: 3,
        frames_dropped: 0,
    }
}

#[test]
fn status_entries_parse_with_exact_names_and_counters() {
    for (state, name) in [
        (CameraState::Running, "running"),
        (CameraState::WaitingForFrames, "waiting_for_frames"),
        (CameraState::InferenceUnavailable, "inference_unavailable"),
        (CameraState::Failed, "failed"),
    ] {
        assert_eq!(state_name(state), name);
        let parsed: Value =
            serde_json::from_str(&status_entry("yard", &status(state))).expect("status parses");
        assert_eq!(parsed["camera"].as_str().expect("camera string"), "yard");
        assert_eq!(parsed["state"].as_str().expect("state string"), name);
        assert_eq!(parsed["frames_in"].as_u64().expect("frames_in number"), 11);
        assert_eq!(
            parsed["frames_tracked"]
                .as_u64()
                .expect("frames_tracked number"),
            10
        );
        assert_eq!(
            parsed["frames_inference_failed"]
                .as_u64()
                .expect("frames_inference_failed number"),
            2
        );
        assert_eq!(
            parsed["source_gaps"].as_u64().expect("source_gaps number"),
            3
        );
    }
}

#[test]
fn off_entry_parses_with_off_state() {
    assert_eq!(OFF_STATE, "off");
    let parsed: Value = serde_json::from_str(&off_entry("living")).expect("off entry parses");
    assert_eq!(parsed["camera"].as_str().expect("camera string"), "living");
    assert_eq!(parsed["state"].as_str().expect("state string"), "off");
    assert_eq!(parsed["state"].as_str().expect("state string"), OFF_STATE);
}
