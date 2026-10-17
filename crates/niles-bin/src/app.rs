//! The app's chat and music, as the API's `Chat` and `Music` traits.

use crate::dispatch::DispatchCtx;
use crate::dispatch::text::dispatch_text;
use crate::prompt::{APP_PEER, SpeakerContext};
use crate::wiring::speech::LiveStt;
use crate::{approvals, chat_log, conversation, music, profile, training};
use niles_core::RoomName;
use std::sync::Arc;
use std::time::Instant;

/// What a chat turn wants the app to know about how it was answered.
#[derive(Default)]
pub(crate) struct ChatNote {
    /// What wrote the reply, when it was not Niles's own models.
    pub(crate) via: Option<&'static str>,
    /// Which assistant did not, when it was meant to, and why. Shown
    /// under the reply: falling back silently made a broken setup look
    /// like one that worked and was merely fast.
    pub(crate) fallback: Option<(&'static str, String)>,
}

tokio::task_local! {
    /// Scoped around one chat turn. A spoken turn has no scope, and
    /// writing to it there does nothing.
    pub(crate) static CHAT_NOTE: std::cell::RefCell<ChatNote>;
}

/// Note something about this chat turn, if it is one.
pub(crate) fn note_chat(write: impl FnOnce(&mut ChatNote)) {
    let _ = CHAT_NOTE.try_with(|note| write(&mut note.borrow_mut()));
}

/// The app's chat, answered by the same dispatch as a spoken command.
///
/// Built before the dispatch context exists, because the API starts
/// first, and handed it once it does.
pub(crate) struct AppChat {
    pub(crate) ctx: Arc<std::sync::OnceLock<DispatchCtx>>,
    /// The satellites' speech-to-text, so dictation hears Danish words
    /// the way a spoken command does.
    pub(crate) stt: Arc<LiveStt>,
}

/// The rooms' music for the app, from the same [`music::Music`] voice
/// uses — so a pause from the card leaves a group the way "pause the
/// kitchen" does.
pub(crate) struct AppMusic(pub(crate) Arc<music::Music>);

#[async_trait::async_trait]
impl niles_api::music::Music for AppMusic {
    async fn rooms(&self) -> Vec<niles_api::music::RoomMusic> {
        self.0.rooms().await
    }

    async fn pause(&self, room: &str) -> Result<(), String> {
        settled(self.0.pause(&room_named(room)?).await)
    }

    async fn resume(&self, room: &str) -> Result<(), String> {
        settled(self.0.resume(&room_named(room)?).await)
    }

    async fn volume(&self, room: &str, percent: u8) -> Result<(), String> {
        settled(self.0.volume(&room_named(room)?, Some(percent), None).await)
    }

    async fn media(&self) -> niles_api::music::MediaView {
        self.0.view().await
    }

    async fn join(&self, speaker: &str, leader: &str) -> Result<(), String> {
        self.0.join_group(speaker, leader).await
    }

    async fn leave(&self, speaker: &str) -> Result<(), String> {
        self.0.leave_group(speaker).await
    }

    async fn group(&self, leader: &str, play: bool) -> Result<(), String> {
        self.0.play_group(leader, play).await
    }

    async fn speaker_volume(&self, speaker: &str, percent: u8) -> Result<(), String> {
        self.0.speaker_volume(speaker, percent).await
    }

    async fn choices(
        &self,
        kind: &str,
        query: Option<&str>,
    ) -> Result<Vec<niles_api::music::Choice>, String> {
        self.0.choices(kind, query).await
    }

    async fn start(
        &self,
        speakers: &[String],
        choice: &niles_api::music::Choice,
    ) -> Result<(), String> {
        self.0.start_on(speakers, choice).await
    }

