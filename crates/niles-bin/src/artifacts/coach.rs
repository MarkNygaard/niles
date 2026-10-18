//! A training plan that keeps up with the runs. The plan's shape is in
//! `niles-artifacts/src/training_plan.rs`.
//!
//! After every sync of a person's Strava their plans are brought up to
//! date in code ([`tracking::track`]): each new run marks the session
//! it was, and the days gone by without one are missed. That is Niles's
//! own version of the plan, and needs no assistant.
//!
//! A plan that got a run is then looked at by the coach: the assistant,
//! in the background, told the run beside what was planned. It may
//! change what is still to come and leave the person a note, and
//! nothing that has happened: the save is refused in code when it
//! tries ([`tracking::keeps_the_past`]), whatever its prompt says.

use super::{Artifacts, Held, JOB_LIMIT, Slot, TRAINING_PLAN};
use crate::approvals::Asker;
use chrono::{Duration, NaiveDate, Utc};
use niles_artifacts::tracking::{self, Done, Sport, Workout};
use niles_artifacts::{Artifact, Author, Filter, NewRevision, Status, kinds};
use niles_training::{Activity, Kind};
use serde_json::Value;
use std::sync::Arc;

/// A plan is followed until a few days after its race, so the race is
/// marked too, however late its upload.
const AFTER_THE_RACE: i64 = 3;

/// How far back the coach is told of the sessions missed.
const MISSED_FOR: i64 = 14;

/// A workout as a plan's session takes it, on the house's day. A walk
/// is no session: it is how people get about.
fn workout(a: &Activity, tz: &chrono_tz::Tz) -> Option<Workout> {
    let sport = match a.kind {
        Kind::Run => Sport::Run,
        Kind::Strength => Sport::Strength,
        Kind::Ride | Kind::Other => Sport::Cross,
        _ => return None,
    };
    let km = a.distance_m.map(|m| m / 1000.0).filter(|km| *km > 0.0);
    let pace = match (sport, km, a.pace_seconds()) {
        (Sport::Run, Some(km), Some(s)) => {
            let per = (s / km).round() as i64;
            Some(format!("{}:{:02}", per / 60, per % 60))
        }
        _ => None,
    };
    Some(Workout {
        id: a.external_id.clone(),
        day: a.started_at.with_timezone(tz).date_naive(),
        sport,
        distance_km: km,
        duration_min: a.duration_s.map(|s| s / 60.0),
        pace,
    })
}

fn race_day(content: &Value) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(content["goal"]["date"].as_str()?, "%Y-%m-%d").ok()
}

impl Artifacts {
    /// What one sync brought, followed in its person's plans.
    pub(crate) async fn runs_arrived(self: &Arc<Self>, arrived: &niles_training::Arrived) {
        let today = Utc::now().with_timezone(&self.tz).date_naive();
        self.follow(&arrived.person, &arrived.activities, today)
            .await;
    }

    /// Every plan of `person`'s still going, brought up to date with
    /// `activities` as of `today`. Not one being written: its job saves
    /// over whatever this would, and reads the runs itself.
    async fn follow(self: &Arc<Self>, person: &str, activities: &[Activity], today: NaiveDate) {
        let filter = Filter {
            archived: Some(false),
            ..Filter::default()
        };
        let plans = match self.store.list(person, filter).await {
            Ok(all) => all,
            Err(e) => {
                tracing::warn!("[coach] {person}: could not read the plans: {e}");
                return;
            }
        };
        let workouts: Vec<Workout> = activities
            .iter()
            .filter_map(|a| workout(a, &self.tz))
            .collect();
        for plan in plans.into_iter().filter(|a| {
            a.kind == TRAINING_PLAN && a.status != Status::Working && a.current_revision > 0
        }) {
            if let Err(e) = self.follow_plan(plan.clone(), &workouts, today).await {
                tracing::warn!("[coach] {person}: {:?} was not kept up: {e}", plan.title);
            }
        }
    }

