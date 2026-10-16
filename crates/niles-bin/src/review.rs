//! What Niles learns, in the background.
//!
//! Every exchange goes into a journal. When the house has been quiet for
//! a few minutes, the assistant reads what was said since the last review and
//! decides what is worth keeping: a fact for memory, a way of doing
//! something for a skill — or, most of the time, nothing. Once a week,
//! after a long quiet, it tidies: merges notes that say the same thing,
//! takes out what is done or kept elsewhere, folds overlapping skills
//! together.
//!
//! Both borrowed from Hermes Agent, which reviews every ten turns rather
//! than every turn. Ours ran after every turn, with a small model looking
//! at one turn at a time, and wrote a half-made automation into memory
//! four times in a minute.
//!
//! Neither job can touch the house: its tools are memory, skills and the
//! list of automations. The review only adds; the tidy-up may take out,
//! and what it takes out can be put back from the app.

use chrono::{DateTime, Utc};
use niles_memory::{MemoryStore, Target};
use niles_skills::SkillStore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

/// Trace of one tool call made during the main turn.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolTrace {
    pub tool: String,
    pub arguments: Value,
    pub result: Value,
}

/// How long the house is quiet before what was said is reviewed: long
/// enough that a conversation is over, and its outcome known.
const REVIEW_AFTER: chrono::Duration = chrono::Duration::minutes(5);
/// How often the tidy-up runs, and how quiet the house must be first.
const TIDY_EVERY: chrono::Duration = chrono::Duration::days(7);
const TIDY_AFTER: chrono::Duration = chrono::Duration::hours(2);
/// After a tidy-up that failed, how long before it is tried again.
const TIDY_RETRY: chrono::Duration = chrono::Duration::hours(6);
const CHECK_EVERY: Duration = Duration::from_secs(60);
/// The most exchanges kept for one review. A day of commands is a few
/// dozen; this is a week without an assistant, and then the oldest go.
const MAX_JOURNAL: usize = 300;

/// One exchange, as the review reads it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Said {
    pub at: DateTime<Utc>,
    /// "the living room", "the app".
    pub place: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub who: Option<String>,
    pub said: String,
    pub reply: String,
}

/// What was said since the last review. Kept in Postgres, so a deploy
/// between a conversation and its review loses neither.
pub(crate) struct Journal {
    said: Mutex<Vec<Said>>,
    last_heard: Mutex<DateTime<Utc>>,
    sink: Option<UnboundedSender<String>>,
}

impl Default for Journal {
    fn default() -> Self {
        Self {
            said: Mutex::default(),
            // A restart counts as being heard: the tidy-up waits for a
            // quiet it has seen, not one it assumed.
            last_heard: Mutex::new(Utc::now()),
            sink: None,
        }
    }
}

impl Journal {
    /// Pick up what an earlier run had not reviewed yet.
    pub(crate) fn with_document(self, document: &str) -> Self {
        match serde_json::from_str::<Vec<Said>>(document) {
            Ok(said) => *self.lock() = said,
            Err(e) => tracing::warn!("[learning] the journal would not read ({e}); starting empty"),
        }
        self
    }

