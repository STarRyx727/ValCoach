use std::collections::BTreeSet;
use std::path::Path;

use async_stream::try_stream;
use async_trait::async_trait;
use futures_core::Stream;
use serde_json::Value;
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_util::sync::CancellationToken;
use valcoach_domain::{
    CapabilityLevel, GenericEvent, MovementSample, ParsedBundle, ParsedReplay, ParsedReplaySummary,
    ReplayCapabilities, ReplayInput, ReplayMetadata, Vector3,
};

use crate::{ReplayDataSource, ReplaySourceError};

#[derive(Debug, Default)]
pub struct ParsedBundleSource;

#[derive(Debug, Clone)]
pub enum NormalizedRecord {
    Event(GenericEvent),
    Movement(MovementSample),
}

impl ParsedBundleSource {
    /// Reads the two NDJSON files line-by-line and emits only stable domain records.
    /// Consumers can batch records without retaining the full replay in memory.
    pub fn records(
        &self,
        bundle: ParsedBundle,
        cancel: CancellationToken,
    ) -> impl Stream<Item = Result<NormalizedRecord, ReplaySourceError>> + Send + 'static {
        try_stream! {
            for (kind, path) in [("event", bundle.events_path), ("movement", bundle.movement_path)] {
                let file = File::open(&path).await.map_err(|source| ReplaySourceError::Read {
                    path: path.clone(),
                    source,
                })?;
                let mut lines = BufReader::new(file).lines();
                let mut line_number = 0;
                while let Some(line) = lines.next_line().await.map_err(|source| ReplaySourceError::Read {
                    path: path.clone(),
                    source,
                })? {
                    if cancel.is_cancelled() {
                        Err(ReplaySourceError::Cancelled)?;
                    }
                    line_number += 1;
                    if line.trim().is_empty() {
                        continue;
                    }
                    if kind == "event" {
                        yield NormalizedRecord::Event(parse_event(&path, line_number, &line)?);
                    } else {
                        yield NormalizedRecord::Movement(parse_movement(&path, line_number, &line)?);
                    }
                }
            }
        }
    }

    async fn summarize(
        &self,
        bundle: ParsedBundle,
        cancel: CancellationToken,
    ) -> Result<ParsedReplay, ReplaySourceError> {
        let (mut summary, actors) = self.read_events(&bundle.events_path, &cancel).await?;
        summary.distinct_actor_guids = actors.len() as u64;
        self.read_movement(&bundle.movement_path, &cancel, &mut summary)
            .await?;

        let gunplay = if summary.has_shot_related_events {
            CapabilityLevel::Supported
        } else {
            CapabilityLevel::Partial
        };
        let duration_ms = summary
            .event_timestamps
            .max_ms
            .into_iter()
            .chain(summary.movement_timestamps.max_ms)
            .max();
        let replay_id = bundle
            .events_path
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .unwrap_or("parsed-bundle")
            .to_owned();

        Ok(ParsedReplay {
            metadata: ReplayMetadata {
                replay_id,
                branch: None,
                map: None,
                duration_ms,
            },
            bundle,
            source_name: self.source_name().to_owned(),
            capabilities: ReplayCapabilities::global_fixture(gunplay),
            summary,
        })
    }

    async fn read_events(
        &self,
        path: &Path,
        cancel: &CancellationToken,
    ) -> Result<(ParsedReplaySummary, BTreeSet<u64>), ReplaySourceError> {
        let file = File::open(path)
            .await
            .map_err(|source| ReplaySourceError::Read {
                path: path.to_path_buf(),
                source,
            })?;
        let mut lines = BufReader::new(file).lines();
        let mut summary = ParsedReplaySummary::default();
        let mut actors = BTreeSet::new();
        let mut line_number = 0;

        while let Some(line) =
            lines
                .next_line()
                .await
                .map_err(|source| ReplaySourceError::Read {
                    path: path.to_path_buf(),
                    source,
                })?
        {
            if cancel.is_cancelled() {
                return Err(ReplaySourceError::Cancelled);
            }
            line_number += 1;
            if line.trim().is_empty() {
                continue;
            }

            let event = parse_event(path, line_number, &line)?;
            summary.event_count += 1;
            *summary
                .event_types
                .entry(event.event_type.clone())
                .or_default() += 1;
            summary.event_timestamps.include(event.timestamp_ms);
            summary.has_shot_related_events |= is_shot_related(&event.event_type);
            if let Some(actor) = event.actor_net_guid {
                actors.insert(actor);
            }
        }

        Ok((summary, actors))
    }

    async fn read_movement(
        &self,
        path: &Path,
        cancel: &CancellationToken,
        summary: &mut ParsedReplaySummary,
    ) -> Result<(), ReplaySourceError> {
        let file = File::open(path)
            .await
            .map_err(|source| ReplaySourceError::Read {
                path: path.to_path_buf(),
                source,
            })?;
        let mut lines = BufReader::new(file).lines();
        let mut line_number = 0;

        while let Some(line) =
            lines
                .next_line()
                .await
                .map_err(|source| ReplaySourceError::Read {
                    path: path.to_path_buf(),
                    source,
                })?
        {
            if cancel.is_cancelled() {
                return Err(ReplaySourceError::Cancelled);
            }
            line_number += 1;
            if line.trim().is_empty() {
                continue;
            }

            let sample = parse_movement(path, line_number, &line)?;
            summary.movement_count += 1;
            summary.movement_timestamps.include(sample.timestamp_ms);
            summary.movement_bounds.include(&sample.position);
        }

        Ok(())
    }
}

