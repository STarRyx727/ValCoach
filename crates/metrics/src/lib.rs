//! Deterministic metrics calculated only from normalized ValCoach domain records.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};
use valcoach_domain::{EvidenceRef, MovementSample, Vector3};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricStatus {
    Ok,
    Partial,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricResult<T> {
    pub status: MetricStatus,
    pub data: Option<T>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MovementMetrics {
    pub sample_count: u64,
    pub timestamp_start_ms: i64,
    pub timestamp_end_ms: i64,
    pub path_distance_raw_units: f64,
    pub average_velocity_raw_units_per_second: Option<f64>,
    pub maximum_velocity_raw_units_per_second: Option<f64>,
    pub evidence: Vec<EvidenceRef>,
}

/// Summarizes one stable replay player identity. Distances deliberately remain in parser/world
/// units until a validated map calibration is available.
pub fn summarize_movement(
    match_id: &str,
    player_id: &str,
    samples: &[MovementSample],
) -> MetricResult<MovementMetrics> {
    if samples.is_empty() {
        return MetricResult {
            status: MetricStatus::Unsupported,
            data: None,
            limitations: vec!["No movement samples are available for this player.".to_owned()],
        };
    }

    let mut ordered = samples.to_vec();
    ordered.sort_by_key(|sample| sample.timestamp_ms);
    let first = ordered.first().expect("non-empty movement samples");
    let last = ordered.last().expect("non-empty movement samples");

    let path_distance_raw_units = ordered
        .windows(2)
        .filter(|pair| {
            pair[0].round_no == pair[1].round_no
                && pair[0].alive.unwrap_or(true)
                && pair[1].alive.unwrap_or(true)
                && pair[1].timestamp_ms - pair[0].timestamp_ms <= 10_000
        })
        .map(|pair| distance(&pair[0].position, &pair[1].position))
        .sum();
    let velocities: Vec<f64> = ordered
        .iter()
        .filter_map(|sample| sample.velocity.as_ref().map(vector_length))
        .collect();
    let average_velocity_raw_units_per_second =
        (!velocities.is_empty()).then(|| velocities.iter().sum::<f64>() / velocities.len() as f64);
    let maximum_velocity_raw_units_per_second = velocities
        .iter()
        .copied()
        .max_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));
    let mut limitations = vec![
        "Path distance and velocity use raw replay coordinate units; no minimap calibration is applied."
            .to_owned(),
    ];
    let status = if velocities.len() == ordered.len() {
        MetricStatus::Ok
    } else {
        limitations.push("Some samples do not contain a velocity vector.".to_owned());
        MetricStatus::Partial
    };

    MetricResult {
        status,
        data: Some(MovementMetrics {
            sample_count: ordered.len() as u64,
            timestamp_start_ms: first.timestamp_ms,
            timestamp_end_ms: last.timestamp_ms,
            path_distance_raw_units,
            average_velocity_raw_units_per_second,
            maximum_velocity_raw_units_per_second,
            evidence: vec![
                EvidenceRef {
                    match_id: match_id.to_owned(),
                    round_no: None,
                    timestamp_ms: Some(first.timestamp_ms),
                    player_id: Some(player_id.to_owned()),
                    evidence_type: "movement_sample".to_owned(),
                    source_file: None,
                    source_row: None,
                    source_event_type: None,
                },
                EvidenceRef {
                    match_id: match_id.to_owned(),
                    round_no: None,
                    timestamp_ms: Some(last.timestamp_ms),
                    player_id: Some(player_id.to_owned()),
                    evidence_type: "movement_sample".to_owned(),
                    source_file: None,
                    source_row: None,
                    source_event_type: None,
                },
            ],
        }),
        limitations,
    }
}

/// Returns the closest observed sample in time; it never invents interpolated positions.
pub fn nearest_sample(samples: &[MovementSample], timestamp_ms: i64) -> Option<&MovementSample> {
    samples
        .iter()
        .min_by_key(|sample| (sample.timestamp_ms - timestamp_ms).unsigned_abs())
}

pub fn distance(left: &Vector3, right: &Vector3) -> f64 {
    vector_length(&Vector3 {
        x: left.x - right.x,
        y: left.y - right.y,
        z: left.z - right.z,
    })
}

fn vector_length(vector: &Vector3) -> f64 {
    (vector.x.powi(2) + vector.y.powi(2) + vector.z.powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use valcoach_domain::{MovementSample, Vector3};

    use super::{MetricStatus, nearest_sample, summarize_movement};

    fn sample(time: i64, x: f64, velocity: Option<f64>) -> MovementSample {
        MovementSample {
            timestamp_ms: time,
            packet_id: None,
            actor_net_guid: Some(1),
            character_net_guid: Some(7),
            position: Vector3 { x, y: 0.0, z: 0.0 },
            velocity: velocity.map(|x| Vector3 { x, y: 0.0, z: 0.0 }),
            yaw: None,
            pitch: None,
            round_no: Some(1),
            alive: Some(true),
            area: None,
        }
    }

    #[test]
    fn movement_summary_uses_observed_positions_and_velocities() {
        let result = summarize_movement(
            "match-1",
            "player-1",
            &[sample(200, 3.0, Some(4.0)), sample(100, 0.0, Some(2.0))],
        );
        let data = result.data.expect("movement data");
        assert_eq!(result.status, MetricStatus::Ok);
        assert_eq!(data.path_distance_raw_units, 3.0);
        assert_eq!(data.average_velocity_raw_units_per_second, Some(3.0));
        assert_eq!(data.evidence[0].timestamp_ms, Some(100));
    }

    #[test]
    fn missing_velocity_is_partial_not_zero() {
        let result = summarize_movement(
            "match-1",
            "player-1",
            &[sample(100, 0.0, None), sample(200, 2.0, Some(5.0))],
        );
        assert_eq!(result.status, MetricStatus::Partial);
        assert_eq!(
            result
                .data
                .expect("partial data remains useful")
                .average_velocity_raw_units_per_second,
            Some(5.0)
        );
    }

    #[test]
    fn nearest_sample_does_not_interpolate() {
        let samples = [sample(100, 0.0, None), sample(220, 2.0, None)];
        assert_eq!(
            nearest_sample(&samples, 180).expect("sample").timestamp_ms,
            220
        );
    }
}
