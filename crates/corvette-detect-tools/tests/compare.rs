use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

const VALUES: usize = 50 * 8_400;

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

fn directory(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(tag);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("the directory is created");
    dir
}

fn text_of(path: &Path) -> String {
    path.to_str().expect("a utf-8 path").to_string()
}

fn base_value(tensor: usize, index: usize) -> f32 {
    let steps = u8::try_from(tensor * 7 + index % 13).expect("a small step count");
    f32::from(steps) * 0.25
}

fn make_set(tag: &str, tensors: usize) -> PathBuf {
    let dir = directory(tag);
    for tensor in 0..tensors {
        let mut bytes = Vec::with_capacity(VALUES * 4);
        for index in 0..VALUES {
            bytes.extend_from_slice(&base_value(tensor, index).to_le_bytes());
        }
        fs::write(dir.join(format!("output-{tensor:05}.f32")), &bytes).expect("an output file");
    }
    dir
}

fn write_value(dir: &Path, tensor: usize, index: usize, value: f32) {
    let path = dir.join(format!("output-{tensor:05}.f32"));
    let mut bytes = fs::read(&path).expect("the output file");
    let start = index * 4;
    bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
    fs::write(&path, &bytes).expect("the rewritten output file");
}

fn write_latency(dir: &Path, microseconds: &[u64]) {
    use std::fmt::Write as _;
    let mut lines = String::from("index,microseconds\n");
    for (index, value) in microseconds.iter().enumerate() {
        writeln!(lines, "{index},{value}").expect("writing to a string cannot fail");
    }
    fs::write(dir.join("latency.csv"), lines).expect("the latency file");
}

fn compare(a: &Path, b: &Path, extra: &[&str]) -> Output {
    let a_text = text_of(a);
    let b_text = text_of(b);
    let mut args = vec!["compare", "--a", &a_text, "--b", &b_text];
    args.extend_from_slice(extra);
    run(&args)
}

#[test]
fn identical_sets_compare_equal() {
    let a = make_set("compare-equal-a", 3);
    let b = make_set("compare-equal-b", 3);
    write_latency(&a, &[100, 200, 300]);
    let output = compare(&a, &b, &[]);
    assert_eq!(code(&output), 0);
    assert_eq!(
        stdout_text(&output),
        "tensors 3\nvalues 420000\nmax_abs_error 0\nfirst_difference none\nlatency a count 3 p50_us 200 p95_us 300 p99_us 300\nlatency b none\n"
    );
}

#[test]
fn a_single_changed_value_in_the_last_position_of_the_last_tensor_is_reported() {
    let a = make_set("compare-last-a", 3);
    let b = make_set("compare-last-b", 3);
    write_value(&b, 2, 419_999, 6.0);
    let output = compare(&a, &b, &[]);
    assert_eq!(code(&output), 1);
    assert_eq!(
        stdout_text(&output),
        "tensors 3\nvalues 420000\nmax_abs_error 0.5\nfirst_difference tensor 2 index 419999 a 5.5 b 6\nlatency a none\nlatency b none\n"
    );
    let tolerant = compare(&a, &b, &["--tolerance", "0.5"]);
    assert_eq!(code(&tolerant), 0);
    assert_eq!(stdout_text(&tolerant), stdout_text(&output));
    assert_eq!(code(&compare(&a, &b, &["--tolerance", "0.25"])), 1);
}

#[test]
fn the_first_difference_is_the_earliest_and_the_maximum_is_the_largest() {
    let a = make_set("compare-order-a", 2);
    let b = make_set("compare-order-b", 2);
    write_value(&b, 1, 5, base_value(1, 5) + 2.0);
    write_value(&b, 0, 9, base_value(0, 9) + 1.0);
    let output = compare(&a, &b, &[]);
    assert_eq!(code(&output), 1);
    assert_eq!(
        stdout_text(&output),
        "tensors 2\nvalues 420000\nmax_abs_error 2\nfirst_difference tensor 0 index 9 a 2.25 b 3.25\nlatency a none\nlatency b none\n"
    );
}

