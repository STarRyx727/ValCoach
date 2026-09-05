use std::{env, fmt, sync::Arc, time::Duration};

use axum::{
    Json,
    extract::{Path, State},
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use uuid::Uuid;
use valcoach_db::{
    AgentMessageRecord, AgentTokenUsage, AgentUsageSummary, Database, DatabaseError,
};

use crate::{
    AppState,
    auth::{AuthApiError, require_user_id},
};

const MAX_QUESTION_BYTES: usize = 4_000;
const SYSTEM_PROMPT: &str = r#"You are ValCoach, an evidence-grounded VALORANT replay coach.
Use only facts in <replay_context>; never invent missing replay facts, player identity, units, rounds, kills, or causes.
Check the capability map before making each factual claim. If a capability is partial or unsupported, state the limitation.
Separate observed facts from coaching recommendations. Cite applicable evidence using its exact match_id, player_id, timestamp_ms, and evidence_type.
Answer in the language used by the player. Be concise and actionable."#;

#[derive(Clone)]
pub struct AgentService {
    database: Database,
    provider: Option<Arc<LlmProvider>>,
}

impl fmt::Debug for AgentService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgentService")
            .field("database", &self.database)
            .field("configured", &self.provider.is_some())
            .finish()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentStatus {
    pub configured: bool,
    pub provider: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CoachRequest {
    pub question: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CoachResponse {
    pub session_id: String,
    pub provider: String,
    pub model: String,
    pub answer: String,
    pub evidence: Vec<Value>,
    pub limitations: Vec<String>,
    pub usage: AgentTokenUsage,
}

impl AgentService {
    pub fn from_env(database: Database) -> Result<Self, AgentError> {
        let Some(provider_name) = optional_env("VALCOACH_LLM_PROVIDER") else {
            return Ok(Self {
                database,
                provider: None,
            });
        };
        let kind = ProviderKind::parse(&provider_name)?;
        let model = required_env("VALCOACH_LLM_MODEL")?;
        let api_key = optional_env("VALCOACH_LLM_API_KEY")
            .or_else(|| optional_env(kind.default_key_variable()))
            .ok_or_else(|| {
                AgentError::Configuration(format!(
                    "{} or VALCOACH_LLM_API_KEY is required",
                    kind.default_key_variable()
                ))
            })?;
        let base_url = optional_env("VALCOACH_LLM_BASE_URL")
            .unwrap_or_else(|| kind.default_base_url().to_owned());
        if !base_url.starts_with("https://") && !base_url.starts_with("http://127.0.0.1") {
            return Err(AgentError::Configuration(
                "VALCOACH_LLM_BASE_URL must use HTTPS (or loopback HTTP for local testing)"
                    .to_owned(),
            ));
        }
        let max_output_tokens = optional_env("VALCOACH_LLM_MAX_OUTPUT_TOKENS")
            .map(|value| parse_positive_u32("VALCOACH_LLM_MAX_OUTPUT_TOKENS", &value))
            .transpose()?
            .unwrap_or(800);
        let input_price = optional_env("VALCOACH_LLM_INPUT_USD_PER_MILLION")
            .map(|value| parse_non_negative_f64("VALCOACH_LLM_INPUT_USD_PER_MILLION", &value))
            .transpose()?;
        let output_price = optional_env("VALCOACH_LLM_OUTPUT_USD_PER_MILLION")
            .map(|value| parse_non_negative_f64("VALCOACH_LLM_OUTPUT_USD_PER_MILLION", &value))
            .transpose()?;
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()?;
        Ok(Self {
            database,
            provider: Some(Arc::new(LlmProvider {
                client,
                kind,
                model,
                base_url: base_url.trim_end_matches('/').to_owned(),
                api_key,
                max_output_tokens,
                input_price,
                output_price,
            })),
        })
    }

    #[cfg(test)]
    pub fn disabled(database: Database) -> Self {
        Self {
            database,
            provider: None,
        }
    }

    pub fn status(&self) -> AgentStatus {
        AgentStatus {
            configured: self.provider.is_some(),
            provider: self
                .provider
                .as_ref()
                .map(|provider| provider.kind.name().to_owned()),
            model: self
                .provider
                .as_ref()
                .map(|provider| provider.model.clone()),
        }
    }

    pub async fn coach(
        &self,
        user_id: &str,
        match_id: &str,
        question: &str,
    ) -> Result<CoachResponse, AgentError> {
        let question = question.trim();
        if question.is_empty() || question.len() > MAX_QUESTION_BYTES {
            return Err(AgentError::InvalidQuestion);
        }
        let provider = self.provider.as_ref().ok_or(AgentError::Disabled)?;
        let replay = self
            .database
            .find_match_for_user(user_id, match_id)
            .await?
            .ok_or(AgentError::MatchNotFound)?;
        let selected_player = self
            .database
            .find_bound_player_for_match(user_id, match_id)
            .await?;
        let metrics = self
            .database
            .list_match_metrics_for_user(user_id, match_id)
            .await?;

        let mut evidence = Vec::new();
        let mut limitations = Vec::new();
        let selected_metrics: Vec<Value> = if let Some(player_id) = selected_player.as_deref() {
            metrics
                .into_iter()
                .filter_map(|metric| {
                    let value: Value = serde_json::from_str(&metric.value_json).ok()?;
                    let applies = evidence_values(&value).iter().any(|item| {
                        item.get("player_id").and_then(Value::as_str) == Some(player_id)
                    });
                    if !applies {
                        return None;
                    }
                    evidence.extend(evidence_values(&value));
                    limitations.extend(limitation_values(&value));
                    Some(json!({ "metric_name": metric.metric_name, "value": value }))
                })
                .collect()
        } else {
            limitations.push(
                "No replay player is bound to this account; personalized movement evidence is unavailable."
                    .to_owned(),
            );
            Vec::new()
        };
        limitations.sort();
        limitations.dedup();
        evidence.sort_by_key(|left| left.to_string());
        evidence.dedup();

        let context = json!({
            "match_id": replay.id,
            "metadata": replay.metadata,
            "capabilities": replay.capabilities,
            "summary": replay.summary,
            "selected_player_id": selected_player,
            "deterministic_metrics": selected_metrics,
            "limitations": limitations,
        });
        let input = format!(
            "<replay_context>\n{}\n</replay_context>\n<player_question>\n{}\n</player_question>",
            serde_json::to_string_pretty(&context)?,
            question
        );
        let reply = provider.complete(SYSTEM_PROMPT, &input).await?;
        let session_id = Uuid::new_v4().to_string();
        self.database
            .insert_agent_exchange(
                user_id,
                match_id,
                &session_id,
                provider.kind.name(),
                &provider.model,
                question,
                &reply.text,
                &serde_json::to_string(&evidence)?,
                &serde_json::to_string(&limitations)?,
                reply.request_id.as_deref(),
                &reply.usage,
            )
            .await?;
        Ok(CoachResponse {
            session_id,
            provider: provider.kind.name().to_owned(),
            model: provider.model.clone(),
            answer: reply.text,
            evidence,
            limitations,
            usage: reply.usage,
        })
    }
}

#[derive(Clone)]
struct LlmProvider {
    client: Client,
    kind: ProviderKind,
    model: String,
    base_url: String,
    api_key: String,
    max_output_tokens: u32,
    input_price: Option<f64>,
    output_price: Option<f64>,
}

impl LlmProvider {
    async fn complete(&self, instructions: &str, input: &str) -> Result<ProviderReply, AgentError> {
        let mut reply = match self.kind {
            ProviderKind::OpenAi => self.complete_openai(instructions, input).await?,
            ProviderKind::Anthropic => self.complete_anthropic(instructions, input).await?,
            ProviderKind::DeepSeek | ProviderKind::OpenAiCompatible => {
                self.complete_chat_completions(instructions, input).await?
            }
        };
        reply.usage.cost_microusd =
            estimate_cost(&reply.usage, self.input_price, self.output_price);
        Ok(reply)
    }

    async fn complete_openai(
        &self,
        instructions: &str,
        input: &str,
    ) -> Result<ProviderReply, AgentError> {
        let value = self
            .send(
                self.client
                    .post(format!("{}/responses", self.base_url))
                    .bearer_auth(&self.api_key)
                    .json(&json!({
                        "model": self.model,
                        "instructions": instructions,
                        "input": input,
                        "max_output_tokens": self.max_output_tokens,
                        "store": false
                    })),
            )
            .await?;
        parse_openai_response(&value)
    }

    async fn complete_anthropic(
        &self,
        instructions: &str,
        input: &str,
    ) -> Result<ProviderReply, AgentError> {
        let value = self
            .send(
                self.client
                    .post(format!("{}/messages", self.base_url))
                    .header("x-api-key", &self.api_key)
                    .header("anthropic-version", "2023-06-01")
                    .json(&json!({
                        "model": self.model,
                        "system": instructions,
                        "messages": [{ "role": "user", "content": input }],
                        "max_tokens": self.max_output_tokens
                    })),
            )
            .await?;
        parse_anthropic_response(&value)
    }

    async fn complete_chat_completions(
        &self,
        instructions: &str,
        input: &str,
    ) -> Result<ProviderReply, AgentError> {
        let value = self
            .send(
                self.client
                    .post(format!("{}/chat/completions", self.base_url))
                    .bearer_auth(&self.api_key)
                    .json(&json!({
                        "model": self.model,
                        "messages": [
                            { "role": "system", "content": instructions },
                            { "role": "user", "content": input }
                        ],
                        "max_tokens": self.max_output_tokens,
                        "stream": false
                    })),
            )
            .await?;
        parse_chat_completion(&value)
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Result<Value, AgentError> {
        let response = request.send().await?;
        let status = response.status();
        let bytes = response.bytes().await?;
        if !status.is_success() {
            let detail = String::from_utf8_lossy(&bytes);
            return Err(AgentError::Provider(format!(
                "provider returned HTTP {status}: {}",
                detail.chars().take(500).collect::<String>()
            )));
        }
        Ok(serde_json::from_slice(&bytes)?)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderKind {
    OpenAi,
    Anthropic,
    DeepSeek,
    OpenAiCompatible,
}

impl ProviderKind {
    fn parse(value: &str) -> Result<Self, AgentError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "openai" => Ok(Self::OpenAi),
            "anthropic" | "claude" => Ok(Self::Anthropic),
            "deepseek" => Ok(Self::DeepSeek),
            "openai-compatible" | "openai_compatible" => Ok(Self::OpenAiCompatible),
            _ => Err(AgentError::Configuration(
                "VALCOACH_LLM_PROVIDER must be openai, anthropic, deepseek, or openai-compatible"
                    .to_owned(),
            )),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::DeepSeek => "deepseek",
            Self::OpenAiCompatible => "openai-compatible",
        }
    }

    fn default_base_url(self) -> &'static str {
        match self {
            Self::OpenAi => "https://api.openai.com/v1",
            Self::Anthropic => "https://api.anthropic.com/v1",
            Self::DeepSeek => "https://api.deepseek.com",
            Self::OpenAiCompatible => "",
        }
    }

    fn default_key_variable(self) -> &'static str {
        match self {
            Self::OpenAi => "OPENAI_API_KEY",
            Self::Anthropic => "ANTHROPIC_API_KEY",
            Self::DeepSeek => "DEEPSEEK_API_KEY",
            Self::OpenAiCompatible => "VALCOACH_LLM_API_KEY",
        }
    }
}

#[derive(Debug)]
struct ProviderReply {
    text: String,
    request_id: Option<String>,
    usage: AgentTokenUsage,
}

fn parse_openai_response(value: &Value) -> Result<ProviderReply, AgentError> {
    let text = value
        .get("output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("content").and_then(Value::as_array))
        .flatten()
        .filter_map(|content| {
            (content.get("type").and_then(Value::as_str) == Some("output_text"))
                .then(|| content.get("text").and_then(Value::as_str))
                .flatten()
        })
        .collect::<Vec<_>>()
        .join("\n");
    let usage = value.get("usage").unwrap_or(&Value::Null);
    provider_reply(
        value,
        text,
        token(usage, "input_tokens"),
        token(usage, "output_tokens"),
        token(usage, "total_tokens"),
    )
}

fn parse_anthropic_response(value: &Value) -> Result<ProviderReply, AgentError> {
    let text = value
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|content| {
            (content.get("type").and_then(Value::as_str) == Some("text"))
                .then(|| content.get("text").and_then(Value::as_str))
                .flatten()
        })
        .collect::<Vec<_>>()
        .join("\n");
    let usage = value.get("usage").unwrap_or(&Value::Null);
    let input = token(usage, "input_tokens")
        .saturating_add(token(usage, "cache_creation_input_tokens"))
        .saturating_add(token(usage, "cache_read_input_tokens"));
    let output = token(usage, "output_tokens");
    provider_reply(value, text, input, output, input.saturating_add(output))
}

fn parse_chat_completion(value: &Value) -> Result<ProviderReply, AgentError> {
    let text = value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let usage = value.get("usage").unwrap_or(&Value::Null);
    provider_reply(
        value,
        text,
        token(usage, "prompt_tokens"),
        token(usage, "completion_tokens"),
        token(usage, "total_tokens"),
    )
}

fn provider_reply(
    value: &Value,
    text: String,
    input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
) -> Result<ProviderReply, AgentError> {
    if text.trim().is_empty() {
        return Err(AgentError::Provider(
            "provider response contained no assistant text".to_owned(),
        ));
    }
    Ok(ProviderReply {
        text,
        request_id: value.get("id").and_then(Value::as_str).map(str::to_owned),
        usage: AgentTokenUsage {
            input_tokens,
            output_tokens,
            total_tokens,
            cost_microusd: None,
        },
    })
}

fn token(value: &Value, field: &str) -> u64 {
    value.get(field).and_then(Value::as_u64).unwrap_or_default()
}

fn estimate_cost(
    usage: &AgentTokenUsage,
    input_price: Option<f64>,
    output_price: Option<f64>,
) -> Option<u64> {
    let (input_price, output_price) = (input_price?, output_price?);
    Some(
        (usage.input_tokens as f64 * input_price + usage.output_tokens as f64 * output_price)
            .round()
            .max(0.0) as u64,
    )
}

fn evidence_values(metric: &Value) -> Vec<Value> {
    metric
        .pointer("/data/evidence")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn limitation_values(metric: &Value) -> Vec<String> {
    metric
        .get("limitations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn optional_env(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

fn required_env(name: &str) -> Result<String, AgentError> {
    optional_env(name).ok_or_else(|| AgentError::Configuration(format!("{name} is required")))
}

fn parse_positive_u32(name: &str, value: &str) -> Result<u32, AgentError> {
    value
        .parse()
        .ok()
        .filter(|number| *number > 0)
        .ok_or_else(|| AgentError::Configuration(format!("{name} must be a positive integer")))
}

fn parse_non_negative_f64(name: &str, value: &str) -> Result<f64, AgentError> {
    value
        .parse::<f64>()
        .ok()
        .filter(|number| number.is_finite() && *number >= 0.0)
        .ok_or_else(|| AgentError::Configuration(format!("{name} must be a non-negative number")))
}

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("LLM provider is not configured")]
    Disabled,
    #[error("question must contain 1-{MAX_QUESTION_BYTES} bytes")]
    InvalidQuestion,
    #[error("match was not found for this user")]
    MatchNotFound,
    #[error("invalid LLM configuration: {0}")]
    Configuration(String),
    #[error("LLM HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("LLM provider request failed: {0}")]
    Provider(String),
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error("failed to serialize Agent context: {0}")]
    Serialization(#[from] serde_json::Error),
}

pub async fn status(State(state): State<AppState>) -> Json<AgentStatus> {
    Json(state.agent.status())
}

pub async fn coach_match(
    State(state): State<AppState>,
    session: tower_sessions::Session,
    Path(match_id): Path<String>,
    Json(request): Json<CoachRequest>,
) -> Result<Json<CoachResponse>, AuthApiError> {
    let user_id = require_user_id(&state.auth, &session).await?;
    state
        .agent
        .coach(&user_id, &match_id, &request.question)
        .await
        .map(Json)
        .map_err(agent_api_error)
}

pub async fn history(
    State(state): State<AppState>,
    session: tower_sessions::Session,
    Path(match_id): Path<String>,
) -> Result<Json<Vec<AgentMessageRecord>>, AuthApiError> {
    let user_id = require_user_id(&state.auth, &session).await?;
    state
        .auth
        .database
        .list_agent_messages_for_match(&user_id, &match_id)
        .await
        .map(Json)
        .map_err(|error| AuthApiError::internal(error.to_string()))
}

pub async fn usage(
    State(state): State<AppState>,
    session: tower_sessions::Session,
) -> Result<Json<AgentUsageSummary>, AuthApiError> {
    let user_id = require_user_id(&state.auth, &session).await?;
    state
        .auth
        .database
        .agent_usage_for_user(&user_id)
        .await
        .map(Json)
        .map_err(|error| AuthApiError::internal(error.to_string()))
}

fn agent_api_error(error: AgentError) -> AuthApiError {
    match error {
        AgentError::Disabled | AgentError::InvalidQuestion => {
            AuthApiError::bad_request(error.to_string())
        }
        AgentError::MatchNotFound | AgentError::Database(DatabaseError::MatchNotFound) => {
            AuthApiError::unauthorized()
        }
        other => {
            tracing::error!(error = %other, "replay coaching request failed");
            AuthApiError::internal("replay coaching request failed")
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use axum::{Json, Router, routing::post};
    use serde_json::json;
    use valcoach_db::{Database, UserRecord};
    use valcoach_domain::{
        CapabilityLevel, ParsedBundle, ParsedReplay, ParsedReplaySummary, ReplayCapabilities,
        ReplayMetadata,
    };

    use super::{
        AgentService, AgentTokenUsage, LlmProvider, ProviderKind, estimate_cost,
        parse_anthropic_response, parse_chat_completion, parse_openai_response,
    };

    #[test]
    fn provider_payloads_report_text_and_tokens() {
        let openai = parse_openai_response(&json!({
            "id":"resp-1",
            "output":[{"content":[{"type":"output_text","text":"OpenAI"}]}],
            "usage":{"input_tokens":10,"output_tokens":4,"total_tokens":14}
        }))
        .expect("OpenAI response");
        assert_eq!(openai.text, "OpenAI");
        assert_eq!(openai.usage.total_tokens, 14);

        let anthropic = parse_anthropic_response(&json!({
            "id":"msg-1",
            "content":[{"type":"text","text":"Claude"}],
            "usage":{"input_tokens":10,"cache_read_input_tokens":3,"output_tokens":5}
        }))
        .expect("Anthropic response");
        assert_eq!(anthropic.usage.input_tokens, 13);
        assert_eq!(anthropic.usage.total_tokens, 18);

        let compatible = parse_chat_completion(&json!({
            "id":"chat-1",
            "choices":[{"message":{"content":"DeepSeek"}}],
            "usage":{"prompt_tokens":8,"completion_tokens":2,"total_tokens":10}
        }))
        .expect("chat completion");
        assert_eq!(compatible.text, "DeepSeek");
    }

    #[test]
    fn provider_selection_and_optional_cost_are_deterministic() {
        assert_eq!(
            ProviderKind::parse("claude").expect("alias"),
            ProviderKind::Anthropic
        );
        let usage = AgentTokenUsage {
            input_tokens: 1_000,
            output_tokens: 200,
            total_tokens: 1_200,
            cost_microusd: None,
        };
        assert_eq!(estimate_cost(&usage, Some(2.5), Some(10.0)), Some(4_500));
        assert_eq!(estimate_cost(&usage, None, Some(10.0)), None);
    }

    #[tokio::test]
    async fn grounded_openai_request_persists_answer_and_usage() {
        let mock = Router::new().route(
            "/responses",
            post(|Json(body): Json<serde_json::Value>| async move {
                assert_eq!(body["store"], false);
                assert!(body["input"].as_str().is_some_and(|text| text.contains("match-1")));
                Json(json!({
                    "id":"resp-mock",
                    "output":[{"content":[{"type":"output_text","text":"Only observed evidence is used."}]}],
                    "usage":{"input_tokens":21,"output_tokens":6,"total_tokens":27}
                }))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock listener");
        let address = listener.local_addr().expect("mock address");
        tokio::spawn(async move { axum::serve(listener, mock).await.expect("mock server") });

        let database = Database::connect("sqlite::memory:")
            .await
            .expect("database");
        database
            .create_user(&UserRecord {
                id: "user-1".to_owned(),
                username: "agent_user".to_owned(),
                password_hash: "hash".to_owned(),
            })
            .await
            .expect("user");
        database
            .insert_match_summary(
                "user-1",
                "match-1",
                &ParsedReplay {
                    metadata: ReplayMetadata {
                        replay_id: "replay-1".to_owned(),
                        branch: Some("++Ares-Core+release-13.05".to_owned()),
                        map: Some("/Game/Maps/Bonsai/Bonsai".to_owned()),
                        duration_ms: Some(1_000),
                    },
                    bundle: ParsedBundle {
                        events_path: "events.ndjson".into(),
                        movement_path: "movement.ndjson".into(),
                    },
                    source_name: "test".to_owned(),
                    capabilities: ReplayCapabilities::global_fixture(CapabilityLevel::Partial),
                    summary: ParsedReplaySummary::default(),
                },
            )
            .await
            .expect("match");
        let service = AgentService {
            database: database.clone(),
            provider: Some(Arc::new(LlmProvider {
                client: reqwest::Client::builder()
                    .timeout(Duration::from_secs(5))
                    .build()
                    .expect("client"),
                kind: ProviderKind::OpenAi,
                model: "mock-model".to_owned(),
                base_url: format!("http://{address}"),
                api_key: "test-key".to_owned(),
                max_output_tokens: 100,
                input_price: Some(1.0),
                output_price: Some(2.0),
            })),
        };
        let answer = service
            .coach("user-1", "match-1", "How should I improve?")
            .await
            .expect("coach answer");
        assert_eq!(answer.usage.total_tokens, 27);
        assert_eq!(answer.usage.cost_microusd, Some(33));
        assert_eq!(answer.limitations.len(), 1);
        let history = database
            .list_agent_messages_for_match("user-1", "match-1")
            .await
            .expect("history");
        assert_eq!(history.len(), 2);
        assert_eq!(history[1].usage.total_tokens, 27);
    }
}
