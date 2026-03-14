//! Plan approval types shared between tools and TUI.
//!
//! `PlanDecision` and `PlanApprovalRequest` live here so that both
//! `opendev-tools-impl` (which blocks inside `PresentPlanTool::execute()`)
//! and `opendev-tui` (which renders the approval panel) can reference them
//! without a circular dependency.

use tokio::sync::{mpsc, oneshot};

/// The user's decision on a presented plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanDecision {
    /// The action chosen: `"approve_auto"`, `"approve"`, or `"modify"`.
    pub action: String,
    /// Optional feedback text (empty unless the user chose to revise).
    pub feedback: String,
}

/// A request sent from `PresentPlanTool` to the TUI for user approval.
///
/// The tool creates a oneshot channel, sends this struct through an mpsc
/// channel, and then awaits the oneshot receiver. The TUI displays the
/// plan, collects the user's decision, and sends it back via `response_tx`.
#[derive(Debug)]
pub struct PlanApprovalRequest {
    /// The full plan content to display.
    pub plan_content: String,
    /// Oneshot sender the TUI uses to return the user's decision.
    pub response_tx: oneshot::Sender<PlanDecision>,
}

/// Convenience type alias for the sender half that `PresentPlanTool` holds.
pub type PlanApprovalSender = mpsc::UnboundedSender<PlanApprovalRequest>;

/// Convenience type alias for the receiver half that the TUI polls.
pub type PlanApprovalReceiver = mpsc::UnboundedReceiver<PlanApprovalRequest>;

/// Create a paired (sender, receiver) for plan approval communication.
pub fn plan_approval_channel() -> (PlanApprovalSender, PlanApprovalReceiver) {
    mpsc::unbounded_channel()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_plan_approval_roundtrip() {
        let (tx, mut rx) = plan_approval_channel();
        let (resp_tx, resp_rx) = oneshot::channel();

        tx.send(PlanApprovalRequest {
            plan_content: "Step 1: Do X".into(),
            response_tx: resp_tx,
        })
        .unwrap();

        let req = rx.recv().await.unwrap();
        assert!(req.plan_content.contains("Step 1"));

        req.response_tx
            .send(PlanDecision {
                action: "approve_auto".into(),
                feedback: String::new(),
            })
            .unwrap();

        let decision = resp_rx.await.unwrap();
        assert_eq!(decision.action, "approve_auto");
    }
}
