//! Linear GraphQL client.

use crate::error::{Error, Result};
use crate::model::{TaskDetail, TaskRef, TaskSummary};
use crate::transport::{HttpTransport, LinearTransport};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::OnceCell;

/// Client configuration.
#[derive(Debug, Clone)]
pub struct LinearConfig {
    pub api_key: String,
    pub team: String,
    pub trigger_label: String,
    pub todo_state: String,
    pub request_timeout: Duration,
}

struct ResolvedIds {
    team_id: String,
    state_id: String,
    label_id: String,
}

/// Linear issue-tracker client.
pub struct LinearClient {
    transport: Arc<dyn LinearTransport>,
    cfg: LinearConfig,
    resolved: OnceCell<ResolvedIds>,
}

impl LinearClient {
    /// Create a client using the default [`HttpTransport`].
    pub fn new(cfg: LinearConfig) -> Result<Self> {
        let transport = Arc::new(HttpTransport::new(
            &cfg.api_key,
            concat!(
                "niles/",
                env!("CARGO_PKG_VERSION"),
                " (https://github.com/MarkNygaard/niles)"
            ),
            cfg.request_timeout,
        )?);
        Ok(Self::with_transport(transport, cfg))
    }

    /// Create a client with a custom transport (useful for testing).
    pub fn with_transport(transport: Arc<dyn LinearTransport>, cfg: LinearConfig) -> Self {
        Self {
            transport,
            cfg,
            resolved: OnceCell::new(),
        }
    }

    async fn graphql<T: DeserializeOwned>(&self, query: &str, variables: Value) -> Result<T> {
        let body = json!({ "query": query, "variables": variables }).to_string();
        let raw = self.transport.post_graphql(&body).await?;
        let resp: GraphQlResp<T> = serde_json::from_str(&raw).map_err(|e| Error::Parse {
            reason: e.to_string(),
        })?;
        if let Some(errors) = resp.errors
            && !errors.is_empty()
        {
            return Err(Error::Api {
                reason: errors
                    .iter()
                    .map(|e| e.message.clone())
                    .collect::<Vec<_>>()
                    .join("; "),
            });
        }
        resp.data.ok_or(Error::Parse {
            reason: "missing data field".into(),
        })
    }

    async fn resolved(&self) -> Result<&ResolvedIds> {
        self.resolved.get_or_try_init(|| self.resolve_ids()).await
    }

    async fn resolve_ids(&self) -> Result<ResolvedIds> {
        let query = r#"
            query Resolve {
                teams { nodes { id key name states { nodes { id name } } } }
                issueLabels { nodes { id name } }
            }
        "#;
        let resp: ResolveResp = self.graphql(query, json!({})).await?;

        let team = resp
            .teams
            .nodes
            .into_iter()
            .find(|t| t.key == self.cfg.team || t.name == self.cfg.team)
            .ok_or(Error::Resolve {
                kind: "team",
                name: self.cfg.team.clone(),
            })?;

        let state = team
            .states
            .nodes
            .into_iter()
            .find(|s| s.name == self.cfg.todo_state)
            .ok_or(Error::Resolve {
                kind: "state",
                name: self.cfg.todo_state.clone(),
            })?;

        let label = resp
            .issue_labels
            .nodes
            .into_iter()
            .find(|l| l.name == self.cfg.trigger_label)
            .ok_or(Error::Resolve {
                kind: "label",
                name: self.cfg.trigger_label.clone(),
            })?;

        Ok(ResolvedIds {
            team_id: team.id,
            state_id: state.id,
            label_id: label.id,
        })
    }

    /// Create a task (issue) in Linear.
    pub async fn create_task(&self, title: &str, description: &str) -> Result<TaskRef> {
        let ids = self.resolved().await?;
        let query = r#"
            mutation Create($input: IssueCreateInput!) {
                issueCreate(input: $input) { success issue { id identifier url } }
            }
        "#;
        let variables = json!({
            "input": {
                "teamId": ids.team_id,
                "title": title,
                "description": description,
                "stateId": ids.state_id,
                "labelIds": [ids.label_id],
            }
        });
        let resp: CreateResp = self.graphql(query, variables).await?;
        if !resp.issue_create.success {
            return Err(Error::Api {
                reason: "issueCreate returned success=false".into(),
            });
        }
        let issue = resp.issue_create.issue.ok_or(Error::Api {
            reason: "issueCreate returned success=true but issue was null".into(),
        })?;
        Ok(TaskRef {
            id: issue.id,
            identifier: issue.identifier,
            url: issue.url,
        })
    }

