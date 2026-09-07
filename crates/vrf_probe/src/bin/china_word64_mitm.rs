//! Meet-in-the-middle synthesis for the China 13.05 first-word transform.
//!
//! Two candidate known-plaintext pairs are required simultaneously. They come from initial
//! BaseReplayController and BaseReplayPlayerState payloads whose first ciphertext word is identical
//! across all four China fixtures and whose shape matches the Global control. The Global plaintext
//! remains a hypothesis, so a hit still requires parser-wide validation.

use std::collections::HashMap;

use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Op {
    Add(u8),
    Sub(u8),
    Xor(u8),
    XorNot(u8),
    Rol(u8),
    Ror(u8),
    Not,
    Swap,
    Reverse,
    Substitute,
}

#[derive(Clone)]
struct Prefix {
    operations: Vec<Op>,
    used_terms: u8,
}

#[derive(Serialize)]
struct Candidate {
    operation_count: usize,
    operations: Vec<String>,
}

const PAIRS: [(u64, u64, u32); 2] = [
    (0xA40C_9E03_EB0F_30E0, 0x0408_0F30_61A4_0C10, 285),
    (0x2080_C105_C318_30D0, 0x3061_A041_0182_0C10, 92),
];

fn table() -> Vec<u8> {
    hex::decode(concat!(
        "77b9042feb7d27c944739a3f36f565ddf7e0302da9985dde69a394a05e170678",
        "a4f6ab0343c828e56a8e1cf270cf5305d30dffa7a23a32255a1f48c1",
        "b7e16e85996047bbe48acbc01bea6164f0c2d88bcdfdadb819b5bf0e9181",
        "839d45d249e9c731bd20bec66680d179d7e6fca15b5fdff1d0506752fe",
        "7b3513f846b3758de33e2ef4dc342a0823e20c094beec30f248f544c",
        "5539cc1d1e3b2272da296b41aaa6122c93ca9c970a56a87a9eb462923",
        "d9f38f3408437b2d4af7633fa21effb716f9082511ac574f95907ba11",
        "b1acd6ede702ae9610167c4f881426bc1501684a2b0b7fa54ee86dec4d",
        "b05cc4009558b6d57e42db5718866cced99b89873c8c63"
    ))
    .expect("embedded table is valid hex")
}

fn inverse_table(table: &[u8]) -> Vec<u8> {
    let mut inverse = vec![0; 256];
    for (index, value) in table.iter().copied().enumerate() {
        inverse[value as usize] = index as u8;
    }
    inverse
}

fn swap(value: u64) -> u64 {
    ((value & 0x5555_5555_5555_5555) << 1) | ((value >> 1) & 0x5555_5555_5555_5555)
}

fn reverse(mut value: u64) -> u64 {
    value = ((value & 0x5555_5555_5555_5555) << 1) | ((value >> 1) & 0x5555_5555_5555_5555);
    value = ((value & 0x3333_3333_3333_3333) << 2) | ((value >> 2) & 0x3333_3333_3333_3333);
    value = ((value & 0x0f0f_0f0f_0f0f_0f0f) << 4) | ((value >> 4) & 0x0f0f_0f0f_0f0f_0f0f);
    value = ((value & 0x00ff_00ff_00ff_00ff) << 8) | ((value >> 8) & 0x00ff_00ff_00ff_00ff);
    value.rotate_left(32)
}

fn substitute(mut value: u64, table: &[u8]) -> u64 {
    let mut output = 0;
    for shift in (0..64).step_by(8) {
        output |= (table[(value & 0xff) as usize] as u64) << shift;
        value >>= 8;
    }
    output
}

fn term(state: u32, index: u8) -> u64 {
    state.rotate_right(u32::from(index) + 1) as u64
}

