//! Detection configuration file: TOML on disk, a `serde_json::Value` as the validated in-memory form.

use std::collections::{HashSet, VecDeque};
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Map, Value};

use crate::replay_segment::Area;

pub const MIN_EDGE: u32 = 16;
pub const MAX_EDGE: u32 = 8192;
pub const MIN_FPS: u32 = 1;
pub const MAX_FPS: u32 = 60;
pub const MIN_DEADLINE_MS: u32 = 1;
pub const MAX_DEADLINE_MS: u32 = 600_000;
pub const MIN_PAD_PIXELS: u32 = 0;
pub const MAX_PAD_PIXELS: u32 = 200;
pub const DEFAULT_PAD_PIXELS: u32 = 16;
pub const DEFAULT_CONFIRM_THRESHOLD: f64 = 0.7;
pub const CONFIRM_MIN_EXCLUSIVE: f64 = 0.0;
pub const CONFIRM_MAX: f64 = 1.0;
pub const MIN_CAMERAS: usize = 1;
pub const MIN_POLYGON_POINTS: usize = 3;
pub const POINT_COORDINATES: usize = 2;
pub const MIN_POLYGON_COORD: f64 = 0.0;
pub const MAX_POLYGON_COORD: f64 = 1.0;

pub const CONFIG_KEYS: &[&str] = &["worker_socket", "labels", "deadline_ms", "cameras"];
pub const REQUIRED_CONFIG_KEYS: &[&str] = &["worker_socket", "labels", "deadline_ms", "cameras"];
pub const CAMERA_KEYS: &[&str] = &[
    "name",
    "width",
    "height",
    "fps",
    "confirm_threshold",
    "area",
];
pub const REQUIRED_CAMERA_KEYS: &[&str] = &["name", "width", "height", "fps"];
pub const AREA_KEYS: &[&str] = &["name", "polygon", "pad_pixels"];
pub const REQUIRED_AREA_KEYS: &[&str] = &["name", "polygon"];

#[derive(Clone, Debug)]
pub struct Config {
    pub worker_socket: PathBuf,
    pub labels: PathBuf,
    pub deadline: Duration,
    pub cameras: Vec<CameraConfig>,
}

#[derive(Clone, Debug)]
pub struct CameraConfig {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub confirm_threshold: f64,
    pub area: Option<Area>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigError {
    Field {
        path: String,
        rule: String,
    },
    Syntax {
        message: String,
        line: usize,
        column: usize,
    },
}

impl Config {
    /// Reads TOML text, converts it to a `serde_json::Value`, and validates that value.
    /// # Errors
    /// `ConfigError` if the text does not parse or any rule below is violated.
    pub fn from_toml(text: &str) -> Result<Self, ConfigError> {
        let doc = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| syntax_error(text, &error))?;
        let value = convert_item(doc.as_item(), "")?;
        Self::from_value(&value)
    }

    /// Validates a value from any source; every rule of the format lives here.
    /// # Errors
    /// `ConfigError` naming the path of the first rule violation.
    pub fn from_value(value: &Value) -> Result<Self, ConfigError> {
        let root = object_at(value, "")?;
        reject_unknown(root, "", CONFIG_KEYS)?;
        require_present(root, "", REQUIRED_CONFIG_KEYS)?;
        let worker_socket = PathBuf::from(text_at(root, "", "worker_socket")?);
        let labels = PathBuf::from(text_at(root, "", "labels")?);
        let deadline = Duration::from_millis(u64::from(integer_at(
            root,
            "",
            "deadline_ms",
            MIN_DEADLINE_MS,
            MAX_DEADLINE_MS,
            None,
        )?));
        let cameras = match root.get("cameras") {
            Some(Value::Array(list)) => list,
            other => {
                return Err(field_error(
                    "cameras".to_owned(),
                    format!("expected an array, found {}", kind_opt(other)),
                ));
            }
        };
        if cameras.len() < MIN_CAMERAS {
            return Err(field_error(
                "cameras".to_owned(),
                format!(
                    "expected at least {MIN_CAMERAS} camera, found {}",
                    cameras.len()
                ),
            ));
        }
        let mut parsed = Vec::with_capacity(cameras.len());
        let mut names: HashSet<String> = HashSet::new();
        for (index, camera) in cameras.iter().enumerate() {
            let base = indexed("", "cameras", index);
            let camera = camera_at(camera, &base)?;
            if !names.insert(camera.name.clone()) {
                return Err(field_error(
                    dotted(&base, "name"),
                    format!("duplicate camera name \"{}\"", camera.name),
                ));
            }
            parsed.push(camera);
        }
        Ok(Self {
            worker_socket,
            labels,
            deadline,
            cameras: parsed,
        })
    }

