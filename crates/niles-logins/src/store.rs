//! Where each person's saved logins are kept: Postgres in the house
//! (niles-db), in memory for tests.
//!
//! Every method is scoped by the person's key, so another person's id
//! behaves exactly like one that was never kept. The sealed username and
//! password, and the browser profile, belong to the row and leave with
//! it.

use crate::activity::{ACTIVITY_SHOWN, Activity, Context, Used};
use crate::error::Result;
use crate::model::{Credentials, KeyChange, Login, LoginEdit, NewLogin, Secret};
use crate::origin::Origin;
use crate::totp::Authenticator;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::sync::Mutex;

/// How long a browser profile outlives its login's last use.
pub const PROFILE_DAYS: i64 = 30;

/// Each person's saved logins. Another person's id answers like a
/// missing one on every method, and changes nothing.
#[async_trait]
pub trait LoginStore: Send + Sync {
    /// The person's own, by label (case-insensitive) then id.
    async fn list(&self, person: &str) -> Result<Vec<Login>>;
    async fn get(&self, person: &str, id: i64) -> Result<Option<Login>>;
    async fn add(&self, person: &str, login: &NewLogin) -> Result<Login>;
    /// The changed login, or `None` when the id isn't the person's.
    ///
    /// An edit that changes the username, or the set of origins compared
    /// as sorted sets, forgets the browser profile: the profile was
    /// signed in as that account on those sites. A label-only or
    /// password-only edit keeps it.
    async fn update(&self, person: &str, id: i64, edit: &LoginEdit) -> Result<Option<Login>>;
    /// The login, its sealed values and its browser profile. False when
    /// the id isn't the person's.
    async fn remove(&self, person: &str, id: i64) -> Result<bool>;
    /// Forgets the browser profile, keeps the login. True even when
    /// there was no profile; false when the id isn't the person's.
    async fn sign_out(&self, person: &str, id: i64) -> Result<bool>;
    /// The sealed values of one login, opened. This is for Niles's
    /// browser to fill in, never for a response or the model.
    async fn credentials(&self, person: &str, id: i64) -> Result<Option<Credentials>>;
    /// Every saved username and password of the person, opened, for
    /// scrubbing what Niles's browser sends the model, the log and
    /// tracing; never for a response or the model.
    async fn secrets(&self, person: &str) -> Result<Vec<Credentials>>;
    /// A browser profile, sealed; also marks the login used now. False
    /// when the id isn't the person's.
    async fn save_profile(&self, person: &str, id: i64, profile: &Secret) -> Result<bool>;
    /// The person's sealed profile for the login, if one is fresh.
    /// `None`, and the profile forgotten, when its login was last used
    /// more than [`PROFILE_DAYS`] ago.
    async fn profile(&self, person: &str, id: i64) -> Result<Option<Secret>>;
    /// Notes the login was used at `at` (a sign-in, or its profile
    /// kept); false when the id isn't the person's.
    async fn used(&self, person: &str, id: i64, at: DateTime<Utc>) -> Result<bool>;
    /// Forgets every person's profile whose login was last used before
    /// `before`, or was never used and saved before it; answers how many
    /// went.
    async fn drop_stale_profiles(&self, before: DateTime<Utc>) -> Result<u64>;
    /// The login's authenticator key, ready to make a code: for Niles's
    /// browser, never for a response or the model.
    async fn authenticator(&self, person: &str, id: i64) -> Result<Option<Authenticator>>;
    /// Notes one use of the login; false when the id isn't the person's.
    /// Adding, editing, signing out and dropping a profile are noted by
    /// the store itself.
    async fn record(&self, person: &str, id: i64, used: Used, context: &Context) -> Result<bool>;
    /// The login's latest uses, newest first, at most
    /// [`ACTIVITY_SHOWN`]; `None` when the id isn't the person's.
    async fn activity(&self, person: &str, id: i64) -> Result<Option<Vec<Activity>>>;
    /// Forgets every use older than `before`; answers how many went.
    async fn drop_old_activity(&self, before: DateTime<Utc>) -> Result<u64>;
}

