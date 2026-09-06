# Replay Semantic Pipeline report

## Completed

- Ingests `server_events.ndjson`, `parser_events.ndjson` and `movement.ndjson` into normalized
  rounds, movement, combat, ability and Spike tables.
- Resolves stable Subject → PlayerState → character pawn mappings and collapses respawns into one
  player. Spawn class observation resolves the former B-side AggroBot/Gekko gap.
- Enriches movement with round, alive state, yaw, pitch and deterministic Split area names.
- Builds question-scoped Agent context with round timelines, area occupancy, selected-player combat,
  ability/Spike events and nearby players at combat timestamps.
- Stores source file, source row/event type, match, round, player and timestamp evidence.
- Writes `semantic_diagnostics.json` and persists the same diagnostics in SQLite.
- Keeps exact provider token accounting; optional configured prices add cost estimates.

## Verified

- Global fixture: 138,065 parser events; 165,047 movement rows; 20 rounds; 161 kills/deaths;
  45 ultimates; 14 plants; 2 defuses; 1 explosion.
- Target Subject `ec3ffefe-e11b-5623-8f56-3c55deef5bc1`: Sova/Hunter, PlayerState 284,
  character pawn 802, 16,762 movement samples, 304 shots, 18 kills and 17 deaths.
- China fixture: 239 server events, 22 rounds and 10 header roster entries imported as a partial
  replay; incompatible ReplayData remains fail-closed.
- Regression commands:

```powershell
cargo test --workspace
cargo test -p valcoach-server --bin valcoach-server jobs::tests::global_13_05_job_reaches_ready_and_persists_a_match_summary -- --ignored --exact
cargo test -p valcoach-server --bin valcoach-server jobs::tests::china_13_05_job_imports_common_timeline_and_roster -- --ignored --exact
cargo clippy --workspace --all-targets -- -D warnings
cd web; npm run build
```

## Known limitations

- The current named-area resolver is Split-specific and deliberately leaves spawn/off-map points
  unresolved.
- Ability semantics currently include reliable ultimate events; ordinary ability taxonomy requires
  more parser-class mappings.
- Round winners are deterministic for defuse/explosion finishes; elimination/time-out winners remain
  unknown until a reliable round-result field is mapped.
- China 13.05 ReplayData cannot be decoded by aliasing the Global transform. Movement, actor-resolved
  combat and abilities remain unavailable until a verified China transform exists.

## Agent impact

The Agent can now answer who the selected player is, which agent they used, round boundaries, where
they moved, shots/damage/headshots/kills/deaths, Spike timing/location, A/B/Mid occupancy, and
cross-round A-defense questions using scoped evidence. It no longer receives only average speed and
path distance.
