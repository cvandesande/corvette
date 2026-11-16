use std::path::PathBuf;
use std::process::Output;
const WIDTH: usize = 8;
const HEIGHT: usize = 6;
const GREY: u8 = 100;
const RED: [u8; 3] = [255, 0, 0];
const PPM_MAX_VALUE: u8 = 255;
fn run(args: &[&str]) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_corvette-detect-tools"))
        .args(args)
        .output()
        .expect("the binary runs")
}
fn stderr_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}
struct TempDir {
    path: PathBuf,
}
impl TempDir {
    fn new(label: &str) -> Self {
        let name = format!("corvette-detect-annotate-{label}-{}", std::process::id());
        let path = std::env::temp_dir().join(name);
        std::fs::create_dir_all(&path).expect("a temporary directory");
        Self { path }
    }
    fn write(&self, name: &str, bytes: &[u8]) -> String {
        let path = self.path.join(name);
        std::fs::write(&path, bytes).expect("a test file");
        path.display().to_string()
    }
    fn dir(&self, name: &str) -> String {
        let path = self.path.join(name);
        std::fs::create_dir_all(&path).expect("a directory");
        path.display().to_string()
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
fn annotate_args<'a>(luma: &'a str, boxes: &'a str, frames: &'a str, out: &'a str) -> Vec<&'a str> {
    let mut args = vec!["annotate", "--luma", luma, "--width", "8", "--height", "6"];
    args.extend_from_slice(&["--boxes", boxes, "--frames", frames, "--out", out]);
    args
}
fn header() -> Vec<u8> {
    format!("P{}\n{WIDTH} {HEIGHT}\n{PPM_MAX_VALUE}\n", 6).into_bytes()
}
fn frame_bytes(out: &str, number: usize) -> Vec<u8> {
    std::fs::read(format!("{out}/frame-{number}.ppm")).expect("a written picture")
}
fn pixel(bytes: &[u8], x: usize, y: usize) -> [u8; 3] {
    let start = header().len() + (y * WIDTH + x) * 3;
    [bytes[start], bytes[start + 1], bytes[start + 2]]
}
#[test]
fn one_box_outlines_exactly_the_expected_pixels_on_a_known_picture() {
    let dir = TempDir::new("one-box");
    let luma = dir.write("in.y", &[GREY; WIDTH * HEIGHT]);
    let boxes = dir.write("boxes.tsv", b"0\t0\t2,1,6,5\n");
    let out = dir.dir("out");
    let output = run(&annotate_args(&luma, &boxes, "0", &out));
    assert_eq!(output.status.code(), Some(0), "{}", stderr_text(&output));
    let summary = String::from_utf8_lossy(&output.stdout);
    assert_eq!(summary, "frames 1, boxes drawn 1\n");
    let bytes = frame_bytes(&out, 0);
    assert_eq!(&bytes[..2], &[80, 54], "the PPM magic");
    assert_eq!(
        &bytes[2..header().len()],
        format!("\n{WIDTH} {HEIGHT}\n{PPM_MAX_VALUE}\n").as_bytes(),
        "the rest of the header"
    );
    assert_eq!(&bytes[..header().len()], header().as_slice());
    assert_eq!(bytes.len(), header().len() + WIDTH * HEIGHT * 3);
    let expected: std::collections::HashSet<(usize, usize)> = (2..6)
        .flat_map(|x| [(x, 1), (x, 4)])
        .chain((1..5).flat_map(|y| [(2, y), (5, y)]))
        .collect();
    assert_eq!(expected.len(), 12, "the outline rule gives twelve pixels");
    for corner in [(2, 1), (5, 1), (2, 4), (5, 4)] {
        assert_eq!(pixel(&bytes, corner.0, corner.1), RED, "corner {corner:?}");
    }
    assert_eq!(pixel(&bytes, 3, 2), [GREY; 3], "inside the box");
    assert_eq!(pixel(&bytes, 6, 1), [GREY; 3], "right of the box");
    assert_eq!(pixel(&bytes, 1, 1), [GREY; 3], "left of the box");
    let body = &bytes[header().len()..];
    let red = body.chunks(3).filter(|p| p[0] != GREY).count();
    assert_eq!(red, 12, "only the twelve outline pixels are not grey");
}
#[test]
fn edge_degenerate_overlapping_and_dash_boxes_follow_the_draw_rule() {
    let dir = TempDir::new("draw-rule");
    let luma = dir.write("in.y", &[GREY; WIDTH * HEIGHT * 3]);
    let boxes = dir.write(
        "b.tsv",
        "0\t0\t4,3,8,6;3,3,3,4\n1\t0\t2,1,6,5;4,3,7,6\n2\t0\t-\n".as_bytes(),
    );
    let out = dir.dir("out");
    let output = run(&annotate_args(&luma, &boxes, "0,1,2", &out));
    assert_eq!(output.status.code(), Some(0), "{}", stderr_text(&output));
    let edge = frame_bytes(&out, 0);
    assert_eq!(edge.len(), header().len() + WIDTH * HEIGHT * 3);
    for x in 4..WIDTH {
        assert_eq!(pixel(&edge, x, 3), RED, "top row to the edge");
        assert_eq!(pixel(&edge, x, 5), RED, "bottom row at the last row");
    }
    for y in 3..HEIGHT {
        assert_eq!(pixel(&edge, 4, y), RED, "left column");
        assert_eq!(pixel(&edge, 7, y), RED, "right column at the last column");
    }
    assert_eq!(pixel(&edge, 3, 3), [GREY; 3], "equal x1 and x2 drew");
    let overlap = frame_bytes(&out, 1);
    assert_eq!(pixel(&overlap, 2, 1), RED, "first box alone");
    assert_eq!(pixel(&overlap, 6, 5), RED, "second box alone");
    assert_eq!(pixel(&overlap, 4, 3), RED, "where the boxes meet");
    let dashed = frame_bytes(&out, 2);
    let plain = dashed[header().len()..].iter().all(|&value| value == GREY);
    assert!(plain, "a dash line drew a box");
}
#[test]
fn listed_pictures_carry_their_own_grey_level_and_a_repeated_number_writes_one_file() {
    let dir = TempDir::new("several");
    let mut luma_bytes = Vec::new();
    for level in 1_u8..=5 {
        luma_bytes.extend_from_slice(&[level * 10; WIDTH * HEIGHT]);
    }
    let luma = dir.write("in.y", &luma_bytes);
    let dashes = b"0\t0\t-\n1\t0\t-\n2\t0\t-\n3\t0\t-\n4\t0\t-\n";
    let boxes = dir.write("boxes.tsv", dashes);
    let out = dir.dir("out");
    let output = run(&annotate_args(&luma, &boxes, "3,1", &out));
    assert_eq!(output.status.code(), Some(0), "{}", stderr_text(&output));
    let count = |path: &str| std::fs::read_dir(path).expect("a directory").count();
    assert_eq!(count(&out), 2, "frame-3 and frame-1 only");
    for (number, level) in [(3, 40_u8), (1, 20_u8)] {
        let bytes = frame_bytes(&out, number);
        let plain = bytes[header().len()..].iter().all(|&value| value == level);
        assert!(plain, "frame {number} does not carry its own grey level");
    }
    let again = dir.dir("again");
    let repeat = run(&annotate_args(&luma, &boxes, "1,1", &again));
    assert_eq!(repeat.status.code(), Some(0));
    assert_eq!(count(&again), 1, "a repeated number wrote two files");
}
#[test]
fn usage_errors_exit_2_and_input_errors_exit_3_naming_their_cause() {
    let dir = TempDir::new("errors");
    let luma = dir.write("one.y", &[GREY; WIDTH * HEIGHT]);
    let three = dir.write("three.y", &[GREY; WIDTH * HEIGHT * 3]);
    let odd = dir.write("odd.y", b"12345");
    let empty = dir.write("empty.y", b"");
    let out = dir.dir("out");
    let good = dir.write("good.tsv", b"0\t0\t-\n");
    let two_fields = dir.write("two.tsv", b"0\t0\t-\nbroken\n");
    let short_group = dir.write("short.tsv", b"0\t0\t1,2,3\n");
    let absent = dir.path.join("absent.y").display().to_string();
    let no_dir = dir.path.join("nodir").display().to_string();
    let mut cases = Vec::new();
    for (index, bad) in [(4, "0"), (10, "x"), (10, "1,,2")] {
        let mut args = annotate_args(&luma, &good, "0", &out);
        args[index] = bad;
        cases.push((args, 2, "annotate"));
    }
    let mut no_boxes = annotate_args(&luma, &good, "0", &out);
    no_boxes.truncate(7);
    cases.push((no_boxes, 2, "usage"));
    let mut extra = annotate_args(&luma, &good, "0", &out);
    extra.extend_from_slice(&["--bogus", "x"]);
    cases.push((extra, 2, "usage"));
    cases.extend([
        (annotate_args(&odd, &good, "0", &out), 3, "5 bytes"),
        (annotate_args(&odd, &good, "0", &out), 3, "48 bytes"),
        (annotate_args(&empty, &good, "0", &out), 3, "0 bytes"),
        (annotate_args(&three, &good, "3", &out), 3, "beyond 3"),
        (annotate_args(&absent, &good, "0", &out), 3, "cannot read"),
        (
            annotate_args(&three, &good, "2", &out),
            3,
            "no line for picture 2",
        ),
        (annotate_args(&luma, &two_fields, "0", &out), 3, "line 2"),
        (annotate_args(&luma, &short_group, "0", &out), 3, "1,2,3"),
        (annotate_args(&luma, &good, "0", &no_dir), 3, "nodir"),
    ]);
    for (args, code, needle) in cases {
        let output = run(&args);
        let err = stderr_text(&output);
        assert_eq!(output.status.code(), Some(code), "{needle}: {err}");
        assert!(stderr_text(&output).contains(needle), "missing {needle}");
    }
}
#[test]
fn a_listed_picture_without_a_box_line_writes_no_file() {
    let dir = TempDir::new("missing-line");
    let luma = dir.write("three.y", &[GREY; WIDTH * HEIGHT * 3]);
    let boxes = dir.write("boxes.tsv", b"0\t0\t-\n1\t0\t-\n");
    for label in ["first", "second"] {
        let out = dir.dir(label);
        let output = run(&annotate_args(&luma, &boxes, "0,1,2", &out));
        assert_eq!(output.status.code(), Some(3), "{}", stderr_text(&output));
        assert!(
            stderr_text(&output).contains("no line for picture 2"),
            "run {label}: {}",
            stderr_text(&output)
        );
        let count = std::fs::read_dir(&out).expect("a directory").count();
        assert_eq!(count, 0, "run {label} left {count} files behind");
    }
}
#[test]
fn boxes_passing_the_picture_edge_draw_only_the_part_inside() {
    let dir = TempDir::new("edge-boxes");
    let luma = dir.write("three.y", &[GREY; WIDTH * HEIGHT * 3]);
    let boxes = dir.write(
        "b.tsv",
        "0\t0\t5,1,9,4\n1\t0\t8,0,12,4\n2\t0\t1,3,4,9\n".as_bytes(),
    );
    let out = dir.dir("out");
    let output = run(&annotate_args(&luma, &boxes, "0,1,2", &out));
    assert_eq!(output.status.code(), Some(0), "{}", stderr_text(&output));
    let expected: [Vec<(usize, usize)>; 3] = [
        vec![(5, 1), (5, 2), (5, 3), (6, 1), (6, 3), (7, 1), (7, 3)],
        vec![],
        vec![(1, 3), (1, 4), (1, 5), (2, 3), (3, 3), (3, 4), (3, 5)],
    ];
    for (number, want) in expected.iter().enumerate() {
        let bytes = frame_bytes(&out, number);
        let mut red: Vec<(usize, usize)> = Vec::new();
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let value = pixel(&bytes, x, y);
                if value == RED {
                    red.push((x, y));
                } else {
                    assert_eq!(value, [GREY; 3], "picture {number} at ({x}, {y})");
                }
            }
        }
        red.sort_unstable();
        assert_eq!(red, *want, "picture {number} red pixels");
    }
}
#[test]
fn a_later_line_for_the_same_picture_replaces_an_earlier_one() {
    let dir = TempDir::new("later-line");
    let luma = dir.write("one.y", &[GREY; WIDTH * HEIGHT]);
    let dash_then_box = dir.write("dash_first.tsv", b"0\t0\t-\n0\t0\t2,1,6,5\n");
    let box_then_dash = dir.write("box_first.tsv", b"0\t0\t2,1,6,5\n0\t0\t-\n");
    let out = dir.dir("outline_wins");
    let output = run(&annotate_args(&luma, &dash_then_box, "0", &out));
    assert_eq!(output.status.code(), Some(0), "{}", stderr_text(&output));
    let bytes = frame_bytes(&out, 0);
    let outline = bytes[header().len()..]
        .chunks(3)
        .filter(|p| p[0] != GREY)
        .count();
    assert_eq!(outline, 12, "the later box line did not replace the dash");
    let out2 = dir.dir("dash_wins");
    let output2 = run(&annotate_args(&luma, &box_then_dash, "0", &out2));
    assert_eq!(output2.status.code(), Some(0), "{}", stderr_text(&output2));
    let bytes2 = frame_bytes(&out2, 0);
    let plain = bytes2[header().len()..]
        .chunks(3)
        .filter(|p| p[0] != GREY)
        .count();
    assert_eq!(plain, 0, "the later dash line did not replace the box");
}