    #[must_use]
    pub fn to_value(&self) -> Value {
        let mut root = Map::new();
        root.insert(
            "worker_socket".to_owned(),
            Value::String(self.worker_socket.display().to_string()),
        );
        root.insert(
            "labels".to_owned(),
            Value::String(self.labels.display().to_string()),
        );
        let seconds_ms = self.deadline.as_secs().saturating_mul(1_000);
        let deadline_ms = seconds_ms.saturating_add(u64::from(self.deadline.subsec_millis()));
        root.insert("deadline_ms".to_owned(), Value::from(deadline_ms));
        let cameras: Vec<Value> = self.cameras.iter().map(camera_to_value).collect();
        root.insert("cameras".to_owned(), Value::Array(cameras));
        Value::Object(root)
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Field { path, rule } => write!(f, "{path}: {rule}"),
            Self::Syntax {
                message,
                line,
                column,
            } => {
                write!(f, "toml line {line} column {column}: {message}")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

fn syntax_error(text: &str, error: &toml_edit::TomlError) -> ConfigError {
    let start = error
        .span()
        .filter(|span| text.get(span.start..).is_some())
        .map_or(0, |span| span.start);
    let before = text.get(..start).unwrap_or_default();
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |index| index + 1);
    ConfigError::Syntax {
        message: error.message().to_owned(),
        line,
        column: start - line_start + 1,
    }
}

fn convert_item(item: &toml_edit::Item, path: &str) -> Result<Value, ConfigError> {
    match item {
        toml_edit::Item::Table(table) => convert_entries(table.iter(), path),
        toml_edit::Item::ArrayOfTables(tables) => {
            let mut entries = Vec::with_capacity(tables.len());
            for (index, table) in tables.iter().enumerate() {
                let base = indexed(path, "", index);
                entries.push(convert_entries(table.iter(), &base)?);
            }
            Ok(Value::Array(entries))
        }
        toml_edit::Item::Value(value) => convert_value(value, path),
        toml_edit::Item::None => Err(field_error(path.to_owned(), "no value".to_owned())),
    }
}

fn convert_entries<'t>(
    entries: impl Iterator<Item = (&'t str, &'t toml_edit::Item)>,
    path: &str,
) -> Result<Value, ConfigError> {
    let mut object = Map::new();
    for (key, item) in entries {
        let entry_path = dotted(path, key);
        object.insert(key.to_owned(), convert_item(item, &entry_path)?);
    }
    Ok(Value::Object(object))
}

fn convert_value(value: &toml_edit::Value, path: &str) -> Result<Value, ConfigError> {
    match value {
        toml_edit::Value::String(text) => Ok(Value::String(text.value().to_owned())),
        toml_edit::Value::Integer(number) => Ok(Value::from(*number.value())),
        toml_edit::Value::Float(number) => serde_json::Number::from_f64(*number.value())
            .map(Value::Number)
            .ok_or_else(|| field_error(path.to_owned(), "not a finite number".to_owned())),
        toml_edit::Value::Boolean(flag) => Ok(Value::Bool(*flag.value())),
        toml_edit::Value::Array(array) => {
            let mut entries = Vec::with_capacity(array.len());
            for (index, entry) in array.iter().enumerate() {
                let entry_path = indexed(path, "", index);
                entries.push(convert_value(entry, &entry_path)?);
            }
            Ok(Value::Array(entries))
        }
        toml_edit::Value::InlineTable(table) => {
            let mut object = Map::new();
            for (key, entry) in table {
                let entry_path = dotted(path, key);
                object.insert(key.to_owned(), convert_value(entry, &entry_path)?);
            }
            Ok(Value::Object(object))
        }
        other @ toml_edit::Value::Datetime(_) => Err(field_error(
            path.to_owned(),
            format!("expected a plain value, found {}", other.type_name()),
        )),
    }
}

fn camera_at(value: &Value, base: &str) -> Result<CameraConfig, ConfigError> {
    let object = object_at(value, base)?;
    reject_unknown(object, base, CAMERA_KEYS)?;
    require_present(object, base, REQUIRED_CAMERA_KEYS)?;
    let name = text_at(object, base, "name")?;
    if name.is_empty() {
        return Err(field_error(dotted(base, "name"), "empty".to_owned()));
    }
    let name = name.to_owned();
    let width = integer_at(object, base, "width", MIN_EDGE, MAX_EDGE, None)?;
    let height = integer_at(object, base, "height", MIN_EDGE, MAX_EDGE, None)?;
    let fps = integer_at(object, base, "fps", MIN_FPS, MAX_FPS, None)?;
    let confirm_threshold = confirm_at(object, base)?;
    let area = match object.get("area") {
        None => None,
        Some(area) => Some(area_at(area, &dotted(base, "area"))?),
    };
    Ok(CameraConfig {
        name,
        width,
        height,
        fps,
        confirm_threshold,
        area,
    })
}

fn area_at(value: &Value, base: &str) -> Result<Area, ConfigError> {
    let object = object_at(value, base)?;
    reject_unknown(object, base, AREA_KEYS)?;
    require_present(object, base, REQUIRED_AREA_KEYS)?;
    let name = text_at(object, base, "name")?.to_owned();
    let polygon = polygon_at(object, base)?;
    let pad_pixels = integer_at(
        object,
        base,
        "pad_pixels",
        MIN_PAD_PIXELS,
        MAX_PAD_PIXELS,
        Some(DEFAULT_PAD_PIXELS),
    )?;
    Ok(Area {
        name,
        polygon,
        pad_pixels,
    })
}

fn polygon_at(object: &Map<String, Value>, base: &str) -> Result<Vec<(f64, f64)>, ConfigError> {
    let path = dotted(base, "polygon");
    let points = match object.get("polygon") {
        Some(Value::Array(points)) => points,
        other => {
            return Err(field_error(
                path,
                format!("expected an array, found {}", kind_opt(other)),
            ));
        }
    };
    if points.len() < MIN_POLYGON_POINTS {
        return Err(field_error(
            path,
            format!(
                "expected at least {MIN_POLYGON_POINTS} points, found {}",
                points.len()
            ),
        ));
    }
    let mut polygon = Vec::with_capacity(points.len());
    for (index, point) in points.iter().enumerate() {
        let point_path = indexed(&path, "", index);
        let coords = match point {
            Value::Array(coords) => coords,
            other => {
                return Err(field_error(
                    point_path,
                    format!("expected an array, found {}", kind_opt(Some(other))),
                ));
            }
        };
        if coords.len() != POINT_COORDINATES {
            return Err(field_error(
                point_path,
                format!(
                    "expected {POINT_COORDINATES} numbers, found {}",
                    coords.len()
                ),
            ));
        }
        let mut pair = VecDeque::with_capacity(POINT_COORDINATES);
        for coordinate in coords {
            let raw = coordinate.as_f64().ok_or_else(|| {
                field_error(
                    point_path.clone(),
                    format!("expected a number, found {}", kind(coordinate)),
                )
            })?;
            if !raw.is_finite() {
                return Err(field_error(
                    point_path,
                    format!("{raw} is not a finite number"),
                ));
            }
            if raw < MIN_POLYGON_COORD {
                return Err(field_error(
                    point_path,
                    format!("{raw} is below {MIN_POLYGON_COORD}"),
                ));
            }
            if raw > MAX_POLYGON_COORD {
                return Err(field_error(
                    point_path,
                    format!("{raw} is above {MAX_POLYGON_COORD}"),
                ));
            }
            pair.push_back(raw);
        }
        let (Some(x), Some(y)) = (pair.pop_front(), pair.pop_front()) else {
            return Err(field_error(point_path, "expected a point".to_owned()));
        };
        polygon.push((x, y));
    }
    Ok(polygon)
}

fn confirm_at(object: &Map<String, Value>, base: &str) -> Result<f64, ConfigError> {
    let path = dotted(base, "confirm_threshold");
    let Some(found) = object.get("confirm_threshold") else {
        return Ok(DEFAULT_CONFIRM_THRESHOLD);
    };
    let raw = found.as_f64().ok_or_else(|| {
        field_error(
            path.clone(),
            format!("expected a number, found {}", kind(found)),
        )
    })?;
    if !raw.is_finite() {
        return Err(field_error(path, format!("{raw} is not a finite number")));
    }
    if raw <= CONFIRM_MIN_EXCLUSIVE {
        return Err(field_error(
            path,
            format!("{raw} is not above {CONFIRM_MIN_EXCLUSIVE}"),
        ));
    }
    if raw > CONFIRM_MAX {
        return Err(field_error(path, format!("{raw} is above {CONFIRM_MAX}")));
    }
    Ok(raw)
}

fn integer_at(
    object: &Map<String, Value>,
    base: &str,
    key: &str,
    min: u32,
    max: u32,
    default: Option<u32>,
) -> Result<u32, ConfigError> {
    let path = dotted(base, key);
    let Some(found) = object.get(key) else {
        return default.map_or_else(|| Err(field_error(path, "missing".to_owned())), Ok);
    };
    let raw = found.as_i64().ok_or_else(|| {
        field_error(
            path.clone(),
            format!("expected an integer, found {}", kind(found)),
        )
    })?;
    if raw < i64::from(min) {
        return Err(field_error(path, format!("{raw} is below {min}")));
    }
    if raw > i64::from(max) {
        return Err(field_error(path, format!("{raw} is above {max}")));
    }
    u32::try_from(raw).map_err(|_| field_error(path, format!("{raw} is outside {min}..={max}")))
}

fn text_at<'v>(
    object: &'v Map<String, Value>,
    base: &str,
    key: &str,
) -> Result<&'v str, ConfigError> {
    let path = dotted(base, key);
    let found = object
        .get(key)
        .ok_or_else(|| field_error(path.clone(), "missing".to_owned()))?;
    found
        .as_str()
        .ok_or_else(|| field_error(path, format!("expected a string, found {}", kind(found))))
}

