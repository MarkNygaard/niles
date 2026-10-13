//! Skill tools: mint, patch, delete, view — and, for the background
//! review and the tidy-up, retire and list.
//!
//! Two sets. In a conversation somebody is there: a skill made is one
//! they asked for, and one deleted is one they wanted gone. In the
//! background nobody is: what Niles makes is its own (and the curator
//! may put it away later), it must read a skill before it changes it,
//! and it retires rather than deletes.

use crate::error::{Error, Result};
use crate::registry::ToolRegistry;
use crate::tool::{Tool, ToolDescriptor};
use async_trait::async_trait;
use niles_skills::{Provenance, SkillStatus, SkillStore};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

/// The skills read in one background run. A change to a skill not read
/// in it is refused: a body edited from what the transcript suggested it
/// says, rather than what it says, loses whatever it got wrong.
pub type Viewed = Arc<Mutex<HashSet<String>>>;

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

fn read_first(viewed: &Option<Viewed>, name: &str) -> Result<()> {
    let Some(viewed) = viewed else {
        return Ok(());
    };
    if viewed
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(name)
    {
        return Ok(());
    }
    Err(Error::Skill(format!(
        "skill `{name}` has not been read in this run; call view_skill first, then change what it actually says"
    )))
}

// ---------- MintSkillTool ----------

pub struct MintSkillTool {
    store: Arc<SkillStore>,
    provenance: Provenance,
}

impl MintSkillTool {
    /// For a conversation: the skill is one somebody asked for, and the
    /// curator leaves it alone.
    pub fn new(store: Arc<SkillStore>) -> Self {
        Self {
            store,
            provenance: Provenance::UserCreated,
        }
    }

    /// For the background: Niles's own, and the curator's to put away.
    pub fn learned(store: Arc<SkillStore>) -> Self {
        Self {
            store,
            provenance: Provenance::AgentCreated,
        }
    }
}

