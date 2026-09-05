//! Replay-source implementations that isolate ValCoach from parser-specific schemas.

mod bundle;
mod china;
mod parser_source;

use std::path::PathBuf;

use async_trait::async_trait;
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use valcoach_domain::{ParsedReplay, ReplayCapabilities, ReplayInput};

pub use bundle::NormalizedRecord;
pub use bundle::ParsedBundleSource;
pub use china::ChinaVrfSource;
pub use parser_source::ValorantReplayParserSource;

#[async_trait]
pub trait ReplayDataSource: Send + Sync {
    async fn ingest(
        &self,
        input: ReplayInput,
        cancel: CancellationToken,
    ) -> Result<ParsedReplay, ReplaySourceError>;

    fn capabilities(&self) -> ReplayCapabilities;

    fn source_name(&self) -> &'static str;
}

#[derive(Debug, Error)]
pub enum ReplaySourceError {
    #[error("replay input is not supported by {source_name}: {reason}")]
    UnsupportedInput {
        source_name: &'static str,
        reason: String,
    },
    #[error("unsupported replay branch: {branch}")]
    UnsupportedBranch { branch: String },
    #[error("no verified payload transform is available for replay branch: {branch}")]
    UnsupportedTransform { branch: String },
    #[error("replay parsing was cancelled")]
    Cancelled,
    #[error("failed to read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid {kind} NDJSON record at {path}:{line}: {reason}")]
    InvalidNdjson {
        kind: &'static str,
        path: PathBuf,
        line: u64,
        reason: String,
    },
    #[error("parser executable could not be started: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("parser process timed out after {seconds} seconds")]
    TimedOut { seconds: u64 },
    #[error("parser process failed with exit code {exit_code:?}")]
    ParserFailed { exit_code: Option<i32> },
}
