use std::path::PathBuf;
use std::process::Output;

const WIDTH: usize = 160;
const HEIGHT: usize = 120;
const QUIET: u8 = 20;
const BRIGHT: u8 = 255;
const BLOCK: (usize, usize, usize, usize) = (60, 40, 99, 79);
const SECOND_BLOCK: (usize, usize, usize, usize) = (10, 40, 49, 79);
const BIG_BLOCK: (usize, usize, usize, usize) = (10, 20, 109, 99);
const BLOCK_MASK: &str = "0.25,0.25,0.75,0.25,0.75,0.75,0.25,0.75";

fn run(args: &[&str]) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_corvette-detect-tools"))
        .args(args)
        .output()
        .expect("the binary runs")
}

fn code(output: &Output) -> i32 {
    output.status.code().expect("the exit code is set")
}

fn stdout_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn motion_args<'a>(luma: &'a str, out: &'a str) -> Vec<&'a str> {
    vec![
        "motion-detect",
        "--luma",
        luma,
        "--width",
        "160",
        "--height",
        "120",
        "--out",
        out,
    ]
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "corvette-detect-motion-{label}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("a temporary directory");
        Self { path }
    }

    fn write(&self, name: &str, bytes: &[u8]) -> String {
        let path = self.path.join(name);
        std::fs::write(&path, bytes).expect("a test file");
        path.display().to_string()
    }

    fn path_str(&self, name: &str) -> String {
        self.path.join(name).display().to_string()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn flat(count: usize, level: u8) -> Vec<u8> {
    vec![level; WIDTH * HEIGHT * count]
}

fn draw(picture: &mut [u8], block: (usize, usize, usize, usize)) {
    for y in block.1..=block.3 {
        for x in block.0..=block.2 {
            picture[y * WIDTH + x] = BRIGHT;
        }
    }
}

fn blocked(quiet: usize, moving: usize, second: Option<(usize, usize, usize, usize)>) -> Vec<u8> {
    let mut bytes = flat(quiet + moving, QUIET);
    for index in 0..moving {
        let start = (quiet + index) * WIDTH * HEIGHT;
        let end = start + WIDTH * HEIGHT;
        draw(&mut bytes[start..end], BLOCK);
        if let Some(extra) = second {
            draw(&mut bytes[start..end], extra);
        }
    }
    bytes
}

fn sequence(
    quiet: usize,
    moving: usize,
    background: u8,
    block: (usize, usize, usize, usize),
    level: u8,
) -> Vec<u8> {
    let mut bytes = vec![background; WIDTH * HEIGHT * (quiet + moving)];
    for index in 0..moving {
        let start = (quiet + index) * WIDTH * HEIGHT;
        for y in block.1..=block.3 {
            for x in block.0..=block.2 {
                bytes[start + y * WIDTH + x] = level;
            }
        }
    }
    bytes
}

struct Line {
    index: usize,
    calibrating: bool,
    boxes: Vec<(usize, usize, usize, usize)>,
}

fn parse_output(text: &str) -> Vec<Line> {
    text.lines()
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            assert_eq!(fields.len(), 3, "three fields in {line:?}");
            let boxes = if fields[2] == "-" {
                Vec::new()
            } else {
                fields[2]
                    .split(';')
                    .map(|one| {
                        let parts: Vec<usize> = one
                            .split(',')
                            .map(|part| part.parse::<usize>().expect("a decimal coordinate"))
                            .collect();
                        assert_eq!(parts.len(), 4, "four numbers per box in {line:?}");
                        (parts[0], parts[1], parts[2], parts[3])
                    })
                    .collect()
            };
            Line {
                index: fields[0].parse().expect("a decimal index"),
                calibrating: match fields[1] {
                    "1" => true,
                    "0" => false,
                    other => panic!("unknown calibrating field {other:?}"),
                },
                boxes,
            }
        })
        .collect()
}

const fn touches(
    box_edges: (usize, usize, usize, usize),
    block: (usize, usize, usize, usize),
) -> bool {
    box_edges.0 <= block.2
        && box_edges.2 >= block.0
        && box_edges.1 <= block.3
        && box_edges.3 >= block.1
}

fn box_touching(lines: &[Line], block: (usize, usize, usize, usize)) -> usize {
    lines
        .iter()
        .filter(|line| line.boxes.iter().any(|one| touches(*one, block)))
        .count()
}

