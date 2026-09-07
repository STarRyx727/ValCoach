//! Reduce the China 13.05 byte transform to arithmetic -> rotation -> arithmetic candidates.

use std::{
    collections::HashSet,
    fs::File,
    io::{BufRead, BufReader},
};

use serde::{Deserialize, Serialize};

const MULTIPLIER: u64 = 0x2545_f491_4f6c_dd1d;
const SEED_ADDEND: u32 = 0xf677_61c9;
type BytePair = (u8, u8, u32);

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum Op {
    Add,
    Sub,
    Xor,
    XorNot,
}

#[derive(Serialize)]
struct Candidate {
    first_operation: Op,
    first_multiplier: String,
    rotate_left_counts: Vec<u32>,
    last_operation: Op,
    last_multiplier: String,
}

#[derive(Serialize)]
struct ExtendedCandidate {
    first_operation: Op,
    first_term: String,
    rotate_left_counts: Vec<u32>,
    last_operation: Op,
    last_term: String,
}

#[derive(Deserialize)]
struct Sample {
    payload_bit_count: usize,
    seed: u32,
    raw_ciphertext_hex: String,
}

fn initial_a(seed: u32) -> u64 {
    let seed_plus = seed.wrapping_add(SEED_ADDEND);
    let mixed = (((seed_plus >> 15) ^ seed_plus) >> 12)
        ^ seed
            .wrapping_add(SEED_ADDEND & 0xff)
            .wrapping_mul(0x0200_0000)
        ^ seed_plus;
    u64::from(mixed).wrapping_mul(MULTIPLIER)
}

fn initial_b(seed: u32) -> u64 {
    let mixed = (((seed >> 15) ^ seed) >> 12) ^ seed.wrapping_shl(25) ^ seed;
    u64::from(mixed).wrapping_mul(MULTIPLIER)
}

fn advance(prng_a: &mut u64, prng_b: &mut u64) -> u32 {
    let sum = prng_b.wrapping_add(*prng_a);
    *prng_b ^= *prng_a;
    *prng_a = prng_a.rotate_right(9) ^ prng_b.wrapping_shl(14) ^ *prng_b;
    *prng_b = prng_b.rotate_left(36);
    (sum >> 32) as u32
}

fn collect_pairs(paths: &[String]) -> Result<Vec<BytePair>, Box<dyn std::error::Error>> {
    let mut pairs = HashSet::new();
    for path in paths {
        for line in BufReader::new(File::open(path)?).lines() {
            let sample: Sample = serde_json::from_str(&line?)?;
            if sample.payload_bit_count != 9 {
                continue;
            }
            let ciphertext = hex::decode(sample.raw_ciphertext_hex)?[0];
            pairs.insert((ciphertext, 0, sample.seed));
        }
    }

    // Stable BaseReplayController grammar shared by all four China 13.05 fixtures.
    pairs.insert((0x04, 0x00, 11));
    pairs.insert((0xfa, 0x40, 13));

    // The 24-bit packet is the same 68 09 00 empty-control structure as Global 13.05.
    let mut prng_a = initial_a(26);
    let mut prng_b = initial_b(26);
    pairs.insert((0xc7, 0x68, 26));
    let state2 = advance(&mut prng_a, &mut prng_b);
    pairs.insert((0x17, 0x09, state2));
    let state3 = advance(&mut prng_a, &mut prng_b);
    pairs.insert((0x40, 0x00, state3));

    let mut pairs: Vec<_> = pairs.into_iter().collect();
    pairs.sort_by_key(|(_, _, state)| *state);
    Ok(pairs)
}

fn apply(value: u8, term: u8, op: Op) -> u8 {
    match op {
        Op::Add => value.wrapping_add(term),
        Op::Sub => value.wrapping_sub(term),
        Op::Xor => value ^ term,
        Op::XorNot => value ^ !term,
    }
}