fn apply(value: u64, state: u32, op: Op, table: &[u8]) -> u64 {
    match op {
        Op::Add(index) => value.wrapping_add(term(state, index)),
        Op::Sub(index) => value.wrapping_sub(term(state, index)),
        Op::Xor(index) => value ^ term(state, index),
        Op::XorNot(index) => value ^ !term(state, index),
        Op::Rol(index) => value.rotate_left((term(state, index) % 63 + 1) as u32),
        Op::Ror(index) => value.rotate_right((term(state, index) % 63 + 1) as u32),
        Op::Not => !value,
        Op::Swap => swap(value),
        Op::Reverse => reverse(value),
        Op::Substitute => substitute(value, table),
    }
}

fn apply_inverse(value: u64, state: u32, op: Op, inverse: &[u8]) -> u64 {
    match op {
        Op::Add(index) => value.wrapping_sub(term(state, index)),
        Op::Sub(index) => value.wrapping_add(term(state, index)),
        Op::Xor(index) => value ^ term(state, index),
        Op::XorNot(index) => value ^ !term(state, index),
        Op::Rol(index) => value.rotate_right((term(state, index) % 63 + 1) as u32),
        Op::Ror(index) => value.rotate_left((term(state, index) % 63 + 1) as u32),
        Op::Not => !value,
        Op::Swap => swap(value),
        Op::Reverse => reverse(value),
        Op::Substitute => substitute(value, inverse),
    }
}

fn all_ops() -> Vec<Op> {
    let mut operations = Vec::with_capacity(52);
    for index in 0..8 {
        operations.extend([
            Op::Add(index),
            Op::Sub(index),
            Op::Xor(index),
            Op::XorNot(index),
            Op::Rol(index),
            Op::Ror(index),
        ]);
    }
    operations.extend([Op::Not, Op::Swap, Op::Reverse, Op::Substitute]);
    operations
}

fn term_index(op: Op) -> Option<u8> {
    match op {
        Op::Add(index)
        | Op::Sub(index)
        | Op::Xor(index)
        | Op::XorNot(index)
        | Op::Rol(index)
        | Op::Ror(index) => Some(index),
        _ => None,
    }
}

fn should_skip(previous: Option<Op>, next: Op) -> bool {
    matches!(
        (previous, next),
        (Some(Op::Not), Op::Not) | (Some(Op::Swap), Op::Swap) | (Some(Op::Reverse), Op::Reverse)
    )
}

#[allow(clippy::too_many_arguments)]
fn enumerate_prefixes(
    depth: usize,
    target_depth: usize,
    values: [u64; 2],
    used_terms: u8,
    operations: &mut Vec<Op>,
    all_operations: &[Op],
    table: &[u8],
    output: &mut HashMap<(u64, u64), Vec<Prefix>>,
) {
    if depth == target_depth {
        output
            .entry((values[0], values[1]))
            .or_default()
            .push(Prefix {
                operations: operations.clone(),
                used_terms,
            });
        return;
    }
    for operation in all_operations.iter().copied() {
        if should_skip(operations.last().copied(), operation) {
            continue;
        }
        let next_mask = if let Some(index) = term_index(operation) {
            let bit = 1 << index;
            if used_terms & bit != 0 {
                continue;
            }
            used_terms | bit
        } else {
            used_terms
        };
        operations.push(operation);
        enumerate_prefixes(
            depth + 1,
            target_depth,
            [
                apply(values[0], PAIRS[0].2, operation, table),
                apply(values[1], PAIRS[1].2, operation, table),
            ],
            next_mask,
            operations,
            all_operations,
            table,
            output,
        );
        operations.pop();
    }
}

