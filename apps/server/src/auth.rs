use argon2::{
    Algorithm, Argon2, Params, Version,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tower_sessions::Session;
use uuid::Uuid;
use valcoach_db::{Database, DatabaseError, UserRecord};

const USER_ID_SESSION_KEY: &str = "user_id";

#[derive(Clone, Debug)]
pub struct AuthState {
    pub database: Database,
}

#[derive(Debug, Deserialize)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PublicUser {
    pub id: String,
    pub username: String,
}

impl From<&UserRecord> for PublicUser {
    fn from(user: &UserRecord) -> Self {
        Self {
            id: user.id.clone(),
            username: user.username.clone(),
        }
    }
}

pub async fn register(
    State(state): State<AuthState>,
    session: Session,
    Json(credentials): Json<Credentials>,
) -> Result<(StatusCode, Json<PublicUser>), AuthApiError> {
    validate_credentials(&credentials)?;
    let password_hash = hash_password(&credentials.password)?;
    let user = UserRecord {
        id: Uuid::new_v4().to_string(),
        username: credentials.username.trim().to_owned(),
        password_hash,
    };
    state
        .database
        .create_user(&user)
        .await
        .map_err(AuthApiError::from)?;
    session
        .insert(USER_ID_SESSION_KEY, user.id.clone())
        .await
        .map_err(AuthApiError::session)?;

    Ok((StatusCode::CREATED, Json(PublicUser::from(&user))))
}

pub async fn login(
    State(state): State<AuthState>,
    session: Session,
    Json(credentials): Json<Credentials>,
) -> Result<Json<PublicUser>, AuthApiError> {
    let username = credentials.username.trim();
    let Some(user) = state
        .database
        .find_user_by_username(username)
        .await
        .map_err(AuthApiError::from)?
    else {
        return Err(AuthApiError::unauthorized());
    };
    verify_password(&credentials.password, &user.password_hash)
        .map_err(|_| AuthApiError::unauthorized())?;
    session
        .insert(USER_ID_SESSION_KEY, user.id.clone())
        .await
        .map_err(AuthApiError::session)?;

    Ok(Json(PublicUser::from(&user)))
}

pub async fn logout(session: Session) -> Result<StatusCode, AuthApiError> {
    session.delete().await.map_err(AuthApiError::session)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn me(
    State(state): State<AuthState>,
    session: Session,
) -> Result<Json<PublicUser>, AuthApiError> {
    let user_id = require_user_id(&state, &session).await?;
    let Some(user) = state
        .database
        .find_user_by_id(&user_id)
        .await
        .map_err(AuthApiError::from)?
    else {
        return Err(AuthApiError::unauthorized());
    };

    Ok(Json(PublicUser::from(&user)))
}

pub(crate) async fn require_user_id(
    state: &AuthState,
    session: &Session,
) -> Result<String, AuthApiError> {
    let Some(user_id) = session
        .get::<String>(USER_ID_SESSION_KEY)
        .await
        .map_err(AuthApiError::session)?
    else {
        return Err(AuthApiError::unauthorized());
    };
    if state
        .database
        .find_user_by_id(&user_id)
        .await
        .map_err(AuthApiError::from)?
        .is_none()
    {
        return Err(AuthApiError::unauthorized());
    }
    Ok(user_id)
}

fn validate_credentials(credentials: &Credentials) -> Result<(), AuthApiError> {
    let username = credentials.username.trim();
    if !(3..=32).contains(&username.len())
        || !username
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        return Err(AuthApiError::bad_request(
            "username must be 3-32 ASCII letters, digits, '_' or '-'",
        ));
    }
    if credentials.password.len() < 8 {
        return Err(AuthApiError::bad_request(
            "password must contain at least 8 characters",
        ));
    }
    Ok(())
}

fn hash_password(password: &str) -> Result<String, AuthApiError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, Params::default())
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| AuthApiError::internal(error.to_string()))
}

fn verify_password(password: &str, password_hash: &str) -> Result<(), PasswordVerificationError> {
    let parsed = PasswordHash::new(password_hash).map_err(|_| PasswordVerificationError)?;
    Argon2::new(Algorithm::Argon2id, Version::V0x13, Params::default())
        .verify_password(password.as_bytes(), &parsed)
        .map_err(|_| PasswordVerificationError)
}

#[derive(Debug, Error)]
#[error("password verification failed")]
struct PasswordVerificationError;

#[derive(Debug)]
pub struct AuthApiError {
    status: StatusCode,
    message: String,
}

impl AuthApiError {
    pub(crate) fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    pub(crate) fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: "invalid credentials or missing session".to_owned(),
        }
    }

    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }

    pub(crate) fn session(error: impl std::fmt::Display) -> Self {
        tracing::error!(error = %error, "session operation failed");
        Self::internal("session operation failed")
    }
}

impl From<DatabaseError> for AuthApiError {
    fn from(error: DatabaseError) -> Self {
        match error {
            DatabaseError::UsernameAlreadyExists => {
                Self::bad_request("username is already registered")
            }
            other => {
                tracing::error!(error = %other, "database operation failed during authentication");
                Self::internal("authentication storage operation failed")
            }
        }
    }
}

impl IntoResponse for AuthApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({ "error": self.message })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::{Credentials, hash_password, validate_credentials, verify_password};

    #[test]
    fn argon2id_hashes_are_verified_and_wrong_passwords_fail() {
        let hash = hash_password("correct horse battery staple").expect("hash");
        assert!(verify_password("correct horse battery staple", &hash).is_ok());
        assert!(verify_password("wrong password", &hash).is_err());
    }

    #[test]
    fn credential_validation_rejects_unsafe_or_short_input() {
        assert!(
            validate_credentials(&Credentials {
                username: "a".to_owned(),
                password: "12345678".to_owned()
            })
            .is_err()
        );
        assert!(
            validate_credentials(&Credentials {
                username: "valid_user".to_owned(),
                password: "12345678".to_owned()
            })
            .is_ok()
        );
    }
}