fn inverse(value: u8, term: u8, op: Op) -> u8 {
    match op {
        Op::Add => value.wrapping_sub(term),
        Op::Sub => value.wrapping_add(term),
        Op::Xor => value ^ term,
        Op::XorNot => value ^ !term,
    }
}

fn extended_normal_form(pairs: &[BytePair]) -> Vec<ExtendedCandidate> {
    let mut terms: Vec<(String, Vec<u8>)> = (0u32..=255)
        .map(|multiplier| {
            (
                format!("low8(state*0x{multiplier:02X})"),
                pairs
                    .iter()
                    .map(|(_, _, state)| state.wrapping_mul(multiplier) as u8)
                    .collect(),
            )
        })
        .collect();
    for rotation in 1..=8 {
        terms.push((
            format!("low8(rol32(state,{rotation}))"),
            pairs
                .iter()
                .map(|(_, _, state)| state.rotate_left(rotation) as u8)
                .collect(),
        ));
        terms.push((
            format!("low8(ror32(state,{rotation}))"),
            pairs
                .iter()
                .map(|(_, _, state)| state.rotate_right(rotation) as u8)
                .collect(),
        ));
    }

    let operations = [Op::Add, Op::Sub, Op::Xor, Op::XorNot];
    let mut candidates = Vec::new();
    for first_operation in operations {
        for (first_name, first_values) in &terms {
            for last_operation in operations {
                for (last_name, last_values) in &terms {
                    let mut counts = Vec::with_capacity(pairs.len());
                    let mut valid = true;
                    for (index, (ciphertext, plaintext, _)) in pairs.iter().copied().enumerate() {
                        let middle_target = inverse(plaintext, last_values[index], last_operation);
                        let first_value = apply(ciphertext, first_values[index], first_operation);
                        let matches: Vec<u32> = (0..8)
                            .filter(|count| first_value.rotate_left(*count) == middle_target)
                            .collect();
                        if matches.len() != 1 {
                            valid = false;
                            break;
                        }
                        counts.push(matches[0]);
                    }
                    if valid {
                        candidates.push(ExtendedCandidate {
                            first_operation,
                            first_term: first_name.clone(),
                            rotate_left_counts: counts,
                            last_operation,
                            last_term: last_name.clone(),
                        });
                    }
                }
            }
        }
    }
    candidates
}

fn analogue(value: u8, state: u32, left_state: bool) -> u8 {
    let term = |rotation| {
        if left_state {
            state.rotate_left(rotation) as u8
        } else {
            state.rotate_right(rotation) as u8
        }
    };
    let mut value = value.wrapping_add(term(8));
    value = value.rotate_left(u32::from(term(6)) % 7 + 1);
    value = value.rotate_right(u32::from(term(7)) % 7 + 1);
    value = value.rotate_left(u32::from(term(4)) % 7 + 1);
    value = value.rotate_right(u32::from(term(2)) % 7 + 1);
    value.wrapping_sub(term(1))
}

fn swap(value: u8) -> u8 {
    ((value & 0x55) << 1) | ((value >> 1) & 0x55)
}

fn reverse(value: u8) -> u8 {
    value.reverse_bits()
}

fn inverse_odd(value: u8) -> u8 {
    (1u16..=255)
        .step_by(2)
        .find(|candidate| (u16::from(value) * candidate) as u8 == 1)
        .unwrap() as u8
}

