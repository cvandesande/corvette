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
    "enabled",
    "confirm_threshold",
    "area",
    "source",
];
pub const REQUIRED_CAMERA_KEYS: &[&str] = &["name", "width", "height", "fps"];
pub const AREA_KEYS: &[&str] = &["name", "polygon", "pad_pixels"];
pub const REQUIRED_AREA_KEYS: &[&str] = &["name", "polygon"];
pub const SOURCE_KEYS: &[&str] = &["main", "sub"];
pub const STREAM_KEYS: &[&str] = &["host", "port", "path", "username", "secret_env"];
pub const REQUIRED_STREAM_KEYS: &[&str] = &["host"];
pub const MIN_STREAM_PORT: u32 = 1;
pub const MAX_STREAM_PORT: u32 = 65535;
pub const DEFAULT_STREAM_PORT: u32 = 554;
pub const DEFAULT_STREAM_PATH: &str = "/";

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
    pub enabled: bool,
    pub confirm_threshold: f64,
    pub area: Option<Area>,
    pub source: Option<CameraSource>,
}

#[derive(Clone, Debug)]
pub struct CameraSource {
    pub main: Option<StreamSource>,
    pub sub: Option<StreamSource>,
}

#[derive(Clone, Debug)]
pub struct StreamSource {
    pub host: String,
    pub port: u32,
    pub path: String,
    pub username: String,
    pub secret_env: Option<String>,
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

/// One rule violation: the loader's path, the rule's message, and what was found
/// together with what was allowed where a closed set or range applies.
#[derive(Clone, Debug)]
pub struct Violation {
    pub path: String,
    pub rule: String,
    pub found: Option<Value>,
    pub allowed: Option<Value>,
}

impl Violation {
    const fn new(path: String, rule: String) -> Self {
        Self {
            path,
            rule,
            found: None,
            allowed: None,
        }
    }
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
        let violations = match Self::validate_all(value) {
            Ok(config) => return Ok(config),
            Err(violations) => violations,
        };
        let Some(first) = violations.first() else {
            unreachable!("validate_all fails only with at least one violation");
        };
        Err(ConfigError::Field {
            path: first.path.clone(),
            rule: first.rule.clone(),
        })
    }