#[async_trait]
impl Tool for MintSkillTool {
    fn descriptor(&self) -> ToolDescriptor {
        let when = match self.provenance {
            Provenance::UserCreated => {
                "Call this ONLY when the user explicitly asks niles to save / remember / learn \
                 a specific named routine."
            }
            Provenance::AgentCreated => {
                "Only for a way of doing something for this household that will come up again \
                 and is not already covered: patch the closest existing skill instead whenever \
                 one is close."
            }
        };
        ToolDescriptor {
            name: "mint_skill".into(),
            description: format!(
                "Create a new persistent skill. {when} The body is the how-to — steps and the \
                 household's particulars — not a transcript or a log of what happened. The \
                 description is at most 60 characters and starts with when to use it: \
                 \"Use when winding the house down at night\". Returns the skill name."
            ),
            parameters: json!({
                "type": "object",
                "required": ["name", "description", "version", "body"],
                "properties": {
                    "name": { "type": "string", "description": "Unique kebab-case skill name." },
                    "description": { "type": "string", "description": "\"Use when ...\", at most 60 characters." },
                    "version": { "type": "string", "description": "Semantic version (e.g. '0.1.0')." },
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
            self.provenance.clone(),
        ))?;
        Ok(json!({"name": name}))
    }
}

// ---------- PatchSkillTool ----------

pub struct PatchSkillTool {
    store: Arc<SkillStore>,
    viewed: Option<Viewed>,
}

impl PatchSkillTool {
    pub fn new(store: Arc<SkillStore>) -> Self {
        Self {
            store,
            viewed: None,
        }
    }

    /// Refuses a skill not read with view_skill in this run.
    pub fn guarded(store: Arc<SkillStore>, viewed: Viewed) -> Self {
        Self {
            store,
            viewed: Some(viewed),
        }
    }
}

#[async_trait]
impl Tool for PatchSkillTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "patch_skill".into(),
            description: "Change an existing skill. Prefer old_text + new_text: replace the one \
                place old_text appears in the body, copied exactly from view_skill, and leave \
                the rest as it is. Only for a wholesale rewrite pass body instead, the complete \
                new how-to. Name, description and version are kept. Returns the skill name."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["name"],
                "properties": {
                    "name": { "type": "string", "description": "Name of the existing skill." },
                    "old_text": { "type": "string", "description": "Exact text in the body to replace; must appear once." },
                    "new_text": { "type": "string", "description": "What to put in its place." },
                    "body": { "type": "string", "description": "A complete new body. Last resort." }
                }
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let name = required_str("patch_skill", &args, "name")?;
        read_first(&self.viewed, name)?;
        let old = args.get("old_text").and_then(Value::as_str);
        let new = args.get("new_text").and_then(Value::as_str);
        let body = args.get("body").and_then(Value::as_str);
        match (old, new, body) {
            (Some(old), Some(new), _) => map_skill_err(self.store.edit(name, old, new))?,
            (None, None, Some(body)) => map_skill_err(self.store.patch(name, body))?,
            _ => {
                return Err(Error::InvalidArgs {
                    tool: "patch_skill".into(),
                    reason: "pass old_text and new_text, or body on its own".into(),
                });
            }
        }
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
            description: "Delete a skill the user wants gone. If its content has been merged \
                into another skill, pass `absorbed_into` to record where future references \
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

// ---------- RetireSkillTool ----------

pub struct RetireSkillTool {
    store: Arc<SkillStore>,
    viewed: Viewed,
}

impl RetireSkillTool {
    pub fn new(store: Arc<SkillStore>, viewed: Viewed) -> Self {
        Self { store, viewed }
    }
}

#[async_trait]
impl Tool for RetireSkillTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "retire_skill".into(),
            description: "Put a skill away because another skill now holds what it said. It is \
                archived, not deleted, and the household can restore it. absorbed_into must be \
                an existing skill — patch that one first so nothing is lost. Pinned skills \
                cannot be retired. Read both with view_skill first."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["name", "absorbed_into"],
                "properties": {
                    "name": { "type": "string", "description": "The skill to retire." },
                    "absorbed_into": { "type": "string", "description": "The existing skill that now covers it." }
                }
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let name = required_str("retire_skill", &args, "name")?;
        let into = required_str("retire_skill", &args, "absorbed_into")?;
        let viewed = Some(self.viewed.clone());
        read_first(&viewed, name)?;
        read_first(&viewed, into)?;
        map_skill_err(self.store.retire(name, into))?;
        Ok(json!({"name": name, "retired_into": into}))
    }
}

// ---------- ViewSkillTool ----------

pub struct ViewSkillTool {
    store: Arc<SkillStore>,
    viewed: Option<Viewed>,
}

impl ViewSkillTool {
    pub fn new(store: Arc<SkillStore>) -> Self {
        Self {
            store,
            viewed: None,
        }
    }

    /// Notes each skill read, for the guarded tools of the same run. A
    /// read by the background is not the household using the skill, so
    /// it is not counted as a view.
    pub fn noting(store: Arc<SkillStore>, viewed: Viewed) -> Self {
        Self {
            store,
            viewed: Some(viewed),
        }
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
        match &self.viewed {
            Some(viewed) => {
                viewed
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(name.to_string());
            }
            None => {
                if let Err(e) = self.store.bump_view(name) {
                    tracing::warn!(skill = name, error = %e, "failed to bump skill view counter");
                }
            }
        }
        Ok(json!({
            "name": skill.name,
            "description": skill.description,
            "version": skill.version,
            "body": skill.body,
            "status": skill.sidecar.status,
            "pinned": skill.sidecar.pinned,
        }))
    }
}

// ---------- ListSkillsTool ----------

pub struct ListSkillsTool {
    store: Arc<SkillStore>,
}

