//! Game server settings and the transition between published configurations.

use crate::domain::{NodeConfig, ServerInput, ServerResponse, ServerState, SetupStatus};
use crate::host_api::{HostApi, ServerInfo};
use crate::http::ApiError;

use super::{game_config, node_setup, paths, store, sync};

pub fn get_server<H: HostApi>(host: &mut H, server_id: u64) -> Result<ServerInfo, ApiError> {
    host.get_server(server_id)?
        .ok_or_else(|| ApiError::not_found("Game server not found"))
}

pub fn view<H: HostApi>(
    host: &mut H,
    server: &ServerInfo,
    can_manage: bool,
) -> Result<ServerResponse, ApiError> {
    let stored = store::get_server_state(host, server.id)?;
    let (defaults, supported) = ServerInput::for_game(&server.game_id);
    let settings = stored
        .as_ref()
        .map(|state| state.settings.clone())
        .unwrap_or(defaults);
    let config = store::get_config(host, server.node_id)?;
    let token = stored.as_ref().map_or("", |state| state.token.as_str());
    let download_url = paths::download_url(&config, token);
    let status = store::get_status(host, server.node_id)?;
    let node_ready = status.status == SetupStatus::Installed;
    let synced = stored.as_ref().is_none_or(|state| state.synced);
    let mut warnings = Vec::new();

    if !node_ready {
        warnings.push("FastDL is not installed on this node".into());
    }
    if config.public_base_url.is_empty() {
        warnings.push("A public FastDL address must be configured by an administrator".into());
    }
    if !synced {
        warnings.push(
            "Changes have not been applied successfully. Save again or ask an administrator to synchronize the node."
                .into(),
        );
    }

    let configuration = if download_url.is_empty() || !settings.enabled {
        Vec::new()
    } else {
        vec![
            format!("sv_downloadurl \"{download_url}\""),
            "sv_allowdownload \"1\"".into(),
        ]
    };

    Ok(ServerResponse {
        server_name: server.name.clone(),
        settings,
        download_url,
        configuration,
        can_manage,
        synced,
        supported: supported || stored.is_some(),
        node_ready,
        warnings,
    })
}

pub fn update_server<H: HostApi>(
    host: &mut H,
    server: &ServerInfo,
    input: ServerInput,
) -> Result<(), ApiError> {
    input.validate()?;

    let node = node_setup::get_node(host, server.node_id)?;
    let config = store::get_config(host, server.node_id)?;
    let previous = store::get_server_state(host, server.id)?;

    if input.enabled {
        ensure_node_ready(host, server.node_id, &config)?;
    }

    let token = match &previous {
        Some(state) => state.token.clone(),
        None => host.random_string(32, Some("0123456789abcdef"))?,
    };
    paths::server_definition(server.id, &token)?;

    let mut next = ServerState {
        server_id: server.id,
        settings: input,
        token,
        node_id: server.node_id,
        server_dir: server.dir.replace('\\', "/"),
        synced: false,
    };
    paths::relative_game_root(&next.server_dir, &next.settings.game_dir)?;

    if let Some(previous) = &previous {
        revoke_previous(host, previous, &next)?;
    }

    // Keep the route disabled until both the game configuration and publication succeed.
    store::save_server_state(host, server.id, &next)?;
    sync::publish_server(host, &node, &next, false)?;

    if next.settings.manage_game_config && next.settings.enabled {
        let download_url = paths::download_url(&config, &next.token);
        game_config::apply(host, &next, Some(&download_url))?;
    }

    sync::publish_server(host, &node, &next, next.settings.enabled && server.enabled)?;

    next.synced = true;
    store::save_server_state(host, server.id, &next)
}

fn ensure_node_ready<H: HostApi>(
    host: &mut H,
    node_id: u64,
    config: &NodeConfig,
) -> Result<(), ApiError> {
    if store::get_status(host, node_id)?.status != SetupStatus::Installed {
        return Err(ApiError::conflict("Install FastDL on this node first"));
    }
    if config.public_base_url.is_empty() {
        return Err(ApiError::conflict(
            "Configure the public FastDL address first",
        ));
    }

    Ok(())
}

fn revoke_previous<H: HostApi>(
    host: &mut H,
    previous: &ServerState,
    next: &ServerState,
) -> Result<(), ApiError> {
    let mut pending = previous.clone();
    pending.synced = false;
    store::save_server_state(host, next.server_id, &pending)?;

    let node = node_setup::get_node(host, previous.node_id)?;
    sync::publish_server(host, &node, previous, false)?;

    if needs_config_cleanup(previous, next)? {
        game_config::apply(host, previous, None)?;
    }

    Ok(())
}

fn needs_config_cleanup(previous: &ServerState, next: &ServerState) -> Result<bool, ApiError> {
    // After a move, the former directory may belong to another server.
    if previous.node_id != next.node_id || previous.server_dir != next.server_dir {
        return Ok(false);
    }
    if !previous.settings.manage_game_config || (!previous.settings.enabled && previous.synced) {
        return Ok(false);
    }

    Ok(paths::game_config(previous)? != paths::game_config(next)?
        || !next.settings.manage_game_config
        || !next.settings.enabled)
}
