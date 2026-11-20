//! Lists the configuration paths a service restart picks up; every change is restart-required.

use serde_json::Value;

use crate::config::Config;

const TOP_KEYS: [&str; 3] = ["worker_socket", "labels", "deadline_ms"];

#[must_use]
pub fn classify(saved: &Config, proposed: &Config) -> Vec<String> {
    let saved_value = saved.to_value();
    let proposed_value = proposed.to_value();
    let mut restart_required = Vec::new();
    for key in TOP_KEYS {
        if saved_value.get(key) != proposed_value.get(key) {
            restart_required.push(key.to_owned());
        }
    }
    let saved_cameras = camera_entries(&saved_value);
    let proposed_cameras = camera_entries(&proposed_value);
    for (name, _) in &saved_cameras {
        if !proposed_cameras.iter().any(|(other, _)| other == name) {
            restart_required.push(format!("cameras[{name}]"));
        }
    }
    for (name, entry) in &proposed_cameras {
        let Some((_, saved_entry)) = saved_cameras.iter().find(|(other, _)| other == name) else {
            restart_required.push(format!("cameras[{name}]"));
            continue;
        };
        if saved_entry != entry {
            restart_required.push(format!("cameras[{name}]"));
        }
    }
    restart_required
}

fn camera_entries(value: &Value) -> Vec<(&str, &Value)> {
    let Some(cameras) = value.get("cameras").and_then(Value::as_array) else {
        return Vec::new();
    };
    cameras
        .iter()
        .filter_map(|entry| Some((entry.get("name")?.as_str()?, entry)))
        .collect()
}