fn box_free(lines: &[Line]) -> bool {
    lines.iter().all(|line| line.boxes.is_empty())
}

fn box_rows_in(lines: &[Line], low: usize, high: usize) -> usize {
    lines
        .iter()
        .filter(|line| (low..=high).contains(&line.index) && !line.boxes.is_empty())
        .count()
}

fn last_box_row(lines: &[Line]) -> Option<usize> {
    lines
        .iter()
        .rev()
        .find(|line| !line.boxes.is_empty())
        .map(|line| line.index)
}

fn run_to_file(args: &[&str], out: &str) -> Vec<Line> {
    let output = run(args);
    assert_eq!(code(&output), 0, "{}", stderr_text(&output));
    parse_output(&std::fs::read_to_string(out).expect("the output file"))
}

#[test]
fn static_pictures_write_a_three_field_box_free_line_for_each_picture() {
    let dir = TempDir::new("static");
    let luma = dir.write("in.y", &flat(40, QUIET));
    let out = dir.path_str("out.tsv");
    let output = run(&motion_args(&luma, &out));
    assert_eq!(code(&output), 0, "{}", stderr_text(&output));
    assert_eq!(
        stdout_text(&output),
        "pictures 40, with boxes 0, calibrating 0\n"
    );
    let lines = parse_output(&std::fs::read_to_string(out).expect("the output file"));
    assert_eq!(lines.len(), 40);
    assert!(
        lines
            .iter()
            .enumerate()
            .all(|(position, line)| line.index == position)
    );
    assert!(box_free(&lines), "a static picture drew a box");
}

#[test]
fn a_block_appearing_at_picture_twenty_draws_a_box_at_its_place() {
    let dir = TempDir::new("block");
    let luma = dir.write("in.y", &blocked(20, 20, None));
    let out = dir.path_str("out.tsv");
    let lines = run_to_file(&motion_args(&luma, &out), &out);
    assert_eq!(lines.len(), 40);
    assert!(box_free(&lines[..20]), "a box appeared before picture 20");
    assert!(
        box_touching(&lines[20..], BLOCK) >= 1,
        "no box at the block's place from picture 20 on"
    );
}

#[test]
fn two_separated_blocks_draw_two_boxes_on_one_line() {
    let dir = TempDir::new("twoblocks");
    let luma = dir.write("in.y", &blocked(20, 20, Some(SECOND_BLOCK)));
    let out = dir.path_str("out.tsv");
    let lines = run_to_file(&motion_args(&luma, &out), &out);
    assert!(
        lines.iter().any(|line| line.boxes.len() >= 2),
        "no line carried two boxes"
    );
}

#[test]
fn a_mask_over_the_block_suppresses_its_box_where_the_unmasked_run_shows_one() {
    let dir = TempDir::new("mask");
    let luma = dir.write("in.y", &blocked(20, 20, None));
    let masked = dir.path_str("masked.tsv");
    let plain = dir.path_str("plain.tsv");
    let mut masked_args = motion_args(&luma, &masked);
    masked_args.extend(["--mask", BLOCK_MASK]);
    let masked_lines = run_to_file(&masked_args, &masked);
    let plain_lines = run_to_file(&motion_args(&luma, &plain), &plain);
    assert!(box_free(&masked_lines), "the mask let a box through");
    assert!(
        box_touching(&plain_lines, BLOCK) >= 1,
        "the unmasked run drew no box at the block's place"
    );
}

#[test]
fn two_runs_on_the_same_luma_file_write_identical_output() {
    let dir = TempDir::new("determinism");
    let luma = dir.write("in.y", &blocked(20, 20, None));
    let mut contents: Vec<Vec<u8>> = Vec::new();
    for name in ["first.tsv", "second.tsv"] {
        let out = dir.path_str(name);
        let output = run(&motion_args(&luma, &out));
        assert_eq!(code(&output), 0, "{}", stderr_text(&output));
        contents.push(std::fs::read(out).expect("an output file"));
    }
    assert!(!contents[0].is_empty());
    assert_eq!(contents[0], contents[1], "two runs wrote different bytes");
}

#[test]
fn the_first_line_reports_calibration_and_a_later_line_reports_none() {
    let dir = TempDir::new("calibration");
    let luma = dir.write("in.y", &flat(40, 100));
    let out = dir.path_str("out.tsv");
    let lines = run_to_file(&motion_args(&luma, &out), &out);
    assert!(
        lines
            .iter()
            .enumerate()
            .all(|(position, line)| line.index == position)
    );
    assert!(
        lines[0].calibrating,
        "the first line does not report calibration"
    );
    assert!(
        lines[1..].iter().any(|line| !line.calibrating),
        "calibration never ended"
    );
}

