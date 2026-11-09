use std::process::Output;

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

#[test]
fn usage_errors_exit_2_and_print_the_usage_on_stderr() {
    let cases: Vec<Vec<&str>> = vec![
        vec![],
        vec!["frobnicate"],
        vec!["tensors", "frobnicate"],
        vec![
            "tensors",
            "synthetic",
            "--out",
            "d",
            "--count",
            "1",
            "--bogus",
            "x",
        ],
        vec!["compare", "--a", "d", "--a", "e"],
        vec!["compare", "--a"],
        vec!["tensors", "synthetic", "--out", "d"],
        vec!["tensors", "synthetic", "--out", "d", "--count", "0"],
        vec!["tensors", "synthetic", "--out", "d", "--count", "100000"],
        vec![
            "tensors",
            "synthetic",
            "--out",
            "d",
            "--count",
            "1",
            "--seed",
            "-1",
        ],
        vec!["compare", "--a", "d", "--b", "e", "--tolerance", "-1"],
        vec!["compare", "--a", "d", "--b", "e", "--tolerance", "nan"],
        vec!["run", "--in", "i", "--out", "o"],
        vec!["run", "--socket", "s", "--out", "o"],
        vec!["run", "--socket", "s", "--in", "i"],
        vec![
            "run",
            "--socket",
            "s",
            "--in",
            "i",
            "--out",
            "o",
            "--deadline-ms",
            "0",
        ],
        vec![
            "run",
            "--socket",
            "s",
            "--in",
            "i",
            "--out",
            "o",
            "--deadline-ms",
            "600001",
        ],
        vec![
            "run",
            "--socket",
            "s",
            "--in",
            "i",
            "--out",
            "o",
            "--deadline-ms",
            "abc",
        ],
        vec![
            "run", "--socket", "s", "--socket", "t", "--in", "i", "--out", "o",
        ],
        vec![
            "run", "--socket", "s", "--in", "i", "--out", "o", "--bogus", "x",
        ],
    ];
    for case in cases {
        let output = run(&case);
        assert_eq!(code(&output), 2, "{case:?}");
        assert!(stdout_text(&output).is_empty(), "{case:?}");
        let usage = stderr_text(&output);
        for needle in ["tensors", "run", "compare", "0", "1", "2", "3"] {
            assert!(usage.contains(needle), "{case:?} names no {needle}");
        }
    }
}
