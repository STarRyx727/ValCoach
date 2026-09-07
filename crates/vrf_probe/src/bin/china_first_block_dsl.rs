//! Replay-only first-uint64 search over historical transform skeletons.
//!
//! Every parameter slot is assigned a distinct ROR(state, 1..=8) term. This checks whether the
//! China transform reused a known operation skeleton with a different state-term permutation.

use std::env;

use serde::Serialize;

#[derive(Clone, Copy)]
enum Op {
    Add(usize),
    Sub(usize),
    Xor(usize),
    XorNot(usize),
    Rol(usize),
    Ror(usize),
    Not,
    Swap,
    Reverse,
    Substitute,
}

struct Skeleton {
    name: &'static str,
    slot_count: usize,
    operations: &'static [Op],
}

#[derive(Clone, Serialize)]
struct Candidate {
    skeleton: &'static str,
    state_rotations: Vec<u32>,
    decoded_hex: String,
    hamming_distance_bits: u32,
    exact_match: bool,
}

const SKELETONS: &[Skeleton] = &[
    Skeleton {
        name: "12.10",
        slot_count: 4,
        operations: &[
            Op::Ror(0),
            Op::Swap,
            Op::Sub(1),
            Op::Ror(2),
            Op::XorNot(3),
            Op::Swap,
        ],
    },
    Skeleton {
        name: "12.11",
        slot_count: 5,
        operations: &[
            Op::Ror(0),
            Op::Swap,
            Op::Add(1),
            Op::Reverse,
            Op::Sub(2),
            Op::Sub(3),
            Op::Sub(4),
            Op::Swap,
        ],
    },
    Skeleton {
        name: "13.00",
        slot_count: 4,
        operations: &[
            Op::Add(0),
            Op::Reverse,
            Op::Add(1),
            Op::Xor(2),
            Op::Substitute,
            Op::Ror(3),
        ],
    },
    Skeleton {
        name: "13.01",
        slot_count: 3,
        operations: &[
            Op::Not,
            Op::Swap,
            Op::XorNot(0),
            Op::Ror(1),
            Op::Not,
            Op::Add(2),
        ],
    },
    Skeleton {
        name: "13.02",
        slot_count: 3,
        operations: &[
            Op::Substitute,
            Op::Reverse,
            Op::Sub(0),
            Op::Not,
            Op::Reverse,
            Op::Rol(1),
            Op::Ror(2),
        ],
    },
    Skeleton {
        name: "13.04",
        slot_count: 6,
        operations: &[
            Op::Ror(0),
            Op::XorNot(1),
            Op::Sub(2),
            Op::Swap,
            Op::Add(3),
            Op::Add(4),
            Op::Rol(5),
        ],
    },
    Skeleton {
        name: "13.05",
        slot_count: 8,
        operations: &[
            Op::XorNot(0),
            Op::Sub(1),
            Op::Rol(2),
            Op::Sub(3),
            Op::XorNot(4),
            Op::XorNot(5),
            Op::Rol(6),
            Op::Rol(7),
        ],
    },
];

fn swap_adjacent(value: u64) -> u64 {
    ((value & 0x5555_5555_5555_5555) << 1) | ((value >> 1) & 0x5555_5555_5555_5555)
}

fn reverse_without_final_16_swap(mut value: u64) -> u64 {
    value = ((value & 0x5555_5555_5555_5555) << 1) | ((value >> 1) & 0x5555_5555_5555_5555);
    value = ((value & 0x3333_3333_3333_3333) << 2) | ((value >> 2) & 0x3333_3333_3333_3333);
    value = ((value & 0x0f0f_0f0f_0f0f_0f0f) << 4) | ((value >> 4) & 0x0f0f_0f0f_0f0f_0f0f);
    value = ((value & 0x00ff_00ff_00ff_00ff) << 8) | ((value >> 8) & 0x00ff_00ff_00ff_00ff);
    value.rotate_left(32)
}

fn substitute(mut value: u64, table: &[u8]) -> u64 {
    let mut output = 0u64;
    for shift in (0..64).step_by(8) {
        output |= (table[(value & 0xff) as usize] as u64) << shift;
        value >>= 8;
    }
    output
}

