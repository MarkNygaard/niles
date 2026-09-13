//! Secrets typed into the app, encrypted at rest.
//!
//! The broker password, the API keys, the session secret: everything a
//! config file would have named an environment variable for. They live
//! here so Niles can be set up from the app rather than from a
//! ConfigMap and a Secret.
//!
//! **Sealed with AES-256-GCM** under a key given at construction, and
//! stored as `nonce(12) || ciphertext`, so a database dump on its own
//! reveals nothing. That is not belt-and-braces: the override document
//! in the next table over is deliberately readable in `psql`, and
//! anybody with that habit should not find a broker password while they
//! are in there.
//!
//! The key itself cannot live here — it is the one thing that has to
//! arrive from outside, alongside the connection string. Those two are
//! the whole of what Niles needs before the app can take over.

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::Engine;
use niles_config::error::{Error, Result};
use rand::Rng;
use sqlx::{Row, postgres::PgPool};
use std::collections::HashMap;

const NONCE_LEN: usize = 12;

const SCHEMA: &str = "
create table if not exists secrets (
    key        text primary key,
    value      bytea not null,
    updated_at timestamptz not null default now()
);
";

impl std::fmt::Debug for PostgresSecrets {
    /// Deliberately says nothing about the key. A `Debug` that prints a
    /// cipher is a key in a log line waiting to happen.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresSecrets")
            .field("database", &self.describe)
            .finish_non_exhaustive()
    }
}

/// The crypto on its own, with no database under it.
///
/// Separated because it has nothing to do with Postgres: sealing a
/// string is testable in isolation, and a test that has to build a
/// connection pool to check an encryption round trip is a test about
/// the wrong thing.
pub struct Sealer {
    cipher: Aes256Gcm,
}

impl std::fmt::Debug for Sealer {
    /// Says nothing about the key. A `Debug` that prints a cipher is a
    /// key in a log line waiting to happen.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Sealer(..)")
    }
}

impl Sealer {
    /// `key_b64` is 32 bytes, base64. Anything else is refused rather
    /// than padded or hashed into shape: a key that is quietly the
    /// wrong one decrypts nothing and looks like data loss.
    pub fn new(key_b64: &str) -> Result<Self> {
        let raw = base64::engine::general_purpose::STANDARD
            .decode(key_b64.trim())
            .map_err(|_| bad_key("it is not valid base64"))?;
        if raw.len() != 32 {
            return Err(bad_key(&format!(
                "it decodes to {} bytes, and AES-256 needs 32",
                raw.len()
            )));
        }
        Ok(Self {
            cipher: Aes256Gcm::new_from_slice(&raw).map_err(|_| bad_key("it was refused"))?,
        })
    }

    /// A fresh key, for printing once so somebody can put it somewhere.
    pub fn generate_key() -> String {
        let mut raw = [0u8; 32];
        // Seeded from the OS and reseeded as it runs — the right source
        // for a key, and the one this workspace already depends on.
        rand::rng().fill_bytes(&mut raw);
        base64::engine::general_purpose::STANDARD.encode(raw)
    }

    fn seal(&self, value: &str) -> Result<Vec<u8>> {
        let mut nonce = [0u8; NONCE_LEN];
        rand::rng().fill_bytes(&mut nonce);
        let ciphertext = self
            .cipher
            .encrypt(Nonce::from_slice(&nonce), value.as_bytes())
            .map_err(|_| bad_key("the value could not be sealed"))?;
        // Nonce first, so a reader knows where the ciphertext starts
        // without storing a length.
        let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }

    fn open(&self, sealed: &[u8]) -> Result<String> {
        if sealed.len() <= NONCE_LEN {
            return Err(bad_key("the stored value is too short to be sealed"));
        }
        let (nonce, ciphertext) = sealed.split_at(NONCE_LEN);
        let plain = self
            .cipher
            .decrypt(Nonce::from_slice(nonce), ciphertext)
            .map_err(|_| bad_key("it did not decrypt"))?;
        String::from_utf8(plain).map_err(|_| bad_key("it did not decrypt to text"))
    }
}

pub struct PostgresSecrets {
    pool: PgPool,
    describe: String,
    sealer: Sealer,
    schema: tokio::sync::OnceCell<()>,
}

impl PostgresSecrets {
    /// `key_b64` is 32 bytes, base64. Anything else is refused rather
    /// than padded or hashed into shape: a key that is quietly the
    /// wrong one decrypts nothing and looks like data loss.
    pub fn new(pool: PgPool, describe: String, key_b64: &str) -> Result<Self> {
        Ok(Self {
            pool,
            describe,
            sealer: Sealer::new(key_b64)?,
            schema: tokio::sync::OnceCell::new(),
        })
    }