#[test]
fn a_nan_on_one_side_is_a_difference_and_nan_on_both_sides_is_not() {
    let a = make_set("compare-nan-a", 2);
    let b = make_set("compare-nan-b", 2);
    write_value(&b, 0, 3, f32::NAN);
    let one_side = compare(&a, &b, &["--tolerance", "1000000"]);
    assert_eq!(code(&one_side), 1);
    assert_eq!(
        stdout_text(&one_side),
        "tensors 2\nvalues 420000\nmax_abs_error inf\nfirst_difference tensor 0 index 3 a 0.75 b NaN\nlatency a none\nlatency b none\n"
    );
    write_value(&a, 0, 3, f32::NAN);
    let both = compare(&a, &b, &[]);
    assert_eq!(code(&both), 0);
    assert!(stdout_text(&both).contains("max_abs_error 0\nfirst_difference none"));
    write_value(&a, 0, 4, -0.0);
    write_value(&b, 0, 4, 0.0);
    let signed_zero = compare(&a, &b, &[]);
    assert_eq!(code(&signed_zero), 0);
    assert!(stdout_text(&signed_zero).contains("max_abs_error 0\nfirst_difference none"));
}

#[test]
fn sets_that_do_not_line_up_are_refused() {
    let full = make_set("compare-mismatch-full", 3);
    let fewer = make_set("compare-mismatch-fewer", 2);
    let gap = make_set("compare-mismatch-gap", 3);
    fs::rename(gap.join("output-00001.f32"), gap.join("output-00007.f32"))
        .expect("the renamed file");
    let short = make_set("compare-mismatch-short", 3);
    let short_file = short.join("output-00002.f32");
    let bytes = fs::read(&short_file).expect("the shortened file");
    fs::write(&short_file, &bytes[..bytes.len() - 4]).expect("the shortened file");
    let emptied = make_set("compare-mismatch-emptied", 3);
    fs::write(emptied.join("output-00001.f32"), b"").expect("the emptied file");
    for b in [&fewer, &gap, &short, &emptied] {
        let output = compare(&full, b, &[]);
        assert_eq!(code(&output), 3);
        assert!(!stdout_text(&output).contains("max_abs_error"));
    }
    let empty_set = directory("compare-mismatch-empty-set");
    let missing = Path::new(env!("CARGO_TARGET_TMPDIR")).join("compare-mismatch-missing");
    for other in [&empty_set, &missing] {
        let output = compare(&full, other, &[]);
        assert_eq!(code(&output), 3);
        assert!(!stdout_text(&output).contains("max_abs_error"));
        assert_eq!(code(&compare(other, &full, &[])), 3);
    }
}

#[test]
fn latency_percentiles_use_the_nearest_rank() {
    let a = make_set("compare-rank-a", 10);
    write_latency(&a, &[100, 10, 90, 20, 80, 30, 70, 40, 60, 50]);
    let a2 = make_set("compare-rank-a2", 10);
    let output = compare(&a, &a2, &[]);
    assert_eq!(code(&output), 0);
    assert!(
        stdout_text(&output)
            .contains("latency a count 10 p50_us 50 p95_us 100 p99_us 100\nlatency b none\n")
    );
    let b = make_set("compare-rank-b", 7);
    write_latency(&b, &[7, 1, 6, 2, 5, 3, 4]);
    let b2 = make_set("compare-rank-b2", 7);
    let output = compare(&b, &b2, &[]);
    assert_eq!(code(&output), 0);
    assert!(
        stdout_text(&output)
            .contains("latency a count 7 p50_us 4 p95_us 7 p99_us 7\nlatency b none\n")
    );
}

#[test]
fn a_malformed_latency_file_is_refused() {
    let b = make_set("compare-bad-latency-b", 3);
    for (tag, content) in [
        ("compare-bad-latency-header", "index,us\n0,1\n1,2\n2,3\n"),
        (
            "compare-bad-latency-field",
            "index,microseconds\n0,1\nx,2\n2,3\n",
        ),
        (
            "compare-bad-latency-count",
            "index,microseconds\n0,1\n1,2\n",
        ),
        (
            "compare-bad-latency-order",
            "index,microseconds\n0,1\n2,3\n1,2\n",
        ),
    ] {
        let a = make_set(tag, 3);
        fs::write(a.join("latency.csv"), content).expect("the latency file");
        let output = compare(&a, &b, &[]);
        assert_eq!(code(&output), 3, "{tag}");
    }
}
