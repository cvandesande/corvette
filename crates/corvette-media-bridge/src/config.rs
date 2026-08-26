//! Process configuration: which cameras to dial, where to host the
//! RTSP-restream listener, and which `MoQ` relay to publish to.
//!
//! Corvette does not yet own configuration generally (see
//! `docs/design/architecture.md`'s "Risk boundary" section); this is a named,
//! owned stub matching issue #18's own C1 precedent, not a TODO without an
//! owner. A single env var (`CORVETTE_MEDIA_BRIDGE_CONFIG`) names a small JSON
//! file listing the cameras to dial and the two listeners this process hosts
//! (the RTSP-restream server and the `MoQ` relay to dial out to). Discovering
//! cameras from Frigate's own `config.yml` is explicitly out of this item's
//! scope (see the plan's own Scope guard).

use corvette_rtsp_client::client::CameraConfig;
use corvette_rtsp_client::session::Credentials;
use serde::Deserialize;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use url::Url;

/// The env var naming the JSON config file this process reads on startup.
pub const CONFIG_PATH_ENV_VAR: &str = "CORVETTE_MEDIA_BRIDGE_CONFIG";

/// One configured camera.
///
/// Exactly the fields needed to build a [`CameraConfig`] (issue #18's own
/// public API), plus a display name used to name both this camera's
/// RTSP-restream stream and its `MoQ` broadcast path.
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
    /// Overrides [`CameraConfig`]'s own default broadcast capacity (64
    /// frames) when present.
    pub channel_capacity: Option<usize>,
}

impl CameraSpec {
    /// Builds the [`CameraConfig`] issue #18's `corvette_rtsp_client::Client`
    /// needs to dial this camera.
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

/// Configuration for this process's own MoQ-publish role (D-5/D-9): which
/// relay to dial, and whether to skip TLS certificate verification.
///
/// `tls_disable_verify` exists only so this item's own tests (and any local
/// development run) can dial a relay presenting an ephemeral self-signed
/// certificate, the same `--tls-generate`/`--client-tls-disable-verify` shape
/// R1's own evidence already established as this project's local-relay test
/// convention. A deployed configuration is K1's job (INV-6: the external
/// listener terminates real TLS); this field defaults to `false` so a config
/// file that omits it never silently disables verification.
#[derive(Debug, Clone, Deserialize)]
pub struct MoqConfig {
    pub relay_url: Url,
    #[serde(default)]
    pub tls_disable_verify: bool,
}

/// Top-level process configuration, read from the file
/// [`CONFIG_PATH_ENV_VAR`] names.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Bind address for the single, whole-process `rtsp_restream::RtspServer`
    /// (D-8: one restream server for the whole process, per the plan's Do
    /// step 4).
    pub rtsp_bind_addr: SocketAddr,
    pub moq: MoqConfig,
    pub cameras: Vec<CameraSpec>,
}

/// Errors loading [`Config`].
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

/// Reads and parses the config file named by [`CONFIG_PATH_ENV_VAR`].
///
/// # Errors
///
/// Returns [`ConfigError`] if the env var is unset, the file can't be read,
/// the JSON doesn't parse, or two cameras share a name (this crate names both
/// the RTSP-restream stream and the `MoQ` broadcast path after a camera's own
/// name, so a collision would make one camera's traffic indistinguishable
/// from another's).
pub fn load_from_env() -> Result<Config, ConfigError> {
    let path = std::env::var(CONFIG_PATH_ENV_VAR)
        .map_err(|_| ConfigError::MissingEnvVar(CONFIG_PATH_ENV_VAR))?;
    load_from_path(&path)
}

/// Reads and parses a config file at `path`. Split out from
/// [`load_from_env`] so a test can point this item's own loader at a fixture
/// file directly rather than mutating the process environment.
///
/// # Errors
///
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
            "moq": { "relay_url": "https://127.0.0.1:4443/anon", "tls_disable_verify": true },
            "cameras": [
                { "name": "front_door", "host": "192.168.1.50", "port": 554, "path": "/Streaming/Channels/101", "username": "admin", "password": "secret" }
            ]
        }"#;
        let dir = std::env::temp_dir().join(format!("g1-config-test-{}", std::process::id()));
        std::fs::write(&dir, json).expect("write fixture");
        let config = load_from_path(&dir).expect("parses");
        std::fs::remove_file(&dir).ok();

        assert_eq!(config.rtsp_bind_addr, "127.0.0.1:8554".parse().unwrap());
        assert!(config.moq.tls_disable_verify);
        assert_eq!(config.cameras.len(), 1);
        assert_eq!(config.cameras[0].name, "front_door");
        assert_eq!(config.cameras[0].channel_capacity, None);
    }

    #[test]
    fn rejects_duplicate_camera_names() {
        let json = r#"{
            "rtsp_bind_addr": "127.0.0.1:8554",
            "moq": { "relay_url": "https://127.0.0.1:4443/anon" },
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
