//! Archon HTTP client.

use crate::error::{Error, Result};
use crate::model::{CancelOutcome, RunDetail, RunSummary, TriggerOutcome, WorkflowSummary};
use crate::transport::{ArchonTransport, HttpTransport};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

/// Client configuration.
#[derive(Debug, Clone)]
pub struct ArchonConfig {
    pub base_url: String,
    pub codebase_id: String,
    pub cwd: Option<String>,
    pub request_timeout: Duration,
}

/// Archon workflow-engine client.
pub struct ArchonClient {
    transport: Arc<dyn ArchonTransport>,
    cfg: ArchonConfig,
}

impl ArchonClient {
    /// Create a client using the default [`HttpTransport`].
    pub fn new(cfg: ArchonConfig) -> Result<Self> {
        let transport = Arc::new(HttpTransport::new(
            concat!(
                "niles/",
                env!("CARGO_PKG_VERSION"),
                " (https://github.com/MarkNygaard/niles)"
            ),
            cfg.request_timeout,
        ));
        Ok(Self::with_transport(transport, cfg))
    }

    /// Create a client with a custom transport (useful for testing).
    pub fn with_transport(transport: Arc<dyn ArchonTransport>, cfg: ArchonConfig) -> Self {
        Self { transport, cfg }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.cfg.base_url.trim_end_matches('/'), path)
    }

    /// List available workflows.
    pub async fn list_workflows(&self) -> Result<Vec<WorkflowSummary>> {
        let mut url = self.url("/api/workflows");
        if let Some(cwd) = &self.cfg.cwd {
            url = format!("{}?cwd={}", url, urlencoding::encode(cwd));
        }
        let body = self.transport.get(&url).await?;
        let parsed: ListResp = serde_json::from_str(&body).map_err(|e| Error::Parse {
            reason: e.to_string(),
        })?;

        Ok(parsed
            .workflows
            .into_iter()
            .map(|entry| WorkflowSummary {
                name: entry.workflow.name,
                description: entry.workflow.description,
                source: entry.source,
            })
            .collect())
    }

    /// Trigger a workflow run.
    ///
    /// This is a **two-step** operation:
    /// 1. Create a conversation (POST `/api/conversations`).
    /// 2. Trigger the workflow (POST `/api/workflows/{name}/run`).
    ///
    /// The returned `accepted: true` only means Archon *accepted* the
    /// request. It does **not** guarantee the run was actually scheduled
    /// or completed. Always follow up with [`Self::list_workflow_runs`]
    /// to verify the run appears in the active list.
    pub async fn run_workflow(&self, name: &str, message: &str) -> Result<TriggerOutcome> {
        let conv_url = self.url("/api/conversations");
        let conv_body = json!({ "codebaseId": self.cfg.codebase_id }).to_string();
        let conv_resp = self.transport.post(&conv_url, &conv_body).await?;
        let conv: ConvCreatedResp = serde_json::from_str(&conv_resp).map_err(|e| Error::Parse {
            reason: e.to_string(),
        })?;
        let conversation_id = conv.conversation_id;

        let run_url = self.url(&format!("/api/workflows/{}/run", encode_path(name)));
        let run_body = json!({ "conversationId": conversation_id, "message": message }).to_string();
        let run_resp = self.transport.post(&run_url, &run_body).await?;
        let run: RunAcceptedResp = serde_json::from_str(&run_resp).map_err(|e| Error::Parse {
            reason: e.to_string(),
        })?;

        Ok(TriggerOutcome {
            accepted: run.accepted,
            status: run.status,
            conversation_id,
        })
    }

    /// List recent workflow runs.
    pub async fn list_workflow_runs(&self, limit: u64) -> Result<Vec<RunSummary>> {
        let url = self.url(&format!("/api/workflows/runs?limit={}", limit));
        let body = self.transport.get(&url).await?;
        let parsed: RunsListResp = serde_json::from_str(&body).map_err(|e| Error::Parse {
            reason: e.to_string(),
        })?;

        Ok(parsed
            .runs
            .into_iter()
            .map(|run| RunSummary {
                id: run.id,
                status: run.status,
                workflow_name: run.workflow_name,
                message_preview: run.user_message.map(|m| truncate_chars(&m, 120)),
            })
            .collect())
    }

    /// Get the full status of a single workflow run.
    pub async fn get_workflow_run(&self, run_id: &str) -> Result<RunDetail> {
        let url = self.url(&format!("/api/workflows/runs/{}", encode_path(run_id)));
        let body = self.transport.get(&url).await.map_err(|e| match e {
            Error::BadStatus { status: 404, .. } => Error::RunNotFound { id: run_id.into() },
            other => other,
        })?;
        let parsed: RunDetailResp = serde_json::from_str(&body).map_err(|e| Error::Parse {
            reason: e.to_string(),
        })?;

        Ok(RunDetail {
            id: parsed.run.id,
            status: parsed.run.status,
            workflow_name: parsed.run.workflow_name,
            message_preview: parsed.run.user_message.map(|m| truncate_chars(&m, 120)),
            steps: parsed.events,
        })
    }

    /// Cancel a running workflow.
    pub async fn cancel_workflow_run(&self, run_id: &str) -> Result<CancelOutcome> {
        let url = self.url(&format!(
            "/api/workflows/runs/{}/cancel",
            encode_path(run_id)
        ));
        let body = self.transport.post(&url, "{}").await.map_err(|e| match e {
            Error::BadStatus { status: 404, .. } => Error::RunNotFound { id: run_id.into() },
            other => other,
        })?;
        let parsed: CancelResp = serde_json::from_str(&body).map_err(|e| Error::Parse {
            reason: e.to_string(),
        })?;

        Ok(CancelOutcome {
            success: parsed.success,
            message: parsed.message,
        })
    }
}

