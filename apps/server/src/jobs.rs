use std::{
    collections::HashMap,
    convert::Infallible,
    path::{Path, PathBuf},
    sync::Arc,
};

use axum::{
    Json,
    extract::{Multipart, Path as AxumPath, State},
    response::sse::{Event, KeepAlive, Sse},
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::{
    io::AsyncWriteExt,
    sync::{Mutex, broadcast},
};
use tokio_stream::wrappers::BroadcastStream;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use valcoach_db::{Database, ParseJobRecord};
use valcoach_domain::{ReplayInput, ReplayRegion};
use valcoach_metrics::summarize_movement;
use valcoach_replay_adapter::{ReplayDataSource, ReplaySourceError, ValorantReplayParserSource};
use valcoach_vrf_probe::{ProbeError, ProbedRegion, probe_file, write_probe_artifacts};

use crate::{
    AppState,
    auth::{AuthApiError, require_user_id},
};

const MAX_REPLAY_UPLOAD_BYTES: usize = 100 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct JobManager {
    database: Database,
    parser_source: ValorantReplayParserSource,
    data_directory: PathBuf,
    controls: Arc<Mutex<HashMap<String, JobControl>>>,
}

#[derive(Clone, Debug)]
struct JobControl {
    cancel: CancellationToken,
    events: broadcast::Sender<JobEvent>,
}

#[derive(Clone, Debug, Serialize)]
pub struct JobEvent {
    pub job_id: String,
    pub status: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct JobCreated {
    pub job_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BundleBackend {
    name: String,
    revision: String,
    dialect: String,
    status: String,
    detail: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct BundleRecordCounts {
    server_events: u64,
    normalized_events: u64,
    movement_samples: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct BundleIntegrity {
    malformed_packets: Option<u64>,
    partial_errors: Option<u64>,
    undecoded_groups: Option<u64>,
    timeline_coverage_ms: u64,
    valid_server_event_payloads: u64,
    invalid_server_event_payloads: u64,
    event_trailing_bytes: u64,
    checkpoint_trailing_bytes: u64,
    replay_data_trailing_bytes: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct ParserDiagnostics {
    stats: ParserStats,
    counts: ParserCounts,
}

#[derive(Debug, Clone, Deserialize)]
struct ParserStats {
    malformed_packet_count: u64,
    partial_error_count: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct ParserCounts {
    undecoded_export_groups: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BundleManifest {
    schema_version: u32,
    source: valcoach_vrf_probe::SourceIdentity,
    replay: valcoach_vrf_probe::ReplayBuildIdentity,
    backend: BundleBackend,
    validation_backends: Vec<BundleBackend>,
    capabilities: valcoach_domain::ReplayCapabilities,
    records: BundleRecordCounts,
    integrity: BundleIntegrity,
    artifacts: Vec<String>,
}

impl JobManager {
    pub fn new(
        database: Database,
        parser_directory: impl Into<PathBuf>,
        dotnet_path: impl Into<PathBuf>,
        data_directory: impl Into<PathBuf>,
    ) -> Self {
        let data_directory = data_directory.into();
        let data_directory = std::path::absolute(&data_directory).unwrap_or(data_directory);
        Self {
            database,
            parser_source: ValorantReplayParserSource::new(parser_directory, dotnet_path),
            data_directory,
            controls: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn save_upload_and_enqueue(
        &self,
        user_id: String,
        mut multipart: Multipart,
    ) -> Result<JobCreated, AuthApiError> {
        let job_id = Uuid::new_v4().to_string();
        let replay_path = self
            .data_directory
            .join("replays")
            .join(&user_id)
            .join(format!("{job_id}.vrf"));
        let mut saved = false;
        let mut source_filename = None;

        while let Some(mut field) = multipart
            .next_field()
            .await
            .map_err(|error| AuthApiError::bad_request(error.to_string()))?
        {
            if field.name() != Some("replay") {
                continue;
            }
            if field
                .file_name()
                .is_some_and(|name| !name.to_ascii_lowercase().ends_with(".vrf"))
            {
                return Err(AuthApiError::bad_request(
                    "uploaded replay must use the .vrf extension",
                ));
            }
            source_filename = Some(safe_source_filename(field.file_name()));
            let parent = replay_path
                .parent()
                .ok_or_else(|| AuthApiError::internal("invalid controlled replay path"))?;
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| AuthApiError::internal(error.to_string()))?;
            let temporary_path = replay_path.with_extension("vrf.part");
            let mut file = tokio::fs::File::create(&temporary_path)
                .await
                .map_err(|error| AuthApiError::internal(error.to_string()))?;
            let mut bytes_written = 0usize;
            while let Some(chunk) = field
                .chunk()
                .await
                .map_err(|error| AuthApiError::bad_request(error.to_string()))?
            {
                bytes_written = bytes_written.saturating_add(chunk.len());
                if bytes_written > MAX_REPLAY_UPLOAD_BYTES {
                    let _ = tokio::fs::remove_file(&temporary_path).await;
                    return Err(AuthApiError::bad_request(
                        "replay exceeds the 100 MiB upload limit",
                    ));
                }
                file.write_all(&chunk)
                    .await
                    .map_err(|error| AuthApiError::internal(error.to_string()))?;
            }
            file.flush()
                .await
                .map_err(|error| AuthApiError::internal(error.to_string()))?;
            tokio::fs::rename(&temporary_path, &replay_path)
                .await
                .map_err(|error| AuthApiError::internal(error.to_string()))?;
            saved = true;
            break;
        }

        if !saved {
            return Err(AuthApiError::bad_request(
                "multipart field 'replay' is required",
            ));
        }
        self.enqueue_replay(
            user_id,
            job_id,
            replay_path,
            source_filename.unwrap_or_else(|| "unknown.vrf".to_owned()),
        )
        .await
        .map_err(|error| AuthApiError::internal(error.to_string()))
    }

    pub async fn enqueue_replay(
        &self,
        user_id: String,
        job_id: String,
        replay_path: PathBuf,
        source_filename: String,
    ) -> Result<JobCreated, JobManagerError> {
        self.database
            .create_parse_job(&job_id, &user_id, self.parser_source.source_name())
            .await?;
        let cancel = CancellationToken::new();
        let (events, _) = broadcast::channel(32);
        self.controls.lock().await.insert(
            job_id.clone(),
            JobControl {
                cancel: cancel.clone(),
                events: events.clone(),
            },
        );
        self.publish(&events, &job_id, "queued", "Replay upload accepted");

        let manager = self.clone();
        let task_job_id = job_id.clone();
        tokio::spawn(async move {
            manager
                .run(
                    task_job_id,
                    user_id,
                    replay_path,
                    source_filename,
                    cancel,
                    events,
                )
                .await;
        });
        Ok(JobCreated { job_id })
    }

    pub async fn find_for_user(
        &self,
        job_id: &str,
        user_id: &str,
    ) -> Result<Option<ParseJobRecord>, JobManagerError> {
        Ok(self
            .database
            .find_parse_job_for_user(job_id, user_id)
            .await?)
    }

    pub async fn cancel_for_user(
        &self,
        job_id: &str,
        user_id: &str,
    ) -> Result<bool, JobManagerError> {
        let Some(job) = self.find_for_user(job_id, user_id).await? else {
            return Ok(false);
        };
        if !matches!(
            job.status.as_str(),
            "queued" | "probing" | "parsing" | "normalizing" | "persisting" | "computing_metrics"
        ) {
            return Ok(false);
        }
        let control = self.controls.lock().await.get(job_id).cloned();
        let Some(control) = control else {
            return Ok(false);
        };
        control.cancel.cancel();
        self.transition(
            &control.events,
            job_id,
            "cancelled",
            "Cancellation requested",
            None,
            None,
        )
        .await?;
        Ok(true)
    }

    pub async fn subscribe(&self, job_id: &str) -> Option<broadcast::Receiver<JobEvent>> {
        self.controls
            .lock()
            .await
            .get(job_id)
            .map(|control| control.events.subscribe())
    }

    async fn run(
        &self,
        job_id: String,
        user_id: String,
        replay_path: PathBuf,
        source_filename: String,
        cancel: CancellationToken,
        events: broadcast::Sender<JobEvent>,
    ) {
        let job_directory = self.data_directory.join("jobs").join(&job_id);
        let output_directory = job_directory.join("parser-output");
        let probe_directory = job_directory.join("bundle");
        let result = async {
            self.transition(
                &events,
                &job_id,
                "probing",
                "Reading replay identity and server timeline",
                None,
                None,
            )
            .await?;
            let probe_path = replay_path.clone();
            let probe_output = probe_directory.clone();
            let probe = tokio::task::spawn_blocking(move || {
                let mut report = probe_file(&probe_path)?;
                report.source.filename = source_filename;
                write_probe_artifacts(&report, &probe_output)?;
                Ok::<_, ProbeError>(report)
            })
            .await
            .map_err(JobManagerError::ProbeTask)??;
            let region = match probe.replay.region {
                ProbedRegion::Global => {
                    write_bundle_manifest(
                        &probe_directory,
                        &probe,
                        "pending",
                        "Awaiting verified Global 13.05 payload export",
                        valcoach_domain::ReplayCapabilities::global_fixture(
                            valcoach_domain::CapabilityLevel::Partial,
                        ),
                        BundleRecordCounts {
                            server_events: probe.chunks.events,
                            ..BundleRecordCounts::default()
                        },
                        None,
                    )
                    .await?;
                    ReplayRegion::Global
                }
                ProbedRegion::China => {
                    write_bundle_manifest(
                        &probe_directory,
                        &probe,
                        "unsupported",
                        "China 13.05 container and server timeline are valid; no verified payload transform is available",
                        valcoach_domain::ReplayCapabilities::china_container_only(),
                        BundleRecordCounts {
                            server_events: probe.chunks.events,
                            ..BundleRecordCounts::default()
                        },
                        None,
                    )
                    .await?;
                    return Err(JobManagerError::Source(
                        ReplaySourceError::UnsupportedTransform {
                            branch: probe.replay.branch,
                        },
                    ));
                }
                ProbedRegion::Unknown => {
                    write_bundle_manifest(
                        &probe_directory,
                        &probe,
                        "unsupported",
                        "Replay branch is not a registered ValCoach payload dialect",
                        valcoach_domain::ReplayCapabilities::unknown_branch(),
                        BundleRecordCounts {
                            server_events: probe.chunks.events,
                            ..BundleRecordCounts::default()
                        },
                        None,
                    )
                    .await?;
                    return Err(JobManagerError::Source(
                        ReplaySourceError::UnsupportedBranch {
                            branch: probe.replay.branch,
                        },
                    ));
                }
            };
            if cancel.is_cancelled() {
                return Err(JobManagerError::Cancelled);
            }
            self.transition(
                &events,
                &job_id,
                "parsing",
                "Running local replay parser",
                None,
                None,
            )
            .await?;
            let mut replay = self
                .parser_source
                .ingest(
                    ReplayInput::Vrf {
                        path: replay_path,
                        region,
                        output_directory: output_directory.clone(),
                    },
                    cancel.clone(),
                )
                .await?;
            replay.metadata.replay_id = probe.replay.internal_replay_id.clone();
            replay.metadata.branch = Some(probe.replay.branch.clone());
            replay.metadata.map = probe.replay.map_asset_path.clone();
            replay.metadata.duration_ms = Some(i64::from(probe.replay.duration_ms));
            let parser_diagnostics = read_parser_diagnostics(&output_directory).await?;
            promote_parser_artifacts(&mut replay, &output_directory, &probe_directory).await?;
            write_bundle_manifest(
                &probe_directory,
                &probe,
                "complete",
                "Verified ValorantReplayParser Global 13.05 export using the valcoach compact profile",
                replay.capabilities.clone(),
                BundleRecordCounts {
                    server_events: probe.chunks.events,
                    normalized_events: replay.summary.event_count,
                    movement_samples: replay.summary.movement_count,
                },
                Some(parser_diagnostics),
            )
            .await?;
            if cancel.is_cancelled() {
                return Err(JobManagerError::Cancelled);
            }
            self.transition(
                &events,
                &job_id,
                "normalizing",
                "Validating streamed NDJSON",
                None,
                None,
            )
            .await?;
            if cancel.is_cancelled() {
                return Err(JobManagerError::Cancelled);
            }
            self.transition(
                &events,
                &job_id,
                "persisting",
                "Writing normalized events and movement samples",
                None,
                None,
            )
            .await?;
            let match_id = Uuid::new_v4().to_string();
            self.database
                .insert_parsed_replay_with_records(&user_id, &match_id, &replay, cancel.clone())
                .await?;
            self.transition(
                &events,
                &job_id,
                "computing_metrics",
                "Computing capability-gated movement metrics",
                None,
                Some(&match_id),
            )
            .await?;
            if cancel.is_cancelled() {
                return Err(JobManagerError::Cancelled);
            }
            self.compute_movement_metrics(&user_id, &match_id, &cancel)
                .await?;
            self.transition(
                &events,
                &job_id,
                "ready",
                "Replay is ready",
                None,
                Some(&match_id),
            )
            .await?;
            Ok::<(), JobManagerError>(())
        }
        .await;

        if let Err(error) = result {
            let error_detail = error.to_string();
            let (status, message) = match &error {
                JobManagerError::Cancelled
                | JobManagerError::Source(ReplaySourceError::Cancelled) => {
                    ("cancelled", "Replay job cancelled")
                }
                JobManagerError::Source(ReplaySourceError::UnsupportedTransform { .. }) => (
                    "unsupported",
                    "This replay container is valid, but its branch has no verified payload transform",
                ),
                JobManagerError::Source(ReplaySourceError::UnsupportedBranch { .. }) => (
                    "unsupported",
                    "This replay branch is not registered as a supported dialect",
                ),
                other => {
                    tracing::error!(job_id = %job_id, error = %other, "replay job failed");
                    (
                        "failed",
                        "Replay parser or import pipeline did not complete",
                    )
                }
            };
            let _ = self
                .transition(&events, &job_id, status, message, Some(&error_detail), None)
                .await;
        }
    }

    async fn compute_movement_metrics(
        &self,
        user_id: &str,
        match_id: &str,
        cancel: &CancellationToken,
    ) -> Result<(), JobManagerError> {
        for player in self
            .database
            .list_players_for_match_for_user(user_id, match_id)
            .await?
        {
            if cancel.is_cancelled() {
                return Err(JobManagerError::Cancelled);
            }
            let samples = self
                .database
                .movement_for_player_for_user(user_id, match_id, &player.id)
                .await?;
            let metric = summarize_movement(match_id, &player.id, &samples);
            self.database
                .insert_match_metric(
                    &Uuid::new_v4().to_string(),
                    match_id,
                    "movement_summary_v1",
                    &serde_json::to_string(&metric)?,
                )
                .await?;
        }
        Ok(())
    }

    async fn transition(
        &self,
        events: &broadcast::Sender<JobEvent>,
        job_id: &str,
        status: &str,
        message: &str,
        error_message: Option<&str>,
        match_id: Option<&str>,
    ) -> Result<(), JobManagerError> {
        self.database
            .update_parse_job(job_id, status, error_message, match_id)
            .await?;
        self.publish(events, job_id, status, message);
        Ok(())
    }

    fn publish(
        &self,
        events: &broadcast::Sender<JobEvent>,
        job_id: &str,
        status: &str,
        message: &str,
    ) {
        let _ = events.send(JobEvent {
            job_id: job_id.to_owned(),
            status: status.to_owned(),
            message: message.to_owned(),
        });
    }
}

fn safe_source_filename(filename: Option<&str>) -> String {
    filename
        .and_then(|name| Path::new(name).file_name())
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("unknown.vrf")
        .to_owned()
}

async fn write_bundle_manifest(
    directory: &Path,
    probe: &valcoach_vrf_probe::ProbeReport,
    payload_status: &str,
    payload_detail: &str,
    capabilities: valcoach_domain::ReplayCapabilities,
    records: BundleRecordCounts,
    parser_diagnostics: Option<ParserDiagnostics>,
) -> Result<(), JobManagerError> {
    let dialect = match probe.replay.region {
        ProbedRegion::Global => "global-13.05",
        ProbedRegion::China => "china-13.05",
        ProbedRegion::Unknown => "unknown",
    };
    let (timeline_min, timeline_max) =
        probe
            .server_events
            .iter()
            .fold((None::<u32>, None::<u32>), |(minimum, maximum), event| {
                (
                    Some(minimum.map_or(event.time_ms, |value| value.min(event.time_ms))),
                    Some(maximum.map_or(event.time_ms, |value| value.max(event.time_ms))),
                )
            });
    let timeline_coverage_ms = match (timeline_min, timeline_max) {
        (Some(minimum), Some(maximum)) => u64::from(maximum.saturating_sub(minimum)),
        _ => 0,
    };
    let mut artifacts = vec![
        "manifest.json".to_owned(),
        "probe.json".to_owned(),
        "server_events.ndjson".to_owned(),
    ];
    if payload_status == "complete" {
        artifacts.extend([
            "parser_events.ndjson".to_owned(),
            "movement.ndjson".to_owned(),
            "diagnostics.json".to_owned(),
        ]);
    }
    let integrity = BundleIntegrity {
        malformed_packets: parser_diagnostics
            .as_ref()
            .map(|value| value.stats.malformed_packet_count),
        partial_errors: parser_diagnostics
            .as_ref()
            .map(|value| value.stats.partial_error_count),
        undecoded_groups: parser_diagnostics
            .as_ref()
            .map(|value| value.counts.undecoded_export_groups),
        timeline_coverage_ms,
        valid_server_event_payloads: probe.integrity.valid_event_payloads,
        invalid_server_event_payloads: probe.integrity.invalid_event_payloads,
        event_trailing_bytes: probe.integrity.event_trailing_bytes,
        checkpoint_trailing_bytes: probe.integrity.checkpoint_trailing_bytes,
        replay_data_trailing_bytes: probe.integrity.replay_data_trailing_bytes,
    };
    let manifest = BundleManifest {
        schema_version: 1,
        source: probe.source.clone(),
        replay: probe.replay.clone(),
        backend: BundleBackend {
            name: "michel-giehl/ValorantReplayParser".to_owned(),
            revision: "b51d67423b7b4952d59051cf91e55efa1c42da05".to_owned(),
            dialect: dialect.to_owned(),
            status: payload_status.to_owned(),
            detail: payload_detail.to_owned(),
        },
        validation_backends: vec![BundleBackend {
            name: "yakisoba0728/vrfkit:vrf-container".to_owned(),
            revision: "a73ee3aab474e38af4de7157fb8d94b34bee0963".to_owned(),
            dialect: "common-container-v7".to_owned(),
            status: "complete".to_owned(),
            detail: "Region-independent container and server-event probe".to_owned(),
        }],
        capabilities,
        records,
        integrity,
        artifacts,
    };
    tokio::fs::create_dir_all(directory).await?;
    let bytes = serde_json::to_vec_pretty(&manifest)?;
    tokio::fs::write(directory.join("manifest.json"), bytes).await?;
    Ok(())
}

async fn read_parser_diagnostics(directory: &Path) -> Result<ParserDiagnostics, JobManagerError> {
    let bytes = tokio::fs::read(directory.join("manifest.json")).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

async fn promote_parser_artifacts(
    replay: &mut valcoach_domain::ParsedReplay,
    parser_directory: &Path,
    bundle_directory: &Path,
) -> Result<(), JobManagerError> {
    tokio::fs::create_dir_all(bundle_directory).await?;
    let parser_events = bundle_directory.join("parser_events.ndjson");
    let movement = bundle_directory.join("movement.ndjson");
    tokio::fs::rename(parser_directory.join("events.ndjson"), &parser_events).await?;
    tokio::fs::rename(parser_directory.join("movement.ndjson"), &movement).await?;
    tokio::fs::rename(
        parser_directory.join("manifest.json"),
        bundle_directory.join("diagnostics.json"),
    )
    .await?;
    replay.bundle.events_path = parser_events;
    replay.bundle.movement_path = movement;
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum JobManagerError {
    #[error(transparent)]
    Database(#[from] valcoach_db::DatabaseError),
    #[error(transparent)]
    Source(#[from] ReplaySourceError),
    #[error(transparent)]
    Probe(#[from] ProbeError),
    #[error("replay probe task failed: {0}")]
    ProbeTask(#[source] tokio::task::JoinError),
    #[error("failed to serialize metric result: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("failed to write replay bundle artifact: {0}")]
    Io(#[from] std::io::Error),
    #[error("job cancelled")]
    Cancelled,
}

pub async fn upload_replay(
    State(state): State<AppState>,
    session: tower_sessions::Session,
    multipart: Multipart,
) -> Result<Json<JobCreated>, AuthApiError> {
    let user_id = require_user_id(&state.auth, &session).await?;
    Ok(Json(
        state
            .jobs
            .save_upload_and_enqueue(user_id, multipart)
            .await?,
    ))
}

pub async fn get_job(
    State(state): State<AppState>,
    session: tower_sessions::Session,
    AxumPath(job_id): AxumPath<String>,
) -> Result<Json<ParseJobRecord>, AuthApiError> {
    let user_id = require_user_id(&state.auth, &session).await?;
    let job = state
        .jobs
        .find_for_user(&job_id, &user_id)
        .await
        .map_err(|error| AuthApiError::internal(error.to_string()))?;
    job.map(Json).ok_or_else(AuthApiError::unauthorized)
}

pub async fn get_job_bundle(
    State(state): State<AppState>,
    session: tower_sessions::Session,
    AxumPath(job_id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, AuthApiError> {
    let user_id = require_user_id(&state.auth, &session).await?;
    if state
        .jobs
        .find_for_user(&job_id, &user_id)
        .await
        .map_err(|error| AuthApiError::internal(error.to_string()))?
        .is_none()
    {
        return Err(AuthApiError::unauthorized());
    }
    let path = state
        .jobs
        .data_directory
        .join("jobs")
        .join(job_id)
        .join("bundle")
        .join("manifest.json");
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|error| AuthApiError::internal(error.to_string()))?;
    let manifest = serde_json::from_slice(&bytes)
        .map_err(|error| AuthApiError::internal(error.to_string()))?;
    Ok(Json(manifest))
}

pub async fn cancel_job(
    State(state): State<AppState>,
    session: tower_sessions::Session,
    AxumPath(job_id): AxumPath<String>,
) -> Result<Json<JobEvent>, AuthApiError> {
    let user_id = require_user_id(&state.auth, &session).await?;
    if !state
        .jobs
        .cancel_for_user(&job_id, &user_id)
        .await
        .map_err(|error| AuthApiError::internal(error.to_string()))?
    {
        return Err(AuthApiError::unauthorized());
    }
    Ok(Json(JobEvent {
        job_id,
        status: "cancelled".to_owned(),
        message: "Cancellation requested".to_owned(),
    }))
}

pub async fn job_events(
    State(state): State<AppState>,
    session: tower_sessions::Session,
    AxumPath(job_id): AxumPath<String>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, AuthApiError> {
    let user_id = require_user_id(&state.auth, &session).await?;
    if state
        .jobs
        .find_for_user(&job_id, &user_id)
        .await
        .map_err(|error| AuthApiError::internal(error.to_string()))?
        .is_none()
    {
        return Err(AuthApiError::unauthorized());
    }
    let receiver = state
        .jobs
        .subscribe(&job_id)
        .await
        .ok_or_else(|| AuthApiError::internal("job event stream is no longer available"))?;
    let stream = BroadcastStream::new(receiver).filter_map(|result| {
        futures_util::future::ready(match result {
            Ok(event) => Some(Ok(Event::default().event(event.status).data(event.message))),
            Err(_) => None,
        })
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tempfile::tempdir;
    use tokio::time::{Duration, sleep};
    use valcoach_db::UserRecord;

    use super::JobManager;

    fn workspace_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .expect("workspace root")
            .to_path_buf()
    }

    #[tokio::test]
    #[ignore = "runs the local C# parser against the full Global 13.05 fixture"]
    async fn global_13_05_job_reaches_ready_and_persists_a_match_summary() {
        // Match the WAL-backed production database. A single-connection in-memory
        // database cannot serve status reads during the atomic replay import.
        let storage = tempdir().expect("temporary storage");
        let database_url = format!(
            "sqlite://{}",
            storage
                .path()
                .join("global-job.db")
                .to_string_lossy()
                .replace('\\', "/")
        );
        let database = valcoach_db::Database::connect(&database_url)
            .await
            .expect("database");
        database
            .create_user(&UserRecord {
                id: "user-1".to_owned(),
                username: "job_user".to_owned(),
                password_hash: "hash".to_owned(),
            })
            .await
            .expect("user");
        let root = workspace_root();
        let manager = JobManager::new(
            database.clone(),
            root.join(".external").join("ValorantReplayParser"),
            "C:\\Program Files\\dotnet\\dotnet.exe",
            storage.path(),
        );
        let fixture = root
            .join("Demos-Global")
            .join("ec22cf8e-b1f4-48b7-8426-c60a20562b3e.vrf");
        let job = manager
            .enqueue_replay(
                "user-1".to_owned(),
                "job-1".to_owned(),
                fixture,
                "global-13.05.vrf".to_owned(),
            )
            .await
            .expect("queue fixture");

        for _ in 0..1_800 {
            let status = manager
                .find_for_user(&job.job_id, "user-1")
                .await
                .expect("job lookup")
                .expect("job");
            if status.status == "ready" {
                let match_id = status.match_id.expect("match id");
                let event_count: i64 =
                    sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE match_id = ?")
                        .bind(&match_id)
                        .fetch_one(database.pool())
                        .await
                        .expect("event count");
                let movement_count: i64 =
                    sqlx::query_scalar("SELECT COUNT(*) FROM movement_samples WHERE match_id = ?")
                        .bind(&match_id)
                        .fetch_one(database.pool())
                        .await
                        .expect("movement count");
                assert_eq!(event_count, 138_065);
                assert_eq!(movement_count, 165_047);
                let metric_count: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM match_metrics WHERE match_id = ? AND metric_name = 'movement_summary_v1'",
                )
                .bind(&match_id)
                .fetch_one(database.pool())
                .await
                .expect("movement metric count");
                assert!(metric_count > 0, "movement metrics were not generated");
                return;
            }
            assert!(
                !matches!(
                    status.status.as_str(),
                    "failed" | "unsupported" | "cancelled"
                ),
                "fixture job ended as {}: {:?}",
                status.status,
                status.error_message
            );
            sleep(Duration::from_millis(100)).await;
        }

        panic!("fixture job did not reach ready within 180 seconds");
    }

    #[tokio::test]
    #[ignore = "reads the full China 13.05 fixture"]
    async fn china_13_05_job_reports_verified_unsupported_transform() {
        let database = valcoach_db::Database::connect("sqlite::memory:")
            .await
            .expect("database");
        database
            .create_user(&UserRecord {
                id: "user-cn".to_owned(),
                username: "job_user_cn".to_owned(),
                password_hash: "hash".to_owned(),
            })
            .await
            .expect("user");
        let root = workspace_root();
        let storage = tempdir().expect("temporary storage");
        let manager = JobManager::new(
            database,
            root.join(".external").join("ValorantReplayParser"),
            "C:\\Program Files\\dotnet\\dotnet.exe",
            storage.path(),
        );
        let fixture = root
            .join("Demos-China")
            .join("0d7e68dd-1563-4f12-ba54-1afdf5f99916.vrf");
        let job = manager
            .enqueue_replay(
                "user-cn".to_owned(),
                "job-cn".to_owned(),
                fixture,
                "china-13.05.vrf".to_owned(),
            )
            .await
            .expect("queue fixture");

        for _ in 0..100 {
            let status = manager
                .find_for_user(&job.job_id, "user-cn")
                .await
                .expect("job lookup")
                .expect("job");
            if status.status == "unsupported" {
                let detail = status.error_message.expect("unsupported detail");
                assert!(detail.contains("++Ares-Core+release-china-13.05"));
                let manifest: serde_json::Value = serde_json::from_slice(
                    &tokio::fs::read(storage.path().join("jobs/job-cn/bundle/manifest.json"))
                        .await
                        .expect("bundle manifest"),
                )
                .expect("valid manifest");
                assert_eq!(manifest["backend"]["status"], "unsupported");
                assert_eq!(manifest["records"]["server_events"], 239);
                return;
            }
            assert_ne!(
                status.status, "failed",
                "probe failed: {:?}",
                status.error_message
            );
            sleep(Duration::from_millis(100)).await;
        }
        panic!("China fixture did not reach unsupported within 10 seconds");
    }
}
