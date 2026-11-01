//! The broker connection, the Zigbee2MQTT command router and the WLED sources.

use anyhow::Context;
use niles_config::Config;
use niles_core::{DeviceId, DeviceRegistry, EventBus};
use niles_mqtt::{CommandRouter, MqttClient, MqttOptions, WledSource};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

pub(crate) fn parse_wled_id(name: &str) -> Option<DeviceId> {
    DeviceId::parse(&format!("wled:{name}"))
        .map_err(|e| tracing::warn!("wled device {name:?} failed to parse: {e}"))
        .ok()
}

/// Build a `CommandRouter` from config.
pub(crate) fn build_command_router(cfg: &Config) -> Arc<CommandRouter> {
    Arc::new(command_router(cfg))
}

pub(crate) fn command_router(cfg: &Config) -> CommandRouter {
    let wled_map: HashMap<_, _> = cfg
        .wled
        .devices
        .iter()
        .filter_map(|dev| parse_wled_id(&dev.name).map(|id| (id, dev.topic.clone())))
        .collect();
    CommandRouter::new(cfg.mqtt.z2m_prefix.clone(), wled_map)
}

/// Spawn a `WledSource` when `[wled]` has at least one device, and the
/// publisher to its client, so a shutdown can say goodbye on that
/// connection too. Returns `None` when WLED is not configured.
pub(crate) async fn spawn_wled_source(
    cfg: &Config,
    registry: Arc<DeviceRegistry>,
    bus: EventBus,
) -> Option<(tokio::task::JoinHandle<()>, niles_mqtt::MqttPublisher)> {
    if cfg.wled.devices.is_empty() {
        return None;
    }
    let (username, password) = match cfg.mqtt.resolve_credentials() {
        Ok(creds) => creds,
        Err(e) => {
            tracing::error!("WLED source: failed to resolve MQTT credentials: {e}");
            return None;
        }
    };
    let client_id = format!("{}-wled", cfg.mqtt.client_id);
    let opts = MqttOptions::new(&cfg.mqtt.host, cfg.mqtt.port, &client_id)
        .with_credentials(username, password);
    let client = MqttClient::connect(opts);
    let devices: Vec<_> = cfg
        .wled
        .devices
        .iter()
        .filter_map(|dev| {
            parse_wled_id(&dev.name).map(|id| niles_mqtt::WledDevice {
                id,
                topic: dev.topic.clone(),
                rgb: dev.rgb,
                white_balance: dev.white_balance,
            })
        })
        .collect();
    let publisher = client.publisher();
    let source = WledSource::new(client, registry, bus, devices);
    tracing::info!("WLED source running: {} device(s)", cfg.wled.devices.len());
    let handle = tokio::spawn(async move {
        if let Err(e) = source.run().await {
            tracing::error!("WledSource exited: {e}");
        }
    });
    Some((handle, publisher))
}

/// Build an `MqttClient` connected to the broker described in
/// `niles.toml`, with credentials resolved from env vars.
pub(crate) async fn connect_from_config(
    config_path: &Path,
) -> anyhow::Result<(Config, MqttClient)> {
    let cfg = Config::load_from_path(config_path)
        .with_context(|| format!("loading config from {}", config_path.display()))?;
    cfg.validate().context("validating config")?;
    let client = connect_with_config(&cfg).await?;
    Ok((cfg, client))
}

/// Connect to the broker described by an already-loaded, already-validated
/// config. Split out so `serve` can connect from a `ConfigStore` snapshot
/// rather than re-reading the file.
pub(crate) async fn connect_with_config(cfg: &Config) -> anyhow::Result<MqttClient> {
    let (username, password) = cfg
        .mqtt
        .resolve_credentials()
        .context("resolving MQTT credentials from environment")?;

    eprintln!(
        "Connecting to mqtt://{}:{} as client_id={} ...",
        cfg.mqtt.host, cfg.mqtt.port, cfg.mqtt.client_id
    );

    let opts = MqttOptions::new(&cfg.mqtt.host, cfg.mqtt.port, &cfg.mqtt.client_id)
        .with_credentials(username, password);
    Ok(MqttClient::connect(opts))
}
