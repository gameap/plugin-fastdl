//! Invokes the Go helper so game files are never accessed through the broader
//! node file API. The helper opens every path component without following links.

use crate::domain::{ConfigureResponse, NodeOs, ServerState, validate_relative};
use crate::host_api::{CommandOutput, HostApi};
use crate::http::ApiError;
use crate::services::{node_setup, paths};
use crate::shell::{shell_join, shell_join_windows};

const MAX_DIAGNOSTIC_CHARS: usize = 2048;

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
    let output = match host.execute_command(state.node_id, &command) {
        Ok(output) => output,
        Err(error) => {
            host.log_error(&format!(
                "[fastdl] Game configuration failed: node_id={} server_id={} operation={} host_error={:?}",
                state.node_id,
                state.server_id,
                if download_url.is_some() { "apply" } else { "remove" },
                diagnostic_detail(error.message(), &state.token),
            ));
            return Err(error.into());
        }
    };
    if output.exit_code != 0 || output.error.is_some() {
        log_failure(host, state, &output, "the configuration command failed");
        return Err(ApiError::new(
            409,
            "GAME_CONFIG_UPDATE_FAILED",
            "FastDL changes were not applied because server.cfg could not be updated. Check the game directory and file permissions, then review the GameAP panel log and save again.",
        ));
    }

    if let Err(reason) = configuration_confirmation(&output.output) {
        log_failure(host, state, &output, reason);
        return Err(unverified_configuration());
    }

    Ok(())
}

fn configuration_confirmation(output: &str) -> Result<(), &'static str> {
    let response = match serde_json::from_str::<ConfigureResponse>(output.trim()) {
        Ok(response) => response,
        Err(_) => {
            // The daemon may wrap the helper's JSON line with a command prompt and exit status.
            let mut confirmations = output
                .lines()
                .filter_map(|line| serde_json::from_str::<ConfigureResponse>(line.trim()).ok());
            let response = confirmations
                .next()
                .ok_or("the command output has no valid configuration confirmation")?;
            if confirmations.next().is_some() {
                return Err("the command output has multiple configuration confirmations");
            }
            response
        }
    };
    if !response.configured {
        return Err("the helper did not confirm the configuration update");
    }
    Ok(())
}

fn log_failure<H: HostApi>(
    host: &mut H,
    state: &ServerState,
    output: &CommandOutput,
    reason: &str,
) {
    host.log_error(&format!(
        "[fastdl] Game configuration failed: node_id={} server_id={} exit_code={} error={:?} output={:?} reason={reason}",
        state.node_id,
        state.server_id,
        output.exit_code,
        output.error.as_deref().map(|error| diagnostic_detail(error, &state.token)),
        diagnostic_detail(&output.output, &state.token),
    ));
}

fn diagnostic_detail(value: &str, token: &str) -> String {
    let redacted = if token.is_empty() {
        value.to_owned()
    } else {
        value.replace(token, "[redacted]")
    };
    let mut characters = redacted.chars();
    let mut detail: String = characters.by_ref().take(MAX_DIAGNOSTIC_CHARS).collect();
    if characters.next().is_some() {
        detail.push_str(" [truncated]");
    }
    detail
}

fn unverified_configuration() -> ApiError {
    ApiError::new(
        502,
        "CONFIGURE_FAILED",
        "FastDL changes were not applied because the update to server.cfg could not be confirmed. The file may already have changed. Check the GameAP panel log before saving again.",
    )
}
