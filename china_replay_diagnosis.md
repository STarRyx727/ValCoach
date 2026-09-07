# VALORANT China replay payload transform diagnosis

Date: 2026-09-07

## Result

The China replay failure is localized to layer H, the property/RPC payload transform. The replay
container, Oodle decompression, playback packets, raw packets, bunch framing, actor channels, and
content-block framing all remain structurally healthy. Applying the matching Global transform is
not a solution: all five China fixtures reach EOF only in the isolated diagnostic mode, but produce
zero movement and tens of thousands of malformed payloads.

China 13.05 `TransformUInt64` differs before PRNG state advancement. The first 64-bit block uses
`state = seed`, so this conclusion is independent of `SeedAddend`, `InitialPrngA`, and later PRNG
state transitions. Constant-only brute force must not be resumed.

The real China 13.05 and China 13.04 transforms are **not recovered yet**. The temporary alias is
opt-in and is rejected by the default registry. ValCoach therefore continues to report China
gameplay payloads as unsupported instead of returning corrupt coaching data.

## Fixtures and metadata

All source `.vrf` files were treated as read-only. File identities are recorded in
`diagnostics/*_baseline.json`.

| ID | Region / patch | Map | Bytes | Duration ms | ReplayData | Events | Network checksum |
|---|---|---:|---:|---:|---:|---:|---:|
| `ec22cf8e` | Global 13.05 | Bonsai | 55,045,079 | 2,092,106 | 21 | 244 | `0xFB60F9FC` |
| `0d7e68dd` | China 13.05 | Ascent | 59,628,611 | 2,189,235 | 23 | 239 | `0xFB60F9FC` |
| `65f18d47` | China 13.05 | Bonsai | 47,501,224 | 1,775,791 | 19 | 204 | `0xFB60F9FC` |
| `a00e078e` | China 13.05 | Bonsai | 86,451,429 | 2,963,376 | 30 | 360 | `0xFB60F9FC` |
| `ec9a105c` | China 13.05 | Triad | 62,928,536 | 2,341,094 | 24 | 269 | `0xFB60F9FC` |
| `5570b968` | China 13.04 | Pitt | 24,786,081 | 887,721 | 10 | 125 | see baseline JSON |

Every outer event payload in the six fixtures is structurally valid. Header, event, checkpoint,
and ReplayData trailing-byte counts are zero.

## Global 13.05 baseline

The unmodified Global transform completed normally.

| Metric | Value |
|---|---:|
| Replay chunks / ReplayData chunks | 286 / 21 |
| Playback packets / raw packets | 630,158 / 630,158 |
| Bunches | 630,158 |
| ActorChannel opens | 2,442 |
| ContentBlocks | 730,249 |
| Resolved export groups | 539 |
| Malformed packets / payloads | 0 / 0 |
| Partial errors | 148 |
| Movement | 165,047 |
| RPC | 4,764 |
| Valorant shots | 3,212 |

The first 1,000 transform samples contain zero exceptions, all 1,000 payloads are fully consumed,
and 222 samples decode at least one bound field.

## Unmodified China baseline

Without diagnostic flags, the parser rejects every China fixture before full gameplay parsing with:

`no payload transform is registered for replay branch '++Ares-Core+release-china-13.xx'.`

This behavior is retained. No catch or fallback converts China into a Global replay.

## Temporary Global alias experiment

The alias runs only with `--diagnose-china-alias`; diagnostic mode isolates each malformed content
block so the experiment can continue to EOF. Counts after H are not trustworthy gameplay counts.
In particular, nonzero RPC/shot-shaped records under the wrong transform are false positives;
movement remains the stronger health signal.

| ID | Raw packets | Actor opens | ContentBlocks | Malformed payloads | Movement | RPC-shaped | Shot-shaped |
|---|---:|---:|---:|---:|---:|---:|---:|
| `0d7e68dd` | 675,925 | 2,460 | 656,237 | 110,203 | 0 | 4,418 | 64 |
| `65f18d47` | 547,044 | 1,927 | 533,039 | 85,782 | 0 | 3,655 | 39 |
| `a00e078e` | 895,514 | 3,729 | 868,411 | 146,029 | 0 | 4,331 | 57 |
| `ec9a105c` | 676,379 | 2,367 | 664,669 | 102,569 | 0 | 4,022 | 53 |
| `5570b968` | 278,380 | 1,315 | 296,800 | 17,660 | 0 | 914 | 597 |

Each replay has 1,000 bounded JSONL samples. Across the four China 13.05 files, only 0–4 of the
first 1,000 samples decode a bound field, compared with 222/1,000 for Global. The sample exception
rate is 4.5%–15.9%; the full-stream malformed-payload rate is about 15%–17%. China 13.04 with the
Global 13.04 alias also yields zero movement and 17,660 malformed payloads.

## First structural divergence