    async fn follow_plan(
        self: &Arc<Self>,
        plan: Artifact,
        workouts: &[Workout],
        today: NaiveDate,
    ) -> Result<(), String> {
        let storage = |e: niles_artifacts::Error| e.to_string();
        let Some(current) = self
            .store
            .revision(&plan.person, plan.id, plan.current_revision)
            .await
            .map_err(storage)?
        else {
            return Ok(());
        };
        if race_day(&current.content)
            .is_none_or(|race| race + Duration::days(AFTER_THE_RACE) < today)
        {
            return Ok(());
        }
        let Some(tracked) = tracking::track(&current.content, workouts, today) else {
            return Ok(());
        };
        let content = kinds::validate(TRAINING_PLAN, &tracked.content).map_err(storage)?;
        let Some(saved) = self
            .store
            .revise(
                &plan.person,
                plan.id,
                NewRevision {
                    content: content.clone(),
                    reason: niles_artifacts::summary(&tracked.reason()),
                    author: Author::Niles,
                    by: None,
                    summary: None,
                    page: niles_artifacts::PageChange::Keep,
                },
            )
            .await
            .map_err(storage)?
        else {
            return Ok(());
        };
        tracing::info!(
            "[coach] {}: {:?}: {}",
            saved.person,
            saved.title,
            tracked.reason()
        );
        self.announce(&saved);
        let runs: Vec<Done> = tracked
            .done
            .into_iter()
            .filter(|d| d.workout.sport == Sport::Run)
            .collect();
        if !runs.is_empty() {
            self.review(saved, &content, &runs, today);
        }
        Ok(())
    }

    /// The coach's look at a plan after its runs, all of them in one
    /// look, when an assistant is on, the person has a job free and the
    /// plan is not being looked at already. Otherwise nobody looks, and
    /// says nothing: the runs are marked either way. Whether it began.
    fn review(
        self: &Arc<Self>,
        plan: Artifact,
        content: &Value,
        runs: &[Done],
        today: NaiveDate,
    ) -> bool {
        if self.runner.available().is_none() {
            return false;
        }
        let Some(reviewing) = Held::claim(&self.reviewing, plan.id) else {
            return false;
        };
        let Some(slot) = self.take_slot(&plan.person) else {
            return false;
        };
        let (system, prompt) = review_prompts(&plan, content, runs, today);
        let this = self.clone();
        tokio::spawn(async move {
            this.run_review(plan, &system, &prompt, today, slot, reviewing)
                .await
        });
        true
    }

    async fn run_review(
        self: Arc<Self>,
        plan: Artifact,
        system: &str,
        prompt: &str,
        today: NaiveDate,
        _slot: Slot,
        _reviewing: Held,
    ) {
        let person = plan.person.clone();
        let asker = Asker {
            person: person.clone(),
            slug: person.strip_prefix("person:").map(str::to_string),
            satellite: None,
            conversation: None,
        };
        let tools = self.job_tools(plan.id, TRAINING_PLAN, Some(today));
        let outcome =
            tokio::time::timeout(JOB_LIMIT, self.runner.run(asker, system, prompt, tools))
                .await
                .unwrap_or_else(|_| Err("it took longer than half an hour".into()));
        match &outcome {
            Ok((via, answer)) => {
                tracing::info!("[coach] {person}: {via} on {:?}: {answer}", plan.title)
            }
            Err(why) => tracing::warn!(
                "[coach] {person}: {:?} was not looked at: {why}",
                plan.title
            ),
        }
        let Some((now, note)) = self.reviewed(&plan).await else {
            return;
        };
        self.announce(&now);
        if let (true, Some(note)) = (now.notify, note) {
            self.notifier.changed(&now, &note).await;
        }
    }

    /// The plan after its look, if the look saved anything; with what
    /// to tell the person when sessions changed: its note, or else its
    /// reason. Nothing to tell when it only added a note.
    async fn reviewed(&self, plan: &Artifact) -> Option<(Artifact, Option<String>)> {
        let now = self.store.get(&plan.person, plan.id).await.ok()??;
        let by_coach: Vec<i32> = self
            .store
            .revisions(&plan.person, plan.id)
            .await
            .ok()?
            .into_iter()
            .filter(|r| {
                r.n > plan.current_revision && matches!(r.author, Author::Claude | Author::Chatgpt)
            })
            .map(|r| r.n)
            .collect();
        let newest = *by_coach.first()?;
        let mut changed = false;
        for n in &by_coach {
            let (Ok(Some(this)), Ok(Some(before))) = (
                self.store.revision(&plan.person, plan.id, *n).await,
                self.store.revision(&plan.person, plan.id, n - 1).await,
            ) else {
                continue;
            };
            changed |= this.content["weeks"] != before.content["weeks"];
        }
        if !changed {
            return Some((now, None));
        }
        let last = self
            .store
            .revision(&plan.person, plan.id, newest)
            .await
            .ok()??;
        let note = last.content["updates"][0]["note"]
            .as_str()
            .map_or(last.reason, str::to_string);
        Some((now, Some(note)))
    }
}