    async fn ensure_schema(&self) -> Result<()> {
        self.schema
            .get_or_try_init(|| async {
                sqlx::raw_sql(SCHEMA)
                    .execute(&self.pool)
                    .await
                    .map(|_| ())
                    .map_err(|e| self.storage(e))
            })
            .await
            .copied()
    }

    /// Every secret, decrypted, ready to be handed to the in-process
    /// store. Read as a whole because that is how it is used: once at
    /// startup, and again when one changes.
    pub async fn load_all(&self) -> Result<HashMap<String, String>> {
        self.ensure_schema().await?;
        let rows = sqlx::query("select key, value from secrets")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| self.storage(e))?;

        let mut out = HashMap::with_capacity(rows.len());
        for row in &rows {
            let key: String = row.try_get("key").map_err(|e| self.storage(e))?;
            let sealed: Vec<u8> = row.try_get("value").map_err(|e| self.storage(e))?;
            match self.sealer.open(&sealed) {
                Ok(value) => {
                    out.insert(key, value);
                }
                // One unreadable row must not cost every other secret.
                // Almost always a changed key, which is worth saying
                // plainly rather than reporting as a database error.
                Err(_) => tracing::error!(
                    "[secrets] {key} cannot be decrypted — was the encryption key changed?"
                ),
            }
        }
        Ok(out)
    }

    pub async fn set(&self, key: &str, value: &str) -> Result<()> {
        self.ensure_schema().await?;
        let sealed = self.sealer.seal(value)?;
        sqlx::query(
            "insert into secrets (key, value, updated_at) values ($1, $2, now())
             on conflict (key) do update
             set value = excluded.value, updated_at = excluded.updated_at",
        )
        .bind(key)
        .bind(sealed)
        .execute(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(())
    }

    pub async fn clear(&self, key: &str) -> Result<()> {
        self.ensure_schema().await?;
        sqlx::query("delete from secrets where key = $1")
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(|e| self.storage(e))?;
        Ok(())
    }

    /// Which secrets exist. Names only — the values never leave here
    /// except into the in-process store.
    pub async fn keys(&self) -> Result<Vec<String>> {
        self.ensure_schema().await?;
        let rows = sqlx::query("select key from secrets order by key")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| self.storage(e))?;
        rows.iter()
            .map(|r| r.try_get("key").map_err(|e| self.storage(e)))
            .collect()
    }

    /// Errors name the database but never the DSN — the connection
    /// string carries a password of its own.
    fn storage(&self, e: impl std::fmt::Display) -> Error {
        Error::InvalidSection {
            section: "secrets",
            reason: format!("secret storage ({}): {e}", self.describe),
        }
    }
}

fn bad_key(reason: &str) -> Error {
    Error::InvalidSection {
        section: "secrets",
        reason: format!("the encryption key is unusable: {reason}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sealer() -> Sealer {
        Sealer::new(&Sealer::generate_key()).expect("a generated key is usable")
    }

    #[test]
    fn a_sealed_secret_comes_back_out() {
        let s = sealer();
        let sealed = s.seal("hunter2").unwrap();
        assert_eq!(s.open(&sealed).unwrap(), "hunter2");
    }

    #[test]
    fn the_ciphertext_does_not_contain_the_secret() {
        // The whole reason for encrypting: a database dump must not
        // show a broker password to somebody reading it in psql.
        let sealed = sealer().seal("hunter2").unwrap();
        assert!(
            !String::from_utf8_lossy(&sealed).contains("hunter2"),
            "the plaintext survived"
        );
    }

    #[test]
    fn the_same_secret_seals_differently_every_time() {
        // A fresh nonce per write, so two identical passwords are not
        // visibly identical in the table.
        let s = sealer();
        assert_ne!(s.seal("same").unwrap(), s.seal("same").unwrap());
    }

    #[test]
    fn another_key_cannot_open_it() {
        let sealed = sealer().seal("hunter2").unwrap();
        assert!(sealer().open(&sealed).is_err());
    }

    #[test]
    fn a_truncated_value_is_refused_rather_than_panicking() {
        let s = sealer();
        assert!(s.open(&[0u8; NONCE_LEN]).is_err());
        assert!(s.open(&[]).is_err());
    }

    #[test]
    fn a_key_that_is_the_wrong_size_is_refused() {
        // Padding or hashing it into shape would decrypt nothing and
        // look like data loss rather than a typo.
        let short = base64::engine::general_purpose::STANDARD.encode([0u8; 16]);
        let err = Sealer::new(&short).unwrap_err();
        assert!(format!("{err}").contains("32"), "{err}");
    }

    #[test]
    fn a_key_that_is_not_base64_is_refused() {
        assert!(Sealer::new("not base64 at all !!").is_err());
    }
}