    /// Validates every rule in one pass and collects each violation in document order.
    /// # Errors
    /// Every violation found, in the loader's check order; never an empty list.
    pub fn validate_all(value: &Value) -> Result<Self, Vec<Violation>> {
        let mut out: Vec<Violation> = Vec::new();
        let Some(root) = object_at(value, "", &mut out) else {
            return Err(out);
        };
        let at = Locator {
            base: "",
            link: dotted,
        };
        reject_unknown(root, at, CONFIG_KEYS, &mut out);
        require_present(root, at, REQUIRED_CONFIG_KEYS, &mut out);
        let worker_socket = text_at(root, at, "worker_socket", &mut out).map(PathBuf::from);
        let labels = text_at(root, at, "labels", &mut out).map(PathBuf::from);
        let deadline = integer_at(
            root,
            at,
            "deadline_ms",
            MIN_DEADLINE_MS,
            MAX_DEADLINE_MS,
            None,
            &mut out,
        )
        .map(|millis| Duration::from_millis(u64::from(millis)));
        let cameras = match root.get("cameras") {
            Some(Value::Array(list)) => {
                if list.len() < MIN_CAMERAS {
                    let rule = format!(
                        "expected at least {MIN_CAMERAS} camera, found {}",
                        list.len()
                    );
                    out.push(ranged(
                        "cameras".to_owned(),
                        rule,
                        count_value(list.len()),
                        at_least(count_value(MIN_CAMERAS)),
                    ));
                }
                Some(list)
            }
            Some(other) => {
                out.push(wrong_type_at("cameras".to_owned(), "an array", other));
                None
            }
            None => None,
        };
        let mut cameras_ok: Vec<CameraConfig> = Vec::new();
        if let Some(list) = cameras {
            let mut names: HashSet<String> = HashSet::new();
            for (index, camera) in list.iter().enumerate() {
                let base = indexed("", "cameras", index);
                if let Some(camera) = camera_at(camera, &base, &mut names, &mut out) {
                    cameras_ok.push(camera);
                }
            }
        }
        match (worker_socket, labels, deadline) {
            (Some(worker_socket), Some(labels), Some(deadline)) if out.is_empty() => Ok(Self {
                worker_socket,
                labels,
                deadline,
                cameras: cameras_ok,
            }),
            _ => Err(out),
        }
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

/// Builds the syntax error for a parse failure, locating line and column in `text`.
#[must_use]
pub fn syntax_error(text: &str, error: &toml_edit::TomlError) -> ConfigError {
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

/// Converts a loaded TOML item into the loader's `serde_json` value form.
///
/// # Errors
/// `ConfigError::Field` if a datetime, a non-finite number, or an absent value appears.
pub fn convert_item(item: &toml_edit::Item, path: &str) -> Result<Value, ConfigError> {
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

fn camera_at(
    value: &Value,
    base: &str,
    names: &mut HashSet<String>,
    out: &mut Vec<Violation>,
) -> Option<CameraConfig> {
    let object = object_at(value, base, out)?;
    let at = Locator { base, link: dotted };
    reject_unknown(object, at, CAMERA_KEYS, out);
    require_present(object, at, REQUIRED_CAMERA_KEYS, out);
    let mut name = text_at(object, at, "name", out).map(str::to_owned);
    if name.as_deref() == Some("") {
        out.push(Violation {
            found: Some(Value::String(String::new())),
            ..Violation::new(dotted(base, "name"), "empty".to_owned())
        });
        name = None;
    }
    // The name reaches every log line as unit="<name>" without escaping, so a quote or
    // newline in it could forge a line; only plain characters are accepted.
    if let Some(found) = name.clone().filter(|name| {
        !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    }) {
        out.push(Violation {
            found: Some(Value::String(found)),
            ..Violation::new(
                dotted(base, "name"),
                "name may hold only letters, digits, _ and -".to_owned(),
            )
        });
        name = None;
    }
    let width = integer_at(object, at, "width", MIN_EDGE, MAX_EDGE, None, out);
    let height = integer_at(object, at, "height", MIN_EDGE, MAX_EDGE, None, out);
    let fps = integer_at(object, at, "fps", MIN_FPS, MAX_FPS, None, out);
    let enabled = object.get("enabled").map_or(Some(true), |flag| match flag {
        Value::Bool(flag) => Some(*flag),
        other => {
            out.push(wrong_type_at(dotted(base, "enabled"), "a boolean", other));
            None
        }
    });
    let confirm_threshold = confirm_at(object, base, out);
    let area = object.get("area").map_or(Some(None), |area| {
        Some(area_at(area, &dotted(base, "area"), out))
    });
    let source_label = name
        .as_ref()
        .map_or_else(|| base.to_owned(), |name| format!("camera {name}"));
    let source_base = format!("{source_label}: source");
    let source = object.get("source").map_or(Some(None), |source| {
        Some(source_at(source, &source_base, out))
    });
    if let Some(name) = &name
        && !names.insert(name.clone())
    {
        out.push(Violation {
            found: Some(Value::String(name.clone())),
            ..Violation::new(
                dotted(base, "name"),
                format!("duplicate camera name \"{name}\""),
            )
        });
    }
    Some(CameraConfig {
        name: name?,
        width: width?,
        height: height?,
        fps: fps?,
        enabled: enabled?,
        confirm_threshold: confirm_threshold?,
        area: area?,
        source: source?,
    })
}

fn area_at(value: &Value, base: &str, out: &mut Vec<Violation>) -> Option<Area> {
    let object = object_at(value, base, out)?;
    let at = Locator { base, link: dotted };
    reject_unknown(object, at, AREA_KEYS, out);
    require_present(object, at, REQUIRED_AREA_KEYS, out);
    let name = text_at(object, at, "name", out);
    let polygon = polygon_at(object, base, out);
    let pad_pixels = integer_at(
        object,
        at,
        "pad_pixels",
        MIN_PAD_PIXELS,
        MAX_PAD_PIXELS,
        Some(DEFAULT_PAD_PIXELS),
        out,
    );
    Some(Area {
        name: name?.to_owned(),
        polygon: polygon?,
        pad_pixels: pad_pixels?,
    })
}

/// Parses one camera's `source` table. `base` names the camera in refusal text,
/// by name when usable and by indexed path otherwise.
fn source_at(value: &Value, base: &str, out: &mut Vec<Violation>) -> Option<CameraSource> {
    let object = object_at(value, base, out)?;
    let at = Locator {
        base,
        link: colonic,
    };
    reject_unknown(object, at, SOURCE_KEYS, out);
    if !object.contains_key("main") && !object.contains_key("sub") {
        out.push(Violation::new(
            base.to_owned(),
            "expected main or sub".to_owned(),
        ));
        return None;
    }
    let main = object.get("main").map_or(Some(None), |main| {
        Some(stream_at(main, &format!("{base} main"), out))
    });
    let sub = object.get("sub").map_or(Some(None), |sub| {
        Some(stream_at(sub, &format!("{base} sub"), out))
    });
    Some(CameraSource {
        main: main?,
        sub: sub?,
    })
}

fn stream_at(value: &Value, base: &str, out: &mut Vec<Violation>) -> Option<StreamSource> {
    let object = object_at(value, base, out)?;
    let at = Locator {
        base,
        link: colonic,
    };
    reject_unknown(object, at, STREAM_KEYS, out);
    require_present(object, at, REQUIRED_STREAM_KEYS, out);
    let mut host = text_at(object, at, "host", out).map(str::to_owned);
    if host.as_deref() == Some("") {
        out.push(Violation {
            found: Some(Value::String(String::new())),
            ..Violation::new(at.key("host"), "empty".to_owned())
        });
        host = None;
    }
    if host.as_deref().is_some_and(|host| host.contains('/')) {
        out.push(Violation::new(
            at.key("host"),
            "expected a host without /".to_owned(),
        ));
        host = None;
    }
    let port = integer_at(
        object,
        at,
        "port",
        MIN_STREAM_PORT,
        MAX_STREAM_PORT,
        Some(DEFAULT_STREAM_PORT),
        out,
    );
    let mut path = match object.get("path") {
        None => Some(DEFAULT_STREAM_PATH.to_owned()),
        Some(_) => text_at(object, at, "path", out).map(str::to_owned),
    };
    if path.as_deref().is_some_and(|path| !path.starts_with('/')) {
        out.push(Violation::new(
            at.key("path"),
            "expected a path starting with /".to_owned(),
        ));
        path = None;
    }
    let username = match object.get("username") {
        None => Some(String::new()),
        Some(_) => text_at(object, at, "username", out).map(str::to_owned),
    };
    let secret_env = match object.get("secret_env") {
        None => Some(None),
        Some(_) => text_at(object, at, "secret_env", out).map(|env| Some(env.to_owned())),
    };
    Some(StreamSource {
        host: host?,
        port: port?,
        path: path?,
        username: username?,
        secret_env: secret_env?,
    })
}

fn polygon_at(
    object: &Map<String, Value>,
    base: &str,
    out: &mut Vec<Violation>,
) -> Option<Vec<(f64, f64)>> {
    let path = dotted(base, "polygon");
    let found = object.get("polygon")?;
    let Value::Array(points) = found else {
        out.push(wrong_type_at(path, "an array", found));
        return None;
    };
    if points.len() < MIN_POLYGON_POINTS {
        let rule = format!(
            "expected at least {MIN_POLYGON_POINTS} points, found {}",
            points.len()
        );
        out.push(ranged(
            path,
            rule,
            count_value(points.len()),
            at_least(count_value(MIN_POLYGON_POINTS)),
        ));
        return None;
    }
    let mut polygon = Vec::with_capacity(points.len());
    for (index, point) in points.iter().enumerate() {
        polygon.extend(point_at(point, indexed(&path, "", index), out));
    }
    (polygon.len() == points.len()).then_some(polygon)
}

fn point_at(point: &Value, point_path: String, out: &mut Vec<Violation>) -> Option<(f64, f64)> {
    let Value::Array(coords) = point else {
        out.push(wrong_type_at(point_path, "an array", point));
        return None;
    };
    if coords.len() != POINT_COORDINATES {
        let rule = format!(
            "expected {POINT_COORDINATES} numbers, found {}",
            coords.len()
        );
        let allowed = bounds(
            count_value(POINT_COORDINATES),
            count_value(POINT_COORDINATES),
        );
        out.push(ranged(point_path, rule, count_value(coords.len()), allowed));
        return None;
    }
    let mut pair = VecDeque::with_capacity(POINT_COORDINATES);
    for coordinate in coords {
        let Some(raw) = coordinate.as_f64() else {
            out.push(wrong_type_at(point_path.clone(), "a number", coordinate));
            continue;
        };
        if !raw.is_finite() {
            out.push(Violation {
                found: Some(coordinate.clone()),
                ..Violation::new(point_path.clone(), format!("{raw} is not a finite number"))
            });
            continue;
        }
        let allowed = bounds(
            number_value(MIN_POLYGON_COORD),
            number_value(MAX_POLYGON_COORD),
        );
        if raw < MIN_POLYGON_COORD {
            let rule = format!("{raw} is below {MIN_POLYGON_COORD}");
            out.push(ranged(
                point_path.clone(),
                rule,
                coordinate.clone(),
                allowed,
            ));
            continue;
        }
        if raw > MAX_POLYGON_COORD {
            let rule = format!("{raw} is above {MAX_POLYGON_COORD}");
            out.push(ranged(
                point_path.clone(),
                rule,
                coordinate.clone(),
                allowed,
            ));
            continue;
        }
        pair.push_back(raw);
    }
    let (Some(x), Some(y)) = (pair.pop_front(), pair.pop_front()) else {
        return None;
    };
    Some((x, y))
}

fn confirm_at(object: &Map<String, Value>, base: &str, out: &mut Vec<Violation>) -> Option<f64> {
    let path = dotted(base, "confirm_threshold");
    let Some(found) = object.get("confirm_threshold") else {
        return Some(DEFAULT_CONFIRM_THRESHOLD);
    };
    let Some(raw) = found.as_f64() else {
        out.push(wrong_type_at(path, "a number", found));
        return None;
    };
    if !raw.is_finite() {
        out.push(Violation {
            found: Some(found.clone()),
            ..Violation::new(path, format!("{raw} is not a finite number"))
        });
        return None;
    }
    let allowed = bounds(
        number_value(CONFIRM_MIN_EXCLUSIVE),
        number_value(CONFIRM_MAX),
    );
    if raw <= CONFIRM_MIN_EXCLUSIVE {
        let rule = format!("{raw} is not above {CONFIRM_MIN_EXCLUSIVE}");
        out.push(ranged(path, rule, found.clone(), allowed));
        return None;
    }
    if raw > CONFIRM_MAX {
        let rule = format!("{raw} is above {CONFIRM_MAX}");
        out.push(ranged(path, rule, found.clone(), allowed));
        return None;
    }
    Some(raw)
}

fn integer_at(
    object: &Map<String, Value>,
    at: Locator<'_>,
    key: &str,
    min: u32,
    max: u32,
    default: Option<u32>,
    out: &mut Vec<Violation>,
) -> Option<u32> {
    let path = at.key(key);
    let Some(found) = object.get(key) else {
        return default;
    };
    let Some(raw) = found.as_i64() else {
        out.push(wrong_type_at(path, "an integer", found));
        return None;
    };
    let allowed = bounds(Value::from(i64::from(min)), Value::from(i64::from(max)));
    if raw < i64::from(min) {
        let rule = format!("{raw} is below {min}");
        out.push(ranged(path, rule, found.clone(), allowed));
        return None;
    }
    if raw > i64::from(max) {
        let rule = format!("{raw} is above {max}");
        out.push(ranged(path, rule, found.clone(), allowed));
        return None;
    }
    let Ok(value) = u32::try_from(raw) else {
        let rule = format!("{raw} is outside {min}..={max}");
        out.push(ranged(path, rule, found.clone(), allowed));
        return None;
    };
    Some(value)
}

fn text_at<'v>(
    object: &'v Map<String, Value>,
    at: Locator<'_>,
    key: &str,
    out: &mut Vec<Violation>,
) -> Option<&'v str> {
    let found = object.get(key)?;
    let Some(text) = found.as_str() else {
        out.push(wrong_type_at(at.key(key), "a string", found));
        return None;
    };
    Some(text)
}

fn object_at<'v>(
    value: &'v Value,
    base: &str,
    out: &mut Vec<Violation>,
) -> Option<&'v Map<String, Value>> {
    match value {
        Value::Object(object) => Some(object),
        other => {
            out.push(wrong_type_at(base.to_owned(), "a table", other));
            None
        }
    }
}

