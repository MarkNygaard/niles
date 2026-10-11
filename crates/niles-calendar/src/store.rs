//! Where the connections are kept. Postgres in the house, with the
//! credential encrypted (niles-db); in memory for tests.

use crate::error::Result;
use crate::model::Connection;
use async_trait::async_trait;
use std::sync::Mutex;

#[async_trait]
pub trait ConnectionStore: Send + Sync {
    /// One person's connections, oldest first.
    async fn list(&self, person: &str) -> Result<Vec<Connection>>;
    /// Add, or replace the one with the same id.
    async fn save(&self, connection: &Connection) -> Result<()>;
    /// Keep the refresh token a provider rotated in.
    async fn update_credential(&self, id: &str, credential: &str) -> Result<()>;
    /// Remove one of `person`'s, and its token. False when they have no
    /// such connection — somebody else's id included.
    async fn delete(&self, person: &str, id: &str) -> Result<bool>;
}

#[derive(Default)]
pub struct MemoryConnections {
    connections: Mutex<Vec<Connection>>,
}

impl MemoryConnections {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Connection>> {
        self.connections.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[async_trait]
impl ConnectionStore for MemoryConnections {
    async fn list(&self, person: &str) -> Result<Vec<Connection>> {
        Ok(self
            .lock()
            .iter()
            .filter(|c| c.person == person)
            .cloned()
            .collect())
    }

    async fn save(&self, connection: &Connection) -> Result<()> {
        let mut all = self.lock();
        match all.iter_mut().find(|c| c.id == connection.id) {
            Some(existing) => *existing = connection.clone(),
            None => all.push(connection.clone()),
        }
        Ok(())
    }

    async fn update_credential(&self, id: &str, credential: &str) -> Result<()> {
        if let Some(c) = self.lock().iter_mut().find(|c| c.id == id) {
            c.credential = credential.to_string();
        }
        Ok(())
    }

    async fn delete(&self, person: &str, id: &str) -> Result<bool> {
        let mut all = self.lock();
        let before = all.len();
        all.retain(|c| !(c.person == person && c.id == id));
        Ok(all.len() < before)
    }
}
