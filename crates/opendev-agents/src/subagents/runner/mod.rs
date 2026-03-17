//! SubagentRunner trait and implementations.
//!
//! Defines a trait for react loop strategies so each subagent type
//! can have its own loop. `StandardReactRunner` wraps the existing
//! `ReactLoop` for General/Planner/Build agents, while `SimpleReactRunner`
//! provides a stripped-down loop for Code-Explorer.

mod simple;
mod standard;

pub use simple::SimpleReactRunner;
pub use standard::StandardReactRunner;

use async_trait::async_trait;
use serde_json::Value;

use crate::llm_calls::LlmCaller;
use crate::traits::{AgentError, AgentEventCallback, AgentResult};
use opendev_http::adapted_client::AdaptedClient;
use opendev_runtime::ToolApprovalSender;
use opendev_tools_core::{ToolContext, ToolRegistry};
use tokio_util::sync::CancellationToken;

/// Dependencies bundled for the runner (avoids many-param functions).
pub struct RunnerContext<'a> {
    pub caller: &'a LlmCaller,
    pub http_client: &'a AdaptedClient,
    pub tool_schemas: &'a [Value],
    pub tool_registry: &'a ToolRegistry,
    pub tool_context: &'a ToolContext,
    pub event_callback: Option<&'a dyn AgentEventCallback>,
    pub cancel: Option<&'a CancellationToken>,
    pub tool_approval_tx: Option<&'a ToolApprovalSender>,
}

/// Trait for react loop strategies.
#[async_trait]
pub trait SubagentRunner: Send + Sync {
    /// Run the react loop over the given message history.
    async fn run(
        &self,
        ctx: &RunnerContext<'_>,
        messages: &mut Vec<Value>,
    ) -> Result<AgentResult, AgentError>;

    /// Name of this runner (for logging).
    fn name(&self) -> &str;
}
