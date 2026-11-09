use std::fs;
use std::path::Path;

use crate::Failure;

const HEADER: &str = "index,microseconds";

#[derive(Debug)]
pub(crate) struct Latency(Vec<u64>);

impl Latency {
    pub(crate) const fn count(&self) -> usize {
        self.0.len()
    }

    pub(crate) fn percentile(&self, quantile: usize) -> u64 {
        let rank = (quantile * self.0.len()).div_ceil(100);
        self.0.get(rank - 1).copied().unwrap_or_default()
    }
}

pub(crate) fn load(path: &Path, expected: usize) -> Result<Option<Latency>, Failure> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(Failure::Input(format!(
                "cannot read {}: {error}",
                path.display()
            )));
        }
    };
    let mut lines = text.lines();
    if lines.next() != Some(HEADER) {
        return Err(Failure::Input(format!(
            "{} does not start with the latency header",
            path.display()
        )));
    }
    let mut values: Vec<u64> = Vec::new();
    for line in lines {
        let malformed =
            || Failure::Input(format!("{} has the malformed row {line}", path.display()));
        let (index, microseconds) = line.split_once(',').ok_or_else(malformed)?;
        let index = index.parse::<usize>().map_err(|_| malformed())?;
        let microseconds = microseconds.parse::<u64>().map_err(|_| malformed())?;
        if index != values.len() {
            return Err(Failure::Input(format!(
                "{} has index {index} where {} was expected",
                path.display(),
                values.len()
            )));
        }
        values.push(microseconds);
    }
    if values.len() != expected {
        return Err(Failure::Input(format!(
            "{} has {} rows for {expected} output files",
            path.display(),
            values.len()
        )));
    }
    values.sort_unstable();
    Ok(Some(Latency(values)))
}
