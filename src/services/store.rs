//! Typed access to persistent settings and node registrations.
//!
//! Node registrations outlive server-scoped storage so deletion events can
//! still revoke published content after GameAP removes the server entity.

use serde::{Serialize, de::DeserializeOwned};

use crate::domain::{NodeConfig, NodeSetupStatus, ServerState};
use crate::host_api::{HostApi, StorageEntity};
use crate::http::ApiError;

const KEY_SERVER_STATE: &str = "fastdl:server";
const KEY_NODE_CONFIG: &str = "fastdl:config";
const KEY_NODE_STATUS: &str = "fastdl:status";
const KEY_REGISTRATION_PREFIX: &str = "fastdl:registration:";

fn load<H: HostApi, T: DeserializeOwned>(
    host: &mut H,
    key: &str,
    entity: StorageEntity,
) -> Result<Option<T>, ApiError> {
    host.storage_get(key, entity)?
        .map(|payload| {
            serde_json::from_slice(&payload)
                .map_err(|_| ApiError::internal("Stored FastDL settings are invalid"))
        })
        .transpose()
}

fn save<H: HostApi, T: Serialize>(
    host: &mut H,
    key: &str,
    entity: StorageEntity,
    value: &T,
) -> Result<(), ApiError> {
    let payload =
        serde_json::to_vec(value).map_err(|_| ApiError::internal("Settings encoding failed"))?;
    host.storage_set(key, entity, &payload)?;

    Ok(())
}

pub fn get_config<H: HostApi>(host: &mut H, node_id: u64) -> Result<NodeConfig, ApiError> {
    Ok(load(host, KEY_NODE_CONFIG, StorageEntity::node(node_id))?.unwrap_or_default())
}

pub fn save_config<H: HostApi>(
    host: &mut H,
    node_id: u64,
    config: &NodeConfig,
) -> Result<(), ApiError> {
    save(host, KEY_NODE_CONFIG, StorageEntity::node(node_id), config)
}

pub fn get_status<H: HostApi>(host: &mut H, node_id: u64) -> Result<NodeSetupStatus, ApiError> {
    Ok(load(host, KEY_NODE_STATUS, StorageEntity::node(node_id))?.unwrap_or_default())
}

pub fn save_status<H: HostApi>(
    host: &mut H,
    node_id: u64,
    status: &NodeSetupStatus,
) -> Result<(), ApiError> {
    save(host, KEY_NODE_STATUS, StorageEntity::node(node_id), status)
}

pub fn get_server_state<H: HostApi>(
    host: &mut H,
    server_id: u64,
) -> Result<Option<ServerState>, ApiError> {
    load(host, KEY_SERVER_STATE, StorageEntity::server(server_id))
}

pub fn save_server_state<H: HostApi>(
    host: &mut H,
    server_id: u64,
    state: &ServerState,
) -> Result<(), ApiError> {
    save(
        host,
        &registration_key(server_id),
        StorageEntity::node(state.node_id),
        state,
    )?;

    save(
        host,
        KEY_SERVER_STATE,
        StorageEntity::server(server_id),
        state,
    )
}

pub fn delete_server_state<H: HostApi>(host: &mut H, server_id: u64) -> Result<(), ApiError> {
    host.storage_delete(KEY_SERVER_STATE, StorageEntity::server(server_id))?;

    Ok(())
}

pub fn get_registration<H: HostApi>(
    host: &mut H,
    node_id: u64,
    server_id: u64,
) -> Result<Option<ServerState>, ApiError> {
    load(
        host,
        &registration_key(server_id),
        StorageEntity::node(node_id),
    )
}

pub fn list_registrations<H: HostApi>(
    host: &mut H,
    node_id: u64,
) -> Result<Vec<(String, ServerState)>, ApiError> {
    host.storage_list(KEY_REGISTRATION_PREFIX, StorageEntity::node(node_id))?
        .into_iter()
        .map(|(key, payload)| {
            let state = serde_json::from_slice(&payload)
                .map_err(|_| ApiError::internal("Stored FastDL registration is invalid"))?;

            Ok((key, state))
        })
        .collect()
}

pub fn delete_registration<H: HostApi>(
    host: &mut H,
    node_id: u64,
    server_id: u64,
) -> Result<(), ApiError> {
    delete_registration_key(host, node_id, &registration_key(server_id))
}

pub(super) fn delete_registration_key<H: HostApi>(
    host: &mut H,
    node_id: u64,
    key: &str,
) -> Result<(), ApiError> {
    host.storage_delete(key, StorageEntity::node(node_id))?;

    Ok(())
}

fn registration_key(server_id: u64) -> String {
    format!("{KEY_REGISTRATION_PREFIX}{server_id}")
}