fn encode_path(s: &str) -> String {
    urlencoding::encode(s).replace('+', "%20")
}

fn truncate_chars(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.chars().count() <= max {
        s.to_string()
    } else {
        match s.char_indices().nth(max) {
            Some((idx, _)) => format!("{}…", &s[..idx]),
            None => s.to_string(),
        }
    }
}

#[derive(Deserialize)]
struct ListResp {
    workflows: Vec<WorkflowEntry>,
}

#[derive(Deserialize)]
struct WorkflowEntry {
    workflow: WorkflowInner,
    source: String,
}

#[derive(Deserialize)]
struct WorkflowInner {
    name: String,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Deserialize)]
struct ConvCreatedResp {
    #[serde(rename = "conversationId")]
    conversation_id: String,
}

#[derive(Deserialize)]
struct RunAcceptedResp {
    #[serde(default)]
    accepted: bool,
    #[serde(default)]
    status: Option<String>,
}

#[derive(Deserialize)]
struct RunsListResp {
    runs: Vec<RunWire>,
}

#[derive(Deserialize)]
struct RunWire {
    id: String,
    status: String,
    #[serde(default, alias = "workflowName")]
    workflow_name: Option<String>,
    #[serde(default, alias = "userMessage")]
    user_message: Option<String>,
}

