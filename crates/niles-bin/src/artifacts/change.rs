//! A change to an artifact made in the background: one that rewrites
//! too much of it for a chat answer (`start_change_job`), or one a chat
//! turn ran out of time in the middle of ([`Artifacts::carry_on`]).
//! Built as a page's run is: the artifact is not marked working, its
//! data goes on being kept up, and the run is the asker's, counted
//! among their jobs.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use niles_artifacts::{Artifact, Author, Operation, Status, kinds, page};
use niles_tools::{Result, Tool, ToolDescriptor};
use serde_json::{Value, json};

use super::{
    Artifacts, Asker, Held, NO_ASSISTANT, PODCAST, Slot, asker, busy, id_arg, invalid, research,
    str_arg, view,
};
use crate::assistant::Searching;

pub(super) const STILL_WORKING: &str = "This is taking longer than a chat answer. I'm still \
    working on it and will let you know when it's done.";

pub(super) struct StartChange(pub(super) Arc<Artifacts>);

#[async_trait]
impl Tool for StartChange {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "start_change_job".into(),
            description: "Make a change to one of the asking person's artifacts, or one shared \
                with them to edit, in the background, when it rewrites much of it (a trip a \
                week longer, a plan rebuilt around a new race date, a document \
                reorganised). It returns at once; another run of you makes the change, and \
                the person is told when it is done. brief is the change in their own words, \
                with what it needs from this conversation, because that run will not see \
                it. Then answer in one sentence that you are making the change and will \
                tell them when it is done. A small change you make yourself with \
                artifact_edit; a new look is start_page_job's."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "integer" },
                    "brief": { "type": "string" }
                },
                "required": ["id", "brief"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        const TOOL: &str = "start_change_job";
        let asker = asker(TOOL)?;
        let id = id_arg(TOOL, &args)?;
        // Before opened, so a bad call reveals nothing about the id.
        let brief = str_arg(&args, "brief")
            .ok_or_else(|| invalid(TOOL, "brief is the change to make, and it is empty"))?
            .to_string();
        let opened = self
            .0
            .opened(TOOL, &asker.person, id, Operation::Edit)
            .await?;
        let a = self
            .0
            .start_change(&asker, opened.artifact, brief)
            .await
            .map_err(|why| invalid(TOOL, why))?;
        self.0.touch(&asker.person, a.id);
        let mut out = view(&a);
        out["note"] = json!(
            "The change is being made now. Tell the person in one sentence that you are \
             making it and will tell them when it is done."
        );
        Ok(out)
    }
}

impl Artifacts {
    /// Start a run making a change to an artifact `asker` may change
    /// that is there already, with `brief` saying what. Not marked
    /// working: like a page's run, so a restart leaves no "failed" on a
    /// whole artifact, and a "Try again" never re-runs its first job
    /// over it.
    async fn start_change(
        self: &Arc<Self>,
        asker: &Asker,
        artifact: Artifact,
        brief: String,
    ) -> std::result::Result<Artifact, String> {
        if self.runner.available().is_none() {
            return Err(NO_ASSISTANT.into());
        }
        if artifact.kind == PODCAST {
            return Err(
                "a podcast is kept as it was recorded; to change it, make a new episode with \
                 start_artifact_job"
                    .into(),
            );
        }
        if artifact.status == Status::Working || artifact.current_revision == 0 {
            return Err("it is still being written; it can be changed once it is there".into());
        }
        let held = Held::claim(&self.changing, artifact.id).ok_or_else(|| {
            "a change is being made to it already; ask for another once that one is done"
                .to_string()
        })?;
        let slot = self.take_slot(&asker.person).ok_or_else(busy)?;
        let this = self.clone();
        let made = artifact.clone();
        let asker = asker.clone();
        tokio::spawn(async move { this.run_change(made, brief, asker, slot, held).await });
        Ok(artifact)
    }

    async fn run_change(
        self: Arc<Self>,
        artifact: Artifact,
        brief: String,
        asker: Asker,
        _slot: Slot,
        _held: Held,
    ) {
        let (person, conversation) = (asker.person.clone(), asker.conversation);
        let owner = artifact.person.clone();
        let asker = Asker {
            satellite: None,
            ..asker
        };
        let (system, prompt) =
            change_prompts(&artifact, &brief, self.searching(self.runner.available()));
        let tools = self.job_tools(artifact.id, &artifact.kind, None);
        let outcome = tokio::time::timeout(
            super::JOB_LIMIT,
            self.runner.run(asker, &system, &prompt, tools),
        )
        .await
        .unwrap_or_else(|_| Err("it took longer than half an hour".into()));
        let now = match self.store.get(&owner, artifact.id).await {
            Ok(Some(now)) => now,
            // Deleted while the change was being made.
            Ok(None) => return,
            Err(e) => {
                tracing::warn!(
                    "[artifacts] the change to {} lost its artifact: {e}",
                    artifact.id
                );
                return;
            }
        };
        let conversation = conversation.or(now.conversation_id.filter(|_| person == owner));
        match outcome {
            Err(why) => {
                tracing::warn!(
                    "[artifacts] {person}: {:?} was not changed: {why}",
                    now.title
                );
                self.say(
                    &person,
                    &now,
                    conversation,
                    format!("The change to {} didn't get made: {why}", now.title),
                    None,
                )
                .await;
            }
            Ok((via, answer)) => {
                let answer = answer.trim();
                if self
                    .changed_by(&now, artifact.current_revision, &person)
                    .await
                {
                    tracing::info!("[artifacts] {person}: {:?} was changed", now.title);
                    self.announce(&now);
                    let reply = if answer.is_empty() {
                        format!("The change to {} is made.", now.title)
                    } else {
                        answer.to_string()
                    };
                    self.say(&person, &now, conversation, reply, Some(via))
                        .await;
                    if now.notify {
                        self.notifier.finished(&person, &now).await;
                    }
                } else {
                    tracing::info!("[artifacts] {person}: nothing changed in {:?}", now.title);
                    let reply = if answer.is_empty() {
                        format!("Nothing in {} needed changing.", now.title)
                    } else {
                        answer.to_string()
                    };
                    self.say(&person, &now, conversation, reply, Some(via))
                        .await;
                }
            }
        }
    }

