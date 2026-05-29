//! Skill tools: mint, patch, delete, view.

use crate::error::{Error, Result};
use crate::registry::ToolRegistry;
use crate::tool::{Tool, ToolDescriptor};
use async_trait::async_trait;
use niles_skills::{Provenance, SkillStore};
use serde_json::{Value, json};
use std::sync::Arc;

fn required_str<'a>(tool: &'static str, args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::InvalidArgs {
            tool: tool.into(),
            reason: format!("missing required field '{key}'"),
        })
}

fn map_skill_err<T>(r: std::result::Result<T, niles_skills::Error>) -> Result<T> {
    r.map_err(|e| Error::Skill(e.to_string()))
}

// ---------- MintSkillTool ----------

pub struct MintSkillTool {
    store: Arc<SkillStore>,
}

impl MintSkillTool {
    pub fn new(store: Arc<SkillStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for MintSkillTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "mint_skill".into(),
            description: "Create a new persistent skill. Call this ONLY when the user \
                explicitly asks niles to save / remember / learn a specific named routine. \
                Prefer patching an existing skill (`patch_skill`) over creating a new one \
                when there's overlap. The body should be the how-to, not a transcript of \
                the conversation. Skills created via this tool belong to the user and are \
                never auto-archived. Returns the skill name on success."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["name", "description", "version", "body"],
                "properties": {
                    "name": { "type": "string", "description": "Unique kebab-case skill name." },
                    "description": { "type": "string", "description": "One-line description of what the skill does." },
                    "version": { "type": "string", "description": "Semantic version. Default '0.1.0'." },
                    "body": { "type": "string", "description": "The how-to content of the skill." }
                }
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let name = required_str("mint_skill", &args, "name")?;
        let description = required_str("mint_skill", &args, "description")?;
        let version = required_str("mint_skill", &args, "version")?;
        let body = required_str("mint_skill", &args, "body")?;
        map_skill_err(self.store.create(
            name,
            description,
            version,
            body,
            Provenance::UserCreated,
        ))?;
        Ok(json!({"name": name}))
    }
}

// ---------- PatchSkillTool ----------

pub struct PatchSkillTool {
    store: Arc<SkillStore>,
}

impl PatchSkillTool {
    pub fn new(store: Arc<SkillStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for PatchSkillTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "patch_skill".into(),
            description: "Replace the body of an existing skill. The frontmatter \
                (name + description + version) is preserved. Use this to refine a skill \
                in place rather than deleting and recreating it. Returns the skill name \
                on success."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["name", "body"],
                "properties": {
                    "name": { "type": "string", "description": "Name of the existing skill." },
                    "body": { "type": "string", "description": "New body content." }
                }
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let name = required_str("patch_skill", &args, "name")?;
        let body = required_str("patch_skill", &args, "body")?;
        map_skill_err(self.store.patch(name, body))?;
        Ok(json!({"name": name}))
    }
}

// ---------- DeleteSkillTool ----------

pub struct DeleteSkillTool {
    store: Arc<SkillStore>,
}

impl DeleteSkillTool {
    pub fn new(store: Arc<SkillStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for DeleteSkillTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "delete_skill".into(),
            description: "Delete a skill. If this skill's content has been merged into \
                another skill, pass `absorbed_into` to record where future references \
                should look. Pinned skills cannot be deleted."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["name"],
                "properties": {
                    "name": { "type": "string", "description": "Name of the skill to delete." },
                    "absorbed_into": { "type": "string", "description": "Name of the skill that absorbed this one's content." }
                }
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let name = required_str("delete_skill", &args, "name")?;
        let absorbed_into = args.get("absorbed_into").and_then(|v| v.as_str());
        map_skill_err(self.store.delete(name, absorbed_into))?;
        Ok(json!({"name": name, "deleted": true}))
    }
}

// ---------- ViewSkillTool ----------

pub struct ViewSkillTool {
    store: Arc<SkillStore>,
}

impl ViewSkillTool {
    pub fn new(store: Arc<SkillStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for ViewSkillTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "view_skill".into(),
            description: "Load the full body of a skill. Use when the system prompt's \
                Available skills list shows a skill that's relevant to the current request. \
                Bumps the skill's view counter."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["name"],
                "properties": {
                    "name": { "type": "string", "description": "Name of the skill to view." }
                }
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let name = required_str("view_skill", &args, "name")?;
        let skill = map_skill_err(self.store.load(name))?;
        if let Err(e) = self.store.bump_view(name) {
            tracing::warn!(skill = name, error = %e, "failed to bump skill view counter");
        }
        Ok(json!({
            "name": skill.name,
            "description": skill.description,
            "version": skill.version,
            "body": skill.body,
        }))
    }
}