fn search_13_00_skeleton(pairs: &[BytePair]) -> Vec<serde_json::Value> {
    let table = hex::decode(concat!(
        "0a6c6996cadc5a08b38339a0f9adf4560e6e4c85649982d4885c8736239a",
        "112db8c4341866136f59e07422faa665e2d7954e94b0779e1aeee705a",
        "2c830900d9bd219c93a471512a9291f53acaf4352aef54dbfbee34a06",
        "d5d0a378a7d61c7a6b81d8dee568fb267ebcbae8cce4727f2cfcf0",
        "ec28716048ef3e038f1ef16a8df2461b9c86f7b476628a10fd6d0b",
        "3f9f2f555fc3c6921627d344840fe1808cb7738945db332550ea0414",
        "c50c32415e79a41d3d5b4037c1cffe2b54eb9d4991f307173cda57",
        "8bcd61f6ce702eff2193972a7d67abb57c5d0042a5d92051eddd0209",
        "c2d1f8bdbbe93524985838aab9a8b27501cbc063df3b8ec731b1a1",
        "b6e67b4b4f"
    ))
    .unwrap();
    let inverse_table = {
        let mut inverse = [0u8; 256];
        for (index, value) in table.iter().copied().enumerate() {
            inverse[value as usize] = index as u8;
        }
        inverse
    };
    let zero_pairs: Vec<_> = pairs
        .iter()
        .copied()
        .filter(|(_, plaintext, _)| *plaintext == 0)
        .collect();
    let first = zero_pairs[0];
    let target = inverse_table[0];
    let mut arithmetic = Vec::new();
    for first_multiplier in 0u32..=255 {
        for second_multiplier in 0u32..=255 {
            let before_xor = !reverse(
                first
                    .0
                    .wrapping_add(first.2.wrapping_mul(first_multiplier) as u8),
            )
            .wrapping_add(first.2.wrapping_mul(second_multiplier) as u8);
            let rhs = before_xor ^ target;
            let third_multiplier = rhs.wrapping_mul(inverse_odd(first.2 as u8));
            if zero_pairs.iter().all(|(ciphertext, _, state)| {
                let before_xor =
                    !reverse(ciphertext.wrapping_add(state.wrapping_mul(first_multiplier) as u8))
                        .wrapping_add(state.wrapping_mul(second_multiplier) as u8);
                before_xor ^ state.wrapping_mul(u32::from(third_multiplier)) as u8 == target
            }) {
                arithmetic.push((
                    first_multiplier as u8,
                    second_multiplier as u8,
                    third_multiplier,
                ));
            }
        }
    }

    let rotation_constants = [
        1u32, 0x0b, 0x79, 0x533, 0x1b0829, 0x2751b, 0x0cc6db61, 0x1b, 0x33, 0x31, 0xc9, 0xf67761c9,
    ];
    let mut matches = Vec::new();
    for (first_multiplier, second_multiplier, third_multiplier) in arithmetic {
        for rotation_multiplier in rotation_constants {
            let valid = pairs.iter().all(|(ciphertext, plaintext, state)| {
                let mut value =
                    ciphertext.wrapping_add(state.wrapping_mul(u32::from(first_multiplier)) as u8);
                value = reverse(value);
                value = !value.wrapping_add(state.wrapping_mul(u32::from(second_multiplier)) as u8)
                    ^ state.wrapping_mul(u32::from(third_multiplier)) as u8;
                value = table[value as usize];
                value.rotate_right(state.wrapping_mul(rotation_multiplier) % 7 + 1) == *plaintext
            });
            if valid {
                matches.push(serde_json::json!({
                    "first_multiplier_low8": format!("0x{first_multiplier:02X}"),
                    "second_multiplier_low8": format!("0x{second_multiplier:02X}"),
                    "third_multiplier_low8": format!("0x{third_multiplier:02X}"),
                    "rotation_multiplier": format!("0x{rotation_multiplier:08X}"),
                }));
            }
        }
    }
    matches
}

