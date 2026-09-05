use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use valcoach_domain::{ParsedReplay, ReplayCapabilities, ReplayInput};

use crate::{ReplayDataSource, ReplaySourceError};

#[derive(Debug, Default)]
pub struct ChinaVrfSource;

#[async_trait]
impl ReplayDataSource for ChinaVrfSource {
    async fn ingest(
        &self,
        _input: ReplayInput,
        _cancel: CancellationToken,
    ) -> Result<ParsedReplay, ReplaySourceError> {
        Err(ReplaySourceError::UnsupportedInput {
            source_name: self.source_name(),
            reason: "CN replay payload transform is not currently supported".to_owned(),
        })
    }

    fn capabilities(&self) -> ReplayCapabilities {
        ReplayCapabilities::china_container_only()
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

    use crate::{ChinaVrfSource, ReplayDataSource, ReplaySourceError};

    #[tokio::test]
    async fn cn_replays_remain_explicitly_unsupported() {
        let result = ChinaVrfSource
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
            Err(ReplaySourceError::UnsupportedInput { .. })
        ));
    }
}
