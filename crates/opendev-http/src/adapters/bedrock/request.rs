//! Request transformation for Amazon Bedrock.
//!
//! Converts OpenAI Chat Completions payloads to Bedrock's Anthropic-style format.

use serde_json::{Value, json};

/// Extract system message from messages array into a top-level field.
///
/// Bedrock's Anthropic format expects system as a separate top-level field.
pub(super) fn extract_system(payload: &mut Value) {
    if let Some(messages) = payload.get_mut("messages").and_then(|m| m.as_array_mut()) {
        let mut system_parts: Vec<String> = Vec::new();
        messages.retain(|msg| {
            if msg.get("role").and_then(|r| r.as_str()) == Some("system") {
                if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                    system_parts.push(content.to_string());
                }
                false
            } else {
                true
            }
        });

        if !system_parts.is_empty() {
            let combined = system_parts.join("\n\n");
            payload["system"] = json!(combined);
        }
    }
}

/// Convert Chat Completions tool schemas to Bedrock/Anthropic format.
///
/// OpenAI: `[{type: "function", function: {name, description, parameters}}]`
/// Bedrock: `[{name, description, input_schema}]`
pub(super) fn convert_tools(payload: &mut Value) {
    if let Some(tools) = payload.get_mut("tools").and_then(|t| t.as_array_mut()) {
        let converted: Vec<Value> = tools
            .iter()
            .filter_map(|tool| {
                let func = tool.get("function")?;
                Some(json!({
                    "name": func.get("name")?,
                    "description": func.get("description").cloned().unwrap_or(json!("")),
                    "input_schema": func.get("parameters").cloned()
                        .unwrap_or(json!({"type": "object", "properties": {}}))
                }))
            })
            .collect();
        if let Some(tools_slot) = payload.get_mut("tools") {
            *tools_slot = json!(converted);
        }
    }
}

/// Convert tool result messages from Chat Completions to Bedrock format.
///
/// Converts `role: "tool"` messages to `role: "user"` with `tool_result` blocks,
/// and assistant `tool_calls` to `tool_use` content blocks.
pub(super) fn convert_tool_messages(payload: &mut Value) {
    if let Some(messages) = payload.get_mut("messages").and_then(|m| m.as_array_mut()) {
        let mut converted: Vec<Value> = Vec::new();

        for msg in messages.iter() {
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");

            match role {
                "assistant" => {
                    if let Some(tool_calls) = msg.get("tool_calls").and_then(|tc| tc.as_array()) {
                        let mut content_blocks: Vec<Value> = Vec::new();

                        if let Some(text) = msg.get("content").and_then(|c| c.as_str())
                            && !text.is_empty()
                        {
                            content_blocks.push(json!({
                                "type": "text",
                                "text": text
                            }));
                        }

                        for tc in tool_calls {
                            let func = tc.get("function").cloned().unwrap_or(json!({}));
                            let args_str = func
                                .get("arguments")
                                .and_then(|a| a.as_str())
                                .unwrap_or("{}");
                            let args: Value = serde_json::from_str(args_str).unwrap_or(json!({}));

                            content_blocks.push(json!({
                                "type": "tool_use",
                                "id": tc.get("id").cloned().unwrap_or(json!("")),
                                "name": func.get("name").cloned().unwrap_or(json!("")),
                                "input": args
                            }));
                        }

                        converted.push(json!({
                            "role": "assistant",
                            "content": content_blocks
                        }));
                    } else {
                        converted.push(msg.clone());
                    }
                }
                "tool" => {
                    let tool_call_id = msg.get("tool_call_id").cloned().unwrap_or(json!(""));
                    let content = msg.get("content").and_then(|c| c.as_str()).unwrap_or("");

                    let result_block = json!({
                        "type": "tool_result",
                        "tool_use_id": tool_call_id,
                        "content": content
                    });

                    let should_merge = converted.last().is_some_and(|last| {
                        last.get("role").and_then(|r| r.as_str()) == Some("user")
                            && last.get("content").and_then(|c| c.as_array()).is_some_and(
                                |blocks| {
                                    blocks.iter().all(|b| {
                                        b.get("type").and_then(|t| t.as_str())
                                            == Some("tool_result")
                                    })
                                },
                            )
                    });

                    if should_merge {
                        if let Some(last) = converted.last_mut()
                            && let Some(blocks) =
                                last.get_mut("content").and_then(|c| c.as_array_mut())
                        {
                            blocks.push(result_block);
                        }
                    } else {
                        converted.push(json!({
                            "role": "user",
                            "content": [result_block]
                        }));
                    }
                }
                _ => {
                    converted.push(msg.clone());
                }
            }
        }

        if let Some(messages_slot) = payload.get_mut("messages") {
            *messages_slot = json!(converted);
        }
    }
}

