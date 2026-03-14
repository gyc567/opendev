//! Task complete tool — signal explicit task completion.
//!
//! Instead of relying on implicit termination (no tool calls = done),
//! agents call this tool to end the ReAct loop. This provides:
//! - Explicit completion signal (no ambiguity)
//! - Required summary of what was accomplished
//! - Natural error recovery (agent keeps trying until this is called)
//! - Clean conversation history

use std::collections::HashMap;

use opendev_tools_core::{BaseTool, ToolContext, ToolResult};

/// Valid completion statuses.
const VALID_STATUSES: &[&str] = &["success", "partial", "failed"];

/// Tool that signals explicit task completion.
#[derive(Debug)]
pub struct TaskCompleteTool;

#[async_trait::async_trait]
impl BaseTool for TaskCompleteTool {
    fn name(&self) -> &str {
        "task_complete"
    }

    fn description(&self) -> &str {
        "Call this tool when you have completed the user's request. \
         You MUST call this tool to end the conversation. \
         Provide your response to the user in the result parameter."
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "result": {
                    "type": "string",
                    "description": "Your response to the user — what was accomplished or your conversational reply"
                },
                "status": {
                    "type": "string",
                    "description": "Completion status: 'success', 'partial', or 'failed'",
                    "enum": VALID_STATUSES,
                    "default": "success"
                }
            },
            "required": ["result"]
        })
    }

    async fn execute(
        &self,
        args: HashMap<String, serde_json::Value>,
        _ctx: &ToolContext,
    ) -> ToolResult {
        let summary = match args
            .get("result")
            .or_else(|| args.get("summary"))
            .and_then(|v| v.as_str())
        {
            Some(s) if !s.trim().is_empty() => s.trim(),
            _ => return ToolResult::fail("Result is required for task_complete"),
        };

        let status = args
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("success");

        if !VALID_STATUSES.contains(&status) {
            return ToolResult::fail(format!(
                "Invalid status '{status}'. Must be one of: {}",
                VALID_STATUSES.join(", ")
            ));
        }

        let output = format!("Task completed ({status}): {summary}");

        let mut metadata = HashMap::new();
        metadata.insert("_completion".into(), serde_json::json!(true));
        metadata.insert("summary".into(), serde_json::json!(summary));
        metadata.insert("status".into(), serde_json::json!(status));

        ToolResult::ok_with_metadata(output, metadata)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_args(pairs: &[(&str, serde_json::Value)]) -> HashMap<String, serde_json::Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[tokio::test]
    async fn test_task_complete_success() {
        let tool = TaskCompleteTool;
        let ctx = ToolContext::new("/tmp");
        let args = make_args(&[(
            "result",
            serde_json::json!("Implemented the feature and added tests"),
        )]);
        let result = tool.execute(args, &ctx).await;
        assert!(result.success);
        let output = result.output.unwrap();
        assert!(output.contains("success"));
        assert!(output.contains("Implemented the feature"));
        assert_eq!(
            result.metadata.get("_completion"),
            Some(&serde_json::json!(true))
        );
        assert_eq!(
            result.metadata.get("status"),
            Some(&serde_json::json!("success"))
        );
    }

    #[tokio::test]
    async fn test_task_complete_partial() {
        let tool = TaskCompleteTool;
        let ctx = ToolContext::new("/tmp");
        let args = make_args(&[
            ("result", serde_json::json!("Completed 3 of 5 items")),
            ("status", serde_json::json!("partial")),
        ]);
        let result = tool.execute(args, &ctx).await;
        assert!(result.success);
        assert!(result.output.unwrap().contains("partial"));
    }

    #[tokio::test]
    async fn test_task_complete_failed() {
        let tool = TaskCompleteTool;
        let ctx = ToolContext::new("/tmp");
        let args = make_args(&[
            ("result", serde_json::json!("Could not resolve the issue")),
            ("status", serde_json::json!("failed")),
        ]);
        let result = tool.execute(args, &ctx).await;
        assert!(result.success);
        assert!(result.output.unwrap().contains("failed"));
    }

    #[tokio::test]
    async fn test_task_complete_missing_result() {
        let tool = TaskCompleteTool;
        let ctx = ToolContext::new("/tmp");
        let result = tool.execute(HashMap::new(), &ctx).await;
        assert!(!result.success);
        assert!(result.error.unwrap().contains("Result is required"));
    }

    #[tokio::test]
    async fn test_task_complete_empty_result() {
        let tool = TaskCompleteTool;
        let ctx = ToolContext::new("/tmp");
        let args = make_args(&[("result", serde_json::json!("   "))]);
        let result = tool.execute(args, &ctx).await;
        assert!(!result.success);
        assert!(result.error.unwrap().contains("Result is required"));
    }

    #[tokio::test]
    async fn test_task_complete_invalid_status() {
        let tool = TaskCompleteTool;
        let ctx = ToolContext::new("/tmp");
        let args = make_args(&[
            ("result", serde_json::json!("Done")),
            ("status", serde_json::json!("cancelled")),
        ]);
        let result = tool.execute(args, &ctx).await;
        assert!(!result.success);
        assert!(result.error.unwrap().contains("Invalid status"));
    }

    #[tokio::test]
    async fn test_task_complete_default_status() {
        let tool = TaskCompleteTool;
        let ctx = ToolContext::new("/tmp");
        let args = make_args(&[("result", serde_json::json!("All done"))]);
        let result = tool.execute(args, &ctx).await;
        assert!(result.success);
        assert_eq!(
            result.metadata.get("status"),
            Some(&serde_json::json!("success"))
        );
    }

    #[tokio::test]
    async fn test_task_complete_trims_result() {
        let tool = TaskCompleteTool;
        let ctx = ToolContext::new("/tmp");
        let args = make_args(&[("result", serde_json::json!("  Trimmed result  "))]);
        let result = tool.execute(args, &ctx).await;
        assert!(result.success);
        assert_eq!(
            result.metadata.get("summary"),
            Some(&serde_json::json!("Trimmed result"))
        );
    }
}
