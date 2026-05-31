//! Archon workflow tools — expose Archon to the LLM.

use crate::error::{Error, Result};
use crate::registry::ToolRegistry;
use crate::tool::{Tool, ToolDescriptor};
use async_trait::async_trait;
use niles_integration_archon::ArchonClient;
use serde_json::{Value, json};
use std::sync::Arc;

fn map_archon_err<T>(r: std::result::Result<T, niles_integration_archon::Error>) -> Result<T> {
    r.map_err(|e| Error::Archon(e.to_string()))
}

pub struct ListWorkflowsTool {
    client: Arc<ArchonClient>,
}

impl ListWorkflowsTool {
    pub fn new(client: Arc<ArchonClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for ListWorkflowsTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "list_workflows".into(),
            description: "List the workflows you can trigger on the Archon workflow engine. \
                Returns each workflow's name, source (bundled or project), and description."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {}
            }),
        }
    }

    async fn execute(&self, _args: Value) -> Result<Value> {
        let workflows = map_archon_err(self.client.list_workflows().await)?;
        let items: Vec<Value> = workflows
            .iter()
            .map(|w| {
                json!({
                    "name": w.name,
                    "source": w.source,
                    "description": w.description,
                })
            })
            .collect();
        Ok(json!({ "workflows": items }))
    }
}

pub struct RunWorkflowTool {
    client: Arc<ArchonClient>,
}

impl RunWorkflowTool {
    pub fn new(client: Arc<ArchonClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for RunWorkflowTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "run_workflow".into(),
            description: "Trigger an Archon workflow by name with a message. \
                Returns `accepted: true` if Archon queued the run. \
                NOTE: 'accepted' does not guarantee the run actually started — \
                Archon may drop it. Always follow up with `list_workflow_runs` \
                to verify the run is in the active list."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["name", "message"],
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "The workflow name to trigger."
                    },
                    "message": {
                        "type": "string",
                        "description": "The message or instruction to pass to the workflow."
                    }
                }
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let name = match args.get("name") {
            Some(v) => v.as_str().ok_or_else(|| Error::InvalidArgs {
                tool: "run_workflow".into(),
                reason: "name must be a string".into(),
            })?,
            None => {
                return Err(Error::InvalidArgs {
                    tool: "run_workflow".into(),
                    reason: "name is required".into(),
                });
            }
        };
        let message = match args.get("message") {
            Some(v) => v.as_str().ok_or_else(|| Error::InvalidArgs {
                tool: "run_workflow".into(),
                reason: "message must be a string".into(),
            })?,
            None => {
                return Err(Error::InvalidArgs {
                    tool: "run_workflow".into(),
                    reason: "message is required".into(),
                });
            }
        };

        let outcome = map_archon_err(self.client.run_workflow(name, message).await)?;
        Ok(json!({
            "accepted": outcome.accepted,
            "status": outcome.status,
            "conversation_id": outcome.conversation_id,
        }))
    }
}

pub struct ListWorkflowRunsTool {
    client: Arc<ArchonClient>,
}

impl ListWorkflowRunsTool {
    pub fn new(client: Arc<ArchonClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for ListWorkflowRunsTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "list_workflow_runs".into(),
            description: "List recent Archon workflow runs (newest first). \
                Use this to see what's currently running or recently finished."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 20,
                        "description": "Maximum number of runs to return (1–20). Default: 5."
                    }
                }
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let limit = match args.get("limit") {
            Some(v) => v.as_u64().ok_or_else(|| Error::InvalidArgs {
                tool: "list_workflow_runs".into(),
                reason: "limit must be an integer".into(),
            })?,
            None => 5,
        };

        if !(1..=20).contains(&limit) {
            return Err(Error::InvalidArgs {
                tool: "list_workflow_runs".into(),
                reason: "limit must be between 1 and 20".into(),
            });
        }

        let runs = map_archon_err(self.client.list_workflow_runs(limit).await)?;
        let items: Vec<Value> = runs
            .iter()
            .map(|r| {
                json!({
                    "id": r.id,
                    "status": r.status,
                    "workflow_name": r.workflow_name,
                    "message_preview": r.message_preview,
                })
            })
            .collect();
        Ok(json!({ "runs": items }))
    }
}

