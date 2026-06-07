//! Linear workflow tools — expose Linear to the LLM.

use crate::error::{Error, Result};
use crate::registry::ToolRegistry;
use crate::tool::{Tool, ToolDescriptor};
use async_trait::async_trait;
use niles_integration_linear::LinearClient;
use serde_json::{Value, json};
use std::sync::Arc;

fn map_linear_err<T>(r: std::result::Result<T, niles_integration_linear::Error>) -> Result<T> {
    r.map_err(|e| Error::Linear(e.to_string()))
}

pub struct CreateTaskTool {
    client: Arc<LinearClient>,
}

impl CreateTaskTool {
    pub fn new(client: Arc<LinearClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for CreateTaskTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "create_task".into(),
            description: "Create a coding task in Linear. \
                This files an issue in the Todo column with the 'AI Eligible' label \
                so that a self-hosted AI harness picks it up and turns it into a GitHub PR. \
                Write a clear title and a description that reads as a task brief."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["title", "description"],
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "Short, clear title for the task."
                    },
                    "description": {
                        "type": "string",
                        "description": "Detailed description that reads as a task brief."
                    }
                }
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let title = match args.get("title") {
            Some(v) => v.as_str().ok_or_else(|| Error::InvalidArgs {
                tool: "create_task".into(),
                reason: "title must be a string".into(),
            })?,
            None => {
                return Err(Error::InvalidArgs {
                    tool: "create_task".into(),
                    reason: "title is required".into(),
                });
            }
        };
        if title.trim().is_empty() {
            return Err(Error::InvalidArgs {
                tool: "create_task".into(),
                reason: "title must not be empty".into(),
            });
        }
        let description = match args.get("description") {
            Some(v) => v.as_str().ok_or_else(|| Error::InvalidArgs {
                tool: "create_task".into(),
                reason: "description must be a string".into(),
            })?,
            None => {
                return Err(Error::InvalidArgs {
                    tool: "create_task".into(),
                    reason: "description is required".into(),
                });
            }
        };
        if description.trim().is_empty() {
            return Err(Error::InvalidArgs {
                tool: "create_task".into(),
                reason: "description must not be empty".into(),
            });
        }

        let task = map_linear_err(self.client.create_task(title, description).await)?;
        Ok(json!({
            "ok": true,
            "identifier": task.identifier,
            "url": task.url,
        }))
    }
}

pub struct ListTasksTool {
    client: Arc<LinearClient>,
}

impl ListTasksTool {
    pub fn new(client: Arc<LinearClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for ListTasksTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "list_tasks".into(),
            description: "List tasks (issues) from Linear for the configured team.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "status": {
                        "type": "string",
                        "description": "Optional state name to filter by (e.g. 'Todo', 'In Progress')."
                    }
                }
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let status = match args.get("status") {
            Some(v) => {
                let s = v.as_str().ok_or_else(|| Error::InvalidArgs {
                    tool: "list_tasks".into(),
                    reason: "status must be a string".into(),
                })?;
                if s.trim().is_empty() {
                    return Err(Error::InvalidArgs {
                        tool: "list_tasks".into(),
                        reason: "status must not be empty".into(),
                    });
                }
                Some(s)
            }
            None => None,
        };

        let tasks = map_linear_err(self.client.list_tasks(status).await)?;
        let items: Vec<Value> = tasks
            .iter()
            .map(|t| {
                json!({
                    "identifier": t.identifier,
                    "title": t.title,
                    "state": t.state,
                })
            })
            .collect();
        Ok(json!({ "tasks": items }))
    }
}

pub struct GetTaskTool {
    client: Arc<LinearClient>,
}

impl GetTaskTool {
    pub fn new(client: Arc<LinearClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for GetTaskTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "get_task".into(),
            description:
                "Get full detail for a single Linear task by its identifier (e.g. 'NIL-1').".into(),
            parameters: json!({
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "The task identifier to look up."
                    }
                }
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let id = match args.get("id") {
            Some(v) => v.as_str().ok_or_else(|| Error::InvalidArgs {
                tool: "get_task".into(),
                reason: "id must be a string".into(),
            })?,
            None => {
                return Err(Error::InvalidArgs {
                    tool: "get_task".into(),
                    reason: "id is required".into(),
                });
            }
        };
        if id.trim().is_empty() {
            return Err(Error::InvalidArgs {
                tool: "get_task".into(),
                reason: "id must not be empty".into(),
            });
        }

        let task = map_linear_err(self.client.get_task(id).await)?;
        Ok(json!({
            "identifier": task.identifier,
            "title": task.title,
            "state": task.state,
            "url": task.url,
            "pr_url": task.pr_url,
        }))
    }
}

