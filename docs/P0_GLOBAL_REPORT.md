# P0-GLOBAL Report: Global Fixture Parser Validation

## Status

**PASS — fixture pipeline validated.**

This gate validates the local Global replay fixture → unmodified C# Parser → NDJSON export path. It does not claim that the short fixture represents a complete personal match, nor that it supports every metric.

## Input and Parser Provenance

| Field | Value |
| --- | --- |
| Fixture | `ValorantReplayParser-main/tests/Test.Integration/Replays/12974d2b-848f-490d-80ba-5f03a033c2d5.13_00.vrf` |
| Fixture size | 431,908 bytes |
| Replay branch | `++Ares-Core+release-13.00` |
| Parsed duration | 39,080 ms |
| Parser source | Existing local `ValorantReplayParser-main` snapshot |
| Parser revision | Unavailable: the local snapshot has no `.git` metadata |
| Upstream freshness | Not verified: this environment cannot reach `github.com:443` |

The Parser source was not modified for this run. The previously documented CN alias remains reverted; see [P0_REPORT.md](P0_REPORT.md).

## Commands and Results

Commands run from `ValorantReplayParser-main`:

```powershell
& 'C:\Program Files\dotnet\dotnet.exe' build 'ValorantReplayParser.sln'
& 'C:\Program Files\dotnet\dotnet.exe' test 'ValorantReplayParser.sln' --no-build
```

Build exited **0** (0 warnings, 0 errors). Tests exited **0**: 351 passed, 0 failed.

The parser smoke run was executed from the workspace:

```powershell
& '.\scripts\smoke_global_fixture.ps1' `
  -ReplayPath '.\ValorantReplayParser-main\tests\Test.Integration\Replays\12974d2b-848f-490d-80ba-5f03a033c2d5.13_00.vrf' `
  -ParserDirectory '.\ValorantReplayParser-main'
```

| Check | Result |
| --- | --- |
| `log` exit code | 0 |
| `export` exit code | 0 |
| `events.ndjson` | 7,801,905 bytes; 6,103 lines |
| `movement.ndjson` | 3,988,766 bytes; 4,946 lines |
| Log warning/error lines | 0 / 0 |
| Parser packet stats | 7,354 packets, 0 malformed packets, 2 partial errors |

## Export Statistics and Sanity Checks

### Event distribution

| Type | Count |
| --- | ---: |
| `rpc_received` | 5,044 |
| `export_group_received` | 961 |
| `actor_spawned` | 84 |
| `actor_closed` | 14 |

There are 84 distinct event actor GUIDs. The export contains 5,044 RPC events, 84 actor spawns, and 14 actor closes. No event type matched the current shot-related filter.

### Movement

| Check | Result |
| --- | ---: |
| Timestamp range | 182–39,080 ms |
| Timestamp regressions in export order | 0 |
| Character GUIDs | 1 |
| X range | 3,682.79–12,715.59 |
| Y range | -1,686.39–974.61 |
| Z range | -367.07–1,106.47 |
| Nonzero position rows | 4,946 / 4,946 |
| NaN/Infinity/missing position rows | 0 |

The parser emitted two partial packet errors in its summary, but no warning/error log lines and no malformed packets. The exported movement is non-empty, monotonic, finite, and spatially varied, so it is suitable for adapter and movement-metric development.

## Capability Conclusion

| Capability | Fixture conclusion |
| --- | --- |
| Movement | Supported |
| Gunplay | Partial / unavailable for this fixture: no shot-related export event was found |
| Abilities | Partial; not an MVP gate |
| Rounds, game state, world state | Do not assume support from this fixture |

## Rust Adapter Validation

The exported bundle was subsequently ingested by `valcoach-replay-adapter`'s `ParsedBundleSource` using asynchronous line-by-line reads. The ignored integration test is intentionally executed only after generating the local P0 artifact:

```powershell
cargo test -p valcoach-replay-adapter --test p0_global_bundle -- --ignored
```

It exited **0** and confirmed the exported counts, timestamp range, and coordinate bounds without depending on C# types or loading the full NDJSON files into a Rust collection. The normal workspace test suite and `cargo clippy --workspace --all-targets -- -D warnings` also exit 0.

## Known Limitations

- This is a 39-second upstream integration fixture, not a complete Global personal replay.
- Upstream Parser freshness cannot be verified until network access to GitHub is restored.
- Full Global replay validation, including player selection, match duration, gunplay, database ingestion time, and richer metrics, remains a later required step.

## Conclusion

P0-GLOBAL is passed for the fixture pipeline. ValCoach development may proceed with a pluggable Rust adapter and `ParsedBundleSource`; all unavailable capabilities must remain explicitly gated.