    pub(crate) fn with_sink(mut self, sink: UnboundedSender<String>) -> Self {
        self.sink = Some(sink);
        self
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Said>> {
        self.said.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// One exchange, as it ended.
    pub(crate) fn note(&self, place: &str, who: Option<&str>, said: &str, reply: &str) {
        let now = Utc::now();
        let mut journal = self.lock();
        journal.push(Said {
            at: now,
            place: place.to_string(),
            who: who.map(str::to_string),
            said: said.to_string(),
            reply: reply.to_string(),
        });
        if journal.len() > MAX_JOURNAL {
            let excess = journal.len() - MAX_JOURNAL;
            journal.drain(..excess);
        }
        *self.last_heard.lock().unwrap_or_else(|e| e.into_inner()) = now;
        self.keep(&journal);
    }

    /// How long nobody has said anything.
    pub(crate) fn quiet_for(&self, now: DateTime<Utc>) -> chrono::Duration {
        now - *self.last_heard.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn pending(&self) -> Vec<Said> {
        self.lock().clone()
    }

    /// The first `count` exchanges have been reviewed.
    pub(crate) fn done_through(&self, count: usize) {
        let mut journal = self.lock();
        let count = count.min(journal.len());
        journal.drain(..count);
        self.keep(&journal);
    }

    fn keep(&self, journal: &[Said]) {
        let Some(sink) = &self.sink else { return };
        match serde_json::to_string(journal) {
            Ok(document) => {
                if sink.send(document).is_err() {
                    tracing::warn!("[learning] nothing is keeping the journal any more");
                }
            }
            Err(e) => tracing::warn!("[learning] the journal could not be written down: {e}"),
        }
    }
}

const REVIEW_PROMPT: &str = "You are the background review of Niles, the voice assistant of one \
household. Below is what was said to Niles since the last review, and what it answered. Decide \
whether any of it should be kept for the future. Most of the time nothing should.

There are three places to keep things.

A person's own notes (note_about_person) hold what is true of one person: a preference, a \
routine, a habit, how they like things done. Niles reads them only when that person speaks. \
Nobody has to have said 'remember': something a person says about themselves, or shows twice, \
and that will still be true next month, belongs there. Their current notes are below; do not \
repeat them.

Memory holds facts about the household as a whole. 'user' is the household: who lives here, \
how the house is set up, what holds for everyone. 'agent' is what you have noted about doing \
the job here. A fact about one person goes in their notes, not here. Keep only facts that \
hold in every conversation, whatever the task, written as plain facts rather than instructions. \
Memory is not for what was done or asked for: automations, scenes, timers and the shopping list \
are kept elsewhere (list_automations shows the rules). Nor for anything stale within a week, \
small talk, or a request that was never finished. You can only add notes, and a note that is \
already there in other words will be refused.

Skills hold how to do something for this household that will come up again: a procedure, with \
the household's particulars. Patch the closest existing skill rather than making a new one, and \
view_skill before you patch. A new skill's description is at most 60 characters and starts with \
'Use when'. One-off requests and anything a tool already does in one step are not skills.

Do not keep: failures and errors, what a tool could not do, passing state (weather, what is \
playing, whether a light is on), a correction already acted on unless it shows a lasting \
preference, or anything you are unsure was meant.

If something is worth keeping, keep it once, in the right place. Finish with one short sentence \
saying what you kept, or 'Nothing to keep.'";

const TIDY_PROMPT: &str = "You are the weekly tidy-up of Niles, the voice assistant of one \
household. Its memory and skills are written a little at a time, by different conversations. \
Once a week you put them in order, so that what goes into every prompt is short, true and said \
once.

Memory ('user' is about the household; 'agent' is Niles's own notes):
- Merge notes that say the same thing into one: replace one, remove the rest.
- Remove notes about things done or asked for, such as an automation, a scene, a timer or a \
shopping item. Those are kept elsewhere; list_automations shows the rules. Remove notes that \
went stale within a week, and instructions-to-self that are not facts.
- Keep every lasting fact about the household. When unsure, keep it.
- The household can put back anything you remove, from the app.

Skills (list_skills, view_skill):
- Fold skills that overlap into one: patch the one that stays so nothing is lost, then \
retire_skill the other into it.
- Fix a skill that is wrong or unclear in place, with patch_skill old_text and new_text.
- Leave pinned skills and skills the household asked for as they are, unless two say the same.
- Do not make new skills.

Read before you change anything. Prefer few, clear changes. Finish with one or two short \
sentences for the household saying what you changed, for example 'Removed 3 notes about the \
curtain reminder, which is an automation.', or 'Nothing needed tidying.'";

/// What memory holds now, with how full each part is: a model that can
/// see the space running out writes shorter, and merges.
fn memory_section(memory: &MemoryStore) -> String {
    let mut out = String::from("# Memory now\n");
    for (target, title) in [
        (Target::User, "About the household ('user')"),
        (Target::Memory, "Noted by Niles ('agent')"),
    ] {
        let (used, limit) = memory.usage(target);
        let percent = used * 100 / limit.max(1);
        out.push_str(&format!(
            "\n## {title} — {percent}% full, {used} of {limit} characters\n"
        ));
        let entries = memory.load(target);
        if entries.is_empty() {
            out.push_str("(nothing)\n");
        }
        for entry in entries {
            out.push_str(&format!("- {}\n", entry.text));
        }
    }
    out
}

fn skills_section(skills: &SkillStore) -> String {
    let mut out = String::from("\n# Skills now\n");
    let summaries = skills.list_summaries();
    if summaries.is_empty() {
        out.push_str("(none)\n");
    }
    for s in summaries {
        out.push_str(&format!("- {}: {}\n", s.name, s.description));
    }
    out
}

/// The exchanges as a transcript, in the house's own time.
fn journal_section(said: &[Said], tz: chrono_tz::Tz) -> String {
    let mut out = String::from("\n# What was said since the last review\n");
    for s in said {
        let at = s.at.with_timezone(&tz).format("%a %-d %b %H:%M");
        let who = s.who.as_deref().unwrap_or("Someone");
        out.push_str(&format!(
            "\n[{at}, {place}]\n{who}: {said}\nNiles: {reply}\n",
            place = s.place,
            said = s.said,
            reply = s.reply,
        ));
    }
    out
}

/// What Niles knows about each person, as their notes stand.
fn people_section(voices: &[niles_recognition::EnrolledSpeaker]) -> String {
    let mut out = String::from("\n# What Niles knows about each person\n");
    if voices.is_empty() {
        out.push_str("(nobody is enrolled)\n");
    }
    for v in voices {
        out.push_str(&format!("\n## {}\n", v.display_name));
        match v.notes.as_deref().filter(|n| !n.trim().is_empty()) {
            Some(notes) => {
                out.push_str(notes.trim());
                out.push('\n');
            }
            None => out.push_str("(nothing yet)\n"),
        }
    }
    out
}

/// The background jobs, and what they need.
pub(crate) struct Learning {
    pub assistant: Arc<crate::assistant::Assistant>,
    pub settings: Arc<niles_config::ConfigStore>,
    pub journal: Arc<Journal>,
    pub memory: Option<Arc<MemoryStore>>,
    pub skills: Option<Arc<SkillStore>>,
    pub automations: Option<Arc<niles_automations::AutomationStore>>,
    /// The enrolled voices, whose own notes the review may add to.
    pub voices: Option<Arc<dyn niles_recognition::VoiceRoster>>,
    pub tidy_tried: Mutex<Option<DateTime<Utc>>>,
}

impl Learning {
    pub(crate) fn spawn(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(CHECK_EVERY).await;
                self.tick(Utc::now()).await;
            }
        })
    }

    async fn tick(&self, now: DateTime<Utc>) {
        let quiet = self.journal.quiet_for(now);
        if !self.journal.pending().is_empty() && quiet >= REVIEW_AFTER {
            self.review().await;
        } else if quiet >= TIDY_AFTER && self.tidy_due(now) {
            *self.tidy_tried.lock().unwrap_or_else(|e| e.into_inner()) = Some(now);
            self.tidy().await;
        }
    }

    /// The assistant to do it, when one is switched on and can run.
    /// Without one there is no review: a missed review costs nothing, a
    /// careless one fills memory that goes into every prompt.
    fn chosen(&self) -> Option<crate::assistant::Chosen> {
        match crate::assistant::choose(&self.settings.current().integrations) {
            crate::assistant::Choice::Use(chosen) => Some(chosen),
            crate::assistant::Choice::Unready { reason, .. } => {
                tracing::warn!("[learning] {reason}");
                None
            }
            crate::assistant::Choice::Off => None,
        }
    }

    fn tz(&self) -> chrono_tz::Tz {
        self.settings
            .current()
            .home
            .timezone
            .parse()
            .unwrap_or(chrono_tz::Tz::UTC)
    }

    async fn review(&self) {
        let said = self.journal.pending();
        let enabled = self.settings.current().skills.review.enabled;
        let chosen = enabled.then(|| self.chosen()).flatten();
        let Some(chosen) = chosen else {
            tracing::info!(
                "[learning] {} exchange(s) left unreviewed: the review is off, or no assistant is on",
                said.len()
            );
            self.journal.done_through(said.len());
            return;
        };
        let mut prompt = String::new();
        if let Some(memory) = &self.memory {
            prompt.push_str(&memory_section(memory));
        }
        if let Some(skills) = &self.skills {
            prompt.push_str(&skills_section(skills));
        }
        let mut tools = niles_tools::background_registry(
            niles_tools::Background::Review,
            self.memory.clone(),
            self.skills.clone(),
            self.automations.clone(),
        );
        if let Some(voices) = &self.voices {
            match voices.voices().await {
                Ok(people) => {
                    prompt.push_str(&people_section(&people));
                    crate::profile::register_for_review(&mut tools, voices.clone());
                }
                Err(e) => tracing::warn!("[learning] the people could not be read: {e}"),
            }
        }
        prompt.push_str(&journal_section(&said, self.tz()));
        let system = format!(
            "{REVIEW_PROMPT}\n\n{}",
            crate::home_context(&self.settings.current().home)
        );
        tracing::info!(
            "[learning] {} is reviewing {} exchange(s)",
            chosen.name(),
            said.len()
        );
        match self
            .assistant
            .background(&chosen, &system, &prompt, tools, None)
            .await
        {
            Ok(summary) => tracing::info!("[learning] review: {summary}"),
            Err(e) => tracing::warn!("[learning] the review did not finish: {e:#}"),
        }
        // Reviewed or not, these are done: a review that fails on the
        // same exchanges every minute would never let the next ones in.
        self.journal.done_through(said.len());
    }

    fn tidy_due(&self, now: DateTime<Utc>) -> bool {
        let Some(skills) = &self.skills else {
            return false;
        };
        let tried = *self.tidy_tried.lock().unwrap_or_else(|e| e.into_inner());
        if tried.is_some_and(|t| now - t < TIDY_RETRY) {
            return false;
        }
        match skills.tidied() {
            Some(t) => now - t.at >= TIDY_EVERY,
            None => {
                !skills.list().is_empty()
                    || self.memory.as_ref().is_some_and(|m| {
                        !m.load(Target::User).is_empty() || !m.load(Target::Memory).is_empty()
                    })
            }
        }
    }

    async fn tidy(&self) {
        let (Some(skills), Some(chosen)) = (&self.skills, self.chosen()) else {
            return;
        };
        let mut prompt = String::new();
        if let Some(memory) = &self.memory {
            prompt.push_str(&memory_section(memory));
        }
        prompt.push_str(&skills_section(skills));
        let tools = niles_tools::background_registry(
            niles_tools::Background::Tidy,
            self.memory.clone(),
            Some(skills.clone()),
            self.automations.clone(),
        );
        let system = format!(
            "{TIDY_PROMPT}\n\n{}",
            crate::home_context(&self.settings.current().home)
        );
        tracing::info!("[learning] {} is tidying memory and skills", chosen.name());
        match self
            .assistant
            .background(&chosen, &system, &prompt, tools, None)
            .await
        {
            Ok(summary) => {
                let summary: String = summary.trim().chars().take(400).collect();
                tracing::info!("[learning] tidy-up: {summary}");
                skills.mark_tidied(&summary);
            }
            Err(e) => tracing::warn!("[learning] the tidy-up did not finish: {e:#}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_journal_holds_what_was_said_until_it_is_reviewed() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let journal = Journal::default().with_sink(tx);
        journal.note(
            "the living room",
            Some("Mark"),
            "turn off the lights",
            "Done.",
        );
        journal.note("the app", None, "add milk", "Added milk.");
        assert_eq!(journal.pending().len(), 2);

        journal.done_through(1);
        assert_eq!(journal.pending()[0].said, "add milk");

        let mut last = None;
        while let Ok(document) = rx.try_recv() {
            last = Some(document);
        }
        let next = Journal::default().with_document(&last.unwrap());
        assert_eq!(next.pending().len(), 1);
    }

    #[test]
    fn quiet_is_counted_from_the_last_thing_said() {
        let journal = Journal::default();
        journal.note("the office", None, "hello", "Good afternoon.");
        let later = Utc::now() + chrono::Duration::minutes(6);
        assert!(journal.quiet_for(later) >= REVIEW_AFTER);
        assert!(journal.quiet_for(Utc::now()) < REVIEW_AFTER);
    }

    #[test]
    fn the_review_reads_a_transcript_in_the_house_s_time() {
        let said = vec![Said {
            at: "2026-10-13T12:46:00Z".parse().unwrap(),
            place: "the living room".into(),
            who: Some("Mark".into()),
            said: "Yes".into(),
            reply: "Saved. When it is 30 minutes after sunset, ...".into(),
        }];
        let out = journal_section(&said, chrono_tz::Europe::Copenhagen);
        assert!(
            out.contains("[Tue 13 Oct 14:46, the living room]\nMark: Yes\nNiles: Saved."),
            "{out}"
        );
    }

    #[test]
    fn each_person_s_notes_are_shown_so_they_are_not_repeated() {
        let voice = |name: &str, notes: Option<&str>| niles_recognition::EnrolledSpeaker {
            speaker: name.to_lowercase(),
            display_name: name.to_string(),
            spoken_as: None,
            address_as: None,
            notes: notes.map(str::to_string),
            birthday: None,
            created_at: Utc::now(),
            last_seen_at: None,
            clip_count: 3,
            embeddings: Vec::new(),
        };
        let out = people_section(&[
            voice("Mark", Some("- Takes his coffee black")),
            voice("Majse", None),
        ]);
        assert!(
            out.contains("## Mark\n- Takes his coffee black\n\n## Majse\n(nothing yet)"),
            "{out}"
        );
    }

    #[test]
    fn memory_is_shown_with_how_full_it_is() {
        let memory = MemoryStore::new(niles_memory::MemoryConfig {
            user_char_limit: 100,
            agent_char_limit: 200,
        });
        memory
            .add(Target::User, "Mark takes his coffee black.")
            .unwrap();
        let out = memory_section(&memory);
        assert!(out.contains("28% full, 28 of 100 characters"), "{out}");
        assert!(out.contains("- Mark takes his coffee black."));
        assert!(out.contains("Noted by Niles ('agent') — 0% full"));
    }
}
