//! Dispatching a turn: what was said goes to the intent router, Tier 1, or the assistant.

use crate::app::{note_chat, proposal_since};
use crate::chat_loop::{
    AUTOMATION_CHANGES, ChatProvider, LoopOutcome, MAX_TOOL_ITERATIONS,
    run_tool_calling_chat_with_messages,
};
use crate::dispatch::hearing::{
    heard_its_name, is_foreign_script_hallucination, is_noise_transcript, is_stopping_an_alarm,
    strip_wake_word,
};
use crate::dispatch::text::dispatch_text;
use crate::prompt::{
    HEARD_NOT_READ, SpeakerContext, TYPED_IN_THE_APP,
    assemble_system_prompt_with_optional_capabilities, join_memory_entries, now_playing_section,
};
use crate::recognition::SpeakerIdentifier;
use crate::satellites::SatelliteRegistry;
use crate::{
    approvals, artifacts, assistant, chat_log, conversation, courtesy, follow_up, last_target,
    music, profile, response, review, speak, speakers, tv,
};
use chrono::{Timelike, Utc};
use niles_capabilities::CapabilityLoader;
use niles_core::{DeviceId, DeviceRegistry, RoomName};
use niles_history::CommandWriter;
use niles_intent::{DeviceIndex, Intent};
use niles_llm::{Message, OpenAiClient};
use niles_memory::MemoryStore;
use niles_mqtt::{CommandRouter, MqttPublisher};
use niles_presence::{PresenceAggregator, TadoSource};
use niles_scheduler::{ManualModeTracker, SceneStore, TimerStore};
use niles_skills::SkillStore;
use niles_stt::Confidence;
use niles_tools::ToolRegistry;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};
use std::time::Instant;

pub(crate) mod hearing;
pub(crate) mod media;
pub(crate) mod targets;
pub(crate) mod text;

/// Shared by every spawned dispatch task. Cheaply cloneable —
/// `MqttPublisher`, `Arc`, `bool` all clone in constant time.
#[derive(Clone)]
pub(crate) struct DispatchCtx {
    pub(crate) publisher: MqttPublisher,
    pub(crate) registry: Arc<DeviceRegistry>,
    pub(crate) router: Arc<CommandRouter>,
    pub(crate) dry_run: bool,
    pub(crate) tracker: Arc<ManualModeTracker>,
    pub(crate) scenes: Arc<SceneStore>,
    pub(crate) timers: Arc<TimerStore>,
    pub(crate) speakers: Arc<speakers::SpeakerRegistry>,
    pub(crate) music: Arc<music::Music>,
    pub(crate) tv: Arc<tv::TvControl>,
    pub(crate) hush: Arc<speak::Hush>,
    pub(crate) llm: Arc<OpenAiClient>,
    pub(crate) tier2: Option<Arc<dyn ChatProvider>>,
    pub(crate) identifier: Option<Arc<dyn SpeakerIdentifier>>,
    pub(crate) tools: Arc<ToolRegistry>,
    pub(crate) capability_loader: Option<Arc<CapabilityLoader>>,
    pub(crate) capability_index: Option<Arc<niles_intent::CapabilityIndex>>,
    #[allow(dead_code)]
    pub(crate) presence: Option<Arc<PresenceAggregator>>,
    pub(crate) satellites: Arc<SatelliteRegistry>,
    pub(crate) device_index: Arc<RwLock<DeviceIndex>>,
    pub(crate) history: Arc<CommandWriter>,
    /// Absent only when the kept memory could not be read.
    pub(crate) memory: Option<Arc<MemoryStore>>,
    pub(crate) skill_store: Option<Arc<SkillStore>>,
    pub(crate) home: Arc<niles_config::HomeConfig>,
    /// Where kept wake audio goes, when there is a database for it.
    pub(crate) captures: Option<Arc<niles_db::PostgresCaptures>>,
    /// The live config, for settings a person changes and expects to
    /// take effect.
    ///
    /// These were copied out at startup, which is how "Only answer
    /// voices Niles knows" could be switched on in the app and go on
    /// answering the television all day: the switch wrote the config,
    /// the config reloaded, and dispatch kept the value it had read
    /// before anybody touched it. A snapshot is right for `home`,
    /// which nobody edits mid-sentence. It is wrong for anything with
    /// a switch in front of it.
    pub(crate) settings: Option<Arc<niles_config::ConfigStore>>,
    /// What was said, for the review once the house is quiet.
    pub(crate) journal: Arc<review::Journal>,
    pub(crate) conversation: Arc<conversation::ConversationMemory>,
    /// Every conversation, kept for the person who had it, for the app's
    /// chat. Absent without a database.
    pub(crate) chat_log: Option<Arc<chat_log::ChatLog>>,
    /// What "it" refers to, per room. See [`last_target`].
    pub(crate) last_target: Arc<last_target::LastTarget>,
    /// For "what's the weather", answered without the LLM.
    pub(crate) weather: Option<Arc<niles_weather::OpenMeteoClient>>,
    /// Who has been called "Sir" lately, and greeted this morning.
    pub(crate) courtesy: Arc<courtesy::Courtesy>,
    /// Claude Code or ChatGPT, whichever is switched on and chosen:
    /// answers the app's chat and spoken turns about rules. Absent
    /// outside `serve`, and when its MCP port could not be bound.
    pub(crate) assistant: Option<Arc<assistant::Assistant>>,
    /// Questions Niles has asked through a satellite and is waiting on.
    pub(crate) follow_ups: Arc<follow_up::FollowUps>,
    /// For the heating by voice. The same session the dashboard and
    /// presence use: tado rotates its refresh token on every use, so
    /// there can only be one.
    pub(crate) tado: Option<Arc<TadoSource>>,
    /// The household's rules, and the one last read back for a yes.
    /// Absent outside `serve`.
    pub(crate) automations: Option<(
        Arc<niles_automations::AutomationStore>,
        Arc<niles_tools::automations::Proposals>,
    )>,
    /// What is waiting for somebody's yes.
    pub(crate) approvals: Arc<approvals::Approvals>,
    /// Each person's artifacts, and the jobs that write them. Absent
    /// without a database, and outside `serve`.
    pub(crate) artifacts: Option<Arc<artifacts::Artifacts>>,
}

