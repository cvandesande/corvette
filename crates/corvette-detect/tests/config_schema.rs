use corvette_detect::{SCHEMA_ID, SCHEMA_JSON};

#[test]
fn served_schema_copy_equals_the_file_on_disk() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/design/detection-config.schema.json"
    );
    let on_disk = std::fs::read(path).expect("schema file present");
    assert_eq!(SCHEMA_JSON.as_bytes(), on_disk.as_slice());
}

#[test]
fn served_schema_carries_its_id_dialect_and_closure() {
    let schema: serde_json::Value = serde_json::from_str(SCHEMA_JSON).expect("schema parses");
    assert_eq!(schema["$id"].as_str().expect("$id is a string"), SCHEMA_ID);
    assert_eq!(
        schema["$schema"].as_str().expect("$schema is a string"),
        "https://json-schema.org/draft/2020-12/schema"
    );
    assert_eq!(schema["additionalProperties"], serde_json::json!(false));
}

#[test]
fn schema_id_equals_the_contract_serving_path() {
    assert_eq!(SCHEMA_ID, "/corvette/api/v1/detection/config/schema");
}
