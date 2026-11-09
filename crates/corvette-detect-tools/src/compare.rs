use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use corvette_detect_proto::RAW_VALUES;

use crate::latency::{self, Latency};
use crate::{EXIT_DIFFERENCE, EXIT_MATCH, Failure, args};

const COMPARE_FLAGS: &[&str] = &["a", "b", "tolerance"];
const VALUE_BYTES: usize = 4;

struct Set {
    paths: Vec<PathBuf>,
    latency: Option<Latency>,
}

pub(crate) fn run(args: &[String]) -> Result<ExitCode, Failure> {
    let flags = args::Flags::parse(args, COMPARE_FLAGS).ok_or(Failure::Usage)?;
    let a_name = flags.required("a")?;
    let b_name = flags.required("b")?;
    let tolerance = flags.get("tolerance").map_or(Ok(0.0), parse_tolerance)?;
    let a = read_set(a_name)?;
    let b = read_set(b_name)?;
    if a.paths.len() != b.paths.len() {
        return Err(Failure::Input(format!(
            "the sets hold {} and {} output files",
            a.paths.len(),
            b.paths.len()
        )));
    }
    let mut maximum = 0.0_f64;
    let mut first: Option<(usize, usize, f32, f32)> = None;
    for (tensor, (path_a, path_b)) in a.paths.iter().zip(b.paths.iter()).enumerate() {
        let values_a = read_values(path_a)?;
        let values_b = read_values(path_b)?;
        for (index, (x, y)) in values_a.iter().zip(values_b.iter()).enumerate() {
            let error = error_between(*x, *y);
            if error > maximum {
                maximum = error;
            }
            if first.is_none() && error > 0.0 {
                first = Some((tensor, index, *x, *y));
            }
        }
    }
    println!("tensors {}", a.paths.len());
    println!("values {RAW_VALUES}");
    println!("max_abs_error {maximum}");
    match first {
        Some((tensor, index, x, y)) => {
            println!("first_difference tensor {tensor} index {index} a {x} b {y}");
        }
        None => println!("first_difference none"),
    }
    print_latency("a", a.latency.as_ref());
    print_latency("b", b.latency.as_ref());
    if maximum > tolerance {
        Ok(ExitCode::from(EXIT_DIFFERENCE))
    } else {
        Ok(ExitCode::from(EXIT_MATCH))
    }
}

fn print_latency(label: &str, latency: Option<&Latency>) {
    if let Some(values) = latency {
        println!(
            "latency {label} count {} p50_us {} p95_us {} p99_us {}",
            values.count(),
            values.percentile(50),
            values.percentile(95),
            values.percentile(99)
        );
    } else {
        println!("latency {label} none");
    }
}

fn read_set(dir: &str) -> Result<Set, Failure> {
    let path = Path::new(dir);
    let entries = fs::read_dir(path)
        .map_err(|error| Failure::Input(format!("cannot list {dir}: {error}")))?;
    let mut numbers: Vec<usize> = Vec::new();
    for entry in entries {
        let name = entry
            .map_err(|error| Failure::Input(format!("cannot list {dir}: {error}")))?
            .file_name();
        let Some(text) = name.to_str() else {
            continue;
        };
        let Some(digits) = text
            .strip_prefix("output-")
            .and_then(|rest| rest.strip_suffix(".f32"))
        else {
            continue;
        };
        if digits.len() == 5
            && digits.bytes().all(|byte| byte.is_ascii_digit())
            && let Ok(number) = digits.parse::<usize>()
        {
            numbers.push(number);
        }
    }
    numbers.sort_unstable();
    if numbers.is_empty() {
        return Err(Failure::Input(format!("{dir} holds no output files")));
    }
    for (position, number) in numbers.iter().enumerate() {
        if *number != position {
            return Err(Failure::Input(format!(
                "{dir} has a gap or an extra file in the output numbering"
            )));
        }
    }
    let expected = u64::try_from(RAW_VALUES * VALUE_BYTES)
        .map_err(|_| Failure::Input(String::from("output size too large")))?;
    let mut paths = Vec::new();
    for number in &numbers {
        let path = path.join(format!("output-{number:05}.f32"));
        let length = fs::metadata(&path)
            .map(|meta| meta.len())
            .map_err(|error| {
                Failure::Input(format!("cannot inspect {}: {error}", path.display()))
            })?;
        if length != expected {
            return Err(Failure::Input(format!(
                "{} is {length} bytes, expected {expected}",
                path.display()
            )));
        }
        paths.push(path);
    }
    let latency = latency::load(&path.join("latency.csv"), paths.len())?;
    Ok(Set { paths, latency })
}

fn read_values(path: &Path) -> Result<Vec<f32>, Failure> {
    let bytes = fs::read(path)
        .map_err(|error| Failure::Input(format!("cannot read {}: {error}", path.display())))?;
    Ok(bytes
        .chunks_exact(VALUE_BYTES)
        .map(|chunk| {
            chunk
                .try_into()
                .map_or(0.0, |fixed: [u8; VALUE_BYTES]| f32::from_le_bytes(fixed))
        })
        .collect())
}

fn error_between(a: f32, b: f32) -> f64 {
    if a.total_cmp(&b).is_eq() || (a.is_nan() && b.is_nan()) {
        0.0
    } else if a.is_nan() || b.is_nan() {
        f64::INFINITY
    } else {
        (f64::from(a) - f64::from(b)).abs()
    }
}

fn parse_tolerance(text: &str) -> Result<f64, Failure> {
    text.parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value >= 0.0)
        .ok_or(Failure::Usage)
}
