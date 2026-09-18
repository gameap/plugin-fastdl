//! Publish daemon configuration and reconcile server lifecycle changes.

use serde::Serialize;

use crate::domain::{DaemonConfig, Engine, NodeConfig, ServerDropIn, ServerState, SetupStatus};
use crate::host_api::{HostApi, NodeInfo};
use crate::http::ApiError;

use super::{node_setup, paths, servers, store};

pub fn sync_node<H: HostApi>(host: &mut H, node_id: u64) -> Result<(), ApiError> {
    apply_node(host, node_id)?;

    if store::get_status(host, node_id)?.status == SetupStatus::Installed {
        let node = node_setup::get_node(host, node_id)?;
        node_setup::restart(host, &node)?;
    }

    Ok(())
}

pub(super) fn apply_node<H: HostApi>(host: &mut H, node_id: u64) -> Result<(), ApiError> {
    let node = node_setup::get_node(host, node_id)?;
    let config = store::get_config(host, node_id)?;
    write_node_config(host, node_id, &config)?;

    let servers = host.find_servers(&[], &[node_id])?;

    for (key, state) in store::list_registrations(host, node_id)? {
        if !servers.iter().any(|server| server.id == state.server_id) {
            publish_server(host, &node, &state, false)?;
            store::delete_registration_key(host, node_id, &key)?;
        }
    }

    for server in servers {
        if let Some(state) = store::get_server_state(host, server.id)? {
            servers::update_server(host, &server, state.settings)?;
        }
    }

    Ok(())
}

pub(super) fn write_node_config<H: HostApi>(
    host: &mut H,
    node_id: u64,
    config: &NodeConfig,
) -> Result<(), ApiError> {
    let daemon_config = DaemonConfig {
        version: 1,
        listen: config.listen.clone(),
        servers_dir: "servers.d".into(),
        cache_dir: "cache".into(),
    };

    upload_json(host, node_id, paths::CONFIG_PATH, &daemon_config)
}

pub(super) fn publish_server<H: HostApi>(
    host: &mut H,
    node: &NodeInfo,
    state: &ServerState,
    enabled: bool,
) -> Result<(), ApiError> {
    let game_root = paths::relative_game_root(&state.server_dir, &state.settings.game_dir)?;
    let definition = ServerDropIn {
        token: state.token.clone(),
        root: paths::absolute(node, &game_root)?,
        engine: state.settings.engine,
        enabled,
        autoindex: state.settings.autoindex,
        generate_bz2: state.settings.generate_bz2 && state.settings.engine == Engine::Source,
    };
    let path = paths::server_definition(state.server_id, &state.token)?;

    upload_json(host, node.id, &path, &definition)
}

fn upload_json<H: HostApi, T: Serialize>(
    host: &mut H,
    node_id: u64,
    path: &str,
    value: &T,
) -> Result<(), ApiError> {
    let payload = serde_json::to_vec_pretty(value)
        .map_err(|_| ApiError::internal("Configuration encoding failed"))?;
    host.upload(node_id, path, &payload, 0o600)?;

    Ok(())
}

pub fn on_deleted<H: HostApi>(
    host: &mut H,
    server_id: u64,
    event_node_id: u64,
) -> Result<(), ApiError> {
    let stored = store::get_server_state(host, server_id)?;
    let registered = if event_node_id == 0 {
        None
    } else {
        store::get_registration(host, event_node_id, server_id)?
    };
    let mut candidates: Vec<ServerState> = stored.into_iter().collect();

    if let Some(registration) = registered
        && !candidates
            .iter()
            .any(|state| state.node_id == registration.node_id)
    {
        candidates.push(registration);
    }

    for state in candidates {
        let node = node_setup::get_node(host, state.node_id)?;
        publish_server(host, &node, &state, false)?;
        store::delete_registration(host, state.node_id, server_id)?;
    }

    store::delete_server_state(host, server_id)
}

pub fn on_updated<H: HostApi>(host: &mut H, server_id: u64) -> Result<(), ApiError> {
    let Some(mut state) = store::get_server_state(host, server_id)? else {
        return Ok(());
    };
    let server = servers::get_server(host, server_id)?;

    if state.node_id != server.node_id
        || state.server_dir != server.dir.replace('\\', "/")
        || !server.enabled
    {
        let node = node_setup::get_node(host, state.node_id)?;
        publish_server(host, &node, &state, false)?;
        state.synced = false;
        store::save_server_state(host, server_id, &state)?;
    }

    servers::update_server(host, &server, state.settings)
}