fn object_at<'v>(value: &'v Value, path: &str) -> Result<&'v Map<String, Value>, ConfigError> {
    match value {
        Value::Object(object) => Ok(object),
        other => Err(field_error(
            path.to_owned(),
            format!("expected a table, found {}", kind(other)),
        )),
    }
}

fn reject_unknown(
    object: &Map<String, Value>,
    base: &str,
    known: &[&str],
) -> Result<(), ConfigError> {
    for key in object.keys() {
        if !known.contains(&key.as_str()) {
            return Err(field_error(dotted(base, key), "unknown key".to_owned()));
        }
    }
    Ok(())
}

fn require_present(
    object: &Map<String, Value>,
    base: &str,
    required: &[&str],
) -> Result<(), ConfigError> {
    for key in required {
        if !object.contains_key(*key) {
            return Err(field_error(dotted(base, key), "missing".to_owned()));
        }
    }
    Ok(())
}

fn camera_to_value(camera: &CameraConfig) -> Value {
    let mut object = Map::new();
    object.insert("name".to_owned(), Value::String(camera.name.clone()));
    object.insert("width".to_owned(), Value::from(u64::from(camera.width)));
    object.insert("height".to_owned(), Value::from(u64::from(camera.height)));
    object.insert("fps".to_owned(), Value::from(u64::from(camera.fps)));
    object.insert(
        "confirm_threshold".to_owned(),
        Value::from(camera.confirm_threshold),
    );
    if let Some(area) = &camera.area {
        object.insert("area".to_owned(), area_to_value(area));
    }
    Value::Object(object)
}