struct Row {
    person: String,
    id: i64,
    label: String,
    origins: Vec<Origin>,
    credentials: Credentials,
    last_used: Option<DateTime<Utc>>,
    profile: Option<Secret>,
    profile_saved: Option<DateTime<Utc>>,
    authenticator: Option<Secret>,
    activity: Vec<Activity>,
}

impl Row {
    fn note(&mut self, used: Used, context: &Context) {
        self.activity.push(Activity {
            at: Utc::now(),
            used,
            site: context.site.clone(),
            conversation: context.conversation,
        });
    }

    /// Forgets the profile, noting it when there was one.
    fn drop_profile(&mut self) {
        if self.profile.take().is_some() {
            self.note(Used::ProfileDropped, &Context::default());
        }
        self.profile_saved = None;
    }
}

struct Kept {
    rows: Vec<Row>,
    /// The next id; 1 is the first.
    next: i64,
}

impl Default for Kept {
    fn default() -> Self {
        Kept {
            rows: Vec::new(),
            next: 1,
        }
    }
}

/// The in-memory store, for tests and for a Niles without a database.
#[derive(Default)]
pub struct MemoryLogins {
    kept: Mutex<Kept>,
}

impl MemoryLogins {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Kept> {
        self.kept.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn fresh(row: &Row, now: DateTime<Utc>) -> bool {
        row.profile.is_some()
            && row
                .last_used
                .or(row.profile_saved)
                .is_some_and(|at| at >= now - chrono::Duration::days(PROFILE_DAYS))
    }

    fn login(row: &Row) -> Login {
        Login {
            id: row.id,
            label: row.label.clone(),
            origins: row.origins.clone(),
            username: row.credentials.username.expose().to_string(),
            last_used: row.last_used,
            signed_in: Self::fresh(row, Utc::now()),
            authenticator: row.authenticator.is_some(),
        }
    }
}

#[async_trait]
impl LoginStore for MemoryLogins {
    async fn list(&self, person: &str) -> Result<Vec<Login>> {
        let kept = self.lock();
        let mut rows: Vec<&Row> = kept.rows.iter().filter(|r| r.person == person).collect();
        rows.sort_by_key(|r| (r.label.to_lowercase(), r.id));
        Ok(rows.iter().map(|r| Self::login(r)).collect())
    }

    async fn get(&self, person: &str, id: i64) -> Result<Option<Login>> {
        Ok(self
            .lock()
            .rows
            .iter()
            .find(|r| r.person == person && r.id == id)
            .map(Self::login))
    }

    async fn add(&self, person: &str, login: &NewLogin) -> Result<Login> {
        let mut kept = self.lock();
        let row = Row {
            person: person.to_string(),
            id: kept.next,
            label: login.label.clone(),
            origins: login.origins.clone(),
            credentials: login.credentials.clone(),
            last_used: None,
            profile: None,
            profile_saved: None,
            authenticator: login.authenticator.clone(),
            activity: Vec::new(),
        };
        kept.next += 1;
        kept.rows.push(row);
        let row = kept.rows.last_mut().expect("just pushed");
        row.note(Used::Added, &Context::default());
        Ok(Self::login(row))
    }

    async fn update(&self, person: &str, id: i64, edit: &LoginEdit) -> Result<Option<Login>> {
        let mut kept = self.lock();
        let Some(row) = kept
            .rows
            .iter_mut()
            .find(|r| r.person == person && r.id == id)
        else {
            return Ok(None);
        };
        let mut before = row.origins.clone();
        before.sort();
        let mut after = edit.origins.clone();
        after.sort();
        row.note(Used::Edited, &Context::default());
        if row.credentials.username != edit.username || before != after {
            row.drop_profile();
        }
        row.label = edit.label.clone();
        row.origins = edit.origins.clone();
        row.credentials.username = edit.username.clone();
        if let Some(password) = &edit.password {
            row.credentials.password = password.clone();
        }
        match &edit.authenticator {
            KeyChange::Keep => {}
            KeyChange::Set(key) => row.authenticator = Some(key.clone()),
            KeyChange::Remove => row.authenticator = None,
        }
        Ok(Some(Self::login(row)))
    }

