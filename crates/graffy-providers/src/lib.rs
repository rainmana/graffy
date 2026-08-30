//! Provider layer (ADR-0005) — rig-core carries the wire clients.
//!
//! Five providers are first-class in M2: **Anthropic**, **OpenAI**,
//! **Ollama** (local), plus **Venice** and **OpenRouter** (rig 0.42 ships
//! native clients for both). Any other OpenAI-compatible endpoint (LM Studio
//! etc.) lands with the generic base-URL client in a follow-up milestone.
//!
//! Routing speaks capability *tiers*, not vendor names. Bind tiers with
//! environment variables:
//!
//! ```text
//! GRAFFY_MODEL_FAST=anthropic:claude-haiku-…
//! GRAFFY_MODEL_BALANCED=ollama:qwen2.5:14b
//! GRAFFY_MODEL_FRONTIER=openrouter:some/frontier-model
//! ```
//!
//! plus the matching credentials (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`,
//! `OPENROUTER_API_KEY`, `VENICE_API_KEY`; Ollama needs none, honoring
//! `OLLAMA_API_BASE_URL`). graffy hardcodes no model names — vendors rename
//! models faster than releases ship, and stale defaults are silent lies.
//!
//! Invariant enforcement: this crate exposes no free "complete this prompt"
//! function. The only consumer is the executor, via [`ModelInvoker`], and the
//! only reachable path to it is a scheduled graph node. Completions are
//! wrapped into Information Units by the node behaviors in graffy-core.

use std::collections::HashMap;
use std::time::Instant;

use graffy_core::error::ModelError;
use graffy_core::exec::{ModelInvoker, ModelRequest, ModelResponse};
use rig_core::client::{CompletionClient, Nothing};
use rig_core::completion::CompletionModel;
use rig_core::completion::message::AssistantContent;
use rig_core::providers::{anthropic, ollama, openai, openrouter, venice};

/// Re-export so downstream crates name one rig. NOTE: the crate is
/// `rig-core`, but its library target is `rig_core`.
pub use rig_core;

/// Providers graffy can bind tiers to in M2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderKind {
    Anthropic,
    OpenAi,
    OpenRouter,
    Venice,
    Ollama,
}

impl ProviderKind {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "anthropic" => Some(Self::Anthropic),
            "openai" => Some(Self::OpenAi),
            "openrouter" => Some(Self::OpenRouter),
            "venice" => Some(Self::Venice),
            "ollama" => Some(Self::Ollama),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAi => "openai",
            Self::OpenRouter => "openrouter",
            Self::Venice => "venice",
            Self::Ollama => "ollama",
        }
    }
}

/// One tier's concrete target.
#[derive(Debug, Clone)]
pub struct TierBinding {
    pub provider: ProviderKind,
    pub model: String,
}

/// Parse `GRAFFY_MODEL_<TIER>=provider:model` bindings from the environment.
pub fn bindings_from_env() -> HashMap<String, TierBinding> {
    let mut bindings = HashMap::new();
    for (key, value) in std::env::vars() {
        let Some(tier) = key.strip_prefix("GRAFFY_MODEL_") else {
            continue;
        };
        let Some((provider_raw, model)) = value.split_once(':') else {
            tracing::warn!(%key, "ignoring malformed binding (want provider:model)");
            continue;
        };
        let Some(provider) = ProviderKind::parse(provider_raw) else {
            tracing::warn!(%key, provider = provider_raw, "ignoring unknown provider");
            continue;
        };
        bindings.insert(
            tier.to_ascii_lowercase(),
            TierBinding {
                provider,
                model: model.trim().to_owned(),
            },
        );
    }
    bindings
}

enum ClientHandle {
    Anthropic(anthropic::Client),
    OpenAi(openai::Client),
    OpenRouter(openrouter::Client),
    Venice(venice::Client),
    Ollama(ollama::Client),
}

/// rig-backed [`ModelInvoker`]: resolves tiers via env bindings and journals
/// nothing itself — the executor owns the journal.
pub struct RigInvoker {
    bindings: HashMap<String, TierBinding>,
    clients: HashMap<ProviderKind, ClientHandle>,
}

