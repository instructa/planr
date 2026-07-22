use serde_json::json;

/// One-click Cursor MCP install link (cursor://anysphere.cursor-deeplink).
/// The embedded config carries no --db path on purpose: Cursor spawns stdio
/// servers with the workspace as working directory, so each project resolves
/// its own `.planr/planr.sqlite`, making the link safe at user scope.
pub fn cursor_deeplink() -> String {
    let config = json!({"command": "planr", "args": ["mcp"]});
    format!(
        "cursor://anysphere.cursor-deeplink/mcp/install?name=planr&config={}",
        base64_url(config.to_string().as_bytes())
    )
}

/// URL-safe base64 (RFC 4648 section 5, with padding), matching what Cursor's
/// deeplink handler accepts. Small enough that a dependency is not warranted.
fn base64_url(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{base64_url, cursor_deeplink};

    #[test]
    fn base64_url_matches_rfc4648_vectors() {
        assert_eq!(base64_url(b""), "");
        assert_eq!(base64_url(b"f"), "Zg==");
        assert_eq!(base64_url(b"fo"), "Zm8=");
        assert_eq!(base64_url(b"foo"), "Zm9v");
        assert_eq!(base64_url(b"foob"), "Zm9vYg==");
        assert_eq!(base64_url(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_url(b"foobar"), "Zm9vYmFy");
        // URL-safe alphabet: 0xfb 0xff maps to -_ instead of +/.
        assert_eq!(base64_url(&[0xfb, 0xff]), "-_8=");
    }

    #[test]
    fn cursor_deeplink_encodes_portable_stdio_config() {
        let link = cursor_deeplink();
        assert!(
            link.starts_with("cursor://anysphere.cursor-deeplink/mcp/install?name=planr&config=")
        );
        let encoded = link.split("config=").nth(1).unwrap();
        assert_eq!(
            encoded,
            base64_url(br#"{"args":["mcp"],"command":"planr"}"#),
            "deeplink config must be the portable stdio server object"
        );
    }
}
