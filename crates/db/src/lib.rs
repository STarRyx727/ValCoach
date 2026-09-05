//! SQLite persistence for ValCoach-owned stable domain data.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    str::FromStr,
};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};
use sqlx::{QueryBuilder, Sqlite, Transaction};
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use valcoach_domain::{
    MovementSample, ParsedReplay, ParsedReplaySummary, ReplayCapabilities, ReplayMetadata, Vector3,
};
use valcoach_replay_adapter::{NormalizedRecord, ParsedBundleSource, ReplaySourceError};

const INSERT_BATCH_SIZE: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserRecord {
    pub id: String,
    pub username: String,
    pub password_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ParseJobRecord {
    pub id: String,
    pub user_id: String,
    pub status: String,
    pub source_name: String,
    pub error_message: Option<String>,
    pub match_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlayerRecord {
    pub id: String,
    pub match_id: String,
    pub stable_player_id: Option<String>,
    pub display_name: Option<String>,
    pub team: Option<String>,
    pub agent_name: Option<String>,
    pub player_slot: Option<i64>,
    pub is_bound: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchRecord {
    pub id: String,
    pub parser_source: String,
    pub metadata: ReplayMetadata,
    pub capabilities: ReplayCapabilities,
    pub summary: ParsedReplaySummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MatchMetricRecord {
    pub id: String,
    pub metric_name: String,
    pub value_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ValorantAccountRecord {
    pub id: String,
    pub user_id: String,
    pub region: String,
    pub subject_id: Option<String>,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub cost_microusd: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AgentMessageRecord {
    pub id: String,
    pub session_id: String,
    pub provider: String,
    pub model: String,
    pub role: String,
    pub content: String,
    pub evidence: serde_json::Value,
    pub limitations: Vec<String>,
    pub usage: AgentTokenUsage,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AgentUsageSummary {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub cost_microusd: u64,
    pub priced_requests: u64,
}

#[derive(Clone, Debug)]
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    pub async fn connect(database_url: &str) -> Result<Self, DatabaseError> {
        let in_memory = database_url.contains(":memory:");
        let mut options = SqliteConnectOptions::from_str(database_url)
            .map_err(DatabaseError::InvalidUrl)?
            .foreign_keys(true)
            .create_if_missing(true)
            .busy_timeout(std::time::Duration::from_secs(10));
        if !in_memory {
            options = options.journal_mode(SqliteJournalMode::Wal);
        }
        let pool = SqlitePoolOptions::new()
            .max_connections(if in_memory { 1 } else { 5 })
            .connect_with(options)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn create_user(&self, user: &UserRecord) -> Result<(), DatabaseError> {
        let result =
            sqlx::query("INSERT INTO users (id, username, password_hash) VALUES (?, ?, ?)")
                .bind(&user.id)
                .bind(&user.username)
                .bind(&user.password_hash)
                .execute(&self.pool)
                .await;

        match result {
            Ok(_) => Ok(()),
            Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
                Err(DatabaseError::UsernameAlreadyExists)
            }
            Err(error) => Err(DatabaseError::Sqlx(error)),
        }
    }

    pub async fn find_user_by_username(
        &self,
        username: &str,
    ) -> Result<Option<UserRecord>, DatabaseError> {
        let row = sqlx::query_as::<_, (String, String, String)>(
            "SELECT id, username, password_hash FROM users WHERE username = ?",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|(id, username, password_hash)| UserRecord {
            id,
            username,
            password_hash,
        }))
    }

    pub async fn find_user_by_id(&self, id: &str) -> Result<Option<UserRecord>, DatabaseError> {
        let row = sqlx::query_as::<_, (String, String, String)>(
            "SELECT id, username, password_hash FROM users WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|(id, username, password_hash)| UserRecord {
            id,
            username,
            password_hash,
        }))
    }

    pub async fn create_parse_job(
        &self,
        job_id: &str,
        user_id: &str,
        source_name: &str,
    ) -> Result<(), DatabaseError> {
        sqlx::query(
            "INSERT INTO parse_jobs (id, user_id, status, source_name) VALUES (?, ?, 'queued', ?)",
        )
        .bind(job_id)
        .bind(user_id)
        .bind(source_name)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn find_parse_job_for_user(
        &self,
        job_id: &str,
        user_id: &str,
    ) -> Result<Option<ParseJobRecord>, DatabaseError> {
        let row = sqlx::query_as::<_, (String, String, String, String, Option<String>, Option<String>)>(
            "SELECT id, user_id, status, source_name, error_message, match_id FROM parse_jobs WHERE id = ? AND user_id = ?",
        )
        .bind(job_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(
            |(id, user_id, status, source_name, error_message, match_id)| ParseJobRecord {
                id,
                user_id,
                status,
                source_name,
                error_message,
                match_id,
            },
        ))
    }

    pub async fn update_parse_job(
        &self,
        job_id: &str,
        status: &str,
        error_message: Option<&str>,
        match_id: Option<&str>,
    ) -> Result<(), DatabaseError> {
        sqlx::query(
            r#"
            UPDATE parse_jobs
            SET status = ?, error_message = ?, match_id = COALESCE(?, match_id), updated_at = CURRENT_TIMESTAMP
            WHERE id = ?
            "#,
        )
        .bind(status)
        .bind(error_message)
        .bind(match_id)
        .bind(job_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Persists the parser-independent metadata and summary immediately after adapter validation.
    /// Event and movement rows are intentionally inserted later through a batched streaming sink.
    pub async fn insert_match_summary(
        &self,
        user_id: &str,
        match_id: &str,
        replay: &ParsedReplay,
    ) -> Result<(), DatabaseError> {
        let metadata_json = serde_json::to_string(&replay.metadata)?;
        let capabilities_json = serde_json::to_string(&replay.capabilities)?;
        let summary_json = serde_json::to_string(&replay.summary)?;

        sqlx::query(
            r#"
            INSERT INTO matches (id, user_id, parser_source, metadata_json, capabilities_json, summary_json)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(match_id)
        .bind(user_id)
        .bind(&replay.source_name)
        .bind(metadata_json)
        .bind(capabilities_json)
        .bind(summary_json)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Persists a complete normalized replay atomically. NDJSON is reread as a stream and
    /// retained only in bounded insert batches, so replay size does not dictate process memory.
    pub async fn insert_parsed_replay_with_records(
        &self,
        user_id: &str,
        match_id: &str,
        replay: &ParsedReplay,
        cancel: CancellationToken,
    ) -> Result<PersistedRecordCounts, DatabaseError> {
        let metadata_json = serde_json::to_string(&replay.metadata)?;
        let capabilities_json = serde_json::to_string(&replay.capabilities)?;
        let summary_json = serde_json::to_string(&replay.summary)?;
        let mut transaction = self.pool.begin().await?;

        sqlx::query(
            r#"
            INSERT INTO matches (id, user_id, parser_source, metadata_json, capabilities_json, summary_json)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(match_id)
        .bind(user_id)
        .bind(&replay.source_name)
        .bind(metadata_json)
        .bind(capabilities_json)
        .bind(summary_json)
        .execute(&mut *transaction)
        .await?;

        let mut events = Vec::with_capacity(INSERT_BATCH_SIZE);
        let mut movement = Vec::with_capacity(INSERT_BATCH_SIZE);
        let mut roster = ReplayRoster::default();
        let mut finalized_roster = None;
        let mut counts = PersistedRecordCounts::default();
        let records = ParsedBundleSource.records(replay.bundle.clone(), cancel);
        futures_util::pin_mut!(records);

        while let Some(record) = records.next().await {
            match record? {
                NormalizedRecord::Event(event) => {
                    roster.observe(&event);
                    events.push(PersistedEvent {
                        timestamp_ms: event.timestamp_ms,
                        event_type: event.event_type,
                        actor_net_guid: event.actor_net_guid.map(|guid| guid.to_string()),
                        payload_json: serde_json::to_string(&event.raw)?,
                    });
                    counts.event_count += 1;
                    if events.len() >= INSERT_BATCH_SIZE {
                        flush_events(&mut transaction, match_id, &mut events).await?;
                    }
                }
                NormalizedRecord::Movement(sample) => {
                    let finalized =
                        finalized_roster.get_or_insert_with(|| roster.finalize(match_id));
                    let player_id = sample
                        .character_net_guid
                        .and_then(|guid| finalized.pawn_to_player.get(&guid).cloned());
                    movement.push(PersistedMovement {
                        player_id,
                        timestamp_ms: sample.timestamp_ms,
                        x: sample.position.x,
                        y: sample.position.y,
                        z: sample.position.z,
                        velocity_x: sample.velocity.as_ref().map(|velocity| velocity.x),
                        velocity_y: sample.velocity.as_ref().map(|velocity| velocity.y),
                        velocity_z: sample.velocity.as_ref().map(|velocity| velocity.z),
                    });
                    counts.movement_count += 1;
                    if movement.len() >= INSERT_BATCH_SIZE {
                        flush_movement(&mut transaction, match_id, &mut movement).await?;
                    }
                }
            }
        }
        flush_events(&mut transaction, match_id, &mut events).await?;
        flush_movement(&mut transaction, match_id, &mut movement).await?;
        let finalized_roster = finalized_roster.unwrap_or_else(|| roster.finalize(match_id));
        insert_replay_players(&mut transaction, match_id, finalized_roster.players).await?;

        if counts.event_count != replay.summary.event_count
            || counts.movement_count != replay.summary.movement_count
        {
            return Err(DatabaseError::SummaryMismatch {
                expected_events: replay.summary.event_count,
                actual_events: counts.event_count,
                expected_movement: replay.summary.movement_count,
                actual_movement: counts.movement_count,
            });
        }

        transaction.commit().await?;
        Ok(counts)
    }

    pub async fn list_players_for_match_for_user(
        &self,
        user_id: &str,
        match_id: &str,
    ) -> Result<Vec<PlayerRecord>, DatabaseError> {
        let rows = sqlx::query_as::<
            _,
            (
                String,
                String,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<i64>,
                i64,
            ),
        >(
            r#"
            SELECT players.id, players.match_id, players.stable_player_id, players.display_name,
                   players.team, players.agent_name, players.player_slot,
                   EXISTS(
                       SELECT 1 FROM valorant_accounts
                       WHERE valorant_accounts.user_id = matches.user_id
                         AND valorant_accounts.subject_id = players.stable_player_id
                   )
            FROM players
            JOIN matches ON matches.id = players.match_id
            WHERE players.match_id = ? AND matches.user_id = ?
              AND players.team IN ('team_a', 'team_b')
            ORDER BY players.team, players.player_slot, players.id
            "#,
        )
        .bind(match_id)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(
                |(
                    id,
                    match_id,
                    stable_player_id,
                    display_name,
                    team,
                    agent_name,
                    player_slot,
                    is_bound,
                )| PlayerRecord {
                    id,
                    match_id,
                    stable_player_id,
                    display_name,
                    team,
                    agent_name,
                    player_slot,
                    is_bound: is_bound != 0,
                },
            )
            .collect())
    }

    pub async fn movement_for_player_for_user(
        &self,
        user_id: &str,
        match_id: &str,
        player_id: &str,
    ) -> Result<Vec<MovementSample>, DatabaseError> {
        let rows = sqlx::query_as::<_, (
            i64,
            f64,
            f64,
            f64,
            Option<f64>,
            Option<f64>,
            Option<f64>,
        )>(
            r#"
            SELECT movement_samples.timestamp_ms, movement_samples.x, movement_samples.y,
                   movement_samples.z, movement_samples.velocity_x, movement_samples.velocity_y,
                   movement_samples.velocity_z
            FROM movement_samples
            JOIN matches ON matches.id = movement_samples.match_id
            WHERE movement_samples.match_id = ? AND movement_samples.player_id = ? AND matches.user_id = ?
            ORDER BY movement_samples.timestamp_ms, movement_samples.id
            "#,
        )
        .bind(match_id)
        .bind(player_id)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(
                |(timestamp_ms, x, y, z, velocity_x, velocity_y, velocity_z)| MovementSample {
                    timestamp_ms,
                    packet_id: None,
                    actor_net_guid: None,
                    character_net_guid: None,
                    position: Vector3 { x, y, z },
                    velocity: match (velocity_x, velocity_y, velocity_z) {
                        (Some(x), Some(y), Some(z)) => Some(Vector3 { x, y, z }),
                        _ => None,
                    },
                },
            )
            .collect())
    }

    pub async fn insert_match_metric(
        &self,
        metric_id: &str,
        match_id: &str,
        metric_name: &str,
        value_json: &str,
    ) -> Result<(), DatabaseError> {
        sqlx::query(
            "INSERT INTO match_metrics (id, match_id, metric_name, value_json) VALUES (?, ?, ?, ?)",
        )
        .bind(metric_id)
        .bind(match_id)
        .bind(metric_name)
        .bind(value_json)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_matches_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<MatchRecord>, DatabaseError> {
        self.match_records(
            r#"
            SELECT id, parser_source, metadata_json, capabilities_json, summary_json
            FROM matches WHERE user_id = ? ORDER BY created_at DESC, id DESC
            "#,
            user_id,
            None,
        )
        .await
    }

    pub async fn find_match_for_user(
        &self,
        user_id: &str,
        match_id: &str,
    ) -> Result<Option<MatchRecord>, DatabaseError> {
        let mut matches = self
            .match_records(
                r#"
                SELECT id, parser_source, metadata_json, capabilities_json, summary_json
                FROM matches WHERE user_id = ? AND id = ?
                "#,
                user_id,
                Some(match_id),
            )
            .await?;
        Ok(matches.pop())
    }

    pub async fn list_match_metrics_for_user(
        &self,
        user_id: &str,
        match_id: &str,
    ) -> Result<Vec<MatchMetricRecord>, DatabaseError> {
        let rows = sqlx::query_as::<_, (String, String, String)>(
            r#"
            SELECT match_metrics.id, match_metrics.metric_name, match_metrics.value_json
            FROM match_metrics
            JOIN matches ON matches.id = match_metrics.match_id
            WHERE match_metrics.match_id = ? AND matches.user_id = ?
            ORDER BY match_metrics.metric_name, match_metrics.id
            "#,
        )
        .bind(match_id)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(id, metric_name, value_json)| MatchMetricRecord {
                id,
                metric_name,
                value_json,
            })
            .collect())
    }

    pub async fn bind_player_to_account(
        &self,
        user_id: &str,
        match_id: &str,
        player_id: &str,
    ) -> Result<ValorantAccountRecord, DatabaseError> {
        let stable_player_id = sqlx::query_scalar::<_, Option<String>>(
            r#"
            SELECT players.stable_player_id
            FROM players
            JOIN matches ON matches.id = players.match_id
            WHERE players.id = ? AND players.match_id = ? AND matches.user_id = ?
            "#,
        )
        .bind(player_id)
        .bind(match_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?
        .flatten()
        .ok_or(DatabaseError::PlayerNotFound)?;
        let account_id = format!("{user_id}:global:{stable_player_id}");

        sqlx::query(
            r#"
            INSERT INTO valorant_accounts (id, user_id, region, subject_id, display_name)
            VALUES (?, ?, 'global', ?, NULL)
            ON CONFLICT(user_id, subject_id) WHERE subject_id IS NOT NULL
            DO UPDATE SET region = excluded.region
            "#,
        )
        .bind(&account_id)
        .bind(user_id)
        .bind(&stable_player_id)
        .execute(&self.pool)
        .await?;

        Ok(ValorantAccountRecord {
            id: account_id,
            user_id: user_id.to_owned(),
            region: "global".to_owned(),
            subject_id: Some(stable_player_id),
            display_name: None,
        })
    }

    pub async fn find_bound_player_for_match(
        &self,
        user_id: &str,
        match_id: &str,
    ) -> Result<Option<String>, DatabaseError> {
        Ok(sqlx::query_scalar::<_, String>(
            r#"
            SELECT players.id
            FROM players
            JOIN matches ON matches.id = players.match_id
            JOIN valorant_accounts
              ON valorant_accounts.user_id = matches.user_id
             AND valorant_accounts.subject_id = players.stable_player_id
            WHERE matches.user_id = ? AND matches.id = ?
            ORDER BY valorant_accounts.id
            LIMIT 1
            "#,
        )
        .bind(user_id)
        .bind(match_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn insert_agent_exchange(
        &self,
        user_id: &str,
        match_id: &str,
        session_id: &str,
        provider: &str,
        model: &str,
        question: &str,
        answer: &str,
        evidence_json: &str,
        limitations_json: &str,
        provider_request_id: Option<&str>,
        usage: &AgentTokenUsage,
    ) -> Result<(), DatabaseError> {
        let mut transaction = self.pool.begin().await?;
        let result = sqlx::query(
            r#"
            INSERT INTO conversations (id, user_id, title, match_id, provider, model)
            SELECT ?, ?, 'Replay coaching', matches.id, ?, ?
            FROM matches
            WHERE matches.id = ? AND matches.user_id = ?
            "#,
        )
        .bind(session_id)
        .bind(user_id)
        .bind(provider)
        .bind(model)
        .bind(match_id)
        .bind(user_id)
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() != 1 {
            return Err(DatabaseError::MatchNotFound);
        }
        sqlx::query(
            r#"
            INSERT INTO messages
                (id, conversation_id, role, content, evidence_json, limitations_json)
            VALUES (?, ?, 'user', ?, '[]', '[]')
            "#,
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(session_id)
        .bind(question)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            r#"
            INSERT INTO messages
                (id, conversation_id, role, content, evidence_json, limitations_json,
                 provider_request_id)
            VALUES (?, ?, 'assistant', ?, ?, ?, ?)
            "#,
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(session_id)
        .bind(answer)
        .bind(evidence_json)
        .bind(limitations_json)
        .bind(provider_request_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            r#"
            INSERT INTO llm_usage
                (id, user_id, conversation_id, model, input_tokens, output_tokens,
                 estimated_cost_micros, provider, total_tokens, cost_is_estimate)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(user_id)
        .bind(session_id)
        .bind(model)
        .bind(i64::try_from(usage.input_tokens).unwrap_or(i64::MAX))
        .bind(i64::try_from(usage.output_tokens).unwrap_or(i64::MAX))
        .bind(
            usage
                .cost_microusd
                .map_or(0, |value| i64::try_from(value).unwrap_or(i64::MAX)),
        )
        .bind(provider)
        .bind(i64::try_from(usage.total_tokens).unwrap_or(i64::MAX))
        .bind(i64::from(usage.cost_microusd.is_some()))
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn list_agent_messages_for_match(
        &self,
        user_id: &str,
        match_id: &str,
    ) -> Result<Vec<AgentMessageRecord>, DatabaseError> {
        let rows = sqlx::query_as::<
            _,
            (
                String,
                String,
                String,
                String,
                String,
                String,
                String,
                String,
                i64,
                i64,
                i64,
                Option<i64>,
            ),
        >(
            r#"
            SELECT messages.id, messages.conversation_id,
                   COALESCE(conversations.provider, 'legacy'),
                   COALESCE(conversations.model, 'unknown'),
                   messages.role, messages.content,
                   messages.evidence_json, messages.limitations_json,
                   CASE WHEN messages.role = 'assistant' THEN COALESCE(llm_usage.input_tokens, 0) ELSE 0 END,
                   CASE WHEN messages.role = 'assistant' THEN COALESCE(llm_usage.output_tokens, 0) ELSE 0 END,
                   CASE WHEN messages.role = 'assistant' THEN COALESCE(llm_usage.total_tokens, 0) ELSE 0 END,
                   CASE WHEN messages.role = 'assistant' AND llm_usage.cost_is_estimate = 1
                        THEN llm_usage.estimated_cost_micros ELSE NULL END
            FROM messages
            JOIN conversations ON conversations.id = messages.conversation_id
            LEFT JOIN llm_usage ON llm_usage.conversation_id = conversations.id
            WHERE conversations.user_id = ? AND conversations.match_id = ?
            ORDER BY conversations.created_at, conversations.id,
                     CASE messages.role WHEN 'user' THEN 0 ELSE 1 END,
                     messages.created_at, messages.id
            "#,
        )
        .bind(user_id)
        .bind(match_id)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(
                |(
                    id,
                    session_id,
                    provider,
                    model,
                    role,
                    content,
                    evidence_json,
                    limitations_json,
                    input_tokens,
                    output_tokens,
                    total_tokens,
                    cost_microusd,
                )| {
                    Ok(AgentMessageRecord {
                        id,
                        session_id,
                        provider,
                        model,
                        role,
                        content,
                        evidence: serde_json::from_str(&evidence_json)?,
                        limitations: serde_json::from_str(&limitations_json)?,
                        usage: AgentTokenUsage {
                            input_tokens: u64::try_from(input_tokens).unwrap_or_default(),
                            output_tokens: u64::try_from(output_tokens).unwrap_or_default(),
                            total_tokens: u64::try_from(total_tokens).unwrap_or_default(),
                            cost_microusd: cost_microusd
                                .and_then(|value| u64::try_from(value).ok()),
                        },
                    })
                },
            )
            .collect()
    }

    pub async fn agent_usage_for_user(
        &self,
        user_id: &str,
    ) -> Result<AgentUsageSummary, DatabaseError> {
        let (input, output, total, cost, priced) = sqlx::query_as::<_, (i64, i64, i64, i64, i64)>(
            r#"
            SELECT COALESCE(SUM(input_tokens), 0),
                   COALESCE(SUM(output_tokens), 0),
                   COALESCE(SUM(total_tokens), 0),
                   COALESCE(SUM(estimated_cost_micros), 0),
                   COALESCE(SUM(cost_is_estimate), 0)
            FROM llm_usage
            WHERE user_id = ?
            "#,
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(AgentUsageSummary {
            input_tokens: u64::try_from(input).unwrap_or_default(),
            output_tokens: u64::try_from(output).unwrap_or_default(),
            total_tokens: u64::try_from(total).unwrap_or_default(),
            cost_microusd: u64::try_from(cost).unwrap_or_default(),
            priced_requests: u64::try_from(priced).unwrap_or_default(),
        })
    }

    async fn match_records(
        &self,
        query: &str,
        user_id: &str,
        match_id: Option<&str>,
    ) -> Result<Vec<MatchRecord>, DatabaseError> {
        let mut request =
            sqlx::query_as::<_, (String, String, String, String, String)>(query).bind(user_id);
        if let Some(match_id) = match_id {
            request = request.bind(match_id);
        }
        let rows = request.fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(
                |(id, parser_source, metadata_json, capabilities_json, summary_json)| {
                    Ok(MatchRecord {
                        id,
                        parser_source,
                        metadata: serde_json::from_str(&metadata_json)?,
                        capabilities: serde_json::from_str(&capabilities_json)?,
                        summary: serde_json::from_str(&summary_json)?,
                    })
                },
            )
            .collect()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PersistedRecordCounts {
    pub event_count: u64,
    pub movement_count: u64,
}

#[derive(Debug, Default)]
struct ReplayRoster {
    player_states: BTreeMap<u64, ReplayPlayerDraft>,
    pawn_to_state: HashMap<u64, u64>,
}

#[derive(Debug, Default)]
struct ReplayPlayerDraft {
    slot: Option<i64>,
    subject: Option<String>,
    pawn_guids: BTreeSet<u64>,
    agent_name: Option<String>,
}

#[derive(Debug)]
struct ReplayPlayerIdentity {
    id: String,
    stable_player_id: String,
    team: String,
    agent_name: Option<String>,
    player_slot: i64,
}

#[derive(Debug, Default)]
struct FinalizedRoster {
    players: Vec<ReplayPlayerIdentity>,
    pawn_to_player: HashMap<u64, String>,
}

impl ReplayRoster {
    fn observe(&mut self, event: &valcoach_domain::GenericEvent) {
        if event.event_type != "export_group_received" {
            return;
        }
        let Some(actor_guid) = event.actor_net_guid else {
            return;
        };
        let payload = event.raw.get("payload").unwrap_or(&serde_json::Value::Null);
        let export_path = event
            .raw
            .get("export_group_path")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();

        if export_path.contains("BombPlayerState.BombPlayerState_C") {
            let draft = self.player_states.entry(actor_guid).or_default();
            if let Some(slot) = payload.get("PlayerId").and_then(serde_json::Value::as_i64) {
                draft.slot = Some(slot);
            }
            if let Some(subject) = payload
                .get("Subject")
                .and_then(serde_json::Value::as_str)
                .filter(|subject| !subject.is_empty())
            {
                draft.subject = Some(subject.to_owned());
            }
            for field in ["PossessedCharacter", "SpawnedCharacter"] {
                if let Some(pawn_guid) = payload.get(field).and_then(serde_json::Value::as_u64) {
                    draft.pawn_guids.insert(pawn_guid);
                    self.pawn_to_state.insert(pawn_guid, actor_guid);
                }
            }
        }

        if let Some(player_state_guid) = payload
            .get("PlayerState")
            .and_then(serde_json::Value::as_u64)
        {
            self.pawn_to_state.insert(actor_guid, player_state_guid);
            let draft = self.player_states.entry(player_state_guid).or_default();
            draft.pawn_guids.insert(actor_guid);
            if draft.agent_name.is_none() {
                draft.agent_name = event
                    .raw
                    .get("class_path")
                    .and_then(serde_json::Value::as_str)
                    .and_then(agent_codename);
            }
        }
    }

    fn finalize(&self, match_id: &str) -> FinalizedRoster {
        let mut candidates = self
            .player_states
            .iter()
            .filter_map(|(state_guid, draft)| {
                Some((*state_guid, draft.slot?, draft.subject.as_ref()?, draft))
            })
            .collect::<Vec<_>>();
        candidates.sort_by_key(|(_, slot, _, _)| *slot);
        candidates.truncate(10);

        let mut finalized = FinalizedRoster::default();
        for (index, (state_guid, slot, subject, draft)) in candidates.into_iter().enumerate() {
            let team = if index < 5 { "team_a" } else { "team_b" };
            let id = format!("{match_id}:player:{subject}");
            finalized.players.push(ReplayPlayerIdentity {
                id: id.clone(),
                stable_player_id: subject.clone(),
                team: team.to_owned(),
                agent_name: draft.agent_name.clone(),
                player_slot: slot,
            });
            for pawn_guid in &draft.pawn_guids {
                finalized.pawn_to_player.insert(*pawn_guid, id.clone());
            }
            for (pawn_guid, mapped_state_guid) in &self.pawn_to_state {
                if *mapped_state_guid == state_guid {
                    finalized.pawn_to_player.insert(*pawn_guid, id.clone());
                }
            }
        }
        finalized
    }
}

fn agent_codename(class_path: &str) -> Option<String> {
    let class_name = class_path.rsplit('/').next()?.strip_suffix("_PC")?;
    (!class_name.is_empty()).then(|| class_name.to_owned())
}

struct PersistedEvent {
    timestamp_ms: i64,
    event_type: String,
    actor_net_guid: Option<String>,
    payload_json: String,
}

struct PersistedMovement {
    player_id: Option<String>,
    timestamp_ms: i64,
    x: f64,
    y: f64,
    z: f64,
    velocity_x: Option<f64>,
    velocity_y: Option<f64>,
    velocity_z: Option<f64>,
}

async fn flush_events(
    transaction: &mut Transaction<'_, Sqlite>,
    match_id: &str,
    records: &mut Vec<PersistedEvent>,
) -> Result<(), DatabaseError> {
    if records.is_empty() {
        return Ok(());
    }
    let rows = std::mem::take(records);
    let mut query = QueryBuilder::<Sqlite>::new(
        "INSERT INTO events (match_id, timestamp_ms, event_type, actor_net_guid, payload_json) ",
    );
    query.push_values(rows, |mut row, event| {
        row.push_bind(match_id)
            .push_bind(event.timestamp_ms)
            .push_bind(event.event_type)
            .push_bind(event.actor_net_guid)
            .push_bind(event.payload_json);
    });
    query.build().execute(&mut **transaction).await?;
    Ok(())
}

async fn flush_movement(
    transaction: &mut Transaction<'_, Sqlite>,
    match_id: &str,
    records: &mut Vec<PersistedMovement>,
) -> Result<(), DatabaseError> {
    if records.is_empty() {
        return Ok(());
    }
    let rows = std::mem::take(records);
    let mut query = QueryBuilder::<Sqlite>::new(
        "INSERT INTO movement_samples (match_id, player_id, timestamp_ms, x, y, z, velocity_x, velocity_y, velocity_z) ",
    );
    query.push_values(rows, |mut row, sample| {
        row.push_bind(match_id)
            .push_bind(sample.player_id)
            .push_bind(sample.timestamp_ms)
            .push_bind(sample.x)
            .push_bind(sample.y)
            .push_bind(sample.z)
            .push_bind(sample.velocity_x)
            .push_bind(sample.velocity_y)
            .push_bind(sample.velocity_z);
    });
    query.build().execute(&mut **transaction).await?;
    Ok(())
}

async fn insert_replay_players(
    transaction: &mut Transaction<'_, Sqlite>,
    match_id: &str,
    players: Vec<ReplayPlayerIdentity>,
) -> Result<(), DatabaseError> {
    if players.is_empty() {
        return Ok(());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "INSERT INTO players (id, match_id, stable_player_id, display_name, team, agent_name, player_slot) ",
    );
    query.push_values(players, |mut row, player| {
        row.push_bind(player.id)
            .push_bind(match_id)
            .push_bind(player.stable_player_id)
            .push_bind(Option::<String>::None)
            .push_bind(player.team)
            .push_bind(player.agent_name)
            .push_bind(player.player_slot);
    });
    query.build().execute(&mut **transaction).await?;
    Ok(())
}

#[derive(Debug, Error)]
pub enum DatabaseError {
    #[error("invalid SQLite connection URL: {0}")]
    InvalidUrl(#[source] sqlx::Error),
    #[error("database migration failed: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("username already exists")]
    UsernameAlreadyExists,
    #[error("selected replay player was not found for the current user")]
    PlayerNotFound,
    #[error("selected match was not found for the current user")]
    MatchNotFound,
    #[error("database operation failed: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("failed to serialize stable domain data: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error(transparent)]
    Adapter(#[from] ReplaySourceError),
    #[error(
        "normalized record count differs from validated summary (events {actual_events}/{expected_events}, movement {actual_movement}/{expected_movement})"
    )]
    SummaryMismatch {
        expected_events: u64,
        actual_events: u64,
        expected_movement: u64,
        actual_movement: u64,
    },
}

#[cfg(test)]
mod tests {
    use valcoach_domain::{
        ParsedBundle, ParsedReplay, ParsedReplaySummary, ReplayCapabilities, ReplayMetadata,
    };

    use super::{AgentTokenUsage, Database, ReplayRoster, UserRecord};

    #[test]
    fn replay_roster_collapses_respawns_into_two_five_player_teams() {
        let mut roster = ReplayRoster::default();
        for index in 0_u64..10 {
            let state_guid = 200 + index;
            let first_pawn = 1_000 + index;
            roster.observe(&valcoach_domain::GenericEvent {
                event_type: "export_group_received".to_owned(),
                timestamp_ms: 8,
                actor_net_guid: Some(state_guid),
                raw: serde_json::json!({
                    "export_group_path": "/Game/GameModes/Bomb/BombPlayerState.BombPlayerState_C",
                    "payload": {
                        "PlayerId": 256 + index,
                        "Subject": format!("subject-{index}"),
                        "PossessedCharacter": first_pawn
                    }
                }),
            });
            roster.observe(&valcoach_domain::GenericEvent {
                event_type: "export_group_received".to_owned(),
                timestamp_ms: 9,
                actor_net_guid: Some(first_pawn),
                raw: serde_json::json!({
                    "export_group_path": "/Game/Characters/Sprinter/Sprinter_PC.Sprinter_PC_C",
                    "class_path": "/Game/Characters/Sprinter/Sprinter_PC",
                    "payload": { "PlayerState": state_guid }
                }),
            });
            let respawn = 2_000 + index;
            roster.observe(&valcoach_domain::GenericEvent {
                event_type: "export_group_received".to_owned(),
                timestamp_ms: 10,
                actor_net_guid: Some(state_guid),
                raw: serde_json::json!({
                    "export_group_path": "/Game/GameModes/Bomb/BombPlayerState.BombPlayerState_C",
                    "payload": { "PossessedCharacter": respawn }
                }),
            });
        }

        let finalized = roster.finalize("match-1");
        assert_eq!(finalized.players.len(), 10);
        assert_eq!(
            finalized
                .players
                .iter()
                .filter(|player| player.team == "team_a")
                .count(),
            5
        );
        assert_eq!(
            finalized
                .players
                .iter()
                .filter(|player| player.team == "team_b")
                .count(),
            5
        );
        assert_eq!(finalized.pawn_to_player.len(), 20);
        assert_eq!(finalized.players[0].agent_name.as_deref(), Some("Sprinter"));
    }

    #[tokio::test]
    async fn migrations_and_match_summary_persistence_work() {
        let database = Database::connect("sqlite::memory:")
            .await
            .expect("database");
        sqlx::query(
            "INSERT INTO users (id, username, password_hash) VALUES ('user-1', 'demo', 'hash')",
        )
        .execute(database.pool())
        .await
        .expect("user");
        let replay = ParsedReplay {
            metadata: ReplayMetadata {
                replay_id: "fixture".to_owned(),
                branch: Some("++Ares-Core+release-13.00".to_owned()),
                map: None,
                duration_ms: Some(39_080),
            },
            bundle: ParsedBundle {
                events_path: "events.ndjson".into(),
                movement_path: "movement.ndjson".into(),
            },
            source_name: "parsed_bundle".to_owned(),
            capabilities: ReplayCapabilities::global_fixture(
                valcoach_domain::CapabilityLevel::Partial,
            ),
            summary: ParsedReplaySummary::default(),
        };

        database
            .insert_match_summary("user-1", "match-1", &replay)
            .await
            .expect("match summary");

        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM matches WHERE user_id = 'user-1'")
                .fetch_one(database.pool())
                .await
                .expect("match count");
        assert_eq!(count, 1);

        database
            .insert_agent_exchange(
                "user-1",
                "match-1",
                "conversation-1",
                "openai",
                "test-model",
                "How did I move?",
                "Observed movement only.",
                r#"[{"match_id":"match-1","timestamp_ms":100}]"#,
                r#"["raw units"]"#,
                Some("resp-1"),
                &AgentTokenUsage {
                    input_tokens: 12,
                    output_tokens: 5,
                    total_tokens: 17,
                    cost_microusd: Some(34),
                },
            )
            .await
            .expect("agent exchange");
        let messages = database
            .list_agent_messages_for_match("user-1", "match-1")
            .await
            .expect("agent messages");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].usage.total_tokens, 17);
        let usage = database
            .agent_usage_for_user("user-1")
            .await
            .expect("agent usage");
        assert_eq!(usage.total_tokens, 17);
        assert_eq!(usage.cost_microusd, 34);
        assert_eq!(usage.priced_requests, 1);
    }

    #[tokio::test]
    async fn user_lookup_and_duplicate_protection_work() {
        let database = Database::connect("sqlite::memory:")
            .await
            .expect("database");
        let user = UserRecord {
            id: "user-1".to_owned(),
            username: "coach".to_owned(),
            password_hash: "hash".to_owned(),
        };

        database.create_user(&user).await.expect("create user");
        assert_eq!(
            database
                .find_user_by_username("coach")
                .await
                .expect("lookup"),
            Some(user.clone())
        );
        assert!(matches!(
            database.create_user(&user).await,
            Err(super::DatabaseError::UsernameAlreadyExists)
        ));
    }

    #[tokio::test]
    async fn parse_job_is_scoped_to_its_owner() {
        let database = Database::connect("sqlite::memory:")
            .await
            .expect("database");
        for id in ["user-1", "user-2"] {
            database
                .create_user(&UserRecord {
                    id: id.to_owned(),
                    username: id.to_owned(),
                    password_hash: "hash".to_owned(),
                })
                .await
                .expect("user");
        }
        database
            .create_parse_job("job-1", "user-1", "valorant_replay_parser")
            .await
            .expect("job");
        database
            .update_parse_job("job-1", "ready", None, None)
            .await
            .expect("job status");

        assert_eq!(
            database
                .find_parse_job_for_user("job-1", "user-2")
                .await
                .expect("foreign lookup"),
            None
        );
        assert_eq!(
            database
                .find_parse_job_for_user("job-1", "user-1")
                .await
                .expect("owner lookup")
                .expect("job")
                .status,
            "ready"
        );
    }
}
