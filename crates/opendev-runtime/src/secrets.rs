//! Secret detection and redaction in tool outputs.
//!
//! Scans text for common secret patterns (API keys, tokens, passwords, base64 blobs)
//! and provides redaction utilities.

use regex::Regex;
use std::sync::OnceLock;

/// The type/category of a detected secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretKind {
    /// Anthropic API key (sk-ant-...)
    AnthropicApiKey,
    /// OpenAI-style API key (sk-...)
    OpenAiApiKey,
    /// Groq API key (gsk_...)
    GroqApiKey,
    /// Google AI key (AIza...)
    GoogleApiKey,
    /// GitHub personal access token (ghp_...)
    GitHubToken,
    /// Bearer token in header
    BearerToken,
    /// Password in key=value assignment
    PasswordAssignment,
    /// Suspiciously long base64-encoded blob
    Base64Blob,
}

/// A single detected secret with its location in the input text.
#[derive(Debug, Clone)]
pub struct SecretMatch {
    /// What kind of secret was detected.
    pub kind: SecretKind,
    /// Byte offset of the start of the match.
    pub start: usize,
    /// Byte offset of the end of the match (exclusive).
    pub end: usize,
    /// The matched text.
    pub matched_text: String,
}

/// Internal pattern definition.
struct SecretPattern {
    kind: SecretKind,
    regex: &'static str,
}

const SECRET_PATTERNS: &[SecretPattern] = &[
    SecretPattern {
        kind: SecretKind::AnthropicApiKey,
        regex: r"sk-ant-[A-Za-z0-9_\-]{20,}",
    },
    SecretPattern {
        kind: SecretKind::OpenAiApiKey,
        // sk- followed by a non-"ant-" prefix and at least 20 chars total
        // Uses character class to exclude 'a' as first char after sk- (crude but avoids lookahead)
        regex: r"sk-(?:proj-|live-|[b-zB-Z0-9_])[A-Za-z0-9_\-]{19,}",
    },
    SecretPattern {
        kind: SecretKind::GroqApiKey,
        regex: r"gsk_[A-Za-z0-9_\-]{20,}",
    },
    SecretPattern {
        kind: SecretKind::GoogleApiKey,
        regex: r"AIza[A-Za-z0-9_\-]{30,}",
    },
    SecretPattern {
        kind: SecretKind::GitHubToken,
        regex: r"ghp_[A-Za-z0-9]{30,}",
    },
    SecretPattern {
        kind: SecretKind::BearerToken,
        regex: r"Bearer\s+[A-Za-z0-9_\-\.]{20,}",
    },
    SecretPattern {
        kind: SecretKind::PasswordAssignment,
        regex: r"(?i)(?:password|passwd|pass)\s*=\s*\S+",
    },
    SecretPattern {
        kind: SecretKind::Base64Blob,
        // 40+ chars of base64 alphabet (with optional padding), bounded by word edges
        regex: r"\b[A-Za-z0-9+/]{40,}={0,2}\b",
    },
];

/// Compiled regex cache.
fn compiled_patterns() -> &'static Vec<(SecretKind, Regex)> {
    static PATTERNS: OnceLock<Vec<(SecretKind, Regex)>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        SECRET_PATTERNS
            .iter()
            .map(|sp| {
                (
                    sp.kind.clone(),
                    Regex::new(sp.regex).expect("invalid secret pattern regex"),
                )
            })
            .collect()
    })
}

/// Scan text for potential secrets.
///
/// Returns all detected secrets with their positions and types.
pub fn detect_secrets(text: &str) -> Vec<SecretMatch> {
    let patterns = compiled_patterns();
    let mut matches = Vec::new();

    for (kind, re) in patterns {
        for m in re.find_iter(text) {
            matches.push(SecretMatch {
                kind: kind.clone(),
                start: m.start(),
                end: m.end(),
                matched_text: m.as_str().to_string(),
            });
        }
    }

    // Sort by position for consistent ordering
    matches.sort_by_key(|m| m.start);
    matches
}