#[test]
fn bad_usage_exits_2() {
    let dir = TempDir::new("usage");
    let luma = dir.write("in.y", &flat(1, QUIET));
    for tail in [
        &["--bogus", "x"][..],
        &["--threshold", "nan"],
        &["--threshold", "256"],
        &["--improve-contrast", "2"],
        &["--frame-alpha", "inf"],
        &["--mask", "0.1,0.2,0.3,0.4,0.5"],
    ] {
        let mut args = motion_args(&luma, "out.tsv");
        args.extend_from_slice(tail);
        let output = run(&args);
        assert_eq!(code(&output), 2, "{tail:?}");
        assert!(stdout_text(&output).is_empty(), "{tail:?}");
        assert!(stderr_text(&output).contains("motion-detect"), "{tail:?}");
    }
    for missing in [
        vec![
            "motion-detect",
            "--width",
            "160",
            "--height",
            "120",
            "--out",
            "out.tsv",
        ],
        vec![
            "motion-detect",
            "--luma",
            luma.as_str(),
            "--height",
            "120",
            "--out",
            "out.tsv",
        ],
        vec![
            "motion-detect",
            "--luma",
            luma.as_str(),
            "--width",
            "160",
            "--height",
            "120",
        ],
    ] {
        assert_eq!(code(&run(&missing)), 2, "{missing:?}");
    }
}

#[test]
fn bad_input_exits_3() {
    let dir = TempDir::new("input");
    let odd = dir.write("odd.y", b"123456");
    let empty = dir.write("empty.y", b"");
    let one = dir.write("one.y", &flat(1, QUIET));
    let absent = dir.path_str("absent.y");
    let out = dir.path_str("out.tsv");
    let small = run(&[
        "motion-detect",
        "--luma",
        &odd,
        "--width",
        "2",
        "--height",
        "2",
        "--out",
        &out,
    ]);
    assert_eq!(code(&small), 3, "a partial last picture");
    let message = stderr_text(&small);
    assert!(
        message.contains("6 bytes") && message.contains("4 bytes"),
        "{message}"
    );
    assert_eq!(
        code(&run(&motion_args(&empty, &out))),
        3,
        "an empty luma file"
    );
    assert_eq!(
        code(&run(&motion_args(&absent, &out))),
        3,
        "a missing luma file"
    );
    let mut mask_args = motion_args(&one, &out);
    mask_args.extend(["--mask", "0.5,0.5,1.5,0.5,0.5,1.5"]);
    let mask_run = run(&mask_args);
    assert_eq!(code(&mask_run), 3, "a mask coordinate outside 0 to 1");
    assert!(
        stderr_text(&mask_run).contains("--mask"),
        "the mask refusal must name --mask"
    );
    let no_dir = run(&motion_args(&one, &dir.path_str("nodir/out.tsv")));
    assert_eq!(code(&no_dir), 3, "an output directory that does not exist");
}

#[test]
fn threshold_above_the_block_contrast_hides_every_block_box() {
    let dir = TempDir::new("threshold-flag");
    let luma = dir.write("in.y", &sequence(20, 20, 20, BLOCK, 70));
    let at_default = dir.path_str("default.tsv");
    let raised = dir.path_str("raised.tsv");
    let mut default_run = motion_args(&luma, &at_default);
    default_run.extend(["--improve-contrast", "0"]);
    let mut high_run = motion_args(&luma, &raised);
    high_run.extend(["--improve-contrast", "0", "--threshold", "100"]);
    let with_default = run_to_file(&default_run, &at_default);
    let with_flag = run_to_file(&high_run, &raised);
    assert!(
        box_touching(&with_default[20..], BLOCK) >= 1,
        "the default threshold hid the block"
    );
    assert!(
        box_free(&with_flag),
        "the raised threshold still drew a box"
    );
}