    async fn tv(&self) -> Result<(), String> {
        self.0.tv_sound().await
    }
}

pub(crate) fn room_named(room: &str) -> Result<RoomName, String> {
    RoomName::parse(room).map_err(|e| format!("{room:?} is not a room: {e}"))
}

pub(crate) fn settled(outcome: music::Outcome) -> Result<(), String> {
    match outcome {
        music::Outcome::Paused | music::Outcome::Resumed | music::Outcome::Volume(_) => Ok(()),
        music::Outcome::NoSpeaker => Err("there is no speaker in that room".into()),
        other => Err(format!("{other:?}")),
    }
}

#[async_trait::async_trait]
impl niles_api::chat::Chat for AppChat {
    async fn reply(
        &self,
        who: &str,
        speaker: Option<&str>,
        text: &str,
        attachments: Vec<niles_api::chat::Upload>,
        into: niles_api::chat::Where,
    ) -> Result<niles_api::chat::Reply, String> {
        let ctx = self
            .ctx
            .get()
            .ok_or_else(|| "Niles is still starting; try again in a moment".to_string())?;
        // Both checks before anything is stored, so a refusal costs
        // nothing and leaves nothing behind.
        if !attachments.is_empty() {
            if !crate::dispatch::assistant_on(ctx) {
                return Err(
                    "Niles needs Claude or ChatGPT to look at a file. Turn one on in \
                     Settings → Voice & AI."
                        .into(),
                );
            }
            if ctx.chat_log.is_none() {
                return Err("Niles has no database here, so it cannot keep a file.".into());
            }
        }
        let thread = conversation::Thread::Chat(who.to_string());
        let person = chat_log::person(speaker, who);
        // The conversation picked up is the context, every turn of it,
        // spoken ones too: a reply to what was said in the kitchen knows
        // what was said in the kitchen.
        let target = match &ctx.chat_log {
            Some(log) => {
                let target = log.target(&person, into).await?;
                let turns = target.as_ref().map_or_else(Vec::new, |(_, turns)| {
                    turns
                        .iter()
                        .map(|t| (t.said.clone(), t.reply.clone(), t.by.clone()))
                        .collect()
                });
                ctx.conversation.seed(&thread, turns);
                target.map(|(id, _)| id)
            }
            None => {
                if into == niles_api::chat::Where::New {
                    ctx.conversation.forget(&thread);
                }
                None
            }
        };
        let known = typed_speaker(ctx, speaker);
        // The profile tools write to whoever is signed in, as they write
        // to whoever is speaking.
        let slug = match &known {
            SpeakerContext::Identified { known, .. } => known.slug.clone(),
            _ => None,
        };
        let started = Instant::now();
        // Whose yes anything held on this turn waits for.
        let asker = approvals::Asker {
            person: person.clone(),
            slug: slug.clone(),
            satellite: None,
            conversation: target,
        };
        // Kept before dispatch: the files are the person's from the
        // moment they are stored, and bound to their turn once the
        // turn is kept. A store failure returns its own reason;
        // earlier rows stay unbound and are swept within a day.
        let mut kept_ids = Vec::new();
        let mut metas = Vec::new();
        let mut attached = Vec::new();
        for (n, upload) in attachments.iter().enumerate() {
            let log = ctx.chat_log.as_ref().expect("checked above");
            let id = log.attach(&person, upload).await?;
            kept_ids.push(id);
            metas.push(niles_api::chat::Attachment {
                id,
                kind: upload.kind,
                name: upload.name.clone(),
                size: upload.bytes.len(),
                width: upload.width,
                height: upload.height,
            });
            attached.push(crate::assistant::Attached {
                name: upload.name.clone(),
                file: crate::assistant::file_name(n + 1, &upload.mime),
                bytes: Arc::new(upload.bytes.clone()),
            });
        }
        let (reply, note) = CHAT_NOTE
            .scope(std::cell::RefCell::default(), async {
                // And the training tools read whoever is signed in, voice
                // or not, by the key their chat is kept under.
                let reply = training::CHAT_PERSON
                    .scope(
                        person.clone(),
                        profile::SPEAKER.scope(
                            slug,
                            approvals::ASKER.scope(asker, async {
                                if attached.is_empty() {
                                    dispatch_text(ctx, APP_PEER, text, &thread, &known, None, true)
                                        .await
                                } else {
                                    // Files bypass the router and
                                    // the fast model entirely: only
                                    // the assistant can see them.
                                    crate::dispatch::dispatch_attached(
                                        ctx, APP_PEER, text, &thread, &known, &attached,
                                    )
                                    .await
                                }
                            }),
                        ),
                    )
                    .await;
                (reply, CHAT_NOTE.with(|note| note.take()))
            })
            .await;
        // A turn with files that nobody answered is an error, not a
        // "Done.": nothing is recorded, so the person keeps their chips
        // and can try again.
        if !attached.is_empty() && reply.is_none() {
            return Err(match note.fallback {
                Some((_, why)) => why,
                None => "Claude or ChatGPT could not answer, and Niles's own model \
                         cannot see a file."
                    .to_string(),
            });
        }
        let via = note.via;
        // Nothing to say is how a spoken turn ends when the action was
        // the answer. On a screen that looks like being ignored.
        let reply = reply.unwrap_or_else(|| "Done.".to_string());
        ctx.conversation.record_via(&thread, text, &reply, via);
        let name = match &known {
            SpeakerContext::Identified { name, .. } => Some(name.as_str()),
            _ => None,
        };
        ctx.journal.note("the app", name, text, &reply);
        let conversation_id = match &ctx.chat_log {
            Some(log) => {
                let turn = niles_api::chat::Turn {
                    at: chrono::Utc::now(),
                    said: text.to_string(),
                    reply: reply.clone(),
                    via: "app".into(),
                    room: None,
                    by: via.map(str::to_string),
                    proposal: proposal_since(ctx, started),
                    approval: ctx.approvals.for_turn(&person, started),
                    artifacts: match &ctx.artifacts {
                        Some(artifacts) => artifacts.for_turn(&person, started).await,
                        None => Vec::new(),
                    },
                    attachments: metas.clone(),
                };
                let kept = log
                    .record(&person, target, &turn)
                    .await
                    .unwrap_or_else(|e| {
                        tracing::warn!("[chat-log] a typed turn was not kept: {e}");
                        None
                    });
                if let (Some(card), Some(id)) = (&turn.approval, kept) {
                    ctx.approvals.place(&card.id, id);
                }
                if let (Some(artifacts), Some(id)) = (&ctx.artifacts, kept) {
                    artifacts.placed(&person, started, id).await;
                }
                if !kept_ids.is_empty()
                    && let (Some(log), Some(id)) = (&ctx.chat_log, kept)
                    && let Err(e) = log.bind(&person, &kept_ids, id).await
                {
                    tracing::warn!("[chat-log] a turn's files were not tied to it: {e}");
                }
                kept
            }
            None => None,
        };
        Ok(niles_api::chat::Reply {
            reply,
            via: via.map(str::to_string),
            fallback: note.fallback.as_ref().map(|(_, why)| why.clone()),
            fallback_from: note.fallback.map(|(from, _)| from.to_string()),
            conversation_id,
            attachments: metas,
        })
    }

