//! SQLite persistence for ValCoach-owned stable domain data.

mod semantic;

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    str::FromStr,
};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};
use sqlx::{QueryBuilder, Sqlite, Transaction};
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use valcoach_domain::{
    MovementSample, ParsedReplay, ParsedReplaySummary, ReplayCapabilities, ReplayMetadata, Vector3,
};
use valcoach_replay_adapter::{NormalizedRecord, ParsedBundleSource, ReplaySourceError};

use semantic::SemanticBuilder;

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

#[derive(Debug, Clone, Serialize)]
pub struct SemanticContext {
    pub context: Value,
    pub evidence: Vec<Value>,
    pub limitations: Vec<String>,
}

fn collect_evidence(events: &[Value], target: &mut Vec<Value>) {
    for event in events {
        if let Some(items) = event.get("evidence").and_then(Value::as_array) {
            target.extend(items.iter().cloned());
        } else if let Some(item) = event.get("evidence") {
            target.push(item.clone());
        }
    }
}

/// Merge consecutive shots from the same attacker + weapon into compact bursts.
/// Each burst summarizes: shot count, total damage, kills, hit regions, positions.
/// Damage events within 200ms of a burst are associated with it.
/// Standalone damage events (no nearby shots) remain as individual events.
fn compact_combat_events(raw: &[Value]) -> Vec<Value> {
    if raw.is_empty() {
        return Vec::new();
    }
    let mut result: Vec<Value> = Vec::new();
    let mut current_burst: Option<BurstAccumulator> = None;
    const BURST_GAP_MS: i64 = 500;
    const DAMAGE_ASSOCIATION_MS: i64 = 200;

    for event in raw {
        let kind = event.get("kind").and_then(Value::as_str).unwrap_or("");
        let time_ms = event.get("time_ms").and_then(Value::as_i64).unwrap_or(0);
        let attacker = event.get("attacker").and_then(Value::as_str).unwrap_or("");
        let weapon = event.get("weapon").and_then(Value::as_str).unwrap_or("");
        let damage = event.get("damage").and_then(Value::as_f64);
        let killed = event.get("killed").and_then(Value::as_bool).unwrap_or(false);
        let hit_region = event.get("hit_region").and_then(Value::as_str).map(str::to_owned);
        let area = event.get("area").and_then(Value::as_str).map(str::to_owned);
        let attacker_pos = event.get("attacker_position").cloned();
        let victim_pos = event.get("victim_position").cloned();
        let victim = event.get("victim").and_then(Value::as_str).map(str::to_owned);
        let evidence = event.get("evidence").cloned().unwrap_or(json!([]));

        if kind == "shot" {
            if let Some(ref burst) = current_burst {
                let same_attacker = burst.attacker == attacker;
                let same_weapon = burst.weapon == weapon || weapon.is_empty();
                let within_gap = time_ms - burst.last_shot_ms <= BURST_GAP_MS;
                if same_attacker && same_weapon && within_gap {
                    current_burst.as_mut().unwrap().add_shot(time_ms, &area, &attacker_pos, evidence);
                    continue;
                }
                result.push(current_burst.take().unwrap().finalize());
            }
            let mut burst = BurstAccumulator::new(attacker, weapon, time_ms, area.clone(), attacker_pos.clone());
            burst.add_shot(time_ms, &area, &attacker_pos, evidence);
            current_burst = Some(burst);
        } else if kind == "damage" {
            if let Some(ref mut burst) = current_burst {
                let within_assoc = time_ms - burst.last_shot_ms <= DAMAGE_ASSOCIATION_MS
                    || burst.first_shot_ms - time_ms <= DAMAGE_ASSOCIATION_MS;
                if within_assoc {
                    burst.add_damage(damage, killed, hit_region.as_deref(), &area, victim.as_deref(), &victim_pos, evidence);
                    continue;
                }
            }
            if let Some(burst) = current_burst.take() {
                result.push(burst.finalize());
            }
            let mut standalone = BurstAccumulator::new(attacker, weapon, time_ms, area.clone(), attacker_pos.clone());
            standalone.add_damage(damage, killed, hit_region.as_deref(), &area, victim.as_deref(), &victim_pos, evidence);
            standalone.shot_count = 0;
            result.push(standalone.finalize());
        } else {
            if let Some(burst) = current_burst.take() {
                result.push(burst.finalize());
            }
            result.push(event.clone());
        }
    }
    if let Some(burst) = current_burst {
        result.push(burst.finalize());
    }
    result
}

struct BurstAccumulator {
    attacker: String,
    weapon: String,
    first_shot_ms: i64,
    last_shot_ms: i64,
    shot_count: usize,
    total_damage: f64,
    killed: bool,
    hit_regions: Vec<String>,
    area: Option<String>,
    attacker_position: Option<Value>,
    victim: Option<String>,
    victim_position: Option<Value>,
    evidence: Vec<Value>,
}

impl BurstAccumulator {
    fn new(attacker: &str, weapon: &str, time_ms: i64, area: Option<String>, pos: Option<Value>) -> Self {
        Self {
            attacker: attacker.to_owned(),
            weapon: weapon.to_owned(),
            first_shot_ms: time_ms,
            last_shot_ms: time_ms,
            shot_count: 0,
            total_damage: 0.0,
            killed: false,
            hit_regions: Vec::new(),
            area,
            attacker_position: pos,
            victim: None,
            victim_position: None,
            evidence: Vec::new(),
        }
    }

