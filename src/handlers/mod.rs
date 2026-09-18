//! Route handlers keep parsing, authorization and HTTP responses separate from services.

pub mod admin;
mod auth;
pub mod events;
pub mod nodes;
pub mod servers;

#[cfg(test)]
mod tests;

use crate::host_api::HostApi;
use crate::http::ApiResult;
use crate::router::{RequestParts, RouteId};

pub fn handle<H: HostApi>(host: &mut H, route: RouteId, parts: &RequestParts) -> ApiResult {
    match route {
        RouteId::AdminNodes => admin::list_nodes(host, parts),
        RouteId::NodeConfigGet => nodes::get_config(host, parts),
        RouteId::NodeConfigUpdate => nodes::update_config(host, parts),
        RouteId::NodeStatus => nodes::status(host, parts),
        RouteId::NodeSetup => nodes::setup(host, parts),
        RouteId::NodeSync => nodes::sync(host, parts),
        RouteId::ServerFastdlGet => servers::get(host, parts),
        RouteId::ServerFastdlUpdate => servers::update(host, parts),
        RouteId::ServerFastdlConfigure => servers::configure(host, parts),
    }
}
