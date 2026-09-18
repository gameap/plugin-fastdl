//! Administrative overview of FastDL installations.

use crate::domain::{NodeResponse, NodesResponse};
use crate::host_api::HostApi;
use crate::http::ApiError;

use super::{node_setup, store};

pub fn list_nodes<H: HostApi>(host: &mut H) -> Result<NodesResponse, ApiError> {
    let mut nodes = Vec::new();

    for node in host.find_nodes()? {
        let status = node_setup::get_status(host, node.id)?;
        let config = store::get_config(host, node.id)?;
        let mut enabled_servers = 0;

        for server in host.find_servers(&[], &[node.id])? {
            if store::get_server_state(host, server.id)?.is_some_and(|state| state.settings.enabled)
            {
                enabled_servers += 1;
            }
        }

        nodes.push(NodeResponse {
            id: node.id,
            name: node.name,
            os: node.os,
            status: status.status,
            version: status.version,
            error_message: status.error_message,
            task_id: status.task_id,
            enabled_servers,
            config,
        });
    }

    Ok(NodesResponse { nodes })
}