#[async_trait]
impl ReplayDataSource for ParsedBundleSource {
    async fn ingest(
        &self,
        input: ReplayInput,
        cancel: CancellationToken,
    ) -> Result<ParsedReplay, ReplaySourceError> {
        match input {
            ReplayInput::ParsedBundle(bundle) => self.summarize(bundle, cancel).await,
            ReplayInput::Vrf { .. } => Err(ReplaySourceError::UnsupportedInput {
                source_name: self.source_name(),
                reason:
                    "ParsedBundleSource accepts only exported events.ndjson and movement.ndjson"
                        .to_owned(),
            }),
        }
    }

    fn capabilities(&self) -> ReplayCapabilities {
        ReplayCapabilities::global_fixture(CapabilityLevel::Partial)
    }

    fn source_name(&self) -> &'static str {
        "parsed_bundle"
    }
}

fn parse_event(path: &Path, line: u64, text: &str) -> Result<GenericEvent, ReplaySourceError> {
    let raw: Value = serde_json::from_str(text)
        .map_err(|error| invalid("event", path, line, error.to_string()))?;
    let event_type = required_string(&raw, "type", "event", path, line)?;
    let timestamp_ms = required_i64(&raw, "time_ms", "event", path, line)?;
    let actor_net_guid = raw.get("actor_net_guid").and_then(Value::as_u64);

    Ok(GenericEvent {
        event_type,
        timestamp_ms,
        actor_net_guid,
        raw,
    })
}

fn parse_movement(path: &Path, line: u64, text: &str) -> Result<MovementSample, ReplaySourceError> {
    let raw: Value = serde_json::from_str(text)
        .map_err(|error| invalid("movement", path, line, error.to_string()))?;
    let timestamp_ms = required_i64(&raw, "time_ms", "movement", path, line)?;
    let position = required_vector(&raw, "position", path, line)?;
    let velocity = raw
        .get("velocity")
        .map(|value| parse_vector(value, "velocity", path, line))
        .transpose()?;

    Ok(MovementSample {
        timestamp_ms,
        packet_id: raw.get("packet_id").and_then(Value::as_i64),
        actor_net_guid: raw.get("actor_net_guid").and_then(Value::as_u64),
        character_net_guid: raw
            .get("shooter_character_net_guid")
            .and_then(Value::as_u64),
        position,
        velocity,
        yaw: raw.get("yaw").and_then(Value::as_f64),
        pitch: raw.get("pitch").and_then(Value::as_f64),
        round_no: None,
        alive: None,
        area: None,
    })
}

