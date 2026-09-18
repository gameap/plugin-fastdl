//! GameAP plugin for configuring the FastDL service and publishing game content.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod domain;
pub mod handlers;
pub mod host_api;
pub mod http;
pub mod router;
pub mod services;
pub mod shell;

use gameap_plugin_sdk::proto::gameap::plugin as pb;
use gameap_plugin_sdk::{Plugin, PluginError, register_plugin};

use crate::host_api::HostApi;

pub const PLUGIN_ID: &str = "i3z7ix336msd4";
pub const REQUIRED_PERMISSIONS: &[&str] =
    &["files", "listen_events", "manage_servers", "node_commands"];

const FRONTEND_JS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/plugin.js"));
const FRONTEND_CSS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/plugin.css"));

pub struct FastdlPlugin<H> {
    host: H,
}

impl<H> FastdlPlugin<H> {
    pub fn new(host: H) -> Self {
        Self { host }
    }
}

impl<H: HostApi> Plugin for FastdlPlugin<H> {
    fn get_info(&mut self, _req: pb::GetInfoRequest) -> Result<pb::PluginInfo, PluginError> {
        Ok(pb::PluginInfo {
            id: PLUGIN_ID.into(),
            name: "FastDL".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "Secure content downloads for GoldSource and Source servers".into(),
            author: "GameAP".into(),
            api_version: "1".into(),
            required_permissions: REQUIRED_PERMISSIONS
                .iter()
                .map(|permission| (*permission).into())
                .collect(),
            ..Default::default()
        })
    }

    fn initialize(
        &mut self,
        _req: pb::InitializeRequest,
    ) -> Result<pb::InitializeResponse, PluginError> {
        Ok(pb::InitializeResponse {
            result: Some(gameap_plugin_sdk::ok_result()),
        })
    }

    fn shutdown(&mut self, _req: pb::ShutdownRequest) -> Result<pb::ShutdownResponse, PluginError> {
        Ok(pb::ShutdownResponse {
            result: Some(gameap_plugin_sdk::ok_result()),
        })
    }

    fn get_http_routes(
        &mut self,
        _req: pb::GetHttpRoutesRequest,
    ) -> Result<pb::GetHttpRoutesResponse, PluginError> {
        Ok(pb::GetHttpRoutesResponse {
            routes: router::http_routes(),
        })
    }

    fn handle_http_request(
        &mut self,
        request: pb::HttpRequest,
    ) -> Result<pb::HttpResponse, PluginError> {
        Ok(router::dispatch(&mut self.host, &request))
    }

    fn get_subscribed_events(
        &mut self,
        _req: pb::GetSubscribedEventsRequest,
    ) -> Result<pb::GetSubscribedEventsResponse, PluginError> {
        Ok(pb::GetSubscribedEventsResponse {
            events: vec![
                pb::EventType::ServerDeleted as i32,
                pb::EventType::ServerUpdated as i32,
                pb::EventType::DaemonTaskCompleted as i32,
                pb::EventType::DaemonTaskFailed as i32,
            ],
        })
    }

    fn handle_event(&mut self, event: pb::Event) -> Result<pb::EventResult, PluginError> {
        Ok(handlers::events::handle(&mut self.host, &event))
    }

    fn get_server_abilities(
        &mut self,
        _req: pb::GetServerAbilitiesRequest,
    ) -> Result<pb::GetServerAbilitiesResponse, PluginError> {
        Ok(pb::GetServerAbilitiesResponse {
            abilities: ["fastdl-view", "fastdl-manage"]
                .iter()
                .map(|name| pb::ServerAbility {
                    name: (*name).into(),
                    title: format!("plugins.i3z7ix336msd4.abilities.{name}"),
                })
                .collect(),
        })
    }

    fn get_frontend_bundle(
        &mut self,
        _req: pb::GetFrontendBundleRequest,
    ) -> Result<pb::GetFrontendBundleResponse, PluginError> {
        Ok(pb::GetFrontendBundleResponse {
            bundle: FRONTEND_JS.to_vec(),
            has_bundle: !FRONTEND_JS.is_empty(),
            styles: FRONTEND_CSS.to_vec(),
            has_styles: !FRONTEND_CSS.is_empty(),
        })
    }
}

register_plugin!(
    FastdlPlugin<host_api::WasmHost>,
    FastdlPlugin::new(host_api::WasmHost)
);
