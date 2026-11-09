use std::ffi::OsStr;
use std::fs;

use crate::Failure;

pub(crate) fn tensor_name(index: u64) -> String {
    format!("tensor-{index:05}.rgb")
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

fn is_tensor_name(name: &OsStr) -> bool {
    let Some(text) = name.to_str() else {
        return false;
    };
    let Some(digits) = text
        .strip_prefix("tensor-")
        .and_then(|rest| rest.strip_suffix(".rgb"))
    else {
        return false;
    };
    digits.len() == 5 && digits.bytes().all(|byte| byte.is_ascii_digit())
}
