# China 13.05 replay report

## Experiment

- name: China 13.05 unmodified baseline, common container probe, and minimal Global-transform alias.

## Completed

- Parsed ReplayInfo, Header, chunk framing, Checkpoint metadata, ReplayData metadata, and all server
  Events without a China payload transform.
- Captured the unmodified Parser failure.
- Applied only an explicit China 13.05 → Global 13.05 transform registration in an isolated
  experiment, ran the full file, judged it against strict criteria, and reverted it from production.
- Added website behavior that terminates as `unsupported` while retaining the valid Bundle evidence.

## Commands

```powershell
cargo run -p valcoach-vrf-probe -- <CHINA_13_05_VRF> <OUTPUT>
dotnet run --no-build --project src\CliReader\CliReader.csproj -- log <CHINA_13_05_VRF>
dotnet run --no-build --project src\CliReader\CliReader.csproj -- export <CHINA_13_05_VRF> --output <OUTPUT>
git apply patches\valorant_parser_cn_13_05_alias.patch  # experiment only
scripts\smoke_cn_13_05.ps1
cargo test -p valcoach-server 13_05_job -- --ignored --nocapture
```

## Verified

- Parser SHA: `b51d67423b7b4952d59051cf91e55efa1c42da05`.
- Fixture SHA-256: `f3169bd7a1b9ee63c8222a0c6d691f1653d639d15bb01e5bd9f14a2633804ce0`.
- Fixture size: 59,628,611 bytes.
- Branch: `++Ares-Core+release-china-13.05`; internal replay id:
  `0d7e68dd-1563-4f12-ba54-1afdf5f99916`; duration: 2,189,235 ms; map:
  `/Game/Maps/Ascent/Ascent`.
- Common container: 23 ReplayData + 22 Checkpoint + 239 Event = 284 chunks after Header, 285
  including Header; unknown: 0.
- Server Events: 239/239 structurally valid and time-consistent, with zero Event, Checkpoint, or
  ReplayData metadata trailing bytes. Counts: 169 deaths, 33 ultimates, 22 round starts, 10 plants,
  1 defuse, 3 explosions, 1 team switch.
- Unmodified baseline exit codes: `log=1`, `export=1`. Exact reason: no payload transform is
  registered for `++Ares-Core+release-china-13.05`.
- Alias experiment exit codes: `log=1`, `export=1`. It emitted only 86 partial event lines
  (35,058 bytes), zero movement, repeated EndOfArchive/malformed field errors, then terminated with
  `OverflowException` in `FieldPayloadParser.ParseProperty`.
- Production website test: PASS. The job reached `unsupported` in about two seconds, error detail
  retained the exact branch, and Bundle validation passed with 239 server Events.

## Files changed

- `patches/valorant_parser_cn_13_05_alias.patch`: preserved experiment only; not applied by setup.
- `crates/vrf_probe/`: common container/Event implementation.
- `apps/server/src/jobs.rs`: Header-derived region and no-silent-fallback behavior.
- `scripts/smoke_cn_13_05.ps1`: reproducible container-level acceptance test.

## Evidence

Raw logs are under `artifacts/replay/china_13_05_baseline`,
`artifacts/replay/china_13_05_alias`, and `artifacts/smoke-china-13.05`. These generated artifacts
and the fixture are Git-ignored. The legacy China 13.00 failure remains in `docs/P0_REPORT.md`.

## Known limitations

- ReplayData content, movement, actors, player identity, gunplay and abilities are unsupported.
- Server Event timeline supports reliable event type/time counts, but does not by itself resolve
  player identities or full round state.
- A valid China transform requires bounded, grammar-validated reverse engineering and a full-file
  validation harness; changing constants until output merely looks plausible is prohibited.

## Decision

- ALIAS FAILED.
- PASS — Milestone 2 (China Container Ready).
- BLOCKED — Milestone 3 (China ReplayData Ready), pending a verified China 13.05 payload transform.

## Next

- Continue China transform work only as a separate evidence-driven research track. Production stays
  exact-branch and container-only for China 13.05.
