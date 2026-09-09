# Replay parser decision

## Decision

ValCoach uses one replay architecture with explicit dialects and one stable Rust-facing bundle.

- Primary production backend: `michel-giehl/ValorantReplayParser`, pinned to
  `b51d67423b7b4952d59051cf91e55efa1c42da05`.
- Secondary container/validation oracle: `yakisoba0728/vrfkit`, pinned to
  `a73ee3aab474e38af4de7157fb8d94b34bee0963`.
- Production support is registered by the exact Header `branch`; unknown branches never
  fall back to the latest Global transform.
- Third-party output is converted into ValCoach Replay Bundle v1 before persistence or UI use.

## Why ValorantReplayParser remains primary

The current source plus the checked ValCoach production patch provides independent Global 13.05
and China 13.05 payload transforms, movement and shot decoding,
branch-based transform registration, and the CLI/NDJSON boundary already used by ValCoach. This is
the shortest verified path to unblock the product. The ValCoach-maintained
`patches/valorant_parser_valcoach_profile.patch` is one atomic production patch: it adds the compact
export profile, independently recovered China transform, exact branch registration, full-fidelity
movement option and parser manifest diagnostics. China is never aliased to Global.

The patch samples each character's movement at 10 Hz and omits undecoded or movement-only export
shells from the normalized event stream. The combined production patch SHA-256 is
`c0705c0650595dc3e776564377080ff05eaa1599bed6dae4249ecf2ab79d868a`.

## Why vrfkit is the secondary oracle

vrfkit supplies a Rust-native, bounds-checked container implementation with lossless raw Event
payload access, ReplayInfo/Header parsing, Checkpoint and ReplayData framing, and strong validation
semantics. ValCoach uses `vrf-container` for the region-independent probe and server timeline. This
provides an independent container and server-timeline validation path for both regions.

It does not replace the primary backend today because its registered payload support stops before
the immediate Global 13.05 requirement, while ValorantReplayParser has a validated 13.05 transform.

## Update and attribution policy

`scripts/setup_parser.ps1` clones the exact primary revision, applies the checked production patch, builds on
.NET 10, and optionally runs the full upstream test suite. The Rust dependency pins the secondary
revision directly in Cargo. `crates/vrf_probe/NOTICE.md` records the MIT attribution. Updating either
revision requires rerunning both real 13.05 fixtures and updating the reports; branch names alone
are not proof of binary compatibility.

## Current region decisions

| Dialect | Container | Server timeline | ReplayData payload | Production decision |
|---|---:|---:|---:|---|
| Global 13.05 | PASS | PASS 244/244 | PASS | supported |
| China 13.05 | PASS | PASS | dedicated China transform PASS | supported |
| China 13.04 and other China branches | container metadata only | version-dependent | no registered transform | explicitly unsupported |
