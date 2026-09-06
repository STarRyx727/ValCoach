use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tracing::info;
use valcoach_domain::{CapabilityLevel, ParsedReplay, ReplayCapabilities, ReplayInput};

use crate::{ParsedBundleSource, ReplayDataSource, ReplaySourceError};

#[derive(Debug, Clone)]
pub struct ValorantReplayParserSource {
    parser_directory: PathBuf,
    dotnet_path: PathBuf,
    timeout: Duration,
}

impl ValorantReplayParserSource {
    pub fn new(parser_directory: impl Into<PathBuf>, dotnet_path: impl Into<PathBuf>) -> Self {
        let parser_directory = parser_directory.into();
        let parser_directory = std::path::absolute(&parser_directory).unwrap_or(parser_directory);
        Self {
            parser_directory,
            dotnet_path: dotnet_path.into(),
            timeout: Duration::from_secs(300),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn cli_project(&self) -> PathBuf {
        self.parser_directory
            .join("src")
            .join("CliReader")
            .join("CliReader.csproj")
    }

    async fn export(
        &self,
        replay_path: &Path,
        output_directory: &Path,
        cancel: &CancellationToken,
    ) -> Result<(), ReplaySourceError> {
        tokio::fs::create_dir_all(output_directory)
            .await
            .map_err(|source| ReplaySourceError::Read {
                path: output_directory.to_path_buf(),
                source,
            })?;

        let stderr_path = output_directory.join("parser_stderr.log");

        let mut command = Command::new(&self.dotnet_path);
        command
            .current_dir(&self.parser_directory)
            .arg("run")
            .arg("--no-build")
            .arg("--project")
            .arg(self.cli_project())
            .arg("--")
            .arg("export")
            .arg(replay_path)
            .arg("--profile")
            .arg("valcoach")
            .arg("--output")
            .arg(output_directory)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        let mut child = command.spawn().map_err(ReplaySourceError::Spawn)?;
        info!(replay = %replay_path.display(), output = %output_directory.display(), "starting parser export");

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let stderr_task = if let Some(stderr) = stderr {
            let stderr_path = stderr_path.clone();
            Some(tokio::spawn(async move {
                use tokio::io::AsyncReadExt;
                let mut reader = stderr;
                let mut buf = vec![0u8; 8192];
                let file_result = tokio::fs::File::create(&stderr_path).await;
                if let Ok(mut file) = file_result {
                    loop {
                        match reader.read(&mut buf).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => { let _ = file.write_all(&buf[..n]).await; }
                        }
                    }
                }
            }))
        } else {
            None
        };
        if let Some(stdout) = stdout {
            let stdout_path = output_directory.join("parser_stdout.log");
            tokio::spawn(async move {
                use tokio::io::AsyncReadExt;
                let mut reader = stdout;
                let mut buf = vec![0u8; 8192];
                let file_result = tokio::fs::File::create(&stdout_path).await;
                if let Ok(mut file) = file_result {
                    loop {
                        match reader.read(&mut buf).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => { let _ = file.write_all(&buf[..n]).await; }
                        }
                    }
                }
            });
        }

        let status = tokio::select! {
            result = timeout(self.timeout, child.wait()) => match result {
                Ok(Ok(status)) => status,
                Ok(Err(source)) => return Err(ReplaySourceError::Spawn(source)),
                Err(_) => {
                    let _ = child.kill().await;
                    return Err(ReplaySourceError::TimedOut { seconds: self.timeout.as_secs() });
                }
            },
            _ = cancel.cancelled() => {
                let _ = child.kill().await;
                return Err(ReplaySourceError::Cancelled);
            }
        };

        if let Some(task) = stderr_task { let _ = task.await; }

        if status.success() {
            Ok(())
        } else {
            let stderr_content = tokio::fs::read_to_string(&stderr_path)
                .await
                .unwrap_or_default();
            tracing::error!(
                exit_code = ?status.code(),
                stderr = %stderr_content.chars().take(2000).collect::<String>(),
                "parser export failed"
            );
            Err(ReplaySourceError::ParserFailed {
                exit_code: status.code(),
            })
        }
    }
}

#[async_trait]
impl ReplayDataSource for ValorantReplayParserSource {
    async fn ingest(
        &self,
        input: ReplayInput,
        cancel: CancellationToken,
    ) -> Result<ParsedReplay, ReplaySourceError> {
        let ReplayInput::Vrf {
            path,
            region: _,
            output_directory,
        } = input
        else {
            return Err(ReplaySourceError::UnsupportedInput {
                source_name: self.source_name(),
                reason: "ValorantReplayParserSource accepts .vrf inputs only".to_owned(),
            });
        };

        self.export(&path, &output_directory, &cancel).await?;
        ParsedBundleSource
            .ingest(
                ReplayInput::ParsedBundle(valcoach_domain::ParsedBundle {
                    events_path: output_directory.join("events.ndjson"),
                    movement_path: output_directory.join("movement.ndjson"),
                    server_events_path: None,
                }),
                cancel,
            )
            .await
    }

    fn capabilities(&self) -> ReplayCapabilities {
        ReplayCapabilities::global_fixture(CapabilityLevel::Partial)
    }

    fn source_name(&self) -> &'static str {
        "valorant_replay_parser"
    }
}
