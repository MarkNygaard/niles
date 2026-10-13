//! How a scheduler store's state outlives the process.

use std::sync::Arc;

use serde::Serialize;

/// Somewhere a store's state outlives the process.
///
/// A trait rather than a path. State kept on a mounted directory needs
/// a config file to say where that directory is, and the config file
/// is the thing Niles is trying to stop needing: in a cluster the state
/// goes to the database instead, alongside the credentials and the
/// overrides.
///
/// Handed the whole document rather than one change. Each store is a
/// handful of entries, and the alternative is two ways for it to be
/// wrong.
///
/// Called with the store's lock held, so documents arrive in the order
/// the changes were made. An implementation must not block: the stores
/// are used on paths that cannot wait for a database.
pub trait DocumentSink: Send + Sync {
    fn store(&self, document: &str);
}

/// Serialize `document` and give it to `sink`, if there is one.
pub(crate) fn hand_over<T: Serialize>(
    sink: Option<&Arc<dyn DocumentSink>>,
    document: &T,
    label: &'static str,
) {
    let Some(sink) = sink else {
        return;
    };
    match serde_json::to_string(document) {
        Ok(document) => sink.store(&document),
        Err(e) => tracing::warn!("persistence: {label} would not serialize: {e}"),
    }
}

/// Records every document it is handed, for tests.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct Spy(std::sync::Mutex<Vec<String>>);

#[cfg(test)]
impl Spy {
    pub(crate) fn documents(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }

    /// The most recent document, parsed.
    pub(crate) fn last(&self) -> serde_json::Value {
        let documents = self.documents();
        let last = documents.last().expect("a document was stored");
        serde_json::from_str(last).expect("the document is JSON")
    }
}

#[cfg(test)]
impl DocumentSink for Spy {
    fn store(&self, document: &str) {
        self.0.lock().unwrap().push(document.to_string());
    }
}