impl ListSkillsTool {
    pub fn new(store: Arc<SkillStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for ListSkillsTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "list_skills".into(),
            description: "Every skill, archived ones too: name, description, who made it, \
                whether pinned, status, and how much it has been used. No bodies — view_skill \
                for those."
                .into(),
            parameters: json!({ "type": "object", "properties": {} }),
        }
    }

    async fn execute(&self, _args: Value) -> Result<Value> {
        let skills: Vec<Value> = self
            .store
            .all()
            .into_iter()
            .map(|s| {
                json!({
                    "name": s.name,
                    "description": s.description,
                    "made_by": match s.sidecar.provenance {
                        Provenance::UserCreated => "asked for by the household",
                        Provenance::AgentCreated => "learned by Niles",
                    },
                    "pinned": s.sidecar.pinned,
                    "status": s.sidecar.status,
                    "archived": s.sidecar.status == SkillStatus::Archived,
                    "used": s.sidecar.usage_count,
                    "viewed": s.sidecar.view_count,
                    "last_activity": s.sidecar.latest_activity_at(),
                    "created": s.sidecar.created_at,
                })
            })
            .collect();
        Ok(json!({ "skills": skills }))
    }
}

/// Register the four skill tools for a conversation.
pub fn register_skill_tools(reg: &mut ToolRegistry, store: Arc<SkillStore>) {
    reg.register(Box::new(MintSkillTool::new(store.clone())));
    reg.register(Box::new(PatchSkillTool::new(store.clone())));
    reg.register(Box::new(DeleteSkillTool::new(store.clone())));
    reg.register(Box::new(ViewSkillTool::new(store)));
}

/// The skill tools for one background run — the review, or the tidy-up:
/// what it makes is Niles's own, it reads before it changes, and it
/// retires rather than deletes.
pub fn register_background_skill_tools(reg: &mut ToolRegistry, store: Arc<SkillStore>) {
    let viewed = Viewed::default();
    reg.register(Box::new(MintSkillTool::learned(store.clone())));
    reg.register(Box::new(PatchSkillTool::guarded(
        store.clone(),
        viewed.clone(),
    )));
    reg.register(Box::new(RetireSkillTool::new(
        store.clone(),
        viewed.clone(),
    )));
    reg.register(Box::new(ListSkillsTool::new(store.clone())));
    reg.register(Box::new(ViewSkillTool::noting(store, viewed)));
}

