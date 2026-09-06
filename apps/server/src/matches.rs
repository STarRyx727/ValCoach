use axum::{
    Json,
    extract::{Path, State},
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
