/// Replace secret-like tokens with `[REDACTED]` markers. Returns `None` when
/// nothing matched. Token patterns match at word boundaries so ordinary words
/// such as `risk-free` are not flagged, while credentials embedded later in a
/// sentence are still removed.
pub(crate) fn redact_secrets(text: &str) -> Option<String> {
    if text.contains("BEGIN PRIVATE KEY") {
        return Some("[REDACTED:private-key]".to_string());
    }

    const REDACTED: &str = "[REDACTED]";
    const TOKEN_PREFIXES: [&str; 5] = ["sk-", "ghp_", "github_pat_", "xoxb-", "AKIA"];
    let mut result = text.to_string();
    let mut changed = false;
    for pattern in TOKEN_PREFIXES {
        let mut from = 0;
        while let Some(start) = find_token(&result, pattern, from) {
            let end = result[start..]
                .char_indices()
                .find(|(_, c)| !(c.is_ascii_alphanumeric() || *c == '-' || *c == '_'))
                .map(|(offset, _)| start + offset)
                .unwrap_or(result.len());
            result.replace_range(start..end, REDACTED);
            changed = true;
            from = start + REDACTED.len();
        }
    }
    changed.then_some(result)
}

pub(crate) fn looks_secret_like(text: &str) -> bool {
    redact_secrets(text).is_some()
}

fn find_token(text: &str, pattern: &str, from: usize) -> Option<usize> {
    let mut search_from = from;
    while let Some(relative) = text.get(search_from..)?.find(pattern) {
        let start = search_from + relative;
        let at_boundary = start == 0 || !text.as_bytes()[start - 1].is_ascii_alphanumeric();
        if at_boundary {
            return Some(start);
        }
        search_from = start + pattern.len();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_aws_and_embedded_tokens_without_substring_false_positives() {
        let text = "aws_access_key_id=AKIAEXAMPLE123 note=rotate xoxb-embedded-token today";
        let redacted = redact_secrets(text).unwrap();
        assert_eq!(
            redacted,
            "aws_access_key_id=[REDACTED] note=rotate [REDACTED] today"
        );
        assert!(!looks_secret_like("risk-free routing and ordinary prose"));
    }
}
