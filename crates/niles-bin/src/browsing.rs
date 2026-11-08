//! Browsing: Niles's own browser, found at `niles-browser:9222`, and the
//! seven tools that work a page for the model, and `request_login`,
//! which asks the person for a saved login. See ARCHITECTURE.md's
//! "Browsing on someone's behalf".
//!
//! There is no switch: Niles looks for the browser at
//! [`niles_browser::HOST`]:[`niles_browser::DEVTOOLS_PORT`] and browsing
//! is on when one answers. Sessions are per person, not per conversation:
//! a first turn has no conversation id yet, and one person browsing two
//! conversations shares one page.

#[cfg(test)]
pub(crate) mod fake;
mod login;
mod step;
mod tools;

use crate::approvals::{Approvals, Asker};
use niles_browser::{Browser, DEVTOOLS_PORT, Driver, HOST, PROXY_PORT, Tab};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;

/// How long an open session may sit unused before it is closed: long
/// enough to read a page and answer its card, never the day.
const IDLE: Duration = Duration::from_secs(15 * 60);
/// The idle browser is restarted this often, so anything a page left
/// inside it dies at least daily.
const RESTART_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// How often a found browser is tended: idle sessions reaped, the daily
/// restart asked for.
const WATCH_EVERY: Duration = Duration::from_secs(30);
/// How often to look again for a browser that wasn't there.
const LOOK_AGAIN: Duration = Duration::from_secs(60);

/// Without a chat log a card has no conversation to be answered in, so
/// browsing is off.
pub(crate) enum Conversations {
    Kept,
    NotKept,
}

/// What looking for the browser found last. `Silent` is a name that
/// resolves but doesn't answer; `Nothing` is no name at all.
#[derive(Clone)]
enum Found {
    Looking,
    Nothing,
    Silent,
    Ready(Arc<dyn Driver>),
}

/// One person's open page.
struct Open {
    tab: Arc<dyn Tab>,
    lock: Arc<tokio::sync::Mutex<()>>,
    serial: u64,
    used: Instant,
    /// The card a held step of this session waits on, if one does: its
    /// session is not reaped while it waits.
    holding: Option<String>,
}

/// The clones of an [`Open`] a tool works with.
#[derive(Clone)]
pub(crate) struct Opened {
    pub(crate) tab: Arc<dyn Tab>,
    pub(crate) lock: Arc<tokio::sync::Mutex<()>>,
    pub(crate) serial: u64,
}

pub(crate) struct Browsing {
    approvals: Arc<Approvals>,
    conversations: Conversations,
    found: Mutex<Found>,
    sessions: Mutex<HashMap<String, Open>>,
    serial: AtomicU64,
    restarted: Mutex<Option<Instant>>,
    proxy: Mutex<Option<(IpAddr, JoinHandle<()>)>>,
}

/// What the model is told when browsing is off or a page won't open.
/// Fixed sentences, no page text in any of them.
pub(crate) const NOT_SET_UP: &str = "Browsing isn't set up here: Niles finds no browser at niles-browser. Its Kustomize component or Compose file adds one. Tell the person in one short sentence.";
pub(crate) const NOT_ANSWERING: &str =
    "Niles's browser isn't answering right now. Tell the person to try again in a minute.";
pub(crate) const IN_THE_APP: &str = "Browsing happens in the Niles app's chat, where every step waits on a card with a picture of the page. Tell the person in one short sentence to ask there. Nothing was opened.";
pub(crate) const NO_DATABASE: &str =
    "Browsing needs Niles's database, where its cards are kept with the conversation.";
pub(crate) const NOT_FOR_JOBS: &str = "Browsing is only for the chat, never for a background job.";

/// Start watching for the browser and tending what it serves.
pub(crate) fn start(approvals: Arc<Approvals>, conversations: Conversations) -> Arc<Browsing> {
    let browsing = Arc::new(Browsing {
        approvals,
        conversations,
        found: Mutex::new(Found::Looking),
        sessions: Mutex::new(HashMap::new()),
        serial: AtomicU64::new(0),
        restarted: Mutex::new(None),
        proxy: Mutex::new(None),
    });
    tokio::spawn(browsing.clone().watch());
    browsing
}