    async fn remove(&self, person: &str, id: i64) -> Result<bool> {
        let mut kept = self.lock();
        let before = kept.rows.len();
        kept.rows.retain(|r| !(r.person == person && r.id == id));
        Ok(kept.rows.len() != before)
    }

    async fn sign_out(&self, person: &str, id: i64) -> Result<bool> {
        let mut kept = self.lock();
        let Some(row) = kept
            .rows
            .iter_mut()
            .find(|r| r.person == person && r.id == id)
        else {
            return Ok(false);
        };
        row.profile = None;
        row.profile_saved = None;
        row.note(Used::SignedOut, &Context::default());
        Ok(true)
    }

    async fn credentials(&self, person: &str, id: i64) -> Result<Option<Credentials>> {
        Ok(self
            .lock()
            .rows
            .iter()
            .find(|r| r.person == person && r.id == id)
            .map(|r| r.credentials.clone()))
    }

    async fn secrets(&self, person: &str) -> Result<Vec<Credentials>> {
        Ok(self
            .lock()
            .rows
            .iter()
            .filter(|r| r.person == person)
            .map(|r| r.credentials.clone())
            .collect())
    }

    async fn save_profile(&self, person: &str, id: i64, profile: &Secret) -> Result<bool> {
        let mut kept = self.lock();
        let Some(row) = kept
            .rows
            .iter_mut()
            .find(|r| r.person == person && r.id == id)
        else {
            return Ok(false);
        };
        let now = Utc::now();
        row.profile = Some(profile.clone());
        row.profile_saved = Some(now);
        row.last_used = Some(now);
        Ok(true)
    }

    async fn profile(&self, person: &str, id: i64) -> Result<Option<Secret>> {
        let mut kept = self.lock();
        let Some(row) = kept
            .rows
            .iter_mut()
            .find(|r| r.person == person && r.id == id)
        else {
            return Ok(None);
        };
        if !Self::fresh(row, Utc::now()) {
            row.drop_profile();
            return Ok(None);
        }
        Ok(row.profile.clone())
    }

    async fn used(&self, person: &str, id: i64, at: DateTime<Utc>) -> Result<bool> {
        let mut kept = self.lock();
        let Some(row) = kept
            .rows
            .iter_mut()
            .find(|r| r.person == person && r.id == id)
        else {
            return Ok(false);
        };
        row.last_used = Some(at);
        Ok(true)
    }

    async fn drop_stale_profiles(&self, before: DateTime<Utc>) -> Result<u64> {
        let mut kept = self.lock();
        let mut dropped = 0;
        for row in &mut kept.rows {
            let stale = row.profile.is_some()
                && row
                    .last_used
                    .or(row.profile_saved)
                    .is_some_and(|at| at < before);
            if stale {
                row.drop_profile();
                dropped += 1;
            }
        }
        Ok(dropped)
    }

    async fn authenticator(&self, person: &str, id: i64) -> Result<Option<Authenticator>> {
        let key = self
            .lock()
            .rows
            .iter()
            .find(|r| r.person == person && r.id == id)
            .and_then(|r| r.authenticator.clone());
        key.map(|k| Authenticator::parse(k.expose())).transpose()
    }

    async fn record(&self, person: &str, id: i64, used: Used, context: &Context) -> Result<bool> {
        let mut kept = self.lock();
        let Some(row) = kept
            .rows
            .iter_mut()
            .find(|r| r.person == person && r.id == id)
        else {
            return Ok(false);
        };
        row.note(used, context);
        Ok(true)
    }

    async fn activity(&self, person: &str, id: i64) -> Result<Option<Vec<Activity>>> {
        let kept = self.lock();
        let Some(row) = kept.rows.iter().find(|r| r.person == person && r.id == id) else {
            return Ok(None);
        };
        let mut uses = row.activity.clone();
        uses.reverse();
        uses.truncate(ACTIVITY_SHOWN);
        Ok(Some(uses))
    }

