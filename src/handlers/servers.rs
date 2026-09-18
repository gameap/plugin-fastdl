use crate::domain::ServerInput;
use crate::host_api::HostApi;
use crate::http::{ApiResult, json_response, parse_json_body, parse_u64_param};
use crate::router::RequestParts;
use crate::services::servers;

use super::auth;

pub fn get<H: HostApi>(host: &mut H, parts: &RequestParts) -> ApiResult {
    let server_id = parse_u64_param(&parts.params, "serverId")?;
    let access = auth::authorize_server(host, parts, server_id)?;

    let server = servers::get_server(host, server_id)?;
    let response = servers::view(host, &server, access.can_manage)?;
    Ok(json_response(200, &response))
}

pub fn update<H: HostApi>(host: &mut H, parts: &RequestParts) -> ApiResult {
    let server_id = parse_u64_param(&parts.params, "serverId")?;
    let access = auth::authorize_server(host, parts, server_id)?;
    if !access.can_manage {
        return Err(auth::forbidden());
    }

    let server = servers::get_server(host, server_id)?;
    let input: ServerInput = parse_json_body(parts.body)?;
    servers::update_server(host, &server, input)?;

    let response = servers::view(host, &server, access.can_manage)?;
    Ok(json_response(200, &response))
}

pub fn configure<H: HostApi>(host: &mut H, parts: &RequestParts) -> ApiResult {
    let server_id = parse_u64_param(&parts.params, "serverId")?;
    let access = auth::authorize_server(host, parts, server_id)?;
    if !access.can_manage {
        return Err(auth::forbidden());
    }

    let server = servers::get_server(host, server_id)?;
    let configuration = servers::configure_server(host, &server)?;
    Ok(json_response(
        200,
        &serde_json::json!({
            "configured": true,
            "configuration": configuration,
        }),
    ))
}
