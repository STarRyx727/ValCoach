# ValCoach evaluation baseline

## A. Parser and adapter

The production fixture gate is the real Global 13.05 replay recorded in
`docs/REPLAY_GLOBAL_13_05_REPORT.md`. The checked integration test runs the common probe and pinned
C# Parser, validates Replay Bundle v1, and verifies transactional persistence of 138,065 event
records and 165,047 movement records. The China 13.05 gate validates 239 server Events and expects
an explicit payload `unsupported` result. The CN 13.00 regression remains preserved; neither branch
falls back to a Global transform.

## B. Deterministic metrics

Movement V1 is assessed from observed samples only:

- path distance is the sum of adjacent raw-coordinate positions ordered by timestamp;
- velocity summary is computed only when velocity components are present;
- each summary carries first/last timestamp evidence;
- all coordinate units are labelled raw until a validated map transform exists.

Unit tests cover ordering, path distance, velocity averaging, missing velocity (`partial`, not
zero), and nearest-observed-sample selection without interpolation. Gunplay, team spacing, rounds,
death context and trading stay unavailable/partial unless a future complete replay exposes the
needed trustworthy data.

## C. Agent evaluation

Offline tests cover OpenAI Responses, Anthropic Messages and OpenAI-compatible/DeepSeek response
shapes, token extraction, optional cost arithmetic, a loopback OpenAI request, grounded context,
conversation persistence and user usage totals. No real provider call or API token is needed for
the test suite.

For model-quality evaluation, compare a generic prompt against ValCoach grounded answers on the
same player questions. Score factual claims for valid Match/Timestamp evidence, clearly separated
observations versus recommendations, and an explicit limitation whenever the required capability
is unavailable. Never score a fabricated fixture statistic as successful analysis. The response
must expose its EvidenceRef and limitations so human review does not rely on prose alone.