    async fn attachment(
        &self,
        who: &str,
        speaker: Option<&str>,
        conversation: i64,
        id: i64,
    ) -> Option<niles_api::chat::Served> {
        let ctx = self.ctx.get()?;
        let log = ctx.chat_log.as_ref()?;
        log.attachment(&chat_log::person(speaker, who), conversation, id)
            .await
            .ok()
            .flatten()
    }

    async fn conversations(
        &self,
        who: &str,
        speaker: Option<&str>,
    ) -> Vec<niles_api::chat::Summary> {
        let Some(log) = self.ctx.get().and_then(|ctx| ctx.chat_log.as_ref()) else {
            return Vec::new();
        };
        log.list(&chat_log::person(speaker, who))
            .await
            .unwrap_or_else(|e| {
                tracing::warn!("[chat-log] {e}");
                Vec::new()
            })
    }

    async fn conversation(
        &self,
        who: &str,
        speaker: Option<&str>,
        id: i64,
    ) -> Option<niles_api::chat::Conversation> {
        let ctx = self.ctx.get()?;
        let log = ctx.chat_log.as_ref()?;
        let mut conversation = log
            .conversation(&chat_log::person(speaker, who), id)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!("[chat-log] {e}");
                None
            })?;
        if let Some(artifacts) = &ctx.artifacts {
            artifacts
                .refresh(&chat_log::person(speaker, who), &mut conversation.turns)
                .await;
        }
        // A card the store no longer holds lapsed, or was held by a Niles
        // that has since restarted: either way nothing is waiting.
        for card in conversation
            .turns
            .iter_mut()
            .filter_map(|t| t.approval.as_mut())
        {
            if card.status == niles_api::chat::ApprovalStatus::Pending
                && !ctx.approvals.waiting(&card.id)
            {
                card.status = niles_api::chat::ApprovalStatus::Lapsed;
            }
        }
        Some(conversation)
    }

    async fn decide(
        &self,
        who: &str,
        speaker: Option<&str>,
        id: i64,
        approval: &str,
        decision: niles_api::chat::Decision,
    ) -> Result<String, niles_api::chat::Undecidable> {
        use niles_api::chat::{Decision, Undecidable};
        let ctx = self.ctx.get().ok_or(Undecidable::NoSuchConversation)?;
        let log = ctx
            .chat_log
            .as_ref()
            .ok_or(Undecidable::NoSuchConversation)?;
        let person = chat_log::person(speaker, who);
        let conversation = log
            .conversation(&person, id)
            .await
            .ok()
            .flatten()
            .ok_or(Undecidable::NoSuchConversation)?;
        let card = conversation
            .turns
            .iter()
            .filter_map(|t| t.approval.as_ref())
            .find(|a| a.id == approval)
            .ok_or(Undecidable::NoSuchApproval)?;
        if decision == Decision::Conversation && !card.allow_conversation {
            return Err(Undecidable::OnceOnly);
        }
        let held = ctx
            .approvals
            .take(approval, &person)
            .ok_or(Undecidable::NotWaiting)?;
        let tool = held.tool_name().to_string();
        let (card, reply) = match decision {
            Decision::Deny => held.deny(),
            Decision::Once => held.run().await,
            Decision::Conversation => {
                ctx.approvals.grant(&person, id, &tool);
                held.run().await
            }
        };
        tracing::info!("[approvals] {tool}: {:?} by {person}", card.status);
        if let Err(e) = log.update_approval(&person, id, &card).await {
            tracing::warn!("[chat-log] the answered card was not kept: {e}");
        }
        let said = match decision {
            Decision::Once => "Allow once",
            Decision::Conversation => "Allow for this conversation",
            Decision::Deny => "Deny",
        };
        // Into the conversation, so the next turn knows what happened.
        let thread = conversation::Thread::Chat(who.to_string());
        ctx.conversation.record_via(&thread, said, &reply, None);
        let answered = niles_api::chat::Turn {
            at: chrono::Utc::now(),
            said: said.into(),
            reply: reply.clone(),
            via: "app".into(),
            room: None,
            by: None,
            proposal: None,
            approval: None,
            artifacts: Vec::new(),
            attachments: Vec::new(),
        };
        if let Err(e) = log.record(&person, Some(id), &answered).await {
            tracing::warn!("[chat-log] the answer was not kept: {e}");
        }
        Ok(reply)
    }

    async fn answer(
        &self,
        who: &str,
        speaker: Option<&str>,
        id: i64,
        turn: usize,
        save: bool,
    ) -> Result<String, niles_api::chat::Unanswerable> {
        use niles_api::chat::Unanswerable;
        let ctx = self.ctx.get().ok_or(Unanswerable::NoSuchConversation)?;
        let log = ctx
            .chat_log
            .as_ref()
            .ok_or(Unanswerable::NoSuchConversation)?;
        let person = chat_log::person(speaker, who);
        let conversation = log
            .conversation(&person, id)
            .await
            .ok()
            .flatten()
            .ok_or(Unanswerable::NoSuchConversation)?;
        // Only the last turn's: anything said after a read-back answered
        // it, or moved on from it.
        if turn + 1 != conversation.turns.len() {
            return Err(Unanswerable::NothingToAnswer);
        }
        let rule = conversation.turns[turn]
            .proposal
            .as_ref()
            .and_then(|p| p.rule.clone())
            .and_then(|rule| serde_json::from_value::<niles_config::AutomationRuleDto>(rule).ok())
            .ok_or(Unanswerable::NothingToAnswer)?;
        let Some((store, proposals)) = ctx.automations.as_ref() else {
            return Err(Unanswerable::NothingToAnswer);
        };
        proposals.settled(&rule);
        let reply = if save {
            match store.create(rule) {
                Ok(made) => {
                    tracing::info!("[chat] automation saved from the app: {}", made.id);
                    format!("Saved. {}", niles_automations::sentence(&made))
                }
                Err(e) => format!("I couldn't save it: {e}"),
            }
        } else {
            "All right, I won't save it.".to_string()
        };
        let answered = niles_api::chat::Turn {
            at: chrono::Utc::now(),
            said: if save { "Save" } else { "Don't save" }.into(),
            reply: reply.clone(),
            via: "app".into(),
            room: None,
            by: None,
            proposal: None,
            approval: None,
            artifacts: Vec::new(),
            attachments: Vec::new(),
        };
        if let Err(e) = log.record(&person, Some(id), &answered).await {
            tracing::warn!("[chat-log] the answer was not kept: {e}");
        }
        Ok(reply)
    }

    async fn delete(&self, who: &str, speaker: Option<&str>, id: i64) -> bool {
        let Some(log) = self.ctx.get().and_then(|ctx| ctx.chat_log.as_ref()) else {
            return false;
        };
        log.delete(&chat_log::person(speaker, who), id)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!("[chat-log] {e}");
                false
            })
    }

    async fn transcribe(&self, audio: Vec<u8>, filename: &str) -> Result<String, String> {
        self.stt
            .current()
            .transcribe(audio, filename)
            .await
            .map(|t| t.text.trim().to_string())
            .map_err(|e| format!("could not transcribe that: {e}"))
    }
}

