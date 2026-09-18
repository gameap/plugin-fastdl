use crate::host_api::{HostApi, HostApiError, StorageEntity};
use crate::http::ApiError;
use crate::router::RequestParts;

const ADMIN_ABILITY: &str = "admin roles & permissions";
const VIEW_ABILITY: &str = "plugin:i3z7ix336msd4:fastdl-view";
const MANAGE_ABILITY: &str = "plugin:i3z7ix336msd4:fastdl-manage";

pub struct ServerAccess {
    pub can_manage: bool,
}

pub fn require_admin<H: HostApi>(host: &mut H, parts: &RequestParts) -> Result<(), ApiError> {
    let user_id = authenticated_user_id(parts)?;
    if !is_admin(host, user_id)? {
        return Err(forbidden());
    }

    Ok(())
}

pub fn authorize_server<H: HostApi>(
    host: &mut H,
    parts: &RequestParts,
    server_id: u64,
) -> Result<ServerAccess, ApiError> {
    let user_id = authenticated_user_id(parts)?;
    let admin = is_admin(host, user_id)?;
    let entity = StorageEntity::server(server_id);
    let can_manage = admin
        || host
            .authz_can_any_for_entity(user_id, entity, &[MANAGE_ABILITY])
            .map_err(authorization_unavailable)?;
    let can_view = can_manage
        || host
            .authz_can_any_for_entity(user_id, entity, &[VIEW_ABILITY])
            .map_err(authorization_unavailable)?;
    if !can_view {
        return Err(forbidden());
    }

    Ok(ServerAccess { can_manage })
}

pub fn forbidden() -> ApiError {
    ApiError::forbidden("You do not have access to these FastDL settings")
}

fn authenticated_user_id(parts: &RequestParts) -> Result<u64, ApiError> {
    parts
        .user_id
        .filter(|id| *id != 0)
        .ok_or_else(|| ApiError::new(401, "UNAUTHENTICATED", "Authentication required"))
}

fn is_admin<H: HostApi>(host: &mut H, user_id: u64) -> Result<bool, ApiError> {
    host.authz_can(user_id, &[ADMIN_ABILITY])
        .map_err(authorization_unavailable)
}

fn authorization_unavailable(_: HostApiError) -> ApiError {
    ApiError::new(
        502,
        "AUTHZ_UNAVAILABLE",
        "Permission checks are temporarily unavailable",
    )
}
