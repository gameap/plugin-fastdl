//! Invokes the Go helper so game files are never accessed through the broader
//! node file API. The helper opens every path component without following links.

use crate::domain::{ConfigureResponse, NodeOs, ServerState, validate_relative};
use crate::host_api::HostApi;
use crate::http::ApiError;
use crate::services::{node_setup, paths};
use crate::shell::{shell_join, shell_join_windows};

pub fn apply<H: HostApi>(
    host: &mut H,
    state: &ServerState,
    download_url: Option<&str>,
) -> Result<(), ApiError> {
    let node = node_setup::get_node(host, state.node_id)?;
    validate_relative(&state.server_dir, false)?;

    let root = paths::absolute(&node, &state.server_dir)?;
    let binary = paths::binary(&node)?;
    let mut args = vec![
        binary.as_str(),
        "configure",
        "--root",
        root.as_str(),
        "--game-dir",
        state.settings.game_dir.as_str(),
        "--engine",
        state.settings.engine.as_str(),
    ];
    if let Some(url) = download_url {
        args.extend(["--url", url]);
    }

    let command = match node.os_kind() {
        NodeOs::Windows => shell_join_windows(&args),
        NodeOs::Linux | NodeOs::Unsupported => shell_join(&args),
    };
    let output = host.execute_command(state.node_id, &command)?;
    if output.exit_code != 0 || output.error.is_some() {
        return Err(ApiError::conflict(
            "Cannot safely update server.cfg. Ensure the configuration exists, is a regular file inside this server, and the FastDL service can access it.",
        ));
    }

    let response: ConfigureResponse =
        serde_json::from_str(output.output.trim()).map_err(|_| unverified_configuration())?;
    if !response.configured {
        return Err(unverified_configuration());
    }

    Ok(())
}

fn unverified_configuration() -> ApiError {
    ApiError::new(
        502,
        "CONFIGURE_FAILED",
        "Game configuration update could not be verified",
    )
}