fn search_13_02_skeleton(pairs: &[BytePair]) -> Vec<serde_json::Value> {
    let table = hex::decode(concat!(
        "0a6c6996cadc5a08b38339a0f9adf4560e6e4c85649982d4885c8736239a",
        "112db8c4341866136f59e07422faa665e2d7954e94b0779e1aeee705a",
        "2c830900d9bd219c93a471512a9291f53acaf4352aef54dbfbee34a06",
        "d5d0a378a7d61c7a6b81d8dee568fb267ebcbae8cce4727f2cfcf0",
        "ec28716048ef3e038f1ef16a8df2461b9c86f7b476628a10fd6d0b",
        "3f9f2f555fc3c6921627d344840fe1808cb7738945db332550ea0414",
        "c50c32415e79a41d3d5b4037c1cffe2b54eb9d4991f307173cda57",
        "8bcd61f6ce702eff2193972a7d67abb57c5d0042a5d92051eddd0209",
        "c2d1f8bdbbe93524985838aab9a8b27501cbc063df3b8ec731b1a1",
        "b6e67b4b4f"
    ))
    .unwrap();
    let rotation_constants = [
        1u32, 0x0b, 0x79, 0x533, 0x1b0829, 0x2751b, 0x0cc6db61, 0x1b, 0x33, 0x31, 0xc9, 0xf67761c9,
    ];
    let mut matches = Vec::new();
    for term_multiplier in 0u32..=255 {
        for rotate_left_multiplier in rotation_constants {
            for rotate_right_multiplier in rotation_constants {
                let valid = pairs.iter().all(|(ciphertext, plaintext, state)| {
                    let mut value = table[*ciphertext as usize];
                    value = reverse(value);
                    value = !value.wrapping_sub(state.wrapping_mul(term_multiplier) as u8);
                    value = reverse(value);
                    value = value.rotate_left(state.wrapping_mul(rotate_left_multiplier) % 7 + 1);
                    value = value.rotate_right(state.wrapping_mul(rotate_right_multiplier) % 7 + 1);
                    value == *plaintext
                });
                if valid {
                    matches.push(serde_json::json!({
                        "term_multiplier_low8": format!("0x{term_multiplier:02X}"),
                        "rotate_left_multiplier": format!("0x{rotate_left_multiplier:08X}"),
                        "rotate_right_multiplier": format!("0x{rotate_right_multiplier:08X}"),
                    }));
                }
            }
        }
    }
    matches
}

const ROTATION_CONSTANTS: [u32; 12] = [
    1, 0x0b, 0x79, 0x533, 0x1b0829, 0x2751b, 0x0cc6db61, 0x1b, 0x33, 0x31, 0xc9, 0xf67761c9,
];

fn search_12_10_skeleton(pairs: &[BytePair]) -> Vec<serde_json::Value> {
    let first = pairs[0];
    let inverse_state = inverse_odd(first.2 as u8);
    let mut matches = Vec::new();
    for first_rotation in ROTATION_CONSTANTS {
        for second_rotation in ROTATION_CONSTANTS {
            for subtract_multiplier in 0u32..=255 {
                let mut value = first
                    .0
                    .rotate_right(first.2.wrapping_mul(first_rotation) % 7 + 1);
                value = swap(value).wrapping_sub(first.2.wrapping_mul(subtract_multiplier) as u8);
                value = value.rotate_right(first.2.wrapping_mul(second_rotation) % 7 + 1);
                let xor_multiplier = (value ^ swap(first.1)).wrapping_mul(inverse_state);
                if pairs.iter().all(|(ciphertext, plaintext, state)| {
                    let mut value =
                        ciphertext.rotate_right(state.wrapping_mul(first_rotation) % 7 + 1);
                    value = swap(value).wrapping_sub(state.wrapping_mul(subtract_multiplier) as u8);
                    value = value.rotate_right(state.wrapping_mul(second_rotation) % 7 + 1);
                    swap(value ^ state.wrapping_mul(u32::from(xor_multiplier)) as u8) == *plaintext
                }) {
                    matches.push(serde_json::json!({
                        "first_rotation_multiplier": format!("0x{first_rotation:08X}"),
                        "subtract_multiplier_low8": format!("0x{subtract_multiplier:02X}"),
                        "second_rotation_multiplier": format!("0x{second_rotation:08X}"),
                        "xor_multiplier_low8": format!("0x{xor_multiplier:02X}"),
                    }));
                }
            }
        }
    }
    matches
}

