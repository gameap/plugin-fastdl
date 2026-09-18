//! JSON responses and errors shared by the HTTP handlers.

use std::collections::HashMap;

use gameap_plugin_sdk::proto::gameap::plugin as pb;
use serde::{Serialize, de::DeserializeOwned};

use crate::host_api::HostApiError;

pub type ApiResult = Result<pb::HttpResponse, ApiError>;

const MAX_REQUEST_BODY_SIZE: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    pub status: i32,
    pub code: &'static str,
    pub message: String,
}

impl ApiError {
    pub fn new(status: i32, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(400, "INVALID_INPUT", message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(404, "NOT_FOUND", message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(409, "CONFLICT", message)
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(403, "FORBIDDEN", message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(500, "INTERNAL_ERROR", message)
    }

    pub fn into_response(self) -> pb::HttpResponse {
        #[derive(Serialize)]
        struct ErrorBody {
            code: &'static str,
            message: String,
        }

        json_response(
            self.status,
            &ErrorBody {
                code: self.code,
                message: self.message,
            },
        )
    }
}

impl From<HostApiError> for ApiError {
    fn from(_: HostApiError) -> Self {
        Self::new(
            502,
            "NODE_UNAVAILABLE",
            "The operation could not be completed. Check the node connection and plugin permissions.",
        )
    }
}

pub fn json_response<T: Serialize>(status: i32, value: &T) -> pb::HttpResponse {
    match serde_json::to_vec(value) {
        Ok(body) => pb::HttpResponse {
            status_code: status,
            headers: json_headers(),
            body,
            file: None,
        },
        Err(_) => pb::HttpResponse {
            status_code: 500,
            headers: json_headers(),
            body: br#"{"code":"INTERNAL_ERROR","message":"Response encoding failed"}"#.to_vec(),
            file: None,
        },
    }
}

fn json_headers() -> HashMap<String, String> {
    HashMap::from([
        ("Content-Type".into(), "application/json".into()),
        ("Cache-Control".into(), "no-store".into()),
    ])
}

pub fn parse_json_body<T: DeserializeOwned>(body: &[u8]) -> Result<T, ApiError> {
    if body.len() > MAX_REQUEST_BODY_SIZE {
        return Err(ApiError::bad_request("Request body too large"));
    }

    serde_json::from_slice(body).map_err(|_| ApiError::bad_request("Invalid request body"))
}

pub fn parse_u64_param(params: &HashMap<String, String>, key: &str) -> Result<u64, ApiError> {
    params
        .get(key)
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|id| *id != 0)
        .ok_or_else(|| ApiError::bad_request("Invalid entity reference"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_errors_do_not_expose_internal_details() {
        let response =
            ApiError::from(HostApiError::Op("secret command and paths".into())).into_response();
        let body: serde_json::Value = serde_json::from_slice(&response.body).unwrap();

        assert_eq!(response.status_code, 502);
        assert_eq!(body["code"], "NODE_UNAVAILABLE");
        assert!(!String::from_utf8_lossy(&response.body).contains("secret"));
        assert_eq!(response.headers.get("Cache-Control").unwrap(), "no-store");
    }

    #[test]
    fn limits_request_bodies_before_json_parsing() {
        let mut body = vec![b' '; MAX_REQUEST_BODY_SIZE - 2];
        body.extend_from_slice(b"{}");
        assert!(parse_json_body::<serde_json::Value>(&body).is_ok());

        body.push(b' ');
        let error = parse_json_body::<serde_json::Value>(&body).unwrap_err();
        assert_eq!(error.status, 400);
        assert_eq!(error.code, "INVALID_INPUT");
        assert_eq!(error.message, "Request body too large");
    }

    #[test]
    fn invalid_json_keeps_the_public_error_message() {
        let error = parse_json_body::<serde_json::Value>(b"{invalid").unwrap_err();
        assert_eq!(error.message, "Invalid request body");
    }

    #[test]
    fn entity_parameters_must_be_positive_u64_values() {
        assert!(parse_u64_param(&HashMap::new(), "serverId").is_err());
        for value in ["", "0", "-1", "abc", "18446744073709551616"] {
            let params = HashMap::from([("serverId".into(), value.into())]);
            let error = parse_u64_param(&params, "serverId").unwrap_err();
            assert_eq!(error.status, 400, "{value}");
            assert_eq!(error.message, "Invalid entity reference", "{value}");
        }

        let params = HashMap::from([("serverId".into(), "17".into())]);
        assert_eq!(parse_u64_param(&params, "serverId").unwrap(), 17);
    }
}
