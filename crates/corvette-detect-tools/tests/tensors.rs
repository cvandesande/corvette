use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

const TENSOR_BYTES: usize = 640 * 640 * 3;

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

#[test]
fn the_synthetic_generator_matches_the_splitmix64_reference_vector() {
    let dir = directory("tensors-reference-vector");
    let output = run(&[
        "tensors",
        "synthetic",
        "--out",
        &text_of(&dir),
        "--count",
        "1",
        "--seed",
        "0",
    ]);
    assert_eq!(code(&output), 0);
    assert_eq!(
        stdout_text(&output),
        format!("wrote 1 tensors to {}\n", dir.display())
    );
    let bytes = fs::read(dir.join("tensor-00000.rgb")).expect("the tensor file");
    assert_eq!(bytes.len(), TENSOR_BYTES);
    let expected: Vec<u8> = [
        0xE220_A839_7B1D_CDAF,
        0x6E78_9E6A_A1B9_65F4,
        0x06C4_5D18_8009_454F,
    ]
    .into_iter()
    .flat_map(u64::to_le_bytes)
    .collect();
    assert_eq!(&bytes[..24], expected.as_slice());
}

#[test]
fn the_same_seed_gives_identical_files_and_a_different_seed_gives_different_files() {
    let first = directory("tensors-seed-first");
    let second = directory("tensors-seed-second");
    let third = directory("tensors-seed-third");
    for dir in [&first, &second] {
        let output = run(&[
            "tensors",
            "synthetic",
            "--out",
            &text_of(dir),
            "--count",
            "3",
            "--seed",
            "5",
        ]);
        assert_eq!(code(&output), 0);
    }
    assert_eq!(
        code(&run(&[
            "tensors",
            "synthetic",
            "--out",
            &text_of(&third),
            "--count",
            "3",
            "--seed",
            "6",
        ])),
        0
    );
    let names: Vec<String> = (0..3)
        .map(|number| format!("tensor-0000{number}.rgb"))
        .collect();
    let mut listed: Vec<String> = fs::read_dir(&first)
        .expect("the listing")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    listed.sort();
    assert_eq!(listed, names);
    for name in &names {
        assert_eq!(
            fs::read(first.join(name)).expect("the first file"),
            fs::read(second.join(name)).expect("the second file"),
        );
    }
    assert_ne!(
        fs::read(first.join(&names[0])).expect("tensor zero"),
        fs::read(first.join(&names[1])).expect("tensor one"),
    );
    assert_ne!(
        fs::read(first.join(&names[0])).expect("seed five"),
        fs::read(third.join(&names[0])).expect("seed six"),
    );
}

#[test]
fn a_directory_that_already_holds_a_tensor_is_refused_and_left_unchanged() {
    let inputs = directory("tensors-refuse-inputs");
    let raw_file = inputs.join("raw.rgb");
    fs::write(&raw_file, vec![7u8; TENSOR_BYTES]).expect("the raw input file");
    for tag in ["tensors-refuse-synthetic", "tensors-refuse-raw-rgb"] {
        let dir = directory(tag);
        let kept = dir.join("tensor-00000.rgb");
        fs::write(&kept, "keep").expect("the kept file");
        let output = if tag.ends_with("synthetic") {
            run(&[
                "tensors",
                "synthetic",
                "--out",
                &text_of(&dir),
                "--count",
                "1",
            ])
        } else {
            run(&[
                "tensors",
                "raw-rgb",
                "--in",
                &text_of(&raw_file),
                "--out",
                &text_of(&dir),
            ])
        };
        assert_eq!(code(&output), 3, "{tag}");
        assert_eq!(fs::read(&kept).expect("still kept"), b"keep", "{tag}");
        assert_eq!(fs::read_dir(&dir).expect("the listing").count(), 1, "{tag}");
    }
}

#[test]
fn a_raw_rgb_file_is_split_into_numbered_tensors() {
    let source: Vec<u8> = (0..2 * TENSOR_BYTES)
        .map(|i| u8::try_from(i % 251).expect("a residue below 256"))
        .collect();
    let inputs = directory("raw-rgb-split-inputs");
    let input = inputs.join("frames.rgb");
    fs::write(&input, &source).expect("the raw file");
    let out = directory("raw-rgb-split-out");
    let output = run(&[
        "tensors",
        "raw-rgb",
        "--in",
        &text_of(&input),
        "--out",
        &text_of(&out),
    ]);
    assert_eq!(code(&output), 0);
    assert_eq!(
        stdout_text(&output),
        format!("wrote 2 tensors to {}\n", out.display())
    );
    assert_eq!(
        fs::read(out.join("tensor-00000.rgb")).expect("the first tensor"),
        &source[..TENSOR_BYTES],
    );
    assert_eq!(
        fs::read(out.join("tensor-00001.rgb")).expect("the second tensor"),
        &source[TENSOR_BYTES..],
    );
}

#[test]
fn a_raw_rgb_file_with_a_partial_tensor_or_no_data_is_refused_and_writes_nothing() {
    let inputs = directory("raw-rgb-refuse-inputs");
    for (tag, length) in [
        ("raw-rgb-refuse-long", 2 * TENSOR_BYTES + 1),
        ("raw-rgb-refuse-short", TENSOR_BYTES - 1),
        ("raw-rgb-refuse-empty", 0),
    ] {
        let input = inputs.join(format!("{tag}.rgb"));
        fs::write(&input, vec![0u8; length]).expect("the input file");
        let out = directory(tag);
        let output = run(&[
            "tensors",
            "raw-rgb",
            "--in",
            &text_of(&input),
            "--out",
            &text_of(&out),
        ]);
        assert_eq!(code(&output), 3, "{tag}");
        let held = fs::read_dir(&out).map_or(0, Iterator::count);
        assert_eq!(held, 0, "{tag}");
    }
    let missing = text_of(&inputs.join("missing.rgb"));
    let out = text_of(&directory("raw-rgb-refuse-missing-out"));
    assert_eq!(
        code(&run(&[
            "tensors", "raw-rgb", "--in", &missing, "--out", &out
        ])),
        3
    );
}