pub struct GetWorkflowRunTool {
    client: Arc<ArchonClient>,
}

impl GetWorkflowRunTool {
    pub fn new(client: Arc<ArchonClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for GetWorkflowRunTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "get_workflow_run".into(),
            description: "Get the full status of a single Archon workflow run by id, \
                including step-level progress."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["run_id"],
                "properties": {
                    "run_id": {
                        "type": "string",
                        "description": "The run id to look up."
                    }
                }
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let run_id = match args.get("run_id") {
            Some(v) => v.as_str().ok_or_else(|| Error::InvalidArgs {
                tool: "get_workflow_run".into(),
                reason: "run_id must be a string".into(),
            })?,
            None => {
                return Err(Error::InvalidArgs {
                    tool: "get_workflow_run".into(),
                    reason: "run_id is required".into(),
                });
            }
        };

        let run = map_archon_err(self.client.get_workflow_run(run_id).await)?;
        Ok(json!({
            "id": run.id,
            "status": run.status,
            "workflow_name": run.workflow_name,
            "message_preview": run.message_preview,
            "steps": run.steps,
        }))
    }
}

pub struct CancelWorkflowRunTool {
    client: Arc<ArchonClient>,
}

impl CancelWorkflowRunTool {
    pub fn new(client: Arc<ArchonClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for CancelWorkflowRunTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "cancel_workflow_run".into(),
            description: "Cancel a running Archon workflow by id.".into(),
            parameters: json!({
                "type": "object",
                "required": ["run_id"],
                "properties": {
                    "run_id": {
                        "type": "string",
                        "description": "The run id to cancel."
                    }
                }
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let run_id = match args.get("run_id") {
            Some(v) => v.as_str().ok_or_else(|| Error::InvalidArgs {
                tool: "cancel_workflow_run".into(),
                reason: "run_id must be a string".into(),
            })?,
            None => {
                return Err(Error::InvalidArgs {
                    tool: "cancel_workflow_run".into(),
                    reason: "run_id is required".into(),
                });
            }
        };

        let outcome = map_archon_err(self.client.cancel_workflow_run(run_id).await)?;
        Ok(json!({
            "success": outcome.success,
            "message": outcome.message,
        }))
    }
}

/// Register all five Archon tools onto an existing registry.
pub fn register_archon_tools(reg: &mut ToolRegistry, client: Arc<ArchonClient>) {
    reg.register(Box::new(ListWorkflowsTool::new(client.clone())));
    reg.register(Box::new(RunWorkflowTool::new(client.clone())));
    reg.register(Box::new(ListWorkflowRunsTool::new(client.clone())));
    reg.register(Box::new(GetWorkflowRunTool::new(client.clone())));
    reg.register(Box::new(CancelWorkflowRunTool::new(client)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use niles_integration_archon::{ArchonConfig, ArchonTransport};
    use serde_json::json;
    use std::sync::Mutex;
    use std::time::Duration;

    #[derive(Clone)]
    struct MockTransport {
        responses: Arc<Mutex<Vec<std::result::Result<String, niles_integration_archon::Error>>>>,
    }

    impl MockTransport {
        fn new(
            responses: Vec<std::result::Result<String, niles_integration_archon::Error>>,
        ) -> Self {
            Self {
                responses: Arc::new(Mutex::new(responses)),
            }
        }
    }

    #[async_trait]
    impl ArchonTransport for MockTransport {
        async fn get(
            &self,
            _url: &str,
        ) -> std::result::Result<String, niles_integration_archon::Error> {
            self.responses.lock().unwrap().remove(0)
        }

        async fn post(
            &self,
            _url: &str,
            _body: &str,
        ) -> std::result::Result<String, niles_integration_archon::Error> {
            self.responses.lock().unwrap().remove(0)
        }
    }

    fn tool_client(
        responses: Vec<std::result::Result<String, niles_integration_archon::Error>>,
    ) -> Arc<ArchonClient> {
        let cfg = ArchonConfig {
            base_url: "https://archon.example.com".into(),
            codebase_id: "x".into(),
            cwd: None,
            request_timeout: Duration::from_secs(10),
        };
        Arc::new(ArchonClient::with_transport(
            Arc::new(MockTransport::new(responses)),
            cfg,
        ))
    }

    #[tokio::test]
    async fn list_workflows_tool_happy_path() {
        let client = tool_client(vec![Ok(
            r#"{"workflows":[{"workflow":{"name":"deploy"},"source":"bundled"}]}"#.into(),
        )]);
        let tool = ListWorkflowsTool::new(client);
        let result = tool.execute(json!({})).await.unwrap();
        let workflows = result["workflows"].as_array().unwrap();
        assert_eq!(workflows.len(), 1);
        assert_eq!(workflows[0]["name"], "deploy");
    }

    #[tokio::test]
    async fn run_workflow_tool_happy_path() {
        let client = tool_client(vec![
            Ok(r#"{"conversationId":"c1"}"#.into()),
            Ok(r#"{"accepted":true,"status":"started"}"#.into()),
        ]);
        let tool = RunWorkflowTool::new(client);
        let result = tool
            .execute(json!({"name": "deploy", "message": "go"}))
            .await
            .unwrap();
        assert_eq!(result["accepted"], true);
        assert_eq!(result["status"], "started");
    }

    #[tokio::test]
    async fn run_workflow_tool_missing_message_errors() {
        let client = tool_client(vec![]);
        let tool = RunWorkflowTool::new(client);
        let err = tool.execute(json!({"name": "deploy"})).await.unwrap_err();
        assert!(
            matches!(err, Error::InvalidArgs { tool, reason } if tool == "run_workflow" && reason.contains("message is required"))
        );
    }

    #[tokio::test]
    async fn list_workflow_runs_tool_clamps_limit() {
        let client = tool_client(vec![Ok(r#"{"runs":[]}"#.into())]);
        let tool = ListWorkflowRunsTool::new(client);
        let result = tool.execute(json!({})).await.unwrap();
        let runs = result["runs"].as_array().unwrap();
        assert!(runs.is_empty());
    }

    #[tokio::test]
    async fn list_workflow_runs_tool_rejects_too_large_limit() {
        let client = tool_client(vec![]);
        let tool = ListWorkflowRunsTool::new(client);
        let err = tool.execute(json!({"limit": 100})).await.unwrap_err();
        assert!(
            matches!(err, Error::InvalidArgs { tool, reason } if tool == "list_workflow_runs" && reason.contains("limit must be between 1 and 20"))
        );
    }

    #[tokio::test]
    async fn get_workflow_run_tool_happy_path() {
        let client = tool_client(vec![Ok(
            r#"{"run":{"id":"r1","status":"done"},"events":[]}"#.into(),
        )]);
        let tool = GetWorkflowRunTool::new(client);
        let result = tool.execute(json!({"run_id": "r1"})).await.unwrap();
        assert_eq!(result["id"], "r1");
        assert_eq!(result["status"], "done");
    }

    #[tokio::test]
    async fn cancel_workflow_run_tool_happy_path() {
        let client = tool_client(vec![Ok(r#"{"success":true,"message":"cancelled"}"#.into())]);
        let tool = CancelWorkflowRunTool::new(client);
        let result = tool.execute(json!({"run_id": "r1"})).await.unwrap();
        assert_eq!(result["success"], true);
    }

    #[tokio::test]
    async fn tool_propagates_archon_error() {
        let client = tool_client(vec![Err(niles_integration_archon::Error::BadStatus {
            status: 500,
            body: "server error".into(),
        })]);
        let tool = ListWorkflowsTool::new(client);
        let err = tool.execute(json!({})).await.unwrap_err();
        assert!(matches!(err, Error::Archon(reason) if reason.contains("500")));
    }

    #[tokio::test]
    async fn run_workflow_tool_descriptor_contains_warning() {
        let client = tool_client(vec![]);
        let tool = RunWorkflowTool::new(client);
        let desc = tool.descriptor();
        assert!(desc.description.contains("accepted"));
        assert!(desc.description.contains("list_workflow_runs"));
    }
}