/// Register all three Linear tools onto an existing registry.
pub fn register_linear_tools(reg: &mut ToolRegistry, client: Arc<LinearClient>) {
    reg.register(Box::new(CreateTaskTool::new(client.clone())));
    reg.register(Box::new(ListTasksTool::new(client.clone())));
    reg.register(Box::new(GetTaskTool::new(client)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use niles_integration_linear::{LinearConfig, LinearTransport};
    use parking_lot::Mutex;
    use serde_json::json;
    use std::sync::Arc;
    use std::time::Duration;

    #[derive(Clone)]
    struct MockTransport {
        responses: Arc<Mutex<Vec<std::result::Result<String, niles_integration_linear::Error>>>>,
    }

    impl MockTransport {
        fn new(
            responses: Vec<std::result::Result<String, niles_integration_linear::Error>>,
        ) -> Self {
            Self {
                responses: Arc::new(Mutex::new(responses)),
            }
        }
    }

    #[async_trait]
    impl LinearTransport for MockTransport {
        async fn post_graphql(
            &self,
            _body: &str,
        ) -> std::result::Result<String, niles_integration_linear::Error> {
            self.responses.lock().remove(0)
        }
    }

    fn tool_client(
        responses: Vec<std::result::Result<String, niles_integration_linear::Error>>,
    ) -> Arc<LinearClient> {
        let cfg = LinearConfig {
            api_key: "lin_test".into(),
            team: "NILES".into(),
            trigger_label: "AI Eligible".into(),
            todo_state: "Todo".into(),
            request_timeout: Duration::from_secs(10),
        };
        Arc::new(LinearClient::with_transport(
            Arc::new(MockTransport::new(responses)),
            cfg,
        ))
    }

    fn resolve_json() -> String {
        json!({
            "data": {
                "teams": {
                    "nodes": [
                        {
                            "id": "99311a1d-1234-5678-9abc-def012345678",
                            "key": "NILES",
                            "name": "Niles Team",
                            "states": {
                                "nodes": [
                                    { "id": "a488ecdb-1234-5678-9abc-def012345678", "name": "Todo" }
                                ]
                            }
                        }
                    ]
                },
                "issueLabels": {
                    "nodes": [
                        { "id": "504fdbac-1234-5678-9abc-def012345678", "name": "AI Eligible" }
                    ]
                }
            }
        })
        .to_string()
    }

    #[tokio::test]
    async fn create_task_tool_happy_path() {
        let client = tool_client(vec![
            Ok(resolve_json()),
            Ok(json!({
                "data": {
                    "issueCreate": {
                        "success": true,
                        "issue": {
                            "id": "i1",
                            "identifier": "NILES-42",
                            "url": "https://linear.app/issue/NILES-42"
                        }
                    }
                }
            })
            .to_string()),
        ]);
        let tool = CreateTaskTool::new(client);
        let result = tool
            .execute(json!({"title": "Fix bug", "description": "desc"}))
            .await
            .unwrap();
        assert_eq!(result["ok"], true);
        assert_eq!(result["identifier"], "NILES-42");
    }

    #[tokio::test]
    async fn create_task_tool_missing_title_errors() {
        let client = tool_client(vec![]);
        let tool = CreateTaskTool::new(client);
        let err = tool
            .execute(json!({"description": "desc"}))
            .await
            .unwrap_err();
        assert!(
            matches!(err, Error::InvalidArgs { tool, reason } if tool == "create_task" && reason.contains("title is required"))
        );
    }

    #[tokio::test]
    async fn create_task_tool_non_string_title_errors() {
        let client = tool_client(vec![]);
        let tool = CreateTaskTool::new(client);
        let err = tool
            .execute(json!({"title": 42, "description": "desc"}))
            .await
            .unwrap_err();
        assert!(
            matches!(err, Error::InvalidArgs { tool, reason } if tool == "create_task" && reason.contains("title must be a string"))
        );
    }

    #[tokio::test]
    async fn create_task_tool_empty_title_errors() {
        let client = tool_client(vec![]);
        let tool = CreateTaskTool::new(client);
        let err = tool
            .execute(json!({"title": "", "description": "desc"}))
            .await
            .unwrap_err();
        assert!(
            matches!(err, Error::InvalidArgs { tool, reason } if tool == "create_task" && reason.contains("title must not be empty"))
        );
    }

    #[tokio::test]
    async fn create_task_tool_missing_description_errors() {
        let client = tool_client(vec![]);
        let tool = CreateTaskTool::new(client);
        let err = tool.execute(json!({"title": "Fix bug"})).await.unwrap_err();
        assert!(
            matches!(err, Error::InvalidArgs { tool, reason } if tool == "create_task" && reason.contains("description is required"))
        );
    }

    #[tokio::test]
    async fn create_task_tool_non_string_description_errors() {
        let client = tool_client(vec![]);
        let tool = CreateTaskTool::new(client);
        let err = tool
            .execute(json!({"title": "Fix bug", "description": 42}))
            .await
            .unwrap_err();
        assert!(
            matches!(err, Error::InvalidArgs { tool, reason } if tool == "create_task" && reason.contains("description must be a string"))
        );
    }

    #[tokio::test]
    async fn create_task_tool_empty_description_errors() {
        let client = tool_client(vec![]);
        let tool = CreateTaskTool::new(client);
        let err = tool
            .execute(json!({"title": "Fix bug", "description": ""}))
            .await
            .unwrap_err();
        assert!(
            matches!(err, Error::InvalidArgs { tool, reason } if tool == "create_task" && reason.contains("description must not be empty"))
        );
    }

    #[tokio::test]
    async fn list_tasks_tool_happy_path() {
        let client = tool_client(vec![
            Ok(resolve_json()),
            Ok(json!({
                "data": {
                    "issues": {
                        "nodes": [
                            {
                                "identifier": "NILES-1",
                                "title": "First",
                                "state": { "name": "Todo" }
                            }
                        ]
                    }
                }
            })
            .to_string()),
        ]);
        let tool = ListTasksTool::new(client);
        let result = tool.execute(json!({})).await.unwrap();
        let tasks = result["tasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["identifier"], "NILES-1");
    }

    #[tokio::test]
    async fn list_tasks_tool_with_status_filter() {
        let client = tool_client(vec![
            Ok(resolve_json()),
            Ok(json!({ "data": { "issues": { "nodes": [] } } }).to_string()),
        ]);
        let tool = ListTasksTool::new(client);
        let result = tool
            .execute(json!({"status": "In Progress"}))
            .await
            .unwrap();
        let tasks = result["tasks"].as_array().unwrap();
        assert!(tasks.is_empty());
    }

    #[tokio::test]
    async fn list_tasks_tool_empty_status_errors() {
        let client = tool_client(vec![]);
        let tool = ListTasksTool::new(client);
        let err = tool.execute(json!({"status": ""})).await.unwrap_err();
        assert!(
            matches!(err, Error::InvalidArgs { tool, reason } if tool == "list_tasks" && reason.contains("status must not be empty"))
        );
    }

    #[tokio::test]
    async fn get_task_tool_happy_path() {
        let client = tool_client(vec![Ok(json!({
            "data": {
                "issue": {
                    "identifier": "NILES-7",
                    "title": "Seven",
                    "url": "https://linear.app/issue/NILES-7",
                    "state": { "name": "Done" },
                    "attachments": { "nodes": [] }
                }
            }
        })
        .to_string())]);
        let tool = GetTaskTool::new(client);
        let result = tool.execute(json!({"id": "NILES-7"})).await.unwrap();
        assert_eq!(result["identifier"], "NILES-7");
        assert_eq!(result["state"], "Done");
    }

    #[tokio::test]
    async fn get_task_tool_not_found_propagates() {
        let client = tool_client(vec![Ok(json!({ "data": { "issue": null } }).to_string())]);
        let tool = GetTaskTool::new(client);
        let err = tool.execute(json!({"id": "NILES-999"})).await.unwrap_err();
        assert!(matches!(err, Error::Linear(reason) if reason.contains("NILES-999")));
    }

    #[tokio::test]
    async fn get_task_tool_empty_id_errors() {
        let client = tool_client(vec![]);
        let tool = GetTaskTool::new(client);
        let err = tool.execute(json!({"id": ""})).await.unwrap_err();
        assert!(
            matches!(err, Error::InvalidArgs { tool, reason } if tool == "get_task" && reason.contains("id must not be empty"))
        );
    }

    #[tokio::test]
    async fn tool_propagates_linear_error() {
        let client = tool_client(vec![Err(niles_integration_linear::Error::BadStatus {
            status: 500,
            body: "server error".into(),
        })]);
        let tool = ListTasksTool::new(client);
        let err = tool.execute(json!({})).await.unwrap_err();
        assert!(matches!(err, Error::Linear(reason) if reason.contains("500")));
    }
}