    /// List tasks (issues) for the configured team.
    pub async fn list_tasks(&self, state_name: Option<&str>) -> Result<Vec<TaskSummary>> {
        let ids = self.resolved().await?;
        let mut filter = json!({ "team": { "id": { "eq": ids.team_id } } });
        if let Some(state) = state_name {
            filter["state"] = json!({ "name": { "eq": state } });
        }
        let query = r#"
            query List($filter: IssueFilter!) {
                issues(filter: $filter) { nodes { identifier title state { name } } }
            }
        "#;
        let resp: ListResp = self.graphql(query, json!({ "filter": filter })).await?;
        Ok(resp
            .issues
            .nodes
            .into_iter()
            .map(|n| TaskSummary {
                identifier: n.identifier,
                title: n.title,
                state: n.state.map(|s| s.name).unwrap_or_default(),
            })
            .collect())
    }

    /// Get full detail for a single task by its human identifier (e.g. "NIL-1").
    pub async fn get_task(&self, identifier: &str) -> Result<TaskDetail> {
        let query = r#"
            query Get($id: String!) {
                issue(id: $id) {
                    identifier title url state { name }
                    attachments { nodes { url title } }
                }
            }
        "#;
        let resp: GetResp = self.graphql(query, json!({ "id": identifier })).await?;
        let issue = resp.issue.ok_or(Error::Resolve {
            kind: "issue",
            name: identifier.into(),
        })?;

        // v1: surface linked PR url from attachments only.
        // Full PR-state querying is out of scope.
        let pr_url = issue.attachments.nodes.into_iter().find_map(|a| {
            if a.url.contains("github.com") && a.url.contains("/pull/") {
                Some(a.url)
            } else {
                None
            }
        });

        Ok(TaskDetail {
            identifier: issue.identifier,
            title: issue.title,
            state: issue.state.map(|s| s.name).unwrap_or_default(),
            url: issue.url,
            pr_url,
        })
    }
}

#[derive(Deserialize)]
struct GraphQlResp<T> {
    data: Option<T>,
    errors: Option<Vec<GqlError>>,
}

#[derive(Deserialize)]
struct GqlError {
    message: String,
}

#[derive(Deserialize)]
struct ResolveResp {
    teams: TeamList,
    #[serde(rename = "issueLabels")]
    issue_labels: LabelList,
}

#[derive(Deserialize)]
struct TeamList {
    nodes: Vec<TeamNode>,
}

#[derive(Deserialize)]
struct TeamNode {
    id: String,
    key: String,
    name: String,
    states: StateList,
}

#[derive(Deserialize)]
struct StateList {
    nodes: Vec<StateNode>,
}

#[derive(Deserialize)]
struct StateNode {
    id: String,
    name: String,
}

#[derive(Deserialize)]
struct LabelList {
    nodes: Vec<LabelNode>,
}

#[derive(Deserialize)]
struct LabelNode {
    id: String,
    name: String,
}

#[derive(Deserialize)]
struct CreateResp {
    #[serde(rename = "issueCreate")]
    issue_create: IssueCreateResult,
}

#[derive(Deserialize)]
struct IssueCreateResult {
    success: bool,
    issue: Option<IssueWire>,
}

#[derive(Deserialize)]
struct IssueWire {
    id: String,
    identifier: String,
    url: String,
}

#[derive(Deserialize)]
struct ListResp {
    issues: IssueNodeList,
}

#[derive(Deserialize)]
struct IssueNodeList {
    nodes: Vec<IssueSummaryNode>,
}

#[derive(Deserialize)]
struct IssueSummaryNode {
    identifier: String,
    title: String,
    state: Option<StateName>,
}

#[derive(Deserialize)]
struct StateName {
    name: String,
}

#[derive(Deserialize)]
struct GetResp {
    issue: Option<IssueDetailWire>,
}

#[derive(Deserialize)]
struct IssueDetailWire {
    identifier: String,
    title: String,
    url: String,
    state: Option<StateName>,
    attachments: AttachmentList,
}

#[derive(Deserialize)]
struct AttachmentList {
    nodes: Vec<AttachmentNode>,
}