/// For the tests: a browsing that starts found, with no watcher.
#[cfg(test)]
pub(crate) fn with_driver(approvals: Arc<Approvals>, driver: Arc<dyn Driver>) -> Arc<Browsing> {
    Arc::new(Browsing {
        approvals,
        conversations: Conversations::Kept,
        found: Mutex::new(Found::Ready(driver)),
        sessions: Mutex::new(HashMap::new()),
        serial: AtomicU64::new(0),
        restarted: Mutex::new(None),
        proxy: Mutex::new(None),
    })
}

/// Register the browsing tools: the seven that work a page, and
/// `request_login`.
pub(crate) fn register(
    reg: &mut niles_tools::registry::ToolRegistry,
    browsing: Arc<Browsing>,
    logins: Arc<Logins>,
) {
    tools::register(reg, browsing.clone());
    reg.register(Box::new(login::RequestLogin { browsing, logins }));
}

pub(crate) use login::{ADDED_A_LOGIN, Logins, added_from_card, added_text};

impl Browsing {
    /// Tend what was found, and look again when there is nothing to
    /// tend. Each change of state is logged once, in `find`.
    async fn watch(self: Arc<Self>) {
        loop {
            let (ready, closed) = {
                let found = self.found.lock().unwrap_or_else(|e| e.into_inner());
                match &*found {
                    Found::Ready(driver) => (true, driver.closed()),
                    _ => (false, false),
                }
            };
            if ready && closed {
                self.lost().await;
                self.find().await;
                tokio::time::sleep(WATCH_EVERY).await;
                continue;
            }
            if ready {
                self.tend(Instant::now()).await;
                tokio::time::sleep(WATCH_EVERY).await;
                continue;
            }
            let found = self.find().await;
            let wait = if matches!(found, Found::Nothing) {
                LOOK_AGAIN
            } else {
                WATCH_EVERY
            };
            tokio::time::sleep(wait).await;
        }
    }

    /// One attempt at the browser; a change of state is logged once.
    async fn find(&self) -> Found {
        let seen = self.discover().await;
        let mut current = self.found.lock().unwrap_or_else(|e| e.into_inner());
        let changed = !matches!(
            (&*current, &seen),
            (Found::Nothing, Seen::Nothing)
                | (Found::Silent, Seen::Silent(_))
                | (Found::Ready(_), Seen::Ready { .. })
        );
        if !changed {
            return match seen {
                Seen::Nothing => Found::Nothing,
                Seen::Silent(_) => Found::Silent,
                Seen::Ready { driver, .. } => Found::Ready(driver),
            };
        }
        match &seen {
            Seen::Nothing => {
                tracing::info!("[browser] no browser at niles-browser:9222, so browsing is off")
            }
            Seen::Silent(e) => {
                tracing::warn!("[browser] the browser at niles-browser:9222 isn't answering: {e}")
            }
            Seen::Ready { product, ip, .. } => tracing::info!(
                "[browser] reached {product} at niles-browser:9222; its proxy listens on {ip}:3128"
            ),
        }
        let found = match seen {
            Seen::Nothing => Found::Nothing,
            Seen::Silent(_) => Found::Silent,
            Seen::Ready { driver, .. } => Found::Ready(driver),
        };
        *current = found.clone();
        found
    }

    /// The connection and proxy setup, with nothing logged: the caller
    /// logs the change of state.
    async fn discover(&self) -> Seen {
        let addresses = match tokio::net::lookup_host((HOST, DEVTOOLS_PORT)).await {
            Ok(addresses) => addresses,
            Err(_) => return Seen::Nothing,
        };
        let Some(addr) = addresses.into_iter().next() else {
            return Seen::Nothing;
        };
        let browser = match Browser::connect(addr, PROXY_PORT).await {
            Ok(browser) => browser,
            Err(e) => return Seen::Silent(e.to_string()),
        };
        let product = browser.product().to_string();
        let ip = browser.local_ip();
        if let Err(e) = self.listen(ip).await {
            tracing::error!("[browser] the proxy could not listen on {ip}: {e}");
            return Seen::Silent(e);
        }
        Seen::Ready {
            driver: Arc::new(browser),
            product,
            ip,
        }
    }

