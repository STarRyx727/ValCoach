use async_trait::async_trait;
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use valcoach_domain::{ParsedReplay, ReplayCapabilities, ReplayInput, ReplayRegion};

use crate::{ReplayDataSource, ReplaySourceError, ValorantReplayParserSource};

pub const CHINA_13_05_BRANCH: &str = "++Ares-Core+release-china-13.05";
const CHINA_13_05_TRANSFORM: &str = "china-13.05";

#[derive(Debug, Clone)]
pub struct ChinaVrfSource {
    parser_source: ValorantReplayParserSource,
    branch: String,
}

impl ChinaVrfSource {
    pub fn new(parser_source: ValorantReplayParserSource, branch: impl Into<String>) -> Self {
        Self {
            parser_source,
            branch: branch.into(),
        }
    }

    async fn validate_manifest(
        output_directory: &std::path::Path,
    ) -> Result<(), ReplaySourceError> {
        let path = output_directory.join("manifest.json");
        let bytes = tokio::fs::read(&path)
            .await
            .map_err(|source| ReplaySourceError::Read {
                path: path.clone(),
                source,
            })?;
        let manifest: Value = serde_json::from_slice(&bytes).map_err(|error| {
            ReplaySourceError::InvalidParserManifest {
                path: path.clone(),
                reason: error.to_string(),
            }
        })?;
        let branch = manifest["replay_build"].as_str().unwrap_or_default();
        let transform = manifest["transform_id"].as_str().unwrap_or_default();
        let replay_data_chunks = manifest["pipeline"]["replay_data_chunk_count"]
            .as_u64()
            .unwrap_or_default();
        let demo_frames = manifest["pipeline"]["demo_frame_count"]
            .as_u64()
            .unwrap_or_default();
        let movement_records = manifest["counts"]["movement_records"]
            .as_u64()
            .unwrap_or_default();
        let movement_errors = manifest["counts"]["movement_decode_errors"]
            .as_u64()
            .unwrap_or(u64::MAX);
        let malformed_payloads = manifest["pipeline"]["malformed_payload_count"]
            .as_u64()
            .unwrap_or(u64::MAX);
        let shots = manifest["counts"]["shots"].as_u64().unwrap_or_default();
        let abilities = manifest["counts"]["abilities"].as_u64().unwrap_or_default();
        let combat = manifest["counts"]["combat"].as_u64().unwrap_or_default();
        if branch != CHINA_13_05_BRANCH
            || transform != CHINA_13_05_TRANSFORM
            || replay_data_chunks == 0
            || demo_frames == 0
            || movement_records == 0
            || movement_errors != 0
            || malformed_payloads != 0
            || shots == 0
            || abilities == 0
            || combat == 0
        {
            return Err(ReplaySourceError::InvalidParserManifest {
                path,
                reason: format!(
                    "China 13.05 integrity check failed: branch='{branch}', transform='{transform}', replay_data_chunks={replay_data_chunks}, demo_frames={demo_frames}, movement_records={movement_records}, movement_errors={movement_errors}, malformed_payloads={malformed_payloads}, shots={shots}, abilities={abilities}, combat={combat}"
                ),
            });
        }
        Ok(())
    }
}

#[async_trait]
impl ReplayDataSource for ChinaVrfSource {
    async fn ingest(
        &self,
        input: ReplayInput,
        cancel: CancellationToken,
    ) -> Result<ParsedReplay, ReplaySourceError> {
        if self.branch != CHINA_13_05_BRANCH {
            return Err(ReplaySourceError::UnsupportedTransform {
                branch: self.branch.clone(),
            });
        }
        let ReplayInput::Vrf {
            path,
            region,
            output_directory,
        } = input
        else {
            return Err(ReplaySourceError::UnsupportedInput {
                source_name: self.source_name(),
                reason: "ChinaVrfSource accepts .vrf inputs only".to_owned(),
            });
        };
        if region != ReplayRegion::China {
            return Err(ReplaySourceError::UnsupportedInput {
                source_name: self.source_name(),
                reason: "ChinaVrfSource requires a replay probed as China".to_owned(),
            });
        }

        let mut replay = self
            .parser_source
            .ingest(
                ReplayInput::Vrf {
                    path,
                    region,
                    output_directory: output_directory.clone(),
                },
                cancel,
            )
            .await?;
        Self::validate_manifest(&output_directory).await?;
        replay.capabilities = ReplayCapabilities::china_13_05();
        replay.source_name = "valorant_replay_parser_china_13_05".to_owned();
        Ok(replay)
    }

    fn capabilities(&self) -> ReplayCapabilities {
        if self.branch == CHINA_13_05_BRANCH {
            ReplayCapabilities::china_13_05()
        } else {
            ReplayCapabilities::unknown_branch()
        }
    }

    fn source_name(&self) -> &'static str {
        "china_vrf"
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tokio_util::sync::CancellationToken;
    use valcoach_domain::{ReplayInput, ReplayRegion};

    use crate::{ChinaVrfSource, ReplayDataSource, ReplaySourceError, ValorantReplayParserSource};

    fn source(branch: &str) -> ChinaVrfSource {
        ChinaVrfSource::new(
            ValorantReplayParserSource::new("missing-parser", "missing-dotnet"),
            branch,
        )
    }

    #[tokio::test]
    async fn unknown_china_branch_is_rejected_before_parser_launch() {
        let result = source("++Ares-Core+release-china-13.04")
            .ingest(
                ReplayInput::Vrf {
                    path: PathBuf::from("cn.vrf"),
                    region: ReplayRegion::China,
                    output_directory: PathBuf::from("output"),
                },
                CancellationToken::new(),
            )
            .await;

        assert!(matches!(
            result,
            Err(ReplaySourceError::UnsupportedTransform { branch })
                if branch == "++Ares-Core+release-china-13.04"
        ));
    }

    #[tokio::test]
    async fn non_china_input_is_rejected_before_parser_launch() {
        let result = source(super::CHINA_13_05_BRANCH)
            .ingest(
                ReplayInput::Vrf {
                    path: PathBuf::from("global.vrf"),
                    region: ReplayRegion::Global,
                    output_directory: PathBuf::from("output"),
                },
                CancellationToken::new(),
            )
            .await;

        assert!(matches!(
            result,
            Err(ReplaySourceError::UnsupportedInput { .. })
        ));
    }
}
