# Global 13.05 replay report

## Experiment

- name: Global 13.05 unmodified baseline, compact production export, Bundle validation, and website
  ingestion.

## Completed

- Built the pinned Parser with .NET SDK 10.0.400 and ran all 363 upstream tests successfully.
- Ran the unmodified `log` and `export` commands against the real Global fixture.
- Parsed the common container and every server Event through the Rust probe.
- Added and tested the output-only `valcoach` profile, then validated the complete Bundle.
- Ran the website job through probe, Parser, normalization, SQLite persistence, metrics, and `ready`.

## Commands

```powershell
dotnet build ValorantReplayParser.sln
dotnet test ValorantReplayParser.sln
dotnet run --no-build --project src\CliReader\CliReader.csproj -- log <GLOBAL_13_05_VRF>
dotnet run --no-build --project src\CliReader\CliReader.csproj -- export <GLOBAL_13_05_VRF> --output <BASELINE>
scripts\setup_parser.ps1 -SkipTests
scripts\smoke_global_13_05.ps1
cargo test -p valcoach-server 13_05_job -- --ignored --nocapture
```

## Verified

- Parser SHA: `b51d67423b7b4952d59051cf91e55efa1c42da05`.
- Fixture SHA-256: `276c1c0ba7e6930f9535c71167d52d474aca14a45f2c015b596bfd8c356d5de0`.
- Fixture size: 55,045,079 bytes.
- Branch: `++Ares-Core+release-13.05`; internal replay id:
  `ec22cf8e-b1f4-48b7-8426-c60a20562b3e`; duration: 2,092,106 ms; map:
  `/Game/Maps/Bonsai/Bonsai`.
- Unmodified `log` exit code: 0; unmodified `export` exit code: 0.
- Container: 21 ReplayData + 20 Checkpoint + 244 Event = 285 chunks after Header, 286 including
  Header; unknown: 0.
- Server Events: 244/244 structurally valid, 0 trailing bytes. Counts: 161 deaths, 45 ultimates,
  20 round starts, 14 plants, 2 defuses, 1 explosion, 1 team switch.
- Parser: 630,158 packets; 0 malformed packets; 148 partial errors; 276,091 undecoded export
  groups; 3,212 shot records.
- Unmodified export: 404,235 events and 2,140,825 movement rows; files expanded to
  2,010,915,845 and 1,753,766,779 bytes respectively.
- Compact production export: 138,065 events and 165,047 movement rows; files are 86,147,603 and
  40,013,415 bytes. Movement covers 624..2,092,085 ms; NaN/Infinity count: 0.
- Bundle validator: PASS with exactly 244 server events, 138,065 parser events and 165,047 movement
  rows.
- Website integration: PASS; job reached `ready`, all 303,112 normalized rows matched the manifest,
  and deterministic movement metrics were generated. Test wall time was 43.66 seconds.

## Files changed

- `crates/vrf_probe/`: common container and server Event probe.
- `crates/replay_adapter/src/parser_source.rs`: exact Global routing and compact Parser invocation.
- `apps/server/src/jobs.rs`: probe-first job, Bundle generation, explicit unsupported status.
- `crates/db/src/lib.rs`: WAL/multi-connection file database so status reads do not wait for ingest.
- `patches/valorant_parser_valcoach_profile.patch`: reproducible output-only Parser patch.
- `scripts/smoke_global_13_05.ps1`, summarizer and validator.

## Evidence

Generated evidence is under `artifacts/replay/global_13_05*` and
`artifacts/smoke-global-13.05`; artifacts and replay files are intentionally Git-ignored. The
checked report and scripts contain no replay payload.

## Known limitations

- The Parser reports 148 recoverable partial field errors and many undecoded groups; therefore
  actor, identity, combat, ability and game-state capabilities remain partial or unsupported.
- Compact movement is sampled at 10 Hz per observed character GUID. The 64 observed character
  GUIDs are source entities, not proof of 64 human players.
- Map coordinates remain raw until a separately validated map transform exists.

## Decision

- PASS — Milestone 1 (Global Parser Ready) and the Global half of the unified layer are complete.

## Next

- Keep Global production enabled and capability-gated. Do not wait for China transform research.
