//! Linear data models.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct TaskRef {
    pub id: String,
    pub identifier: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskSummary {
    pub identifier: String,
    pub title: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskDetail {
    pub identifier: String,
    pub title: String,
    pub state: String,
    pub url: String,
    pub pr_url: Option<String>,
}
