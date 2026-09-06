//! Stable, parser-independent data contracts for ValCoach.

pub mod humanize;

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type MatchId = String;
pub type PlayerId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayRegion {
    Global,
    China,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReplayInput {
    Vrf {
        path: PathBuf,
        region: ReplayRegion,
        output_directory: PathBuf,
    },
    ParsedBundle(ParsedBundle),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedBundle {
    pub events_path: PathBuf,
    pub movement_path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_events_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayMetadata {
    pub replay_id: String,
    pub branch: Option<String>,
    pub map: Option<String>,
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityLevel {
    #[serde(rename = "complete", alias = "supported")]
    Supported,
    Partial,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayCapabilities {
    pub metadata: CapabilityLevel,
    pub container: CapabilityLevel,
    pub server_events: CapabilityLevel,
    pub movement: CapabilityLevel,
    pub actors: CapabilityLevel,
    pub player_identity: CapabilityLevel,
    pub gunplay: CapabilityLevel,
    pub combat: CapabilityLevel,
    pub abilities: CapabilityLevel,
    pub economy: CapabilityLevel,
    pub spike_state: CapabilityLevel,
    pub rounds: CapabilityLevel,
    pub game_state: CapabilityLevel,
    pub world_state: CapabilityLevel,
    pub checkpoints: CapabilityLevel,
}

impl ReplayCapabilities {
    pub fn global_fixture(gunplay: CapabilityLevel) -> Self {
        Self {
            metadata: CapabilityLevel::Supported,
            container: CapabilityLevel::Supported,
            server_events: CapabilityLevel::Supported,
            movement: CapabilityLevel::Supported,
            actors: CapabilityLevel::Supported,
            player_identity: CapabilityLevel::Supported,
            gunplay,
            combat: CapabilityLevel::Supported,
            abilities: CapabilityLevel::Partial,
            economy: CapabilityLevel::Partial,
            spike_state: CapabilityLevel::Supported,
            rounds: CapabilityLevel::Supported,
            game_state: CapabilityLevel::Partial,
            world_state: CapabilityLevel::Unsupported,
            checkpoints: CapabilityLevel::Partial,
        }
    }

    pub fn china_container_only() -> Self {
        Self {
            metadata: CapabilityLevel::Supported,
            container: CapabilityLevel::Supported,
            server_events: CapabilityLevel::Supported,
            movement: CapabilityLevel::Unsupported,
            actors: CapabilityLevel::Unsupported,
            player_identity: CapabilityLevel::Unsupported,
            gunplay: CapabilityLevel::Unsupported,
            combat: CapabilityLevel::Unsupported,
            abilities: CapabilityLevel::Unsupported,
            economy: CapabilityLevel::Unsupported,
            spike_state: CapabilityLevel::Partial,
            rounds: CapabilityLevel::Partial,
            game_state: CapabilityLevel::Partial,
            world_state: CapabilityLevel::Unsupported,
            checkpoints: CapabilityLevel::Partial,
        }
    }

    pub fn unknown_branch() -> Self {
        Self {
            metadata: CapabilityLevel::Supported,
            container: CapabilityLevel::Supported,
            server_events: CapabilityLevel::Partial,
            movement: CapabilityLevel::Unsupported,
            actors: CapabilityLevel::Unsupported,
            player_identity: CapabilityLevel::Unsupported,
            gunplay: CapabilityLevel::Unsupported,
            combat: CapabilityLevel::Unsupported,
            abilities: CapabilityLevel::Unsupported,
            economy: CapabilityLevel::Unsupported,
            spike_state: CapabilityLevel::Unsupported,
            rounds: CapabilityLevel::Unsupported,
            game_state: CapabilityLevel::Unsupported,
            world_state: CapabilityLevel::Unsupported,
            checkpoints: CapabilityLevel::Partial,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Vector3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MovementSample {
    pub timestamp_ms: i64,
    pub packet_id: Option<i64>,
    pub actor_net_guid: Option<u64>,
    pub character_net_guid: Option<u64>,
    pub position: Vector3,
    pub velocity: Option<Vector3>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yaw: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitch: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub round_no: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alive: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenericEvent {
    pub event_type: String,
    pub timestamp_ms: i64,
    pub actor_net_guid: Option<u64>,
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub match_id: MatchId,
    pub round_no: Option<u32>,
    pub timestamp_ms: Option<i64>,
    pub player_id: Option<PlayerId>,
    pub evidence_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_row: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_event_type: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CoordinateBounds {
    pub min: Option<Vector3>,
    pub max: Option<Vector3>,
}

impl CoordinateBounds {
    pub fn include(&mut self, point: &Vector3) {
        match (&mut self.min, &mut self.max) {
            (Some(min), Some(max)) => {
                min.x = min.x.min(point.x);
                min.y = min.y.min(point.y);
                min.z = min.z.min(point.z);
                max.x = max.x.max(point.x);
                max.y = max.y.max(point.y);
                max.z = max.z.max(point.z);
            }
            _ => {
                self.min = Some(point.clone());
                self.max = Some(point.clone());
            }
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimestampRange {
    pub min_ms: Option<i64>,
    pub max_ms: Option<i64>,
}

impl TimestampRange {
    pub fn include(&mut self, timestamp_ms: i64) {
        self.min_ms = Some(
            self.min_ms
                .map_or(timestamp_ms, |value| value.min(timestamp_ms)),
        );
        self.max_ms = Some(
            self.max_ms
                .map_or(timestamp_ms, |value| value.max(timestamp_ms)),
        );
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ParsedReplaySummary {
    pub event_count: u64,
    pub movement_count: u64,
    pub event_types: BTreeMap<String, u64>,
    pub distinct_actor_guids: u64,
    pub event_timestamps: TimestampRange,
    pub movement_timestamps: TimestampRange,
    pub movement_bounds: CoordinateBounds,
    pub has_shot_related_events: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedReplay {
    pub metadata: ReplayMetadata,
    pub bundle: ParsedBundle,
    pub source_name: String,
    pub capabilities: ReplayCapabilities,
    pub summary: ParsedReplaySummary,
}

#[cfg(test)]
mod tests {
    use super::{CoordinateBounds, Vector3};

    #[test]
    fn coordinate_bounds_expand_for_each_point() {
        let mut bounds = CoordinateBounds::default();
        bounds.include(&Vector3 {
            x: 2.0,
            y: -1.0,
            z: 5.0,
        });
        bounds.include(&Vector3 {
            x: -3.0,
            y: 4.0,
            z: 1.0,
        });

        assert_eq!(bounds.min.expect("minimum is set").x, -3.0);
        assert_eq!(bounds.max.expect("maximum is set").y, 4.0);
    }
}
