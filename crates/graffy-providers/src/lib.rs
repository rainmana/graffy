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
    output_limits: HashMap<String, u64>,
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

        let output_limits = bindings
            .keys()
            .map(|tier| {
                let key = format!("GRAFFY_MAX_OUTPUT_TOKENS_{}", tier.to_ascii_uppercase());
                let value = match std::env::var(&key) {
                    Ok(value) => Some(value),
                    Err(std::env::VarError::NotPresent) => None,
                    Err(_) => {
                        return Err(ModelError::Provider(format!("{key} must be valid Unicode")));
                    }
                };
                Ok((tier.clone(), parse_output_limit(&key, value.as_deref())?))
            })
            .collect::<Result<HashMap<_, _>, ModelError>>()?;

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

        Ok(Self {
            bindings,
            clients,
            output_limits,
        })
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

// An application default, not a claim about every model's capabilities.
// Each tier can override it to match the bound model.
const DEFAULT_MAX_OUTPUT_TOKENS: u64 = 4096;

fn parse_output_limit(key: &str, value: Option<&str>) -> Result<u64, ModelError> {
    match value {
        None => Ok(DEFAULT_MAX_OUTPUT_TOKENS),
        Some(value) => value
            .trim()
            .parse::<u64>()
            .ok()
            .filter(|n| *n > 0)
            .ok_or_else(|| ModelError::Provider(format!("{key} must be a positive integer"))),
    }
}

fn output_token_limit(request: &ModelRequest, configured_limit: u64) -> Result<u64, ModelError> {
    let Some(remaining) = request.remaining_token_budget else {
        return Ok(configured_limit);
    };
    // Reserve one token per UTF-8 byte plus message framing. This intentionally
    // conservative estimate is not provider usage or a universal tokenizer bound.
    // The executor still accounts for actual input + output reported by the provider.
    let input_reserve = (request.system.len() as u64)
        .saturating_add(request.prompt.len() as u64)
        .saturating_add(64);
    if remaining <= input_reserve {
        return Err(ModelError::TokenBudgetExhausted {
            remaining,
            input_reserve,
        });
    }
    Ok(configured_limit.min(remaining - input_reserve))
}

