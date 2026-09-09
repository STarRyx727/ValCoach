use std::{collections::HashMap, env, fmt, sync::Arc, time::Duration};

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use valcoach_db::{
    AgentMessageRecord, AgentTokenUsage, AgentUsageSummary, Database, DatabaseError,
};

use crate::{
    AppState,
    auth::{AuthApiError, require_user_id},
};

const MAX_QUESTION_BYTES: usize = 4_000;
const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 32_768;
const MAX_MAX_OUTPUT_TOKENS: u32 = 393_216;
const MAX_CONTEXT_BYTES: usize = 96_000;
const MAX_HISTORY_MESSAGES: usize = 6;
const MAX_HISTORY_CHARS_PER_MESSAGE: usize = 1_600;
const SYSTEM_PROMPT: &str = r#"You are ValCoach, an evidence-grounded VALORANT replay coach.
Use only facts in <replay_context>; never invent missing replay facts, player identity, units, rounds, kills, or causes.
Read retrieval_mode first. When it is personal_history_only, do not analyze or imply facts from the current match. Use only personal_issues and recent_coaching_exchanges. If structured issues are empty but prior exchanges exist, summarize those prior coaching findings and say they are not yet confirmed as recurring issues.
Check the capability map before making each factual claim. If a capability is partial or unsupported, state the limitation.
Separate observed facts from coaching recommendations. Cite applicable evidence using its exact match_id, player_id, human_time, and evidence_type.
Always use human_time (format "R8 00:26.1") when referring to timestamps. Never use raw milliseconds.
When referring to positions, use the "area" field (e.g. "A Site", "A Main") rather than raw coordinates.
Only mention a missing capability when it is directly relevant to the player's question; do not lead with generic limitations.
Abilities include both ultimate events from server and ability-use actor spawns from the parser.
Always use official VALORANT agent display names, never internal codenames.
The "agent" field can contain replay codenames. Translate them to official English display names. In particular Pine is Veto, Nox is Vyse, Iris is Miks, Cashew is Tejo, Terra is Waylay, and AggroBot is Gekko.
Similarly, map names must use display names: Bonsai->Split, Duality->Bind, Triad->Haven, Juliett->Sunset, Jam->Lotus, Pitt->Pearl, Canyon->Fracture, Foxtrot->Breeze, Port->Icebox, Infinity->Abyss, Rook->Corrode, Plummet->Summit.
Weapon names in shot events may be null or use internal names; use common names (e.g. "Classic", "Vandal", "Phantom") when available.
If personal_issues are present in the context, relate current observations to known recurring problems and mention trends. An occurrence count of 1 is an initial observation, not yet a recurring pattern.
Treat player_profile only as user-provided preferences and training background, never as replay evidence. Tailor explanations and drills to the player's rank, main role, agents, and training goals when present. Never infer a missing profile field.
For match_analysis mode, whenever the answer identifies an evidence-backed weakness or actionable tactical problem, append one machine-readable block in exactly this form: <coaching_issues>[...]</coaching_issues>. Each object must contain issue_key, category, title, description, map, side, area, severity (0-1), and confidence (0-1). Use a short stable snake_case issue_key so the same problem can be matched in later games. Emit an empty array inside the paired tags if no problem was identified. Do not wrap this JSON in markdown fences. Never emit this block in personal_history_only mode.
Answer in the language used by the player. Be concise and actionable. Use markdown formatting (headers, bold, lists) for readability."#;

#[derive(Clone)]
pub struct AgentService {
    database: Database,
    default_provider: Option<Arc<LlmProvider>>,
    user_providers: Arc<RwLock<HashMap<String, Arc<LlmProvider>>>>,
    active_requests: Arc<Mutex<HashMap<(String, String), CancellationToken>>>,
}

