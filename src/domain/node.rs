//! Per-node configuration, installation state, and daemon configuration.

use std::net::SocketAddr;

use serde::{Deserialize, Serialize};

use crate::http::ApiError;

use super::validate_url;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeOs {
    Linux,
    Windows,
    Unsupported,
}

impl NodeOs {
    /// Older hosts do not report an OS; those nodes use the Linux installer.
    pub fn parse(os: &str) -> Self {
        match os.trim().to_ascii_lowercase().as_str() {
            "" | "linux" => Self::Linux,
            "windows" => Self::Windows,
            _ => Self::Unsupported,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeConfig {
    pub listen: String,
    pub public_base_url: String,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            listen: "0.0.0.0:8080".into(),
            public_base_url: String::new(),
        }
    }
}

impl NodeConfig {
    pub fn validate(&self) -> Result<(), ApiError> {
        let address = self
            .listen
            .parse::<SocketAddr>()
            .map_err(|_| ApiError::bad_request("Listen address must be an IP address and port"))?;

        if address.port() == 0 {
            return Err(ApiError::bad_request("Listen port must not be zero"));
        }

        if !self.public_base_url.is_empty() {
            validate_url(&self.public_base_url, false)?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupStatus {
    #[default]
    NotInstalled,
    Installing,
    Installed,
    Failed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeSetupStatus {
    pub status: SetupStatus,
    pub version: String,
    pub task_id: u64,
    /// Retained to finish installations started by the get-tool-based release.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub download_task_id: u64,
    pub error_message: String,
    pub started_at: i64,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SetupInput {
    pub download_url: String,
    pub sha256: String,
}

impl SetupInput {
    pub fn validate(&self) -> Result<(), ApiError> {
        if self.download_url.is_empty() && self.sha256.is_empty() {
            return Ok(());
        }
        if self.download_url.is_empty() || self.sha256.is_empty() {
            return Err(ApiError::bad_request(
                "Custom downloads require both an HTTPS URL and SHA256",
            ));
        }
        validate_url(&self.download_url, true)?;

        if self.sha256.len() != 64 || !self.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ApiError::bad_request(
                "SHA256 must contain exactly 64 hexadecimal characters",
            ));
        }

        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct NodesResponse {
    pub nodes: Vec<NodeResponse>,
}

#[derive(Debug, Serialize)]
pub struct NodeResponse {
    pub id: u64,
    pub name: String,
    pub os: String,
    pub status: SetupStatus,
    pub version: String,
    pub error_message: String,
    pub task_id: u64,
    pub enabled_servers: usize,
    pub config: NodeConfig,
}

/// Written to the daemon's config.json; the public URL is panel-only state.
#[derive(Debug, Serialize)]
pub struct DaemonConfig {
    pub version: u32,
    pub listen: String,
    pub servers_dir: String,
    pub cache_dir: String,
}

#[cfg(test)]
mod tests {
    use super::{NodeConfig, NodeOs, NodeSetupStatus, SetupStatus};

    #[test]
    fn setup_status_preserves_stored_json_fields() {
        let fixture = concat!(
            r#"{"status":"installing","version":"","task_id":42,"#,
            r#""error_message":"","started_at":1700000000}"#,
        );

        let status: NodeSetupStatus = serde_json::from_str(fixture).unwrap();

        assert_eq!(status.status, SetupStatus::Installing);
        assert_eq!(serde_json::to_string(&status).unwrap(), fixture);
    }

    #[test]
    fn absent_status_preserves_empty_fields() {
        let status = NodeSetupStatus::default();
        let fixture = concat!(
            r#"{"status":"not_installed","version":"","task_id":0,"#,
            r#""error_message":"","started_at":0}"#,
        );

        assert_eq!(serde_json::to_string(&status).unwrap(), fixture);
    }

    #[test]
    fn node_os_accepts_legacy_and_normalized_names() {
        for (name, expected) in [
            ("", NodeOs::Linux),
            (" Linux ", NodeOs::Linux),
            ("Windows", NodeOs::Windows),
            ("macos", NodeOs::Unsupported),
        ] {
            assert_eq!(NodeOs::parse(name), expected, "{name}");
        }
    }

    #[test]
    fn node_config_requires_an_ip_address_and_nonzero_port() {
        for listen in [
            "example.com:8080",
            "0.0.0.0:0",
            "0.0.0.0:00",
            "0.0.0.0:000",
            "[::]:0",
            "[::]:00",
            "[::1]:000",
        ] {
            let config = NodeConfig {
                listen: listen.into(),
                ..Default::default()
            };

            assert!(config.validate().is_err(), "{listen}");
        }
    }
}