#[derive(Deserialize)]
struct AttachmentNode {
    url: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use parking_lot::Mutex;

    #[derive(Clone)]
    struct MockTransport {
        last_post_body: Arc<Mutex<Option<String>>>,
        responses: Arc<Mutex<Vec<Result<String>>>>,
        call_count: Arc<Mutex<usize>>,
    }
    impl MockTransport {
        fn new(responses: Vec<Result<String>>) -> Self {
            Self {
                last_post_body: Arc::new(Mutex::new(None)),
                responses: Arc::new(Mutex::new(responses)),
                call_count: Arc::new(Mutex::new(0)),
            }
        }
        fn last_post_body(&self) -> Option<String> {
            self.last_post_body.lock().clone()
        }
        fn call_count(&self) -> usize {
            *self.call_count.lock()
        }
    }

    #[async_trait]
    impl LinearTransport for MockTransport {
        async fn post_graphql(&self, body: &str) -> Result<String> {
            *self.last_post_body.lock() = Some(body.to_string());
            *self.call_count.lock() += 1;
            self.responses.lock().remove(0)
        }
    }

    fn client_with(responses: Vec<Result<String>>) -> (MockTransport, LinearClient) {
        let mock = MockTransport::new(responses);
        let cfg = LinearConfig {
            api_key: "lin_test_key".into(),
            team: "NILES".into(),
            trigger_label: "AI Eligible".into(),
            todo_state: "Todo".into(),
            request_timeout: Duration::from_secs(10),
        };
        let client = LinearClient::with_transport(Arc::new(mock.clone()), cfg);
        (mock, client)
    }

