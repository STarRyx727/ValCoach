//! Humanized replay time formatting.
//!
//! Converts raw milliseconds into `R8 00:26.1` format for UI and LLM context.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayTime {
    pub absolute_ms: i64,
    pub round_no: Option<u32>,
    pub round_elapsed_ms: Option<i64>,
}

impl ReplayTime {
    pub fn humanized(&self) -> String {
        match (self.round_no, self.round_elapsed_ms) {
            (Some(round), Some(elapsed)) => {
                let secs = elapsed.max(0) as f64 / 1000.0;
                let mins = (secs / 60.0).floor() as u32;
                let rem = secs - (mins as f64 * 60.0);
                format!("R{} {:02}:{:04.1}", round, mins, rem)
            }
            _ => {
                let secs = self.absolute_ms.max(0) as f64 / 1000.0;
                let mins = (secs / 60.0).floor() as u32;
                let rem = secs - (mins as f64 * 60.0);
                format!("{:02}:{:04.1}", mins, rem)
            }
        }
    }

    pub fn short_time(&self) -> String {
        let ms = self.round_elapsed_ms.unwrap_or(self.absolute_ms).max(0);
        let secs = ms as f64 / 1000.0;
        let mins = (secs / 60.0).floor() as u32;
        let rem = secs - (mins as f64 * 60.0);
        format!("{:02}:{:04.1}", mins, rem)
    }
}

pub fn humanize_time(absolute_ms: i64, round_no: Option<u32>, round_start_ms: Option<i64>) -> String {
    let round_elapsed_ms = round_start_ms.map(|start| absolute_ms - start);
    ReplayTime {
        absolute_ms,
        round_no,
        round_elapsed_ms,
    }
    .humanized()
}

pub fn humanize_timestamp(absolute_ms: i64) -> String {
    let secs = absolute_ms.max(0) as f64 / 1000.0;
    let mins = (secs / 60.0).floor() as u32;
    let rem = secs - (mins as f64 * 60.0);
    format!("{:02}:{:04.1}", mins, rem)
}

#[cfg(test)]
mod tests {
    use super::{humanize_time, humanize_timestamp, ReplayTime};

    #[test]
    fn humanized_time_with_round() {
        let t = ReplayTime {
            absolute_ms: 809_771,
            round_no: Some(8),
            round_elapsed_ms: Some(24_800),
        };
        assert_eq!(t.humanized(), "R8 00:24.8");
    }

    #[test]
    fn humanized_time_without_round() {
        let t = ReplayTime {
            absolute_ms: 101_363,
            round_no: None,
            round_elapsed_ms: None,
        };
        assert_eq!(t.humanized(), "01:41.4");
    }

    #[test]
    fn humanize_time_helper() {
        assert_eq!(humanize_time(809_771, Some(8), Some(784_971)), "R8 00:24.8");
        assert_eq!(humanize_timestamp(101_363), "01:41.4");
    }
}
