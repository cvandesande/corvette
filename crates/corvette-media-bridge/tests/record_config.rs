use corvette_media_bridge::record_config::{self, CAMERA_TOML_PATH_ENV_VAR};
use std::path::PathBuf;

#[test]
fn valid_file_gives_records_defaults_and_one_missing_main_warning() {
    let text = r#"
[[cameras]]
name = "front"
enabled = false

[cameras.record]
enabled = true
piece_seconds = 8
retention_days = 14

[cameras.source.main]
host = "127.0.0.1"

[[cameras]]
name = "living"

[cameras.source.sub]
host = "127.0.0.1"

[storage]
retention_enforce = true

[storage.block]
path = "/srv/corvette"
"#;
    let config = record_config::parse(text).expect("valid file parses");

    assert_eq!(config.cameras.len(), 2);
    let front = &config.cameras[0];
    assert_eq!(front.name, "front");
    assert!(!front.enabled);
    assert!(front.record_enabled);
    assert_eq!(front.piece_seconds, 8);
    assert_eq!(front.retention_days, 14);
    assert!(front.streams.main);
    assert!(!front.streams.sub);
    assert!(!front.records());
    let living = &config.cameras[1];
    assert_eq!(living.name, "living");
    assert!(living.enabled);
    assert!(living.record_enabled);
    assert_eq!(living.piece_seconds, 4);
    assert_eq!(living.retention_days, 7);
    assert!(!living.streams.main);
    assert!(living.streams.sub);
    assert!(!living.records());
    assert_eq!(
        config.warnings,
        vec!["camera living has no main stream; recording stays off".to_owned()]
    );
    assert_eq!(
        config.storage.block_path,
        Some(PathBuf::from("/srv/corvette"))
    );
    assert!(config.storage.retention_enforce);
}

#[test]
fn camera_without_record_table_uses_defaults_and_retention_defaults_to_dry_run() {
    let text = r#"
[[cameras]]
name = "front"

[cameras.source.main]
host = "127.0.0.1"

[storage.block]
path = "/srv/corvette"
"#;
    let config = record_config::parse(text).expect("defaults parse");

    let front = &config.cameras[0];
    assert!(front.enabled);
    assert!(front.record_enabled);
    assert_eq!(front.piece_seconds, 4);
    assert_eq!(front.retention_days, 7);
    assert!(front.records());
    assert!(config.warnings.is_empty());
    assert_eq!(
        config.storage.block_path,
        Some(PathBuf::from("/srv/corvette"))
    );
    assert!(!config.storage.retention_enforce);
}

#[test]
fn each_bad_setting_is_refused_naming_camera_and_key() {
    let refusals: &[(&str, &str, &str)] = &[
        (
            "[[cameras]]\nname = \"front\"\n[cameras.record]\npiece_seconds = 0\n",
            "front",
            "record.piece_seconds",
        ),
        (
            "[[cameras]]\nname = \"front\"\n[cameras.record]\npiece_seconds = 61\n",
            "front",
            "record.piece_seconds",
        ),
        (
            "[[cameras]]\nname = \"front\"\n[cameras.record]\nretention_days = -1\n",
            "front",
            "record.retention_days",
        ),
        (
            "[[cameras]]\nname = \"front\"\n[cameras.record]\nretention_days = 3651\n",
            "front",
            "record.retention_days",
        ),
        ("[[cameras]]\nname = \"\"\n", "file", "cameras[0].name"),
        (
            "[[cameras]]\nname = \"living room\"\n",
            "file",
            "cameras[0].name",
        ),
        (
            "[[cameras]]\nname = \"front\"\n\n[[cameras]]\nname = \"front\"\n",
            "front",
            "name",
        ),
        (
            "[[cameras]]\nname = \"front\"\nenabled = \"yes\"\n",
            "front",
            "enabled",
        ),
        (
            "[[cameras]]\nname = \"front\"\n\n[cameras.source.main]\nhost = \"127.0.0.1\"\n",
            "file",
            "storage.block.path",
        ),
    ];
    for (text, camera, key) in refusals {
        let error = record_config::parse(text)
            .err()
            .unwrap_or_else(|| panic!("expected a refusal, got Ok, for: {text}"))
            .to_string();
        assert!(error.contains(camera), "{error} must name {camera}");
        assert!(error.contains(key), "{error} must name {key}");
    }
}

#[test]
fn record_config_is_absent_when_the_env_var_is_unset() {
    if std::env::var_os(CAMERA_TOML_PATH_ENV_VAR).is_none() {
        assert!(
            record_config::load_from_env()
                .expect("an unset env var is not an error")
                .is_none()
        );
    }
}

#[test]
fn load_from_path_reads_a_file_and_names_it_when_missing() {
    let path = std::env::temp_dir().join(format!(
        "corvette-record-config-test-{}",
        std::process::id()
    ));
    let text = "[[cameras]]\nname = \"front\"\n\n[cameras.source.main]\nhost = \"127.0.0.1\"\n\n[storage.block]\npath = \"/srv/corvette\"\n";
    std::fs::write(&path, text).expect("write fixture");
    let config = record_config::load_from_path(&path).expect("fixture parses");
    std::fs::remove_file(&path).ok();

    assert!(config.cameras[0].records());

    let missing = std::env::temp_dir().join("corvette-record-config-missing-file");
    let error = record_config::load_from_path(&missing)
        .expect_err("a missing file is an error")
        .to_string();
    assert!(error.contains(&missing.display().to_string()));
}