impl DispatchCtx {
    /// The plugs that have a lamp on them, as the config has it now.
    pub(crate) fn lamp_plugs(&self) -> HashSet<DeviceId> {
        self.settings
            .as_ref()
            .map(|s| s.current().lamp_plugs.ids().clone())
            .unwrap_or_default()
    }
}

pub(crate) async fn handle_transcript(
    ctx: &DispatchCtx,
    peer: SocketAddr,
    text: &str,
    confidence: Option<Confidence>,
    speaker: &SpeakerContext,
    voice: Option<&[f32]>,
    // An answer to Niles's own question, heard without the wake word.
    answering: bool,
) -> Option<String> {
    let started = Instant::now();
    // The first thing somebody says to Niles in a morning is greeted, as
    // Alexa does. Decided before the reply is built, because a greeting
    // addresses them and the confirmation must not then do it again.
    let greeting = match speaker {
        SpeakerContext::Identified { name, known } => {
            let local = ctx
                .home
                .timezone
                .parse::<chrono_tz::Tz>()
                .ok()
                .map(|tz| Utc::now().with_timezone(&tz));
            let whom = known.address.clone().or_else(|| {
                let id = ctx.identifier.as_ref()?;
                id.whose_voice(voice?).and_then(|slug| id.how_to_say(&slug))
            });
            local
                .filter(|l| {
                    ctx.courtesy
                        .greet_now(name, l.date_naive(), l.hour(), Instant::now())
                })
                .map(|l| {
                    let birthday = known
                        .birthday
                        .as_deref()
                        .is_some_and(|b| niles_recognition::birthday::is_today(b, l.date_naive()));
                    (name.clone(), whom.unwrap_or_else(|| name.clone()), birthday)
                })
        }
        _ => None,
    };
    // The profile tools write to whoever is speaking, and nobody else:
    // the voice decides that, not the model.
    let slug = match speaker {
        SpeakerContext::Identified { known, .. } => known.slug.clone(),
        _ => None,
    };
    // A question asked aloud is answered at the satellite that asked it,
    // by the voice that asked: or, where nobody is recognised, by whoever
    // is there.
    let asker = approvals::Asker {
        person: match &slug {
            Some(slug) => chat_log::person(Some(slug), ""),
            None => format!("satellite:{peer}"),
        },
        slug: slug.clone(),
        satellite: Some(peer),
        conversation: None,
    };
    let mut response = profile::SPEAKER
        .scope(
            slug,
            approvals::ASKER.scope(
                asker,
                dispatch_transcript(ctx, peer, text, confidence, speaker, voice, answering),
            ),
        )
        .await;
    if let Some((name, whom, birthday)) = greeting {
        match &response {
            Some(reply) => response = Some(courtesy::greeted(reply, &whom, birthday)),
            None => ctx.courtesy.ungreet(&name),
        }
    }
    if let Some(reply) = &response {
        let room = ctx.satellites.room_for(peer).cloned();
        if let (Some(log), SpeakerContext::Identified { known, .. }) = (&ctx.chat_log, speaker)
            && let Some(slug) = &known.slug
        {
            let turn = niles_api::chat::Turn {
                at: chrono::Utc::now(),
                said: text.to_string(),
                reply: reply.clone(),
                via: "voice".into(),
                room: room.as_ref().map(|r| r.as_str().to_string()),
                by: None,
                proposal: proposal_since(ctx, started),
                // A mail asked for aloud waits in the app, under the
                // turn that asked for it.
                approval: ctx
                    .approvals
                    .for_turn(&chat_log::person(Some(slug), ""), started),
                artifacts: Vec::new(),
                attachments: Vec::new(),
            };
            let (log, person) = (log.clone(), chat_log::person(Some(slug), ""));
            // Its card learns its conversation, as a typed turn's does, so
            // the panel can answer it and the card in the chat follows.
            let approvals = ctx.approvals.clone();
            tokio::spawn(async move {
                if let Some(id) = log.record_spoken(&person, &turn).await
                    && let Some(card) = &turn.approval
                {
                    approvals.place(&card.id, id);
                }
            });
        }
        let place = room.as_ref().map_or_else(
            || "a room".to_string(),
            |r| format!("the {}", r.as_str().replace('_', " ")),
        );
        let who = match speaker {
            SpeakerContext::Identified { name, .. } => Some(name.as_str()),
            _ => None,
        };
        ctx.journal.note(&place, who, text, reply);
        ctx.conversation
            .record(&conversation::Thread::Room(room), text, reply);
    }
    response
}