fn reject_unknown(
    object: &Map<String, Value>,
    at: Locator<'_>,
    known: &[&str],
    out: &mut Vec<Violation>,
) {
    let names: Vec<Value> = known
        .iter()
        .map(|key| Value::String((*key).to_owned()))
        .collect();
    for (key, value) in object {
        if !known.contains(&key.as_str()) {
            let scalar = value.is_string() || value.is_number() || value.as_bool().is_some();
            out.push(Violation {
                found: scalar.then(|| value.clone()),
                allowed: Some(Value::Array(names.clone())),
                ..Violation::new(at.key(key), "unknown key".to_owned())
            });
        }
    }
}

fn require_present(
    object: &Map<String, Value>,
    at: Locator<'_>,
    required: &[&str],
    out: &mut Vec<Violation>,
) {
    for key in required {
        if !object.contains_key(*key) {
            out.push(Violation::new(at.key(key), "missing".to_owned()));
        }
    }
}

fn bounds(min: Value, max: Value) -> Value {
    let mut allowed = Map::new();
    allowed.insert("min".to_owned(), min);
    allowed.insert("max".to_owned(), max);
    Value::Object(allowed)
}

fn at_least(min: Value) -> Value {
    let mut allowed = Map::new();
    allowed.insert("min".to_owned(), min);
    Value::Object(allowed)
}

