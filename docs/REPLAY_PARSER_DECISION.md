# Replay parser decision

## Decision

ValCoach uses one replay architecture with explicit dialects and one stable Rust-facing bundle.

- Primary production backend: `michel-giehl/ValorantReplayParser`, pinned to
  `b51d67423b7b4952d59051cf91e55efa1c42da05`.
- Secondary container/validation oracle: `yakisoba0728/vrfkit`, pinned to
  `a73ee3aab474e38af4de7157fb8d94b34bee0963`.
- Production support is registered by the exact Header `branch`; unknown and China branches never
  fall back to the latest Global transform.
- Third-party output is converted into ValCoach Replay Bundle v1 before persistence or UI use.

## Why ValorantReplayParser remains primary

The current source provides the Global 13.05 payload transform, movement and shot decoding,
branch-based transform registration, and the CLI/NDJSON boundary already used by ValCoach. This is
the shortest verified path to unblock the Global product. The small ValCoach-maintained patch
`patches/valorant_parser_valcoach_profile.patch` adds only a compact export profile: it does not
change packets, transforms, schemas, or field decoding.

The patch samples each character's movement at 10 Hz and omits undecoded or movement-only export
shells from the normalized event stream. It is pinned to the Parser revision above and its SHA-256
is `458058df240d60b2712172f0f4f93329baa2f971e649ec0eebfd3422af5a61b0`.

## Why vrfkit is the secondary oracle

vrfkit supplies a Rust-native, bounds-checked container implementation with lossless raw Event
payload access, ReplayInfo/Header parsing, Checkpoint and ReplayData framing, and strong validation
semantics. ValCoach uses `vrf-container` for the region-independent probe and server timeline. This
lets a China replay retain trustworthy metadata and 239 server events even before its ReplayData
payload transform is known.

It does not replace the primary backend today because its registered payload support stops before
the immediate Global 13.05 requirement, while ValorantReplayParser has a validated 13.05 transform.

## Update and attribution policy

`scripts/setup_parser.ps1` clones the exact primary revision, applies the checked patch, builds on
.NET 10, and optionally runs the full upstream test suite. The Rust dependency pins the secondary
revision directly in Cargo. `crates/vrf_probe/NOTICE.md` records the MIT attribution. Updating either
revision requires rerunning both real 13.05 fixtures and updating the reports; branch names alone
are not proof of binary compatibility.

## Current region decisions

| Dialect | Container | Server timeline | ReplayData payload | Production decision |
|---|---:|---:|---:|---|
| Global 13.05 | PASS | PASS 244/244 | PASS | supported |
| China 13.05 | PASS | PASS 239/239 | no verified transform | container-only / payload unsupported |
| China 13.00 | preserved legacy evidence | historical | Global alias known bad | unsupported |