/// "Wed 14 Oct".
fn short(day: NaiveDate) -> String {
    day.format("%a %-d %b").to_string()
}

fn trim(x: f64) -> String {
    let s = format!("{x:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// "6.2 km in 39 min at 6:17 /km".
fn ran(w: &Workout) -> String {
    let mut out = Vec::new();
    if let Some(km) = w.distance_km {
        out.push(format!("{} km", trim(km)));
    }
    if let Some(min) = w.duration_min {
        out.push(format!("in {} min", min.round()));
    }
    if let Some(pace) = &w.pace {
        out.push(format!("at {pace} /km"));
    }
    if out.is_empty() {
        "a run".into()
    } else {
        out.join(" ")
    }
}

/// "Easy run (easy) on Tue 13 Oct: 6 km, 6:30 /km. Conversational."
fn planned(s: &Value) -> String {
    let mut line = format!(
        "{} ({})",
        s["title"].as_str().unwrap_or("a session"),
        s["type"].as_str().unwrap_or("run")
    );
    if let Some(day) = s["date"]
        .as_str()
        .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
    {
        line.push_str(&format!(" on {}", short(day)));
    }
    let mut measures = Vec::new();
    if let Some(km) = s["distance_km"].as_f64() {
        measures.push(format!("{} km", trim(km)));
    }
    if let Some(min) = s["duration_min"].as_f64() {
        measures.push(format!("{} min", trim(min)));
    }
    if let Some(pace) = s["pace"].as_str() {
        measures.push(format!("{pace} /km"));
    }
    if !measures.is_empty() {
        line.push_str(&format!(": {}", measures.join(", ")));
    }
    if let Some(details) = s["details"].as_str() {
        line.push_str(&format!(". {}", details.replace('\n', " ")));
    }
    line
}

/// The coach's instructions, and what it is to look at: the runs beside
/// what was planned for them, and what was missed lately.
fn review_prompts(
    plan: &Artifact,
    content: &Value,
    runs: &[Done],
    today: NaiveDate,
) -> (String, String) {
    let id = plan.id;
    let help = kinds::kind(TRAINING_PLAN).map_or("", |k| k.content_help);
    let system = format!(
        "You are the running coach of one person in the household Niles serves, looking over \
         their training plan, artifact {id}, after their latest run. You are in the \
         background: nobody is waiting in a chat, and nobody will answer a question.\n\n\
         Niles has marked the run on the plan already. Read the plan with artifact_get; \
         training_activities and training_summary have their running if you need more than \
         what you are told here.\n\n\
         Change the plan only if what was run warrants it: a run much slower, faster, shorter \
         or longer than planned, sessions missed one after another, signs that they are tired \
         or ahead of the plan. Then change the sessions still to come, from {today} on and \
         still planned, and only a little, keeping the plan's shape and the race on its day. \
         Never a session before {today}, and never one that is done, missed, skipped or \
         moved: Niles refuses a save that changes any of them. Save the whole content with \
         artifact_update on artifact {id}, with a short reason, and first in updates \
         {{\"date\": \"{today}\", \"note\": \"…\"}}: one to three sentences to them, saying \
         what you changed and why. Keep the updates that are there.\n\n\
         If nothing needs changing, save nothing. Only if a word would really help them (a \
         long run well done, a missed week to come back from gently), add just that note to \
         updates, with the sessions as they are.\n\n\
         Finish with one short sentence saying what you did. Write in the language the plan \
         is in.\n\nThe content is {help}"
    );

    let mut prompt = format!("Today is {}.", today.format("%A %-d %B %Y"));
    if let Some(race) = race_day(content) {
        let to_go = (race - today).num_days();
        prompt.push_str(&format!(
            " The plan: {}, toward {} on {} ({}).",
            plan.title,
            content["goal"]["race"].as_str().unwrap_or("the race"),
            short(race),
            match to_go {
                ..0 => "the race is run".to_string(),
                0 => "race day".to_string(),
                1 => "tomorrow".to_string(),
                n => format!("{n} days to go"),
            }
        ));
    }
    prompt.push_str("\n\nRun since the last look:\n");
    for d in runs {
        prompt.push_str(&format!(
            "- {}: {} (Strava activity {}), done as {}\n",
            short(d.workout.day),
            ran(&d.workout),
            d.workout.id,
            planned(&d.session),
        ));
    }
    let since = today - Duration::days(MISSED_FOR);
    let missed: Vec<String> = content["weeks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|w| w["sessions"].as_array())
        .flatten()
        .filter(|s| {
            s["status"] == "missed"
                && s["date"]
                    .as_str()
                    .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
                    .is_some_and(|d| d >= since)
        })
        .map(|s| format!("- {}\n", planned(s)))
        .collect();
    if missed.is_empty() {
        prompt.push_str("\nNothing missed in the last two weeks.");
    } else {
        prompt.push_str("\nMissed in the last two weeks:\n");
        prompt.push_str(&missed.concat());
    }
    (system, prompt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approvals;
    use crate::artifacts::{Notifier, Runner};
    use async_trait::async_trait;
    use niles_artifacts::{ArtifactStore, MemoryArtifacts, NewArtifact};
    use niles_core::{Event, EventBus};
    use niles_tools::ToolRegistry;
    use serde_json::json;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    const MAJSE: &str = "person:majse";
    const TZ: chrono_tz::Tz = chrono_tz::Europe::Copenhagen;

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    /// What the fake coach does with the plan it reads.
    enum Does {
        Nothing,
        Saves(fn(&mut Value)),
    }

    #[derive(Default)]
    struct FakeCoach {
        off: bool,
        does: Mutex<VecDeque<Does>>,
        prompts: Mutex<Vec<(String, String)>>,
        /// What each save it tried was answered.
        saves: Mutex<Vec<Result<(), String>>>,
        looked: Mutex<usize>,
    }

    impl FakeCoach {
        fn doing(does: Vec<Does>) -> Arc<Self> {
            Arc::new(Self {
                does: Mutex::new(does.into()),
                ..Self::default()
            })
        }
    }

    async fn call(
        tools: &ToolRegistry,
        asker: &Asker,
        name: &str,
        args: Value,
    ) -> Result<Value, String> {
        approvals::ASKER
            .scope(
                asker.clone(),
                crate::assistant::tests_support::as_assistant(
                    "claude",
                    tools.execute(&niles_llm::ToolCall {
                        id: String::new(),
                        name: name.into(),
                        arguments: args,
                    }),
                ),
            )
            .await
            .map_err(|e| e.to_string())
    }

    #[async_trait]
    impl Runner for FakeCoach {
        fn available(&self) -> Option<&'static str> {
            (!self.off).then_some("claude")
        }

        async fn run(
            &self,
            asker: Asker,
            system: &str,
            prompt: &str,
            tools: ToolRegistry,
        ) -> Result<(&'static str, String), String> {
            self.prompts
                .lock()
                .unwrap()
                .push((system.into(), prompt.into()));
            let does = self
                .does
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Does::Nothing);
            if let Does::Saves(change) = does {
                let got = call(&tools, &asker, "artifact_get", json!({ "id": 1 }))
                    .await
                    .unwrap();
                let mut content = got["content"].clone();
                change(&mut content);
                let saved = call(
                    &tools,
                    &asker,
                    "artifact_update",
                    json!({ "id": 1, "content": content, "reason": "Eased next week" }),
                )
                .await
                .map(|_| ());
                self.saves.lock().unwrap().push(saved);
            }
            *self.looked.lock().unwrap() += 1;
            Ok(("claude", "Looked at it.".into()))
        }
    }

    #[derive(Default)]
    struct Told(Mutex<Vec<(i64, String)>>);

    #[async_trait]
    impl Notifier for Told {
        async fn finished(&self, _person: &str, _artifact: &Artifact) {}

        async fn changed(&self, artifact: &Artifact, note: &str) {
            self.0.lock().unwrap().push((artifact.id, note.into()));
        }
    }

    struct Fixture {
        artifacts: Arc<Artifacts>,
        store: Arc<MemoryArtifacts>,
        coach: Arc<FakeCoach>,
        told: Arc<Told>,
        bus: EventBus,
    }

    fn fixture(coach: Arc<FakeCoach>) -> Fixture {
        let store = Arc::new(MemoryArtifacts::new());
        let told = Arc::new(Told::default());
        let bus = EventBus::default();
        let artifacts = Arc::new(
            Artifacts::new(
                store.clone(),
                bus.clone(),
                coach.clone(),
                told.clone(),
                None,
                Box::new(|_| {}),
            )
            .with_training(Box::new(|_| {}), TZ),
        );
        Fixture {
            artifacts,
            store,
            coach,
            told,
            bus,
        }
    }

    /// Her plan: an easy run on Tuesday 13 October, intervals Thursday,
    /// a long run Sunday; next week an easy run and the race.
    fn plan() -> Value {
        json!({
            "goal": { "race": "Hellerup 10K", "distance_km": 10, "date": "2026-10-25" },
            "weeks": [
                { "starts": "2026-10-12", "sessions": [
                    { "date": "2026-10-13", "type": "easy", "title": "Easy run", "distance_km": 6, "pace": "6:30" },
                    { "date": "2026-10-15", "type": "intervals", "title": "6 × 400 m", "duration_min": 40 },
                    { "date": "2026-10-18", "type": "long", "title": "Long run", "distance_km": 10 }
                ]},
                { "starts": "2026-10-19", "sessions": [
                    { "date": "2026-10-20", "type": "easy", "title": "Easy run", "distance_km": 5 },
                    { "date": "2026-10-25", "type": "race", "title": "Race day", "distance_km": 10 }
                ]}
            ]
        })
    }

    async fn made(f: &Fixture, kind: &str, content: Value) -> Artifact {
        f.store
            .create(
                MAJSE,
                NewArtifact {
                    kind: kind.into(),
                    title: "Hellerup plan".into(),
                    summary: String::new(),
                    notify: true,
                    status: Status::Ready,
                    status_text: None,
                    job_id: None,
                    brief: Some("a 10K on the 25th".into()),
                    conversation_id: None,
                    first: Some(NewRevision {
                        content: kinds::validate(kind, &content).unwrap(),
                        reason: "First draft".into(),
                        author: Author::Claude,
                        by: None,
                        summary: None,
                        page: niles_artifacts::PageChange::Keep,
                    }),
                },
            )
            .await
            .unwrap()
    }

    /// Her run on Tuesday morning, Copenhagen time.
    fn tuesday_run() -> Activity {
        niles_training::strava::activity(&json!({
            "id": 12345678901u64,
            "name": "Morning Run",
            "sport_type": "Run",
            "start_date": "2026-10-13T05:30:00Z",
            "distance": 6200.0,
            "moving_time": 2340,
            "elapsed_time": 2400
        }))
        .unwrap()
    }

    async fn content(f: &Fixture, n: i32) -> (Value, Author, String) {
        let r = f.store.revision(MAJSE, 1, n).await.unwrap().unwrap();
        (r.content, r.author, r.reason)
    }

    /// Until the coach has looked `n` times, and its look is kept.
    async fn looked(f: &Fixture, n: usize) {
        for _ in 0..200 {
            if *f.coach.looked.lock().unwrap() >= n
                && f.artifacts.reviewing.lock().unwrap().is_empty()
            {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("the coach did not look");
    }

    #[tokio::test]
    async fn a_new_run_marks_its_session_done_in_a_version_by_niles() {
        let f = fixture(Arc::new(FakeCoach {
            off: true,
            ..FakeCoach::default()
        }));
        made(&f, TRAINING_PLAN, plan()).await;
        let mut events = f.bus.subscribe();
        f.artifacts
            .follow(MAJSE, &[tuesday_run()], day("2026-10-13"))
            .await;

        let (v, author, reason) = content(&f, 2).await;
        assert_eq!(author, Author::Niles);
        assert_eq!(reason, "Done: Easy run on Tue 13 Oct");
        let s = &v["weeks"][0]["sessions"][0];
        assert_eq!(s["status"], "done");
        assert_eq!(s["activity"], "12345678901");
        assert_eq!(
            s["actual"],
            json!({ "distance_km": 6.2, "duration_min": 40.0, "pace": "6:17" })
        );
        let a = f.store.get(MAJSE, 1).await.unwrap().unwrap();
        assert_eq!(a.preview.lines[1].text, "Thu 15 Oct · 6 × 400 m · 40 min");
        assert!(matches!(
            events.try_recv(),
            Ok(Event::ArtifactChanged { id: 1, .. })
        ));
    }

    #[tokio::test]
    async fn a_plan_follows_its_owners_runs_and_never_those_it_is_shared_with() {
        let f = fixture(Arc::new(FakeCoach {
            off: true,
            ..FakeCoach::default()
        }));
        made(&f, TRAINING_PLAN, plan()).await;
        let mark = "person:mark";
        f.store
            .set_share(MAJSE, 1, mark, Some(niles_artifacts::Level::Edit))
            .await
            .unwrap();
        f.artifacts
            .follow(mark, &[tuesday_run()], day("2026-10-13"))
            .await;
        assert_eq!(
            f.store
                .get(MAJSE, 1)
                .await
                .unwrap()
                .unwrap()
                .current_revision,
            1,
            "his run is not her session"
        );
        f.artifacts
            .follow(MAJSE, &[tuesday_run()], day("2026-10-13"))
            .await;
        assert_eq!(
            f.store
                .get(MAJSE, 1)
                .await
                .unwrap()
                .unwrap()
                .current_revision,
            2
        );
    }

    #[tokio::test]
    async fn a_run_already_marked_and_a_day_not_over_save_nothing() {
        let f = fixture(Arc::new(FakeCoach {
            off: true,
            ..FakeCoach::default()
        }));
        made(&f, TRAINING_PLAN, plan()).await;
        let today = day("2026-10-14");
        f.artifacts.follow(MAJSE, &[tuesday_run()], today).await;
        f.artifacts.follow(MAJSE, &[tuesday_run()], today).await;
        f.artifacts.follow(MAJSE, &[], today).await;
        let a = f.store.get(MAJSE, 1).await.unwrap().unwrap();
        assert_eq!(a.current_revision, 2);
    }

    #[tokio::test]
    async fn the_days_gone_by_are_missed_with_no_run_at_all() {
        let f = fixture(FakeCoach::doing(vec![]));
        made(&f, TRAINING_PLAN, plan()).await;
        f.artifacts.follow(MAJSE, &[], day("2026-10-17")).await;
        let (v, author, reason) = content(&f, 2).await;
        assert_eq!(author, Author::Niles);
        assert_eq!(v["weeks"][0]["sessions"][1]["status"], "missed");
        assert!(
            reason.starts_with("Missed: Easy run on Tue 13 Oct"),
            "{reason}"
        );
        assert!(
            f.coach.prompts.lock().unwrap().is_empty(),
            "nobody looks at a plan for what was not run"
        );
    }

    #[tokio::test]
    async fn only_her_plans_still_going_are_followed() {
        let f = fixture(FakeCoach::doing(vec![]));
        made(&f, TRAINING_PLAN, plan()).await;
        let done_with = day("2026-10-29");
        f.artifacts.follow(MAJSE, &[tuesday_run()], done_with).await;
        assert_eq!(
            f.store
                .get(MAJSE, 1)
                .await
                .unwrap()
                .unwrap()
                .current_revision,
            1,
            "four days after the race"
        );

        let doc = made(&f, "document", json!({ "markdown": "- [ ] run" })).await;
        f.store
            .change(
                MAJSE,
                1,
                niles_artifacts::Changes {
                    archived: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        f.artifacts
            .follow(MAJSE, &[tuesday_run()], day("2026-10-13"))
            .await;
        assert_eq!(
            f.store
                .get(MAJSE, 1)
                .await
                .unwrap()
                .unwrap()
                .current_revision,
            1
        );
        assert_eq!(
            f.store
                .get(MAJSE, doc.id)
                .await
                .unwrap()
                .unwrap()
                .current_revision,
            1
        );
    }

    #[tokio::test]
    async fn a_run_has_the_coach_look_with_the_run_beside_the_plan() {
        let f = fixture(FakeCoach::doing(vec![Does::Nothing]));
        made(&f, TRAINING_PLAN, plan()).await;
        f.artifacts
            .follow(MAJSE, &[tuesday_run()], day("2026-10-13"))
            .await;
        looked(&f, 1).await;

        let (system, prompt) = f.coach.prompts.lock().unwrap()[0].clone();
        assert!(system.contains("artifact 1"), "{system}");
        assert!(
            system.contains("Never a session before 2026-10-13"),
            "{system}"
        );
        assert!(prompt.contains("Tuesday 13 October 2026"), "{prompt}");
        assert!(prompt.contains("12 days to go"), "{prompt}");
        assert!(
            prompt.contains(
                "- Tue 13 Oct: 6.2 km in 40 min at 6:17 /km (Strava activity 12345678901), done \
                 as Easy run (easy) on Tue 13 Oct: 6 km, 6:30 /km"
            ),
            "{prompt}"
        );
        assert!(prompt.contains("Nothing missed"), "{prompt}");

        assert_eq!(
            f.store
                .get(MAJSE, 1)
                .await
                .unwrap()
                .unwrap()
                .current_revision,
            2
        );
        assert!(f.told.0.lock().unwrap().is_empty(), "nothing changed");
    }

    #[tokio::test]
    async fn the_coach_s_edits_keep_the_past_too() {
        let f = fixture(FakeCoach::doing(vec![]));
        let plan = made(&f, TRAINING_PLAN, plan()).await;
        let tools = f
            .artifacts
            .job_tools(plan.id, TRAINING_PLAN, Some(day("2026-10-15")));
        assert!(tools.has("artifact_edit"));
        assert!(!tools.has("artifact_edit_page"));
        let asker = Asker {
            person: MAJSE.into(),
            slug: Some("majse".into()),
            satellite: None,
            conversation: None,
        };
        let e = call(
            &tools,
            &asker,
            "artifact_edit",
            json!({
                "id": plan.id,
                "edits": [{ "op": "set", "path": "/weeks/0/sessions/0/distance_km", "value": 8 }],
                "reason": "Longer",
            }),
        )
        .await
        .unwrap_err();
        assert!(e.contains("before today"), "{e}");
        call(
            &tools,
            &asker,
            "artifact_edit",
            json!({
                "id": plan.id,
                "edits": [{ "op": "set", "path": "/weeks/0/sessions/2/distance_km", "value": 8 }],
                "reason": "Longer",
            }),
        )
        .await
        .unwrap();
        let a = f.store.get(MAJSE, plan.id).await.unwrap().unwrap();
        assert_eq!(a.current_revision, 2, "the planned session changed");
    }

    #[tokio::test]
    async fn a_look_that_changes_what_is_coming_tells_her_with_its_note() {
        let f = fixture(FakeCoach::doing(vec![Does::Saves(|p| {
            p["weeks"][1]["sessions"][0]["distance_km"] = json!(4);
            p["updates"] = json!([{ "date": "2026-10-13", "note": "A little less next week." }]);
        })]));
        made(&f, TRAINING_PLAN, plan()).await;
        f.artifacts
            .follow(MAJSE, &[tuesday_run()], day("2026-10-13"))
            .await;
        looked(&f, 1).await;
        assert_eq!(*f.coach.saves.lock().unwrap(), vec![Ok(())]);
        let (v, author, _) = content(&f, 3).await;
        assert_eq!(author, Author::Claude);
        assert_eq!(v["weeks"][1]["sessions"][0]["distance_km"], 4.0);
        assert_eq!(v["weeks"][0]["sessions"][0]["status"], "done");
        assert_eq!(
            *f.told.0.lock().unwrap(),
            vec![(1, "A little less next week.".to_string())]
        );
    }

    #[tokio::test]
    async fn a_look_that_only_leaves_a_note_tells_nobody() {
        let f = fixture(FakeCoach::doing(vec![Does::Saves(|p| {
            p["updates"] = json!([{ "date": "2026-10-13", "note": "Lovely pace." }]);
        })]));
        made(&f, TRAINING_PLAN, plan()).await;
        f.artifacts
            .follow(MAJSE, &[tuesday_run()], day("2026-10-13"))
            .await;
        looked(&f, 1).await;
        let (v, _, _) = content(&f, 3).await;
        assert_eq!(v["updates"][0]["note"], "Lovely pace.");
        assert!(f.told.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_runs_and_the_coach_change_the_data_and_the_page_stays() {
        let f = fixture(FakeCoach::doing(vec![Does::Saves(|p| {
            p["weeks"][1]["sessions"][0]["distance_km"] = json!(4);
            p["updates"] = json!([{ "date": "2026-10-13", "note": "A little less next week." }]);
        })]));
        let plan = made(&f, TRAINING_PLAN, plan()).await;
        let paged = f
            .store
            .set_page(
                MAJSE,
                plan.id,
                niles_artifacts::PageChange::New("<h1 id=t></h1>".into()),
                "A page for it".into(),
                Author::Claude,
                None,
            )
            .await
            .unwrap()
            .unwrap();
        assert!(
            !f.artifacts
                .job_tools(plan.id, TRAINING_PLAN, Some(day("2026-10-13")))
                .has("artifact_set_page"),
            "the coach has no way to touch the page"
        );
        assert!(
            !f.artifacts
                .job_tools(plan.id, TRAINING_PLAN, Some(day("2026-10-13")))
                .has("artifact_edit_page"),
            "the coach has no way to touch the page"
        );
        f.artifacts
            .follow(MAJSE, &[tuesday_run()], day("2026-10-13"))
            .await;
        looked(&f, 1).await;
        let now = f.store.get(MAJSE, plan.id).await.unwrap().unwrap();
        assert_eq!(now.current_revision, 4, "Niles's version, then the coach's");
        assert_eq!(now.page, paged.page);
        let (v, author, _) = content(&f, 4).await;
        assert_eq!(author, Author::Claude);
        assert_eq!(v["weeks"][1]["sessions"][0]["distance_km"], 4.0);
    }

    #[tokio::test]
    async fn the_coach_cannot_rewrite_the_run_or_mark_a_session() {
        let f = fixture(FakeCoach::doing(vec![
            Does::Saves(|p| p["weeks"][0]["sessions"][1]["status"] = json!("skipped")),
            Does::Saves(|p| p["weeks"][0]["sessions"][0]["actual"]["distance_km"] = json!(10)),
        ]));
        made(&f, TRAINING_PLAN, plan()).await;
        f.artifacts
            .follow(MAJSE, &[tuesday_run()], day("2026-10-13"))
            .await;
        looked(&f, 1).await;
        // A run on Thursday too, after the first look.
        let thursday = niles_training::strava::activity(&json!({
            "id": 2, "sport_type": "Run", "start_date": "2026-10-15T16:00:00Z",
            "distance": 5000.0, "moving_time": 1500
        }))
        .unwrap();
        f.artifacts
            .follow(MAJSE, &[thursday], day("2026-10-15"))
            .await;
        looked(&f, 2).await;

        let saves = f.coach.saves.lock().unwrap().clone();
        assert!(
            saves[0].as_ref().unwrap_err().contains("is marked skipped"),
            "{saves:?}"
        );
        assert!(
            saves[1]
                .as_ref()
                .unwrap_err()
                .contains("stay exactly as they are"),
            "{saves:?}"
        );
        let a = f.store.get(MAJSE, 1).await.unwrap().unwrap();
        assert_eq!(
            a.current_revision, 3,
            "the two runs, and nothing of the coach's"
        );
        assert!(f.told.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn without_an_assistant_or_a_free_job_nobody_looks() {
        let f = fixture(Arc::new(FakeCoach {
            off: true,
            ..FakeCoach::default()
        }));
        made(&f, TRAINING_PLAN, plan()).await;
        f.artifacts
            .follow(MAJSE, &[tuesday_run()], day("2026-10-13"))
            .await;
        assert!(f.coach.prompts.lock().unwrap().is_empty());

        let f = fixture(FakeCoach::doing(vec![]));
        let a = made(&f, TRAINING_PLAN, plan()).await;
        let held = (f.artifacts.take_slot(MAJSE), f.artifacts.take_slot(MAJSE));
        let saved = f.store.get(MAJSE, a.id).await.unwrap().unwrap();
        let runs = [Done {
            session: json!({}),
            workout: workout(&tuesday_run(), &TZ).unwrap(),
        }];
        assert!(
            !f.artifacts
                .review(saved.clone(), &plan(), &runs, day("2026-10-13"))
        );
        drop(held);
        assert!(
            f.artifacts
                .review(saved.clone(), &plan(), &runs, day("2026-10-13"))
        );
        assert!(
            !f.artifacts.review(saved, &plan(), &runs, day("2026-10-13")),
            "one look at a time"
        );
    }

    #[test]
    fn a_walk_is_no_session_and_a_ride_is_cross_training() {
        let of = |sport: &str| {
            let a = niles_training::strava::activity(&json!({
                "id": 3, "sport_type": sport, "start_date": "2026-10-13T22:30:00Z",
                "distance": 10000.0, "moving_time": 3000
            }))
            .unwrap();
            workout(&a, &TZ)
        };
        assert_eq!(of("Walk"), None);
        let ride = of("Ride").unwrap();
        assert_eq!(ride.sport, Sport::Cross);
        assert_eq!(ride.pace, None);
        let run = of("Run").unwrap();
        assert_eq!(
            run.day,
            day("2026-10-14"),
            "half past midnight in Copenhagen"
        );
        assert_eq!(run.pace.as_deref(), Some("5:00"));
    }
}