/// Run the Tier 1 LLM (with Tier 2 escalation) for `text` and return the
/// spoken response. Invoked on a Tier 0 miss, and also when a Tier 0 intent
/// matched but resolved no target device — e.g. "living room ceiling light"
/// makes Tier 0 grab "living room ceiling" as the room (no such devices);
/// rather than dead-ending, the LLM resolves the real `living_room/ceiling`.
#[expect(
    clippy::too_many_lines,
    reason = "one Tier 1 turn; its outcomes (done, hand-off, escalate) share the messages built first"
)]
pub(crate) async fn dispatch_tier1(
    ctx: &DispatchCtx,
    peer: SocketAddr,
    text: &str,
    origin_room: Option<&RoomName>,
    thread: &conversation::Thread,
    speaker: &SpeakerContext,
) -> Option<String> {
    tracing::info!("[{peer}] dispatching to Tier 1 LLM: {text:?}");
    let system_prompt = turn_system_prompt(ctx, text, origin_room, thread, speaker).await;
    if matches!(thread, conversation::Thread::Chat(_)) {
        if let Some(reply) =
            ask_assistant(ctx, peer, text, thread, speaker, &system_prompt, &[]).await
        {
            return Some(reply);
        }
    } else if let Some((_, proposals)) = ctx.automations.as_ref()
        && (niles_tools::automations::about_automations(text) || proposals.in_talk())
    {
        // A rule spoken aloud goes to the assistant when one is on: the
        // fast model dropped "half an hour after" and left out the
        // trigger, and five turns went by before the rule was right.
        // Slower, but the satellite is kept waiting (`keep_waiting`).
        if let Some(reply) =
            ask_assistant(ctx, peer, text, thread, speaker, &system_prompt, &[]).await
        {
            if follow_up::asks(&reply) {
                proposals.discussed();
            }
            return Some(reply);
        }
    }
    let mut messages = Vec::new();
    messages.push(Message::System {
        content: system_prompt.clone(),
    });
    messages.extend(ctx.conversation.recent_messages(thread));
    messages.push(Message::User {
        content: text.to_string(),
    });
    // A spoken rule the keyword check missed — "every weekday at 6:30
    // turn on the bedroom light" — still reaches the assistant: the fast
    // model reaches for the automation tools, and is stopped there. So
    // does a tool server's, where the household left it to the assistant:
    // filing a well-written issue is worth a few seconds.
    let mut hand_off: Vec<String> = Vec::new();
    if matches!(thread, conversation::Thread::Room(_)) && assistant_on(ctx) {
        if ctx.automations.is_some() {
            hand_off.extend(AUTOMATION_CHANGES.iter().map(|t| t.to_string()));
        }
        hand_off.extend(handed_to_the_assistant(ctx));
    }
    let hand_off: Vec<&str> = hand_off.iter().map(String::as_str).collect();
    let unhanded = messages.clone();
    match run_tool_calling_chat_with_messages(
        ctx.llm.as_ref(),
        ctx.tools.as_ref(),
        messages,
        MAX_TOOL_ITERATIONS,
        None,
        &hand_off,
    )
    .await
    {
        Ok((LoopOutcome::Done(response), _)) => {
            tracing::info!("[{peer}] \"{text}\" -> (Tier 1) {response}");
            Some(response)
        }
        Ok((LoopOutcome::HandOff { tool }, _)) => {
            tracing::info!(
                "[{peer}] the fast model reached for {tool}; \"{text}\" goes to the assistant"
            );
            if let Some(reply) =
                ask_assistant(ctx, peer, text, thread, speaker, &system_prompt, &[]).await
            {
                if follow_up::asks(&reply)
                    && let Some((_, proposals)) = ctx.automations.as_ref()
                {
                    proposals.discussed();
                }
                return Some(reply);
            }
            // The assistant could not answer: the fast model does it
            // after all.
            match run_tool_calling_chat_with_messages(
                ctx.llm.as_ref(),
                ctx.tools.as_ref(),
                unhanded,
                MAX_TOOL_ITERATIONS,
                None,
                &[],
            )
            .await
            {
                Ok((LoopOutcome::Done(response), _)) => Some(response),
                Ok(_) => Some("Sorry, I'm not able to handle that request.".into()),
                Err(e) => {
                    tracing::warn!("[{peer}] Tier 1 LLM dispatch failed: {e:#}");
                    Some("Sorry, something went wrong.".into())
                }
            }
        }
        Ok((LoopOutcome::EscalateRequested { reason, messages }, _tool_trace)) => {
            tracing::info!("[{peer}] Tier 1 requested escalation: {reason}");
            match &ctx.tier2 {
                Some(tier2) => {
                    tracing::info!("[{peer}] \"{text}\" -> escalating to Tier 2 ({reason})");
                    match run_tool_calling_chat_with_messages(
                        tier2.as_ref(),
                        ctx.tools.as_ref(),
                        messages,
                        MAX_TOOL_ITERATIONS,
                        Some(niles_tools::escalate::ESCALATE_TOOL_NAME),
                        &[],
                    )
                    .await
                    {
                        Ok((LoopOutcome::Done(response), _)) => {
                            tracing::info!("[{peer}] \"{text}\" -> (Tier 2) {response}");
                            Some(response)
                        }
                        Ok((
                            LoopOutcome::EscalateRequested { .. } | LoopOutcome::HandOff { .. },
                            _,
                        )) => {
                            println!(
                                "[{peer}] \"{text}\" -> (Tier 2) escalation requested; ignoring"
                            );
                            tracing::warn!("[{peer}] Tier 2 also requested escalation; ignoring");
                            Some("Sorry, I'm not able to handle that request.".into())
                        }
                        Err(e) => {
                            tracing::info!("[{peer}] \"{text}\" -> (Tier 2) error: {e:#}");
                            tracing::warn!("[{peer}] Tier 2 dispatch failed: {e:#}");
                            Some("Sorry, something went wrong.".into())
                        }
                    }
                }
                None => {
                    println!(
                        "[{peer}] \"{text}\" -> (Tier 1) escalation requested but Tier 2 not configured"
                    );
                    tracing::warn!("[{peer}] Tier 2 not configured, falling through");
                    Some("Sorry, I'm not able to escalate that request right now.".into())
                }
            }
        }
        Err(e) => {
            // Mirror the success-path stdout line so a Tier 1 failure is
            // visible without enabling tracing.
            tracing::info!("[{peer}] \"{text}\" -> (Tier 1) error: {e:#}");
            tracing::warn!("[{peer}] Tier 1 LLM dispatch failed: {e:#}");
            Some("Sorry, something went wrong.".into())
        }
    }
}

