//! The route table supplies both the host registration and local dispatch.

use std::collections::HashMap;

use gameap_plugin_sdk::proto::gameap::plugin as pb;

use crate::handlers;
use crate::host_api::HostApi;
use crate::http::ApiError;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RouteId {
    AdminNodes,
    NodeConfigGet,
    NodeConfigUpdate,
    NodeStatus,
    NodeSetup,
    NodeSync,
    ServerFastdlGet,
    ServerFastdlUpdate,
}

pub struct RouteDef {
    pub id: RouteId,
    pub method: &'static str,
    pub pattern: &'static str,
    pub admin_only: bool,
    pub description: &'static str,
}

pub const ROUTES: &[RouteDef] = &[
    RouteDef {
        id: RouteId::AdminNodes,
        method: "GET",
        pattern: "/admin/nodes",
        admin_only: true,
        description: "List FastDL nodes",
    },
    RouteDef {
        id: RouteId::NodeConfigGet,
        method: "GET",
        pattern: "/nodes/{nodeId}/config",
        admin_only: true,
        description: "Read FastDL node settings",
    },
    RouteDef {
        id: RouteId::NodeConfigUpdate,
        method: "PUT",
        pattern: "/nodes/{nodeId}/config",
        admin_only: true,
        description: "Update FastDL node settings",
    },
    RouteDef {
        id: RouteId::NodeStatus,
        method: "GET",
        pattern: "/nodes/{nodeId}/status",
        admin_only: true,
        description: "Read FastDL installation status",
    },
    RouteDef {
        id: RouteId::NodeSetup,
        method: "POST",
        pattern: "/nodes/{nodeId}/setup",
        admin_only: true,
        description: "Install or update FastDL",
    },
    RouteDef {
        id: RouteId::NodeSync,
        method: "POST",
        pattern: "/nodes/{nodeId}/sync",
        admin_only: true,
        description: "Synchronize FastDL settings",
    },
    RouteDef {
        id: RouteId::ServerFastdlGet,
        method: "GET",
        pattern: "/servers/{serverId}/fastdl",
        admin_only: false,
        description: "Read game server FastDL settings",
    },
    RouteDef {
        id: RouteId::ServerFastdlUpdate,
        method: "PUT",
        pattern: "/servers/{serverId}/fastdl",
        admin_only: false,
        description: "Update game server FastDL settings",
    },
];

pub fn http_routes() -> Vec<pb::HttpRoute> {
    ROUTES
        .iter()
        .map(|route| pb::HttpRoute {
            path: route.pattern.into(),
            methods: vec![route.method.into()],
            requires_auth: true,
            admin_only: route.admin_only,
            description: route.description.into(),
        })
        .collect()
}

pub fn match_route(method: &str, path: &str) -> Option<(RouteId, HashMap<String, String>)> {
    ROUTES.iter().find_map(|route| {
        if route.method != method {
            return None;
        }

        match_pattern(route.pattern, path).map(|params| (route.id, params))
    })
}

fn match_pattern(pattern: &str, path: &str) -> Option<HashMap<String, String>> {
    let pattern_segments: Vec<&str> = pattern.trim_end_matches('/').split('/').collect();
    let path_segments: Vec<&str> = path.trim_end_matches('/').split('/').collect();
    if pattern_segments.len() != path_segments.len() {
        return None;
    }

    let mut params = HashMap::new();
    for (pattern, segment) in pattern_segments.iter().zip(path_segments.iter()) {
        if let Some(name) = pattern
            .strip_prefix('{')
            .and_then(|name| name.strip_suffix('}'))
        {
            params.insert(name.to_string(), (*segment).to_string());
        } else if pattern != segment {
            return None;
        }
    }

    Some(params)
}

pub struct RequestParts<'a> {
    pub params: HashMap<String, String>,
    pub body: &'a [u8],
    pub user_id: Option<u64>,
}

pub fn dispatch<H: HostApi>(host: &mut H, request: &pb::HttpRequest) -> pb::HttpResponse {
    let user_id = request
        .session
        .as_ref()
        .and_then(|session| session.user.as_ref())
        .map(|user| user.id)
        .filter(|id| *id != 0);
    if user_id.is_none() {
        return ApiError::new(401, "UNAUTHENTICATED", "Authentication required").into_response();
    }

    let Some((route, params)) = match_route(&request.method, &request.path) else {
        return ApiError::not_found("Route not found").into_response();
    };
    let parts = RequestParts {
        params,
        body: &request.body,
        user_id,
    };

    handlers::handle(host, route, &parts).unwrap_or_else(ApiError::into_response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_each_public_route() {
        let cases = [
            ("GET", "/admin/nodes", RouteId::AdminNodes),
            ("GET", "/nodes/1/config", RouteId::NodeConfigGet),
            ("PUT", "/nodes/1/config", RouteId::NodeConfigUpdate),
            ("GET", "/nodes/1/status", RouteId::NodeStatus),
            ("POST", "/nodes/1/setup", RouteId::NodeSetup),
            ("POST", "/nodes/1/sync", RouteId::NodeSync),
            ("GET", "/servers/3/fastdl", RouteId::ServerFastdlGet),
            ("PUT", "/servers/3/fastdl", RouteId::ServerFastdlUpdate),
        ];

        for (method, path, expected) in cases {
            let (route, _) = match_route(method, path).unwrap();
            assert_eq!(route, expected, "{method} {path}");
        }
    }

    #[test]
    fn captures_entity_parameters_with_trailing_slash() {
        let (_, params) = match_route("PUT", "/servers/37/fastdl/").unwrap();
        assert_eq!(params.get("serverId").map(String::as_str), Some("37"));

        let (_, params) = match_route("POST", "/nodes/5/setup").unwrap();
        assert_eq!(params.get("nodeId").map(String::as_str), Some("5"));
    }

    #[test]
    fn rejects_unknown_methods_and_extra_segments() {
        for (method, path) in [
            ("DELETE", "/servers/3/fastdl"),
            ("GET", "/servers/3/fastdl/extra"),
            ("POST", "/nodes/1/unknown"),
            ("GET", "/unknown"),
            ("GET", "servers/3/fastdl"),
        ] {
            assert!(match_route(method, path).is_none(), "{method} {path}");
        }
    }

    #[test]
    fn registers_authentication_and_admin_scope() {
        let routes = http_routes();
        assert_eq!(routes.len(), 8);
        for route in routes {
            assert!(route.requires_auth, "{}", route.path);
            assert_eq!(
                route.admin_only,
                route.path.starts_with("/admin/") || route.path.starts_with("/nodes/"),
                "{}",
                route.path
            );
        }
    }
}
