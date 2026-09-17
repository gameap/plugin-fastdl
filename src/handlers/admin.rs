use crate::host_api::HostApi;
use crate::http::{ApiResult, json_response};
use crate::router::RequestParts;
use crate::services::admin;

use super::auth;

pub fn list_nodes<H: HostApi>(host: &mut H, parts: &RequestParts) -> ApiResult {
    auth::require_admin(host, parts)?;

    let response = admin::list_nodes(host)?;
    Ok(json_response(200, &response))
}
