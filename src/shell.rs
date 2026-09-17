//! Command quoting for `nodecmd` and daemon tasks.
//!
//! The daemon splits commands into arguments without invoking a shell. Each
//! quoted value must survive `shellquote.Split` as exactly one argument.
//!
//! On Windows the daemon doubles backslashes before splitting. Double quotes
//! preserve the original path; single quotes would retain doubled backslashes.

/// Joins arguments into a single command string, single-quoting any token that
/// is empty or contains characters outside a conservative safe set. Compatible
/// with `github.com/kballard/go-shellquote`'s `Split`.
pub fn shell_join(args: &[&str]) -> String {
    args.iter()
        .map(|argument| quote(argument))
        .collect::<Vec<_>>()
        .join(" ")
}

/// [`shell_join`] for commands a Windows daemon splits (see the module docs).
pub fn shell_join_windows(args: &[&str]) -> String {
    args.iter()
        .map(|argument| quote_windows(argument))
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_safe_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'_' | b'@' | b'%' | b'+' | b'=' | b':' | b',' | b'.' | b'/' | b'-'
        )
}

fn is_safe(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(is_safe_byte)
}

fn is_safe_windows(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| is_safe_byte(byte) || byte == b'\\')
}

fn quote(value: &str) -> String {
    if is_safe(value) {
        return value.to_owned();
    }

    // Single-quote wrap; close-quote, escaped literal quote, reopen for each '.
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('\'');

    for character in value.chars() {
        if character == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(character);
        }
    }

    quoted.push('\'');
    quoted
}

fn quote_windows(value: &str) -> String {
    if is_safe_windows(value) {
        return value.to_owned();
    }

    // Splice embedded double quotes into adjacent quoted segments so the
    // daemon's backslash rewriting cannot alter the escape sequence.
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');

    for character in value.chars() {
        if character == '"' {
            quoted.push_str("\"'\"'\"");
        } else {
            quoted.push(character);
        }
    }

    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use super::{quote, quote_windows, shell_join, shell_join_windows};

    #[test]
    fn safe_tokens_pass_through() {
        assert_eq!(
            shell_join(&["get-tool", "https://example.com/install.sh"]),
            "get-tool https://example.com/install.sh"
        );
        assert_eq!(
            shell_join(&["serve", "--config=/srv/gameap/.plugins/fastdla/config.json"]),
            "serve --config=/srv/gameap/.plugins/fastdla/config.json"
        );
    }

    #[test]
    fn spaces_are_quoted() {
        assert_eq!(
            shell_join(&["--data-dir=/srv/game ap"]),
            "'--data-dir=/srv/game ap'"
        );
    }

    #[test]
    fn embedded_quote_is_escaped() {
        assert_eq!(quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn empty_token_is_quoted() {
        assert_eq!(quote(""), "''");
    }

    #[test]
    fn windows_bare_backslash_path_passes_through() {
        assert_eq!(
            shell_join_windows(&[r"C:\gameap\.plugins\fastdla\gameap-fastdl.exe", "version"]),
            r"C:\gameap\.plugins\fastdla\gameap-fastdl.exe version"
        );
    }

    #[test]
    fn windows_spaces_use_double_quotes() {
        assert_eq!(
            shell_join_windows(&["-DataDir", r"C:\Program Files\gameap"]),
            r#"-DataDir "C:\Program Files\gameap""#
        );
    }

    #[test]
    fn windows_placeholder_token_is_double_quoted() {
        assert_eq!(quote_windows("{node_work_path}"), r#""{node_work_path}""#);
        assert_eq!(
            quote_windows("{node_tools_path}/install-fastdl-windows.ps1"),
            r#""{node_tools_path}/install-fastdl-windows.ps1""#
        );
    }

    #[test]
    fn windows_embedded_double_quote() {
        assert_eq!(quote_windows(r#"a"b"#), r#""a"'"'"b""#);
    }

    #[test]
    fn windows_empty_token() {
        assert_eq!(quote_windows(""), r#""""#);
    }
}