/// Ensure max_tokens is set (required by Bedrock Anthropic models).
pub(super) fn ensure_max_tokens(payload: &mut Value) {
    if payload.get("max_tokens").is_none() {
        if let Some(val) = payload.get("max_completion_tokens").cloned() {
            if let Some(obj) = payload.as_object_mut() {
                obj.remove("max_completion_tokens");
            }
            payload["max_tokens"] = val;
        } else {
            payload["max_tokens"] = json!(4096);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_extract_system() {
        let mut payload = json!({
            "messages": [
                {"role": "system", "content": "You are helpful."},
                {"role": "user", "content": "Hello"}
            ]
        });
        extract_system(&mut payload);
        assert_eq!(payload["system"], "You are helpful.");
        let messages = payload["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
    }

    #[test]
    fn test_extract_system_multiple() {
        let mut payload = json!({
            "messages": [
                {"role": "system", "content": "Part 1"},
                {"role": "system", "content": "Part 2"},
                {"role": "user", "content": "Hello"}
            ]
        });
        extract_system(&mut payload);
        assert_eq!(payload["system"], "Part 1\n\nPart 2");
    }

    #[test]
    fn test_extract_system_none() {
        let mut payload = json!({
            "messages": [
                {"role": "user", "content": "Hello"}
            ]
        });
        extract_system(&mut payload);
        assert!(payload.get("system").is_none());
    }

    #[test]
    fn test_convert_tools() {
        let mut payload = json!({
            "tools": [{
                "type": "function",
                "function": {
                    "name": "read_file",
                    "description": "Read a file",
                    "parameters": {"type": "object", "properties": {"path": {"type": "string"}}}
                }
            }]
        });
        convert_tools(&mut payload);
        let tools = payload["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "read_file");
        assert_eq!(tools[0]["description"], "Read a file");
        assert!(tools[0].get("input_schema").is_some());
    }

    #[test]
    fn test_convert_tool_messages() {
        let mut payload = json!({
            "messages": [
                {"role": "user", "content": "Read the file"},
                {
                    "role": "assistant",
                    "content": "I'll read it.",
                    "tool_calls": [{
                        "id": "tc-1",
                        "function": {"name": "read_file", "arguments": "{\"path\": \"a.rs\"}"}
                    }]
                },
                {
                    "role": "tool",
                    "tool_call_id": "tc-1",
                    "content": "fn main() {}"
                }
            ]
        });
        convert_tool_messages(&mut payload);
        let messages = payload["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);

        // Assistant message should have tool_use blocks
        let assistant = &messages[1];
        let content = assistant["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "tool_use");
        assert_eq!(content[1]["name"], "read_file");

        // Tool result should be converted to user with tool_result
        let tool_result = &messages[2];
        assert_eq!(tool_result["role"], "user");
        let blocks = tool_result["content"].as_array().unwrap();
        assert_eq!(blocks[0]["type"], "tool_result");
        assert_eq!(blocks[0]["tool_use_id"], "tc-1");
    }

    #[test]
    fn test_convert_tool_messages_merge_consecutive() {
        let mut payload = json!({
            "messages": [
                {
                    "role": "assistant",
                    "content": "",
                    "tool_calls": [
                        {"id": "tc-1", "function": {"name": "read_file", "arguments": "{}"}},
                        {"id": "tc-2", "function": {"name": "search", "arguments": "{}"}}
                    ]
                },
                {"role": "tool", "tool_call_id": "tc-1", "content": "file1"},
                {"role": "tool", "tool_call_id": "tc-2", "content": "file2"}
            ]
        });
        convert_tool_messages(&mut payload);
        let messages = payload["messages"].as_array().unwrap();
        // Two consecutive tool messages should be merged into one user message
        assert_eq!(messages.len(), 2);
        let user_msg = &messages[1];
        assert_eq!(user_msg["role"], "user");
        let blocks = user_msg["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["tool_use_id"], "tc-1");
        assert_eq!(blocks[1]["tool_use_id"], "tc-2");
    }

    #[test]
    fn test_ensure_max_tokens_default() {
        let mut payload = json!({"messages": []});
        ensure_max_tokens(&mut payload);
        assert_eq!(payload["max_tokens"], 4096);
    }

    #[test]
    fn test_ensure_max_tokens_preserves_existing() {
        let mut payload = json!({"messages": [], "max_tokens": 8192});
        ensure_max_tokens(&mut payload);
        assert_eq!(payload["max_tokens"], 8192);
    }

    #[test]
    fn test_ensure_max_tokens_converts_max_completion_tokens() {
        let mut payload = json!({"messages": [], "max_completion_tokens": 2048});
        ensure_max_tokens(&mut payload);
        assert_eq!(payload["max_tokens"], 2048);
        assert!(payload.get("max_completion_tokens").is_none());
    }
}