/// Redact all detected secrets in the text, replacing them with `[REDACTED]`.
///
/// Handles overlapping matches by processing from right to left.
pub fn redact_secrets(text: &str) -> String {
    let mut matches = detect_secrets(text);
    if matches.is_empty() {
        return text.to_string();
    }

    // Deduplicate overlapping ranges: merge overlapping intervals
    matches.sort_by_key(|m| m.start);
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for m in &matches {
        if let Some(last) = merged.last_mut()
            && m.start <= last.1
        {
            last.1 = last.1.max(m.end);
            continue;
        }
        merged.push((m.start, m.end));
    }

    // Replace from right to left to preserve byte offsets
    let mut result = text.to_string();
    for (start, end) in merged.into_iter().rev() {
        result.replace_range(start..end, "[REDACTED]");
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_anthropic_key() {
        let text = "key=sk-ant-api03-abcdefghijklmnopqrstuvwxyz1234567890";
        let matches = detect_secrets(text);
        assert!(!matches.is_empty());
        assert!(
            matches
                .iter()
                .any(|m| m.kind == SecretKind::AnthropicApiKey)
        );
    }

    #[test]
    fn test_detect_openai_key() {
        let text = "OPENAI_API_KEY=sk-proj-abcdefghijklmnopqrstuvwxyz";
        let matches = detect_secrets(text);
        assert!(matches.iter().any(|m| m.kind == SecretKind::OpenAiApiKey));
    }

    #[test]
    fn test_detect_groq_key() {
        let text = "export GROQ_KEY=gsk_abcdefghijklmnopqrstuvwxyz1234";
        let matches = detect_secrets(text);
        assert!(matches.iter().any(|m| m.kind == SecretKind::GroqApiKey));
    }

    #[test]
    fn test_detect_google_key() {
        let text = "api_key: AIzaSyAbcdefghijklmnopqrstuvwxyz0123456789";
        let matches = detect_secrets(text);
        assert!(matches.iter().any(|m| m.kind == SecretKind::GoogleApiKey));
    }

    #[test]
    fn test_detect_github_token() {
        let text = "Authorization: token ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefgh";
        let matches = detect_secrets(text);
        assert!(matches.iter().any(|m| m.kind == SecretKind::GitHubToken));
    }

    #[test]
    fn test_detect_bearer_token() {
        let text = "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.payload";
        let matches = detect_secrets(text);
        assert!(matches.iter().any(|m| m.kind == SecretKind::BearerToken));
    }

    #[test]
    fn test_detect_password_assignment() {
        let text = "password=mysupersecretpassword123";
        let matches = detect_secrets(text);
        assert!(
            matches
                .iter()
                .any(|m| m.kind == SecretKind::PasswordAssignment)
        );
    }

    #[test]
    fn test_detect_password_case_insensitive() {
        let text = "PASSWORD = hunter2";
        let matches = detect_secrets(text);
        assert!(
            matches
                .iter()
                .any(|m| m.kind == SecretKind::PasswordAssignment)
        );
    }

    #[test]
    fn test_detect_base64_blob() {
        let text = "data: ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/==";
        let matches = detect_secrets(text);
        assert!(matches.iter().any(|m| m.kind == SecretKind::Base64Blob));
    }

    #[test]
    fn test_no_false_positive_short_string() {
        let text = "hello world this is normal text";
        let matches = detect_secrets(text);
        assert!(matches.is_empty());
    }

    #[test]
    fn test_redact_secrets_single() {
        let text = "key=sk-ant-api03-abcdefghijklmnopqrstuvwxyz1234567890 done";
        let redacted = redact_secrets(text);
        assert!(redacted.contains("[REDACTED]"));
        assert!(!redacted.contains("sk-ant-"));
        assert!(redacted.contains("done"));
    }

    #[test]
    fn test_redact_secrets_multiple() {
        let text = "key1=sk-ant-api03-abcdefghijklmnopqrstuvwxyz12345 key2=ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefgh";
        let redacted = redact_secrets(text);
        assert!(!redacted.contains("sk-ant-"));
        assert!(!redacted.contains("ghp_"));
        // Should have two [REDACTED] markers
        assert_eq!(redacted.matches("[REDACTED]").count(), 2);
    }

    #[test]
    fn test_redact_no_secrets() {
        let text = "just normal output\nno secrets here";
        assert_eq!(redact_secrets(text), text);
    }

    #[test]
    fn test_redact_preserves_surrounding_text() {
        let text = "before password=secret123 after";
        let redacted = redact_secrets(text);
        assert!(redacted.starts_with("before "));
        assert!(redacted.ends_with(" after"));
        assert!(redacted.contains("[REDACTED]"));
    }

    #[test]
    fn test_detect_passwd_variant() {
        let text = "passwd=supersecret";
        let matches = detect_secrets(text);
        assert!(
            matches
                .iter()
                .any(|m| m.kind == SecretKind::PasswordAssignment)
        );
    }

    #[test]
    fn test_secret_match_positions() {
        let text = "prefix sk-ant-api03-abcdefghijklmnopqrstuvwxyz1234567890 suffix";
        let matches = detect_secrets(text);
        assert!(!matches.is_empty());
        let m = &matches[0];
        assert_eq!(&text[m.start..m.end], m.matched_text);
    }
}
