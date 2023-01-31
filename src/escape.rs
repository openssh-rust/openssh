//! Escape characters that may have special meaning in a shell, including spaces.
//! This is a modified version of the [`shell-escape::unix`] module of [`shell-escape`] crate.
//! 
//! [`shell-escape`]: https://crates.io/crates/shell-escape
//! [`shell-escape::unix`]: https://docs.rs/shell-escape/latest/src/shell_escape/lib.rs.html#101


fn whitelisted(ch: char) -> bool {
    match ch {
        'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '=' | '/' | ',' | '.' | '+' => true,
        _ => false,
    }
}

/// Escape characters that may have special meaning in a shell, including spaces.
/// 
/// **Note**: This function is an adaptation of [`shell-escape::unix::escape`].
/// This function exists only for type compatibility and the implementation is
/// almost exactly the same as [`shell-escape::unix::escape`].
/// 
/// [`shell-escape::unix::escape`]: https://docs.rs/shell-escape/latest/src/shell_escape/lib.rs.html#101
/// 
pub fn escape(s: &[u8]) -> String {
    let all_whitelisted = s.iter().all(|x| whitelisted(*x as char));

    if !s.is_empty() && all_whitelisted {
        // All bytes are whitelisted and valid single-byte UTF-8, 
        // so we can build the original string and return as is.
        return String::from_utf8(s.to_vec()).unwrap();
    }

    let mut escaped = String::with_capacity(s.len() + 2);
    escaped.push('\'');

    for &b in s {
        match b {
            b'\'' | b'!' => {
                escaped.push_str("'\\");
                escaped.push(b as char);
                escaped.push('\'');
            }
            _ => escaped.push(b as char),
        }
    }
    escaped.push('\'');
    escaped
}


#[cfg(test)]
mod tests {
    use super::*;

    // These tests are courtesy of the `shell-escape` crate.
    #[test]
    fn test_escape() {
        assert_eq!(
            escape(b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_=/,.+"), 
            "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_=/,.+"
        );
        assert_eq!(
            escape(b"--aaa=bbb-ccc"), 
            "--aaa=bbb-ccc"
        );
        assert_eq!(
            escape(b"linker=gcc -L/foo -Wl,bar"), 
            r#"'linker=gcc -L/foo -Wl,bar'"#
        );
        assert_eq!(
            escape(br#"--features="default""#), 
            r#"'--features="default"'"#
        );
        assert_eq!(
            escape(br#"'!\$`\\\n "#), 
            r#"''\'''\!'\$`\\\n '"#
        );
        assert_eq!(escape(b""), r#"''"#);
        assert_eq!(escape(b" "), r#"' '"#);
        assert_eq!(escape(b"\xC4b"), r#"'Äb'"#);
    }
}