    /// Bind the proxy on the address the browser reaches Niles at — the
    /// local end of its DevTools connection — so the proxy listens on no
    /// other network.
    async fn listen(&self, ip: IpAddr) -> Result<(), String> {
        let already = self
            .proxy
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|(old, _)| *old == ip);
        if already {
            return Ok(());
        }
        let proxy = niles_browser::Proxy::bind(SocketAddr::new(ip, PROXY_PORT))
            .await
            .map_err(|e| e.to_string())?;
        let handle = tokio::spawn(proxy.serve());
        let old = self
            .proxy
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .replace((ip, handle));
        if let Some((_, old)) = old {
            old.abort();
        }
        Ok(())
    }

    /// The browser went away: every session ends, and every step waiting
    /// on a card lapses at once.
    async fn lost(&self) {
        let sessions = {
            let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::take(&mut *sessions)
        };
        for (_, open) in sessions {
            if let Some(id) = open.holding {
                self.approvals.lapse(&id).await;
            }
        }
        *self.found.lock().unwrap_or_else(|e| e.into_inner()) = Found::Looking;
        tracing::warn!("[browser] the browser went away; its sessions ended");
    }

    /// Close sessions idle past their quarter hour, unless a card waits
    /// on them, and restart the idle browser once a day.
    async fn tend(&self, now: Instant) {
        let driver = {
            let found = self.found.lock().unwrap_or_else(|e| e.into_inner());
            match &*found {
                Found::Ready(driver) => driver.clone(),
                _ => return,
            }
        };
        let reaped: Vec<Open> = {
            let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            let idle_ids: Vec<String> = sessions
                .iter()
                .filter(|(_, open)| {
                    let waits = open
                        .holding
                        .as_deref()
                        .is_some_and(|id| self.approvals.waiting(id));
                    idle(open.used, waits, now)
                })
                .map(|(person, _)| person.clone())
                .collect();
            idle_ids
                .into_iter()
                .filter_map(|person| sessions.remove(&person))
                .collect()
        };
        for open in reaped {
            open.tab.close().await;
        }
        let empty = self
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty();
        let restarted = *self.restarted.lock().unwrap_or_else(|e| e.into_inner());
        if !empty || !restart_due(restarted, now) {
            return;
        }
        match driver.restart().await {
            Ok(()) => {
                *self.restarted.lock().unwrap_or_else(|e| e.into_inner()) = Some(now);
                *self.found.lock().unwrap_or_else(|e| e.into_inner()) = Found::Looking;
                tracing::info!("[browser] restarting the idle browser, as once a day");
            }
            Err(e) => {
                tracing::warn!("[browser] restarting the idle browser failed: {e}");
                *self.found.lock().unwrap_or_else(|e| e.into_inner()) = Found::Looking;
            }
        }
    }

    /// Whether browsing is on, and the driver if it is.
    pub(crate) fn state(&self) -> State {
        match self.conversations {
            Conversations::NotKept => State::Off(NO_DATABASE),
            Conversations::Kept => match &*self.found.lock().unwrap_or_else(|e| e.into_inner()) {
                Found::Ready(driver) => State::Ready(driver.clone()),
                Found::Nothing => State::Off(NOT_SET_UP),
                Found::Looking | Found::Silent => State::Off(NOT_ANSWERING),
            },
        }
    }

    /// The person's open session, touched. With `Create::No`, none open
    /// is the model's error to fix; with `Create::Yes` a new one is made
    /// outside the lock, and a duplicate made meanwhile is closed.
    pub(crate) async fn session(
        &self,
        asker: &Asker,
        create: Create,
    ) -> Result<Opened, &'static str> {
        {
            let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(open) = sessions.get_mut(&asker.person) {
                open.used = Instant::now();
                return Ok(Opened {
                    tab: open.tab.clone(),
                    lock: open.lock.clone(),
                    serial: open.serial,
                });
            }
        }
        if matches!(create, Create::No) {
            return Err("Nothing is open in the browser yet: open a page with browser_open first.");
        }
        let State::Ready(driver) = self.state() else {
            return Err(NOT_ANSWERING);
        };
        let tab = driver.session().await.map_err(|_| NOT_ANSWERING)?;
        let open = Open {
            tab,
            lock: Arc::new(tokio::sync::Mutex::new(())),
            serial: self.serial.fetch_add(1, Ordering::SeqCst),
            used: Instant::now(),
            holding: None,
        };
        let existing = {
            let sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            sessions.get(&asker.person).map(|existing| Opened {
                tab: existing.tab.clone(),
                lock: existing.lock.clone(),
                serial: existing.serial,
            })
        };
        if let Some(opened) = existing {
            // One appeared while the browser worked: keep the older,
            // close ours.
            open.tab.close().await;
            return Ok(opened);
        }
        let opened = Opened {
            tab: open.tab.clone(),
            lock: open.lock.clone(),
            serial: open.serial,
        };
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(asker.person.clone(), open);
        Ok(opened)
    }

    /// The person's session, if it is still the one `serial` names: an
    /// older caller's session may have been reaped and remade.
    pub(crate) fn session_of(&self, person: &str, serial: u64) -> Option<Opened> {
        let sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        let open = sessions.get(person)?;
        (open.serial == serial).then(|| Opened {
            tab: open.tab.clone(),
            lock: open.lock.clone(),
            serial: open.serial,
        })
    }

    /// Note the session was used, so it isn't reaped under it.
    pub(crate) fn touch(&self, person: &str, serial: u64) {
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(open) = sessions.get_mut(person)
            && open.serial == serial
        {
            open.used = Instant::now();
        }
    }

    /// The card this session's held step waits on, if one does.
    pub(crate) fn hold(&self, person: &str, serial: u64, id: String) {
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(open) = sessions.get_mut(person)
            && open.serial == serial
        {
            open.holding = Some(id);
        }
    }

    /// Whether one of the person's steps waits on its card now.
    pub(crate) fn waiting_step(&self, person: &str) -> bool {
        let sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        sessions
            .get(person)
            .and_then(|open| open.holding.as_deref())
            .is_some_and(|id| self.approvals.waiting(id))
    }
}