    async fn drop_old_activity(&self, before: DateTime<Utc>) -> Result<u64> {
        let mut kept = self.lock();
        let mut dropped = 0;
        for row in &mut kept.rows {
            let had = row.activity.len();
            row.activity.retain(|a| a.at >= before);
            dropped += (had - row.activity.len()) as u64;
        }
        Ok(dropped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAJSE: &str = "person:majse";
    const MARK: &str = "person:mark";
    const SITES: &[&str] = &["shop.example"];

    fn new_login() -> NewLogin {
        NewLogin::new(
            "Shop",
            &[SITES[0].to_string()],
            crate::Secret::new("majse.private@example.com"),
            crate::Secret::new("correct horse battery staple 42"),
        )
        .unwrap()
    }

    fn edit() -> LoginEdit {
        LoginEdit::new(
            "Shop",
            &[SITES[0].to_string()],
            crate::Secret::new("majse.private@example.com"),
            None,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn add_then_list_gives_the_person_s_own_login_only() {
        let store = MemoryLogins::new();
        let login = store.add(MAJSE, &new_login()).await.unwrap();
        assert_eq!(login.id, 1);
        assert_eq!(login.username, "majse.private@example.com");
        assert!(!login.signed_in);
        let list = store.list(MAJSE).await.unwrap();
        assert_eq!(list.len(), 1);
        assert!(store.list(MARK).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn none_password_keeps_the_current_one_and_some_replaces_it() {
        let store = MemoryLogins::new();
        let login = store.add(MAJSE, &new_login()).await.unwrap();
        store.update(MAJSE, login.id, &edit()).await.unwrap();
        let kept = store.credentials(MAJSE, login.id).await.unwrap().unwrap();
        assert_eq!(kept.password.expose(), "correct horse battery staple 42");
        let replaced = LoginEdit::new(
            "Shop",
            &[SITES[0].to_string()],
            crate::Secret::new("majse.private@example.com"),
            Some(crate::Secret::new("a new one")),
        )
        .unwrap();
        store.update(MAJSE, login.id, &replaced).await.unwrap();
        let kept = store.credentials(MAJSE, login.id).await.unwrap().unwrap();
        assert_eq!(kept.password.expose(), "a new one");
    }

    #[tokio::test]
    async fn a_username_change_drops_the_profile_a_label_change_keeps_it() {
        let store = MemoryLogins::new();
        let login = store.add(MAJSE, &new_login()).await.unwrap();
        store
            .save_profile(MAJSE, login.id, &crate::Secret::new("cookies"))
            .await
            .unwrap();
        assert!(store.profile(MAJSE, login.id).await.unwrap().is_some());
        let renamed = LoginEdit::new(
            "Bog & Ide",
            &[SITES[0].to_string()],
            crate::Secret::new("majse.private@example.com"),
            None,
        )
        .unwrap();
        let after = store
            .update(MAJSE, login.id, &renamed)
            .await
            .unwrap()
            .unwrap();
        assert!(after.signed_in);
        let other_account = LoginEdit::new(
            "Bog & Ide",
            &[SITES[0].to_string()],
            crate::Secret::new("other@example.com"),
            None,
        )
        .unwrap();
        store.update(MAJSE, login.id, &other_account).await.unwrap();
        assert!(store.profile(MAJSE, login.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn an_origins_change_drops_the_profile_a_reorder_keeps_it() {
        let store = MemoryLogins::new();
        let both = NewLogin::new(
            "Shop",
            &["shop.example".to_string(), "mail.example".to_string()],
            crate::Secret::new("majse.private@example.com"),
            crate::Secret::new("correct horse battery staple 42"),
        )
        .unwrap();
        let login = store.add(MAJSE, &both).await.unwrap();
        store
            .save_profile(MAJSE, login.id, &crate::Secret::new("cookies"))
            .await
            .unwrap();

        // The same set in another order: the profile is kept.
        let reordered = LoginEdit::new(
            "Shop",
            &["mail.example".to_string(), "shop.example".to_string()],
            crate::Secret::new("majse.private@example.com"),
            None,
        )
        .unwrap();
        let after = store
            .update(MAJSE, login.id, &reordered)
            .await
            .unwrap()
            .unwrap();
        assert!(
            after.signed_in,
            "the sites read as a set: another order keeps the profile"
        );

        // A different set, even a smaller one: the profile is forgotten.
        let fewer = LoginEdit::new(
            "Shop",
            &["shop.example".to_string()],
            crate::Secret::new("majse.private@example.com"),
            None,
        )
        .unwrap();
        store.update(MAJSE, login.id, &fewer).await.unwrap();
        assert!(store.profile(MAJSE, login.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn remove_drops_the_profile_and_the_credentials() {
        let store = MemoryLogins::new();
        let login = store.add(MAJSE, &new_login()).await.unwrap();
        store
            .save_profile(MAJSE, login.id, &crate::Secret::new("cookies"))
            .await
            .unwrap();
        assert!(store.remove(MAJSE, login.id).await.unwrap());
        assert!(store.credentials(MAJSE, login.id).await.unwrap().is_none());
        assert!(store.profile(MAJSE, login.id).await.unwrap().is_none());
        assert!(!store.remove(MAJSE, login.id).await.unwrap());
    }

    #[tokio::test]
    async fn sign_out_keeps_the_login_and_clears_signed_in() {
        let store = MemoryLogins::new();
        let login = store.add(MAJSE, &new_login()).await.unwrap();
        store
            .save_profile(MAJSE, login.id, &crate::Secret::new("cookies"))
            .await
            .unwrap();
        assert!(store.sign_out(MAJSE, login.id).await.unwrap());
        let after = store.get(MAJSE, login.id).await.unwrap().unwrap();
        assert!(!after.signed_in);
        assert_eq!(after.label, "Shop");
        assert!(!store.sign_out(MAJSE, 999).await.unwrap());
        assert!(!store.sign_out(MARK, login.id).await.unwrap());
    }

    #[tokio::test]
    async fn another_person_s_id_answers_like_a_missing_one() {
        let store = MemoryLogins::new();
        let login = store.add(MAJSE, &new_login()).await.unwrap();
        store
            .save_profile(MAJSE, login.id, &crate::Secret::new("cookies"))
            .await
            .unwrap();
        assert!(store.get(MARK, login.id).await.unwrap().is_none());
        assert!(store.credentials(MARK, login.id).await.unwrap().is_none());
        assert!(store.profile(MARK, login.id).await.unwrap().is_none());
        assert!(!store.remove(MARK, login.id).await.unwrap());
        assert!(!store.sign_out(MARK, login.id).await.unwrap());
        assert!(
            !store
                .save_profile(MARK, login.id, &crate::Secret::new("x"))
                .await
                .unwrap()
        );
        assert!(
            store
                .update(MARK, login.id, &edit())
                .await
                .unwrap()
                .is_none()
        );
        // Nothing changed for the owner.
        let after = store.get(MAJSE, login.id).await.unwrap().unwrap();
        assert_eq!(after.label, "Shop");
        assert!(after.signed_in);
        assert!(store.list(MARK).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn list_sorts_by_label_case_insensitively_then_id() {
        let store = MemoryLogins::new();
        for (i, label) in ["beta", "Alpha", "alpha"].iter().enumerate() {
            let login = NewLogin::new(
                label,
                &[SITES[0].to_string()],
                crate::Secret::new(format!("user{i}")),
                crate::Secret::new("password"),
            )
            .unwrap();
            store.add(MAJSE, &login).await.unwrap();
        }
        let names: Vec<String> = store
            .list(MAJSE)
            .await
            .unwrap()
            .into_iter()
            .map(|l| l.label)
            .collect();
        assert_eq!(names, ["Alpha", "alpha", "beta"]);
    }

    #[tokio::test]
    async fn a_profile_unused_for_30_days_is_dropped() {
        let store = MemoryLogins::new();
        let login = store.add(MAJSE, &new_login()).await.unwrap();
        store
            .save_profile(MAJSE, login.id, &crate::Secret::new("cookies"))
            .await
            .unwrap();
        let now = Utc::now();
        store
            .used(MAJSE, login.id, now - chrono::Duration::days(31))
            .await
            .unwrap();
        assert!(store.profile(MAJSE, login.id).await.unwrap().is_none());
        let after = store.get(MAJSE, login.id).await.unwrap().unwrap();
        assert!(!after.signed_in);
        // A second read is still none: the profile was forgotten, not
        // just refused.
        assert!(store.profile(MAJSE, login.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_profile_used_within_30_days_is_kept() {
        let store = MemoryLogins::new();
        let login = store.add(MAJSE, &new_login()).await.unwrap();
        store
            .save_profile(MAJSE, login.id, &crate::Secret::new("cookies"))
            .await
            .unwrap();
        let now = Utc::now();
        store
            .used(MAJSE, login.id, now - chrono::Duration::days(29))
            .await
            .unwrap();
        assert!(store.profile(MAJSE, login.id).await.unwrap().is_some());
        assert!(store.get(MAJSE, login.id).await.unwrap().unwrap().signed_in);
    }

    #[tokio::test]
    async fn saving_a_profile_marks_the_login_used() {
        let store = MemoryLogins::new();
        let login = store.add(MAJSE, &new_login()).await.unwrap();
        store
            .save_profile(MAJSE, login.id, &crate::Secret::new("cookies"))
            .await
            .unwrap();
        let after = store.get(MAJSE, login.id).await.unwrap().unwrap();
        assert!(after.last_used.is_some());
    }

    #[tokio::test]
    async fn the_sweep_drops_only_stale_profiles() {
        let store = MemoryLogins::new();
        let stale = store.add(MAJSE, &new_login()).await.unwrap();
        let fresh = store
            .add(
                MAJSE,
                &NewLogin::new(
                    "Mail",
                    &["mail.example".to_string()],
                    crate::Secret::new("majse@example.com"),
                    crate::Secret::new("another password"),
                )
                .unwrap(),
            )
            .await
            .unwrap();
        store
            .save_profile(MAJSE, stale.id, &crate::Secret::new("stale cookies"))
            .await
            .unwrap();
        store
            .save_profile(MAJSE, fresh.id, &crate::Secret::new("fresh cookies"))
            .await
            .unwrap();
        let now = Utc::now();
        store
            .used(MAJSE, stale.id, now - chrono::Duration::days(31))
            .await
            .unwrap();
        let dropped = store
            .drop_stale_profiles(now - chrono::Duration::days(PROFILE_DAYS))
            .await
            .unwrap();
        assert_eq!(dropped, 1);
        assert!(store.profile(MAJSE, stale.id).await.unwrap().is_none());
        assert!(store.profile(MAJSE, fresh.id).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn secrets_are_the_persons_own() {
        let store = MemoryLogins::new();
        store.add(MAJSE, &new_login()).await.unwrap();
        let marks = NewLogin::new(
            "Mail",
            &["mail.example".to_string()],
            crate::Secret::new("mark@example.com"),
            crate::Secret::new("mark's password"),
        )
        .unwrap();
        store.add(MARK, &marks).await.unwrap();
        let mark_secrets = store.secrets(MARK).await.unwrap();
        assert_eq!(mark_secrets.len(), 1);
        assert_eq!(mark_secrets[0].username.expose(), "mark@example.com");
        let majse_secrets = store.secrets(MAJSE).await.unwrap();
        assert_eq!(majse_secrets.len(), 1);
        assert_eq!(
            majse_secrets[0].username.expose(),
            "majse.private@example.com"
        );
    }

    #[tokio::test]
    async fn used_refuses_another_persons_login() {
        let store = MemoryLogins::new();
        let login = store.add(MAJSE, &new_login()).await.unwrap();
        assert!(!store.used(MARK, login.id, Utc::now()).await.unwrap());
        assert!(store.get(MARK, login.id).await.unwrap().is_none());
    }

    const KEY: &str = "JBSWY3DPEHPK3PXP";

    #[tokio::test]
    async fn activity_is_the_persons_own_newest_first() {
        let store = MemoryLogins::new();
        let login = store.add(MAJSE, &new_login()).await.unwrap();
        let shop = Context {
            site: Some("https://shop.example".into()),
            conversation: Some(7),
        };
        assert!(
            store
                .record(MAJSE, login.id, Used::SignedIn, &shop)
                .await
                .unwrap()
        );
        store.update(MAJSE, login.id, &edit()).await.unwrap();
        let uses: Vec<Used> = store
            .activity(MAJSE, login.id)
            .await
            .unwrap()
            .expect("hers")
            .iter()
            .map(|a| a.used)
            .collect();
        assert_eq!(uses, [Used::Edited, Used::SignedIn, Used::Added]);

        // Another person's id is a missing one: nothing to read, and
        // nothing noted on it.
        assert_eq!(store.activity(MARK, login.id).await.unwrap(), None);
        assert!(
            !store
                .record(MARK, login.id, Used::SignedIn, &shop)
                .await
                .unwrap()
        );
        assert_eq!(
            store
                .activity(MAJSE, login.id)
                .await
                .unwrap()
                .unwrap()
                .len(),
            3
        );
    }

    #[tokio::test]
    async fn a_changed_account_notes_the_profile_it_drops() {
        let store = MemoryLogins::new();
        let login = store.add(MAJSE, &new_login()).await.unwrap();
        store
            .save_profile(MAJSE, login.id, &crate::Secret::new("{}"))
            .await
            .unwrap();
        let other = LoginEdit::new(
            "Shop",
            &[SITES[0].to_string()],
            crate::Secret::new("someone.else@example.com"),
            None,
        )
        .unwrap();
        store.update(MAJSE, login.id, &other).await.unwrap();
        let uses: Vec<Used> = store
            .activity(MAJSE, login.id)
            .await
            .unwrap()
            .unwrap()
            .iter()
            .map(|a| a.used)
            .collect();
        assert_eq!(uses, [Used::ProfileDropped, Used::Edited, Used::Added]);
    }

    #[tokio::test]
    async fn a_key_is_kept_replaced_and_removed_and_never_anyone_elses() {
        let store = MemoryLogins::new();
        let new = new_login()
            .with_authenticator(Some(crate::Secret::new(KEY)))
            .unwrap();
        let login = store.add(MAJSE, &new).await.unwrap();
        assert!(login.authenticator, "Vault shows it is set");
        assert!(
            store
                .authenticator(MAJSE, login.id)
                .await
                .unwrap()
                .is_some()
        );
        assert!(store.authenticator(MARK, login.id).await.unwrap().is_none());

        // An empty field keeps it.
        let keep = edit()
            .with_authenticator(KeyChange::Set(crate::Secret::new("")))
            .unwrap();
        let kept = store.update(MAJSE, login.id, &keep).await.unwrap().unwrap();
        assert!(kept.authenticator);

        let remove = edit().with_authenticator(KeyChange::Remove).unwrap();
        let removed = store
            .update(MAJSE, login.id, &remove)
            .await
            .unwrap()
            .unwrap();
        assert!(!removed.authenticator);
        assert!(
            store
                .authenticator(MAJSE, login.id)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_bad_key_is_refused_before_anything_is_kept() {
        assert!(
            new_login()
                .with_authenticator(Some(crate::Secret::new("not a key!")))
                .is_err()
        );
        assert!(
            edit()
                .with_authenticator(KeyChange::Set(crate::Secret::new("not a key!")))
                .is_err()
        );
    }

    #[tokio::test]
    async fn old_activity_is_swept() {
        let store = MemoryLogins::new();
        let login = store.add(MAJSE, &new_login()).await.unwrap();
        let later = Utc::now() + chrono::Duration::seconds(1);
        assert_eq!(store.drop_old_activity(later).await.unwrap(), 1);
        assert!(
            store
                .activity(MAJSE, login.id)
                .await
                .unwrap()
                .unwrap()
                .is_empty()
        );
    }
}