    /// Whether a version after version `since` is `person`'s and not
    /// their own tap in the app: what their change's run saved.
    async fn changed_by(&self, artifact: &Artifact, since: i32, person: &str) -> bool {
        self.store
            .revisions(&artifact.person, artifact.id)
            .await
            .map(|all| {
                all.iter().any(|r| {
                    r.n > since && r.by.as_deref() == Some(person) && r.author != Author::Person
                })
            })
            .unwrap_or(false)
    }

    /// A chat turn of `asker`'s that began at `since` and failed with
    /// `why`, carried on as a change job when it ran out of time in the
    /// middle of changing one artifact: better a late result than a
    /// lost one. `brief` is what they asked for, with the conversation
    /// they asked in. The answer to give them when it was; `None` falls
    /// back as any failure does.
    pub(crate) async fn carry_on(
        self: &Arc<Self>,
        why: &anyhow::Error,
        asker: &Asker,
        since: Instant,
        brief: &str,
    ) -> Option<String> {
        why.downcast_ref::<crate::assistant::TimedOut>()?;
        let person = &asker.person;
        let read = self.read_since(person, since);
        let [id] = read.as_slice() else {
            if read.len() > 1 {
                tracing::info!(
                    "[artifacts] {person}: the chat read {} artifacts it may change, so none \
                     goes on as a job",
                    read.len()
                );
            }
            return None;
        };
        let id = *id;
        if self.touched_since(person, since).iter().any(|t| *t != id) {
            tracing::info!(
                "[artifacts] {person}: the chat made or started something else as well, so \
                 nothing goes on as a job"
            );
            return None;
        }
        let opened = match self
            .opened("start_change_job", person, id, Operation::Edit)
            .await
        {
            Ok(o) => o,
            Err(e) => {
                tracing::info!(
                    "[artifacts] {person}: the change to artifact {id} could not go on as a \
                     job: {e}"
                );
                return None;
            }
        };
        match self
            .start_change(asker, opened.artifact, brief.to_string())
            .await
        {
            Ok(a) => {
                self.touch(person, a.id);
                tracing::info!(
                    "[artifacts] {person}: the chat ran out of time changing {:?}; it goes on \
                     as a job",
                    a.title
                );
                Some(STILL_WORKING.to_string())
            }
            Err(why) => {
                tracing::info!(
                    "[artifacts] {person}: the change to artifact {id} could not go on as a \
                     job: {why}"
                );
                None
            }
        }
    }
}

/// A change run's instructions, and the change it is to make.
pub(super) fn change_prompts(
    artifact: &Artifact,
    brief: &str,
    searching: Searching,
) -> (String, String) {
    let kind = kinds::kind(&artifact.kind);
    let label = kind.map_or("document", |k| k.label).to_lowercase();
    let help = kind.map_or("", |k| k.content_help);
    let page = if artifact.page.is_some() {
        format!(
            "\n\nIt has a page. Read it with artifact_get with with_page: true if the change \
             is to the page; change it with artifact_edit_page, replacing only what changes, \
             or write a whole new one with artifact_set_page for a new layout.\n\n{}",
            page::HELP
        )
    } else {
        String::new()
    };
    let system = format!(
        "You are changing one person's {label} in the household Niles serves, in the \
         background: they asked for the change in a chat, and it was more than a chat \
         answer has time for. Nobody is waiting on you: take the time to do it well, and \
         ask nothing, because nobody will answer.\n\nRead it first with artifact_get on \
         artifact {id}: it is as it is now, with whatever they ticked since, and part of \
         the change may be made already. Then make only what they asked for that is not \
         there yet. Save a change to the content with artifact_edit, saying only what \
         changes, or with artifact_update and the whole content when most of it changes, \
         keeping what they ticked; give each a short reason. If a save is refused, the \
         answer says what to fix, so fix that and save again. If they asked a question \
         rather than for a change, save nothing. Finish with one or two sentences to \
         them saying what you changed, or answering their question, in the language they \
         wrote in.\n\nThe content is {help}\n\n{research}{page}",
        id = artifact.id,
        research = research(searching),
    );
    let prompt = format!(
        "Title: {title}\nSummary: {summary}\n\nWhat they asked for, from the conversation \
         they asked in (the change is what they said last):\n\n{brief}",
        title = artifact.title,
        summary = artifact.summary,
    );
    (system, prompt)
}