fn count_value(count: usize) -> Value {
    u64::try_from(count).map_or(Value::Null, Value::from)
}

fn number_value(raw: f64) -> Value {
    serde_json::Number::from_f64(raw).map_or(Value::Null, Value::Number)
}

fn wrong_type_at(path: String, expected: &str, found: &Value) -> Violation {
    Violation {
        found: Some(found.clone()),
        ..Violation::new(path, format!("expected {expected}, found {}", kind(found)))
    }
}

fn ranged(path: String, rule: String, found: Value, allowed: Value) -> Violation {
    Violation {
        found: Some(found),
        allowed: Some(allowed),
        ..Violation::new(path, rule)
    }
}

fn camera_to_value(camera: &CameraConfig) -> Value {
    // `source` stays out on purpose: no route reads or serves stream sources.
    let mut object = Map::new();
    object.insert("name".to_owned(), Value::String(camera.name.clone()));
    object.insert("width".to_owned(), Value::from(u64::from(camera.width)));
    object.insert("height".to_owned(), Value::from(u64::from(camera.height)));
    object.insert("fps".to_owned(), Value::from(u64::from(camera.fps)));
    if !camera.enabled {
        object.insert("enabled".to_owned(), Value::Bool(false));
    }
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

fn colonic(base: &str, key: &str) -> String {
    format!("{base}: {key}")
}

/// Where a key sits in the document: the table's path and how a key appends to it.
#[derive(Clone, Copy)]
struct Locator<'a> {
    base: &'a str,
    link: fn(&str, &str) -> String,
}

impl Locator<'_> {
    fn key(self, key: &str) -> String {
        (self.link)(self.base, key)
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

const fn field_error(path: String, rule: String) -> ConfigError {
    ConfigError::Field { path, rule }
}
