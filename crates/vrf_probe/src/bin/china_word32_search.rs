//! Synthesize the China 13.05 UInt32 transform from three cross-fixture pairs.

use serde::Serialize;

const PAIRS: [(u32, u32, u32); 3] = [
    (0x1a20_9b66, 0x61a2_0c10, 43),
    (0x90ff_f017, 0x1002_2115, 50),
    (0x80ff_f017, 0x1002_2114, 50),
];

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum Arithmetic {
    Add,
    Sub,
    Xor,
    XorNot,
}

#[derive(Serialize)]
struct Candidate {
    first: Arithmetic,
    rotations: [char; 4],
    last: Arithmetic,
    state_terms: [u8; 6],
}

fn term(state: u32, rotation: u8) -> u32 {
    state.rotate_left(u32::from(rotation))
}

fn arithmetic(value: u32, term: u32, operation: Arithmetic) -> u32 {
    match operation {
        Arithmetic::Add => value.wrapping_add(term),
        Arithmetic::Sub => value.wrapping_sub(term),
        Arithmetic::Xor => value ^ term,
        Arithmetic::XorNot => value ^ !term,
    }
}

fn apply(
    mut value: u32,
    state: u32,
    first: Arithmetic,
    rotation_bits: u8,
    last: Arithmetic,
    assignment: &[u8; 6],
) -> u32 {
    let terms = assignment.map(|rotation| term(state, rotation));
    value = arithmetic(value, terms[0], first);
    for index in 0..4 {
        let count = terms[index + 1] % 31 + 1;
        value = if rotation_bits & (1 << index) == 0 {
            value.rotate_left(count)
        } else {
            value.rotate_right(count)
        };
    }
    arithmetic(value, terms[5], last)
}

#[allow(clippy::too_many_arguments)]
fn assignments(
    depth: usize,
    assignment: &mut [u8; 6],
    used: u8,
    first: Arithmetic,
    rotation_bits: u8,
    last: Arithmetic,
    candidates: &mut Vec<Candidate>,
) {
    if depth == assignment.len() {
        if PAIRS.iter().all(|(ciphertext, plaintext, state)| {
            apply(*ciphertext, *state, first, rotation_bits, last, assignment) == *plaintext
        }) {
            candidates.push(Candidate {
                first,
                rotations: std::array::from_fn(|index| {
                    if rotation_bits & (1 << index) == 0 {
                        'L'
                    } else {
                        'R'
                    }
                }),
                last,
                state_terms: *assignment,
            });
        }
        return;
    }
    for rotation in 1..=8 {
        let bit = 1 << (rotation - 1);
        if used & bit != 0 {
            continue;
        }
        assignment[depth] = rotation;
        assignments(
            depth + 1,
            assignment,
            used | bit,
            first,
            rotation_bits,
            last,
            candidates,
        );
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arithmetic_ops = [
        Arithmetic::Add,
        Arithmetic::Sub,
        Arithmetic::Xor,
        Arithmetic::XorNot,
    ];
    let mut candidates = Vec::new();
    for first in arithmetic_ops {
        for rotation_bits in 0..16 {
            for last in arithmetic_ops {
                assignments(
                    0,
                    &mut [0; 6],
                    0,
                    first,
                    rotation_bits,
                    last,
                    &mut candidates,
                );
            }
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "pairs": PAIRS.iter().map(|(ciphertext, plaintext, seed)| serde_json::json!({
                "ciphertext_le": format!("0x{ciphertext:08X}"),
                "plaintext_le": format!("0x{plaintext:08X}"),
                "seed": seed,
            })).collect::<Vec<_>>(),
            "search_space": "arithmetic + four value rotations + arithmetic, distinct ROL(state,1..8) terms",
            "candidates": candidates,
        }))?
    );
    Ok(())
}