fn search_12_11_skeleton(pairs: &[BytePair]) -> Vec<serde_json::Value> {
    let first = pairs[0];
    let inverse_state = inverse_odd(first.2 as u8);
    let mut matches = Vec::new();
    for rotation in ROTATION_CONSTANTS {
        for first_add_multiplier in 0u32..=255 {
            let mut value = first.0.rotate_right(first.2.wrapping_mul(rotation) % 7 + 1);
            value = swap(value).wrapping_add(first.2.wrapping_mul(first_add_multiplier) as u8);
            value = reverse(value);
            let second_add_multiplier = swap(first.1)
                .wrapping_sub(value)
                .wrapping_mul(inverse_state);
            if pairs.iter().all(|(ciphertext, plaintext, state)| {
                let mut value = ciphertext.rotate_right(state.wrapping_mul(rotation) % 7 + 1);
                value = swap(value).wrapping_add(state.wrapping_mul(first_add_multiplier) as u8);
                value = reverse(value)
                    .wrapping_add(state.wrapping_mul(u32::from(second_add_multiplier)) as u8);
                swap(value) == *plaintext
            }) {
                matches.push(serde_json::json!({
                    "rotation_multiplier": format!("0x{rotation:08X}"),
                    "first_add_multiplier_low8": format!("0x{first_add_multiplier:02X}"),
                    "second_add_multiplier_low8": format!("0x{second_add_multiplier:02X}"),
                }));
            }
        }
    }
    matches
}

fn search_13_04_skeleton(pairs: &[BytePair]) -> Vec<serde_json::Value> {
    let first = pairs[0];
    let inverse_state = inverse_odd(first.2 as u8);
    let mut matches = Vec::new();
    for first_rotation in ROTATION_CONSTANTS {
        for second_rotation in ROTATION_CONSTANTS {
            for xor_multiplier in 0u32..=255 {
                for subtract_multiplier in 0u32..=255 {
                    let mut value = first
                        .0
                        .rotate_right(first.2.wrapping_mul(first_rotation) % 7 + 1);
                    value = (value ^ first.2.wrapping_mul(xor_multiplier) as u8)
                        .wrapping_sub(first.2.wrapping_mul(subtract_multiplier) as u8);
                    value = swap(value);
                    let before_final_rotation = first
                        .1
                        .rotate_right(first.2.wrapping_mul(second_rotation) % 7 + 1);
                    let add_multiplier = before_final_rotation
                        .wrapping_sub(value)
                        .wrapping_mul(inverse_state);
                    if pairs.iter().all(|(ciphertext, plaintext, state)| {
                        let mut value =
                            ciphertext.rotate_right(state.wrapping_mul(first_rotation) % 7 + 1);
                        value = (value ^ state.wrapping_mul(xor_multiplier) as u8)
                            .wrapping_sub(state.wrapping_mul(subtract_multiplier) as u8);
                        value = swap(value)
                            .wrapping_add(state.wrapping_mul(u32::from(add_multiplier)) as u8);
                        value.rotate_left(state.wrapping_mul(second_rotation) % 7 + 1) == *plaintext
                    }) {
                        matches.push(serde_json::json!({
                            "first_rotation_multiplier": format!("0x{first_rotation:08X}"),
                            "xor_multiplier_low8": format!("0x{xor_multiplier:02X}"),
                            "subtract_multiplier_low8": format!("0x{subtract_multiplier:02X}"),
                            "add_multiplier_low8": format!("0x{add_multiplier:02X}"),
                            "second_rotation_multiplier": format!("0x{second_rotation:08X}"),
                        }));
                    }
                }
            }
        }
    }
    matches
}

fn global_13_01(value: u8, state: u32) -> u8 {
    let state11 = state.wrapping_mul(0x0b);
    let mix = state11.wrapping_mul(0x533);
    let value = swap(!value) ^ mix.wrapping_mul(0x0b) as u8;
    let value = !value.rotate_right(mix % 7 + 1);
    value.wrapping_add(state11 as u8)
}

