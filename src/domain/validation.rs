//! Input restrictions shared by panel settings and daemon command arguments.

use crate::http::ApiError;

pub fn validate_url(value: &str, https_only: bool) -> Result<(), ApiError> {
    let rest = value
        .strip_prefix("https://")
        .or_else(|| {
            if https_only {
                None
            } else {
                value.strip_prefix("http://")
            }
        })
        .ok_or_else(|| {
            ApiError::bad_request("A valid HTTP URL is required (HTTPS for downloads)")
        })?;

    let authority = rest.split('/').next().unwrap_or("");
    let invalid_authority = authority.is_empty() || authority.starts_with('-');
    let invalid_character = value.bytes().any(|byte| {
        byte <= 32
            || byte >= 127
            || matches!(byte, b'"' | b'\'' | b'\\' | b'#' | b'?' | b'%' | b'@')
    });
    let traversal = rest
        .split('/')
        .any(|component| matches!(component, "." | ".."));

    if value.len() > 2048
        || invalid_authority
        || !value.is_ascii()
        || invalid_character
        || traversal
    {
        return Err(ApiError::bad_request("Invalid public or download URL"));
    }

    Ok(())
}

pub fn validate_relative(value: &str, allow_empty: bool) -> Result<(), ApiError> {
    if allow_empty && value.is_empty() {
        return Ok(());
    }

    let invalid_component = value.split('/').any(|component| {
        component.is_empty()
            || component == "."
            || component == ".."
            || component.starts_with('.')
            || component.ends_with('.')
            || component.ends_with(' ')
    });
    let invalid_character = value.chars().any(|character| {
        character.is_control()
            || matches!(
                character,
                '\\' | ':' | '%' | '"' | '\'' | '<' | '>' | '|' | '?' | '*' | '{' | '}'
            )
    });

    if value.is_empty() || value.len() > 512 || invalid_component || invalid_character {
        return Err(ApiError::bad_request(
            "Game directory must be a relative path inside the game server",
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{validate_relative, validate_url};

    #[test]
    fn relative_paths_reject_traversal_and_ambiguous_windows_paths() {
        for path in [
            "../other",
            "/etc",
            "cstrike/../../private",
            "cstrike\\..\\private",
            "C:/games",
            "cstrike/%2e%2e",
            "cstrike//maps",
            ".git",
            "cstrike/.",
            "cstrike/trailing.",
            "cstrike/trailing ",
            "cstrike/file:stream",
            "cstrike/line\nbreak",
        ] {
            assert!(validate_relative(path, false).is_err(), "{path}");
        }

        assert!(validate_relative("servers/css", false).is_ok());
        assert!(validate_relative("", true).is_ok());
        assert!(validate_relative("", false).is_err());
    }

    #[test]
    fn urls_reject_credentials_encoded_paths_and_configuration_breakouts() {
        for url in [
            "https://",
            "https://-example.com/file",
            "https://user@example.com/file",
            "https://example.com/../private",
            "https://example.com/%2e%2e/private",
            "https://example.com/file?query=value",
            "https://example.com/file#fragment",
            "https://example.com/a\\b",
            "https://example.com/a\"b",
            "https://example.com/line\nbreak",
        ] {
            assert!(validate_url(url, false).is_err(), "{url}");
        }

        assert!(validate_url("https://example.com/downloads", true).is_ok());
        assert!(validate_url("http://example.com/downloads", false).is_ok());
        assert!(validate_url("http://example.com/downloads", true).is_err());
    }
}
