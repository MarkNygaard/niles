//! The automation engine's ends: its device sink, its notifier, its waits and its store.

use crate::lighting::house_location;
use crate::wiring::stores::database;
use niles_automations::{DeviceSink, Notifier, Priority as AutomationPriority, Rule};
use niles_config::Config;
use niles_core::{DeviceId, DeviceState};
use niles_mqtt::{CommandRouter, MqttPublisher};
use niles_notifications::NotificationCenter;
use niles_scheduler::ManualModeTracker;
use std::sync::Arc;

/// Where the automations keep a run waiting out a delay.
pub(crate) fn automation_waits(cfg: &Config) -> Option<Arc<dyn niles_automations::WaitStore>> {
    let db = database(cfg)?;
    Some(Arc::new(KeptWaits(niles_db::PostgresAutomationWaits::new(
        db.pool(),
        db.describe_target(),
    ))))
}

/// The automations' waits, as JSON in Postgres.
pub(crate) struct KeptWaits(pub(crate) niles_db::PostgresAutomationWaits);

#[async_trait::async_trait]
impl niles_automations::WaitStore for KeptWaits {
    async fn keep(&self, wait: &niles_automations::Waiting) {
        let kept = match serde_json::to_string(wait) {
            Ok(document) => self.0.keep(&wait.rule, &document).await,
            Err(e) => Err(e.to_string()),
        };
        if let Err(e) = kept {
            tracing::warn!("automation '{}': its wait is not kept: {e}", wait.rule);
        }
    }

    async fn forget(&self, rule: &str) {
        if let Err(e) = self.0.forget(rule).await {
            tracing::warn!("automation '{rule}': its wait was not forgotten: {e}");
        }
    }

    async fn all(&self) -> Vec<niles_automations::Waiting> {
        match self.0.all().await {
            Ok(documents) => documents
                .iter()
                .filter_map(|d| {
                    serde_json::from_str(d)
                        .inspect_err(|e| tracing::warn!("an automation wait would not read: {e}"))
                        .ok()
                })
                .collect(),
            Err(e) => {
                tracing::warn!("automation waits could not be read: {e}");
                Vec::new()
            }
        }
    }
}

// ------------------------------------------------------------------
// Automation engine wiring
// ------------------------------------------------------------------

/// Send `desired` to a device Niles does not reach over MQTT — a
/// plugin's — logged under `what` like the publishes beside it. True
/// when it was taken, or would have been in a dry run.
pub(crate) async fn command_directly(
    commander: &dyn niles_mqtt::DeviceCommander,
    id: &DeviceId,
    desired: &DeviceState,
    dry_run: bool,
    what: &str,
) -> bool {
    if dry_run {
        tracing::info!("{what} [dry-run] {id} {desired:?}");
        return true;
    }
    match commander.command(id, desired).await {
        Ok(()) => {
            tracing::info!("{what} {id} {desired:?}");
            true
        }
        Err(e) => {
            tracing::warn!("{what} {id} did not take it: {e}");
            false
        }
    }
}

pub(crate) struct MqttDeviceSink {
    pub(crate) publisher: MqttPublisher,
    pub(crate) router: Arc<CommandRouter>,
    /// A rule the household made is an explicit adjustment like any
    /// other: what it sets, the curve leaves alone.
    pub(crate) manual: Arc<ManualModeTracker>,
    pub(crate) dry_run: bool,
}

#[async_trait::async_trait]
impl DeviceSink for MqttDeviceSink {
    async fn set(&self, device: &DeviceId, desired: &DeviceState) {
        self.manual.adjusted(device, desired);
        if let Some(commander) = self.router.commander_for(device, desired) {
            command_directly(commander, device, desired, self.dry_run, "[automation]").await;
            return;
        }
        let Some((topic, payload)) = self.router.format(device, desired) else {
            tracing::debug!("[automation] skipped set for {device}: no actionable fields");
            return;
        };
        if self.dry_run {
            tracing::info!("[automation] [dry-run] {topic}  {payload}");
            return;
        }
        match self.publisher.publish(&topic, payload.clone()).await {
            Ok(()) => tracing::info!("[automation] published {topic}  {payload}"),
            Err(e) => tracing::warn!("[automation] publish to {topic} failed: {e}"),
        }
    }
}

pub(crate) struct CenterNotifier {
    pub(crate) center: Arc<NotificationCenter>,
}

#[async_trait::async_trait]
impl Notifier for CenterNotifier {
    async fn notify(&self, body: &str, room: Option<&str>, priority: AutomationPriority) {
        let p = match priority {
            AutomationPriority::Routine => niles_notifications::Priority::Routine,
            AutomationPriority::Important => niles_notifications::Priority::Important,
            AutomationPriority::Urgent => niles_notifications::Priority::Urgent,
            _ => {
                tracing::warn!("[automation] unknown priority variant, falling back to Routine");
                niles_notifications::Priority::Routine
            }
        };
        self.center.deliver(body, room.map(String::from), p);
    }
}

