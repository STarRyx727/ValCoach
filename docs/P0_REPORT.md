# P0 Report: China Replay Compatibility

## Status

**FAIL — stop all development that depends on full Replay parsing.**

The original parser rejects the CN branch as unsupported. A single, reversible alias from CN 13.00 to the existing global 13.00 transform was then tested. It produces malformed payloads, no movement data, and a terminal parsing overflow. Per the project gate, no transform algorithm/seed/bit-level modification was attempted; the alias has been reverted.

## Test Input

| Field | Value |
| --- | --- |
| Replay | `Demos/ab3410f3-7f39-4131-befb-c3fc52000c91.vrf` |
| Size | 43,569,598 bytes |
| Branch | `++Ares-Core+release-china-13.00` |
| Map | `/Game/Maps/Ascent/Ascent` |

The branch and map were found directly in the binary with `rg -a`; no replay data was invented.

## Environment

| Tool | Result |
| --- | --- |
| Rust | `rustc 1.97.1`, `cargo 1.97.1` |
| .NET | SDK `10.0.400` at `C:\Program Files\dotnet\dotnet.exe` (not on `PATH`) |
| Node / npm | `v24.19.0` / `11.17.0` |
| Git | `2.53.0.windows.3` |

## Parser Provenance

The provided `ValorantReplayParser-main` directory is a source snapshot without `.git` metadata, so its tested upstream commit SHA cannot be truthfully reported. Attempting to clone the required official repository failed because this environment could not connect to `github.com:443`.

Snapshot fingerprints retained for reproducibility:

| File | SHA-256 |
| --- | --- |
| `ValorantReplayParser.sln` | `6F96B9A4F6B9E3E9D0EE8EE4A75194ABCD548C33CD566FBB00A835AC864B7F27` |
| unmodified `ValorantSeededTransform13_00.cs` | `E35C6055283053A80C42C6A5C7563C240E4217961C28FAE2324CF8EC04CBDAC7` |

`scripts/setup_parser.ps1` clones and records a real SHA when network access is available. It is not used to claim that this snapshot is current `main`.

## Original Baseline (No Parser Modification)

Commands run from `ValorantReplayParser-main`:

```powershell
& 'C:\Program Files\dotnet\dotnet.exe' build 'ValorantReplayParser.sln'
& 'C:\Program Files\dotnet\dotnet.exe' test 'ValorantReplayParser.sln' --no-build
```

Results: build exit code **0** with 0 warnings/errors; test exit code **0**, 351 passed / 0 failed.

Replay commands, invoked through `scripts/smoke_cn_replay.ps1`:

```powershell
dotnet run --project 'src\CliReader\CliReader.csproj' -- log '<replay>'
dotnet run --project 'src\CliReader\CliReader.csproj' -- export '<replay>' --output 'artifacts\p0\original'
```

Results: `log` exit code **1**; `export` exit code **1**; both NDJSON files were created but are 0 bytes.

Exact blocking error:

```text
InvalidReplayInfoException: Unsupported VALORANT replay version:
no payload transform is registered for replay branch
'++Ares-Core+release-china-13.00'.
```

Full stdout/stderr are retained locally under `artifacts/p0/original/` and are ignored by Git.

## Minimal Alias Experiment

The original error met the sole condition for a compatibility alias. The experiment added only the CN branch string to `ValorantSeededTransform13_00.SupportedReplayVersions`; it did not alter the transform class's registration, seed constants, transform implementation, or bit logic. The exact reversible change is retained as [valorant_parser_cn_13_00_alias.patch](../patches/valorant_parser_cn_13_00_alias.patch).

After rebuilding (exit code **0**, 0 warnings/errors) and rerunning all tests (351 passed / 0 failed), the same replay commands returned:

| Check | Result |
| --- | --- |
| `log` exit code | 1 |
| `export` exit code | 1 |
| `events.ndjson` | 38,261 bytes; 91 rows (69 `actor_spawned`, 22 `export_group_received`) |
| `movement.ndjson` | 0 bytes |
| Parser warnings | 56 malformed-payload warnings |
| Parser errors | 32 content-block/parser errors |

The partial event file is startup metadata only and is not usable for metric computation. The exact terminal failure is:

```text
InvalidReplayDataException: Error while parsing replay-data packet stream:
Arithmetic operation resulted in an overflow.
```

The inner exception occurs in `FieldPayloadParser.ParseProperty` (line 176), reached through `ContentBlockFramer.FrameRepLayoutContentBlock`. Prior log evidence includes impossible field sizes, `EndOfArchive`, and packed integers that do not terminate within five bytes. This is the deeper payload-decoding failure specified by the P0 FAIL condition, not a recoverable missing registration.

Full stdout/stderr and machine-readable exit-code records are retained locally under `artifacts/p0/cn_13_00_alias/` and are ignored by Git.

## Decision and Minimal Reproduction

The alias was reverted. The current source again registers only `++Ares-Core+release-13.00` for the 13.00 transform.

```powershell
Set-Location 'D:\all programs\Summer_semester_of_freshman_year\rust\AIAgent'
& '.\scripts\smoke_cn_replay.ps1' `
  -ReplayPath '.\Demos\ab3410f3-7f39-4131-befb-c3fc52000c91.vrf' `
  -ParserDirectory '.\ValorantReplayParser-main' `
  -OutputDirectory '.\artifacts\p0\cn_13_00_alias'
```

Do not apply the alias for production use: the command is included solely to reproduce this documented failed experiment. P0 remains failed until a parser version/verified transform that produces a successful non-empty `events.ndjson` and `movement.ndjson` is available.
