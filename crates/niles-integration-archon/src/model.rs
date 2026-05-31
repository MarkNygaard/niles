//! Archon data models.

use serde::Serialize;

/// Summary of a single workflow.
#[derive(Debug, Clone, Serialize)]
pub struct WorkflowSummary {
    pub name: String,
    pub description: Option<String>,
    pub source: String,
}

/// Outcome of triggering a workflow run.
#[derive(Debug, Clone, Serialize)]
pub struct TriggerOutcome {
    pub accepted: bool,
    pub status: Option<String>,
    pub conversation_id: String,
}

/// Summary of a workflow run.
#[derive(Debug, Clone, Serialize)]
pub struct RunSummary {
    pub id: String,
    pub status: String,
    pub workflow_name: Option<String>,
    pub message_preview: Option<String>,
}

/// Full detail of a workflow run.
#[derive(Debug, Clone, Serialize)]
pub struct RunDetail {
    pub id: String,
    pub status: String,
    pub workflow_name: Option<String>,
    pub message_preview: Option<String>,
    pub steps: Vec<serde_json::Value>,
}

/// Outcome of cancelling a workflow run.
#[derive(Debug, Clone, Serialize)]
pub struct CancelOutcome {
    pub success: bool,
    pub message: Option<String>,
}