/// The system prompt for one turn, the same for the fast model and
/// for the assistant.
async fn turn_system_prompt(
    ctx: &DispatchCtx,
    text: &str,
    origin_room: Option<&RoomName>,
    thread: &conversation::Thread,
    speaker: &SpeakerContext,
) -> String {
    let user_mem = ctx
        .memory
        .as_ref()
        .map(|m| m.load(niles_memory::Target::User));
    let agent_mem = ctx
        .memory
        .as_ref()
        .map(|m| m.load(niles_memory::Target::Memory));
    let user_mem_str = user_mem.as_deref().and_then(join_memory_entries);
    let agent_mem_str = agent_mem.as_deref().and_then(join_memory_entries);
    let skill_summaries = ctx.skill_store.as_ref().map(|s| s.list_summaries());
    let mut system_prompt = assemble_system_prompt_with_optional_capabilities(
        text,
        &ctx.home,
        ctx.capability_index.as_deref(),
        ctx.capability_loader.as_deref(),
        origin_room,
        user_mem_str.as_deref(),
        agent_mem_str.as_deref(),
        skill_summaries.as_deref(),
        speaker,
    );
    system_prompt.push_str(&now_playing_section(&ctx.music.now_playing().await));
    // What the household wrote about a tool server's tools, with the
    // tools themselves: where a coding task goes, which team is theirs.
    for note in ctx.tools.notes_for(text) {
        system_prompt.push_str("\n\n");
        system_prompt.push_str(&note);
    }
    if !matches!(thread, conversation::Thread::Chat(_)) {
        system_prompt.push_str(HEARD_NOT_READ);
    }
    if matches!(thread, conversation::Thread::Chat(_)) {
        system_prompt.push_str(TYPED_IN_THE_APP);
    }
    system_prompt
}