/// The browser as a tool sees it: on, or off with the why in the
/// model's terms.
pub(crate) enum State {
    Off(&'static str),
    Ready(Arc<dyn Driver>),
}

/// Whether a tool may make a session to do its work.
pub(crate) enum Create {
    Yes,
    No,
}

/// What `discover` saw, before the state decides what is logged.
enum Seen {
    Nothing,
    Silent(String),
    Ready {
        driver: Arc<dyn Driver>,
        product: String,
        ip: IpAddr,
    },
}

/// An open session may close once it sat idle this long, unless a card
/// waits on its step.
fn idle(used: Instant, waits: bool, now: Instant) -> bool {
    !waits && now.duration_since(used) >= IDLE
}

/// When the idle browser may be restarted. Niles can't know how long
/// the browser has run, so the first idle moment after Niles starts
/// counts, and every day after that.
fn restart_due(restarted: Option<Instant>, now: Instant) -> bool {
    restarted.is_none_or(|at| now.duration_since(at) >= RESTART_EVERY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_idle_session_ends_unless_its_card_waits() {
        let now = Instant::now();
        assert!(idle(now - IDLE, false, now), "idle past its quarter hour");
        assert!(!idle(now - IDLE, true, now), "a card waits on it");
        assert!(!idle(now, false, now), "just used");
    }

    #[test]
    fn the_browser_restarts_once_a_day_and_after_niles_starts() {
        let now = Instant::now();
        assert!(
            restart_due(None, now),
            "the first idle moment after Niles starts"
        );
        assert!(!restart_due(Some(now), now), "not twice in a day");
        assert!(restart_due(Some(now - RESTART_EVERY), now), "a day after");
    }
}