    fn cfg() -> LinearConfig {
        LinearConfig {
            api_key: "lin_test_key".into(),
            team: "NILES".into(),
            trigger_label: "AI Eligible".into(),
            todo_state: "Todo".into(),
            request_timeout: Duration::from_secs(10),
        }
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
    async fn create_task_happy_path() {
        let (mock, client) = client_with(vec![
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
        let task = client.create_task("Fix bug", "desc").await.unwrap();
        assert_eq!(task.identifier, "NILES-42");
        assert_eq!(task.url, "https://linear.app/issue/NILES-42");
        let body = mock.last_post_body().unwrap();
        assert!(body.contains("issueCreate"));
        assert!(body.contains("Fix bug"));
        assert!(body.contains("desc"));
    }

    #[test]
    fn new_rejects_invalid_api_key_header() {
        let mut cfg = cfg();
        cfg.api_key = "lin_test_key\n".into();
        let err = match LinearClient::new(cfg) {
            Ok(_) => panic!("invalid api key header should be rejected"),
            Err(err) => err,
        };
        assert!(matches!(err, Error::InvalidHeader { name, .. } if name == "authorization"));
    }

    #[tokio::test]
    async fn create_task_failure() {
        let (_, client) = client_with(vec![
            Ok(resolve_json()),
            Ok(json!({
                "data": {
                    "issueCreate": {
                        "success": false,
                        "issue": null
                    }
                }
            })
            .to_string()),
        ]);
        let err = client.create_task("Fix bug", "desc").await.unwrap_err();
        assert!(matches!(err, Error::Api { .. }));
    }
    #[tokio::test]
    async fn create_task_success_with_null_issue() {
        let (_, client) = client_with(vec![
            Ok(resolve_json()),
            Ok(json!({
                "data": {
                    "issueCreate": {
                        "success": true,
                        "issue": null
                    }
                }
            })
            .to_string()),
        ]);
        let err = client.create_task("Fix bug", "desc").await.unwrap_err();
        assert!(
            matches!(err, Error::Api { reason } if reason.contains("success=true but issue was null"))
        );
    }
    #[tokio::test]
    async fn list_tasks_happy_path() {
        let (mock, client) = client_with(vec![
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
        let tasks = client.list_tasks(None).await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].identifier, "NILES-1");
        assert_eq!(tasks[0].state, "Todo");
        let body = mock.last_post_body().unwrap();
        assert!(body.contains("issues"));
    }
    #[tokio::test]
    async fn list_tasks_with_state_filter() {
        let (mock, client) = client_with(vec![
            Ok(resolve_json()),
            Ok(json!({ "data": { "issues": { "nodes": [] } } }).to_string()),
        ]);
        let tasks = client.list_tasks(Some("In Progress")).await.unwrap();
        assert!(tasks.is_empty());
        let body = mock.last_post_body().unwrap();
        assert!(body.contains("In Progress"));
    }

    #[tokio::test]
    async fn get_task_happy_path() {
        let (_, client) = client_with(vec![Ok(json!({
            "data": {
                "issue": {
                    "identifier": "NILES-7",
                    "title": "Seven",
                    "url": "https://linear.app/issue/NILES-7",
                    "state": { "name": "Done" },
                    "attachments": {
                        "nodes": [
                            {
                                "url": "https://github.com/MarkNygaard/niles/pull/123",
                                "title": "PR #123"
                            }
                        ]
                    }
                }
            }
        })
        .to_string())]);
        let task = client.get_task("NILES-7").await.unwrap();
        assert_eq!(task.identifier, "NILES-7");
        assert_eq!(task.state, "Done");
        assert_eq!(
            task.pr_url,
            Some("https://github.com/MarkNygaard/niles/pull/123".into())
        );
    }

    #[tokio::test]
    async fn get_task_not_found() {
        let (_, client) = client_with(vec![Ok(json!({ "data": { "issue": null } }).to_string())]);
        let err = client.get_task("NILES-999").await.unwrap_err();
        assert!(matches!(err, Error::Resolve { kind: "issue", .. }));
    }

    #[tokio::test]
    async fn resolve_team_not_found() {
        let (_, client) = client_with(vec![Ok(json!({
            "data": {
                "teams": { "nodes": [] },
                "issueLabels": { "nodes": [] }
            }
        })
        .to_string())]);
        let err = client.create_task("t", "d").await.unwrap_err();
        assert!(matches!(err, Error::Resolve { kind: "team", .. }));
    }

    #[tokio::test]
    async fn resolve_state_not_found() {
        let (_, client) = client_with(vec![Ok(json!({
            "data": {
                "teams": {
                    "nodes": [
                        {
                            "id": "t1",
                            "key": "NILES",
                            "name": "Niles Team",
                            "states": { "nodes": [] }
                        }
                    ]
                },
                "issueLabels": {
                    "nodes": [
                        { "id": "l1", "name": "AI Eligible" }
                    ]
                }
            }
        })
        .to_string())]);
        let err = client.create_task("t", "d").await.unwrap_err();
        assert!(matches!(err, Error::Resolve { kind: "state", .. }));
    }

    #[tokio::test]
    async fn resolve_label_not_found() {
        let (_, client) = client_with(vec![Ok(json!({
            "data": {
                "teams": {
                    "nodes": [
                        {
                            "id": "t1",
                            "key": "NILES",
                            "name": "Niles Team",
                            "states": {
                                "nodes": [
                                    { "id": "s1", "name": "Todo" }
                                ]
                            }
                        }
                    ]
                },
                "issueLabels": { "nodes": [] }
            }
        })
        .to_string())]);
        let err = client.create_task("t", "d").await.unwrap_err();
        assert!(matches!(err, Error::Resolve { kind: "label", .. }));
    }

    #[tokio::test]
    async fn graphql_error_propagates() {
        let (_, client) = client_with(vec![Ok(json!({
            "data": null,
            "errors": [
                { "message": "Something went wrong" }
            ]
        })
        .to_string())]);
        let err = client.get_task("NILES-1").await.unwrap_err();
        assert!(matches!(err, Error::Api { reason } if reason == "Something went wrong"));
    }

    #[tokio::test]
    async fn bad_status_propagates() {
        let (_, client) = client_with(vec![Err(Error::BadStatus {
            status: 401,
            body: "unauthorized".into(),
        })]);
        let err = client.get_task("NILES-1").await.unwrap_err();
        assert!(matches!(err, Error::BadStatus { status: 401, .. }));
    }

    #[tokio::test]
    async fn ids_resolved_once_and_cached() {
        let (mock, client) = client_with(vec![
            Ok(resolve_json()),
            Ok(json!({ "data": { "issues": { "nodes": [] } } }).to_string()),
            Ok(json!({ "data": { "issues": { "nodes": [] } } }).to_string()),
        ]);
        client.list_tasks(None).await.unwrap();
        client.list_tasks(None).await.unwrap();
        // One resolve + two list queries = 3 total calls.
        // Without caching it would be 4 (two resolves + two lists).
        assert_eq!(mock.call_count(), 3);
    }
}
