# Global / China 13.05 schema diff

## Status

**NOT RUN.** A meaningful schema comparison requires both fixtures to pass payload transformation
and strict full-file decoding. The minimal China → Global transform alias failed before stable
ReplayData content was available, so comparing its 86 partial event lines with the complete Global
export would create false conclusions.

## What is already comparable

The region-independent container layer is validated in `REPLAY_CHINA_13_05_REPORT.md` and
`REPLAY_GLOBAL_13_05_REPORT.md`. Both fixtures share container version 7, NetworkMagic
`0x2CF5A13D`, network version 19, checksum 4217436668, engine network protocol 32, replay version
5.3.2, UE4 522, UE5 1009, license 80, and LinuxServer platform. Their branch and changelists differ.

## Gate for running this diff

Run `scripts/compare_replay_schemas.py <global-bundle> <china-bundle>` only after a China transform:

- passes fixed golden vectors;
- consumes payload bits exactly under a grammar oracle;
- completes the full China file without catastrophic errors;
- produces movement greater than zero and near-full timeline coverage;
- passes `scripts/validate_replay_bundle.py`.

Until then the production decision is `unsupported`, not inferred schema compatibility.