| Layer | Result |
|---|---|
| A. Oodle decompression | Healthy; every ReplayData chunk is read to EOF |
| B. PlaybackPacketReader | Healthy; hundreds of thousands of packets |
| C. RawPacketReader | Healthy; malformed packet count is zero |
| D. Bunch header | Healthy; packet/bunch counts remain aligned |
| E. PackageMap / NetGUID | Paths and 472–539 export groups resolve |
| F. ActorChannel | Healthy, with 1,315–3,729 opens per replay |
| G. ContentBlock framing | Healthy, with 296,800–868,411 blocks |
| H. Property transform | First actual failure |
| I. RepLayout/ClassNetCache | Invalid packed integers, overruns, huge handles |
| J. Valorant descriptors | No valid movement; downstream counts are corrupted |

The first Global and China 13.05 samples share `bitCount=287`, `actorNetGuid=2`, and `seed=285`.

| Sample | Ciphertext first 8 bytes | Global-13.05 decoded first 8 bytes |
|---|---|---|
| Global | `BB91E1A8B1DFE7B7` | `100CA461300F0804` |
| China | `E0300FEB039E0CA4` | `B0F92DCE60F8E7EE` |

Global consumes all 287 bits and decodes two fields. The China alias throws after 191 bits with
`Packed integer did not terminate within five bytes`. Because the first uint64 is transformed before
`AdvanceTransformState`, later PRNG constants cannot repair this block.

## Historical transform and DSL search

Every complete historical Global first-block transform (12.10, 12.11, 13.00, 13.01, 13.02,
13.04, 13.05) was applied to the China ciphertext. None produces a valid first-block parse under
the property grammar. For the same-size Global control block used only as a comparison oracle, all
outputs have zero common-prefix bytes and the best whole-block Hamming distance is 25/64 bits. The
Global control plaintext is not assumed to be the exact China plaintext.

A second replay-only DSL search kept each historical operation skeleton but assigned every distinct
`ROR(state, 1..8)` term permutation. None maps the China block to the analogous Global control
block; the best candidates remain 18/64 bits away. This comparison excludes a trivial skeleton and
rotation-term reuse, but does not prove the unknown China plaintext or exhaust all possible
algorithms. Results are preserved in:

- `diagnostics/first_uint64_historical_transforms.json`
- `diagnostics/first_uint64_dsl_search.json`

Together with the parser-wide structural scores, this excludes direct old-algorithm reuse and makes
a simple permutation of the tested state-rotation terms unlikely.

## Executable analysis

Both installed clients were found and read statically:

| Client | SHA-256 | Bytes |
|---|---|---:|
| Riot Global | `d0e1ec92df82ff3041de3d37584653c399d641a1694df6b32d9223177da6d6d1` | 232,803,544 |
| Tencent China | `e4e78d23735a93f70000eca1fcd7928f8bc570800fdc1fd55862e45597d9e827` | 239,708,800 |

The Global executable contains valid 13.05 initialization code at raw offset `0x044AD71B`:

- `lea edx,[r9+0x48c26613]`
- `movabs rdx,0x2545f4914f6cdd1d`

The Tencent executable uses protected `.std` sections. Its static image contains zero copies of the
64-bit PRNG multiplier. Its single `0x48C26613` byte sequence is surrounded by high-entropy bytes
that do not disassemble as a function. The China transform is therefore not available in plaintext
for a static BinDiff/Diaphora comparison. Full anchor evidence and hashes are in
`diagnostics/binary_anchor_scan.json`.

Reading the unpacked China function requires an explicitly authorized runtime-memory dump while the
China client is running. That operation was not performed because it interacts with a protected game
process and anti-cheat environment.

## Implemented diagnostic tooling

The reproducible parser patch now provides:

- `--diagnose-transform <jsonl>` and a bounded `--diagnose-transform-limit`;
- `--diagnose-china-alias`, separate from normal support registration;
- raw/decoded hex, replay/packet/channel/actor/object/path context, verified seed, parsed bits,
  decoded fields, malformed flag, and exception details;
- per-layer manifest counters;
- a `transform-first-block` command covering all historical transforms;
- tests proving China aliases are unavailable by default and available only in diagnostic mode.

The Rust probe now preserves source filenames, recognizes China 13.04, scans executable anchors, and
searches historical first-block DSL skeletons. `scripts/collect_china_diagnostics.ps1` reproduces the
summary JSON files from parser manifests and JSONL samples.

## Remaining work and acceptance status

China 13.05 acceptance is not met: no authentic China transform exists yet, movement is zero, and
the alias malformed rate is far above Global. Consequently China 13.04 must not be promoted either.
No `ValorantSeededTransformChina13_05` or `ValorantSeededTransformChina13_04` production class was
invented, and no known-vector test was fabricated.

The shortest remaining path is:

1. Explicitly authorize a read-only runtime dump of the unpacked Tencent module while the China
   client is running.
2. Locate the Global-analog transform caller/function in that dump and transcribe UInt64, PRNG,
   UInt32, byte, and tail stages.
3. Add isolated China 13.05 known vectors for 0/1/7/8/31/32/63/64/65/>256 bits.
4. Re-run all four China 13.05 fixtures and require nonzero, finite movement plus credible gunplay
   and RPC counts with a malformed rate near Global.
5. Repeat for China 13.04, then investigate descriptor overlays only if the transform is healthy.
