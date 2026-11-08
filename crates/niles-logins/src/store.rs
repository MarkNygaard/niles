//! Where each person's saved logins are kept: Postgres in the house
//! (niles-db), in memory for tests.
//!
//! Every method is scoped by the person's key, so another person's id
//! behaves exactly like one that was never kept. The sealed username and
//! password, and the browser profile, belong to the row and leave with
//! it.

use crate::error::Result;
use crate::model::{Credentials, Login, LoginEdit, NewLogin, Secret};
use crate::origin::Origin;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::sync::Mutex;

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
    /// The sealed values, opened. This is for Niles's browser to fill
    /// in (NIL-76), never for a response or the model.
    async fn credentials(&self, person: &str, id: i64) -> Result<Option<Credentials>>;
    /// A browser profile, sealed. For NIL-76's restore, never a
    /// response or the model.
    async fn save_profile(&self, person: &str, id: i64, profile: &Secret) -> Result<bool>;
    /// The person's sealed profile for the login, if one was saved.
    async fn profile(&self, person: &str, id: i64) -> Result<Option<Secret>>;
}

struct Row {
    person: String,
    id: i64,
    label: String,
    origins: Vec<Origin>,
    credentials: Credentials,
    last_used: Option<DateTime<Utc>>,
    profile: Option<Secret>,
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

    fn login(row: &Row) -> Login {
        Login {
            id: row.id,
            label: row.label.clone(),
            origins: row.origins.clone(),
            username: row.credentials.username.expose().to_string(),
            last_used: row.last_used,
            signed_in: row.profile.is_some(),
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
        };
        kept.next += 1;
        kept.rows.push(row);
        Ok(Self::login(kept.rows.last().expect("just pushed")))
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
        if row.credentials.username != edit.username || before != after {
            row.profile = None;
        }
        row.label = edit.label.clone();
        row.origins = edit.origins.clone();
        row.credentials.username = edit.username.clone();
        if let Some(password) = &edit.password {
            row.credentials.password = password.clone();
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

    async fn save_profile(&self, person: &str, id: i64, profile: &Secret) -> Result<bool> {
        let mut kept = self.lock();
        let Some(row) = kept
            .rows
            .iter_mut()
            .find(|r| r.person == person && r.id == id)
        else {
            return Ok(false);
        };
        row.profile = Some(profile.clone());
        Ok(true)
    }

    async fn profile(&self, person: &str, id: i64) -> Result<Option<Secret>> {
        Ok(self
            .lock()
            .rows
            .iter()
            .find(|r| r.person == person && r.id == id)
            .and_then(|r| r.profile.clone()))
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
}
