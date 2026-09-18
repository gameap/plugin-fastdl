use serde::Serialize;

use crate::domain::{NodeConfig, SetupInput};
use crate::host_api::HostApi;
use crate::http::{ApiResult, json_response, parse_json_body, parse_u64_param};
use crate::router::RequestParts;
use crate::services::{node_setup, sync};

use super::auth;

#[derive(Serialize)]
struct SyncResponse {
    synced: bool,
}

pub fn setup<H: HostApi>(host: &mut H, parts: &RequestParts) -> ApiResult {
    let node_id = parse_u64_param(&parts.params, "nodeId")?;
    auth::require_admin(host, parts)?;
    node_setup::get_node(host, node_id)?;
    let input: SetupInput = parse_json_body(parts.body)?;

    let status = node_setup::setup_node(host, node_id, input)?;
    Ok(json_response(202, &status))
}

pub fn status<H: HostApi>(host: &mut H, parts: &RequestParts) -> ApiResult {
    let node_id = parse_u64_param(&parts.params, "nodeId")?;
    auth::require_admin(host, parts)?;
    node_setup::get_node(host, node_id)?;

    let status = node_setup::get_status(host, node_id)?;
    Ok(json_response(200, &status))
}

pub fn get_config<H: HostApi>(host: &mut H, parts: &RequestParts) -> ApiResult {
    let node_id = parse_u64_param(&parts.params, "nodeId")?;
    auth::require_admin(host, parts)?;
    node_setup::get_node(host, node_id)?;

    let config = node_setup::get_config(host, node_id)?;
    Ok(json_response(200, &config))
}

pub fn update_config<H: HostApi>(host: &mut H, parts: &RequestParts) -> ApiResult {
    let node_id = parse_u64_param(&parts.params, "nodeId")?;
    auth::require_admin(host, parts)?;
    node_setup::get_node(host, node_id)?;
    let input: NodeConfig = parse_json_body(parts.body)?;

    node_setup::update_config(host, node_id, input)?;
    let config = node_setup::get_config(host, node_id)?;
    Ok(json_response(200, &config))
}

pub fn sync<H: HostApi>(host: &mut H, parts: &RequestParts) -> ApiResult {
    let node_id = parse_u64_param(&parts.params, "nodeId")?;
    auth::require_admin(host, parts)?;
    node_setup::get_node(host, node_id)?;

    sync::sync_node(host, node_id)?;
    Ok(json_response(200, &SyncResponse { synced: true }))
}