/// A typed turn that came with files.
///
/// Tier 0 cannot see a picture and the fast model cannot either, so
/// there is no router and no fallback here: the assistant answers it
/// or nobody does. `None` is "it could not", and why is on the note.
pub(crate) async fn dispatch_attached(
    ctx: &DispatchCtx,
    peer: SocketAddr,
    text: &str,
    thread: &conversation::Thread,
    speaker: &SpeakerContext,
    attachments: &[assistant::Attached],
) -> Option<String> {
    let system_prompt = turn_system_prompt(ctx, text, None, thread, speaker).await;
    // An empty message with files is a message: the run is asked to
    // look at them. What the turn keeps as said stays exactly as typed.
    let text = if text.is_empty() {
        "Have a look at the attached file."
    } else {
        text
    };
    ask_assistant(
        ctx,
        peer,
        text,
        thread,
        speaker,
        &system_prompt,
        attachments,
    )
    .await
}
/// The tools of every tool server the household left to the assistant,
/// as they are offered now.
pub(crate) fn handed_to_the_assistant(ctx: &DispatchCtx) -> Vec<String> {
    let Some(settings) = ctx.settings.as_ref() else {
        return Vec::new();
    };
    handed(&settings.current().tool_servers, ctx.tools.dynamic_names())
}

/// Of the tools offered, `(owner, name)`, those of the servers handed
/// to the assistant.
pub(crate) fn handed(
    servers: &std::collections::BTreeMap<String, niles_config::ToolServerConfig>,
    offered: Vec<(String, String)>,
) -> Vec<String> {
    let owners: Vec<String> = servers
        .iter()
        .filter(|(_, server)| server.hand_off)
        .map(|(id, _)| format!("mcp:{id}"))
        .collect();
    offered
        .into_iter()
        .filter(|(owner, _)| owners.contains(owner))
        .map(|(_, name)| name)
        .collect()
}

/// Whether an assistant is there, switched on and able to run, read per
/// turn like the chat's switch.
pub(crate) fn assistant_on(ctx: &DispatchCtx) -> bool {
    ctx.assistant.is_some()
        && ctx.settings.as_ref().is_some_and(|s| {
            matches!(
                assistant::choose(&s.current().integrations),
                assistant::Choice::Use(_)
            )
        })
}

/// The answer from the assistant — Claude Code or ChatGPT — when one is
/// switched on and can give it.
///
/// `None` sends the turn on to `[llm]` as if there were no assistant: a
/// subscription that has lapsed, a sign-in that has run out or a run
/// that timed out costs the quality of the answer, not the answer.
/// Callers with files must not let that happen: a fast model that
/// cannot see a picture answering about it is worse than no answer.
pub(crate) async fn ask_assistant(
    ctx: &DispatchCtx,
    peer: SocketAddr,
    text: &str,
    thread: &conversation::Thread,
    speaker: &SpeakerContext,
    system_prompt: &str,
    attachments: &[assistant::Attached],
) -> Option<String> {
    let assistant = ctx.assistant.as_ref()?;
    // Read per turn: switching it on in the app takes hold with the next
    // message, not the next restart.
    let chosen = match assistant::choose(&ctx.settings.as_ref()?.current().integrations) {
        assistant::Choice::Off => return None,
        assistant::Choice::Unready { via, reason } => {
            tracing::warn!("[{peer}] {reason}; using [llm]");
            note_chat(|n| n.fallback = Some((via, reason)));
            return None;
        }
        assistant::Choice::Use(chosen) => chosen,
    };
    let slug = match speaker {
        SpeakerContext::Identified { known, .. } => known.slug.clone(),
        _ => None,
    };
    let history = ctx.conversation.recent_messages(thread);
    let started = Instant::now();
    let name = chosen.name();
    // Only in the app's chat, where a card can show what was saved, and
    // for somebody Niles knows: an unrecognised voice has nowhere to
    // keep one.
    let mut system_prompt = system_prompt.to_string();
    if let Some(artifacts) = &ctx.artifacts
        && matches!(thread, conversation::Thread::Chat(_))
        && approvals::asker().is_some_and(|a| !a.person.starts_with("satellite:"))
    {
        system_prompt.push_str(&artifacts::turn_prompt(
            artifacts.searching(Some(chosen.via())),
        ));
    }
    system_prompt.push_str(&assistant::attachments_section(attachments));
    match assistant
        .answer(&chosen, &system_prompt, &history, text, slug, attachments)
        .await
    {
        Ok(reply) => {
            println!(
                "[{peer}] \"{text}\" -> ({name}, {}ms) {reply}",
                started.elapsed().as_millis()
            );
            note_chat(|n| n.via = Some(chosen.via()));
            Some(reply)
        }
        Err(e) => {
            tracing::warn!("[{peer}] {name} could not answer ({e:#}); using [llm]");
            note_chat(|n| {
                n.fallback = Some((chosen.via(), format!("{name} could not answer: {e:#}")));
            });
            None
        }
    }
}

