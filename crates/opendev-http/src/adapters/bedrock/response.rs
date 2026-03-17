//! Response transformation for Amazon Bedrock.
//!
//! Converts Bedrock's Anthropic-style responses back to OpenAI Chat Completions format.

use serde_json::{Value, json};

/// Convert Bedrock's Anthropic-style response to Chat Completions format.
pub(super) fn response_to_chat_completions(response: Value, model_id: &str) -> Value {
    let blocks = response
        .get("content")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();

    // Extract text content
    let content: String = blocks
        .iter()
        .filter_map(|b| {
            if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                b.get("text").and_then(|t| t.as_str()).map(String::from)
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("");

    // Extract tool_use blocks
    let tool_calls: Vec<Value> = blocks
        .iter()
        .filter_map(|b| {
            if b.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                let id = b.get("id").and_then(|i| i.as_str()).unwrap_or("");
                let name = b.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let input = b.get("input").cloned().unwrap_or(json!({}));
                Some(json!({
                    "id": id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": serde_json::to_string(&input).unwrap_or_default()
                    }
                }))
            } else {
                None
            }
        })
        .collect();

    let stop_reason = response
        .get("stop_reason")
        .and_then(|r| r.as_str())
        .unwrap_or("stop");

    let finish_reason = match stop_reason {
        "end_turn" => "stop",
        "max_tokens" => "length",
        "tool_use" => "tool_calls",
        other => other,
    };

    let usage = response.get("usage").cloned().unwrap_or(json!({}));

    let mut message = json!({
        "role": "assistant",
        "content": content
    });

    if !tool_calls.is_empty() {
        message["tool_calls"] = json!(tool_calls);
    }

    json!({
        "id": response.get("id").cloned().unwrap_or(json!("")),
        "object": "chat.completion",
        "model": model_id,
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": finish_reason
        }],
        "usage": {
            "prompt_tokens": usage.get("input_tokens").cloned().unwrap_or(json!(0)),
            "completion_tokens": usage.get("output_tokens").cloned().unwrap_or(json!(0)),
            "total_tokens": usage.get("input_tokens").and_then(|i| i.as_u64())
                .unwrap_or(0)
                + usage.get("output_tokens").and_then(|o| o.as_u64())
                .unwrap_or(0)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_response_to_chat_completions_text() {
        let response = json!({
            "id": "msg_123",
            "content": [{"type": "text", "text": "Hello!"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 10, "output_tokens": 5}
        });
        let result = response_to_chat_completions(response, "anthropic.claude-3-sonnet");
        assert_eq!(result["object"], "chat.completion");
        assert_eq!(result["model"], "anthropic.claude-3-sonnet");
        assert_eq!(result["choices"][0]["message"]["content"], "Hello!");
        assert_eq!(result["choices"][0]["finish_reason"], "stop");
        assert_eq!(result["usage"]["prompt_tokens"], 10);
        assert_eq!(result["usage"]["completion_tokens"], 5);
        assert_eq!(result["usage"]["total_tokens"], 15);
    }

    #[test]
    fn test_response_to_chat_completions_tool_use() {
        let response = json!({
            "id": "msg_456",
            "content": [
                {"type": "text", "text": "Let me read that file."},
                {
                    "type": "tool_use",
                    "id": "tu_1",
                    "name": "read_file",
                    "input": {"path": "src/main.rs"}
                }
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 20, "output_tokens": 10}
        });
        let result = response_to_chat_completions(response, "claude-3");
        assert_eq!(result["choices"][0]["finish_reason"], "tool_calls");
        let tool_calls = result["choices"][0]["message"]["tool_calls"]
            .as_array()
            .unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0]["id"], "tu_1");
        assert_eq!(tool_calls[0]["function"]["name"], "read_file");
    }

    #[test]
    fn test_response_max_tokens_finish_reason() {
        let response = json!({
            "content": [{"type": "text", "text": "truncated..."}],
            "stop_reason": "max_tokens",
            "usage": {"input_tokens": 0, "output_tokens": 0}
        });
        let result = response_to_chat_completions(response, "model");
        assert_eq!(result["choices"][0]["finish_reason"], "length");
    }
}