impl RigInvoker {
    /// Build from environment bindings + credentials. Fails with actionable
    /// hints rather than limping into a run that would guess.
    pub fn from_env() -> Result<Self, ModelError> {
        let bindings = bindings_from_env();
        if bindings.is_empty() {
            return Err(ModelError::UnboundTier {
                tier: "(none configured)".to_owned(),
                hint: "set GRAFFY_MODEL_FAST / GRAFFY_MODEL_BALANCED / GRAFFY_MODEL_FRONTIER \
                       to 'provider:model' (providers: anthropic, openai, openrouter, venice, \
                       ollama), or run with --offline"
                    .to_owned(),
            });
        }

        let mut clients = HashMap::new();
        for binding in bindings.values() {
            if clients.contains_key(&binding.provider) {
                continue;
            }
            let handle = match binding.provider {
                ProviderKind::Anthropic => {
                    let key = require_env("ANTHROPIC_API_KEY")?;
                    ClientHandle::Anthropic(anthropic::Client::new(&key).map_err(provider_err)?)
                }
                ProviderKind::OpenAi => {
                    let key = require_env("OPENAI_API_KEY")?;
                    ClientHandle::OpenAi(openai::Client::new(&key).map_err(provider_err)?)
                }
                ProviderKind::OpenRouter => {
                    let key = require_env("OPENROUTER_API_KEY")?;
                    ClientHandle::OpenRouter(openrouter::Client::new(&key).map_err(provider_err)?)
                }
                ProviderKind::Venice => {
                    let key = require_env("VENICE_API_KEY")?;
                    ClientHandle::Venice(venice::Client::new(&key).map_err(provider_err)?)
                }
                ProviderKind::Ollama => {
                    let client = match std::env::var("OLLAMA_API_BASE_URL") {
                        Ok(url) if !url.is_empty() => ollama::Client::builder()
                            .api_key(Nothing)
                            .base_url(&url)
                            .build()
                            .map_err(provider_err)?,
                        _ => ollama::Client::new(Nothing).map_err(provider_err)?,
                    };
                    ClientHandle::Ollama(client)
                }
            };
            clients.insert(binding.provider, handle);
        }

        Ok(Self { bindings, clients })
    }

    /// Tier names currently bound (for doctor output).
    pub fn bound_tiers(&self) -> Vec<(String, String)> {
        let mut tiers: Vec<(String, String)> = self
            .bindings
            .iter()
            .map(|(tier, b)| (tier.clone(), format!("{}:{}", b.provider.name(), b.model)))
            .collect();
        tiers.sort();
        tiers
    }
}

fn require_env(name: &str) -> Result<String, ModelError> {
    std::env::var(name)
        .ok()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| ModelError::Provider(format!("{name} is not set")))
}

fn provider_err(err: impl std::fmt::Display) -> ModelError {
    ModelError::Provider(err.to_string())
}

async fn send_request<M>(model: M, request: &ModelRequest) -> Result<(String, u64, u64), ModelError>
where
    M: CompletionModel + Clone,
{
    let response = model
        .completion_request(request.prompt.as_str())
        .preamble(request.system.clone())
        .temperature_opt(request.temperature)
        .max_tokens_opt(request.max_tokens)
        .send()
        .await
        .map_err(provider_err)?;

    let mut text = String::new();
    for content in &response.choice {
        if let AssistantContent::Text(t) = content {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&t.text);
        }
    }
    if text.is_empty() {
        return Err(ModelError::Provider(
            "model returned no text content".to_owned(),
        ));
    }
    Ok((
        text,
        response.usage.input_tokens,
        response.usage.output_tokens,
    ))
}

#[async_trait::async_trait]
impl ModelInvoker for RigInvoker {
    async fn complete(&self, request: &ModelRequest) -> Result<ModelResponse, ModelError> {
        let binding = self
            .bindings
            .get(&request.tier)
            .ok_or_else(|| ModelError::UnboundTier {
                tier: request.tier.clone(),
                hint: format!(
                    "set GRAFFY_MODEL_{}=provider:model",
                    request.tier.to_ascii_uppercase()
                ),
            })?;
        let client = self.clients.get(&binding.provider).ok_or_else(|| {
            ModelError::Provider(format!("no client built for {}", binding.provider.name()))
        })?;

        let started = Instant::now();
        let (text, input_tokens, output_tokens) = match client {
            ClientHandle::Anthropic(c) => {
                send_request(c.completion_model(&binding.model), request).await?
            }
            ClientHandle::OpenAi(c) => {
                send_request(c.completion_model(&binding.model), request).await?
            }
            ClientHandle::OpenRouter(c) => {
                send_request(c.completion_model(&binding.model), request).await?
            }
            ClientHandle::Venice(c) => {
                send_request(c.completion_model(&binding.model), request).await?
            }
            ClientHandle::Ollama(c) => {
                send_request(c.completion_model(&binding.model), request).await?
            }
        };

        Ok(ModelResponse {
            provider: binding.provider.name().to_owned(),
            model: binding.model.clone(),
            text,
            input_tokens,
            output_tokens,
            // Honest zero: price tables land with the eval phase; recording a
            // guessed cost would poison every budget and benchmark after it.
            cost_usd: 0.0,
            latency_ms: started.elapsed().as_millis() as u64,
        })
    }

    fn tier_candidates(&self, tier: &str) -> Vec<String> {
        self.bindings
            .get(tier)
            .map(|b| vec![format!("{}:{}", b.provider.name(), b.model)])
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::ProviderKind;

    #[test]
    fn provider_names_roundtrip() {
        for kind in [
            ProviderKind::Anthropic,
            ProviderKind::OpenAi,
            ProviderKind::OpenRouter,
            ProviderKind::Venice,
            ProviderKind::Ollama,
        ] {
            assert_eq!(ProviderKind::parse(kind.name()), Some(kind));
        }
        assert_eq!(ProviderKind::parse("unknown"), None);
    }
}