fn area_to_value(area: &Area) -> Value {
    let polygon: Vec<Value> = area
        .polygon
        .iter()
        .map(|(x, y)| Value::Array(vec![Value::from(*x), Value::from(*y)]))
        .collect();
    let mut object = Map::new();
    object.insert("name".to_owned(), Value::String(area.name.clone()));
    object.insert("polygon".to_owned(), Value::Array(polygon));
    object.insert(
        "pad_pixels".to_owned(),
        Value::from(u64::from(area.pad_pixels)),
    );
    Value::Object(object)
}

fn dotted(base: &str, key: &str) -> String {
    if base.is_empty() {
        key.to_owned()
    } else {
        format!("{base}.{key}")
    }
}

fn indexed(base: &str, key: &str, index: usize) -> String {
    if base.is_empty() {
        format!("{key}[{index}]")
    } else if key.is_empty() {
        format!("{base}[{index}]")
    } else {
        format!("{base}.{key}[{index}]")
    }
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "a table",
        Value::Number(number) if number.is_i64() || number.is_u64() => "an integer",
        Value::Number(_) => "a float",
    }
}

fn kind_opt(value: Option<&Value>) -> &'static str {
    value.map_or("nothing", kind)
}

const fn field_error(path: String, rule: String) -> ConfigError {
    ConfigError::Field { path, rule }
}
