//! The tools that change part of an artifact or its page, for a change
//! that writing everything out again would make too slow for a chat
//! answer.

use std::sync::Arc;

use async_trait::async_trait;
use niles_artifacts::{Operation, PageChange, kinds, page};
use niles_tools::{Result, Tool, ToolDescriptor};
use serde_json::{Value, json};

use super::{Artifacts, Update, asker, author, id_arg, invalid, refused, str_arg, view};

/// artifact_update, with the content worked out from edits.
pub(super) struct Edit(pub(super) Update);

#[async_trait]
impl Tool for Edit {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "artifact_edit".into(),
            description: "Change part of one of the asking person's artifacts, or of one \
                shared with them to edit, without writing the rest out again: a ticked item, \
                a time, a day added, a paragraph reworded. Read it with artifact_get first. \
                A document is changed by its text: each edit is {old, new}, old copied \
                exactly from its markdown and found there exactly once (take in more of the \
                text around it when it is not), new what replaces it. A trip or a training \
                plan is changed by place: each edit is {op, path, value}, path a JSON \
                pointer into its content such as /days/2/items/0/time; set puts value there \
                (a field, or an entry of a list), insert puts value into a list before the \
                entry path names (/packing/- adds it at the end), remove takes that field \
                or entry away. The edits are made in order, each on what the ones before \
                left, and all or none; what results is checked as a whole new version is, \
                and saved as one, so it can be undone. reason is for the list of versions \
                (\"Added sunscreen\")."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "integer" },
                    "edits": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "old": { "type": "string", "description": "A document's: the exact text to replace" },
                                "new": { "type": "string", "description": "A document's: what replaces it" },
                                "op": { "type": "string", "enum": ["set", "insert", "remove"], "description": "A trip's or a training plan's" },
                                "path": { "type": "string", "description": "A JSON pointer: /days/2/items/0/time" },
                                "value": { "description": "What set or insert puts there" }
                            }
                        }
                    },
                    "summary": { "type": "string" },
                    "reason": { "type": "string" }
                },
                "required": ["id", "edits", "reason"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        const TOOL: &str = "artifact_edit";
        let asker = asker(TOOL)?;
        let id = id_arg(TOOL, &args)?;
        self.0.may_write(TOOL, id)?;
        let opened = self
            .0
            .artifacts
            .opened(TOOL, &asker.person, id, Operation::Edit)
            .await?;
        self.0.may_change(TOOL, &opened.artifact)?;
        let store = &self.0.artifacts.store;
        let (kind, owner, at) = {
            let a = &opened.artifact;
            (
                a.kind.clone(),
                opened.owner().to_string(),
                a.current_revision,
            )
        };
        let current = match at {
            0 => None,
            n => store
                .revision(&owner, id, n)
                .await
                .map_err(|e| refused(TOOL, e))?,
        }
        .ok_or_else(|| {
            invalid(
                TOOL,
                "nothing is written in it yet to change: save the whole content with \
                 artifact_update",
            )
        })?;
        let edited = niles_artifacts::edit::content(
            &kind,
            &current.content,
            args.get("edits").unwrap_or(&Value::Null),
        )
        .map_err(|e| refused(TOOL, e))?;
        let checked = kinds::validate(&kind, &edited).map_err(|e| refused(TOOL, e))?;
        if checked == current.content {
            return Err(invalid(
                TOOL,
                "these edits leave it as it is, so there is nothing to save",
            ));
        }
        self.0
            .save(TOOL, &asker, opened, &checked, &args, Some(current.n))
            .await
    }
}

pub(super) struct EditPage {
    pub(super) artifacts: Arc<Artifacts>,
    /// In a job, the one artifact whose page it may change.
    pub(super) only: Option<i64>,
}

#[async_trait]
impl Tool for EditPage {
    fn descriptor(&self) -> ToolDescriptor {
        let mut description = "Change the page of one of the asking person's artifacts, or of \
            one shared with them to edit, by exact replacements in its HTML: for a small \
            change, such as a colour, a heading or a label. Read it with artifact_get with \
            with_page: true first. Each edit is {old, new}: old copied exactly from \
            page_html and found there exactly once (take in more of the text around it when \
            it is not), new what replaces it. The edits are made in order, all or none; the \
            page that results is checked as a whole page is, and saved as a new version, so \
            it can be undone. reason is for the list of versions (\"Green headings\")."
            .to_string();
        description.push_str(if self.only.is_none() {
            " A new layout is start_page_job's."
        } else {
            " A new page is artifact_set_page's."
        });
        ToolDescriptor {
            name: "artifact_edit_page".into(),
            description,
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "integer" },
                    "edits": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "old": { "type": "string", "description": "The exact text to replace" },
                                "new": { "type": "string", "description": "What replaces it" }
                            },
                            "required": ["old", "new"]
                        }
                    },
                    "reason": { "type": "string" }
                },
                "required": ["id", "edits", "reason"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        const TOOL: &str = "artifact_edit_page";
        let asker = asker(TOOL)?;
        let id = id_arg(TOOL, &args)?;
        if self.only.is_some_and(|only| only != id) {
            return Err(invalid(
                TOOL,
                format!(
                    "this job makes the page of artifact {}, and only that one",
                    self.only.unwrap_or(id)
                ),
            ));
        }
        let opened = self
            .artifacts
            .opened(TOOL, &asker.person, id, Operation::Edit)
            .await?;
        if self.only.is_none()
            && self
                .artifacts
                .paging
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains(&id)
        {
            return Err(invalid(
                TOOL,
                "a new page is being made for it now, and would replace this change: ask \
                 again once that page is there",
            ));
        }
        let (page_n, owner) = (opened.artifact.page, opened.owner().to_string());
        let store = &self.artifacts.store;
        let page = match page_n {
            Some(n) => store
                .page(&owner, id, n)
                .await
                .map_err(|e| refused(TOOL, e))?,
            None => None,
        }
        .ok_or_else(|| {
            invalid(
                TOOL,
                if self.only.is_none() {
                    "it has no page to change; start_page_job makes one"
                } else {
                    "it has no page to change; artifact_set_page makes one"
                },
            )
        })?;
        let edited = niles_artifacts::edit::text(&page, args.get("edits").unwrap_or(&Value::Null))
            .map_err(|e| refused(TOOL, e))?;
        let html = page::page(&edited).map_err(|e| refused(TOOL, e))?;
        if html == page {
            return Err(invalid(
                TOOL,
                "these edits leave the page as it is, so there is nothing to save",
            ));
        }
        let reason =
            niles_artifacts::summary(str_arg(&args, "reason").unwrap_or("Changed the page"));
        let a = store
            .set_page(
                &owner,
                id,
                PageChange::Edit {
                    from: page_n.expect("read above"),
                    html,
                },
                reason,
                author(crate::assistant::via()),
                Some(asker.person.clone()),
            )
            .await
            .map_err(|e| refused(TOOL, e))?
            .ok_or_else(|| invalid(TOOL, format!("there is no artifact {id}")))?;
        if self.only.is_none() {
            self.artifacts.touch(&asker.person, a.id);
            self.artifacts.announce(&a);
        }
        Ok(view(&a))
    }
}
