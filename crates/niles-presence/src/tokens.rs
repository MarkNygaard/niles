//! Where the Tado refresh token lives between restarts.

use crate::error::Result;
use async_trait::async_trait;
use std::sync::Mutex;

/// Somewhere to keep the refresh token.
///
/// It has to outlive the process: the device flow needs a person and a
/// browser, and asking for that on every restart would make Niles
/// unattended-restartable only until the next deploy.
///
/// Tado issues a **new refresh token every time one is used** and
/// invalidates the old one, so [`save`](Self::save) is called on every
/// refresh, not only at sign-in. Skipping that leaves a stored token
/// that was already spent, and the next restart has to ask a person
/// again — a day later, with no obvious cause.
#[async_trait]
pub trait TokenStore: Send + Sync {
    /// The stored refresh token, or `None` if nobody has authorised yet.
    async fn load(&self) -> Result<Option<String>>;

    /// Replace the stored refresh token.
    async fn save(&self, refresh_token: &str) -> Result<()>;
}

/// A store that forgets, for tests and for `niles presence --once`.
#[derive(Default)]
pub struct MemoryTokenStore {
    token: Mutex<Option<String>>,
}

impl MemoryTokenStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start out already authorised.
    pub fn with_token(token: impl Into<String>) -> Self {
        Self {
            token: Mutex::new(Some(token.into())),
        }
    }
}

#[async_trait]
impl TokenStore for MemoryTokenStore {
    async fn load(&self) -> Result<Option<String>> {
        Ok(self.token.lock().unwrap().clone())
    }

    async fn save(&self, refresh_token: &str) -> Result<()> {
        *self.token.lock().unwrap() = Some(refresh_token.to_string());
        Ok(())
    }
}
