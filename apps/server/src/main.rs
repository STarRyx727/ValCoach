mod agent;
mod auth;
mod jobs;
mod matches;

use std::net::SocketAddr;

use axum::{
    Router,
    extract::{DefaultBodyLimit, FromRef},
    routing::{get, post},
};
use tower_sessions::{MemoryStore, SessionManagerLayer, cookie::SameSite};
use valcoach_db::Database;

use crate::{auth::AuthState, jobs::JobManager};

#[derive(Clone, Debug)]
struct AppState {
    agent: agent::AgentService,
    auth: AuthState,
    jobs: JobManager,
}

impl FromRef<AppState> for AuthState {
    fn from_ref(state: &AppState) -> Self {
        state.auth.clone()
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let database_url = std::env::var("VALCOACH_DATABASE_URL")
        .unwrap_or_else(|_| "sqlite://data/valcoach.db".to_owned());
    std::fs::create_dir_all("data")?;
    let database = Database::connect(&database_url).await?;
    let parser_directory = std::env::var("VALCOACH_PARSER_DIR")
        .unwrap_or_else(|_| ".external/ValorantReplayParser".to_owned());
    let dotnet_path = std::env::var("VALCOACH_DOTNET_PATH")
        .unwrap_or_else(|_| "C:\\Program Files\\dotnet\\dotnet.exe".to_owned());
    let agent = agent::AgentService::from_env(database.clone())?;
    let app = app(AppState {
        agent,
        auth: AuthState {
            database: database.clone(),
        },
        jobs: JobManager::new(database, parser_directory, dotnet_path, "data"),
    });

    let address: SocketAddr = "127.0.0.1:3000".parse()?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(%address, "ValCoach server listening");
    axum::serve(listener, app).await?;
    Ok(())
}

fn app(state: AppState) -> Router {
    Router::new()
        .route("/api/auth/register", post(auth::register))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/auth/me", get(auth::me))
        .route("/api/replays", post(jobs::upload_replay))
        .route("/api/matches", get(matches::list_matches))
        .route("/api/matches/{id}", get(matches::get_match))
        .route("/api/matches/{id}/coach", post(agent::coach_match))
        .route("/api/matches/{id}/coaching", get(agent::history))
        .route("/api/matches/{id}/bind-player", post(matches::bind_player))
        .route("/api/agent/status", get(agent::status))
        .route("/api/agent/usage", get(agent::usage))
        .route("/api/jobs/{id}", get(jobs::get_job))
        .route("/api/jobs/{id}/bundle", get(jobs::get_job_bundle))
        .route("/api/jobs/{id}/events", get(jobs::job_events))
        .route("/api/jobs/{id}/cancel", post(jobs::cancel_job))
        .layer(DefaultBodyLimit::max(100 * 1024 * 1024))
        .with_state(state)
        .layer(
            SessionManagerLayer::new(MemoryStore::default())
                .with_name("valcoach.sid")
                .with_http_only(true)
                .with_same_site(SameSite::Lax)
                // Local MVP serves plain HTTP. HTTPS deployments must set this to true.
                .with_secure(false),
        )
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode, header},
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    use valcoach_db::Database;

    use crate::{AppState, app, auth::AuthState, jobs::JobManager};

    #[tokio::test]
    async fn auth_routes_register_read_session_and_logout() {
        let database = Database::connect("sqlite::memory:")
            .await
            .expect("database");
        let application = app(AppState {
            agent: crate::agent::AgentService::disabled(database.clone()),
            auth: AuthState {
                database: database.clone(),
            },
            jobs: JobManager::new(database, "parser", "dotnet", "data"),
        });
        let register = Request::builder()
            .method("POST")
            .uri("/api/auth/register")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                r#"{"username":"smoke_user","password":"SmokeTestPassword_2026"}"#,
            ))
            .expect("register request");

        let response = application
            .clone()
            .oneshot(register)
            .await
            .expect("register response");
        assert_eq!(response.status(), StatusCode::CREATED);
        let set_cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .expect("session cookie")
            .to_str()
            .expect("cookie text");
        assert!(set_cookie.contains("HttpOnly"));
        assert!(set_cookie.contains("SameSite=Lax"));
        let cookie = set_cookie.split(';').next().expect("cookie value");

        let current = Request::builder()
            .uri("/api/auth/me")
            .header(header::COOKIE, cookie)
            .body(Body::empty())
            .expect("current-user request");
        let response = application
            .clone()
            .oneshot(current)
            .await
            .expect("current-user response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("response body")
            .to_bytes();
        assert!(
            std::str::from_utf8(&body)
                .expect("JSON body")
                .contains("smoke_user")
        );

        let logout = Request::builder()
            .method("POST")
            .uri("/api/auth/logout")
            .header(header::COOKIE, cookie)
            .body(Body::empty())
            .expect("logout request");
        assert_eq!(
            application
                .clone()
                .oneshot(logout)
                .await
                .expect("logout response")
                .status(),
            StatusCode::NO_CONTENT
        );

        let after_logout = Request::builder()
            .uri("/api/auth/me")
            .header(header::COOKIE, cookie)
            .body(Body::empty())
            .expect("post-logout request");
        assert_eq!(
            application
                .oneshot(after_logout)
                .await
                .expect("post-logout response")
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
}
