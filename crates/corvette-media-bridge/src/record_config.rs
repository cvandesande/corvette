//! The recording view of the camera TOML: which cameras record, at what piece
//! length, retention and storage root. Other programs own the rest of the file.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item, Table};

pub const CAMERA_TOML_PATH_ENV_VAR: &str = "CORVETTE_CAMERA_TOML";

const DEFAULT_PIECE_SECONDS: u32 = 4;
const DEFAULT_RETENTION_DAYS: u32 = 7;
const MAX_PIECE_SECONDS: u32 = 60;
const MAX_RETENTION_DAYS: u32 = 3650;
pub const DEFAULT_ALERT_LABELS: [&str; 2] = ["person", "car"];

fn default_alert_labels() -> Vec<String> {
    DEFAULT_ALERT_LABELS
        .iter()
        .map(|label| (*label).to_owned())
        .collect()
}

#[derive(Debug, Clone)]
pub struct RecordConfig {
    pub cameras: Vec<CameraRecord>,
    pub storage: Storage,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Streams {
    pub main: bool,
    pub sub: bool,
}

#[derive(Debug, Clone)]
pub struct CameraRecord {
    pub name: String,
    pub enabled: bool,
    pub record_enabled: bool,
    pub piece_seconds: u32,
    pub retention_days: u32,
    pub streams: Streams,
    pub review: Review,
}

/// Which labels a camera turns into events and which of those count as alerts.
#[derive(Debug, Clone)]
pub struct Review {
    pub alert_labels: Vec<String>,
    pub detection_labels: Option<Vec<String>>,
}

impl Default for Review {
    fn default() -> Self {
        Self {
            alert_labels: default_alert_labels(),
            detection_labels: None,
        }
    }
}

impl CameraRecord {
    #[must_use]
    pub const fn records(&self) -> bool {
        self.enabled && self.record_enabled && self.streams.main
    }
}

#[derive(Debug, Clone)]
pub struct Storage {
    pub block_path: Option<PathBuf>,
    pub retention_enforce: bool,
}

#[derive(Debug)]
pub enum RecordConfigError {
    Read {
        path: String,
        source: std::io::Error,
    },
    EnvValue(&'static str),
    Syntax {
        line: usize,
        column: usize,
    },
    Field {
        camera: String,
        key: String,
        rule: String,
    },
}

fn field(camera: &str, key: &str, rule: &str) -> RecordConfigError {
    RecordConfigError::Field {
        camera: camera.to_owned(),
        key: key.to_owned(),
        rule: rule.to_owned(),
    }
}

impl std::fmt::Display for RecordConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read { path, source } => write!(f, "reading camera TOML {path}: {source}"),
            Self::EnvValue(name) => write!(f, "environment variable {name} is not valid unicode"),
            Self::Syntax { line, column } => {
                write!(
                    f,
                    "camera TOML is not valid TOML at line {line} column {column}"
                )
            }
            Self::Field { camera, key, rule } => {
                write!(f, "camera TOML {camera}: {key}: {rule}")
            }
        }
    }
}

impl std::error::Error for RecordConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::EnvValue(_) | Self::Syntax { .. } | Self::Field { .. } => None,
        }
    }
}

/// # Errors
/// See [`load_from_path`].
pub fn load_from_env() -> Result<Option<RecordConfig>, RecordConfigError> {
    match std::env::var(CAMERA_TOML_PATH_ENV_VAR) {
        Ok(path) => load_from_path(path).map(Some),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            Err(RecordConfigError::EnvValue(CAMERA_TOML_PATH_ENV_VAR))
        }
    }
}

/// # Errors
/// See [`parse`], plus [`RecordConfigError::Read`] when the file cannot be read.
pub fn load_from_path(path: impl AsRef<Path>) -> Result<RecordConfig, RecordConfigError> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path).map_err(|source| RecordConfigError::Read {
        path: path.display().to_string(),
        source,
    })?;
    parse(&text)
}

/// # Errors
/// [`RecordConfigError`] naming the first rule violation; a syntax failure names line and column.
pub fn parse(text: &str) -> Result<RecordConfig, RecordConfigError> {
    let doc = match text.parse::<DocumentMut>() {
        Ok(doc) => doc,
        Err(error) => {
            let start = error
                .span()
                .filter(|span| text.get(span.start..).is_some())
                .map_or(0, |span| span.start);
            let before = text.get(..start).unwrap_or_default();
            let line = before.matches('\n').count() + 1;
            let column = start - before.rfind('\n').map_or(0, |index| index + 1) + 1;
            return Err(RecordConfigError::Syntax { line, column });
        }
    };
    let root = doc.as_table();

    let mut cameras = Vec::new();
    let mut warnings = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    if let Some(item) = root.get("cameras") {
        let tables = item
            .as_array_of_tables()
            .ok_or_else(|| field("file", "cameras", "expected an array of camera tables"))?;
        for (index, table) in tables.iter().enumerate() {
            let record = camera_from(table, index, &mut seen)?;
            if record.enabled && record.record_enabled && !record.streams.main {
                warnings.push(format!(
                    "camera {} has no main stream; recording stays off",
                    record.name
                ));
            }
            cameras.push(record);
        }
    }

    let storage = Storage {
        block_path: match lookup(root, "storage.block.path", "file")? {
            None => None,
            Some(item) => Some(PathBuf::from(
                item.as_str()
                    .filter(|path| !path.is_empty())
                    .ok_or_else(|| {
                        field("file", "storage.block.path", "expected a non-empty string")
                    })?,
            )),
        },
        retention_enforce: bool_at(root, "storage.retention_enforce", false, "file")?,
    };
    if cameras.iter().any(CameraRecord::records) && storage.block_path.is_none() {
        return Err(field(
            "file",
            "storage.block.path",
            "required when a camera records",
        ));
    }
    Ok(RecordConfig {
        cameras,
        storage,
        warnings,
    })
}

