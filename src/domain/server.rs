//! Game server settings and the restricted configuration sent to FastDL.

use serde::{Deserialize, Serialize};

use crate::http::ApiError;

use super::validate_relative;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Goldsource,
    Source,
}

impl Engine {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Goldsource => "goldsource",
            Self::Source => "source",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerInput {
    pub enabled: bool,
    pub autoindex: bool,
    pub engine: Engine,
    pub game_dir: String,
    #[serde(default = "default_true")]
    pub manage_game_config: bool,
    #[serde(default = "default_true")]
    pub generate_bz2: bool,
}

fn default_true() -> bool {
    true
}

impl ServerInput {
    pub fn validate(&self) -> Result<(), ApiError> {
        validate_relative(&self.game_dir, true)?;

        if self.game_dir.split('/').any(is_internal_directory) {
            return Err(ApiError::bad_request(
                "Internal server directories cannot be used as a game directory",
            ));
        }

        Ok(())
    }

    pub fn for_game(game: &str) -> (Self, bool) {
        let (engine, game_dir, supported) = match game.to_ascii_lowercase().as_str() {
            "cstrike" | "cs16" | "cs" => (Engine::Goldsource, "cstrike", true),
            "valve" | "hldm" | "hl" => (Engine::Goldsource, "valve", true),
            "czero" => (Engine::Goldsource, "czero", true),
            "dod" => (Engine::Goldsource, "dod", true),
            "tfc" => (Engine::Goldsource, "tfc", true),
            "css" | "cstrike_source" | "cs-source" => (Engine::Source, "cstrike", true),
            "csgo" => (Engine::Source, "csgo", true),
            "tf2" | "tf" => (Engine::Source, "tf", true),
            "gmod" | "garrysmod" => (Engine::Source, "garrysmod", true),
            "hl2dm" | "hl2mp" => (Engine::Source, "hl2mp", true),
            "l4d" | "left4dead" => (Engine::Source, "left4dead", true),
            "l4d2" | "left4dead2" => (Engine::Source, "left4dead2", true),
            "dods" | "dod_source" => (Engine::Source, "dod", true),
            _ => (Engine::Source, "", false),
        };

        let settings = Self {
            enabled: false,
            autoindex: false,
            engine,
            game_dir: game_dir.into(),
            manage_game_config: true,
            generate_bz2: true,
        };

        (settings, supported)
    }
}

fn is_internal_directory(directory: &str) -> bool {
    matches!(
        directory.to_ascii_lowercase().as_str(),
        "cfg"
            | "config"
            | "configs"
            | "logs"
            | "log"
            | "addons"
            | "plugins"
            | "bin"
            | "cache"
            | "data"
            | "download"
            | "downloads"
            | "backup"
            | "backups"
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerState {
    pub server_id: u64,
    pub settings: ServerInput,
    pub token: String,
    pub node_id: u64,
    pub server_dir: String,
    #[serde(default)]
    pub synced: bool,
}

/// Public settings omit internal IDs, filesystem roots, and the stored token.
#[derive(Debug, Serialize)]
pub struct ServerResponse {
    pub server_name: String,
    #[serde(flatten)]
    pub settings: ServerInput,
    pub download_url: String,
    pub configuration: Vec<String>,
    pub can_manage: bool,
    pub synced: bool,
    pub supported: bool,
    pub node_ready: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ServerDropIn {
    pub token: String,
    pub root: String,
    pub engine: Engine,
    pub enabled: bool,
    pub autoindex: bool,
    pub generate_bz2: bool,
}

#[derive(Debug, Deserialize)]
pub struct ConfigureResponse {
    pub configured: bool,
}

#[cfg(test)]
mod tests {
    use super::{Engine, ServerInput, ServerResponse, ServerState};

    #[test]
    fn omitted_automatic_settings_remain_enabled() {
        let fixture = concat!(
            r#"{"enabled":false,"autoindex":false,"engine":"source","#,
            r#""game_dir":"cstrike"}"#,
        );

        let settings: ServerInput = serde_json::from_str(fixture).unwrap();

        assert!(settings.manage_game_config);
        assert!(settings.generate_bz2);
    }

    #[test]
    fn legacy_state_is_unsynchronized_until_reapplied() {
        let fixture = concat!(
            r#"{"server_id":1,"settings":{"enabled":true,"autoindex":false,"#,
            r#""engine":"source","game_dir":"cstrike"},"token":"opaque","#,
            r#""node_id":2,"server_dir":"servers/css"}"#,
        );

        let state: ServerState = serde_json::from_str(fixture).unwrap();

        assert!(!state.synced);
    }

    #[test]
    fn public_response_keeps_settings_at_the_top_level() {
        let (settings, _) = ServerInput::for_game("css");
        let response = ServerResponse {
            server_name: "Counter-Strike: Source".into(),
            settings,
            download_url: String::new(),
            configuration: Vec::new(),
            can_manage: true,
            synced: true,
            supported: true,
            node_ready: false,
            warnings: Vec::new(),
        };

        let value = serde_json::to_value(response).unwrap();

        assert_eq!(value["engine"], "source");
        assert_eq!(value["game_dir"], "cstrike");
        assert_eq!(value["manage_game_config"], true);
        assert!(value.get("settings").is_none());
        assert!(value.get("server_id").is_none());
        assert!(value.get("node_id").is_none());
        assert!(value.get("token").is_none());
    }

    #[test]
    fn internal_directories_are_rejected_case_insensitively() {
        for game_dir in ["cfg", "cstrike/Addons", "Plugins", "cstrike/Backups"] {
            let settings = ServerInput {
                game_dir: game_dir.into(),
                ..ServerInput::for_game("css").0
            };

            assert!(settings.validate().is_err(), "{game_dir}");
        }
    }

    #[test]
    fn game_defaults_preserve_engine_and_directory_mappings() {
        for (game, expected_engine, expected_directory) in [
            ("CS16", Engine::Goldsource, "cstrike"),
            ("hl", Engine::Goldsource, "valve"),
            ("css", Engine::Source, "cstrike"),
            ("tf2", Engine::Source, "tf"),
            ("l4d2", Engine::Source, "left4dead2"),
        ] {
            let (settings, supported) = ServerInput::for_game(game);

            assert!(supported, "{game}");
            assert_eq!(settings.engine, expected_engine, "{game}");
            assert_eq!(settings.game_dir, expected_directory, "{game}");
        }

        let (settings, supported) = ServerInput::for_game("unknown");

        assert!(!supported);
        assert!(!settings.enabled);
        assert!(settings.game_dir.is_empty());
    }
}