/// Register the four skill tools onto an existing registry.
pub fn register_skill_tools(reg: &mut ToolRegistry, store: Arc<SkillStore>) {
    reg.register(Box::new(MintSkillTool::new(store.clone())));
    reg.register(Box::new(PatchSkillTool::new(store.clone())));
    reg.register(Box::new(DeleteSkillTool::new(store.clone())));
    reg.register(Box::new(ViewSkillTool::new(store)));
}

// ------------------------------------------------------------------
// Tests
// ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn store() -> (TempDir, Arc<SkillStore>) {
        let tmp = TempDir::new().unwrap();
        let s = SkillStore::open(tmp.path(), 100_000, 1_048_576).unwrap();
        (tmp, Arc::new(s))
    }

    #[tokio::test]
    async fn mint_then_view_round_trips() {
        let (_tmp, s) = store();
        let mint = MintSkillTool::new(s.clone());
        let view = ViewSkillTool::new(s.clone());

        mint.execute(json!({
            "name": "my-skill",
            "description": "D",
            "version": "0.1.0",
            "body": "# Body"
        }))
        .await
        .unwrap();

        let result = view.execute(json!({"name": "my-skill"})).await.unwrap();
        assert_eq!(result["body"], "# Body");
    }

    #[tokio::test]
    async fn mint_invalid_name_errors() {
        let (_tmp, s) = store();
        let mint = MintSkillTool::new(s);
        let err = mint
            .execute(json!({
                "name": "Foo",
                "description": "D",
                "version": "0.1.0",
                "body": "B"
            }))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Skill(_)));
    }

    #[tokio::test]
    async fn mint_missing_required_field_errors() {
        let (_tmp, s) = store();
        let mint = MintSkillTool::new(s);
        let err = mint
            .execute(json!({
                "name": "my-skill",
                "description": "D",
                "version": "0.1.0"
            }))
            .await
            .unwrap_err();
        assert!(
            matches!(err, Error::InvalidArgs { tool, reason } if tool == "mint_skill" && reason.contains("body"))
        );
    }

    #[tokio::test]
    async fn patch_updates_body_preserves_meta() {
        let (_tmp, s) = store();
        let mint = MintSkillTool::new(s.clone());
        let patch = PatchSkillTool::new(s.clone());
        let view = ViewSkillTool::new(s);

        mint.execute(json!({
            "name": "my-skill",
            "description": "Original",
            "version": "0.1.0",
            "body": "Old"
        }))
        .await
        .unwrap();

        patch
            .execute(json!({"name": "my-skill", "body": "New"}))
            .await
            .unwrap();

        let result = view.execute(json!({"name": "my-skill"})).await.unwrap();
        assert_eq!(result["body"], "New");
        assert_eq!(result["description"], "Original");
        assert_eq!(result["version"], "0.1.0");
    }

    #[tokio::test]
    async fn delete_removes_skill() {
        let (_tmp, s) = store();
        let mint = MintSkillTool::new(s.clone());
        let delete = DeleteSkillTool::new(s.clone());
        let view = ViewSkillTool::new(s);

        mint.execute(json!({
            "name": "my-skill",
            "description": "D",
            "version": "0.1.0",
            "body": "B"
        }))
        .await
        .unwrap();

        delete.execute(json!({"name": "my-skill"})).await.unwrap();

        let err = view.execute(json!({"name": "my-skill"})).await.unwrap_err();
        assert!(matches!(err, Error::Skill(_)));
    }

    #[tokio::test]
    async fn delete_with_absorbed_into_writes_graveyard() {
        let (tmp, s) = store();
        let mint = MintSkillTool::new(s.clone());
        let delete = DeleteSkillTool::new(s);

        mint.execute(json!({
            "name": "my-skill",
            "description": "D",
            "version": "0.1.0",
            "body": "B"
        }))
        .await
        .unwrap();

        delete
            .execute(json!({"name": "my-skill", "absorbed_into": "other"}))
            .await
            .unwrap();

        assert!(tmp.path().join(".absorbed").join("my-skill.json").exists());
    }

    #[tokio::test]
    async fn view_bumps_view_counter() {
        let (_tmp, s) = store();
        let mint = MintSkillTool::new(s.clone());
        let view = ViewSkillTool::new(s.clone());

        mint.execute(json!({
            "name": "my-skill",
            "description": "D",
            "version": "0.1.0",
            "body": "B"
        }))
        .await
        .unwrap();

        view.execute(json!({"name": "my-skill"})).await.unwrap();
        view.execute(json!({"name": "my-skill"})).await.unwrap();

        let skill = s.load("my-skill").unwrap();
        assert_eq!(skill.sidecar.view_count, 2);
    }

    #[tokio::test]
    async fn view_unknown_skill_errors() {
        let (_tmp, s) = store();
        let view = ViewSkillTool::new(s);
        let err = view.execute(json!({"name": "nope"})).await.unwrap_err();
        assert!(matches!(err, Error::Skill(_)));
    }
}