impl fmt::Debug for AgentService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgentService")
            .field("database", &self.database)
            .field("environment_configured", &self.default_provider.is_some())
            .finish()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentStatus {
    pub configured: bool,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub source: Option<String>,
    pub api_key_in_memory: bool,
    pub max_output_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct AgentSettingsRequest {
    pub provider: String,
    pub model: String,
    pub api_key: String,
    pub base_url: Option<String>,
    pub max_output_tokens: Option<u32>,
    pub input_usd_per_million: Option<f64>,
    pub output_usd_per_million: Option<f64>,
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
        let default_provider = provider_from_env()?.map(Arc::new);
        Ok(Self {
            database,
            default_provider,
            user_providers: Arc::new(RwLock::new(HashMap::new())),
            active_requests: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    #[cfg(test)]
    pub fn disabled(database: Database) -> Self {
        Self {
            database,
            default_provider: None,
            user_providers: Arc::new(RwLock::new(HashMap::new())),
            active_requests: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn provider_for(&self, user_id: &str) -> Option<(Arc<LlmProvider>, &'static str)> {
        if let Some(provider) = self.user_providers.read().await.get(user_id).cloned() {
            Some((provider, "web"))
        } else {
            self.default_provider
                .as_ref()
                .cloned()
                .map(|provider| (provider, "environment"))
        }
    }

    pub async fn status_for(&self, user_id: &str) -> AgentStatus {
        let Some((provider, source)) = self.provider_for(user_id).await else {
            return AgentStatus {
                configured: false,
                provider: None,
                model: None,
                source: None,
                api_key_in_memory: false,
                max_output_tokens: None,
            };
        };
        AgentStatus {
            configured: true,
            provider: Some(provider.kind.name().to_owned()),
            model: Some(provider.model.clone()),
            source: Some(source.to_owned()),
            api_key_in_memory: source == "web",
            max_output_tokens: Some(provider.max_output_tokens),
        }
    }

    pub async fn configure_for(
        &self,
        user_id: &str,
        settings: AgentSettingsRequest,
    ) -> Result<AgentStatus, AgentError> {
        let provider = Arc::new(LlmProvider::from_settings(settings)?);
        self.user_providers
            .write()
            .await
            .insert(user_id.to_owned(), provider);
        Ok(self.status_for(user_id).await)
    }

    pub async fn clear_for(&self, user_id: &str) -> AgentStatus {
        self.user_providers.write().await.remove(user_id);
        self.status_for(user_id).await
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
        let (provider, _) = self
            .provider_for(user_id)
            .await
            .ok_or(AgentError::Disabled)?;
        let replay = self
            .database
            .find_match_for_user(user_id, match_id)
            .await?
            .ok_or(AgentError::MatchNotFound)?;
        let retrieval_mode = question_retrieval_mode(question);
        let selected_player = self
            .database
            .find_bound_player_for_match(user_id, match_id)
            .await?;
        let mut evidence = Vec::new();
        let mut limitations = Vec::new();
        let scope = question_scope(question);
        let personal_issues = self.database.list_player_issues(user_id).await?;
        let player_profile = self.database.user_profile(user_id).await?;
        let previous = self
            .database
            .list_agent_messages_for_match(user_id, match_id)
            .await?;

        let (selected_metrics, semantic_context) = if retrieval_mode == RetrievalMode::MatchAnalysis
        {
            let metrics = self
                .database
                .list_match_metrics_for_user(user_id, match_id)
                .await?;
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
            let semantic_context = if let Some(player_id) = selected_player.as_deref() {
                let semantic = self
                    .database
                    .build_semantic_coaching_context(
                        user_id,
                        match_id,
                        player_id,
                        scope.round_no,
                        scope.area.as_deref(),
                        scope.side.as_deref(),
                    )
                    .await?;
                evidence.extend(semantic.evidence);
                limitations.extend(semantic.limitations);
                Some(semantic.context)
            } else {
                None
            };
            (selected_metrics, semantic_context)
        } else {
            (Vec::new(), None)
        };
        limitations.sort();
        limitations.dedup();
        if !evidence.is_empty() {
            evidence.sort_by_key(|v| v.to_string());
            evidence.dedup();
            evidence.truncate(96);
        }
        let context = if retrieval_mode == RetrievalMode::PersonalHistoryOnly {
            let recent = self
                .database
                .recent_coaching_exchanges_for_user(user_id, 8)
                .await?
                .into_iter()
                .map(|mut exchange| {
                    if let Some(answer) = exchange.get_mut("answer")
                        && let Some(text) = answer.as_str()
                    {
                        *answer = Value::String(extract_coaching_issues(text).0);
                    }
                    exchange
                })
                .collect::<Vec<_>>();
            json!({
                "retrieval_mode": retrieval_mode.as_str(),
                "personal_issues": personal_issues,
                "recent_coaching_exchanges": recent,
                "player_profile": player_profile,
                "current_match_loaded": false,
            })
        } else {
            json!({
                "retrieval_mode": retrieval_mode.as_str(),
                "match_id": replay.id,
                "metadata": {
                    "replay_id": replay.metadata.replay_id,
                    "branch": replay.metadata.branch,
                    "map": replay.metadata.map.as_deref().map(valcoach_domain::map_display_name),
                    "map_raw": replay.metadata.map,
                    "duration_ms": replay.metadata.duration_ms,
                },
                "capabilities": replay.capabilities,
                "summary": replay.summary,
                "selected_player_id": selected_player,
                "deterministic_metrics": selected_metrics,
                "semantic_replay": semantic_context,
                "personal_issues": personal_issues,
                "player_profile": player_profile,
                "limitations": limitations,
            })
        };
        let conversation_history = if retrieval_mode == RetrievalMode::PersonalHistoryOnly {
            Vec::new()
        } else {
            let history_start = previous.len().saturating_sub(MAX_HISTORY_MESSAGES);
            previous[history_start..]
                .iter()
                .map(|message| {
                    json!({
                        "role": message.role,
                        "content": message.content.chars().take(MAX_HISTORY_CHARS_PER_MESSAGE).collect::<String>(),
                    })
                })
                .collect::<Vec<_>>()
        };
        let serialized_context = serialize_context_with_budget(context)?;
        let input = format!(
            "<replay_context>\n{}\n</replay_context>\n<conversation_history>\n{}\n</conversation_history>\n<player_question>\n{}\n</player_question>",
            serialized_context,
            serde_json::to_string(&conversation_history)?,
            question
        );
        let request_key = (user_id.to_owned(), match_id.to_owned());
        let cancel = CancellationToken::new();
        if let Some(previous) = self
            .active_requests
            .lock()
            .await
            .insert(request_key.clone(), cancel.clone())
        {
            previous.cancel();
        }
        let reply_result = tokio::select! {
            reply = provider.complete(SYSTEM_PROMPT, &input) => reply,
            _ = cancel.cancelled() => Err(AgentError::Cancelled),
        };
        self.active_requests.lock().await.remove(&request_key);
        let reply = reply_result?;
        let session_id = Uuid::new_v4().to_string();
        let (clean_answer, extracted_issues) = extract_coaching_issues(&reply.text);
        if retrieval_mode == RetrievalMode::MatchAnalysis {
            for issue in &extracted_issues {
                if let Err(error) = self
                    .database
                    .upsert_player_issue(
                        user_id,
                        &issue.issue_key,
                        &issue.category,
                        &issue.title,
                        issue.description.as_deref(),
                        issue.map.as_deref(),
                        issue.side.as_deref(),
                        issue.area.as_deref(),
                        issue.severity,
                        issue.confidence,
                        Some(match_id),
                        None,
                        None,
                    )
                    .await
                {
                    tracing::error!(%error, issue_key = %issue.issue_key, "failed to save coaching issue");
                    limitations.push(
                        "A coaching issue was identified but could not be added to personal history."
                            .to_owned(),
                    );
                }
            }
        }
        self.database
            .insert_agent_exchange(
                user_id,
                match_id,
                &session_id,
                provider.kind.name(),
                &provider.model,
                question,
                &clean_answer,
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
            answer: clean_answer,
            evidence,
            limitations,
            usage: reply.usage,
        })
    }

    pub async fn cancel(&self, user_id: &str, match_id: &str) {
        if let Some(token) = self
            .active_requests
            .lock()
            .await
            .remove(&(user_id.to_owned(), match_id.to_owned()))
        {
            token.cancel();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetrievalMode {
    MatchAnalysis,
    PersonalHistoryOnly,
}

impl RetrievalMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::MatchAnalysis => "match_analysis",
            Self::PersonalHistoryOnly => "personal_history_only",
        }
    }
}

fn question_retrieval_mode(question: &str) -> RetrievalMode {
    let lower = question.to_ascii_lowercase();
    let explicitly_excludes_match = [
        "不看这局",
        "不看本局",
        "忽略这局",
        "忽略本局",
        "只分析历史",
        "只看历史",
        "不要分析这局",
        "不要看这局",
        "ignore this match",
        "without this match",
        "history only",
        "historical issues only",
    ]
    .iter()
    .any(|term| question.contains(term) || lower.contains(term));
    let asks_for_history = [
        "历史问题",
        "长期问题",
        "过往问题",
        "以前的问题",
        "常见问题",
        "历史记录",
        "historical issue",
        "past issue",
        "recurring issue",
        "coaching history",
    ]
    .iter()
    .any(|term| question.contains(term) || lower.contains(term));
    let explicitly_compares_current_match = [
        "这局相比",
        "本局相比",
        "结合这局",
        "结合本局",
        "compare this match",
        "current match compared",
    ]
    .iter()
    .any(|term| question.contains(term) || lower.contains(term));
    if explicitly_excludes_match || (asks_for_history && !explicitly_compares_current_match) {
        RetrievalMode::PersonalHistoryOnly
    } else {
        RetrievalMode::MatchAnalysis
    }
}

fn serialize_context_with_budget(mut context: Value) -> Result<String, serde_json::Error> {
    loop {
        let serialized = serde_json::to_string(&context)?;
        if serialized.len() <= MAX_CONTEXT_BYTES {
            return Ok(serialized);
        }

        if let Some(rounds) = context
            .pointer_mut("/semantic_replay/relevant_rounds")
            .and_then(Value::as_array_mut)
            && rounds.len() > 1
        {
            rounds.pop();
            continue;
        }
        if let Some(exchanges) = context
            .get_mut("recent_coaching_exchanges")
            .and_then(Value::as_array_mut)
            && exchanges.len() > 1
        {
            exchanges.pop();
            continue;
        }
        if let Some(metrics) = context
            .get_mut("deterministic_metrics")
            .and_then(Value::as_array_mut)
            && !metrics.is_empty()
        {
            metrics.pop();
            continue;
        }
        if let Some(semantic) = context.get_mut("semantic_replay")
            && !semantic.is_null()
        {
            let summary = json!({
                "question_scope": semantic.get("question_scope"),
                "player": semantic.get("player"),
                "players": semantic.get("players"),
                "all_rounds": semantic.get("all_rounds"),
                "context_truncated": true,
            });
            *semantic = summary;
            continue;
        }

        return serde_json::to_string(&json!({
            "retrieval_mode": context.get("retrieval_mode"),
            "personal_issues": context.get("personal_issues"),
            "player_profile": context.get("player_profile"),
            "context_truncated": true,
        }));
    }
}

#[derive(Debug, Default)]
struct QuestionScope {
    round_no: Option<u32>,
    area: Option<String>,
    side: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CoachingIssue {
    issue_key: String,
    category: String,
    title: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    #[serde(alias = "map_name")]
    map: Option<String>,
    #[serde(default)]
    side: Option<String>,
    #[serde(default)]
    area: Option<String>,
    #[serde(default = "default_issue_score")]
    severity: f64,
    #[serde(default = "default_issue_score")]
    confidence: f64,
}

fn extract_coaching_issues(text: &str) -> (String, Vec<CoachingIssue>) {
    let mut issues = Vec::new();
    let mut clean = text.to_owned();
    extract_issue_tag(&mut clean, "coaching_issues", &mut issues);
    extract_issue_tag(&mut clean, "coaching_issue", &mut issues);
    issues = issues
        .into_iter()
        .filter_map(normalize_coaching_issue)
        .collect();
    (clean.trim().to_owned(), issues)
}

fn extract_issue_tag(clean: &mut String, tag: &str, issues: &mut Vec<CoachingIssue>) {
    let opening = format!("<{tag}>");
    let closing = format!("</{tag}>");
    loop {
        let lower = clean.to_ascii_lowercase();
        let Some(start) = lower.find(&opening) else {
            break;
        };
        let content_start = start + opening.len();
        let (end, block_end) = match lower[content_start..].find(&closing) {
            Some(relative_end) => {
                let end = content_start + relative_end;
                (end, end + closing.len())
            }
            None => (clean.len(), clean.len()),
        };
        let block = clean[content_start..end].to_owned();
        issues.extend(parse_coaching_issue_block(&block));
        clean.replace_range(start..block_end, "");
    }
}

fn parse_coaching_issue_block(block: &str) -> Vec<CoachingIssue> {
    let mut json_text = block.trim();
    if let Some(fenced) = json_text.strip_prefix("```json") {
        json_text = fenced.strip_suffix("```").unwrap_or(fenced).trim();
    } else if let Some(fenced) = json_text.strip_prefix("```") {
        json_text = fenced.strip_suffix("```").unwrap_or(fenced).trim();
    }
    serde_json::from_str::<Vec<CoachingIssue>>(json_text).unwrap_or_else(|_| {
        serde_json::from_str::<CoachingIssue>(json_text)
            .map(|issue| vec![issue])
            .unwrap_or_default()
    })
}

fn normalize_coaching_issue(mut issue: CoachingIssue) -> Option<CoachingIssue> {
    issue.issue_key = issue.issue_key.trim().to_owned();
    issue.category = issue.category.trim().to_owned();
    issue.title = issue.title.trim().to_owned();
    if issue.issue_key.is_empty() || issue.category.is_empty() || issue.title.is_empty() {
        return None;
    }
    issue.description = issue.description.map(|value| value.trim().to_owned());
    issue.map = issue.map.map(|value| value.trim().to_owned());
    issue.side = issue.side.map(|value| value.trim().to_owned());
    issue.area = issue.area.map(|value| value.trim().to_owned());
    issue.severity = issue.severity.clamp(0.0, 1.0);
    issue.confidence = issue.confidence.clamp(0.0, 1.0);
    Some(issue)
}

fn default_issue_score() -> f64 {
    0.5
}

fn question_scope(question: &str) -> QuestionScope {
    let lower = question.to_ascii_lowercase();
    let area = if question.contains("A点") || question.contains("A 区") || lower.contains("a site")
    {
        Some("A".to_owned())
    } else if question.contains("B点") || question.contains("B 区") || lower.contains("b site") {
        Some("B".to_owned())
    } else if question.contains("中路") || lower.contains("mid") {
        Some("Mid".to_owned())
    } else {
        None
    };
    let side =
        if question.contains("防守") || lower.contains("defense") || lower.contains("defence") {
            Some("defense".to_owned())
        } else if question.contains("进攻") || lower.contains("attack") {
            Some("attack".to_owned())
        } else {
            None
        };
    let round_no = parse_round_number(question, &lower);
    QuestionScope {
        round_no,
        area,
        side,
    }
}

fn parse_round_number(question: &str, lower: &str) -> Option<u32> {
    if let Some(start) = question.find('第') {
        let tail = &question[start + '第'.len_utf8()..];
        let digits = tail
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>();
        if tail[digits.len()..].starts_with("回合") {
            return digits.parse().ok();
        }
    }
    if let Some(start) = lower.find("round") {
        let digits = lower[start + 5..]
            .trim_start()
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>();
        return digits.parse().ok();
    }
    None
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
    fn from_settings(settings: AgentSettingsRequest) -> Result<Self, AgentError> {
        let kind = ProviderKind::parse(&settings.provider)?;
        let model = normalize_model(kind, &settings.model);
        let api_key = settings.api_key.trim().to_owned();
        if model.is_empty() || model.len() > 200 {
            return Err(AgentError::Configuration(
                "model must contain 1-200 characters".to_owned(),
            ));
        }
        if api_key.is_empty() || api_key.len() > 8_192 {
            return Err(AgentError::Configuration(
                "API key must contain 1-8192 characters".to_owned(),
            ));
        }
        let base_url = settings
            .base_url
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| kind.default_base_url().to_owned());
        let max_output_tokens = settings
            .max_output_tokens
            .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS);
        if max_output_tokens == 0 || max_output_tokens > MAX_MAX_OUTPUT_TOKENS {
            return Err(AgentError::Configuration(format!(
                "max output tokens must be between 1 and {MAX_MAX_OUTPUT_TOKENS}"
            )));
        }
        validate_base_url(&base_url)?;
        let base_url = normalize_base_url(kind, &base_url);
        validate_optional_price("input_usd_per_million", settings.input_usd_per_million)?;
        validate_optional_price("output_usd_per_million", settings.output_usd_per_million)?;
        if settings.input_usd_per_million.is_some() != settings.output_usd_per_million.is_some() {
            return Err(AgentError::Configuration(
                "input and output prices must be provided together".to_owned(),
            ));
        }
        Ok(Self {
            client: Client::builder()
                .timeout(Duration::from_secs(300))
                .connect_timeout(Duration::from_secs(30))
                .pool_idle_timeout(Duration::from_secs(90))
                .tcp_keepalive(Duration::from_secs(60))
                .build()?,
            kind,
            model,
            base_url,
            api_key,
            max_output_tokens,
            input_price: settings.input_usd_per_million,
            output_price: settings.output_usd_per_million,
        })
    }

    async fn complete(&self, instructions: &str, input: &str) -> Result<ProviderReply, AgentError> {
        let mut reply = match self.kind {
            ProviderKind::OpenAi => self.complete_openai(instructions, input).await?,
            ProviderKind::Anthropic => self.complete_anthropic(instructions, input).await?,
            ProviderKind::DeepSeek
            | ProviderKind::Gemini
            | ProviderKind::Xai
            | ProviderKind::Zhipu
            | ProviderKind::Moonshot
            | ProviderKind::Qwen
            | ProviderKind::OpenAiCompatible => {
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
                        "store": false,
                        "truncation": "auto"
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
        let mut last_error: Option<AgentError> = None;
        for attempt in 0..3u32 {
            let attempt_request = match request.try_clone() {
                Some(req) => req,
                None => {
                    let response = request.send().await.map_err(AgentError::Http)?;
                    return self.process_response(response).await;
                }
            };
            let response = match attempt_request.send().await {
                Ok(resp) => resp,
                Err(error) => {
                    if attempt < 2 && error.is_connect() {
                        tracing::warn!(attempt, error = %error, "retrying LLM request (connect error)");
                        tokio::time::sleep(std::time::Duration::from_millis(
                            500 * (1 + attempt as u64),
                        ))
                        .await;
                        last_error = Some(AgentError::Http(error));
                        continue;
                    }
                    if attempt < 2 && error.is_timeout() {
                        tracing::warn!(attempt, error = %error, "LLM request timed out (likely still generating); not retrying to avoid duplicate charges");
                        return Err(AgentError::Http(error));
                    }
                    return Err(AgentError::Http(error));
                }
            };
            let status = response.status();
            let request_id = response
                .headers()
                .get("x-request-id")
                .or_else(|| response.headers().get("request-id"))
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            let bytes = response.bytes().await?;
            if !status.is_success() {
                let detail = provider_error_detail(&bytes).replace(&self.api_key, "[redacted]");
                if status.is_server_error() && attempt < 2 {
                    tracing::warn!(
                        attempt,
                        status = status.as_u16(),
                        "retrying LLM request after server error"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(
                        500 * (1 + attempt as u64),
                    ))
                    .await;
                    last_error = Some(AgentError::Provider {
                        status: status.as_u16(),
                        detail,
                        request_id,
                    });
                    continue;
                }
                return Err(AgentError::Provider {
                    status: status.as_u16(),
                    detail,
                    request_id,
                });
            }
            return serde_json::from_slice(&bytes).map_err(|error| {
                AgentError::InvalidResponse(format!("response was not valid JSON: {error}"))
            });
        }
        Err(last_error
            .unwrap_or_else(|| AgentError::InvalidResponse("all retries exhausted".to_owned())))
    }

    async fn process_response(&self, response: reqwest::Response) -> Result<Value, AgentError> {
        let status = response.status();
        let request_id = response
            .headers()
            .get("x-request-id")
            .or_else(|| response.headers().get("request-id"))
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let bytes = response.bytes().await?;
        if !status.is_success() {
            let detail = provider_error_detail(&bytes).replace(&self.api_key, "[redacted]");
            return Err(AgentError::Provider {
                status: status.as_u16(),
                detail,
                request_id,
            });
        }
        serde_json::from_slice(&bytes).map_err(|error| {
            AgentError::InvalidResponse(format!("response was not valid JSON: {error}"))
        })
    }
}

fn provider_from_env() -> Result<Option<LlmProvider>, AgentError> {
    let Some(provider_name) = optional_env("VALCOACH_LLM_PROVIDER") else {
        return Ok(None);
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
    let base_url =
        optional_env("VALCOACH_LLM_BASE_URL").unwrap_or_else(|| kind.default_base_url().to_owned());
    let max_output_tokens = optional_env("VALCOACH_LLM_MAX_OUTPUT_TOKENS")
        .map(|value| parse_positive_u32("VALCOACH_LLM_MAX_OUTPUT_TOKENS", &value))
        .transpose()?
        .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS);
    let input_price = optional_env("VALCOACH_LLM_INPUT_USD_PER_MILLION")
        .map(|value| parse_non_negative_f64("VALCOACH_LLM_INPUT_USD_PER_MILLION", &value))
        .transpose()?;
    let output_price = optional_env("VALCOACH_LLM_OUTPUT_USD_PER_MILLION")
        .map(|value| parse_non_negative_f64("VALCOACH_LLM_OUTPUT_USD_PER_MILLION", &value))
        .transpose()?;
    LlmProvider::from_settings(AgentSettingsRequest {
        provider: kind.name().to_owned(),
        model,
        api_key,
        base_url: Some(base_url),
        max_output_tokens: Some(max_output_tokens),
        input_usd_per_million: input_price,
        output_usd_per_million: output_price,
    })
    .map(Some)
}

fn validate_base_url(base_url: &str) -> Result<(), AgentError> {
    if base_url.is_empty() {
        return Err(AgentError::Configuration(
            "base URL is required for an OpenAI-compatible provider".to_owned(),
        ));
    }
    if base_url.len() > 2_048 {
        return Err(AgentError::Configuration("base URL is too long".to_owned()));
    }
    if !base_url.starts_with("https://") && !base_url.starts_with("http://127.0.0.1") {
        return Err(AgentError::Configuration(
            "base URL must use HTTPS (or loopback HTTP for local testing)".to_owned(),
        ));
    }
    Ok(())
}

fn normalize_base_url(kind: ProviderKind, base_url: &str) -> String {
    let base_url = base_url.trim().trim_end_matches('/');
    let endpoint = match kind {
        ProviderKind::OpenAi => "/responses",
        ProviderKind::Anthropic => "/messages",
        ProviderKind::DeepSeek
        | ProviderKind::Gemini
        | ProviderKind::Xai
        | ProviderKind::Zhipu
        | ProviderKind::Moonshot
        | ProviderKind::Qwen
        | ProviderKind::OpenAiCompatible => "/chat/completions",
    };
    base_url
        .strip_suffix(endpoint)
        .unwrap_or(base_url)
        .to_owned()
}

fn normalize_model(kind: ProviderKind, model: &str) -> String {
    let model = model.trim();
    if kind == ProviderKind::DeepSeek
        && (model.eq_ignore_ascii_case("deepseek") || model.eq_ignore_ascii_case("deepseek-chat"))
    {
        "deepseek-v4-flash".to_owned()
    } else {
        model.to_owned()
    }
}

fn provider_error_detail(bytes: &[u8]) -> String {
    let parsed = serde_json::from_slice::<Value>(bytes).ok();
    let detail = parsed
        .as_ref()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .or_else(|| value.get("message"))
                .and_then(Value::as_str)
        })
        .unwrap_or_else(|| std::str::from_utf8(bytes).unwrap_or("unreadable response body"));
    detail.chars().take(500).collect()
}

fn validate_optional_price(name: &str, price: Option<f64>) -> Result<(), AgentError> {
    if price.is_some_and(|value| !value.is_finite() || value < 0.0) {
        return Err(AgentError::Configuration(format!(
            "{name} must be a non-negative number"
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderKind {
    OpenAi,
    Anthropic,
    DeepSeek,
    Gemini,
    Xai,
    Zhipu,
    Moonshot,
    Qwen,
    OpenAiCompatible,
}

impl ProviderKind {
    fn parse(value: &str) -> Result<Self, AgentError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "openai" => Ok(Self::OpenAi),
            "anthropic" | "claude" => Ok(Self::Anthropic),
            "deepseek" => Ok(Self::DeepSeek),
            "gemini" | "google" => Ok(Self::Gemini),
            "xai" | "grok" => Ok(Self::Xai),
            "zhipu" | "glm" => Ok(Self::Zhipu),
            "moonshot" | "kimi" => Ok(Self::Moonshot),
            "qwen" | "dashscope" => Ok(Self::Qwen),
            "openai-compatible" | "openai_compatible" => Ok(Self::OpenAiCompatible),
            _ => Err(AgentError::Configuration(
                "provider must be openai, anthropic, deepseek, gemini, xai, zhipu, moonshot, qwen, or openai-compatible"
                    .to_owned(),
            )),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::DeepSeek => "deepseek",
            Self::Gemini => "gemini",
            Self::Xai => "xai",
            Self::Zhipu => "zhipu",
            Self::Moonshot => "moonshot",
            Self::Qwen => "qwen",
            Self::OpenAiCompatible => "openai-compatible",
        }
    }

    fn default_base_url(self) -> &'static str {
        match self {
            Self::OpenAi => "https://api.openai.com/v1",
            Self::Anthropic => "https://api.anthropic.com/v1",
            Self::DeepSeek => "https://api.deepseek.com",
            Self::Gemini => "https://generativelanguage.googleapis.com/v1beta/openai",
            Self::Xai => "https://api.x.ai/v1",
            Self::Zhipu => "https://open.bigmodel.cn/api/paas/v4",
            Self::Moonshot => "https://api.moonshot.cn/v1",
            Self::Qwen => "https://dashscope.aliyuncs.com/compatible-mode/v1",
            Self::OpenAiCompatible => "",
        }
    }

    fn default_key_variable(self) -> &'static str {
        match self {
            Self::OpenAi => "OPENAI_API_KEY",
            Self::Anthropic => "ANTHROPIC_API_KEY",
            Self::DeepSeek => "DEEPSEEK_API_KEY",
            Self::Gemini => "GEMINI_API_KEY",
            Self::Xai => "XAI_API_KEY",
            Self::Zhipu => "ZHIPU_API_KEY",
            Self::Moonshot => "MOONSHOT_API_KEY",
            Self::Qwen => "DASHSCOPE_API_KEY",
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
    let nested_text = value
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
    let text = value
        .get("output_text")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .unwrap_or(&nested_text)
        .to_owned();
    if text.trim().is_empty() && value.get("status").and_then(Value::as_str) == Some("incomplete") {
        let reason = value
            .pointer("/incomplete_details/reason")
            .and_then(Value::as_str)
            .unwrap_or("unknown reason");
        return Err(AgentError::Incomplete(reason.to_owned()));
    }
    if text.trim().is_empty()
        && value
            .pointer("/error/message")
            .and_then(Value::as_str)
            .is_some()
    {
        return Err(AgentError::InvalidResponse(
            "provider reported a failed response".to_owned(),
        ));
    }
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
    if text.trim().is_empty()
        && value.get("stop_reason").and_then(Value::as_str) == Some("max_tokens")
    {
        return Err(AgentError::Incomplete("max_tokens".to_owned()));
    }
    let usage = value.get("usage").unwrap_or(&Value::Null);
    let input = token(usage, "input_tokens")
        .saturating_add(token(usage, "cache_creation_input_tokens"))
        .saturating_add(token(usage, "cache_read_input_tokens"));
    let output = token(usage, "output_tokens");
    provider_reply(value, text, input, output, input.saturating_add(output))
}

fn parse_chat_completion(value: &Value) -> Result<ProviderReply, AgentError> {
    let content = value
        .pointer("/choices/0/message/content")
        .unwrap_or(&Value::Null);
    let text = if let Some(text) = content.as_str() {
        text.to_owned()
    } else {
        content
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n")
    };
    if text.trim().is_empty()
        && value
            .pointer("/choices/0/finish_reason")
            .and_then(Value::as_str)
            == Some("length")
    {
        return Err(AgentError::Incomplete("max_tokens".to_owned()));
    }
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
        return Err(AgentError::InvalidResponse(
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
    #[error("LLM provider returned HTTP {status}: {detail} (request_id: {request_id:?})")]
    Provider {
        status: u16,
        detail: String,
        request_id: Option<String>,
    },
    #[error("LLM provider returned an incomplete response: {0}")]
    Incomplete(String),
    #[error("LLM provider returned an invalid response: {0}")]
    InvalidResponse(String),
    #[error("coaching request was cancelled")]
    Cancelled,
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error("failed to serialize Agent context: {0}")]
    Serialization(#[from] serde_json::Error),
}

pub async fn list_issues(
    State(state): State<AppState>,
    session: tower_sessions::Session,
) -> Result<Json<Vec<Value>>, AuthApiError> {
    let user_id = require_user_id(&state.auth, &session).await?;
    state
        .auth
        .database
        .list_player_issues(&user_id)
        .await
        .map(Json)
        .map_err(|error| AuthApiError::internal(error.to_string()))
}

pub async fn status(
    State(state): State<AppState>,
    session: tower_sessions::Session,
) -> Result<Json<AgentStatus>, AuthApiError> {
    let user_id = require_user_id(&state.auth, &session).await?;
    Ok(Json(state.agent.status_for(&user_id).await))
}

pub async fn configure_settings(
    State(state): State<AppState>,
    session: tower_sessions::Session,
    Json(settings): Json<AgentSettingsRequest>,
) -> Result<Json<AgentStatus>, AuthApiError> {
    let user_id = require_user_id(&state.auth, &session).await?;
    state
        .agent
        .configure_for(&user_id, settings)
        .await
        .map(Json)
        .map_err(agent_api_error)
}

pub async fn clear_settings(
    State(state): State<AppState>,
    session: tower_sessions::Session,
) -> Result<Json<AgentStatus>, AuthApiError> {
    let user_id = require_user_id(&state.auth, &session).await?;
    Ok(Json(state.agent.clear_for(&user_id).await))
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

pub async fn cancel_coach(
    State(state): State<AppState>,
    session: tower_sessions::Session,
    Path(match_id): Path<String>,
) -> Result<StatusCode, AuthApiError> {
    let user_id = require_user_id(&state.auth, &session).await?;
    state.agent.cancel(&user_id, &match_id).await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn history(
    State(state): State<AppState>,
    session: tower_sessions::Session,
    Path(match_id): Path<String>,
) -> Result<Json<Vec<AgentMessageRecord>>, AuthApiError> {
    let user_id = require_user_id(&state.auth, &session).await?;
    let mut messages = state
        .auth
        .database
        .list_agent_messages_for_match(&user_id, &match_id)
        .await
        .map_err(|error| AuthApiError::internal(error.to_string()))?;
    for message in &mut messages {
        if message.role == "assistant" {
            message.content = extract_coaching_issues(&message.content).0;
        }
    }
    Ok(Json(messages))
}

pub async fn clear_history(
    State(state): State<AppState>,
    session: tower_sessions::Session,
    Path(match_id): Path<String>,
) -> Result<StatusCode, AuthApiError> {
    let user_id = require_user_id(&state.auth, &session).await?;
    state
        .auth
        .database
        .delete_agent_history_for_match(&user_id, &match_id)
        .await
        .map_err(|error| AuthApiError::internal(error.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
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
        AgentError::Cancelled => AuthApiError::bad_request("复盘请求已中止。"),
        AgentError::Disabled | AgentError::InvalidQuestion | AgentError::Configuration(_) => {
            AuthApiError::bad_request(error.to_string())
        }
        AgentError::MatchNotFound | AgentError::Database(DatabaseError::MatchNotFound) => {
            AuthApiError::unauthorized()
        }
        AgentError::Http(error) => {
            tracing::error!(error = %error, "replay coaching HTTP request failed");
            if error.is_timeout() {
                AuthApiError::gateway_timeout(
                    "连接模型服务超时，请稍后重试，或检查 Base URL 和网络代理。",
                )
            } else if error.is_connect() {
                AuthApiError::bad_gateway("无法连接模型服务，请检查 Base URL、网络连接和代理设置。")
            } else {
                AuthApiError::bad_gateway("模型请求发送失败，请检查模型设置后重试。")
            }
        }
        AgentError::Provider {
            status,
            detail,
            request_id,
        } => {
            tracing::error!(status, %detail, ?request_id, "replay coaching provider rejected request");
            provider_status_error(status)
        }
        AgentError::Incomplete(reason) => {
            tracing::error!(%reason, "replay coaching provider response was incomplete");
            AuthApiError::bad_gateway(if reason == "max_output_tokens" || reason == "max_tokens" {
                "模型在生成答案前用完了输出 Token。请检查模型设置；最大输出 Tokens 的默认值为 32768。"
            } else {
                "模型没有完成答案，请稍后重试。"
            })
        }
        AgentError::InvalidResponse(detail) => {
            tracing::error!(%detail, "replay coaching provider returned invalid response");
            AuthApiError::bad_gateway(
                "模型服务返回了无法识别的响应。请确认服务商类型与 Base URL 相匹配。",
            )
        }
        other => {
            tracing::error!(error = %other, "replay coaching request failed");
            AuthApiError::internal("复盘结果保存失败，请重试。")
        }
    }
}

fn provider_status_error(status: u16) -> AuthApiError {
    let message = match status {
        400 | 422 => "模型服务拒绝了请求。请检查模型 ID、Base URL 和最大输出 Tokens。",
        401 => "API Key 验证失败，请在模型设置中重新填写正确的 Key。",
        402 => "模型账户余额不足，请充值或更换可用的 API Key。",
        403 => "当前 API Key 没有访问该模型的权限，请更换模型或 Key。",
        404 => "没有找到模型接口。请检查模型 ID；Base URL 应填写到 /v1，不要填写完整请求地址。",
        408 => "模型服务响应超时，请稍后重试。",
        413 => "发送给模型的复盘上下文过长，请换用上下文窗口更大的模型。",
        429 => "模型服务已限流或账户额度不足，请稍后重试并检查 API 余额。",
        500..=599 => "模型服务暂时不可用，请稍后重试。",
        _ => "模型服务拒绝了请求，请检查模型设置后重试。",
    };
    if status == 408 {
        AuthApiError::gateway_timeout(message)
    } else {
        AuthApiError::bad_gateway(message)
    }
}

#[cfg(test)]
mod tests {
    use axum::{Json, Router, http::StatusCode, response::IntoResponse, routing::post};
    use http_body_util::BodyExt;
    use serde_json::json;
    use valcoach_db::{Database, UserProfileRecord, UserRecord};
    use valcoach_domain::{
        CapabilityLevel, ParsedBundle, ParsedReplay, ParsedReplaySummary, ReplayCapabilities,
        ReplayMetadata,
    };

    use super::{
        AgentError, AgentService, AgentSettingsRequest, AgentTokenUsage, LlmProvider, ProviderKind,
        RetrievalMode, agent_api_error, estimate_cost, extract_coaching_issues, normalize_base_url,
        normalize_model, parse_anthropic_response, parse_chat_completion, parse_openai_response,
        question_retrieval_mode, serialize_context_with_budget,
    };

    #[test]
    fn history_questions_skip_current_match_retrieval() {
        for question in [
            "不看这局游戏，只分析历史问题",
            "我以前有哪些长期问题？",
            "Show my coaching history only",
        ] {
            assert_eq!(
                question_retrieval_mode(question),
                RetrievalMode::PersonalHistoryOnly
            );
        }
        assert_eq!(
            question_retrieval_mode("结合这局和历史问题给我建议"),
            RetrievalMode::MatchAnalysis
        );
        assert_eq!(
            question_retrieval_mode("分析这一局的防守"),
            RetrievalMode::MatchAnalysis
        );
    }

    #[test]
    fn coaching_issue_blocks_are_hidden_and_parsed_robustly() {
        let answer = r#"先减少无信息前压。
<COACHING_ISSUES>
```json
[{"issue_key":"defense_overpush","category":"positioning","title":"防守无信息前压","description":null,"side":"defense","severity":0.8,"confidence":0.75}]
```
</COACHING_ISSUES>"#;
        let (clean, issues) = extract_coaching_issues(answer);
        assert_eq!(clean, "先减少无信息前压。");
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].issue_key, "defense_overpush");
        assert_eq!(issues[0].severity, 0.8);

        let unclosed = r#"先换位再接第二枪。
<coaching_issues> [{"issue_key":"post_kill_positioning","category":"positioning","title":"击杀后未换位","severity":0.68,"confidence":0.72}]"#;
        let (clean, issues) = extract_coaching_issues(unclosed);
        assert_eq!(clean, "先换位再接第二枪。");
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].issue_key, "post_kill_positioning");
    }

    #[test]
    fn serialized_context_has_a_hard_size_budget() {
        let oversized_rounds = (0..20)
            .map(|round| json!({"round":round,"combat":["x".repeat(20_000)]}))
            .collect::<Vec<_>>();
        let serialized = serialize_context_with_budget(json!({
            "retrieval_mode":"match_analysis",
            "personal_issues":[],
            "player_profile":{},
            "semantic_replay":{"relevant_rounds":oversized_rounds}
        }))
        .expect("context serialization");
        assert!(serialized.len() <= super::MAX_CONTEXT_BYTES);
        assert!(serde_json::from_str::<serde_json::Value>(&serialized).is_ok());
    }

    #[test]
    fn deepseek_legacy_names_are_normalized_to_v4_flash() {
        assert_eq!(
            normalize_model(ProviderKind::DeepSeek, "DeepSeek"),
            "deepseek-v4-flash"
        );
        assert_eq!(
            normalize_model(ProviderKind::DeepSeek, "deepseek-chat"),
            "deepseek-v4-flash"
        );
        assert_eq!(
            normalize_model(ProviderKind::DeepSeek, "deepseek-reasoner"),
            "deepseek-reasoner"
        );
    }

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

        let openai_top_level = parse_openai_response(&json!({
            "id":"resp-2",
            "status":"completed",
            "output_text":"Top-level OpenAI text",
            "usage":{"input_tokens":2,"output_tokens":3,"total_tokens":5}
        }))
        .expect("OpenAI top-level response");
        assert_eq!(openai_top_level.text, "Top-level OpenAI text");

        assert!(
            parse_openai_response(&json!({
                "id":"resp-3",
                "status":"incomplete",
                "incomplete_details":{"reason":"max_output_tokens"},
                "output":[],
                "usage":{"input_tokens":10,"output_tokens":800,"total_tokens":810}
            }))
            .is_err()
        );

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
        let provider = LlmProvider::from_settings(AgentSettingsRequest {
            provider: "openai".to_owned(),
            model: "test-model".to_owned(),
            api_key: "test-key".to_owned(),
            base_url: None,
            max_output_tokens: None,
            input_usd_per_million: None,
            output_usd_per_million: None,
        })
        .expect("provider defaults");
        assert_eq!(provider.max_output_tokens, 32_768);

        assert_eq!(
            ProviderKind::parse("claude").expect("alias"),
            ProviderKind::Anthropic
        );
        for (name, expected) in [
            ("gemini", ProviderKind::Gemini),
            ("grok", ProviderKind::Xai),
            ("glm", ProviderKind::Zhipu),
            ("kimi", ProviderKind::Moonshot),
            ("qwen", ProviderKind::Qwen),
        ] {
            assert_eq!(ProviderKind::parse(name).expect("provider alias"), expected);
            assert!(!expected.default_base_url().is_empty());
        }
        assert_eq!(
            normalize_base_url(ProviderKind::OpenAi, "https://api.openai.com/v1/responses/"),
            "https://api.openai.com/v1"
        );
        assert_eq!(
            normalize_base_url(
                ProviderKind::OpenAiCompatible,
                "https://example.com/v1/chat/completions"
            ),
            "https://example.com/v1"
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
                assert_eq!(body["truncation"], "auto");
                let input = body["input"].as_str().expect("request input");
                assert!(input.contains("match-1"));
                assert!(input.contains("player_profile") && input.contains("GOLD 2"));
                let history_only = input.contains("只分析历史问题");
                if history_only {
                    assert!(input.contains(r#""retrieval_mode":"personal_history_only""#));
                    assert!(input.contains(r#""current_match_loaded":false"#));
                    assert!(!input.contains("semantic_replay"));
                    assert!(input.contains("defense_overpush"));
                } else {
                    assert!(input.contains(r#""retrieval_mode":"match_analysis""#));
                }
                Json(json!({
                    "id":"resp-mock",
                    "output":[{"content":[{"type":"output_text","text": if history_only {
                        "Your saved issue is defensive over-pushing."
                    } else {
                        "Only observed evidence is used.\n<coaching_issues>[{\"issue_key\":\"defense_overpush\",\"category\":\"positioning\",\"title\":\"Defensive over-push\",\"description\":null,\"side\":\"defense\",\"severity\":0.7,\"confidence\":0.8}]</coaching_issues>"
                    }}]}],
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
                        server_events_path: None,
                    },
                    source_name: "test".to_owned(),
                    capabilities: ReplayCapabilities::global_fixture(CapabilityLevel::Partial),
                    summary: ParsedReplaySummary::default(),
                },
            )
            .await
            .expect("match");
        database
            .upsert_user_profile(
                "user-1",
                &UserProfileRecord {
                    rank_name: Some("GOLD 2".to_owned()),
                    main_role: Some("Controller".to_owned()),
                    main_agents: vec!["Viper".to_owned()],
                    training_goals: vec!["地图控制".to_owned()],
                    goal_notes: String::new(),
                    updated_at: None,
                },
            )
            .await
            .expect("profile");
        let service = AgentService::disabled(database.clone());
        let status = service
            .configure_for(
                "user-1",
                AgentSettingsRequest {
                    provider: "openai".to_owned(),
                    model: "mock-model".to_owned(),
                    api_key: "test-key".to_owned(),
                    base_url: Some(format!("http://{address}")),
                    max_output_tokens: Some(100),
                    input_usd_per_million: Some(1.0),
                    output_usd_per_million: Some(2.0),
                },
            )
            .await
            .expect("web settings");
        assert!(status.configured);
        assert_eq!(status.source.as_deref(), Some("web"));
        let answer = service
            .coach("user-1", "match-1", "How should I improve?")
            .await
            .expect("coach answer");
        assert_eq!(answer.usage.total_tokens, 27);
        assert_eq!(answer.usage.cost_microusd, Some(33));
        assert_eq!(answer.limitations.len(), 1);
        assert_eq!(answer.answer, "Only observed evidence is used.");
        let issues = database
            .list_player_issues("user-1")
            .await
            .expect("persisted coaching issue");
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0]["occurrences"], 1);
        let history_answer = service
            .coach("user-1", "match-1", "不看这局游戏，只分析历史问题")
            .await
            .expect("history-only coach answer");
        assert_eq!(
            history_answer.answer,
            "Your saved issue is defensive over-pushing."
        );
        let history = database
            .list_agent_messages_for_match("user-1", "match-1")
            .await
            .expect("history");
        assert_eq!(history.len(), 4);
        assert_eq!(history[1].usage.total_tokens, 27);
    }

    #[tokio::test]
    async fn provider_http_error_is_classified_and_redacts_the_api_key() {
        let mock = Router::new().route(
            "/responses",
            post(|| async {
                (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"error":{"message":"invalid credential test-key"}})),
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock listener");
        let address = listener.local_addr().expect("mock address");
        tokio::spawn(async move { axum::serve(listener, mock).await.expect("mock server") });

        let provider = LlmProvider::from_settings(AgentSettingsRequest {
            provider: "openai".to_owned(),
            model: "mock-model".to_owned(),
            api_key: "test-key".to_owned(),
            base_url: Some(format!("http://{address}/responses")),
            max_output_tokens: None,
            input_usd_per_million: None,
            output_usd_per_million: None,
        })
        .expect("provider settings");
        let error = provider
            .complete("system", "input")
            .await
            .expect_err("provider must reject request");
        match error {
            AgentError::Provider { status, detail, .. } => {
                assert_eq!(status, 401);
                assert_eq!(detail, "invalid credential [redacted]");
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[tokio::test]
    async fn provider_rejection_returns_an_actionable_safe_api_error() {
        let response = agent_api_error(AgentError::Provider {
            status: 401,
            detail: "private upstream detail".to_owned(),
            request_id: Some("request-1".to_owned()),
        })
        .into_response();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("error response body")
            .to_bytes();
        let body = std::str::from_utf8(&body).expect("UTF-8 body");
        assert!(body.contains("API Key"));
        assert!(!body.contains("private upstream detail"));
        assert!(!body.contains("request-1"));
    }
}
