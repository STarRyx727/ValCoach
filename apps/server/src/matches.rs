use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use valcoach_db::{MatchMetricRecord, MatchRecord, PlayerRecord, ValorantAccountRecord};

use crate::{
    AppState,
    auth::{AuthApiError, require_user_id},
};

#[derive(Debug, Serialize)]
pub struct MatchDetail {
    #[serde(flatten)]
    pub replay: MatchRecord,
    pub players: Vec<PlayerRecord>,
    pub metrics: Vec<MetricView>,
}

#[derive(Debug, Serialize)]
pub struct MetricView {
    pub id: String,
    pub metric_name: String,
    pub value: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct BindPlayerRequest {
    pub player_id: String,
}

pub async fn list_matches(
    State(state): State<AppState>,
    session: tower_sessions::Session,
) -> Result<Json<Vec<MatchRecord>>, AuthApiError> {
    let user_id = require_user_id(&state.auth, &session).await?;
    state
        .auth
        .database
        .list_matches_for_user(&user_id)
        .await
        .map(Json)
        .map_err(|error| AuthApiError::internal(error.to_string()))
}

pub async fn get_match(
    State(state): State<AppState>,
    session: tower_sessions::Session,
    Path(match_id): Path<String>,
) -> Result<Json<MatchDetail>, AuthApiError> {
    let user_id = require_user_id(&state.auth, &session).await?;
    let replay = state
        .auth
        .database
        .find_match_for_user(&user_id, &match_id)
        .await
        .map_err(|error| AuthApiError::internal(error.to_string()))?
        .ok_or_else(AuthApiError::unauthorized)?;
    let players = state
        .auth
        .database
        .list_players_for_match_for_user(&user_id, &match_id)
        .await
        .map_err(|error| AuthApiError::internal(error.to_string()))?;
    let metrics = state
        .auth
        .database
        .list_match_metrics_for_user(&user_id, &match_id)
        .await
        .map_err(|error| AuthApiError::internal(error.to_string()))?
        .into_iter()
        .map(metric_view)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(MatchDetail {
        replay,
        players,
        metrics,
    }))
}

pub async fn bind_player(
    State(state): State<AppState>,
    session: tower_sessions::Session,
    Path(match_id): Path<String>,
    Json(request): Json<BindPlayerRequest>,
) -> Result<Json<ValorantAccountRecord>, AuthApiError> {
    let user_id = require_user_id(&state.auth, &session).await?;
    if request.player_id.trim().is_empty() {
        return Err(AuthApiError::bad_request("player_id is required"));
    }
    state
        .auth
        .database
        .bind_player_to_account(&user_id, &match_id, &request.player_id)
        .await
        .map(Json)
        .map_err(|error| match error {
            valcoach_db::DatabaseError::PlayerNotFound => AuthApiError::unauthorized(),
            other => AuthApiError::internal(other.to_string()),
        })
}

pub async fn unbind_player(
    State(state): State<AppState>,
    session: tower_sessions::Session,
    Path(match_id): Path<String>,
    Json(request): Json<BindPlayerRequest>,
) -> Result<axum::http::StatusCode, AuthApiError> {
    let user_id = require_user_id(&state.auth, &session).await?;
    state
        .auth
        .database
        .unbind_player_from_account(&user_id, &match_id, &request.player_id)
        .await
        .map_err(|error| match error {
            valcoach_db::DatabaseError::PlayerNotFound => AuthApiError::unauthorized(),
            other => AuthApiError::internal(other.to_string()),
        })?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub async fn delete_match(
    State(state): State<AppState>,
    session: tower_sessions::Session,
    Path(match_id): Path<String>,
) -> Result<StatusCode, AuthApiError> {
    let user_id = require_user_id(&state.auth, &session).await?;
    let job_ids = state
        .auth
        .database
        .delete_match_for_user(&user_id, &match_id)
        .await
        .map_err(|error| match error {
            valcoach_db::DatabaseError::MatchNotFound => AuthApiError::unauthorized(),
            other => AuthApiError::internal(other.to_string()),
        })?;
    for job_id in &job_ids {
        let replay_path = state
            .jobs
            .data_directory
            .join("replays")
            .join(&user_id)
            .join(format!("{job_id}.vrf"));
        let _ = tokio::fs::remove_file(&replay_path).await;
        let job_dir = state.jobs.data_directory.join("jobs").join(job_id);
        let _ = tokio::fs::remove_dir_all(&job_dir).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn get_compact_replay(
    State(state): State<AppState>,
    session: tower_sessions::Session,
    Path(match_id): Path<String>,
) -> Result<Json<serde_json::Value>, AuthApiError> {
    let user_id = require_user_id(&state.auth, &session).await?;
    state
        .auth
        .database
        .build_compact_replay(&user_id, &match_id)
        .await
        .map(Json)
        .map_err(|error| match error {
            valcoach_db::DatabaseError::MatchNotFound => AuthApiError::unauthorized(),
            other => AuthApiError::internal(other.to_string()),
        })
}

pub async fn list_maps(
    State(_state): State<AppState>,
    _session: tower_sessions::Session,
) -> Result<Json<Vec<serde_json::Value>>, AuthApiError> {
    let maps_dir = std::path::Path::new("data").join("maps");
    let mut maps = Vec::new();
    if let Ok(mut entries) = tokio::fs::read_dir(&maps_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            if entry.path().extension().is_some_and(|ext| ext == "json")
                && let Ok(bytes) = tokio::fs::read(entry.path()).await
                && let Ok(meta) = serde_json::from_slice::<serde_json::Value>(&bytes)
            {
                maps.push(meta);
            }
        }
    }
    maps.sort_by(|a, b| {
        a.get("display_name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .cmp(b.get("display_name").and_then(serde_json::Value::as_str).unwrap_or(""))
    });
    Ok(Json(maps))
}

fn metric_view(metric: MatchMetricRecord) -> Result<MetricView, AuthApiError> {
    let value = serde_json::from_str(&metric.value_json).map_err(|error| {
        AuthApiError::internal(format!("stored metric is invalid JSON: {error}"))
    })?;
    Ok(MetricView {
        id: metric.id,
        metric_name: metric.metric_name,
        value,
    })
}