#[derive(Deserialize)]
struct RunDetailResp {
    run: RunWire,
    #[serde(default)]
    events: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct CancelResp {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    message: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    #[derive(Clone)]
    struct MockTransport {
        last_url: Arc<Mutex<Option<String>>>,
        last_post_body: Arc<Mutex<Option<String>>>,
        responses: Arc<Mutex<Vec<Result<String>>>>,
    }

    impl MockTransport {
        fn new(responses: Vec<Result<String>>) -> Self {
            Self {
                last_url: Arc::new(Mutex::new(None)),
                last_post_body: Arc::new(Mutex::new(None)),
                responses: Arc::new(Mutex::new(responses)),
            }
        }

        fn last_url(&self) -> Option<String> {
            self.last_url.lock().unwrap().clone()
        }

        fn last_post_body(&self) -> Option<String> {
            self.last_post_body.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ArchonTransport for MockTransport {
        async fn get(&self, url: &str) -> Result<String> {
            *self.last_url.lock().unwrap() = Some(url.to_string());
            self.responses.lock().unwrap().remove(0)
        }

        async fn post(&self, url: &str, body: &str) -> Result<String> {
            *self.last_url.lock().unwrap() = Some(url.to_string());
            *self.last_post_body.lock().unwrap() = Some(body.to_string());
            self.responses.lock().unwrap().remove(0)
        }
    }

    fn client_with(responses: Vec<Result<String>>) -> (MockTransport, ArchonClient) {
        let mock = MockTransport::new(responses);
        let cfg = ArchonConfig {
            base_url: "https://archon.example.com".into(),
            codebase_id: "fb05cf4e-07da-41cc-bba9-6d770107a7cb".into(),
            cwd: None,
            request_timeout: Duration::from_secs(10),
        };
        let client = ArchonClient::with_transport(Arc::new(mock.clone()), cfg);
        (mock, client)
    }

    fn client_with_cwd(responses: Vec<Result<String>>) -> (MockTransport, ArchonClient) {
        let mock = MockTransport::new(responses);
        let cfg = ArchonConfig {
            base_url: "https://archon.example.com".into(),
            codebase_id: "fb05cf4e-07da-41cc-bba9-6d770107a7cb".into(),
            cwd: Some("/tmp/project".into()),
            request_timeout: Duration::from_secs(10),
        };
        let client = ArchonClient::with_transport(Arc::new(mock.clone()), cfg);
        (mock, client)
    }

    #[tokio::test]
    async fn list_workflows_parses_envelope() {
        let (_, client) = client_with(vec![Ok(r#"{"workflows":[{"workflow":{"name":"deploy","description":"Deploy to prod"},"source":"bundled"}]}"#.into())]);
        let workflows = client.list_workflows().await.unwrap();
        assert_eq!(workflows.len(), 1);
        assert_eq!(workflows[0].name, "deploy");
        assert_eq!(workflows[0].description, Some("Deploy to prod".into()));
        assert_eq!(workflows[0].source, "bundled");
    }

    #[tokio::test]
    async fn list_workflows_appends_cwd_when_set() {
        let (mock, client) = client_with_cwd(vec![Ok(r#"{"workflows":[]}"#.into())]);
        client.list_workflows().await.unwrap();
        let url = mock.last_url().unwrap();
        assert!(url.contains("?cwd="));
        assert!(url.contains("%2Ftmp%2Fproject"));
    }

    #[tokio::test]
    async fn list_workflows_omits_cwd_when_none() {
        let (mock, client) = client_with(vec![Ok(r#"{"workflows":[]}"#.into())]);
        client.list_workflows().await.unwrap();
        let url = mock.last_url().unwrap();
        assert!(!url.contains("?cwd="));
    }

    #[tokio::test]
    async fn run_workflow_two_step_ordering() {
        let (mock, client) = client_with(vec![
            Ok(r#"{"conversationId":"conv-123"}"#.into()),
            Ok(r#"{"accepted":true,"status":"started"}"#.into()),
        ]);
        let outcome = client.run_workflow("deploy", "deploy now").await.unwrap();
        assert_eq!(outcome.conversation_id, "conv-123");
        assert!(outcome.accepted);

        let post_body = mock.last_post_body().unwrap();
        assert!(post_body.contains("conv-123"));
        assert!(post_body.contains("deploy now"));
    }

    #[tokio::test]
    async fn run_workflow_accepted_with_status() {
        let (_, client) = client_with(vec![
            Ok(r#"{"conversationId":"c1"}"#.into()),
            Ok(r#"{"accepted":true,"status":"queued"}"#.into()),
        ]);
        let outcome = client.run_workflow("test", "go").await.unwrap();
        assert!(outcome.accepted);
        assert_eq!(outcome.status, Some("queued".into()));
    }

    #[tokio::test]
    async fn run_workflow_accepted_false() {
        let (_, client) = client_with(vec![
            Ok(r#"{"conversationId":"c1"}"#.into()),
            Ok(r#"{"accepted":false}"#.into()),
        ]);
        let outcome = client.run_workflow("test", "go").await.unwrap();
        assert!(!outcome.accepted);
        assert_eq!(outcome.status, None);
    }

    #[tokio::test]
    async fn list_workflow_runs_truncates_preview() {
        let long = "a".repeat(200);
        let json = format!(
            r#"{{"runs":[{{"id":"r1","status":"running","workflow_name":"deploy","user_message":"{}"}}]}}"#,
            long
        );
        let (_, client) = client_with(vec![Ok(json)]);
        let runs = client.list_workflow_runs(5).await.unwrap();
        assert_eq!(
            runs[0].message_preview.as_ref().unwrap().chars().count(),
            121
        ); // 120 chars + '…'
    }

    #[tokio::test]
    async fn list_workflow_runs_handles_empty() {
        let (_, client) = client_with(vec![Ok(r#"{"runs": []}"#.into())]);
        let runs = client.list_workflow_runs(5).await.unwrap();
        assert!(runs.is_empty());
    }

    #[tokio::test]
    async fn list_workflow_runs_multibyte_truncation() {
        let msg = "🎉".repeat(100);
        let json = format!(
            r#"{{"runs":[{{"id":"r1","status":"done","user_message":"{}"}}]}}"#,
            msg
        );
        let (_, client) = client_with(vec![Ok(json)]);
        let runs = client.list_workflow_runs(5).await.unwrap();
        let preview = runs[0].message_preview.as_ref().unwrap();
        assert!(preview.chars().count() <= 121);
    }

    #[tokio::test]
    async fn list_workflow_runs_accepts_camel_case_fields() {
        let (_, client) = client_with(vec![Ok(
            r#"{"runs":[{"id":"r1","status":"done","workflowName":"deploy","userMessage":"hi"}]}"#
                .into(),
        )]);
        let runs = client.list_workflow_runs(5).await.unwrap();
        assert_eq!(runs[0].workflow_name, Some("deploy".into()));
        assert_eq!(runs[0].message_preview, Some("hi".into()));
    }

    #[tokio::test]
    async fn get_workflow_run_parses_run_envelope() {
        let (_, client) = client_with(vec![Ok(r#"{"run":{"id":"r1","status":"done","workflow_name":"deploy","user_message":"hi"},"events":[{"step":"build"}]}"#.into())]);
        let run = client.get_workflow_run("r1").await.unwrap();
        assert_eq!(run.id, "r1");
        assert_eq!(run.status, "done");
        assert_eq!(run.workflow_name, Some("deploy".into()));
        assert_eq!(run.steps.len(), 1);
    }

    #[tokio::test]
    async fn get_workflow_run_accepts_camel_case_fields() {
        let (_, client) = client_with(vec![Ok(r#"{"run":{"id":"r1","status":"done","workflowName":"deploy","userMessage":"hi"},"events":[]}"#.into())]);
        let run = client.get_workflow_run("r1").await.unwrap();
        assert_eq!(run.workflow_name, Some("deploy".into()));
        assert_eq!(run.message_preview, Some("hi".into()));
    }

    #[tokio::test]
    async fn get_workflow_run_404_maps_to_run_not_found() {
        let (_, client) = client_with(vec![Err(Error::BadStatus {
            status: 404,
            body: "not found".into(),
        })]);
        let err = client.get_workflow_run("r-missing").await.unwrap_err();
        assert!(matches!(err, Error::RunNotFound { id } if id == "r-missing"));
    }

    #[tokio::test]
    async fn cancel_workflow_run_parses_success_message() {
        let (_, client) = client_with(vec![Ok(r#"{"success":true,"message":"cancelled"}"#.into())]);
        let outcome = client.cancel_workflow_run("r1").await.unwrap();
        assert!(outcome.success);
        assert_eq!(outcome.message, Some("cancelled".into()));
    }

    #[tokio::test]
    async fn cancel_workflow_run_404_maps_to_run_not_found() {
        let (_, client) = client_with(vec![Err(Error::BadStatus {
            status: 404,
            body: "not found".into(),
        })]);
        let err = client.cancel_workflow_run("r-missing").await.unwrap_err();
        assert!(matches!(err, Error::RunNotFound { id } if id == "r-missing"));
    }

    #[tokio::test]
    async fn bad_status_propagates() {
        let (_, client) = client_with(vec![Err(Error::BadStatus {
            status: 500,
            body: "oops".into(),
        })]);
        let err = client.list_workflows().await.unwrap_err();
        assert!(matches!(err, Error::BadStatus { status: 500, body } if body == "oops"));
    }

    #[tokio::test]
    async fn junk_json_returns_parse_error() {
        let (_, client) = client_with(vec![Ok("not json".into())]);
        let err = client.list_workflows().await.unwrap_err();
        assert!(matches!(err, Error::Parse { .. }));
    }

    #[tokio::test]
    async fn base_url_with_trailing_slash() {
        let mock = MockTransport::new(vec![Ok(r#"{"workflows":[]}"#.into())]);
        let cfg = ArchonConfig {
            base_url: "https://archon.example.com/".into(),
            codebase_id: "x".into(),
            cwd: None,
            request_timeout: Duration::from_secs(10),
        };
        let client = ArchonClient::with_transport(Arc::new(mock.clone()), cfg);
        client.list_workflows().await.unwrap();
        let url = mock.last_url().unwrap();
        assert_eq!(url, "https://archon.example.com/api/workflows");
    }

    #[tokio::test]
    async fn run_workflow_encodes_name_in_path() {
        let (mock, client) = client_with(vec![
            Ok(r#"{"conversationId":"c1"}"#.into()),
            Ok(r#"{"accepted":true}"#.into()),
        ]);
        client.run_workflow("deploy/prod", "go").await.unwrap();
        let url = mock.last_url().unwrap();
        assert!(url.contains("/api/workflows/deploy%2Fprod/run"));
        assert!(!url.contains('+'));
    }

    #[tokio::test]
    async fn get_workflow_run_encodes_run_id_in_path() {
        let (mock, client) = client_with(vec![Ok(
            r#"{"run":{"id":"r/1","status":"done"},"events":[]}"#.into(),
        )]);
        client.get_workflow_run("r/1").await.unwrap();
        let url = mock.last_url().unwrap();
        assert!(url.contains("/api/workflows/runs/r%2F1"));
        assert!(!url.contains('+'));
    }

    #[tokio::test]
    async fn cancel_workflow_run_encodes_run_id_in_path() {
        let (mock, client) = client_with(vec![Ok(r#"{"success":true}"#.into())]);
        client.cancel_workflow_run("r/1").await.unwrap();
        let url = mock.last_url().unwrap();
        assert!(url.contains("/api/workflows/runs/r%2F1/cancel"));
        assert!(!url.contains('+'));
    }

    #[test]
    fn truncate_chars_zero_max_returns_empty() {
        assert_eq!(truncate_chars("hello", 0), "");
    }
}
