use gameap_plugin_sdk::proto::gameap::plugin as pb;

use crate::host_api::HostApi;
use crate::services::{node_setup, sync};

pub fn handle<H: HostApi>(host: &mut H, event: &pb::Event) -> pb::EventResult {
    let result = match (&event.payload, event.r#type()) {
        (Some(pb::event::Payload::ServerEvent(payload)), pb::EventType::ServerDeleted) => payload
            .server
            .as_ref()
            .map(|server| sync::on_deleted(host, server.id, server.ds_id)),
        (Some(pb::event::Payload::ServerEvent(payload)), pb::EventType::ServerUpdated) => payload
            .server
            .as_ref()
            .map(|server| sync::on_updated(host, server.id)),
        (
            Some(pb::event::Payload::TaskEvent(payload)),
            pb::EventType::DaemonTaskCompleted | pb::EventType::DaemonTaskFailed,
        ) if payload.task_type == "cmdexec" => {
            Some(node_setup::get_status(host, payload.node_id).map(|_| ()))
        }
        _ => None,
    };

    let handled = result.is_some();
    if let Some(Err(error)) = result {
        host.log_error(&format!(
            "FastDL event synchronization failed: {}",
            error.message
        ));
    }

    pb::EventResult {
        handled,
        ..Default::default()
    }
}
