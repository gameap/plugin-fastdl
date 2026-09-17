//! Paths passed to the daemon or to the scoped game-configuration helper.

use crate::domain::{Engine, NodeConfig, NodeOs, ServerState, validate_relative};
use crate::host_api::NodeInfo;
use crate::http::ApiError;

pub const PLUGIN_DIR: &str = ".plugins/fastdla";
pub const CONFIG_PATH: &str = ".plugins/fastdla/config.json";

pub fn server_definition(server_id: u64, token: &str) -> Result<String, ApiError> {
    let valid_token = token.len() == 32
        && token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if server_id == 0 || !valid_token {
        return Err(ApiError::internal("Invalid stored public token"));
    }

    Ok(format!("{PLUGIN_DIR}/servers.d/server-{server_id}.json"))
}

pub fn relative_game_root(server_dir: &str, game_dir: &str) -> Result<String, ApiError> {
    validate_relative(server_dir, false)?;
    validate_relative(game_dir, true)?;

    if game_dir.is_empty() {
        return Ok(server_dir.into());
    }

    Ok(format!("{server_dir}/{game_dir}"))
}

pub fn absolute(node: &NodeInfo, relative: &str) -> Result<String, ApiError> {
    let work_path = node.work_path.trim_end_matches(['/', '\\']);
    let windows = node.os_kind() == NodeOs::Windows;
    let rooted = if windows {
        work_path.len() > 2
            && work_path.as_bytes()[1] == b':'
            && matches!(work_path.as_bytes()[2], b'/' | b'\\')
    } else {
        work_path.starts_with('/')
    };
    let invalid_character = work_path
        .chars()
        .any(|character| character.is_control() || matches!(character, '"' | '\'' | '{' | '}'));
    if !rooted || invalid_character {
        return Err(ApiError::bad_request("Node work directory is invalid"));
    }

    if windows {
        return Ok(format!(
            "{}\\{}",
            work_path.replace('/', "\\"),
            relative.replace('/', "\\"),
        ));
    }

    Ok(format!("{work_path}/{relative}"))
}

pub fn binary(node: &NodeInfo) -> Result<String, ApiError> {
    let filename = match node.os_kind() {
        NodeOs::Windows => "gameap-fastdl.exe",
        NodeOs::Linux | NodeOs::Unsupported => "gameap-fastdl",
    };

    absolute(node, &format!("{PLUGIN_DIR}/{filename}"))
}

pub fn game_config(state: &ServerState) -> Result<String, ApiError> {
    let root = relative_game_root(&state.server_dir, &state.settings.game_dir)?;
    let filename = match state.settings.engine {
        Engine::Source => "cfg/server.cfg",
        Engine::Goldsource => "server.cfg",
    };

    Ok(format!("{root}/{filename}"))
}

pub fn download_url(config: &NodeConfig, token: &str) -> String {
    if config.public_base_url.is_empty() || token.is_empty() {
        return String::new();
    }

    format!("{}/{token}/", config.public_base_url.trim_end_matches('/'))
}