fn global_13_04(value: u8, state: u32) -> u8 {
    let mix_a = state.wrapping_mul(0x0b);
    let mix_b = mix_a.wrapping_mul(0x0b);
    let mix_c = mix_b.wrapping_mul(0x0b);
    let mix_d = mix_c.wrapping_mul(0x79);
    let mix_e = mix_d.wrapping_mul(0x0b);
    let mix_f = mix_e.wrapping_mul(0x0b);
    let value = value.rotate_right(mix_f % 7 + 1);
    let value = (value ^ mix_e as u8).wrapping_sub(mix_d as u8);
    let value = swap(value);
    let value = value.wrapping_add(mix_b as u8).wrapping_add(mix_c as u8);
    value.rotate_left(mix_a % 7 + 1)
}

fn global_13_05(value: u8, state: u32) -> u8 {
    let state_byte = state as u8;
    let mix_a = state.wrapping_mul(0x1b0829);
    let value =
        ((mix_a.wrapping_mul(0x79) as u8) ^ value).wrapping_sub(mix_a.wrapping_mul(0x0b) as u8);
    let value = value.rotate_left(mix_a % 7 + 1);
    let value = value.wrapping_sub(state_byte.wrapping_mul(0x1b))
        ^ state_byte.wrapping_mul(0x33)
        ^ state_byte.wrapping_mul(0x31);
    value
        .rotate_left(state.wrapping_mul(0x79) % 7 + 1)
        .rotate_left(state.wrapping_mul(0x0b) % 7 + 1)
}

fn china_word32_low(value: u8, state: u32) -> u8 {
    let mut output = u32::from(value).wrapping_add(state.rotate_left(8));
    output = output.rotate_left(state.rotate_left(6) % 31 + 1);
    output = output.rotate_right(state.rotate_left(7) % 31 + 1);
    output = output.rotate_left(state.rotate_left(4) % 31 + 1);
    output = output.rotate_right(state.rotate_left(2) % 31 + 1);
    output.wrapping_sub(state.rotate_left(1)) as u8
}

fn china_word64_low(value: u8, state: u32) -> u8 {
    let mut output = u64::from(value).wrapping_add(u64::from(state.rotate_right(8)));
    output = output.rotate_left(u64::from(state.rotate_right(6)) as u32 % 63 + 1);
    output = output.rotate_right(u64::from(state.rotate_right(7)) as u32 % 63 + 1);
    output = output.rotate_left(u64::from(state.rotate_right(4)) as u32 % 63 + 1);
    output = output.rotate_right(u64::from(state.rotate_right(2)) as u32 % 63 + 1);
    output.wrapping_sub(u64::from(state.rotate_right(1))) as u8
}