/// The rule Niles read back during a turn that began at `started`, to
/// keep with the turn: the app shows it with Save and Don't save.
pub(crate) fn proposal_since(
    ctx: &DispatchCtx,
    started: Instant,
) -> Option<niles_api::chat::Proposal> {
    let (_, proposals) = ctx.automations.as_ref()?;
    let rule = proposals.offered_since(started)?;
    Some(niles_api::chat::Proposal {
        sentence: niles_automations::sentence(&rule),
        in_full: Some(niles_automations::describe(&rule)),
        warnings: niles_automations::warnings(&rule),
        rule: serde_json::to_value(&rule).ok(),
    })
}

/// Who is typing, as the prompt would know them by voice.
///
/// Only a voice the recogniser still has: a link to one since forgotten
/// is nobody in particular.
pub(crate) fn typed_speaker(ctx: &DispatchCtx, speaker: Option<&str>) -> SpeakerContext {
    let (Some(slug), Some(id)) = (speaker, ctx.identifier.as_ref()) else {
        return SpeakerContext::Disabled;
    };
    let Some(name) = id.how_to_say(slug) else {
        return SpeakerContext::Disabled;
    };
    let (notes, birthday) = id.profile(slug);
    SpeakerContext::Identified {
        name,
        known: profile::Known {
            slug: Some(slug.to_string()),
            address: id.address_as(slug),
            notes,
            birthday,
        },
    }
}
