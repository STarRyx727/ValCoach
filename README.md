# ValCoach

**Personalized Evidence-Grounded VALORANT Replay Coaching Agent**

ValCoach keeps replay files local, probes every `.vrf` through a common Rust container layer,
converts supported Global 13.05 payloads through a pinned `ValorantReplayParser`, normalizes a
versioned Replay Bundle in Rust, builds an evidence-linked Semantic IR in SQLite, and exposes
question-scoped replay facts to the coaching Agent and local web UI.

## Current MVP

- Argon2id registration/login/logout with session-based ownership checks.
- Header-derived Global/China dialect detection, common server timeline, parser job lifecycle, SSE
  status stream, cancellation, and SQLite transaction/batch ingestion.
- Global 13.05 full-file support (138,065 parser events and 165,047 compact movement samples on the
  checked fixture); file databases use WAL so progress queries stay responsive during ingestion.
- China 13.05 trustworthy partial import: 239/239 server Events, 22 rounds and the 10-player header
  roster reach `ready`; ReplayData movement/combat remains explicitly unavailable and never falls
  back to the incompatible Global transform.
- Semantic rounds, movement (position/aim/alive/area), shots, damage, kills, ultimates and Spike
  events with source-file/row evidence. The Agent retrieves only relevant rounds, including area
  occupancy and nearby-player snapshots, instead of receiving a lossy movement summary or raw dump.
- Match browsing, exact 5v5 team rosters derived from stable player-state identities, agent-based
  player selection, and local account binding. Re-spawned character actors are collapsed into the
  same player instead of appearing as duplicate GUIDs.
- Evidence-grounded coaching with OpenAI Responses, Claude Messages, DeepSeek, or another
  OpenAI-compatible API; conversations and input/output/total token usage are stored locally.
- React/Vite local UI with in-browser model settings, team-separated roster selection, coaching
  history, collapsible evidence/limitations, and token totals. API keys entered in the browser are
  held only in backend process memory and are never returned to the page.

China-region ReplayData remains unsupported because its payload transform differs from the Global
release format; its metadata, roster, rounds and server timeline are still imported. See
`docs/REPLAY_CHINA_13_05_REPORT.md`.

## Run locally

```powershell
scripts\setup_parser.ps1
cargo run -p valcoach-server
cd web
npm install
npm run dev
```

The backend listens on `http://127.0.0.1:3000`; Vite proxies `/api` requests during development.
The setup script checks out Parser commit `b51d674…`, applies the ValCoach compact-output patch,
builds it, and runs its tests. Set `VALCOACH_PARSER_DIR` and `VALCOACH_DOTNET_PATH` if the checkout
or dotnet executable is elsewhere. Replay files, SQLite databases, parser output, API keys, and
Node build artifacts are ignored by Git and are never uploaded by this project.

The Agent is optional. The easiest setup is **模型设置** in the web UI; environment variables from
`.env.example` remain available as a server-wide fallback. Model selection is explicit and cost
estimates are only computed when current per-million-token prices are supplied. See
`docs/AGENT_PROVIDERS.md`.

## Verification

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cd web; npm run build
scripts\smoke_global_13_05.ps1
scripts\smoke_cn_13_05.ps1
cargo test -p valcoach-server 13_05_job -- --ignored --nocapture
```

The ignored tests require the two local fixture directories. They run the real Global file through
probe → Parser → Bundle → Semantic IR → SQLite and verify China reaches a capability-gated partial
`ready` result. Fixture and generated artifact hashes/counts are documented under `docs/`.