fn search_13_01_skeleton(pairs: &[BytePair]) -> Vec<serde_json::Value> {
    let rotation_constants = [
        1u32, 0x0b, 0x79, 0x533, 0x1b0829, 0x2751b, 0x0cc6db61, 0x1b, 0x33, 0x31, 0xc9, 0xf67761c9,
    ];
    let mut matches = Vec::new();
    for rotation_multiplier in rotation_constants {
        for xor_multiplier in 0u32..=255 {
            for add_multiplier in 0u32..=255 {
                let valid = pairs.iter().all(|(ciphertext, plaintext, state)| {
                    let value = swap(!ciphertext) ^ state.wrapping_mul(xor_multiplier) as u8;
                    let value =
                        !value.rotate_right(state.wrapping_mul(rotation_multiplier) % 7 + 1);
                    value.wrapping_add(state.wrapping_mul(add_multiplier) as u8) == *plaintext
                });
                if valid {
                    matches.push(serde_json::json!({
                        "xor_multiplier_low8": format!("0x{xor_multiplier:02X}"),
                        "rotation_multiplier": format!("0x{rotation_multiplier:08X}"),
                        "add_multiplier_low8": format!("0x{add_multiplier:02X}"),
                    }));
                }
            }
        }
    }
    matches
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        return Err("usage: china_byte_search <china13.05.jsonl> [more.jsonl ...]".into());
    }
    let pairs = collect_pairs(&paths)?;
    let analogue_scores = [false, true].map(|left_state| {
        serde_json::json!({
            "state_terms": if left_state { "rotate_left" } else { "rotate_right" },
            "matching_pairs": pairs.iter().filter(|(ciphertext, plaintext, state)| {
                analogue(*ciphertext, *state, left_state) == *plaintext
            }).count(),
            "total_pairs": pairs.len(),
        })
    });
    let historical_scores = [
        ("13.01", global_13_01 as fn(u8, u32) -> u8),
        ("13.04", global_13_04 as fn(u8, u32) -> u8),
        ("13.05", global_13_05 as fn(u8, u32) -> u8),
        ("china-word32-low", china_word32_low as fn(u8, u32) -> u8),
        ("china-word64-low", china_word64_low as fn(u8, u32) -> u8),
    ]
    .map(|(name, transform)| {
        serde_json::json!({
            "skeleton": name,
            "matching_pairs": pairs.iter().filter(|(ciphertext, plaintext, state)| {
                transform(*ciphertext, *state) == *plaintext
            }).count(),
            "total_pairs": pairs.len(),
        })
    });
    let skeleton_13_01_candidates = search_13_01_skeleton(&pairs);
    let skeleton_13_00_candidates = search_13_00_skeleton(&pairs);
    let skeleton_13_02_candidates = search_13_02_skeleton(&pairs);
    let skeleton_12_10_candidates = search_12_10_skeleton(&pairs);
    let skeleton_12_11_candidates = search_12_11_skeleton(&pairs);
    let skeleton_13_04_candidates = search_13_04_skeleton(&pairs);
    let extended_candidates = extended_normal_form(&pairs);
    let operations = [Op::Add, Op::Sub, Op::Xor, Op::XorNot];
    let mut candidates = Vec::new();
    for first_operation in operations {
        for first_multiplier in 0u32..=255 {
            for last_operation in operations {
                for last_multiplier in 0u32..=255 {
                    let mut counts = Vec::with_capacity(pairs.len());
                    let mut valid = true;
                    for (ciphertext, plaintext, state) in pairs.iter().copied() {
                        let first_term = state.wrapping_mul(first_multiplier) as u8;
                        let last_term = state.wrapping_mul(last_multiplier) as u8;
                        let middle_target = inverse(plaintext, last_term, last_operation);
                        let first_value = apply(ciphertext, first_term, first_operation);
                        let matches: Vec<u32> = (0..8)
                            .filter(|count| first_value.rotate_left(*count) == middle_target)
                            .collect();
                        if matches.len() != 1 {
                            valid = false;
                            break;
                        }
                        counts.push(matches[0]);
                    }
                    if valid {
                        candidates.push(Candidate {
                            first_operation,
                            first_multiplier: format!("0x{first_multiplier:02X}"),
                            rotate_left_counts: counts,
                            last_operation,
                            last_multiplier: format!("0x{last_multiplier:02X}"),
                        });
                    }
                }
            }
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "pairs": pairs.iter().map(|(ciphertext, plaintext, seed)| serde_json::json!({
                "ciphertext": format!("0x{ciphertext:02X}"),
                "plaintext": format!("0x{plaintext:02X}"),
                "seed": seed,
            })).collect::<Vec<_>>(),
            "normal_form": "op(ciphertext, low8(state*k1)) -> rotate-left -> op(low8(state*k2))",
            "candidate_count": candidates.len(),
            "analogue_scores": analogue_scores,
            "historical_scores": historical_scores,
            "skeleton_13_01_candidates": skeleton_13_01_candidates,
            "skeleton_13_00_candidates": skeleton_13_00_candidates,
            "skeleton_13_02_candidates": skeleton_13_02_candidates,
            "skeleton_12_10_candidates": skeleton_12_10_candidates,
            "skeleton_12_11_candidates": skeleton_12_11_candidates,
            "skeleton_13_04_candidates": skeleton_13_04_candidates,
            "extended_normal_form_candidates": extended_candidates,
            "candidates": candidates,
        }))?
    );
    Ok(())
}
