# China Replay Status

## Status

**Container and server timeline supported; ReplayData payload unsupported**

China 13.05 metadata, exact Header branch, ReplayData/Checkpoint framing, and 239/239 server Events
are validated. The minimal China 13.05 → Global 13.05 payload-transform alias failed with malformed
fields, EndOfArchive errors, zero movement, and an arithmetic overflow, so it was reverted.

The production adapter derives the region from Header `branch`, emits a container-only Replay
Bundle, and terminates the website job as `unsupported`; it never silently falls back to Global.
See `REPLAY_CHINA_13_05_REPORT.md` for current evidence and `P0_REPORT.md` for the preserved China
13.00 regression.