// ------------------------------------------------------------------
// Tests
// ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn store() -> Arc<SkillStore> {
        Arc::new(SkillStore::new(100_000))
    }

    #[tokio::test]
    async fn mint_then_view_round_trips() {
        let s = store();
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
        let s = store();
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
        let s = store();
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
        let s = store();
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
        let s = store();
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
    async fn delete_with_absorbed_into_says_where_it_went() {
        let s = store();
        let mint = MintSkillTool::new(s.clone());
        let delete = DeleteSkillTool::new(s.clone());

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

        let err = ViewSkillTool::new(s)
            .execute(json!({"name": "my-skill"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("folded into `other`"), "{err}");
    }

    fn evening(s: &Arc<SkillStore>) {
        s.create(
            "evening",
            "Use when winding the house down",
            "0.1.0",
            "Dim the living room to 30%. Turn off the kitchen.",
            Provenance::AgentCreated,
        )
        .unwrap();
    }

    #[tokio::test]
    async fn patch_replaces_the_text_named_and_keeps_the_rest() {
        let s = store();
        evening(&s);
        PatchSkillTool::new(s.clone())
            .execute(json!({"name": "evening", "old_text": "30%", "new_text": "20%"}))
            .await
            .unwrap();
        assert_eq!(
            s.load("evening").unwrap().body,
            "Dim the living room to 20%. Turn off the kitchen."
        );
        let err = PatchSkillTool::new(s.clone())
            .execute(json!({"name": "evening", "old_text": "30%"}))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::InvalidArgs { .. }), "{err}");
    }

    #[tokio::test]
    async fn the_background_reads_a_skill_before_changing_it() {
        let s = store();
        evening(&s);
        let mut reg = ToolRegistry::new();
        register_background_skill_tools(&mut reg, s.clone());
        let call = |name: &str, args: Value| niles_llm::ToolCall {
            id: "1".into(),
            name: name.into(),
            arguments: args,
        };
        let edit = call(
            "patch_skill",
            json!({"name": "evening", "old_text": "30%", "new_text": "20%"}),
        );
        let err = reg.execute(&edit).await.unwrap_err();
        assert!(err.to_string().contains("view_skill first"), "{err}");

        reg.execute(&call("view_skill", json!({"name": "evening"})))
            .await
            .unwrap();
        reg.execute(&edit).await.unwrap();
        assert!(s.load("evening").unwrap().body.contains("20%"));
        // Read by the background, not used by the household.
        assert_eq!(s.load("evening").unwrap().sidecar.view_count, 0);
    }

    #[tokio::test]
    async fn the_background_retires_rather_than_deletes_and_its_skills_are_its_own() {
        let s = store();
        evening(&s);
        let mut reg = ToolRegistry::new();
        register_background_skill_tools(&mut reg, s.clone());
        let names: HashSet<String> = reg.llm_tools().into_iter().map(|t| t.name).collect();
        assert!(!names.contains("delete_skill"));
        let call = |name: &str, args: Value| niles_llm::ToolCall {
            id: "1".into(),
            name: name.into(),
            arguments: args,
        };
        reg.execute(&call(
            "mint_skill",
            json!({"name": "night", "description": "Use when going to bed", "version": "0.1.0", "body": "All off."}),
        ))
        .await
        .unwrap();
        assert_eq!(
            s.load("night").unwrap().sidecar.provenance,
            Provenance::AgentCreated
        );
        for name in ["night", "evening"] {
            reg.execute(&call("view_skill", json!({"name": name})))
                .await
                .unwrap();
        }
        reg.execute(&call(
            "retire_skill",
            json!({"name": "night", "absorbed_into": "evening"}),
        ))
        .await
        .unwrap();
        assert_eq!(
            s.load("night").unwrap().sidecar.status,
            SkillStatus::Archived
        );
        let listed = reg.execute(&call("list_skills", json!({}))).await.unwrap();
        assert_eq!(listed["skills"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn view_bumps_view_counter() {
        let s = store();
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
        let s = store();
        let view = ViewSkillTool::new(s);
        let err = view.execute(json!({"name": "nope"})).await.unwrap_err();
        assert!(matches!(err, Error::Skill(_)));
    }

    #[tokio::test]
    async fn patch_missing_skill_errors() {
        let s = store();
        let patch = PatchSkillTool::new(s);
        let err = patch
            .execute(json!({"name": "nope", "body": "New"}))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Skill(_)));
    }

    #[tokio::test]
    async fn delete_missing_skill_errors() {
        let s = store();
        let delete = DeleteSkillTool::new(s);
        let err = delete.execute(json!({"name": "nope"})).await.unwrap_err();
        assert!(matches!(err, Error::Skill(_)));
    }

    #[tokio::test]
    async fn delete_pinned_skill_errors() {
        let s = store();
        let mint = MintSkillTool::new(s.clone());
        let delete = DeleteSkillTool::new(s.clone());

        mint.execute(json!({
            "name": "pinned-skill",
            "description": "D",
            "version": "0.1.0",
            "body": "B"
        }))
        .await
        .unwrap();

        s.set_pinned("pinned-skill", true).unwrap();

        let err = delete
            .execute(json!({"name": "pinned-skill"}))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Skill(_)));
    }

    #[tokio::test]
    async fn mint_existing_skill_errors() {
        let s = store();
        let mint = MintSkillTool::new(s);

        mint.execute(json!({
            "name": "dup",
            "description": "D",
            "version": "0.1.0",
            "body": "B"
        }))
        .await
        .unwrap();

        let err = mint
            .execute(json!({
                "name": "dup",
                "description": "D2",
                "version": "0.2.0",
                "body": "B2"
            }))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Skill(_)));
    }
}
