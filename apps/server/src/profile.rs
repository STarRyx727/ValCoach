use std::collections::HashSet;

use axum::{Json, extract::State};
use serde::Deserialize;
use valcoach_db::{MatchTrendRecord, UserProfileRecord};

use crate::{
    AppState,
    auth::{AuthApiError, require_user_id},
};

const ROLES: [&str; 5] = ["Duelist", "Initiator", "Controller", "Sentinel", "Flex"];

#[derive(Debug, Deserialize)]
pub struct UpdateProfileRequest {
    pub rank_name: Option<String>,
    pub main_role: Option<String>,
    #[serde(default)]
    pub main_agents: Vec<String>,
    #[serde(default)]
    pub training_goals: Vec<String>,
    #[serde(default)]
    pub goal_notes: String,
}

pub async fn get_profile(
    State(state): State<AppState>,
    session: tower_sessions::Session,
) -> Result<Json<UserProfileRecord>, AuthApiError> {
    let user_id = require_user_id(&state.auth, &session).await?;
    state
        .auth
        .database
        .user_profile(&user_id)
        .await
        .map(Json)
        .map_err(|error| AuthApiError::internal(error.to_string()))
}

pub async fn update_profile(
    State(state): State<AppState>,
    session: tower_sessions::Session,
    Json(request): Json<UpdateProfileRequest>,
) -> Result<Json<UserProfileRecord>, AuthApiError> {
    let user_id = require_user_id(&state.auth, &session).await?;
    let rank_name = clean_optional(request.rank_name, 80, "rank_name")?;
    let main_role = clean_optional(request.main_role, 40, "main_role")?;
    if let Some(role) = &main_role
        && !ROLES.contains(&role.as_str())
    {
        return Err(AuthApiError::bad_request("unsupported main_role"));
    }
    let main_agents = clean_list(request.main_agents, 5, 50, "main_agents")?;
    if main_agents
        .iter()
        .any(|agent| !valcoach_domain::is_official_agent_display_name(agent))
    {
        return Err(AuthApiError::bad_request(
            "main_agents must use official English agent names",
        ));
    }
    let training_goals = clean_list(request.training_goals, 8, 100, "training_goals")?;
    let goal_notes = request.goal_notes.trim().to_owned();
    if goal_notes.chars().count() > 1_000 {
        return Err(AuthApiError::bad_request(
            "goal_notes must be at most 1000 characters",
        ));
    }
    let profile = UserProfileRecord {
        rank_name,
        main_role,
        main_agents,
        training_goals,
        goal_notes,
        updated_at: None,
    };
    state
        .auth
        .database
        .upsert_user_profile(&user_id, &profile)
        .await
        .map(Json)
        .map_err(|error| AuthApiError::internal(error.to_string()))
}

pub async fn trends(
    State(state): State<AppState>,
    session: tower_sessions::Session,
) -> Result<Json<Vec<MatchTrendRecord>>, AuthApiError> {
    let user_id = require_user_id(&state.auth, &session).await?;
    state
        .auth
        .database
        .match_trends_for_user(&user_id)
        .await
        .map(Json)
        .map_err(|error| AuthApiError::internal(error.to_string()))
}

fn clean_optional(
    value: Option<String>,
    maximum: usize,
    field: &str,
) -> Result<Option<String>, AuthApiError> {
    let Some(value) = value else { return Ok(None) };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.chars().count() > maximum {
        return Err(AuthApiError::bad_request(format!(
            "{field} must be at most {maximum} characters"
        )));
    }
    Ok(Some(value.to_owned()))
}

fn clean_list(
    values: Vec<String>,
    maximum_items: usize,
    maximum_length: usize,
    field: &str,
) -> Result<Vec<String>, AuthApiError> {
    if values.len() > maximum_items {
        return Err(AuthApiError::bad_request(format!(
            "{field} must contain at most {maximum_items} items"
        )));
    }
    let mut seen = HashSet::new();
    let mut clean = Vec::new();
    for value in values {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        if value.chars().count() > maximum_length {
            return Err(AuthApiError::bad_request(format!(
                "each {field} item must be at most {maximum_length} characters"
            )));
        }
        if seen.insert(value.to_lowercase()) {
            clean.push(value.to_owned());
        }
    }
    Ok(clean)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_lists_are_trimmed_and_deduplicated() {
        let values = clean_list(
            vec!["Viper".to_owned(), " viper ".to_owned(), "Omen".to_owned()],
            5,
            50,
            "agents",
        )
        .expect("valid list");
        assert_eq!(values, vec!["Viper", "Omen"]);
    }
}
