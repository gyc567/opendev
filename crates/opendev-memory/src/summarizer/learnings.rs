//! Learning pattern detection and consolidation from conversations.
//!
//! Scans conversation messages for patterns worth remembering and extracts
//! them as concise learning strings suitable for playbook bullets.

use serde_json::Value;

/// Extract key learnings from a conversation for playbook consolidation.
///
/// Scans messages for patterns worth remembering:
/// - Error patterns that were fixed (error followed by a successful resolution)
/// - Configuration/environment discoveries (paths, settings, commands)
/// - File patterns and project structure learned
///
/// Returns a list of concise learning strings suitable for playbook bullets.
pub fn consolidate_learnings(messages: &[Value]) -> Vec<String> {
    let mut learnings = Vec::new();

    // Track error -> fix patterns
    let mut last_error: Option<String> = None;
    let mut seen_configs: Vec<String> = Vec::new();
    let mut seen_file_patterns: Vec<String> = Vec::new();

    for msg in messages {
        let role = msg.get("role").and_then(Value::as_str).unwrap_or("");
        let content = msg.get("content").and_then(Value::as_str).unwrap_or("");

        // Detect error messages in tool results
        if role == "tool" {
            let lower = content.to_lowercase();
            if lower.contains("error")
                || lower.contains("failed")
                || lower.contains("not found")
                || lower.contains("permission denied")
            {
                // Extract a short summary of the error
                let error_summary = content.lines().next().unwrap_or(content);
                let truncated = if error_summary.len() > 120 {
                    &error_summary[..120]
                } else {
                    error_summary
                };
                last_error = Some(truncated.to_string());
            }
        }

        // Detect fixes following errors
        if role == "assistant" && last_error.is_some() {
            let lower = content.to_lowercase();
            if lower.contains("fixed")
                || lower.contains("resolved")
                || lower.contains("the issue was")
                || lower.contains("the problem was")
                || lower.contains("solution")
            {
                if let Some(ref err) = last_error {
                    let fix_summary = content.lines().next().unwrap_or(content);
                    let truncated_fix = if fix_summary.len() > 120 {
                        &fix_summary[..120]
                    } else {
                        fix_summary
                    };
                    learnings.push(format!(
                        "Error pattern fixed: '{}' -> '{}'",
                        err, truncated_fix
                    ));
                }
                last_error = None;
            }
        }

        // Detect configuration discoveries
        if role == "assistant" {
            let lower = content.to_lowercase();
            let config_keywords = [
                "config",
                "configuration",
                "setting",
                "environment variable",
                "env var",
                ".env",
            ];
            for keyword in &config_keywords {
                if lower.contains(keyword) {
                    let config_line = content.lines().next().unwrap_or(content);
                    let truncated = if config_line.len() > 150 {
                        &config_line[..150]
                    } else {
                        config_line
                    };
                    if !seen_configs.contains(&truncated.to_string()) {
                        seen_configs.push(truncated.to_string());
                        learnings.push(format!("Configuration discovered: {}", truncated));
                    }
                    break;
                }
            }
        }

        // Detect file pattern usage in tool calls
        if let Some(tool_calls) = msg.get("tool_calls").and_then(Value::as_array) {
            for tc in tool_calls {
                let name = tc
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let args = tc
                    .get("function")
                    .and_then(|f| f.get("arguments"))
                    .and_then(Value::as_str)
                    .unwrap_or("");

                if (name == "read_file" || name == "write_file" || name == "edit_file")
                    && args.contains("path")
                {
                    // Extract file extension pattern
                    if let Some(ext_start) = args.rfind('.') {
                        let ext = &args[ext_start..];
                        let ext_clean: String = ext
                            .chars()
                            .take_while(|c| c.is_alphanumeric() || *c == '.')
                            .collect();
                        if ext_clean.len() > 1 && !seen_file_patterns.contains(&ext_clean) {
                            seen_file_patterns.push(ext_clean.clone());
                        }
                    }
                }
            }
        }
    }

    // Add file pattern summary if multiple patterns observed
    if seen_file_patterns.len() >= 2 {
        learnings.push(format!(
            "File patterns used: {}",
            seen_file_patterns.join(", ")
        ));
    }

    learnings
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_msg(role: &str, content: &str) -> Value {
        json!({"role": role, "content": content})
    }

    #[test]
    fn test_consolidate_learnings_error_fix_pattern() {
        let messages = vec![
            make_msg("user", "fix the build"),
            json!({
                "role": "tool",
                "content": "error: module 'foo' not found"
            }),
            json!({
                "role": "assistant",
                "content": "The issue was a missing import. Fixed by adding the module."
            }),
        ];
        let learnings = consolidate_learnings(&messages);
        assert!(
            learnings.iter().any(|l| l.contains("Error pattern fixed")),
            "Should detect error->fix pattern, got: {learnings:?}"
        );
    }

    #[test]
    fn test_consolidate_learnings_config_discovery() {
        let messages = vec![
            make_msg("user", "how to connect to the database?"),
            json!({
                "role": "assistant",
                "content": "The database configuration is in config/database.yml with host and port settings."
            }),
        ];
        let learnings = consolidate_learnings(&messages);
        assert!(
            learnings
                .iter()
                .any(|l| l.contains("Configuration discovered")),
            "Should detect config discovery, got: {learnings:?}"
        );
    }

    #[test]
    fn test_consolidate_learnings_file_patterns() {
        let messages = vec![json!({
            "role": "assistant",
            "content": "",
            "tool_calls": [
                {"function": {"name": "read_file", "arguments": "{\"path\": \"src/main.rs\"}"}},
                {"function": {"name": "edit_file", "arguments": "{\"path\": \"src/lib.rs\"}"}},
                {"function": {"name": "read_file", "arguments": "{\"path\": \"tests/test.py\"}"}}
            ]
        })];
        let learnings = consolidate_learnings(&messages);
        assert!(
            learnings.iter().any(|l| l.contains("File patterns used")),
            "Should detect file patterns, got: {learnings:?}"
        );
    }

    #[test]
    fn test_consolidate_learnings_empty() {
        let messages: Vec<Value> = vec![];
        let learnings = consolidate_learnings(&messages);
        assert!(learnings.is_empty());
    }

    #[test]
    fn test_consolidate_learnings_no_patterns() {
        let messages = vec![make_msg("user", "hello"), make_msg("assistant", "hi there")];
        let learnings = consolidate_learnings(&messages);
        assert!(learnings.is_empty());
    }
}