/// The rules to run: the household's own, in the order they were made,
/// then the config file's. Any that do not parse are logged and left
/// out.
pub(crate) fn automation_rules(
    stored: Option<&niles_automations::AutomationStore>,
    cfg: &niles_config::AutomationsConfig,
) -> Vec<Rule> {
    let mut rules = stored.map(|s| s.runnable()).unwrap_or_default();
    for dto in &cfg.rules {
        match Rule::from_dto(dto, niles_automations::store::DEFAULT_SOURCE) {
            Ok(rule) => rules.push(rule),
            Err(e) => tracing::warn!("skipping automation '{}': {e}", dto.id),
        }
    }
    tracing::info!("automations: running {} rule(s)", rules.len());
    rules
}

/// Hands each new automations document to a task that writes it, as
/// [`QueuedGroceries`] does: saving a rule must not wait on the database.
pub(crate) struct QueuedAutomations {
    pub(crate) tx: tokio::sync::mpsc::UnboundedSender<String>,
}

impl niles_automations::AutomationPersistence for QueuedAutomations {
    fn store(&self, document: &str) {
        if self.tx.send(document.to_string()).is_err() {
            tracing::warn!(
                "[automations] nothing is writing them any more; this change is in memory only"
            );
        }
    }
}

/// The household's automations, kept in Postgres when there is one.
///
/// Without a database they live as long as the process, and say so. A
/// set that cannot be read is left out rather than started empty: the
/// first rule made would otherwise overwrite the real ones.
pub(crate) async fn build_automation_store(
    cfg: &Config,
) -> Option<Arc<niles_automations::AutomationStore>> {
    let location = house_location(cfg);
    let in_memory = || {
        tracing::warn!("[automations] no database; rules made now will not survive a restart");
        Some(Arc::new(
            niles_automations::AutomationStore::new().with_location(location),
        ))
    };
    let Some(database) = &cfg.database else {
        return in_memory();
    };
    let Ok(url) = database.resolve_url() else {
        return in_memory();
    };
    let Ok(backend) = niles_db::PostgresBackend::connect_lazy(&url, database.max_connections)
    else {
        return in_memory();
    };
    let describe = backend.describe_target();
    let table = Arc::new(niles_db::PostgresAutomations::new(backend.pool(), describe));
    let store = match table.load().await {
        Ok(Some(document)) => match niles_automations::AutomationStore::from_json(&document) {
            Ok(store) => store,
            Err(e) => {
                tracing::error!(
                    "[automations] the stored rules would not parse ({e}); they are unavailable this run"
                );
                return None;
            }
        },
        Ok(None) => niles_automations::AutomationStore::new(),
        Err(e) => {
            tracing::error!("[automations] could not be read ({e}); they are unavailable this run");
            return None;
        }
    };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        while let Some(document) = rx.recv().await {
            if let Err(e) = table.store(&document).await {
                tracing::error!("[automations] save failed: {e}");
            }
        }
    });
    Some(Arc::new(
        store
            .with_location(location)
            .with_sink(Arc::new(QueuedAutomations { tx })),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sink() -> (MqttDeviceSink, Arc<ManualModeTracker>) {
        // A broker that is never reached: the sink runs dry.
        let mqtt = niles_mqtt::MqttClient::connect(niles_mqtt::MqttOptions::new(
            "127.0.0.1",
            1,
            "automation-sink-test",
        ));
        let manual = Arc::new(ManualModeTracker::new());
        let sink = MqttDeviceSink {
            publisher: mqtt.publisher(),
            router: Arc::new(CommandRouter::z2m_only("zigbee2mqtt")),
            manual: manual.clone(),
            dry_run: true,
        };
        (sink, manual)
    }

    #[tokio::test]
    async fn a_level_a_rule_sets_is_not_pulled_back_by_the_curve() {
        let (sink, manual) = sink();
        let lamp = DeviceId::parse("z2m:living_room/floor_lamp").unwrap();
        sink.set(
            &lamp,
            &DeviceState {
                brightness: Some(20),
                color_temp_kelvin: Some(2200),
                ..Default::default()
            },
        )
        .await;
        assert!(manual.is_flagged(&lamp));
    }

    #[tokio::test]
    async fn a_rule_switching_a_light_leaves_it_to_the_curve() {
        let (sink, manual) = sink();
        let lamp = DeviceId::parse("z2m:hallway/ceiling").unwrap();
        for on in [true, false] {
            sink.set(
                &lamp,
                &DeviceState {
                    on: Some(on),
                    ..Default::default()
                },
            )
            .await;
        }
        assert!(!manual.is_flagged(&lamp));
    }
}