#[test]
fn contour_area_above_the_block_area_hides_its_box() {
    let dir = TempDir::new("contour-area-flag");
    let luma = dir.write("in.y", &blocked(20, 20, None));
    let at_default = dir.path_str("default.tsv");
    let raised = dir.path_str("raised.tsv");
    let mut default_run = motion_args(&luma, &at_default);
    default_run.extend(["--improve-contrast", "0"]);
    let mut large_run = motion_args(&luma, &raised);
    large_run.extend(["--improve-contrast", "0", "--contour-area", "1500"]);
    let with_default = run_to_file(&default_run, &at_default);
    let with_flag = run_to_file(&large_run, &raised);
    assert!(
        box_touching(&with_default[20..], BLOCK) >= 1,
        "the default area hid the block"
    );
    assert!(
        box_free(&with_flag),
        "an area above the block still drew a box"
    );
}

#[test]
fn the_contrast_stretch_makes_a_low_contrast_block_visible() {
    let dir = TempDir::new("improve-contrast-flag");
    let luma = dir.write("in.y", &sequence(20, 70, 230, BLOCK, 234));
    let off = dir.path_str("off.tsv");
    let on = dir.path_str("on.tsv");
    let mut off_run = motion_args(&luma, &off);
    off_run.extend(["--improve-contrast", "0"]);
    let without = run_to_file(&off_run, &off);
    let with = run_to_file(&motion_args(&luma, &on), &on);
    assert_eq!(
        box_rows_in(&without, 70, 85),
        0,
        "without the stretch a late box appeared"
    );
    assert!(
        box_rows_in(&with, 70, 85) >= 1,
        "the stretch never made the block visible"
    );
}

#[test]
fn a_low_lightning_threshold_keeps_calibration_while_the_motion_persists() {
    let dir = TempDir::new("lightning-flag");
    let luma = dir.write("in.y", &sequence(20, 20, 20, BIG_BLOCK, 255));
    let at_default = dir.path_str("default.tsv");
    let lowered = dir.path_str("lowered.tsv");
    let mut default_run = motion_args(&luma, &at_default);
    default_run.extend(["--improve-contrast", "0"]);
    let mut low_run = motion_args(&luma, &lowered);
    low_run.extend(["--improve-contrast", "0", "--lightning-threshold", "0.05"]);
    let with_default = run_to_file(&default_run, &at_default);
    let with_flag = run_to_file(&low_run, &lowered);
    assert!(
        !with_default[20..].iter().any(|line| line.calibrating),
        "the default lightning calibrated on late pictures"
    );
    assert!(
        with_flag[20..].iter().any(|line| line.calibrating),
        "the low lightning never entered calibration"
    );
}

#[test]
fn a_larger_frame_alpha_absorbs_a_standing_block_sooner() {
    let dir = TempDir::new("frame-alpha-flag");
    let luma = dir.write("in.y", &blocked(20, 20, None));
    let at_default = dir.path_str("default.tsv");
    let raised = dir.path_str("raised.tsv");
    let mut default_run = motion_args(&luma, &at_default);
    default_run.extend(["--improve-contrast", "0"]);
    let mut quick_run = motion_args(&luma, &raised);
    quick_run.extend(["--improve-contrast", "0", "--frame-alpha", "0.2"]);
    let with_default = run_to_file(&default_run, &at_default);
    let with_flag = run_to_file(&quick_run, &raised);
    assert_eq!(
        last_box_row(&with_default),
        Some(39),
        "the default alpha gave up the block early"
    );
    let last = last_box_row(&with_flag).expect("the raised-alpha run kept some boxes");
    assert!(
        last < 39,
        "the larger frame alpha did not stop the boxes sooner"
    );
}

#[test]
fn a_different_frame_height_gives_different_boxes_for_one_input() {
    let dir = TempDir::new("frame-height-flag");
    let luma = dir.write("in.y", &blocked(20, 20, None));
    let lower = dir.path_str("lower.tsv");
    let higher = dir.path_str("higher.tsv");
    let mut low_run = motion_args(&luma, &lower);
    low_run.extend(["--improve-contrast", "0", "--frame-height", "50"]);
    let mut high_run = motion_args(&luma, &higher);
    high_run.extend(["--improve-contrast", "0", "--frame-height", "80"]);
    let with_low = run_to_file(&low_run, &lower);
    let with_high = run_to_file(&high_run, &higher);
    assert_eq!(
        last_box_row(&with_low),
        Some(39),
        "the lower height lost the last box"
    );
    assert_eq!(
        last_box_row(&with_high),
        Some(39),
        "the higher height lost the last box"
    );
    assert_ne!(
        with_low.last().expect("lines").boxes,
        with_high.last().expect("lines").boxes,
        "one frame height gave the other height's boxes"
    );
}
