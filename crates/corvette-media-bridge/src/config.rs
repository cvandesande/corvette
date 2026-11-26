use corvette_rtsp_client::client::CameraConfig;
use corvette_rtsp_client::session::Credentials;
use serde::Deserialize;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;

pub const CONFIG_PATH_ENV_VAR: &str = "CORVETTE_MEDIA_BRIDGE_CONFIG";

#[derive(Debug, Clone, Deserialize)]
pub struct CameraSpec {
    pub name: String,
    pub host: IpAddr,
    pub port: u16,
    pub path: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    pub channel_capacity: Option<usize>,
}

impl CameraSpec {
    #[must_use]
    pub fn to_camera_config(&self) -> CameraConfig {
        let credentials = Credentials::new(self.username.clone(), self.password.clone());
        let config = CameraConfig::new(
            self.name.clone(),
            self.host,
            self.port,
            self.path.clone(),
            credentials,
        );
        match self.channel_capacity {
            Some(capacity) => config.channel_capacity(capacity),
            None => config,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub rtsp_bind_addr: SocketAddr,
    pub fmp4_ws_bind_addr: SocketAddr,
    pub hls_bind_addr: SocketAddr,
    pub cameras: Vec<CameraSpec>,
}

#[derive(Debug)]
pub enum ConfigError {
    MissingEnvVar(&'static str),
    Read {
        path: String,
        source: std::io::Error,
    },
    Parse {
        path: String,
        source: serde_json::Error,
    },
    DuplicateCameraName(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingEnvVar(name) => write!(f, "environment variable {name} is not set"),
            Self::Read { path, source } => write!(f, "reading config file {path}: {source}"),
            Self::Parse { path, source } => {
                write!(f, "parsing config file {path} as JSON: {source}")
            }
            Self::DuplicateCameraName(name) => {
                write!(
                    f,
                    "config file declares camera name {name:?} more than once"
                )
            }
        }
    }
}

impl std::error::Error for ConfigError {}

/// # Errors
/// unset var, unreadable file, invalid JSON, or duplicate camera names
pub fn load_from_env() -> Result<Config, ConfigError> {
    let path = std::env::var(CONFIG_PATH_ENV_VAR)
        .map_err(|_| ConfigError::MissingEnvVar(CONFIG_PATH_ENV_VAR))?;
    load_from_path(&path)
}

/// # Errors
/// See [`load_from_env`].
pub fn load_from_path(path: impl AsRef<Path>) -> Result<Config, ConfigError> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.display().to_string(),
        source,
    })?;
    let config: Config = serde_json::from_str(&text).map_err(|source| ConfigError::Parse {
        path: path.display().to_string(),
        source,
    })?;

    let mut seen = std::collections::HashSet::new();
    for camera in &config.cameras {
        if !seen.insert(camera.name.as_str()) {
            return Err(ConfigError::DuplicateCameraName(camera.name.clone()));
        }
    }

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_minimal_config() {
        let json = r#"{
            "rtsp_bind_addr": "127.0.0.1:8554",
            "fmp4_ws_bind_addr": "127.0.0.1:8555",
            "hls_bind_addr": "127.0.0.1:8556",
            "cameras": [
                { "name": "front_door", "host": "192.168.1.50", "port": 554, "path": "/Streaming/Channels/101", "username": "admin", "password": "secret" }
            ]
        }"#;
        let dir = std::env::temp_dir().join(format!("g1-config-test-{}", std::process::id()));
        std::fs::write(&dir, json).expect("write fixture");
        let config = load_from_path(&dir).expect("parses");
        std::fs::remove_file(&dir).ok();

        assert_eq!(config.rtsp_bind_addr, "127.0.0.1:8554".parse().unwrap());
        assert_eq!(config.cameras.len(), 1);
        assert_eq!(config.cameras[0].name, "front_door");
        assert_eq!(config.cameras[0].channel_capacity, None);
    }

    #[test]
    fn rejects_an_unknown_top_level_key() {
        let json = r#"{
            "rtsp_bind_addr": "127.0.0.1:8554",
            "fmp4_ws_bind_addr": "127.0.0.1:8555",
            "hls_bind_addr": "127.0.0.1:8556",
            "retired_section": {},
            "cameras": [
                { "name": "front_door", "host": "192.168.1.50", "port": 554, "path": "/a" }
            ]
        }"#;
        let dir =
            std::env::temp_dir().join(format!("g1-config-test-unknown-{}", std::process::id()));
        std::fs::write(&dir, json).expect("write fixture");
        let err = load_from_path(&dir).expect_err("unknown key must be rejected");
        std::fs::remove_file(&dir).ok();

        match err {
            ConfigError::Parse { source, .. } => {
                assert!(source.to_string().contains("unknown field"));
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn rejects_duplicate_camera_names() {
        let json = r#"{
            "rtsp_bind_addr": "127.0.0.1:8554",
            "fmp4_ws_bind_addr": "127.0.0.1:8555",
            "hls_bind_addr": "127.0.0.1:8556",
            "cameras": [
                { "name": "dup", "host": "127.0.0.1", "port": 554, "path": "/a" },
                { "name": "dup", "host": "127.0.0.1", "port": 555, "path": "/b" }
            ]
        }"#;
        let dir = std::env::temp_dir().join(format!("g1-config-test-dup-{}", std::process::id()));
        std::fs::write(&dir, json).expect("write fixture");
        let err = load_from_path(&dir).expect_err("duplicate names must be rejected");
        std::fs::remove_file(&dir).ok();

        assert!(matches!(err, ConfigError::DuplicateCameraName(name) if name == "dup"));
    }
}