#[allow(clippy::too_many_arguments)]
fn enumerate_suffixes(
    depth: usize,
    target_depth: usize,
    values: [u64; 2],
    used_terms: u8,
    reverse_operations: &mut Vec<Op>,
    all_operations: &[Op],
    inverse: &[u8],
    prefixes: &HashMap<(u64, u64), Vec<Prefix>>,
    total_length: usize,
    candidates: &mut Vec<Candidate>,
) {
    if candidates.len() >= 100 {
        return;
    }
    if depth == target_depth {
        if let Some(matches) = prefixes.get(&(values[0], values[1])) {
            for prefix in matches {
                if prefix.used_terms & used_terms != 0 {
                    continue;
                }
                let mut sequence = prefix.operations.clone();
                sequence.extend(reverse_operations.iter().rev());
                candidates.push(Candidate {
                    operation_count: total_length,
                    operations: sequence.into_iter().map(format_op).collect(),
                });
                if candidates.len() >= 100 {
                    break;
                }
            }
        }
        return;
    }
    for operation in all_operations.iter().copied() {
        if should_skip(reverse_operations.last().copied(), operation) {
            continue;
        }
        let next_mask = if let Some(index) = term_index(operation) {
            let bit = 1 << index;
            if used_terms & bit != 0 {
                continue;
            }
            used_terms | bit
        } else {
            used_terms
        };
        reverse_operations.push(operation);
        enumerate_suffixes(
            depth + 1,
            target_depth,
            [
                apply_inverse(values[0], PAIRS[0].2, operation, inverse),
                apply_inverse(values[1], PAIRS[1].2, operation, inverse),
            ],
            next_mask,
            reverse_operations,
            all_operations,
            inverse,
            prefixes,
            total_length,
            candidates,
        );
        reverse_operations.pop();
    }
}

fn format_op(operation: Op) -> String {
    match operation {
        Op::Add(index) => format!("add(ror_state_{})", index + 1),
        Op::Sub(index) => format!("sub(ror_state_{})", index + 1),
        Op::Xor(index) => format!("xor(ror_state_{})", index + 1),
        Op::XorNot(index) => format!("xor_not(ror_state_{})", index + 1),
        Op::Rol(index) => format!("rol_by(ror_state_{})", index + 1),
        Op::Ror(index) => format!("ror_by(ror_state_{})", index + 1),
        Op::Not => "not".into(),
        Op::Swap => "swap_adjacent_bits".into(),
        Op::Reverse => "reverse_bits".into(),
        Op::Substitute => "substitute_bytes".into(),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let max_length: usize = std::env::args().nth(1).as_deref().unwrap_or("7").parse()?;
    let table = table();
    let inverse = inverse_table(&table);
    let operations = all_ops();
    let mut results = Vec::new();
    let mut searched = Vec::new();

    for length in 1..=max_length {
        let prefix_depth = length / 2;
        let suffix_depth = length - prefix_depth;
        let mut prefixes = HashMap::new();
        enumerate_prefixes(
            0,
            prefix_depth,
            [PAIRS[0].0, PAIRS[1].0],
            0,
            &mut Vec::new(),
            &operations,
            &table,
            &mut prefixes,
        );
        enumerate_suffixes(
            0,
            suffix_depth,
            [PAIRS[0].1, PAIRS[1].1],
            0,
            &mut Vec::new(),
            &operations,
            &inverse,
            &prefixes,
            length,
            &mut results,
        );
        searched.push(serde_json::json!({
            "operation_count": length,
            "prefix_states": prefixes.values().map(Vec::len).sum::<usize>(),
            "distinct_prefix_outputs": prefixes.len(),
        }));
        if !results.is_empty() {
            break;
        }
    }

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "assumption": "the two cross-China-consistent initialization blocks share the analogous Global control plaintext",
            "pairs": [
                {"seed": PAIRS[0].2, "china_ciphertext_le": format!("{:016X}", PAIRS[0].0), "candidate_plaintext_le": format!("{:016X}", PAIRS[0].1)},
                {"seed": PAIRS[1].2, "china_ciphertext_le": format!("{:016X}", PAIRS[1].0), "candidate_plaintext_le": format!("{:016X}", PAIRS[1].1)},
            ],
            "grammar": "historical reversible word64 primitives with distinct ROR(state,1..8) terms",
            "searched": searched,
            "candidates": results,
        }))?
    );
    Ok(())
}