fn apply(
    skeleton: &Skeleton,
    assignment: &[u32],
    ciphertext: u64,
    state: u32,
    table: &[u8],
) -> u64 {
    let terms: Vec<u64> = assignment
        .iter()
        .map(|count| state.rotate_right(*count) as u64)
        .collect();
    let mut value = ciphertext;
    for operation in skeleton.operations {
        value = match *operation {
            Op::Add(slot) => value.wrapping_add(terms[slot]),
            Op::Sub(slot) => value.wrapping_sub(terms[slot]),
            Op::Xor(slot) => value ^ terms[slot],
            Op::XorNot(slot) => value ^ !terms[slot],
            Op::Rol(slot) => value.rotate_left((terms[slot] % 63 + 1) as u32),
            Op::Ror(slot) => value.rotate_right((terms[slot] % 63 + 1) as u32),
            Op::Not => !value,
            Op::Swap => swap_adjacent(value),
            Op::Reverse => reverse_without_final_16_swap(value),
            Op::Substitute => substitute(value, table),
        };
    }
    value
}

#[allow(clippy::too_many_arguments)]
fn visit_assignments(
    skeleton: &Skeleton,
    ciphertext: u64,
    expected: u64,
    state: u32,
    table: &[u8],
    assignment: &mut Vec<u32>,
    used: &mut [bool; 9],
    candidates: &mut Vec<Candidate>,
) {
    if assignment.len() == skeleton.slot_count {
        let decoded = apply(skeleton, assignment, ciphertext, state, table);
        let distance = (decoded ^ expected).count_ones();
        candidates.push(Candidate {
            skeleton: skeleton.name,
            state_rotations: assignment.clone(),
            decoded_hex: hex::encode_upper(decoded.to_le_bytes()),
            hamming_distance_bits: distance,
            exact_match: distance == 0,
        });
        return;
    }
    for rotation in 1..=8 {
        if !used[rotation] {
            used[rotation] = true;
            assignment.push(rotation as u32);
            visit_assignments(
                skeleton, ciphertext, expected, state, table, assignment, used, candidates,
            );
            assignment.pop();
            used[rotation] = false;
        }
    }
}

fn parse_u64_le(hex_text: &str) -> Result<u64, Box<dyn std::error::Error>> {
    let bytes = hex::decode(hex_text)?;
    if bytes.len() != 8 {
        return Err(
            "ciphertext and expected plaintext must each contain exactly eight bytes".into(),
        );
    }
    Ok(u64::from_le_bytes(bytes.try_into().unwrap()))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 4 {
        return Err(
            "usage: china_first_block_dsl <ciphertext-hex-8-bytes> <expected-hex-8-bytes> <seed>"
                .into(),
        );
    }
    let ciphertext = parse_u64_le(&args[1])?;
    let expected = parse_u64_le(&args[2])?;
    let state: u32 = args[3].parse()?;
    let table = hex::decode(concat!(
        "77b9042feb7d27c944739a3f36f565ddf7e0302da9985dde69a394a05e170678",
        "a4f6ab0343c828e56a8e1cf270cf5305d30dffa7a23a32255a1f48c1",
        "b7e16e85996047bbe48acbc01bea6164f0c2d88bcdfdadb819b5bf0e9181",
        "839d45d249e9c731bd20bec66680d179d7e6fca15b5fdff1d0506752fe",
        "7b3513f846b3758de33e2ef4dc342a0823e20c094beec30f248f544c",
        "5539cc1d1e3b2272da296b41aaa6122c93ca9c970a56a87a9eb462923",
        "d9f38f3408437b2d4af7633fa21effb716f9082511ac574f95907ba11",
        "b1acd6ede702ae9610167c4f881426bc1501684a2b0b7fa54ee86dec4d",
        "b05cc4009558b6d57e42db5718866cced99b89873c8c63"
    ))?;
    let mut candidates = Vec::new();
    for skeleton in SKELETONS {
        visit_assignments(
            skeleton,
            ciphertext,
            expected,
            state,
            &table,
            &mut Vec::new(),
            &mut [false; 9],
            &mut candidates,
        );
    }
    candidates.sort_by_key(|candidate| candidate.hamming_distance_bits);
    let exact: Vec<_> = candidates
        .iter()
        .filter(|candidate| candidate.exact_match)
        .cloned()
        .collect();
    let best: Vec<_> = candidates.into_iter().take(20).collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "ciphertext_hex": args[1].to_uppercase(),
            "expected_plaintext_hex": args[2].to_uppercase(),
            "seed": state,
            "search_space": "historical skeletons with distinct ROR(state,1..8) assignments",
            "exact_candidates": exact,
            "best_candidates": best,
        }))?
    );
    Ok(())
}
