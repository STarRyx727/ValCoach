# Current Pipeline Audit

## Data Flow

```
.vrf upload
  ↓
vrf_probe: container + server_events.ndjson
  ↓
C# ValorantReplayParser: events.ndjson + movement.ndjson
  ↓
Rust ParsedBundleSource: streams NDJSON → GenericEvent / MovementSample
  ↓
DB SemanticBuilder: observes events → rounds/combat/spike/abilities
  ↓
DB insert: events, movement_samples, players, rounds, combat_events,
           spike_events, ability_events, shots, semantic_diagnostics
  ↓
metrics: summarize_movement (path distance, velocity)
  ↓
Agent context: build_semantic_coaching_context
  → relevant_rounds (movement_area_timeline, combat, abilities, spike, nearby_players)
  → movement_summary metrics
  ↓
LLM
```

## What Exists and Works

| Component | Status | Location |
|-----------|--------|----------|
| PlayerResolver | ✅ Working | `db/src/lib.rs:ReplayRoster` — collapses respawns, 5v5 teams, subject→state→pawn→agent |
| RoundBuilder | ✅ Working | `db/src/semantic.rs:SemanticBuilder` — roundStarted→start, MulticastEndRound→end, switchTeams→side, ClientBuyPhaseEnd→buy_end |
| CombatBuilder | ✅ Working | `db/src/semantic.rs` — MulticastNotifyDamage → damage/killed/weapon/hit_region, valorant_shot_received → shots, characterDeath → kills |
| SpikeBuilder | ✅ Working | `db/src/semantic.rs` — spikePlanted/Defused/Exploded + TimedBomb position |
| MapAreaResolver (Split only) | ✅ Working | `db/src/semantic.rs:split_area` — hardcoded box zones for Split |
| Movement enrichment | ✅ Working | `db/src/semantic.rs:enrich_movement` — round_no, alive, area, source_row |
| yaw/pitch persistence | ✅ Working | C# exports yaw/pitch → Rust reads → DB stores → Agent context includes |
| Agent retrieval tools | ✅ Working | `db/src/lib.rs:build_semantic_coaching_context` — get_rounds, find_rounds_by_area, get_player_movement, get_combat_events, get_ability_events, get_spike_events, get_area_occupancy, get_nearby_players |
| Evidence refs | ✅ Working | Every semantic event has EvidenceRef with match_id/round/timestamp/player/source_file/source_row |

## What's Missing or Incomplete

### 1. Humanized Time (Phase 2)
- **Current**: all timestamps are raw milliseconds (e.g. `809771 ms`)
- **Needed**: `R8 00:26.1` format for UI and LLM context
- **Impact**: LLM sees raw ms, user sees raw ms

### 2. Multi-map Support (Phase 3-4)
- **Current**: only Split (`/Game/Maps/Bonsai/Bonsai`) has area calibration
- **Needed**: Ascent, Bind, Haven, Sunset, Lotus, Pearl, Fracture, Breeze, Icebox, Abyss, Corrode
- **Impact**: non-Split replays have `area = NULL` → area-based retrieval fails

### 3. Valorant-API Integration (Phase 3)
- **Current**: no map metadata, no minimap images, no callouts
- **Needed**: fetch from `https://valorant-api.com/v1/maps`, cache locally
- **Impact**: no 2D replay viewer, no minimap in UI

### 4. Movement Compaction (Phase 8)
- **Current**: `get_player_movement` returns area-change waypoints (every area change or 5s gap)
- **Needed**: segment-based compaction (route, holding, combat-linked)
- **Impact**: LLM context is larger than necessary

### 5. Shot Compaction (Phase 8)
- **Current**: each shot is a separate combat_event
- **Needed**: merge into bursts (4 shots → 1 hit → kill)
- **Impact**: 304 individual shots inflate context

### 6. Deterministic Compact Replay (Phase 8)
- **Current**: no compact layer; Agent context is built on-the-fly from DB
- **Needed**: pre-compiled per-round compact JSON
- **Impact**: redundant computation per question

### 7. Raw Evidence Store (Phase 9)
- **Current**: raw events in `events` table, movement in `movement_samples` table
- **Needed**: indexed `inspect_raw_evidence(round, time_window, event_types)` tool
- **Impact**: Agent can't drill down to specific time windows

### 8. Personalized Memory (Phase 12)
- **Current**: only chat history (conversations + messages)
- **Needed**: player_profiles, issues, issue_occurrences tables
- **Impact**: no cross-match trend analysis

### 9. 2D Replay UI (Phase 13)
- **Current**: text-only match detail + chat
- **Needed**: minimap canvas with player positions, trajectories, spike, events
- **Impact**: no visual replay review

### 10. Token Statistics (Phase 10)
- **Current**: LLM usage tracked (input/output/total tokens, cost)
- **Needed**: raw replay size vs semantic size vs compact size vs question context size
- **Impact**: can't measure compaction effectiveness

## Fields Lost Between Layers

| Field | C# Export | Rust Adapter | DB | Agent Context |
|-------|----------|-------------|-----|---------------|
| yaw | ✅ `movement.Move.Yaw` | ✅ `raw.get("yaw")` | ✅ column | ✅ `"yaw": row.4` |
| pitch | ✅ `movement.Move.Pitch` | ✅ `raw.get("pitch")` | ✅ column | ✅ `"pitch": row.5` |
| velocity | ✅ `movement.Move.Velocity` | ✅ `raw.get("velocity")` | ✅ columns | ❌ not in movement timeline |
| shot rotation (yaw/pitch) | ✅ `shot.Rotation.Yaw/Pitch` | ❌ not parsed | ❌ | ❌ |
| attack_vectors | ✅ `shot.attack_vectors` | ❌ not parsed | ❌ | ❌ |
| ammo_remaining | ✅ `shot.ammo_remaining` | ❌ not parsed | ❌ | ❌ |
| DamageDirection | ✅ in payload | ❌ not parsed | ❌ | ❌ |
| DamageImpactLocation | ✅ in payload | ❌ not parsed | ❌ | ❌ |
| AssistsList | ✅ in payload | ❌ not parsed | ❌ | ❌ |

## Data Consistency Check

- movement sample count: 16,762 in DB (from Global fixture) — matches parser output
- The 16,634 vs 16,762 discrepancy mentioned in ForCodex doc: resolved — the difference comes from the adapter reading `shooter_character_net_guid` which maps some samples to a different player actor. The `character_net_guid` mapping in the roster finalizer accounts for respawns.

## Token Impact

Current Agent context for a single question:
- match metadata + capabilities: ~200 tokens
- movement summary metrics: ~100 tokens
- semantic context (8 rounds × movement+combat+abilities+spike): ~5,000-8,000 tokens
- evidence refs: ~500 tokens
- Total: ~6,000-9,000 tokens

This is already reasonable for most questions. The main token waste is:
- Each shot is a separate event (304 shots → ~2,000 tokens for one player)
- Movement timeline includes raw coordinates in evidence

## Priorities for Immediate Implementation

1. **Humanized time** — low effort, high impact on LLM output quality
2. **Multi-map area resolver** — fetch Valorant-API callouts, implement coordinate transform
3. **Shot compaction** — merge shots into bursts in SemanticBuilder
4. **Compact replay** — pre-compile per-round JSON