/// Teach Niles a voice from the sentence that introduced it.
///
/// Refuses to add clips to a name that is already enrolled to a voice
/// this one doesn't match: a guest saying "I am Mark" would otherwise
/// become Mark, and every later sentence of theirs would be attributed
/// to him. `niles enroll` remains the way round it, which is the right
/// place for that power — it needs shell access to the cluster.
pub(crate) async fn enroll_by_voice(
    ctx: &DispatchCtx,
    peer: SocketAddr,
    name: &str,
    voice: Option<&[f32]>,
) -> String {
    let Some(identifier) = ctx.identifier.as_ref() else {
        return response::enrollment_unavailable();
    };
    let Some(voice) = voice else {
        return response::enrollment_no_audio();
    };
    // A voice Niles already knows is that person, whatever Whisper made
    // of the name this time.
    //
    // The name is the primary key, and it comes from a transcript. That
    // is fine for "Mark" and disastrous for a name Whisper cannot spell
    // twice: one person introducing herself four times became Maisa,
    // Maise, Maize and Meise, four residents with one clip each and no
    // way to recognise any of them. Matching on the voice first makes
    // the spelling a label rather than an identity.
    let known = identifier.whose_voice(voice);
    if let Some(known) = &known
        && known != name
    {
        tracing::info!(
            "[{peer}] heard the name {name:?}, but this voice is {known:?} — adding a clip to them"
        );
    }
    let name = known.as_deref().unwrap_or(name);

    if identifier.is_someone_else(name, voice).await {
        tracing::warn!("[{peer}] refused to enrol {name}: voice does not match the enrolled one");
        return response::enrollment_name_taken(name);
    }
    match identifier.enroll(name, voice).await {
        Ok(clips) => {
            tracing::info!("[{peer}] enrolled {name} ({clips} clip(s))");
            // Said, not keyed. The slug is what Whisper heard; what
            // Niles says back is the name somebody wrote down, or the
            // respelling that makes Piper pronounce it.
            let spoken = identifier
                .how_to_say(name)
                .unwrap_or_else(|| name.to_string());
            response::enrolled(&spoken, clips)
        }
        Err(e) => {
            tracing::warn!("[{peer}] enrolling {name} failed: {e:#}");
            response::enrollment_failed()
        }
    }
}

/// A yes or no to what a satellite just asked before running a tool set
/// to Ask first: decided here, from the words, never by the model.
/// Anything that is not a yes or a no leaves it waiting.
pub(crate) async fn answer_to_approval(
    ctx: &DispatchCtx,
    peer: SocketAddr,
    text: &str,
    speaker: &SpeakerContext,
) -> Option<String> {
    let (id, asker) = ctx.approvals.spoken(peer)?;
    // Only the voice that asked, when Niles knew it.
    if let Some(asked) = &asker.slug {
        let answering = match speaker {
            SpeakerContext::Identified { known, .. } => known.slug.as_ref(),
            _ => None,
        };
        if answering != Some(asked) {
            return None;
        }
    }
    let yes = niles_tools::automations::answer(text)?;
    let held = ctx.approvals.take(&id, &asker.person)?;
    let tool = held.tool_name().to_string();
    let decision = if yes {
        niles_api::chat::Decision::Once
    } else {
        niles_api::chat::Decision::Deny
    };
    let (card, reply) = ctx
        .approvals
        .settle(held, decision, approvals::Via::Voice, None)
        .await;
    ctx.approvals.changed(&asker.person, &card);
    tracing::info!(
        "[{peer}] {tool}: {} by voice",
        if yes { "allowed" } else { "denied" }
    );
    Some(reply)
}

