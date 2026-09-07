# VALORANT China replay payload transform diagnosis

Date: 2026-09-08

## Result

The China replay failure is localized to layer H, the property/RPC payload transform. The replay
container, Oodle decompression, playback packets, raw packets, bunch framing, actor channels, and
content-block framing all remain structurally healthy. Applying the matching Global transform is
not a solution: all five China fixtures reach EOF only in the isolated diagnostic mode, but produce
zero movement and tens of thousands of malformed payloads.

China 13.05 `TransformUInt64` differs before PRNG state advancement. The first 64-bit block uses
`state = seed`, so this conclusion is independent of `SeedAddend`, `InitialPrngA`, and later PRNG
state transitions. Constant-only brute force must not be resumed.

Replay-only solving has now recovered and cross-validated the China 13.05 `TransformUInt64`, PRNG
initialization/state advance, and `TransformUInt32` stages. The byte transform remains unsolved;
the tail constant `0xC9` is a strong but not independently complete hypothesis. Therefore the full
China 13.05 transform, and all of China 13.04, are **not recovered yet**. The partial candidate is
opt-in and rejected by the default registry. ValCoach continues to report China gameplay payloads
as unsupported instead of returning corrupt coaching data.

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

## Replay-only recovery of China 13.05

Four China 13.05 fixtures were solved jointly; no result below was selected from a single replay.
The first-word meet-in-the-middle search and grammar score recovered this transform:

```text
value += ROR32(state, 8)
value  = ROL64(value, ROR32(state, 6) % 63 + 1)
value  = ROR64(value, ROR32(state, 7) % 63 + 1)
value  = ROL64(value, ROR32(state, 4) % 63 + 1)
value  = ROR64(value, ROR32(state, 2) % 63 + 1)
value -= ROR32(state, 1)
```

It maps the China first ciphertext word `E0300FEB039E0CA4`, seed 285, to
`100CA461300F0804`. Across the four bounded sample sets, the recovered transform gives valid first
handles/lengths for every scored row and the observed maximum handle is 59, matching the healthy
Global distribution.

For the second word of the seed-285 sample, an exhaustive 32-bit state search against a 40-bit
grammar prefix yielded exactly one state, `0x05905425`, and decoded
`FE245F1C0000A09C` to `9340010000004039`. Cross-fixture scoring then uniquely selected the shared
state generator parameters:

```text
SeedAddend      = 0xF67761C9
InitASeedAddend = 0xC9
Initial A uses  seed + 0xC9
state advance   = the existing xoroshiro-style shared helper
```

The recovered 32-bit stage is:

```text
value += ROL32(state, 8)
value  = ROR32(value, ROL32(state, 2) % 31 + 1)
value  = ROR32(value, ROL32(state, 7) % 31 + 1)
value  = ROL32(value, ROL32(state, 4) % 31 + 1)
value  = ROL32(value, ROL32(state, 6) % 31 + 1)
value -= ROL32(state, 1)
```

Independent vectors include `669B201A -> 100CA261` at seed 43 and the 64/128-bit vectors now
covered by diagnostic unit tests. Rotation order is algebraically interchangeable where adjacent
rotations have no intervening arithmetic.

### Byte-stage rejection evidence

The four fixtures provide 83 distinct 9-bit empty-RepLayout samples whose plaintext first byte is
zero, plus the shared control vectors `FA -> 40` (seed 13) and `C7 17 40 -> 68 09 00` (seed 26
with two recovered PRNG states). In total, 87 byte constraints were used.

The solver rejected:

- every complete historical byte transform from 12.10 through 13.05;
- historical templates with substituted arithmetic multipliers and state-rotation terms;
- simple arithmetic/rotation normal forms;
- a bidirectional program search through six reversible primitives;
- the compact `add 0x61 / rotate / subtract 0x0B` candidate: it decodes the 24-bit control as
  `68 F8 70`, not `68 09 00`.

Full-file score also rejects every partial byte hypothesis. Depending on the rejected candidate,
the Ascent replay reports 291,511–354,408 malformed payloads and implausible 684,841–1,341,956
movement-shaped rows, compared with zero malformed payloads and 165,047 movement rows in the
Global control. These are parser false positives, not gameplay recovery.

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
64-bit PRNG multiplier and also zero copies of the independently recovered `0xF67761C9` addend. Its
single `0x48C26613` byte sequence is surrounded by high-entropy bytes that do not disassemble as a
function. The China transform is therefore not available in plaintext for a static
BinDiff/Diaphora comparison. Full anchor evidence and hashes are in
`diagnostics/binary_anchor_scan.json`.

After explicit authorization, the read-only runtime scanner was tested against a running Tencent
China process (PID 211232) and later against a running Riot Global process (PID 216764). Windows
denied `OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ)` with error 5 in both cases,
including the approved elevated attempt. No injection, handle hijacking, driver, protection change,
or anti-cheat bypass was attempted. The installed Tencent `VALORANT-Win64-ShippingBase.dll` is also
protected (`.tvm0`) and contains no usable transform anchors.

## Implemented diagnostic tooling

The reproducible parser patch now provides:

- `--diagnose-transform <jsonl>` and a bounded `--diagnose-transform-limit`;
- `--diagnose-china-alias`, separate from normal support registration;
- raw/decoded hex, replay/packet/channel/actor/object/path context, verified seed, parsed bits,
  decoded fields, malformed flag, and exception details;
- per-layer manifest counters;
- a `transform-first-block` command covering all historical transforms;
- tests proving China aliases are unavailable by default and available only in diagnostic mode.

The Rust probe now preserves source filenames, recognizes China 13.04, scans static/runtime anchors,
and includes reproducible UInt64, PRNG, state, UInt32, byte-template, and bidirectional byte-program
solvers. The parser diagnostic candidate has unit vectors for the recovered 32/64/128-bit stages but
is never installed by the default registry. `scripts/collect_china_diagnostics.ps1` reproduces the
summary JSON files from parser manifests and JSONL samples.

## Remaining work and acceptance status

China 13.05 acceptance is not met: three stages are recovered, but the byte stage is not, and every
partial full-file run has a malformed rate far above Global. Consequently full China 13.04 transform
recovery must not proceed to promotion under the task's gating rule; its baseline and Global-alias
diagnostics have already been completed. No `ValorantSeededTransformChina13_05` or
`ValorantSeededTransformChina13_04` production class was invented, and no unsupported vector was
presented as final.

The shortest remaining path is:

1. Obtain a legally readable unpacked Tencent module/function (the authorized process-read route is
   blocked by Windows/anti-cheat access control), or extend replay-only synthesis with the new byte
   template used by China.
2. Transcribe and validate the remaining byte stage and independently prove the tail bits.
3. Add final China 13.05 vectors for 0/1/7/8/31/32/63/64/65/>256 bits.
4. Re-run all four China 13.05 fixtures and require nonzero, finite movement plus credible gunplay
   and RPC counts with a malformed rate near Global.
5. Repeat for China 13.04, then investigate descriptor overlays only if the transform is healthy.