async fn send_request<M>(
    model: M,
    request: &ModelRequest,
    output_limit: u64,
) -> Result<(String, u64, u64), ModelError>
where
    M: CompletionModel + Clone,
{
    let max_tokens = output_token_limit(request, output_limit)?;
    let response = model
        .completion_request(request.prompt.as_str())
        .preamble(request.system.clone())
        .temperature_opt(request.temperature)
        .max_tokens(max_tokens)
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

        let output_limit = self.output_limits[&request.tier];
        let started = Instant::now();
        let (text, input_tokens, output_tokens) = match client {
            ClientHandle::Anthropic(c) => {
                send_request(c.completion_model(&binding.model), request, output_limit).await?
            }
            ClientHandle::OpenAi(c) => {
                send_request(c.completion_model(&binding.model), request, output_limit).await?
            }
            ClientHandle::OpenRouter(c) => {
                send_request(c.completion_model(&binding.model), request, output_limit).await?
            }
            ClientHandle::Venice(c) => {
                send_request(c.completion_model(&binding.model), request, output_limit).await?
            }
            ClientHandle::Ollama(c) => {
                send_request(c.completion_model(&binding.model), request, output_limit).await?
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

    #[derive(Clone, Default)]
    struct CappedModel {
        requests: std::sync::Arc<std::sync::Mutex<Vec<rig_core::completion::CompletionRequest>>>,
    }

    impl rig_core::completion::CompletionModel for CappedModel {
        async fn completion(
            &self,
            request: rig_core::completion::CompletionRequest,
        ) -> Result<rig_core::completion::CompletionResponse, rig_core::completion::CompletionError>
        {
            let cap = request
                .max_tokens
                .expect("a per-call output cap is required");
            self.requests.lock().unwrap().push(request);
            if cap == 0 || cap > 4096 {
                return Err(rig_core::completion::CompletionError::ProviderError(
                    "this fixture model supports 1..=4096 output tokens".into(),
                ));
            }
            Ok(rig_core::completion::CompletionResponse::new(
                vec![rig_core::completion::message::AssistantContent::text(
                    "PASS",
                )],
                rig_core::completion::Usage::default(),
                "fixture",
            ))
        }

        async fn stream(
            &self,
            _request: rig_core::completion::CompletionRequest,
        ) -> Result<
            rig_core::streaming::StreamingCompletionResponse,
            rig_core::completion::CompletionError,
        > {
            unreachable!("this adapter uses non-streaming completions")
        }
    }

    #[tokio::test]
    async fn default_run_budget_does_not_become_provider_output_limit() {
        let model = CappedModel::default();
        let request = graffy_core::exec::ModelRequest {
            tier: "balanced".into(),
            purpose: "draft".into(),
            system: "Be concise.".into(),
            prompt: "Hello".into(),
            temperature: None,
            remaining_token_budget: Some(200_000),
        };
        let response =
            super::send_request(model.clone(), &request, super::DEFAULT_MAX_OUTPUT_TOKENS)
                .await
                .unwrap();
        assert_eq!(response.0, "PASS");
        assert_eq!(model.requests.lock().unwrap()[0].max_tokens, Some(4096));
    }

    fn request_with_budget(remaining_token_budget: Option<u64>) -> graffy_core::exec::ModelRequest {
        graffy_core::exec::ModelRequest {
            tier: "balanced".into(),
            purpose: "draft".into(),
            system: "guide".into(),
            prompt: "é🙂".into(),
            temperature: None,
            remaining_token_budget,
        }
    }

    #[test]
    fn output_cap_configuration_requires_positive_integer() {
        let key = "GRAFFY_MAX_OUTPUT_TOKENS_BALANCED";
        assert_eq!(super::parse_output_limit(key, None).unwrap(), 4096);
        assert_eq!(super::parse_output_limit(key, Some(" 512 ")).unwrap(), 512);
        for invalid in ["", "0", "-1", "1.5", "nope", "18446744073709551616"] {
            let error = super::parse_output_limit(key, Some(invalid)).unwrap_err();
            assert!(error.to_string().contains(key));
            assert!(error.to_string().contains("positive integer"));
        }
    }

    #[tokio::test]
    async fn small_budgets_reserve_system_and_unicode_prompt_before_generation() {
        // 5 system bytes + 6 prompt bytes + 64 framing reserve = 75.
        for (budget, expected) in [(Some(160), 85), (Some(10_000), 512), (None, 512)] {
            let model = CappedModel::default();
            super::send_request(model.clone(), &request_with_budget(budget), 512)
                .await
                .unwrap();
            assert_eq!(model.requests.lock().unwrap()[0].max_tokens, Some(expected));
        }
    }

    #[tokio::test]
    async fn input_reserve_exhaustion_never_reaches_provider() {
        for remaining in [0, 74, 75] {
            let model = CappedModel::default();
            let error =
                super::send_request(model.clone(), &request_with_budget(Some(remaining)), 512)
                    .await
                    .unwrap_err();
            assert!(
                matches!(error, graffy_core::error::ModelError::TokenBudgetExhausted {
                remaining: actual, input_reserve: 75,
            } if actual == remaining)
            );
            assert!(model.requests.lock().unwrap().is_empty());
        }
    }

    #[async_trait::async_trait]
    impl graffy_core::exec::ModelInvoker for CappedModel {
        async fn complete(
            &self,
            request: &graffy_core::exec::ModelRequest,
        ) -> Result<graffy_core::exec::ModelResponse, graffy_core::error::ModelError> {
            let (text, input_tokens, output_tokens) =
                super::send_request(self.clone(), request, super::DEFAULT_MAX_OUTPUT_TOKENS)
                    .await?;
            Ok(graffy_core::exec::ModelResponse {
                provider: "fixture".into(),
                model: "capped".into(),
                text,
                input_tokens,
                output_tokens,
                cost_usd: 0.0,
                latency_ms: 0,
            })
        }
    }

    #[tokio::test]
    async fn graph_caps_each_completion_and_records_preflight_budget_exhaustion() {
        use graffy_core::{
            control::RemainingBudget,
            exec::{AutoApprove, Executor, RunInput},
            journal::{JournalReader, wire},
            spec::GraphSpec,
        };
        let spec_text = include_str!("../../../graphs/conversation.default.toml");
        let spec = GraphSpec::from_toml_str(spec_text).unwrap();
        assert_eq!(spec.policy.budget.max_tokens, Some(200_000));
        for (budget, status, expected_calls) in [
            (200_000, wire::RunStatus::Succeeded, 2),
            (64, wire::RunStatus::BudgetExhausted, 0),
        ] {
            let path = std::env::temp_dir().join(format!(
                "graffy-provider-{}-{budget}-{}.journal",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
            ));
            let model = CappedModel::default();
            let executor = Executor {
                remaining_budget: RemainingBudget {
                    tokens: Some(budget),
                    ..Default::default()
                },
                ..Default::default()
            };
            let out = executor
                .run(
                    &spec,
                    spec_text,
                    RunInput {
                        prompt: "Hello".into(),
                        ..Default::default()
                    },
                    &path,
                    &model,
                    &AutoApprove,
                )
                .await
                .unwrap();
            assert_eq!(out.status, status);
            assert_eq!(
                out.final_text.is_some(),
                status == wire::RunStatus::Succeeded
            );
            let requests = model.requests.lock().unwrap();
            assert_eq!(requests.len(), expected_calls);
            assert!(requests.iter().all(|r| r.max_tokens == Some(4096)));
            let events = JournalReader::read_all(&path).unwrap();
            assert!(events.iter().any(|e| matches!(
                &e.event,
                Some(wire::run_event::Event::RunFinished(f)) if f.status == status as i32
            )));
            if status == wire::RunStatus::BudgetExhausted {
                assert!(
                    out.notes
                        .iter()
                        .any(|note| note.contains("reserved for input"))
                );
            }
            std::fs::remove_file(path).unwrap();
        }
    }
}
