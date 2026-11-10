use std::fs;
use std::os::unix::fs::PermissionsExt;

use corvette_detect::{Decode, FrameSpec, Input, MAX_FRAME_BYTES, SpecError, decode_command};

const LIVE_ARGS: [&str; 17] = [
    "-hide_banner",
    "-loglevel",
    "error",
    "-nostdin",
    "-rtsp_transport",
    "tcp",
    "-i",
    "rtsp://camera.test/front_sub",
    "-r",
    "5",
    "-vf",
    "fps=5,scale=704:576,eq=gamma=1.4:gamma_weight=0.5",
    "-f",
    "rawvideo",
    "-pix_fmt",
    "yuv420p",
    "pipe:1",
];

fn args(command: &tokio::process::Command) -> Vec<String> {
    command
        .as_std()
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn a_live_input_builds_the_cpu_detect_command() {
    let spec = FrameSpec::new(704, 576, 5, Decode::Cpu).expect("valid spec");
    let input = Input::Live {
        url: "rtsp://camera.test/front_sub".to_string(),
    };
    let command = decode_command("ffmpeg", &input, spec);
    assert_eq!(command.as_std().get_program(), "ffmpeg");
    assert_eq!(args(&command), LIVE_ARGS);
}

#[test]
fn a_segment_input_builds_the_same_command_without_the_transport() {
    let spec = FrameSpec::new(640, 480, 10, Decode::Cpu).expect("valid spec");
    let input = Input::Segment {
        path: "/data/segment.ts".into(),
    };
    let command = decode_command("ffmpeg", &input, spec);
    assert_eq!(
        args(&command),
        [
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-i",
            "/data/segment.ts",
            "-r",
            "10",
            "-vf",
            "fps=10,scale=640:480,eq=gamma=1.4:gamma_weight=0.5",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "yuv420p",
            "pipe:1",
        ]
    );
}

#[test]
fn a_frame_spec_refuses_a_zero_or_odd_size_and_a_zero_rate() {
    assert_eq!(FrameSpec::new(0, 576, 5, Decode::Cpu), Err(SpecError::Size));
    assert_eq!(FrameSpec::new(704, 0, 5, Decode::Cpu), Err(SpecError::Size));
    assert_eq!(
        FrameSpec::new(703, 576, 5, Decode::Cpu),
        Err(SpecError::Size)
    );
    assert_eq!(
        FrameSpec::new(704, 575, 5, Decode::Cpu),
        Err(SpecError::Size)
    );
    assert_eq!(
        FrameSpec::new(704, 576, 0, Decode::Cpu),
        Err(SpecError::Rate)
    );
    assert_eq!(FrameSpec::new(0, 0, 0, Decode::Cpu), Err(SpecError::Size));
    let spec = FrameSpec::new(704, 576, 5, Decode::Cpu).expect("valid spec");
    assert_eq!(spec.width(), 704);
    assert_eq!(spec.height(), 576);
    assert_eq!(spec.fps(), 5);
}

#[test]
fn the_error_messages_name_the_cause() {
    assert_eq!(
        SpecError::Size.to_string(),
        "frame size must be non-zero and even in both directions"
    );
    assert_eq!(SpecError::Rate.to_string(), "frame rate must be non-zero");
    assert_eq!(
        SpecError::TooLarge.to_string(),
        "frame size is over the 67108864 byte limit"
    );
}

#[test]
fn a_frame_spec_reports_its_frame_size_and_refuses_one_over_the_limit() {
    assert_eq!(
        FrameSpec::new(704, 576, 5, Decode::Cpu)
            .expect("valid spec")
            .frame_bytes(),
        608_256
    );
    assert_eq!(
        FrameSpec::new(640, 480, 5, Decode::Cpu)
            .expect("valid spec")
            .frame_bytes(),
        460_800
    );
    assert_eq!(
        FrameSpec::new(4, 2, 5, Decode::Cpu)
            .expect("valid spec")
            .frame_bytes(),
        12
    );
    assert!(FrameSpec::new(7680, 4320, 5, Decode::Cpu).is_ok());
    assert_eq!(
        FrameSpec::new(8192, 8192, 5, Decode::Cpu),
        Err(SpecError::TooLarge)
    );
    assert_eq!(
        FrameSpec::new(4_294_967_294, 4_294_967_294, 5, Decode::Cpu),
        Err(SpecError::TooLarge)
    );
    assert_eq!(
        FrameSpec::new(0, 8192, 5, Decode::Cpu),
        Err(SpecError::Size)
    );
    assert_eq!(
        FrameSpec::new(8192, 8192, 0, Decode::Cpu),
        Err(SpecError::Rate)
    );
    assert_eq!(MAX_FRAME_BYTES, 67_108_864);
}

#[tokio::test]
async fn the_child_has_its_standard_output_piped_and_its_standard_input_null() {
    let dir = std::env::temp_dir().join(format!(
        "the_child_has_its_standard_output_piped_and_its_standard_input_null_{}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("scratch directory");
    let fake = dir.join("fake-ffmpeg");
    fs::write(
        &fake,
        "#!/bin/sh\nprintf '%s\\n' \"$@\"\nreadlink /proc/self/fd/0\n",
    )
    .expect("fake ffmpeg written");
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).expect("fake ffmpeg executable");

    let spec = FrameSpec::new(704, 576, 5, Decode::Cpu).expect("valid spec");
    let input = Input::Live {
        url: "rtsp://camera.test/front_sub".to_string(),
    };
    let mut command = decode_command(&fake, &input, spec);
    let output = command
        .spawn()
        .expect("fake ffmpeg spawns")
        .wait_with_output()
        .await
        .expect("fake ffmpeg finishes");
    let lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_owned)
        .collect();
    let expected: Vec<String> = LIVE_ARGS
        .iter()
        .map(|arg| (*arg).to_string())
        .chain(std::iter::once("/dev/null".to_string()))
        .collect();
    let status = output.status;
    let removed = fs::remove_dir_all(&dir);
    assert!(status.success(), "fake ffmpeg exited with {status}");
    assert_eq!(lines, expected);
    removed.expect("scratch directory removed");
}