fn camera_from(
    table: &Table,
    index: usize,
    seen: &mut HashSet<String>,
) -> Result<CameraRecord, RecordConfigError> {
    let name_key = format!("cameras[{index}].name");
    let raw_name = match lookup(table, "name", "file")? {
        None => return Err(field("file", &name_key, "expected a required string name")),
        Some(item) => item
            .as_str()
            .ok_or_else(|| field("file", &name_key, "expected a string name"))?,
    };
    if raw_name.is_empty()
        || !raw_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err(field(
            "file",
            &name_key,
            "expected a non-empty name of ASCII letters, digits, '_' and '-'",
        ));
    }
    let name = raw_name.to_owned();
    if !seen.insert(name.clone()) {
        return Err(field(&name, "name", "declared more than once"));
    }
    let enabled = bool_at(table, "enabled", true, &name)?;
    let record_enabled = bool_at(table, "record.enabled", true, &name)?;
    let piece_seconds = match int_at(table, "record.piece_seconds", &name)? {
        None => DEFAULT_PIECE_SECONDS,
        Some(value) => u32::try_from(value)
            .ok()
            .filter(|seconds| *seconds >= 1 && *seconds <= MAX_PIECE_SECONDS)
            .ok_or_else(|| {
                field(
                    &name,
                    "record.piece_seconds",
                    "expected an integer in 1 to 60",
                )
            })?,
    };
    let retention_days = match int_at(table, "record.retention_days", &name)? {
        None => DEFAULT_RETENTION_DAYS,
        Some(value) => u32::try_from(value)
            .ok()
            .filter(|days| *days <= MAX_RETENTION_DAYS)
            .ok_or_else(|| {
                field(
                    &name,
                    "record.retention_days",
                    "expected an integer in 0 to 3650",
                )
            })?,
    };
    let alert_labels =
        string_array_at(table, "review.alert_labels", &name)?.unwrap_or_else(default_alert_labels);
    let detection_labels = string_array_at(table, "review.detection_labels", &name)?;
    Ok(CameraRecord {
        name,
        enabled,
        record_enabled,
        piece_seconds,
        retention_days,
        streams: Streams {
            main: has_stream(table, "main"),
            sub: has_stream(table, "sub"),
        },
        review: Review {
            alert_labels,
            detection_labels,
        },
    })
}

fn lookup<'a>(
    root: &'a Table,
    path: &str,
    camera: &str,
) -> Result<Option<&'a Item>, RecordConfigError> {
    let mut segments = path.split('.');
    let Some(first) = segments.next() else {
        unreachable!("a lookup path names at least one key")
    };
    let Some(mut item) = root.get(first) else {
        return Ok(None);
    };
    for segment in segments {
        let table = item
            .as_table_like()
            .ok_or_else(|| field(camera, path, "expected a table"))?;
        let Some(next) = table.get(segment) else {
            return Ok(None);
        };
        item = next;
    }
    Ok(Some(item))
}

fn bool_at(
    table: &Table,
    path: &str,
    default: bool,
    camera: &str,
) -> Result<bool, RecordConfigError> {
    lookup(table, path, camera)?.map_or(Ok(default), |item| {
        item.as_bool()
            .ok_or_else(|| field(camera, path, "expected a boolean"))
    })
}

fn int_at(table: &Table, path: &str, camera: &str) -> Result<Option<i64>, RecordConfigError> {
    match lookup(table, path, camera)? {
        None => Ok(None),
        Some(item) => {
            Ok(Some(item.as_integer().ok_or_else(|| {
                field(camera, path, "expected an integer")
            })?))
        }
    }
}

fn string_array_at(
    table: &Table,
    path: &str,
    camera: &str,
) -> Result<Option<Vec<String>>, RecordConfigError> {
    let Some(item) = lookup(table, path, camera)? else {
        return Ok(None);
    };
    let array = item
        .as_array()
        .ok_or_else(|| field(camera, path, "expected an array of strings"))?;
    let mut values = Vec::with_capacity(array.len());
    for value in array {
        let value = value
            .as_str()
            .ok_or_else(|| field(camera, path, "expected an array of strings"))?;
        if value.is_empty() {
            return Err(field(camera, path, "expected non-empty strings"));
        }
        values.push(value.to_owned());
    }
    Ok(Some(values))
}

fn has_stream(camera: &Table, stream: &str) -> bool {
    camera
        .get("source")
        .and_then(Item::as_table_like)
        .is_some_and(|source| source.contains_key(stream))
}
