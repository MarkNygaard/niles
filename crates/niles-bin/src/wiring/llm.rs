//! The language-model clients and the tools they are offered.

use crate::chat_loop::ChatProvider;
use anyhow::Context;
use niles_capabilities::CapabilityLoader;
use niles_config::Config;
use niles_core::DeviceRegistry;
use niles_llm::{OpenAiClient, OpenAiConfig};
use niles_memory::MemoryStore;
use niles_mqtt::{CommandRouter, MqttPublisher};
use niles_presence::PresenceAggregator;
use niles_skills::SkillStore;
use niles_tools::{LookUpCapability, ToolRegistry};
use std::sync::Arc;
use std::time::Duration;

/// Build the Tier 1 client from the `[llm]` section of an already-
/// validated config, resolving its key.
pub(crate) fn build_llm_client(cfg: &Config) -> anyhow::Result<OpenAiClient> {
    let endpoint = cfg
        .endpoint_for(
            niles_config::Role::Llm,
            cfg.llm.provider.as_deref(),
            cfg.llm.base_url.as_deref(),
            &cfg.llm.api_key_env,
            "llm",
        )
        .context("resolving where the language model should go")?;
    OpenAiClient::new(OpenAiConfig {
        api_key: endpoint.api_key,
        base_url: endpoint.base_url,
        model: cfg.llm.model.clone(),
        reasoning_effort: cfg.llm.reasoning_effort,
        request_timeout: Duration::from_secs(cfg.llm.timeout_seconds),
    })
    .context("building the language model's HTTP client")
}

/// Build an OpenAI client for Tier 2, or `None` when `[llm.tier2]` is absent.
pub(crate) fn build_tier2_client(cfg: &Config) -> anyhow::Result<Option<Arc<dyn ChatProvider>>> {
    let Some(tier2_cfg) = &cfg.llm.tier2 else {
        return Ok(None);
    };
    // The role that most wants a provider: escalation is by definition
    // somewhere other than the tier below it, and used to say so by
    // repeating a whole endpoint inline.
    let endpoint = cfg
        .endpoint_for(
            niles_config::Role::Llm,
            tier2_cfg.provider.as_deref(),
            Some(&tier2_cfg.base_url),
            &tier2_cfg.api_key_env,
            "llm.tier2",
        )
        .context("resolving where the escalation tier should go")?;
    let openai_cfg = OpenAiConfig {
        api_key: endpoint.api_key,
        base_url: endpoint.base_url,
        model: tier2_cfg.model.clone(),
        reasoning_effort: tier2_cfg.reasoning_effort,
        request_timeout: Duration::from_secs(tier2_cfg.timeout_seconds),
    };
    let client = OpenAiClient::new(openai_cfg).context("building OpenAI HTTP client")?;
    Ok(Some(Arc::new(client)))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_tool_registry(
    registry: Arc<DeviceRegistry>,
    publisher: MqttPublisher,
    router: Arc<CommandRouter>,
    capability_loader: Option<Arc<CapabilityLoader>>,
    memory_store: Option<Arc<MemoryStore>>,
    skill_store: Option<Arc<SkillStore>>,
    weather_client: Option<Arc<niles_weather::OpenMeteoClient>>,
    websearch_client: Option<Arc<niles_websearch::SearXngClient>>,
    websearch_default_num_results: u8,
    presence: Option<Arc<PresenceAggregator>>,
    home: &niles_config::HomeConfig,
    settings: Option<Arc<niles_config::ConfigStore>>,
    dry_run: bool,
) -> ToolRegistry {
    let mut tools = niles_tools::default_registry(registry, publisher, router, settings, dry_run);
    if let Some(loader) = capability_loader {
        tools.register(Box::new(LookUpCapability::new(loader)));
    }
    if let Some(store) = memory_store {
        niles_tools::register_memory_tools(&mut tools, store);
    }
    if let Some(store) = skill_store {
        niles_tools::register_skill_tools(&mut tools, store);
    }
    if let Some(client) = weather_client {
        let units = match home.resolved_units() {
            niles_config::Units::Metric => niles_weather::Units::Metric,
            niles_config::Units::Imperial => niles_weather::Units::Imperial,
        };
        niles_tools::register_weather_tools(
            &mut tools,
            client,
            home.latitude,
            home.longitude,
            units,
        );
    }
    if let Some(client) = websearch_client {
        niles_tools::register_web_search_tool(&mut tools, client, websearch_default_num_results);
    }
    if let Some(agg) = presence {
        niles_tools::register_presence_tools(&mut tools, agg);
    }
    niles_tools::register_datetime_tool(&mut tools, &home.timezone);
    tools
}