    fn add_shot(&mut self, time_ms: i64, area: &Option<String>, pos: &Option<Value>, evidence: Value) {
        self.shot_count += 1;
        self.last_shot_ms = time_ms;
        if self.area.is_none() {
            self.area = area.clone();
        }
        if self.attacker_position.is_none() {
            self.attacker_position = pos.clone();
        }
        if let Some(arr) = evidence.as_array() {
            self.evidence.extend(arr.iter().cloned());
        } else {
            self.evidence.push(evidence);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn add_damage(
        &mut self,
        damage: Option<f64>,
        killed: bool,
        hit_region: Option<&str>,
        area: &Option<String>,
        victim: Option<&str>,
        victim_pos: &Option<Value>,
        evidence: Value,
    ) {
        if let Some(d) = damage {
            self.total_damage += d;
        }
        if killed {
            self.killed = true;
        }
        if let Some(region) = hit_region {
            self.hit_regions.push(region.to_owned());
        }
        if self.area.is_none() {
            self.area = area.clone();
        }
        if self.victim.is_none() {
            self.victim = victim.map(str::to_owned);
        }
        if self.victim_position.is_none() {
            self.victim_position = victim_pos.clone();
        }
        if let Some(arr) = evidence.as_array() {
            self.evidence.extend(arr.iter().cloned());
        } else {
            self.evidence.push(evidence);
        }
    }

    fn finalize(self) -> Value {
        let kind = if self.shot_count > 0 { "burst" } else { "damage" };
        if self.shot_count == 1 && self.total_damage == 0.0 && !self.killed {
            json!({
                "kind": "shot",
                "time_ms": self.first_shot_ms,
                "attacker": self.attacker,
                "weapon": self.weapon,
                "area": self.area,
                "attacker_position": self.attacker_position,
                "evidence": self.evidence,
            })
        } else {
            json!({
                "kind": kind,
                "time_ms": self.first_shot_ms,
                "attacker": self.attacker,
                "victim": self.victim,
                "weapon": self.weapon,
                "shots": self.shot_count,
                "damage": if self.total_damage > 0.0 { Some(self.total_damage) } else { None },
                "killed": self.killed,
                "hit_regions": if self.hit_regions.is_empty() { None } else { Some(self.hit_regions) },
                "area": self.area,
                "attacker_position": self.attacker_position,
                "victim_position": self.victim_position,
                "evidence": self.evidence,
            })
        }
    }
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
        let mut semantic =
            SemanticBuilder::load(match_id, replay.bundle.server_events_path.as_deref()).await?;
        let records = ParsedBundleSource.records(replay.bundle.clone(), cancel);
        futures_util::pin_mut!(records);

        while let Some(record) = records.next().await {
            match record? {
                NormalizedRecord::Event(event) => {
                    roster.observe(&event);
                    semantic.observe_event(&event, counts.event_count + 1);
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
                    if counts.movement_count == 0 {
                        semantic.resolve_players(finalized);
                    }
                    let player_id = sample
                        .character_net_guid
                        .and_then(|guid| finalized.pawn_to_player.get(&guid).cloned());
                    let enrichment = semantic.enrich_movement(
                        &sample,
                        player_id.as_deref(),
                        counts.movement_count + 1,
                    );
                    movement.push(PersistedMovement {
                        player_id,
                        timestamp_ms: sample.timestamp_ms,
                        x: sample.position.x,
                        y: sample.position.y,
                        z: sample.position.z,
                        velocity_x: sample.velocity.as_ref().map(|velocity| velocity.x),
                        velocity_y: sample.velocity.as_ref().map(|velocity| velocity.y),
                        velocity_z: sample.velocity.as_ref().map(|velocity| velocity.z),
                        round_no: enrichment.round_no,
                        yaw: sample.yaw,
                        pitch: sample.pitch,
                        alive: enrichment.alive,
                        area: enrichment.area,
                        source_row: enrichment.source_row,
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
        if counts.movement_count == 0 {
            semantic.resolve_players(&finalized_roster);
        }
        if let Some(map) = &replay.metadata.map {
            semantic.set_map(map);
        }
        semantic.set_duration(replay.metadata.duration_ms);
        semantic.finish();
        insert_semantic_replay(&mut transaction, match_id, &semantic).await?;
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

    pub async fn insert_probe_players(
        &self,
        user_id: &str,
        match_id: &str,
        players: &[(String, String)],
    ) -> Result<(), DatabaseError> {
        let owns_match = sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM matches WHERE id = ? AND user_id = ?)",
        )
        .bind(match_id)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?
            != 0;
        if !owns_match {
            return Err(DatabaseError::MatchNotFound);
        }
        for (index, (subject, agent_name)) in players.iter().take(10).enumerate() {
            sqlx::query(
                r#"INSERT INTO players
                   (id, match_id, stable_player_id, display_name, team, agent_name, player_slot,
                    player_state_net_guid, character_net_guids_json)
                   VALUES (?, ?, ?, NULL, ?, ?, ?, NULL, '[]')"#,
            )
            .bind(format!("{match_id}:player:{subject}"))
            .bind(match_id)
            .bind(subject)
            .bind(if index < 5 { "team_a" } else { "team_b" })
            .bind(agent_name)
            .bind(index as i64)
            .execute(&self.pool)
            .await?;
        }
        if let Some(mut diagnostics) = self.semantic_diagnostics(match_id).await? {
            diagnostics["players"]["resolved"] = json!(players.len().min(10));
            sqlx::query("UPDATE semantic_diagnostics SET value_json = ? WHERE match_id = ?")
                .bind(serde_json::to_string(&diagnostics)?)
                .bind(match_id)
                .execute(&self.pool)
                .await?;
        }
        Ok(())
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
            Option<i64>,
            Option<i64>,
            Option<String>,
        )>(
            r#"
            SELECT movement_samples.timestamp_ms, movement_samples.x, movement_samples.y,
                   movement_samples.z, movement_samples.velocity_x, movement_samples.velocity_y,
                   movement_samples.velocity_z, movement_samples.round_no,
                   movement_samples.alive, movement_samples.area
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
                |(
                    timestamp_ms,
                    x,
                    y,
                    z,
                    velocity_x,
                    velocity_y,
                    velocity_z,
                    round_no,
                    alive,
                    area,
                )| MovementSample {
                    timestamp_ms,
                    packet_id: None,
                    actor_net_guid: None,
                    character_net_guid: None,
                    position: Vector3 { x, y, z },
                    velocity: match (velocity_x, velocity_y, velocity_z) {
                        (Some(x), Some(y), Some(z)) => Some(Vector3 { x, y, z }),
                        _ => None,
                    },
                    yaw: None,
                    pitch: None,
                    round_no: round_no.map(|value| value as u32),
                    alive: alive.map(|value| value != 0),
                    area,
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

    pub async fn delete_match_for_user(
        &self,
        user_id: &str,
        match_id: &str,
    ) -> Result<Vec<String>, DatabaseError> {
        let job_ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM parse_jobs WHERE match_id = ? AND user_id = ?",
        )
        .bind(match_id)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        sqlx::query("DELETE FROM matches WHERE id = ? AND user_id = ?")
            .bind(match_id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(job_ids)
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

    pub async fn semantic_diagnostics(
        &self,
        match_id: &str,
    ) -> Result<Option<Value>, DatabaseError> {
        let value = sqlx::query_scalar::<_, String>(
            "SELECT value_json FROM semantic_diagnostics WHERE match_id = ?",
        )
        .bind(match_id)
        .fetch_optional(&self.pool)
        .await?;
        value
            .map(|json| serde_json::from_str(&json))
            .transpose()
            .map_err(Into::into)
    }

    /// Deterministic Agent retrieval tools. The returned context contains only the rounds
    /// selected by the user's question scope, never the raw replay dump.
    pub async fn build_semantic_coaching_context(
        &self,
        user_id: &str,
        match_id: &str,
        player_id: &str,
        requested_round: Option<u32>,
        requested_area: Option<&str>,
        requested_side: Option<&str>,
    ) -> Result<SemanticContext, DatabaseError> {
        let player = sqlx::query_as::<_, (Option<String>, Option<String>, Option<String>, String)>(
            r#"SELECT stable_player_id, team, agent_name, character_net_guids_json
               FROM players JOIN matches ON matches.id = players.match_id
               WHERE players.id = ? AND players.match_id = ? AND matches.user_id = ?"#,
        )
        .bind(player_id)
        .bind(match_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(DatabaseError::PlayerNotFound)?;

        let all_players = sqlx::query_as::<
            _,
            (
                String,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<i64>,
            ),
        >(
            "SELECT id, stable_player_id, team, agent_name, player_slot FROM players WHERE match_id = ? ORDER BY team, player_slot, id",
        )
        .bind(match_id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|row| {
            json!({"id":row.0,"subject":row.1,"team":row.2,"agent":row.3,"slot":row.4})
        })
        .collect::<Vec<_>>();

        let rounds = self.get_rounds(match_id).await?;
        let selected_numbers = if let Some(round_no) = requested_round {
            vec![round_no]
        } else if let Some(area) = requested_area {
            self.find_rounds_by_area(
                match_id,
                player_id,
                area,
                requested_side,
                player.1.as_deref(),
            )
            .await?
        } else {
            let mut active = sqlx::query_scalar::<_, i64>(
                r#"SELECT DISTINCT round_no FROM combat_events
                   WHERE match_id = ? AND (attacker_player_id = ? OR victim_player_id = ?)
                   ORDER BY round_no"#,
            )
            .bind(match_id)
            .bind(player_id)
            .bind(player_id)
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(|value| value as u32)
            .collect::<Vec<_>>();
            if active.len() > 8 {
                active = active.split_off(active.len() - 8);
            }
            if active.is_empty() {
                active = rounds
                    .iter()
                    .rev()
                    .take(8)
                    .filter_map(|round| {
                        round
                            .get("round_no")
                            .and_then(Value::as_u64)
                            .map(|value| value as u32)
                    })
                    .collect();
                active.reverse();
            }
            active
        };

        let mut round_contexts = Vec::new();
        let mut evidence = Vec::new();
        let area_occupancy = if let Some(area) = requested_area {
            self.get_area_occupancy(
                match_id,
                player_id,
                area,
                requested_side,
                player.1.as_deref(),
            )
            .await?
        } else {
            Vec::new()
        };
        for round_no in selected_numbers.into_iter().take(8) {
            let movement = self
                .get_player_movement(match_id, player_id, round_no)
                .await?;
            let raw_combat = self
                .get_combat_events(match_id, player_id, round_no)
                .await?;
            let combat = compact_combat_events(&raw_combat);
            let abilities = self
                .get_ability_events(match_id, player_id, round_no)
                .await?;
            let spike = self.get_spike_events(match_id, round_no).await?;
            let mut nearby_players_at_combat = Vec::new();
            for event in combat.iter().take(16) {
                let selected_is_attacker =
                    event.get("attacker").and_then(Value::as_str) == Some(player_id);
                let position_key = if selected_is_attacker {
                    "attacker_position"
                } else {
                    "victim_position"
                };
                let Some(position) = event.get(position_key) else {
                    continue;
                };
                let (Some(timestamp_ms), Some(x), Some(y), Some(z)) = (
                    event.get("time_ms").and_then(Value::as_i64),
                    position.get("x").and_then(Value::as_f64),
                    position.get("y").and_then(Value::as_f64),
                    position.get("z").and_then(Value::as_f64),
                ) else {
                    continue;
                };
                nearby_players_at_combat.push(json!({
                    "time_ms": timestamp_ms,
                    "human_time": valcoach_domain::humanize::humanize_time(timestamp_ms, Some(round_no),
                        rounds.iter().find(|r| r.get("round_no").and_then(Value::as_u64) == Some(round_no as u64))
                        .and_then(|r| r.get("start_ms").and_then(Value::as_i64))),
                    "origin": position,
                    "radius_units": 2500,
                    "players": self.get_nearby_players(match_id, timestamp_ms, x, y, z, 2500.0).await?
                }));
            }
            let round_start_ms = rounds.iter()
                .find(|round| round.get("round_no").and_then(Value::as_u64) == Some(round_no as u64))
                .and_then(|round| round.get("start_ms").and_then(Value::as_i64));
            let humanized_combat: Vec<Value> = combat.iter().map(|event| {
                let time_ms = event.get("time_ms").and_then(Value::as_i64).unwrap_or(0);
                let human_time = valcoach_domain::humanize::humanize_time(time_ms, Some(round_no), round_start_ms);
                let mut enriched = event.clone();
                if let Some(obj) = enriched.as_object_mut() {
                    obj.insert("human_time".to_string(), json!(human_time));
                }
                enriched
            }).collect();
            let humanized_abilities: Vec<Value> = abilities.iter().map(|event| {
                let time_ms = event.get("time_ms").and_then(Value::as_i64).unwrap_or(0);
                let human_time = valcoach_domain::humanize::humanize_time(time_ms, Some(round_no), round_start_ms);
                let mut enriched = event.clone();
                if let Some(obj) = enriched.as_object_mut() {
                    obj.insert("human_time".to_string(), json!(human_time));
                }
                enriched
            }).collect();
            let humanized_spike: Vec<Value> = spike.iter().map(|event| {
                let time_ms = event.get("time_ms").and_then(Value::as_i64).unwrap_or(0);
                let human_time = valcoach_domain::humanize::humanize_time(time_ms, Some(round_no), round_start_ms);
                let mut enriched = event.clone();
                if let Some(obj) = enriched.as_object_mut() {
                    obj.insert("human_time".to_string(), json!(human_time));
                }
                enriched
            }).collect();
            let humanized_movement: Vec<Value> = movement.iter().map(|event| {
                let time_ms = event.get("time_ms").and_then(Value::as_i64).unwrap_or(0);
                let human_time = valcoach_domain::humanize::humanize_time(time_ms, Some(round_no), round_start_ms);
                let mut enriched = event.clone();
                if let Some(obj) = enriched.as_object_mut() {
                    obj.insert("human_time".to_string(), json!(human_time));
                }
                enriched
            }).collect();
            collect_evidence(&humanized_combat, &mut evidence);
            collect_evidence(&humanized_abilities, &mut evidence);
            collect_evidence(&humanized_spike, &mut evidence);
            collect_evidence(&humanized_movement, &mut evidence);
            if let Some(round) = rounds.iter().find(|round| {
                round.get("round_no").and_then(Value::as_u64) == Some(round_no as u64)
            }) {
                collect_evidence(std::slice::from_ref(round), &mut evidence);
            }
            round_contexts.push(json!({
                "round": rounds.iter().find(|round| round.get("round_no").and_then(Value::as_u64) == Some(round_no as u64)),
                "movement_area_timeline": humanized_movement,
                "combat": humanized_combat,
                "abilities": humanized_abilities,
                "spike": humanized_spike,
                "nearby_players_at_combat": nearby_players_at_combat,
            }));
        }
        evidence.sort_by_key(Value::to_string);
        evidence.dedup();
        let diagnostics = self.semantic_diagnostics(match_id).await?;
        let mut limitations = Vec::new();
        if round_contexts.is_empty() {
            limitations
                .push("No semantic rounds matched the requested area/side scope.".to_owned());
        }
        if diagnostics
            .as_ref()
            .and_then(|value| value.pointer("/movement/semantic_rows"))
            .and_then(Value::as_u64)
            == Some(0)
        {
            limitations.push(
                "This replay branch has no decoded movement, aim, alive-state, or area evidence."
                    .to_owned(),
            );
        }
        if diagnostics
            .as_ref()
            .and_then(|value| value.pointer("/combat/shots"))
            .and_then(Value::as_u64)
            == Some(0)
        {
            limitations.push(
                "This replay branch has no parser-decoded shot or damage evidence.".to_owned(),
            );
        }
        Ok(SemanticContext {
            context: json!({
                "question_scope": { "round": requested_round, "area": requested_area, "side": requested_side },
                "player": { "id": player_id, "subject": player.0, "team": player.1, "agent": player.2,
                    "character_net_guids": serde_json::from_str::<Value>(&player.3).unwrap_or_else(|_| json!([])) },
                "players": all_players,
                "all_rounds": rounds,
                "area_occupancy": area_occupancy,
                "relevant_rounds": round_contexts,
                "semantic_diagnostics": diagnostics,
                "tools_used": ["get_players", "get_rounds", "find_rounds_by_area", "get_round_timeline",
                    "get_player_movement", "get_combat_events", "get_ability_events", "get_spike_events",
                    "get_area_occupancy", "get_nearby_players"]
            }),
            evidence,
            limitations,
        })
    }

    async fn get_rounds(&self, match_id: &str) -> Result<Vec<Value>, DatabaseError> {
        let rows = sqlx::query_as::<_, (i64, Option<i64>, Option<i64>, Option<i64>, Option<String>, Option<String>, Option<String>, String)>(
            "SELECT round_no, start_ms, buy_end_ms, end_ms, team_a_side, team_b_side, winner_team, evidence_json FROM rounds WHERE match_id = ? ORDER BY round_no",
        )
        .bind(match_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                json!({ "round_no": row.0, "start_ms": row.1, "buy_end_ms": row.2,
            "end_ms": row.3, "team_a_side": row.4, "team_b_side": row.5, "winner_team": row.6,
            "evidence": serde_json::from_str::<Value>(&row.7).unwrap_or_else(|_| json!([])) })
            })
            .collect())
    }

    async fn find_rounds_by_area(
        &self,
        match_id: &str,
        player_id: &str,
        area: &str,
        side: Option<&str>,
        team: Option<&str>,
    ) -> Result<Vec<u32>, DatabaseError> {
        let area_pattern = if matches!(area, "A" | "B") {
            format!("{area} %")
        } else {
            area.to_owned()
        };
        let side_column = if team == Some("team_b") {
            "rounds.team_b_side"
        } else {
            "rounds.team_a_side"
        };
        let sql = format!(
            "SELECT DISTINCT movement_samples.round_no FROM movement_samples JOIN rounds ON rounds.match_id = movement_samples.match_id AND rounds.round_no = movement_samples.round_no WHERE movement_samples.match_id = ? AND movement_samples.player_id = ? AND movement_samples.area LIKE ? AND (? IS NULL OR {side_column} = ?) ORDER BY movement_samples.round_no"
        );
        Ok(sqlx::query_scalar::<_, i64>(&sql)
            .bind(match_id)
            .bind(player_id)
            .bind(area_pattern)
            .bind(side)
            .bind(side)
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(|value| value as u32)
            .collect())
    }

    async fn get_player_movement(
        &self,
        match_id: &str,
        player_id: &str,
        round_no: u32,
    ) -> Result<Vec<Value>, DatabaseError> {
        let rows = sqlx::query_as::<_, (i64, f64, f64, f64, Option<f64>, Option<f64>, Option<f64>, Option<f64>, Option<f64>, Option<i64>, Option<String>, Option<i64>)>(
            "SELECT timestamp_ms, x, y, z, velocity_x, velocity_y, velocity_z, yaw, pitch, alive, area, source_row FROM movement_samples WHERE match_id = ? AND player_id = ? AND round_no = ? ORDER BY timestamp_ms, id",
        ).bind(match_id).bind(player_id).bind(round_no).fetch_all(&self.pool).await?;
        let mut result = Vec::new();
        let mut last_area: Option<String> = None;
        let mut last_time = i64::MIN / 2;
        for row in rows {
            if row.10 != last_area || row.0 - last_time >= 5_000 {
                last_area = row.10.clone();
                last_time = row.0;
                let velocity = match (row.4, row.5, row.6) {
                    (Some(vx), Some(vy), Some(vz)) => Some(json!({"x": vx, "y": vy, "z": vz})),
                    _ => None,
                };
                result.push(json!({ "time_ms": row.0, "position": {"x":row.1,"y":row.2,"z":row.3},
                    "velocity": velocity,
                    "yaw": row.7, "pitch": row.8, "alive": row.9.map(|value| value != 0), "area": row.10,
                    "evidence": {"match_id":match_id,"round_no":round_no,"timestamp_ms":row.0,
                        "player_id":player_id,"evidence_type":"movement","source_file":"movement.ndjson",
                        "source_row":row.11,"source_event_type":"movement_sample"} }));
            }
        }
        Ok(result)
    }

    async fn get_area_occupancy(
        &self,
        match_id: &str,
        player_id: &str,
        area: &str,
        side: Option<&str>,
        team: Option<&str>,
    ) -> Result<Vec<Value>, DatabaseError> {
        let area_pattern = if matches!(area, "A" | "B") {
            format!("{area} %")
        } else {
            area.to_owned()
        };
        let side_column = if team == Some("team_b") {
            "rounds.team_b_side"
        } else {
            "rounds.team_a_side"
        };
        let sql = format!(
            "SELECT movement_samples.round_no, COUNT(*), MIN(movement_samples.timestamp_ms), MAX(movement_samples.timestamp_ms), {side_column} FROM movement_samples JOIN rounds ON rounds.match_id = movement_samples.match_id AND rounds.round_no = movement_samples.round_no WHERE movement_samples.match_id = ? AND movement_samples.player_id = ? AND movement_samples.area LIKE ? AND (? IS NULL OR {side_column} = ?) GROUP BY movement_samples.round_no, {side_column} ORDER BY movement_samples.round_no"
        );
        let rows = sqlx::query_as::<_, (i64, i64, i64, i64, Option<String>)>(&sql)
            .bind(match_id)
            .bind(player_id)
            .bind(area_pattern)
            .bind(side)
            .bind(side)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                json!({"round_no":row.0,"sample_count":row.1,"first_seen_ms":row.2,
                    "last_seen_ms":row.3,"side":row.4})
            })
            .collect())
    }

    async fn get_nearby_players(
        &self,
        match_id: &str,
        timestamp_ms: i64,
        x: f64,
        y: f64,
        z: f64,
        radius: f64,
    ) -> Result<Vec<Value>, DatabaseError> {
        let rows =
            sqlx::query_as::<_, (String, i64, f64, f64, f64, Option<String>, Option<String>)>(
                r#"SELECT movement_samples.player_id, movement_samples.timestamp_ms,
                      movement_samples.x, movement_samples.y, movement_samples.z,
                      players.team, players.agent_name
               FROM movement_samples JOIN players ON players.id = movement_samples.player_id
               WHERE movement_samples.match_id = ? AND movement_samples.player_id IS NOT NULL
                 AND movement_samples.timestamp_ms BETWEEN ? AND ?
                 AND ((movement_samples.x - ?) * (movement_samples.x - ?)
                    + (movement_samples.y - ?) * (movement_samples.y - ?)
                    + (movement_samples.z - ?) * (movement_samples.z - ?)) <= ?
               ORDER BY ABS(movement_samples.timestamp_ms - ?), movement_samples.player_id"#,
            )
            .bind(match_id)
            .bind(timestamp_ms - 500)
            .bind(timestamp_ms + 500)
            .bind(x)
            .bind(x)
            .bind(y)
            .bind(y)
            .bind(z)
            .bind(z)
            .bind(radius * radius)
            .bind(timestamp_ms)
            .fetch_all(&self.pool)
            .await?;
        let mut seen = BTreeSet::new();
        Ok(rows
            .into_iter()
            .filter(|row| seen.insert(row.0.clone()))
            .map(|row| {
                let distance =
                    ((row.2 - x).powi(2) + (row.3 - y).powi(2) + (row.4 - z).powi(2)).sqrt();
                json!({"player_id":row.0,"time_ms":row.1,"position":{"x":row.2,"y":row.3,"z":row.4},
                    "distance_units":distance,"team":row.5,"agent":row.6,
                    "evidence":{"match_id":match_id,"timestamp_ms":row.1,"player_id":row.0,
                        "evidence_type":"nearby_movement","source_file":"movement.ndjson",
                        "source_event_type":"movement_sample"}})
            })
            .collect())
    }

    async fn get_combat_events(
        &self,
        match_id: &str,
        player_id: &str,
        round_no: u32,
    ) -> Result<Vec<Value>, DatabaseError> {
        let rows = sqlx::query_as::<_, (i64,String,Option<String>,Option<String>,Option<f64>,i64,Option<String>,Option<String>,Option<String>,String,Option<f64>,Option<f64>,Option<f64>,Option<f64>,Option<f64>,Option<f64>)>(
            "SELECT timestamp_ms, kind, attacker_player_id, victim_player_id, damage, killed, weapon, hit_region, area, evidence_json, attacker_x, attacker_y, attacker_z, victim_x, victim_y, victim_z FROM combat_events WHERE match_id = ? AND round_no = ? AND (attacker_player_id = ? OR victim_player_id = ?) ORDER BY timestamp_ms",
        ).bind(match_id).bind(round_no).bind(player_id).bind(player_id).fetch_all(&self.pool).await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                json!({"time_ms":row.0,"kind":row.1,"attacker":row.2,"victim":row.3,
            "damage":row.4,"killed":row.5 != 0,"weapon":row.6,"hit_region":row.7,"area":row.8,
            "evidence":serde_json::from_str::<Value>(&row.9).unwrap_or_else(|_| json!([])),
            "attacker_position":{"x":row.10,"y":row.11,"z":row.12},
            "victim_position":{"x":row.13,"y":row.14,"z":row.15}})
            })
            .collect())
    }

    async fn get_ability_events(
        &self,
        match_id: &str,
        player_id: &str,
        round_no: u32,
    ) -> Result<Vec<Value>, DatabaseError> {
        let rows = sqlx::query_as::<_, (i64,Option<String>,Option<String>,String)>(
            "SELECT timestamp_ms, ability_name, area, evidence_json FROM ability_events WHERE match_id = ? AND round_no = ? AND player_id = ? ORDER BY timestamp_ms",
        ).bind(match_id).bind(round_no).bind(player_id).fetch_all(&self.pool).await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                json!({"time_ms":row.0,"ability":row.1,"area":row.2,
            "evidence":serde_json::from_str::<Value>(&row.3).unwrap_or_else(|_| json!([]))})
            })
            .collect())
    }

    async fn get_spike_events(
        &self,
        match_id: &str,
        round_no: u32,
    ) -> Result<Vec<Value>, DatabaseError> {
        let rows = sqlx::query_as::<_, (i64,String,Option<String>,Option<f64>,Option<f64>,Option<f64>,Option<String>,String)>(
            "SELECT timestamp_ms, kind, player_id, x, y, z, area, evidence_json FROM spike_events WHERE match_id = ? AND round_no = ? ORDER BY timestamp_ms",
        ).bind(match_id).bind(round_no).fetch_all(&self.pool).await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                json!({"time_ms":row.0,"kind":row.1,"player":row.2,
            "position":{"x":row.3,"y":row.4,"z":row.5},"area":row.6,
            "evidence":serde_json::from_str::<Value>(&row.7).unwrap_or_else(|_| json!([]))})
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
            r#"DELETE FROM valorant_accounts
               WHERE user_id = ? AND subject_id IN (
                   SELECT stable_player_id FROM players
                   WHERE match_id = ? AND stable_player_id IS NOT NULL
               )"#,
        )
        .bind(user_id)
        .bind(match_id)
        .execute(&self.pool)
        .await?;

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

    pub async fn unbind_player_from_account(
        &self,
        user_id: &str,
        match_id: &str,
        player_id: &str,
    ) -> Result<(), DatabaseError> {
        let result = sqlx::query(
            r#"DELETE FROM valorant_accounts
               WHERE user_id = ? AND subject_id = (
                   SELECT players.stable_player_id
                   FROM players JOIN matches ON matches.id = players.match_id
                   WHERE players.id = ? AND players.match_id = ? AND matches.user_id = ?
               )"#,
        )
        .bind(user_id)
        .bind(player_id)
        .bind(match_id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(DatabaseError::PlayerNotFound);
        }
        Ok(())
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

    /// Build a deterministic compact replay for the entire match.
    /// This is a 0-LLM-token compilation that summarizes each round into:
    /// - route segments (area transitions with timing)
    /// - combat bursts (merged shots + damage)
    /// - abilities, spike events, key timing
    ///
    /// The result is cached in compact_replays table.
    pub async fn build_compact_replay(
        &self,
        user_id: &str,
        match_id: &str,
    ) -> Result<Value, DatabaseError> {
        if let Some(cached) = self.get_cached_compact(match_id).await? {
            return Ok(cached);
        }
        let replay = self
            .find_match_for_user(user_id, match_id)
            .await?
            .ok_or(DatabaseError::MatchNotFound)?;
        let rounds = self.get_rounds(match_id).await?;
        let players = self
            .list_players_for_match_for_user(user_id, match_id)
            .await?;
        let bound_player = self.find_bound_player_for_match(user_id, match_id).await?;
        let diagnostics = self.semantic_diagnostics(match_id).await?;

        let mut compact_rounds = Vec::new();
        for round in &rounds {
            let round_no = round
                .get("round_no")
                .and_then(Value::as_u64)
                .unwrap_or(0) as u32;
            let round_start_ms = round.get("start_ms").and_then(Value::as_i64);

            let movement = if let Some(ref player_id) = bound_player {
                self.get_player_movement(match_id, player_id, round_no)
                    .await?
            } else {
                Vec::new()
            };
            let raw_combat = if let Some(ref player_id) = bound_player {
                self.get_combat_events(match_id, player_id, round_no)
                    .await?
            } else {
                Vec::new()
            };
            let combat = compact_combat_events(&raw_combat);
            let abilities = if let Some(ref player_id) = bound_player {
                self.get_ability_events(match_id, player_id, round_no)
                    .await?
            } else {
                Vec::new()
            };
            let spike = self.get_spike_events(match_id, round_no).await?;

            let route = compact_movement_segments(&movement, round_start_ms);
            let combat_summary = summarize_combat(&combat, round_no, round_start_ms);
            let ability_summary = summarize_abilities(&abilities, round_no, round_start_ms);
            let spike_summary = summarize_spike(&spike, round_no, round_start_ms);

            compact_rounds.push(json!({
                "round_no": round_no,
                "human_round": format!("R{}", round_no),
                "side": round.get("team_a_side"),
                "winner": round.get("winner_team"),
                "start_ms": round_start_ms,
                "end_ms": round.get("end_ms"),
                "route": route,
                "combat": combat_summary,
                "abilities": ability_summary,
                "spike": spike_summary,
            }));
        }

        let compact = json!({
            "match_id": match_id,
            "map": replay.metadata.map.as_deref().map(valcoach_domain::map_display_name),
            "map_raw": replay.metadata.map,
            "duration_ms": replay.metadata.duration_ms,
            "player_agent": players.iter()
                .find(|p| p.id == bound_player.as_deref().unwrap_or(""))
                .and_then(|p| p.agent_name.as_deref())
                .map(valcoach_domain::agent_display_name)
                .unwrap_or("unknown"),
            "rounds": compact_rounds,
            "diagnostics": diagnostics,
        });

        self.save_cached_compact(match_id, &compact).await?;
        Ok(compact)
    }

    async fn get_cached_compact(&self, match_id: &str) -> Result<Option<Value>, DatabaseError> {
        let value = sqlx::query_scalar::<_, String>(
            "SELECT compact_json FROM compact_replays WHERE match_id = ?",
        )
        .bind(match_id)
        .fetch_optional(&self.pool)
        .await?;
        value
            .map(|json| serde_json::from_str(&json))
            .transpose()
            .map_err(Into::into)
    }

    async fn save_cached_compact(
        &self,
        match_id: &str,
        compact: &Value,
    ) -> Result<(), DatabaseError> {
        sqlx::query(
            "INSERT INTO compact_replays (match_id, compact_json) VALUES (?, ?) \
             ON CONFLICT(match_id) DO UPDATE SET compact_json = excluded.compact_json, created_at = CURRENT_TIMESTAMP",
        )
        .bind(match_id)
        .bind(serde_json::to_string(compact)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Retrieve the player's personal issues for coaching context.
    pub async fn list_player_issues(
        &self,
        user_id: &str,
    ) -> Result<Vec<Value>, DatabaseError> {
        let rows = sqlx::query_as::<_, (String, String, String, String, Option<String>, Option<String>, Option<String>, f64, f64, String, i64, Option<String>, Option<i64>, Option<i64>)>(
            "SELECT issue_key, category, title, description, map_name, side, area, severity, confidence, status, occurrences, last_match_id, last_round_no, last_timestamp_ms FROM player_issues WHERE user_id = ? AND status != 'resolved' ORDER BY occurrences DESC, severity DESC LIMIT 10",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| json!({
            "issue_key": r.0, "category": r.1, "title": r.2, "description": r.3,
            "map": r.4, "side": r.5, "area": r.6,
            "severity": r.7, "confidence": r.8, "status": r.9, "occurrences": r.10,
            "last_match_id": r.11, "last_round_no": r.12, "last_timestamp_ms": r.13,
        })).collect())
    }

    /// Record or update a player issue from coaching feedback.
    #[allow(clippy::too_many_arguments)]
    pub async fn upsert_player_issue(
        &self,
        user_id: &str,
        issue_key: &str,
        category: &str,
        title: &str,
        description: Option<&str>,
        map_name: Option<&str>,
        side: Option<&str>,
        area: Option<&str>,
        severity: f64,
        confidence: f64,
        match_id: Option<&str>,
        round_no: Option<i64>,
        timestamp_ms: Option<i64>,
    ) -> Result<(), DatabaseError> {
        sqlx::query(
            r#"INSERT INTO player_issues (id, user_id, issue_key, category, title, description, map_name, side, area, severity, confidence, last_match_id, last_round_no, last_timestamp_ms)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(user_id, issue_key) DO UPDATE SET
                category = excluded.category,
                title = excluded.title,
                description = excluded.description,
                severity = (player_issues.severity * 0.6 + excluded.severity * 0.4),
                confidence = excluded.confidence,
                status = 'active',
                occurrences = player_issues.occurrences + 1,
                last_match_id = excluded.last_match_id,
                last_round_no = excluded.last_round_no,
                last_timestamp_ms = excluded.last_timestamp_ms,
                updated_at = CURRENT_TIMESTAMP"#,
        )
        .bind(format!("{user_id}:{issue_key}"))
        .bind(user_id)
        .bind(issue_key)
        .bind(category)
        .bind(title)
        .bind(description)
        .bind(map_name)
        .bind(side)
        .bind(area)
        .bind(severity)
        .bind(confidence)
        .bind(match_id)
        .bind(round_no)
        .bind(timestamp_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn agent_usage_for_user(
        &self,
        user_id: &str,
    ) -> Result<AgentUsageSummary, DatabaseError> {        let (input, output, total, cost, priced) = sqlx::query_as::<_, (i64, i64, i64, i64, i64)>(
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
    pawn_agents: HashMap<u64, String>,
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
    player_state_net_guid: u64,
    character_net_guids: Vec<u64>,
}

#[derive(Debug, Default)]
struct FinalizedRoster {
    players: Vec<ReplayPlayerIdentity>,
    pawn_to_player: HashMap<u64, String>,
    state_to_player: HashMap<u64, String>,
}

impl ReplayRoster {
    fn observe(&mut self, event: &valcoach_domain::GenericEvent) {
        if event.event_type == "actor_spawned" {
            if let (Some(actor_guid), Some(name)) = (
                event.actor_net_guid,
                event
                    .raw
                    .get("replication_class_path")
                    .and_then(serde_json::Value::as_str)
                    .and_then(agent_codename),
            ) {
                self.pawn_agents.insert(actor_guid, name);
            }
            return;
        }
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
                agent_name: draft.agent_name.clone().or_else(|| {
                    draft
                        .pawn_guids
                        .iter()
                        .find_map(|guid| self.pawn_agents.get(guid).cloned())
                }),
                player_slot: slot,
                player_state_net_guid: state_guid,
                character_net_guids: draft.pawn_guids.iter().copied().collect(),
            });
            finalized.state_to_player.insert(state_guid, id.clone());
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

/// Compact movement waypoints into route segments.
/// Groups consecutive samples by area, producing a route like:
/// [{ "from": "A Tower", "to": "A Site", "start": "00:04.2", "end": "00:10.8" }, ...]
fn compact_movement_segments(movement: &[Value], round_start_ms: Option<i64>) -> Vec<Value> {
    if movement.is_empty() {
        return Vec::new();
    }
    let mut segments: Vec<Value> = Vec::new();
    let mut current_area: Option<String> = None;
    let mut segment_start_ms: i64 = 0;
    let mut segment_positions: Vec<Value> = Vec::new();
    let mut has_alive = true;

    for sample in movement {
        let time_ms = sample.get("time_ms").and_then(Value::as_i64).unwrap_or(0);
        let area = sample.get("area").and_then(Value::as_str).map(str::to_owned);
        let alive = sample.get("alive").and_then(Value::as_bool).unwrap_or(true);

        if area != current_area || (alive != has_alive && !alive) {
            if let Some(ref prev_area) = current_area {
                let end_area = area.as_deref().unwrap_or("unknown");
                let human_start = valcoach_domain::humanize::humanize_time(
                    segment_start_ms,
                    None,
                    round_start_ms,
                );
                let human_end = valcoach_domain::humanize::humanize_time(
                    time_ms,
                    None,
                    round_start_ms,
                );
                segments.push(json!({
                    "from": prev_area,
                    "to": end_area,
                    "start": human_start,
                    "end": human_end,
                    "start_ms": segment_start_ms,
                    "end_ms": time_ms,
                    "alive": has_alive,
                    "waypoints": segment_positions.len(),
                }));
            }
            current_area = area;
            segment_start_ms = time_ms;
            segment_positions.clear();
            has_alive = alive;
        }
        if let Some(pos) = sample.get("position") {
            segment_positions.push(pos.clone());
        }
    }

    if let Some(ref last_area) = current_area {
        let last_time = movement
            .last()
            .and_then(|s| s.get("time_ms").and_then(Value::as_i64))
            .unwrap_or(0);
        let human_start = valcoach_domain::humanize::humanize_time(
            segment_start_ms,
            None,
            round_start_ms,
        );
        let human_end = valcoach_domain::humanize::humanize_time(
            last_time,
            None,
            round_start_ms,
        );
        segments.push(json!({
            "area": last_area,
            "start": human_start,
            "end": human_end,
            "start_ms": segment_start_ms,
            "end_ms": last_time,
            "alive": has_alive,
            "waypoints": segment_positions.len(),
        }));
    }
    segments
}

/// Summarize compacted combat events for a round.
fn summarize_combat(combat: &[Value], round_no: u32, round_start_ms: Option<i64>) -> Value {
    let total_shots: usize = combat
        .iter()
        .filter_map(|e| e.get("shots").and_then(Value::as_u64).map(|v| v as usize))
        .sum();
    let total_damage: f64 = combat
        .iter()
        .filter_map(|e| e.get("damage").and_then(Value::as_f64))
        .sum();
    let kills = combat
        .iter()
        .filter(|e| e.get("killed").and_then(Value::as_bool).unwrap_or(false))
        .count();
    let deaths = combat
        .iter()
        .filter(|e| {
            e.get("kind").and_then(Value::as_str) == Some("damage")
                && e.get("killed").and_then(Value::as_bool).unwrap_or(false)
                && e.get("victim").is_some()
        })
        .count();

    let events: Vec<Value> = combat
        .iter()
        .map(|e| {
            let time_ms = e.get("time_ms").and_then(Value::as_i64).unwrap_or(0);
            let human_time = valcoach_domain::humanize::humanize_time(time_ms, Some(round_no), round_start_ms);
            let mut summary = json!({
                "time": human_time,
                "kind": e.get("kind"),
                "weapon": e.get("weapon"),
                "area": e.get("area"),
            });
            if let Some(shots) = e.get("shots") {
                summary["shots"] = shots.clone();
            }
            if let Some(damage) = e.get("damage") {
                summary["damage"] = damage.clone();
            }
            if e.get("killed").and_then(Value::as_bool).unwrap_or(false) {
                summary["result"] = json!("kill");
            }
            if let Some(regions) = e.get("hit_regions") {
                summary["hit_regions"] = regions.clone();
            }
            summary
        })
        .collect();

    json!({
        "events": events,
        "totals": {
            "shots": total_shots,
            "damage": total_damage,
            "kills": kills,
            "deaths": deaths,
        }
    })
}

/// Summarize ability events for a round.
fn summarize_abilities(abilities: &[Value], round_no: u32, round_start_ms: Option<i64>) -> Vec<Value> {
    abilities
        .iter()
        .map(|e| {
            let time_ms = e.get("time_ms").and_then(Value::as_i64).unwrap_or(0);
            let human_time = valcoach_domain::humanize::humanize_time(time_ms, Some(round_no), round_start_ms);
            json!({
                "time": human_time,
                "ability": e.get("ability"),
                "area": e.get("area"),
            })
        })
        .collect()
}

/// Summarize spike events for a round.
fn summarize_spike(spike: &[Value], round_no: u32, round_start_ms: Option<i64>) -> Vec<Value> {
    spike
        .iter()
        .map(|e| {
            let time_ms = e.get("time_ms").and_then(Value::as_i64).unwrap_or(0);
            let human_time = valcoach_domain::humanize::humanize_time(time_ms, Some(round_no), round_start_ms);
            json!({
                "time": human_time,
                "kind": e.get("kind"),
                "area": e.get("area"),
            })
        })
        .collect()
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
    round_no: Option<u32>,
    yaw: Option<f64>,
    pitch: Option<f64>,
    alive: Option<bool>,
    area: Option<String>,
    source_row: u64,
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
        "INSERT INTO movement_samples (match_id, player_id, timestamp_ms, x, y, z, velocity_x, velocity_y, velocity_z, round_no, yaw, pitch, alive, area, source_file, source_row) ",
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
            .push_bind(sample.velocity_z)
            .push_bind(sample.round_no)
            .push_bind(sample.yaw)
            .push_bind(sample.pitch)
            .push_bind(sample.alive)
            .push_bind(sample.area)
            .push_bind("movement.ndjson")
            .push_bind(sample.source_row as i64);
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
        "INSERT INTO players (id, match_id, stable_player_id, display_name, team, agent_name, player_slot, player_state_net_guid, character_net_guids_json) ",
    );
    query.push_values(players, |mut row, player| {
        row.push_bind(player.id)
            .push_bind(match_id)
            .push_bind(player.stable_player_id)
            .push_bind(Option::<String>::None)
            .push_bind(player.team)
            .push_bind(player.agent_name)
            .push_bind(player.player_slot)
            .push_bind(player.player_state_net_guid.to_string())
            .push_bind(
                serde_json::to_string(&player.character_net_guids).expect("GUID list serializes"),
            );
    });
    query.build().execute(&mut **transaction).await?;
    Ok(())
}

async fn insert_semantic_replay(
    transaction: &mut Transaction<'_, Sqlite>,
    match_id: &str,
    semantic: &SemanticBuilder,
) -> Result<(), DatabaseError> {
    for round in &semantic.rounds {
        sqlx::query(
            r#"INSERT INTO rounds
               (id, match_id, round_no, start_ms, buy_end_ms, end_ms, team_a_side, team_b_side,
                winner_team, evidence_json)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(format!("{match_id}:round:{}", round.round_no))
        .bind(match_id)
        .bind(round.round_no)
        .bind(round.start_ms)
        .bind(round.buy_end_ms)
        .bind(round.end_ms)
        .bind(&round.team_a_side)
        .bind(&round.team_b_side)
        .bind(&round.winner_team)
        .bind(serde_json::to_string(&round.evidence)?)
        .execute(&mut **transaction)
        .await?;
    }
    for event in &semantic.combat {
        let attacker = event.attacker_position.as_ref();
        let victim = event.victim_position.as_ref();
        sqlx::query(
            r#"INSERT INTO combat_events
               (match_id, round_no, timestamp_ms, kind, attacker_player_id, victim_player_id,
                damage, killed, weapon, hit_region, attacker_x, attacker_y, attacker_z,
                victim_x, victim_y, victim_z, area, evidence_json)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(match_id)
        .bind(event.round_no)
        .bind(event.timestamp_ms)
        .bind(&event.kind)
        .bind(&event.attacker_player_id)
        .bind(&event.victim_player_id)
        .bind(event.damage)
        .bind(event.killed)
        .bind(&event.weapon)
        .bind(&event.hit_region)
        .bind(attacker.map(|point| point.x))
        .bind(attacker.map(|point| point.y))
        .bind(attacker.map(|point| point.z))
        .bind(victim.map(|point| point.x))
        .bind(victim.map(|point| point.y))
        .bind(victim.map(|point| point.z))
        .bind(&event.area)
        .bind(serde_json::to_string(&event.evidence)?)
        .execute(&mut **transaction)
        .await?;
        if event.kind == "shot" {
            sqlx::query(
                "INSERT INTO shots (id, match_id, player_id, timestamp_ms, payload_json) VALUES (?, ?, ?, ?, ?)",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(match_id)
            .bind(&event.attacker_player_id)
            .bind(event.timestamp_ms)
            .bind(serde_json::to_string(event)?)
            .execute(&mut **transaction)
            .await?;
        }
    }
    for event in &semantic.spike {
        let position = event.position.as_ref();
        sqlx::query(
            r#"INSERT INTO spike_events
               (match_id, round_no, timestamp_ms, kind, player_id, x, y, z, area, evidence_json)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(match_id)
        .bind(event.round_no)
        .bind(event.timestamp_ms)
        .bind(&event.kind)
        .bind(&event.player_id)
        .bind(position.map(|point| point.x))
        .bind(position.map(|point| point.y))
        .bind(position.map(|point| point.z))
        .bind(&event.area)
        .bind(serde_json::to_string(&event.evidence)?)
        .execute(&mut **transaction)
        .await?;
    }
    for event in &semantic.abilities {
        sqlx::query(
            r#"INSERT INTO ability_events
               (id, match_id, player_id, timestamp_ms, payload_json, round_no, ability_name, area, evidence_json)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(match_id)
        .bind(&event.player_id)
        .bind(event.timestamp_ms)
        .bind(serde_json::to_string(event)?)
        .bind(event.round_no)
        .bind(&event.ability_name)
        .bind(&event.area)
        .bind(serde_json::to_string(&event.evidence)?)
        .execute(&mut **transaction)
        .await?;
    }
    sqlx::query("INSERT INTO semantic_diagnostics (match_id, value_json) VALUES (?, ?)")
        .bind(match_id)
        .bind(serde_json::to_string(&semantic.diagnostics_json())?)
        .execute(&mut **transaction)
        .await?;
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
    #[error("failed to read semantic replay evidence: {0}")]
    Io(#[from] std::io::Error),
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
                server_events_path: None,
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
