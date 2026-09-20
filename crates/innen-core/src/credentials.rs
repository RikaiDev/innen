//! Credential scan (secret patterns + redacted preview).
//!
//! Single owner for everything credential-shaped: pattern matchers,
//! [`scan_credentials`], and the redacted [`credential_preview`] used in
//! skip reports. Callers ([`crate::harvest`], [`crate::trace`],
//! [`crate::wiki_graph`]) scan before append/expand; a hit skips the
//! record + reports it, never appends. False positives go to a human via
//! report, never auto-delete.
//!
//! Each pattern below is specified exactly as implemented (prefix, tail
//! charset, minimum length); the matcher code is the definition —
//! hand-rolled, no regex dependency.
//! - `aws-access-key` (`AKIA[0-9A-Z]{16}`): fixed prefix `AKIA` + 16 chars
//!   each in `[0-9A-Z]`. Matches the regex on ASCII input; no word-boundary
//!   handling (same as the bare regex).
//! - `github-token` (`ghp_[A-Za-z0-9]{36,}`): fixed prefix `ghp_` + run of
//!   `[A-Za-z0-9]` of length ≥ 36. Only alphanumerics counted (no `_` tail).
//! - `private-key` (`-----BEGIN .*PRIVATE KEY`): line-oriented — a line
//!   containing literal `-----BEGIN ` with `PRIVATE KEY` later on the same
//!   line. Covers single-line PEM headers (`-----BEGIN RSA PRIVATE KEY-----`);
//!   multi-line split headers are not matched.
//! - `client-secret` (`client_secret\s*[:=]`): literal `client_secret`,
//!   ASCII-whitespace skip (` `, `\t`, `\n`, `\r`, VT, FF), then `:` or `=`.
//!   (Regex `\s` also covers Unicode whitespace; we cover ASCII only.)
//! - `slack-token` (`xox[bap]-`): literal `xoxb-` / `xoxa-` / `xoxp-`.
//! - `anthropic-key` (`sk-ant-[A-Za-z0-9-]+`): fixed prefix `sk-ant-` +
//!   ≥ 1 chars each in `[A-Za-z0-9-]`. No minimum-length beyond 1.
//! - `openai-key` (`sk-` + long tail): fixed prefix `sk-` + run of
//!   `[A-Za-z0-9_-]` of length ≥ 20. Short `sk-` words (e.g. `sk-aware`,
//!   `sk-sandbox`) do not match. Overlaps `sk-ant-…` (reported as
//!   `anthropic-key` first by canonical order).
//! - `upstash-token` (`UPSTASH_REDIS_REST_TOKEN\s*[:=]`): literal env-var
//!   name, ASCII-whitespace skip, then `:` or `=`. The sibling
//!   `UPSTASH_REDIS_REST_URL` carries no secret and does not match.

fn is_aws_tail(b: u8) -> bool {
    matches!(b, b'0'..=b'9' | b'A'..=b'Z')
}

