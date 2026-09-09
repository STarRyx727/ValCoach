# ValCoach Replay Bundle v1

The Bundle is the region-independent boundary consumed by Rust persistence, metrics, UI, and future
Agent tools. Third-party Parser object graphs are never the public contract.

## Layout

```text
bundle/
├─ manifest.json
├─ probe.json
├─ server_events.ndjson
├─ parser_events.ndjson     # when payload backend is complete
├─ movement.ndjson          # when movement is complete
└─ diagnostics.json         # raw Parser manifest when payload backend ran
```

Future optional files (`actors.ndjson`, `players.json`, `rounds.json`) may be added without changing
the meaning of existing files. Every present file must be declared in `manifest.artifacts`.

## Manifest

`schema_version` is `1`. `source` contains the original upload filename, SHA-256 and byte size.
`replay` contains the internal replay id, Header-derived region/branch, changelists, version,
duration, map and network versions. File names are never used as replay identity or region evidence.

`backend` declares the primary Parser name, pinned revision, exact dialect, terminal status and
detail. `validation_backends` records the common container oracle. Current dialects are
`global-13.05`, `china-13.05`, and `unknown`; the two exact 13.05 dialects have complete payload backends.

`capabilities` uses `complete`, `partial`, or `unsupported` for metadata, container, server events,
movement, actors, player identity, gunplay, combat, abilities, economy, spike state, rounds, game
state, world state and checkpoints. Checkpoint `partial` currently means metadata-only. Consumers
must query these values before answering or computing; unavailable values must never be filled with
zero.

`records` contains counts for server events, normalized parser events, product movement samples,
decoded movement records/RPCs, shots and dynamic entities.
`integrity` includes malformed packets, partial errors, undecoded groups, server timeline coverage,
valid/invalid server Event payloads, movement decode errors/leftover bits and trailing-byte counts. Parser-specific values are `null`
when the payload backend did not run.

## NDJSON records

- `server_events.ndjson`: lossless common Event records with schema version, replay id, Event id,
  group, two timestamps, metadata, decoded known payload fields, raw payload hex, trailing-byte
  count, structural validity and time consistency.
- `parser_events.ndjson`: stable generic records requiring `type` and integer `time_ms`; raw source
  GUID identities remain distinct.
- `movement.ndjson`: `remote_character_movement` records requiring `time_ms`, raw identity fields,
  position and optional velocity. Production output is sampled at 10 Hz per primary player GUID;
  dynamic character/ability entities stay out of the product stream.
- `movement_full.ndjson.gz`: optional parser-only audit artifact containing every decoded movement
  record, including dynamic entities; it is not persisted to ValCoach SQLite.
- `diagnostics.json`: third-party diagnostics retained for audit, not a stable application schema.

## Validation

Bundle validation is implemented in the Rust replay adapter. It checks declared artifacts, required
fields, timestamps, finite numbers and record structure while ingesting the data. A complete 13.05
production Bundle additionally requires the exact region transform and zero malformed packets. Recoverable partial errors and
undecoded groups remain visible and lower the relevant capabilities.
