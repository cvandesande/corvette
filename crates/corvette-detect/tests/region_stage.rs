// Tests may unwrap, expect and index: `docs/standards/STYLE.md` states
// "Tests may unwrap, expect and index", and `clippy.toml` does not forbid them.
use corvette_detect::{RegionStage, RegionStageError, RegionStageSettings, RegionTensor};
use corvette_motion::{MotionSettings, Region, Shape, SizeCell, SizeGrid};

const WIDTH: usize = 160;
const HEIGHT: usize = 120;
const FRAME_BYTES: usize = WIDTH * HEIGHT * 3 / 2;
const MODEL_BYTES: usize = 64 * 64 * 3;
const SETTLE_FRAMES: usize = 40;

// The motion detector needs 17 identical frames before it stops reporting the picture as motion.
const QUIET_AFTER: usize = 17;

fn flat_frame() -> Vec<u8> {
    vec![128; FRAME_BYTES]
}

fn block_frame() -> Vec<u8> {
    let mut frame = flat_frame();
    for row in 30..90 {
        for column in 60..120 {
            frame[row * WIDTH + column] = 250;
        }
    }
    frame
}

const fn frame_shape() -> Shape {
    Shape::new(WIDTH, HEIGHT).expect("frame shape")
}

const fn model_shape() -> Shape {
    Shape::new(64, 64).expect("model shape")
}

fn settings(motion_enabled: bool, grid: SizeGrid) -> RegionStageSettings {
    RegionStageSettings {
        motion: MotionSettings::default(),
        floor: 32,
        grid,
        model: model_shape(),
        motion_enabled,
    }
}

fn stage(motion_enabled: bool) -> RegionStage {
    RegionStage::new(
        settings(motion_enabled, SizeGrid::empty()),
        frame_shape(),
        None,
    )
    .expect("stage for the frame shape")
}

fn grid_with_startup() -> SizeGrid {
    let mut grid = SizeGrid::empty();
    let left = SizeCell {
        samples: 3,
        mean: 0.25,
        std_dev: 0.05,
    };
    let right = SizeCell {
        samples: 1,
        mean: 0.1,
        std_dev: 0.02,
    };
    assert!(grid.set(0, 0, left));
    assert!(grid.set(3, 5, right));
    grid
}

fn startup_stage() -> RegionStage {
    RegionStage::new(settings(false, grid_with_startup()), frame_shape(), None)
        .expect("stage for the frame shape")
}

fn regions(output: &[RegionTensor]) -> Vec<Region> {
    output.iter().map(|entry| entry.region).collect()
}

const fn covers(region: Region, object: Region) -> bool {
    region.x1 <= object.x1
        && region.y1 <= object.y1
        && region.x2 >= object.x2
        && region.y2 >= object.y2
}

/// Feeds flat frames until the detector reports none of them as motion.
fn settle(stage: &mut RegionStage) {
    for _ in 0..SETTLE_FRAMES {
        stage.step(&flat_frame(), &[]).expect("flat frame");
    }
}

#[test]
fn wrong_buffer_length_is_reported_without_panicking() {
    let mut stage = stage(true);
    let empty = stage.step(&[], &[]);
    assert_eq!(
        empty,
        Err(RegionStageError::WrongFrameLength {
            expected: 28800,
            got: 0
        })
    );
    let short = stage.step(&[0u8; 100], &[]);
    assert_eq!(
        short,
        Err(RegionStageError::WrongFrameLength {
            expected: 28800,
            got: 100
        })
    );
}

#[test]
fn first_call_returns_the_builder_startup_regions() {
    let mut stage = startup_stage();
    let first = stage.step(&flat_frame(), &[]).expect("first call");
    assert_eq!(
        regions(&first),
        vec![
            Region {
                x1: 0,
                y1: 0,
                x2: 40,
                y2: 40
            },
            Region {
                x1: 54,
                y1: 66,
                x2: 86,
                y2: 98
            },
        ]
    );
    for entry in &first {
        assert_eq!(entry.tensor.len(), MODEL_BYTES);
    }
}