fn is_ghp_tail(b: u8) -> bool {
    matches!(b, b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z')
}

fn is_anthropic_tail(b: u8) -> bool {
    matches!(b, b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' | b'-')
}

fn is_openai_tail(b: u8) -> bool {
    matches!(b, b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' | b'_' | b'-')
}

fn is_ascii_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

fn has_aws_key(bytes: &[u8]) -> bool {
    if bytes.len() < 20 {
        return false;
    }
    for i in 0..=(bytes.len() - 4) {
        if bytes[i..].starts_with(b"AKIA")
            && i + 20 <= bytes.len()
            && bytes[i + 4..i + 20].iter().all(|&b| is_aws_tail(b))
        {
            return true;
        }
    }
    false
}

fn has_github_token(bytes: &[u8]) -> bool {
    if bytes.len() < 4 {
        return false;
    }
    for i in 0..=(bytes.len() - 4) {
        if bytes[i..].starts_with(b"ghp_") {
            let mut n = 0;
            for &b in &bytes[i + 4..] {
                if is_ghp_tail(b) {
                    n += 1;
                } else {
                    break;
                }
            }
            if n >= 36 {
                return true;
            }
        }
    }
    false
}

fn has_private_key(text: &str) -> bool {
    for line in text.lines() {
        if let Some(begin) = line.find("-----BEGIN ") {
            if line[begin + "-----BEGIN ".len()..].contains("PRIVATE KEY") {
                return true;
            }
        }
    }
    false
}

fn has_client_secret(bytes: &[u8]) -> bool {
    const NEEDLE: &[u8] = b"client_secret";
    if bytes.len() < NEEDLE.len() {
        return false;
    }
    for i in 0..=(bytes.len() - NEEDLE.len()) {
        if &bytes[i..i + NEEDLE.len()] == NEEDLE {
            let mut j = i + NEEDLE.len();
            while j < bytes.len() && is_ascii_ws(bytes[j]) {
                j += 1;
            }
            if j < bytes.len() && (bytes[j] == b':' || bytes[j] == b'=') {
                return true;
            }
        }
    }
    false
}

fn has_slack_token(bytes: &[u8]) -> bool {
    if bytes.len() < 5 {
        return false;
    }
    for i in 0..=(bytes.len() - 5) {
        let w = &bytes[i..i + 5];
        if w == b"xoxb-" || w == b"xoxa-" || w == b"xoxp-" {
            return true;
        }
    }
    false
}

fn has_anthropic_key(bytes: &[u8]) -> bool {
    const PREFIX: &[u8] = b"sk-ant-";
    if bytes.len() <= PREFIX.len() {
        // Need prefix + at least one tail char.
        return false;
    }
    for i in 0..=(bytes.len() - PREFIX.len()) {
        if &bytes[i..i + PREFIX.len()] == PREFIX
            && i + PREFIX.len() < bytes.len()
            && is_anthropic_tail(bytes[i + PREFIX.len()])
        {
            return true;
        }
    }
    false
}

fn has_openai_key(bytes: &[u8]) -> bool {
    const PREFIX: &[u8] = b"sk-";
    const MIN_TAIL: usize = 20;
    if bytes.len() < PREFIX.len() + MIN_TAIL {
        return false;
    }
    for i in 0..=(bytes.len() - PREFIX.len()) {
        if &bytes[i..i + PREFIX.len()] == PREFIX {
            let mut n = 0;
            for &b in &bytes[i + PREFIX.len()..] {
                if is_openai_tail(b) {
                    n += 1;
                } else {
                    break;
                }
            }
            if n >= MIN_TAIL {
                return true;
            }
        }
    }
    false
}

fn has_upstash_token(bytes: &[u8]) -> bool {
    const NEEDLE: &[u8] = b"UPSTASH_REDIS_REST_TOKEN";
    if bytes.len() < NEEDLE.len() {
        return false;
    }
    for i in 0..=(bytes.len() - NEEDLE.len()) {
        if &bytes[i..i + NEEDLE.len()] == NEEDLE {
            let mut j = i + NEEDLE.len();
            while j < bytes.len() && is_ascii_ws(bytes[j]) {
                j += 1;
            }
            if j < bytes.len() && (bytes[j] == b':' || bytes[j] == b'=') {
                return true;
            }
        }
    }
    false
}

/// Return matched credential pattern NAMES (each at most once, canonical order).
pub fn scan_credentials(text: &str) -> Vec<&'static str> {
    let bytes = text.as_bytes();
    let mut hits = Vec::new();
    if has_aws_key(bytes) {
        hits.push("aws-access-key");
    }
    if has_github_token(bytes) {
        hits.push("github-token");
    }
    if has_private_key(text) {
        hits.push("private-key");
    }
    if has_client_secret(bytes) {
        hits.push("client-secret");
    }
    if has_slack_token(bytes) {
        hits.push("slack-token");
    }
    if has_anthropic_key(bytes) {
        hits.push("anthropic-key");
    }
    if has_openai_key(bytes) {
        hits.push("openai-key");
    }
    if has_upstash_token(bytes) {
        hits.push("upstash-token");
    }
    hits
}

/// Redacted preview: first 6 chars starting at the first match of `pattern`
/// followed by `"***"`. Falls back to the first 6 chars of content when
/// the pattern has no locatable match.
pub fn credential_preview(content: &str, pattern: &str) -> String {
    let off = match pattern {
        "aws-access-key" => find_aws_offset(content.as_bytes()),
        "github-token" => find_github_offset(content.as_bytes()),
        "private-key" => find_private_key_offset(content),
        "client-secret" => find_client_secret_offset(content.as_bytes()),
        "slack-token" => find_slack_offset(content.as_bytes()),
        "anthropic-key" => find_anthropic_offset(content.as_bytes()),
        "openai-key" => find_openai_offset(content.as_bytes()),
        "upstash-token" => find_upstash_offset(content.as_bytes()),
        _ => None,
    };
    match off.and_then(|o| content.get(o..)) {
        Some(tail) => format!("{}***", tail.chars().take(6).collect::<String>()),
        None => format!("{}***", content.chars().take(6).collect::<String>()),
    }
}

fn find_aws_offset(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 20 {
        return None;
    }
    (0..=(bytes.len() - 4)).find(|&i| {
        bytes[i..].starts_with(b"AKIA")
            && i + 20 <= bytes.len()
            && bytes[i + 4..i + 20]
                .iter()
                .all(|&b| matches!(b, b'0'..=b'9' | b'A'..=b'Z'))
    })
}

fn find_github_offset(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 4 {
        return None;
    }
    for i in 0..=(bytes.len() - 4) {
        if bytes[i..].starts_with(b"ghp_") {
            let mut n = 0;
            for &b in &bytes[i + 4..] {
                if matches!(b, b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z') {
                    n += 1;
                } else {
                    break;
                }
            }
            if n >= 36 {
                return Some(i);
            }
        }
    }
    None
}

fn find_private_key_offset(text: &str) -> Option<usize> {
    // Byte offset of the `-----BEGIN ` marker on a line that also contains
    // `PRIVATE KEY` later on the same line.
    let mut base = 0usize;
    for line in text.split_inclusive('\n') {
        if let Some(pos) = line.find("-----BEGIN ") {
            if line[pos + "-----BEGIN ".len()..].contains("PRIVATE KEY") {
                return Some(base + pos);
            }
        }
        base += line.len();
    }
    None
}

fn find_client_secret_offset(bytes: &[u8]) -> Option<usize> {
    const NEEDLE: &[u8] = b"client_secret";
    if bytes.len() < NEEDLE.len() {
        return None;
    }
    for i in 0..=(bytes.len() - NEEDLE.len()) {
        if &bytes[i..i + NEEDLE.len()] == NEEDLE {
            let mut j = i + NEEDLE.len();
            while j < bytes.len() && matches!(bytes[j], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
            {
                j += 1;
            }
            if j < bytes.len() && (bytes[j] == b':' || bytes[j] == b'=') {
                return Some(i);
            }
        }
    }
    None
}

fn find_slack_offset(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 5 {
        return None;
    }
    for i in 0..=(bytes.len() - 5) {
        let w = &bytes[i..i + 5];
        if w == b"xoxb-" || w == b"xoxa-" || w == b"xoxp-" {
            return Some(i);
        }
    }
    None
}

fn find_anthropic_offset(bytes: &[u8]) -> Option<usize> {
    const PREFIX: &[u8] = b"sk-ant-";
    if bytes.len() <= PREFIX.len() {
        return None;
    }
    for i in 0..=(bytes.len() - PREFIX.len()) {
        if &bytes[i..i + PREFIX.len()] == PREFIX
            && i + PREFIX.len() < bytes.len()
            && matches!(
                bytes[i + PREFIX.len()],
                b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' | b'-'
            )
        {
            return Some(i);
        }
    }
    None
}

fn find_openai_offset(bytes: &[u8]) -> Option<usize> {
    const PREFIX: &[u8] = b"sk-";
    const MIN_TAIL: usize = 20;
    if bytes.len() < PREFIX.len() + MIN_TAIL {
        return None;
    }
    for i in 0..=(bytes.len() - PREFIX.len()) {
        if &bytes[i..i + PREFIX.len()] == PREFIX {
            let mut n = 0;
            for &b in &bytes[i + PREFIX.len()..] {
                if matches!(b, b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' | b'_' | b'-') {
                    n += 1;
                } else {
                    break;
                }
            }
            if n >= MIN_TAIL {
                return Some(i);
            }
        }
    }
    None
}

fn find_upstash_offset(bytes: &[u8]) -> Option<usize> {
    const NEEDLE: &[u8] = b"UPSTASH_REDIS_REST_TOKEN";
    if bytes.len() < NEEDLE.len() {
        return None;
    }
    (0..=(bytes.len() - NEEDLE.len())).find(|&i| &bytes[i..i + NEEDLE.len()] == NEEDLE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_hits_aws_key_and_private_key() {
        let hits = scan_credentials("key AKIAIOSFODNN7EXAMPLE\n-----BEGIN RSA PRIVATE KEY-----\n");
        assert!(hits.contains(&"aws-access-key"));
        assert!(hits.contains(&"private-key"));
    }

    #[test]
    fn scan_clean_text_empty() {
        assert!(scan_credentials("hello world, nothing secret here").is_empty());
    }

    #[test]
    fn scan_hits_openai_key() {
        let hits = scan_credentials("api_key=sk-ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnop end");
        assert!(hits.contains(&"openai-key"));
    }

    #[test]
    fn scan_ignores_short_sk_words() {
        // Corpus noise: sandbox, ssh-server, aware… all short tails.
        assert!(scan_credentials("run sk-sandbox and sk-aware tasks").is_empty());
        assert!(scan_credentials("tail -f sk-ssh-server.log").is_empty());
    }

    #[test]
    fn scan_hits_upstash_token_assignment() {
        let hits = scan_credentials("UPSTASH_REDIS_REST_TOKEN=Ax09QWERTYUIOPasdfgh end");
        assert!(hits.contains(&"upstash-token"));
    }

    #[test]
    fn scan_ignores_upstash_url() {
        // The REST URL carries no secret.
        assert!(
            scan_credentials("UPSTASH_REDIS_REST_URL=https://example.upstash.io end").is_empty()
        );
    }

    #[test]
    fn preview_redacts_to_six_chars() {
        assert_eq!(
            credential_preview("key AKIAIOSFODNN7EXAMPLE end", "aws-access-key"),
            "AKIAIO***"
        );
        assert_eq!(
            credential_preview(
                "token sk-ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnop here",
                "openai-key"
            ),
            "sk-ABC***"
        );
    }
}
