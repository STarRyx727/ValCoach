use std::path::PathBuf;

use tokio_util::sync::CancellationToken;
use valcoach_domain::{ParsedBundle, ReplayInput};
use valcoach_replay_adapter::{ParsedBundleSource, ReplayDataSource};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("workspace root")
        .to_path_buf()
}

#[tokio::test]
#[ignore = "requires artifacts/p0-global created by scripts/smoke_global_fixture.ps1"]
async fn p0_global_export_normalizes_as_a_streaming_bundle() {
    let output = workspace_root().join("artifacts").join("p0-global");
    let result = ParsedBundleSource
        .ingest(
            ReplayInput::ParsedBundle(ParsedBundle {
                events_path: output.join("events.ndjson"),
                movement_path: output.join("movement.ndjson"),
            }),
            CancellationToken::new(),
        )
        .await
        .expect("P0-GLOBAL bundle must be valid");

    assert_eq!(result.summary.event_count, 6_103);
    assert_eq!(result.summary.movement_count, 4_946);
    assert_eq!(result.summary.movement_timestamps.min_ms, Some(182));
    assert_eq!(result.summary.movement_timestamps.max_ms, Some(39_080));
    assert_eq!(
        result
            .summary
            .movement_bounds
            .min
            .expect("movement minimum")
            .x,
        3_682.79
    );
    assert_eq!(
        result
            .summary
            .movement_bounds
            .max
            .expect("movement maximum")
            .z,
        1_106.47
    );
}