fn required_string(
    raw: &Value,
    key: &str,
    kind: &'static str,
    path: &Path,
    line: u64,
) -> Result<String, ReplaySourceError> {
    raw.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| invalid(kind, path, line, format!("missing or non-string '{key}'")))
}

fn required_i64(
    raw: &Value,
    key: &str,
    kind: &'static str,
    path: &Path,
    line: u64,
) -> Result<i64, ReplaySourceError> {
    raw.get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| invalid(kind, path, line, format!("missing or non-integer '{key}'")))
}

fn required_vector(
    raw: &Value,
    key: &str,
    path: &Path,
    line: u64,
) -> Result<Vector3, ReplaySourceError> {
    raw.get(key)
        .ok_or_else(|| invalid("movement", path, line, format!("missing '{key}'")))
        .and_then(|value| parse_vector(value, key, path, line))
}

fn parse_vector(
    value: &Value,
    key: &str,
    path: &Path,
    line: u64,
) -> Result<Vector3, ReplaySourceError> {
    let coordinate = |axis: &str| {
        value
            .get(axis)
            .and_then(Value::as_f64)
            .filter(|number| number.is_finite())
            .ok_or_else(|| {
                invalid(
                    "movement",
                    path,
                    line,
                    format!("'{key}.{axis}' must be finite"),
                )
            })
    };

    Ok(Vector3 {
        x: coordinate("x")?,
        y: coordinate("y")?,
        z: coordinate("z")?,
    })
}

fn invalid(kind: &'static str, path: &Path, line: u64, reason: String) -> ReplaySourceError {
    ReplaySourceError::InvalidNdjson {
        kind,
        path: path.to_path_buf(),
        line,
        reason,
    }
}

fn is_shot_related(event_type: &str) -> bool {
    let lower = event_type.to_ascii_lowercase();
    lower.contains("shot") || lower.contains("damage") || lower.contains("kill")
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::tempdir;
    use tokio_util::sync::CancellationToken;
    use valcoach_domain::{CapabilityLevel, ParsedBundle, ReplayInput};

    use crate::{ParsedBundleSource, ReplayDataSource};

    #[tokio::test]
    async fn summarizes_bundle_without_loading_parser_types() {
        let directory = tempdir().expect("temporary directory");
        let events_path = directory.path().join("events.ndjson");
        let movement_path = directory.path().join("movement.ndjson");
        let mut events = std::fs::File::create(&events_path).expect("events file");
        writeln!(
            events,
            r#"{{"type":"actor_spawned","time_ms":8,"actor_net_guid":2}}"#
        )
        .expect("event row");
        writeln!(
            events,
            r#"{{"type":"rpc_received","time_ms":12,"actor_net_guid":2}}"#
        )
        .expect("event row");
        let mut movement = std::fs::File::create(&movement_path).expect("movement file");
        writeln!(movement, r#"{{"type":"remote_character_movement","time_ms":12,"actor_net_guid":2,"position":{{"x":1.0,"y":2.0,"z":3.0}}}}"#).expect("movement row");

        let result = ParsedBundleSource
            .ingest(
                ReplayInput::ParsedBundle(ParsedBundle {
                    events_path,
                    movement_path,
                    server_events_path: None,
                }),
                CancellationToken::new(),
            )
            .await
            .expect("valid bundle");

        assert_eq!(result.summary.event_count, 2);
        assert_eq!(result.summary.movement_count, 1);
        assert_eq!(result.capabilities.movement, CapabilityLevel::Supported);
        assert_eq!(result.capabilities.gunplay, CapabilityLevel::Partial);
    }
}
