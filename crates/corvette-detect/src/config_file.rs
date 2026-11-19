//! The saved configuration as text: a validated `Config` rendered into existing TOML in place.

use serde_json::Value;
use toml_edit::{ArrayOfTables, DocumentMut, Item, Key, Table};

use crate::config::{
    AREA_KEYS, CAMERA_KEYS, CONFIG_KEYS, CameraConfig, Config, ConfigError,
    DEFAULT_CONFIRM_THRESHOLD, DEFAULT_PAD_PIXELS, convert_item, syntax_error,
};
use crate::replay_segment::Area;

/// Renders the new text of the saved configuration file: `existing` is the saved TOML text,
/// `None` a first write; a value equal to what the document holds keeps its formatting.
/// # Errors
/// `ConfigError::Syntax` if `existing` does not parse.
pub fn render(existing: Option<&str>, config: &Config) -> Result<String, ConfigError> {
    let mut doc = match existing {
        Some(text) => text
            .parse::<DocumentMut>()
            .map_err(|error| syntax_error(text, &error))?,
        None => DocumentMut::new(),
    };
    write_root_fields(&mut doc, config);
    write_cameras(&mut doc, config);
    order_document(&mut doc);
    let text = doc.to_string();
    Ok(format!("{}\n", text.trim_end()))
}

fn write_root_fields(doc: &mut DocumentMut, config: &Config) {
    let Value::Object(fields) = config.to_value() else {
        unreachable!("to_value always returns an object");
    };
    for key in ["worker_socket", "labels", "deadline_ms"] {
        if let Some(want) = fields.get(key) {
            write_field(doc.as_table_mut(), key, want, None);
        }
    }
}

fn write_field(table: &mut Table, key: &str, want: &Value, default: Option<&Value>) {
    if !table.contains_key(key) && default.is_some_and(|keep| keep == want) {
        return;
    }
    match table.get_mut(key) {
        Some(slot) => {
            if convert_item(slot, "").map_or(true, |found| &found != want) {
                *slot = Item::Value(to_toml_value(want));
            }
        }
        None => {
            table.insert(key, Item::Value(to_toml_value(want)));
        }
    }
}

fn write_cameras(doc: &mut DocumentMut, config: &Config) {
    let mut saved: Vec<Table> = doc
        .get("cameras")
        .and_then(Item::as_array_of_tables)
        .map_or_else(Vec::new, |existing| existing.iter().cloned().collect());
    let mut rebuilt = ArrayOfTables::new();
    for camera in &config.cameras {
        let found = saved
            .iter()
            .position(|table| camera_name(table) == Some(camera.name.as_str()));
        let mut table = found.map_or_else(Table::new, |index| saved.swap_remove(index));
        write_camera(&mut table, camera);
        rebuilt.push(table);
    }
    // The encoder sorts sub-tables by position; stride the positions so each area follows its camera.
    for (index, table) in rebuilt.iter_mut().enumerate() {
        let slot = index.saturating_mul(2);
        table.set_position(isize::try_from(slot).ok());
        if let Some(area) = table.get_mut("area").and_then(Item::as_table_mut) {
            area.set_position(isize::try_from(slot.saturating_add(1)).ok());
        }
    }
    if let Some(slot) = doc.get_mut("cameras") {
        *slot = Item::ArrayOfTables(rebuilt);
    } else {
        doc.as_table_mut()
            .insert("cameras", Item::ArrayOfTables(rebuilt));
    }
}

fn camera_name(table: &Table) -> Option<&str> {
    table.get("name").and_then(Item::as_str)
}

fn write_camera(camera_table: &mut Table, camera: &CameraConfig) {
    write_field(
        camera_table,
        "name",
        &Value::String(camera.name.clone()),
        None,
    );
    write_field(
        camera_table,
        "width",
        &Value::from(u64::from(camera.width)),
        None,
    );
    write_field(
        camera_table,
        "height",
        &Value::from(u64::from(camera.height)),
        None,
    );
    write_field(
        camera_table,
        "fps",
        &Value::from(u64::from(camera.fps)),
        None,
    );
    let threshold = Value::from(camera.confirm_threshold);
    write_field(
        camera_table,
        "confirm_threshold",
        &threshold,
        Some(&Value::from(DEFAULT_CONFIRM_THRESHOLD)),
    );
    match &camera.area {
        Some(area) => write_area(camera_table, area),
        None => {
            camera_table.remove("area");
        }
    }
}

fn write_area(camera_table: &mut Table, area: &Area) {
    if let Some(existing) = camera_table.get_mut("area").and_then(Item::as_table_mut) {
        fill_area(existing, area);
    } else {
        let mut table = Table::new();
        fill_area(&mut table, area);
        camera_table.insert("area", Item::Table(table));
    }
}

fn fill_area(area_table: &mut Table, area: &Area) {
    write_field(area_table, "name", &Value::String(area.name.clone()), None);
    let polygon: Vec<Value> = area
        .polygon
        .iter()
        .map(|(x, y)| Value::Array(vec![Value::from(*x), Value::from(*y)]))
        .collect();
    write_field(area_table, "polygon", &Value::Array(polygon), None);
    write_field(
        area_table,
        "pad_pixels",
        &Value::from(u64::from(area.pad_pixels)),
        Some(&Value::from(u64::from(DEFAULT_PAD_PIXELS))),
    );
}

fn order_document(doc: &mut DocumentMut) {
    sort_into_declared_order(doc.as_table_mut(), CONFIG_KEYS);
    if let Some(cameras) = doc
        .get_mut("cameras")
        .and_then(Item::as_array_of_tables_mut)
    {
        for camera in cameras.iter_mut() {
            sort_into_declared_order(camera, CAMERA_KEYS);
            if let Some(area) = camera.get_mut("area").and_then(Item::as_table_mut) {
                sort_into_declared_order(area, AREA_KEYS);
            }
        }
    }
}

fn sort_into_declared_order(table: &mut Table, order: &[&str]) {
    let rank = |key: &Key| {
        order
            .iter()
            .position(|name| *key == **name)
            .unwrap_or(usize::MAX)
    };
    table.sort_values_by(|first, _, second, _| rank(first).cmp(&rank(second)));
}

fn to_toml_value(want: &Value) -> toml_edit::Value {
    match want {
        Value::String(text) => toml_edit::Value::from(text.as_str()),
        Value::Bool(flag) => toml_edit::Value::from(*flag),
        Value::Number(number) => number.as_i64().map_or_else(
            || toml_edit::Value::from(number.as_f64().unwrap_or_default()),
            toml_edit::Value::from,
        ),
        Value::Array(entries) => {
            let mut array = toml_edit::Array::new();
            for entry in entries {
                array.push(to_toml_value(entry));
            }
            toml_edit::Value::Array(array)
        }
        Value::Object(entries) => {
            let mut inline = toml_edit::InlineTable::new();
            for (key, entry) in entries {
                inline.insert(key.as_str(), to_toml_value(entry));
            }
            toml_edit::Value::InlineTable(inline)
        }
        Value::Null => unreachable!("a validated config holds no null"),
    }
}