#[test]
fn startup_regions_appear_on_the_first_call_only() {
    let mut stage = startup_stage();
    let first = stage.step(&flat_frame(), &[]).expect("first call");
    assert_eq!(
        regions(&first).len(),
        2,
        "first call shows the startup scan"
    );
    let second = stage.step(&flat_frame(), &[]).expect("second call");
    assert_eq!(second, Vec::new(), "second call repeats no startup region");
}

#[test]
fn a_length_error_leaves_the_startup_regions_for_the_next_good_call() {
    let mut stage = startup_stage();
    let error = stage.step(&[0u8; 100], &[]);
    assert_eq!(
        error,
        Err(RegionStageError::WrongFrameLength {
            expected: 28800,
            got: 100
        })
    );
    let next = stage
        .step(&flat_frame(), &[])
        .expect("the frame after the length error");
    assert_eq!(
        regions(&next),
        vec![
            Region {
                x1: 0,
                y1: 0,
                x2: 40,
                y2: 40
            },
            Region {
                x1: 54,
                y1: 66,
                x2: 86,
                y2: 98
            },
        ]
    );
}

#[test]
fn identical_frames_quiet_the_motion_regions() {
    let mut stage = stage(true);
    for frame_index in 0..SETTLE_FRAMES {
        let output = stage.step(&flat_frame(), &[]).expect("flat frame");
        if frame_index >= QUIET_AFTER {
            assert!(
                output.is_empty(),
                "frame {frame_index} still reported regions"
            );
        }
    }
}

#[test]
fn a_bright_block_yields_a_region_covering_it() {
    let mut stage = stage(true);
    settle(&mut stage);
    let output = stage.step(&block_frame(), &[]).expect("block frame");
    assert_eq!(
        regions(&output),
        vec![Region {
            x1: 25,
            y1: 0,
            x2: 153,
            y2: 128
        }]
    );
    let block = Region {
        x1: 60,
        y1: 30,
        x2: 120,
        y2: 90,
    };
    assert!(regions(&output).iter().any(|region| covers(*region, block)));
    for entry in &output {
        assert_eq!(entry.tensor.len(), MODEL_BYTES);
    }
}

#[test]
fn an_object_box_yields_a_region_with_no_motion() {
    let mut stage = stage(true);
    settle(&mut stage);
    let object = Region {
        x1: 20,
        y1: 20,
        x2: 60,
        y2: 52,
    };
    let output = stage.step(&flat_frame(), &[object]).expect("object frame");
    assert_eq!(
        regions(&output),
        vec![Region {
            x1: 14,
            y1: 10,
            x2: 66,
            y2: 62
        }]
    );
    assert!(
        regions(&output)
            .iter()
            .any(|region| covers(*region, object))
    );
}

#[test]
fn motion_disabled_ignores_a_bright_block() {
    let mut stage = stage(false);
    settle(&mut stage);
    let output = stage.step(&block_frame(), &[]).expect("block frame");
    assert_eq!(output, Vec::new(), "a disabled motion feed shows no region");
}

#[test]
fn two_stages_on_the_same_frames_give_identical_output() {
    let mut first = stage(true);
    let mut second = stage(true);
    let mut history = Vec::new();
    for stage in [&mut first, &mut second] {
        let mut outputs = Vec::new();
        for _ in 0..SETTLE_FRAMES {
            outputs.push(stage.step(&flat_frame(), &[]).expect("flat frame"));
        }
        outputs.push(stage.step(&block_frame(), &[]).expect("block frame"));
        for _ in 0..5 {
            outputs.push(stage.step(&flat_frame(), &[]).expect("flat frame"));
        }
        history.push(outputs);
    }
    assert_eq!(history[0], history[1], "the two stages diverged");
    assert!(
        history[0].iter().any(|output| !output.is_empty()),
        "the shared sequence produced nothing to compare"
    );
}