/// A plain yes or no to the rule Niles just read back, answered here
/// rather than by the model. Asked to save it, the model proposed it
/// again and said "saved" — and nothing was.
pub(crate) fn answer_to_proposal(
    ctx: &DispatchCtx,
    peer: SocketAddr,
    text: &str,
) -> Option<String> {
    let (store, proposals) = ctx.automations.as_ref()?;
    if !proposals.pending() {
        return None;
    }
    let yes = niles_tools::automations::answer(text)?;
    let rule = proposals.take()?;
    if !yes {
        tracing::info!("[{peer}] automation not saved: {:?}", rule.description);
        return Some("All right, I won't save it.".into());
    }
    Some(match store.create(rule) {
        Ok(made) => {
            tracing::info!("[{peer}] automation saved: {}", made.id);
            format!("Saved. {}", niles_automations::sentence(&made))
        }
        Err(e) => {
            tracing::warn!("[{peer}] automation could not be saved: {e}");
            format!("I couldn't save it: {e}")
        }
    })
}

pub(crate) async fn dispatch_transcript(
    ctx: &DispatchCtx,
    peer: SocketAddr,
    text: &str,
    // What Whisper thought of its own answer, when the provider says.
    confidence: Option<Confidence>,
    speaker: &SpeakerContext,
    // The voice print of this very utterance, so "I am Mark" can enrol
    // the sentence that said it. `None` when recognition is off or the
    // audio was unusable.
    voice: Option<&[f32]>,
    // An answer to a question Niles asked: it has no name in front of it
    // and may be a single word, and that is fine.
    answering: bool,
) -> Option<String> {
    let called_by_name = heard_its_name(text);
    let text = strip_wake_word(text);

    // `transcribe_session` already trims, so an empty `text` here means
    // the transcriber returned nothing for a silent/noise session. Don't
    // burn an LLM round-trip on it — Tier 0 wouldn't match either.
    if text.is_empty() {
        tracing::debug!("[{peer}] empty transcript, skipping dispatch");
        return None;
    }

    // Drop transcripts with no words at all — pure punctuation / symbols
    // Whisper occasionally emits for non-speech. The satellite VAD keeps
    // silence and click noise out of STT, so we no longer denylist
    // phrases like "thank you"; a real "thank you" gets a real reply.
    if is_noise_transcript(text) {
        tracing::debug!("[{peer}] dropping wordless STT output: {text:?}");
        return None;
    }

    // Whisper hallucinates CJK stock phrases on quiet/noisy audio; in a
    // non-CJK household those are never real commands. Drop them before they
    // burn a Tier 1 round-trip and a spoken non-sequitur reply.
    if is_foreign_script_hallucination(text, &ctx.home.resolved_language()) {
        tracing::debug!("[{peer}] dropping foreign-script hallucination: {text:?}");
        return None;
    }

    // Everything above reads the words. This reads Whisper's opinion of
    // whether there were any, which is the only thing that can tell a
    // door closing from somebody saying "thank you" — the invented
    // sentence is a perfectly ordinary one, and no rule about shape
    // will catch it.
    // Read per turn, not copied at startup: there is a switch in front
    // of this in the app, and a setting that needs a restart to take
    // effect is a setting somebody will reasonably believe is on.
    let settings = ctx.settings.as_ref().map(|s| s.current());
    let stt_gate = settings
        .as_ref()
        .map(|c| c.stt.noise_gate)
        .unwrap_or_default();
    let known_voices_only = settings
        .as_ref()
        .is_some_and(|c| c.recognition.known_voices_only);

    let stopping_an_alarm = is_stopping_an_alarm(text, &ctx.timers);

    if let Some(c) = confidence
        && !stopping_an_alarm
        && stt_gate.rejects(c)
    {
        tracing::info!(
            no_speech_prob = c.no_speech_prob,
            avg_logprob = c.avg_logprob,
            "[{peer}] not acting on {text:?}: the transcriber does not think that was speech"
        );
        // Silent on purpose. The house saying "sorry?" to a door being
        // closed is the same interruption as acting on it, minus the
        // lights.
        return None;
    }

    // The words are fine and somebody really said them. Whether it was
    // somebody who lives here is a different question, and the only
    // one that separates a command from a television.
    //
    // Placed before Tier 0, not after: a stranger must not be able to
    // switch a light off through a regex either. And before the
    // enrolment pattern, which is the point — "I am Sofia" from an
    // unknown voice is exactly what the lock is for.
    // A wake the transcript does not start with the name for is the
    // room, not a person: a television, a conversation. Silent, like the
    // other gates — and "stop" to a ringing alarm still gets through.
    let require_name = settings.as_ref().is_some_and(|c| c.stt.require_name);
    if require_name && !called_by_name && !stopping_an_alarm && !answering {
        tracing::info!("[{peer}] not acting on {text:?}: it did not start with Niles's name");
        return None;
    }

    if known_voices_only && !stopping_an_alarm && matches!(speaker, SpeakerContext::Unknown) {
        // Nobody enrolled means nobody known, and a lock with no key
        // cut would refuse the whole house — including whoever wants
        // to turn it off.
        let anyone_known = ctx.identifier.as_ref().is_some_and(|i| i.knows_anybody());
        if anyone_known {
            tracing::info!("[{peer}] not acting on {text:?}: voice not recognised");
            // Silent for the room, for the same reason the noise gate is:
            // a television set this off fifty-three times in one day, and
            // answering each one replaces a wrong answer with a longer
            // one. But a sentence that began with Niles's name was said
            // *to* Niles — the television almost never does that — and
            // silence there read as a broken satellite to somebody who
            // had asked for the floor lamp with the television on.
            return called_by_name.then(response::voice_not_recognised);
        }
        tracing::warn!(
            "[{peer}] known_voices_only is on and nobody is enrolled; letting {text:?} through"
        );
    }

    let thread = conversation::Thread::Room(ctx.satellites.room_for(peer).cloned());
    dispatch_text(ctx, peer, text, &thread, speaker, voice, answering).await
}

