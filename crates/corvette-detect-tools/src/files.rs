use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use corvette_detect_proto::TENSOR_BYTES;

use crate::Failure;

pub(crate) fn tensor_name(index: u64) -> String {
    format!("tensor-{index:05}.rgb")
}

pub(crate) fn output_name(index: u64) -> String {
    format!("output-{index:05}.f32")
}

pub(crate) fn holds_tensor_files(dir: &str) -> Result<bool, Failure> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(Failure::Input(format!("cannot list {dir}: {error}")));
        }
    };
    Ok(entries
        .filter_map(Result::ok)
        .any(|entry| is_tensor_name(&entry.file_name())))
}

pub(crate) fn tensor_set(dir: &str) -> Result<Vec<PathBuf>, Failure> {
    let path = Path::new(dir);
    let entries = fs::read_dir(path)
        .map_err(|error| Failure::Input(format!("cannot list {dir}: {error}")))?;
    let mut numbers: Vec<u64> = Vec::new();
    for entry in entries {
        let name = entry
            .map_err(|error| Failure::Input(format!("cannot list {dir}: {error}")))?
            .file_name();
        if let Some(number) = number_of(&name, "tensor-", ".rgb") {
            numbers.push(number);
        }
    }
    numbers.sort_unstable();
    if numbers.is_empty() {
        return Err(Failure::Input(format!("{dir} holds no tensor files")));
    }
    for (position, number) in numbers.iter().enumerate() {
        let expected = u64::try_from(position)
            .map_err(|_| Failure::Input(format!("{dir} holds too many tensor files")))?;
        if *number != expected {
            return Err(Failure::Input(format!(
                "{dir} has a gap or an extra file in the tensor numbering"
            )));
        }
    }
    let expected_bytes = u64::try_from(TENSOR_BYTES)
        .map_err(|_| Failure::Input(String::from("tensor size too large")))?;
    let mut paths = Vec::new();
    for number in &numbers {
        let file = path.join(tensor_name(*number));
        let length = fs::metadata(&file)
            .map(|meta| meta.len())
            .map_err(|error| {
                Failure::Input(format!("cannot inspect {}: {error}", file.display()))
            })?;
        if length != expected_bytes {
            return Err(Failure::Input(format!(
                "{} is {length} bytes, expected {expected_bytes}",
                file.display()
            )));
        }
        paths.push(file);
    }
    Ok(paths)
}

pub(crate) fn holds_output_files(dir: &Path) -> Result<bool, Failure> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(Failure::Input(format!(
                "cannot list {}: {error}",
                dir.display()
            )));
        }
    };
    Ok(entries.filter_map(Result::ok).any(|entry| {
        let name = entry.file_name();
        name.as_os_str() == OsStr::new("latency.csv") || is_output_name(&name)
    }))
}

fn is_tensor_name(name: &OsStr) -> bool {
    number_of(name, "tensor-", ".rgb").is_some()
}

fn is_output_name(name: &OsStr) -> bool {
    number_of(name, "output-", ".f32").is_some()
}

fn number_of(name: &OsStr, prefix: &str, suffix: &str) -> Option<u64> {
    let digits = name.to_str()?.strip_prefix(prefix)?.strip_suffix(suffix)?;
    if digits.len() != 5 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u64>().ok()
}
