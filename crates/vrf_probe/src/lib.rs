//! Region-independent `.vrf` container probe for ValCoach.
//!
//! This deliberately does not decode UE packets or payload transforms. Container parsing is
//! delegated to the MIT-licensed `vrf-container` crate from yakisoba0728/vrfkit; see NOTICE.md.

use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{BufWriter, Write},
    path::Path,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use vrf_container::{
    ChunkIterator, ChunkType, event_payload_seconds_matches_time, parse_checkpoint_chunk,
    parse_event_chunk, parse_known_event_payload, parse_preamble, parse_replay_data_meta,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbedRegion {
    Global,
    China,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceIdentity {
    pub filename: String,
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayBuildIdentity {
    pub internal_replay_id: String,
    pub region: ProbedRegion,
    pub branch: String,
    pub build_changelist: u32,
    pub header_changelist: u32,
    pub version: String,
    pub duration_ms: i32,
    pub map_asset_path: Option<String>,
    pub network_version: u32,
    pub network_checksum: u32,
    pub engine_network_protocol_version: u32,
    pub ue4_version: u32,
    pub ue5_version: u32,
    pub package_version_license: u32,
    pub platform: String,
    pub header_trailing_bytes: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkCounts {
    pub replay_data: u64,
    pub checkpoints: u64,
    pub events: u64,
    pub unknown: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerTimelineEvent {
    pub schema_version: u32,
    pub replay_id: String,
    pub id: String,
    pub group: String,
    pub time_ms: u32,
    pub time2_ms: u32,
    pub metadata: String,
    pub tag: Option<u32>,
    pub words: Option<Vec<u32>>,
    pub enum_name: Option<String>,
    pub seconds: Option<f32>,
    pub raw_payload_hex: String,
    pub trailing_bytes: usize,
    pub structurally_valid: bool,
    pub time_consistent: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeIntegrity {
    pub valid_event_payloads: u64,
    pub invalid_event_payloads: u64,
    pub event_trailing_bytes: u64,
    pub checkpoint_trailing_bytes: u64,
    pub replay_data_trailing_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeReport {
    pub schema_version: u32,
    pub source: SourceIdentity,
    pub replay: ReplayBuildIdentity,
    pub chunks: ChunkCounts,
    pub server_event_counts: BTreeMap<String, u64>,
    pub integrity: ProbeIntegrity,
    pub server_events: Vec<ServerTimelineEvent>,
    pub player_loadouts: Vec<ProbePlayerLoadout>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbePlayerLoadout {
    pub subject: String,
    pub character_id: String,
}

pub fn probe_file(path: &Path) -> Result<ProbeReport, ProbeError> {
    let data = fs::read(path).map_err(|source| ProbeError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let preamble = parse_preamble(&data)?;
    let branch = preamble.header.replay_version.branch.clone();
    let region = match branch.as_str() {
        "++Ares-Core+release-13.05" => ProbedRegion::Global,
        "++Ares-Core+release-china-13.05" => ProbedRegion::China,
        _ => ProbedRegion::Unknown,
    };
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown.vrf")
        .to_owned();
    let source = SourceIdentity {
        filename,
        sha256: hex::encode(Sha256::digest(&data)),
        size_bytes: data.len() as u64,
    };
    let version = &preamble.header.replay_version;
    let replay = ReplayBuildIdentity {
        internal_replay_id: preamble.info.friendly_name.clone(),
        region,
        branch,
        build_changelist: preamble.info.changelist,
        header_changelist: version.changelist,
        version: format!("{}.{}.{}", version.major, version.minor, version.patch),
        duration_ms: preamble.info.length_in_ms,
        map_asset_path: preamble
            .header
            .level_names_and_times
            .first()
            .map(|(name, _)| name.clone()),
        network_version: preamble.header.network_version,
        network_checksum: preamble.header.network_checksum,
        engine_network_protocol_version: preamble.header.engine_network_protocol_version,
        ue4_version: preamble.header.ue4_version,
        ue5_version: preamble.header.ue5_version,
        package_version_license: preamble.header.package_version_license,
        platform: preamble.header.platform.clone(),
        header_trailing_bytes: preamble.header.trailing_bytes,
    };
    let mut chunks = ChunkCounts::default();
    let mut server_event_counts = BTreeMap::new();
    let mut integrity = ProbeIntegrity::default();
    let mut server_events = Vec::new();
    let mut iterator = ChunkIterator::new(&data, preamble.remaining_offset);

    while let Some(chunk) = iterator.next_chunk()? {
        let end = chunk.data_offset + chunk.size_in_bytes as usize;
        let payload = &data[chunk.data_offset..end];
        match chunk.chunk_type {
            ChunkType::ReplayData => {
                chunks.replay_data += 1;
                integrity.replay_data_trailing_bytes +=
                    parse_replay_data_meta(payload)?.trailing_bytes as u64;
            }
            ChunkType::Checkpoint => {
                chunks.checkpoints += 1;
                integrity.checkpoint_trailing_bytes +=
                    parse_checkpoint_chunk(payload)?.trailing_bytes as u64;
            }
            ChunkType::Event => {
                chunks.events += 1;
                let event = parse_event_chunk(payload)?;
                *server_event_counts.entry(event.group.clone()).or_default() += 1;
                integrity.event_trailing_bytes += event.trailing_bytes as u64;
                let decoded = parse_known_event_payload(&event.group, event.payload);
                let structurally_valid = decoded.is_some();
                if structurally_valid {
                    integrity.valid_event_payloads += 1;
                } else {
                    integrity.invalid_event_payloads += 1;
                }
                let time_consistent = decoded.as_ref().is_some_and(|value| {
                    event_payload_seconds_matches_time(event.time1, value.seconds)
                });
                server_events.push(ServerTimelineEvent {
                    schema_version: 1,
                    replay_id: replay.internal_replay_id.clone(),
                    id: event.id,
                    group: event.group,
                    time_ms: event.time1,
                    time2_ms: event.time2,
                    metadata: event.metadata,
                    tag: decoded.as_ref().map(|value| value.tag),
                    words: decoded.as_ref().map(|value| value.words.clone()),
                    enum_name: decoded.as_ref().map(|value| value.name.clone()),
                    seconds: decoded.as_ref().map(|value| value.seconds),
                    raw_payload_hex: hex::encode(event.payload),
                    trailing_bytes: event.trailing_bytes,
                    structurally_valid,
                    time_consistent,
                });
            }
            ChunkType::Unknown(_) => chunks.unknown += 1,
            ChunkType::Header => {}
        }
    }

    Ok(ProbeReport {
        schema_version: 1,
        source,
        replay,
        chunks,
        server_event_counts,
        integrity,
        server_events,
        player_loadouts: extract_player_loadouts(&preamble.header.game_specific_data),
    })
}

fn extract_player_loadouts(entries: &[String]) -> Vec<ProbePlayerLoadout> {
    entries
        .iter()
        .filter_map(|entry| serde_json::from_str::<serde_json::Value>(entry).ok())
        .filter_map(|value| {
            value
                .get("playerLoadouts")
                .and_then(serde_json::Value::as_array)
                .cloned()
        })
        .flatten()
        .filter_map(|item| {
            Some(ProbePlayerLoadout {
                subject: item.get("subject")?.as_str()?.to_owned(),
                character_id: item.get("characterId")?.as_str()?.to_ascii_lowercase(),
            })
        })
        .collect()
}

pub fn write_probe_artifacts(report: &ProbeReport, output: &Path) -> Result<(), ProbeError> {
    fs::create_dir_all(output).map_err(|source| ProbeError::Write {
        path: output.to_path_buf(),
        source,
    })?;
    let events_path = output.join("server_events.ndjson");
    let events_file = File::create(&events_path).map_err(|source| ProbeError::Write {
        path: events_path.clone(),
        source,
    })?;
    let mut events = BufWriter::new(events_file);
    for event in &report.server_events {
        serde_json::to_writer(&mut events, event)?;
        events
            .write_all(b"\n")
            .map_err(|source| ProbeError::Write {
                path: events_path.clone(),
                source,
            })?;
    }
    events.flush().map_err(|source| ProbeError::Write {
        path: events_path,
        source,
    })?;

    let report_path = output.join("probe.json");
    let report_file = File::create(&report_path).map_err(|source| ProbeError::Write {
        path: report_path.clone(),
        source,
    })?;
    let mut public_report = report.clone();
    public_report.server_events.clear();
    serde_json::to_writer_pretty(BufWriter::new(report_file), &public_report)?;
    Ok(())
}

#[derive(Debug, Error)]
pub enum ProbeError {
    #[error("failed to read replay at {path}: {source}")]
    Read {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("failed to write probe artifact at {path}: {source}")]
    Write {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("failed to serialize probe artifact: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("VRF container corruption or unsupported container layout: {0}")]
    Container(#[from] vrf_container::ContainerError),
}

#[cfg(test)]
mod fixture_tests {
    use std::{fs, path::PathBuf};

    use super::{ProbeReport, ProbedRegion, probe_file};

    const FILE_MAGIC: u32 = 0x43F4_EFDD;
    const FILE_VERSION: u32 = 7;
    const REPLAY_INFO_BYTES: usize = 586;
    const NETWORK_MAGIC: u32 = 0x2CF5_A13D;

    fn workspace_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .expect("workspace root")
            .to_path_buf()
    }

    fn little_u32(data: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes(data[offset..offset + 4].try_into().expect("four bytes"))
    }

    fn assert_common(path: &PathBuf, report: &ProbeReport) {
        let data = fs::read(path).expect("fixture bytes");
        assert_eq!(little_u32(&data, 0), FILE_MAGIC);
        assert_eq!(little_u32(&data, 4), FILE_VERSION);
        assert_eq!(little_u32(&data, REPLAY_INFO_BYTES), 0, "Header chunk type");
        assert_eq!(little_u32(&data, REPLAY_INFO_BYTES + 8), NETWORK_MAGIC);
        assert_eq!(report.replay.network_version, 19);
        assert_eq!(report.replay.network_checksum, 4_217_436_668);
        assert_eq!(report.replay.engine_network_protocol_version, 32);
        assert_eq!(report.replay.version, "5.3.2");
        assert_eq!(report.replay.ue4_version, 522);
        assert_eq!(report.replay.ue5_version, 1009);
        assert_eq!(report.replay.package_version_license, 80);
        assert_eq!(report.replay.platform, "LinuxServer");
        assert_eq!(report.replay.header_trailing_bytes, 0);
        assert_eq!(report.chunks.unknown, 0);
        assert_eq!(report.integrity.invalid_event_payloads, 0);
        assert_eq!(report.integrity.event_trailing_bytes, 0);
        assert_eq!(report.integrity.checkpoint_trailing_bytes, 0);
        assert_eq!(report.integrity.replay_data_trailing_bytes, 0);
    }

    #[test]
    #[ignore = "requires the local Global 13.05 fixture"]
    fn global_13_05_container_regression() {
        let path = workspace_root()
            .join("Demos-Global")
            .join("ec22cf8e-b1f4-48b7-8426-c60a20562b3e.vrf");
        let report = probe_file(&path).expect("Global probe");
        assert_common(&path, &report);
        assert_eq!(report.replay.region, ProbedRegion::Global);
        assert_eq!(report.replay.branch, "++Ares-Core+release-13.05");
        assert_eq!(report.chunks.replay_data, 21);
        assert_eq!(report.chunks.checkpoints, 20);
        assert_eq!(report.chunks.events, 244);
        assert_eq!(report.integrity.valid_event_payloads, 244);
        assert_eq!(
            1 + report.chunks.replay_data + report.chunks.checkpoints + report.chunks.events,
            286
        );
    }

    #[test]
    #[ignore = "requires the local China 13.05 fixture"]
    fn china_13_05_container_regression() {
        let path = workspace_root()
            .join("Demos-China")
            .join("0d7e68dd-1563-4f12-ba54-1afdf5f99916.vrf");
        let report = probe_file(&path).expect("China probe");
        assert_common(&path, &report);
        assert_eq!(report.replay.region, ProbedRegion::China);
        assert_eq!(report.replay.branch, "++Ares-Core+release-china-13.05");
        assert_eq!(report.chunks.replay_data, 23);
        assert_eq!(report.chunks.checkpoints, 22);
        assert_eq!(report.chunks.events, 239);
        assert_eq!(report.integrity.valid_event_payloads, 239);
        assert_eq!(
            1 + report.chunks.replay_data + report.chunks.checkpoints + report.chunks.events,
            285
        );
    }
}