/// Compact, human-readable rendering of an [`Intent`] for the dev
/// dispatch log. `{:?}` works but the output is noisier than we want
/// when each line is a single utterance.
pub(crate) fn format_intent(intent: &Intent) -> String {
    // `Intent` is `#[non_exhaustive]` — a future variant lands as
    // `{:?}` until this dev tool catches up.
    match intent {
        Intent::LightSet { room, on } => {
            let state = if *on { "on" } else { "off" };
            format!("LightSet({room} -> {state})")
        }
        Intent::LightSetAll { on } => {
            let state = if *on { "on" } else { "off" };
            format!("LightSetAll(home -> {state})")
        }
        Intent::LightDim { room, percent } => {
            format!("LightDim({room} -> {percent}%)")
        }
        Intent::LightStep {
            room,
            delta_percent,
        } => {
            format!("LightStep({room} {delta_percent:+}%)")
        }
        Intent::LightKelvinStep { room, delta_kelvin } => {
            format!("LightKelvinStep({room} {delta_kelvin:+}K)")
        }
        Intent::LightKelvinSet { room, kelvin } => {
            format!("LightKelvinSet({room} {kelvin}K)")
        }
        Intent::DeviceSet { device_id, on } => {
            let state = if *on { "on" } else { "off" };
            format!("DeviceSet({device_id} -> {state})")
        }
        Intent::DeviceDim { device_id, percent } => {
            format!("DeviceDim({device_id} -> {percent}%)")
        }
        Intent::ClearManualMode { room } => {
            format!("ClearManualMode({})", room.as_deref().unwrap_or("home"))
        }
        Intent::MediaPause { room } => format!("MediaPause({room})"),
        Intent::MediaPlay { room } => format!("MediaPlay({room})"),
        Intent::MediaNext { room } => format!("MediaNext({room})"),
        Intent::MediaPrevious { room } => format!("MediaPrevious({room})"),
        Intent::MediaVolumeSet { room, percent } => format!("MediaVolumeSet({room} -> {percent}%)"),
        Intent::MediaVolumeStep { room, delta } => format!("MediaVolumeStep({room} -> {delta:+})"),
        Intent::TimerSet { duration, name } => match name {
            Some(n) => format!("TimerSet({}s, name={n:?})", duration.as_secs()),
            None => format!("TimerSet({}s)", duration.as_secs()),
        },
        Intent::SceneSave { name, room } => match room {
            Some(r) => format!("SceneSave({name:?} in {r})"),
            None => format!("SceneSave({name:?})"),
        },
        Intent::SceneApply { name } => format!("SceneApply({name:?})"),
        Intent::SceneList => "SceneList".into(),
        Intent::SceneDelete { name } => format!("SceneDelete({name:?})"),
        Intent::TimerCancel { name } => format!("TimerCancel({name:?})"),
        Intent::TimerList => "TimerList".into(),
        Intent::TimerRemaining => "TimerRemaining".into(),
        Intent::Stop => "Stop".into(),
        Intent::Cancel => "Cancel".into(),
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod hand_off_tests {
    use super::handed;

    #[test]
    fn a_servers_tools_go_to_the_assistant_unless_kept_back() {
        let servers: std::collections::BTreeMap<String, niles_config::ToolServerConfig> =
            toml::from_str(
                "[linear]
url = \"https://mcp.linear.app/mcp\"
[sentry]
url = \"https://mcp.sentry.dev/mcp\"
hand_off = false
",
            )
            .unwrap();
        let offered = vec![
            ("mcp:linear".to_string(), "linear__create_issue".to_string()),
            ("mcp:sentry".to_string(), "sentry__list_issues".to_string()),
            ("spotprice".to_string(), "spotprice_prices".to_string()),
        ];
        assert_eq!(handed(&servers, offered), ["linear__create_issue"]);
    }
}
