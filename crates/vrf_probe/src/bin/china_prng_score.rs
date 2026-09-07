//! Score recovered China PRNG candidates over the first two RepLayout words.

use std::{fs::File, io::BufRead, io::BufReader};

use serde::{Deserialize, Serialize};

const MULTIPLIER: u64 = 0x2545_f491_4f6c_dd1d;
const CANDIDATES: &[(u32, bool)] = &[
    (0x267a_610f, false),
    (0xd275_21a9, false),
    (0xf677_61c9, true),
];

#[derive(Deserialize)]
struct Sample {
    export_group_path: Option<String>,
    payload_bit_count: usize,
    seed: u32,
    raw_ciphertext_hex: String,
    decoded_hex: String,
}

#[derive(Default, Serialize)]
struct Score {
    rows: usize,
    valid_prefix: usize,
    invalid_handle: usize,
    invalid_payload_length: usize,
    packed_values: usize,
    fields_seen: usize,
    fields_after_first_word: usize,
    terminators: usize,
}

fn ror_term(state: u32, rotation: u32) -> u64 {
    state.rotate_right(rotation) as u64
}

fn word64(value: u64, state: u32) -> u64 {
    let value = value.wrapping_add(ror_term(state, 8));
    let value = value.rotate_left((ror_term(state, 6) % 63 + 1) as u32);
    let value = value.rotate_right((ror_term(state, 7) % 63 + 1) as u32);
    let value = value.rotate_left((ror_term(state, 4) % 63 + 1) as u32);
    let value = value.rotate_right((ror_term(state, 2) % 63 + 1) as u32);
    value.wrapping_sub(ror_term(state, 1))
}

fn initial_a(seed: u32, seed_addend: u32, add_offset: bool) -> u64 {
    let seed_plus = seed.wrapping_add(seed_addend);
    let offset = seed_addend & 0xff;
    let offset_seed = if add_offset {
        seed.wrapping_add(offset)
    } else {
        seed.wrapping_sub(offset)
    };
    let mixed =
        (((seed_plus >> 15) ^ seed_plus) >> 12) ^ offset_seed.wrapping_mul(0x0200_0000) ^ seed_plus;
    u64::from(mixed).wrapping_mul(MULTIPLIER)
}

fn initial_b(seed: u32) -> u64 {
    let mixed = (((seed >> 15) ^ seed) >> 12) ^ seed.wrapping_shl(25) ^ seed;
    u64::from(mixed).wrapping_mul(MULTIPLIER)
}

fn state2(seed: u32, seed_addend: u32, add_offset: bool) -> u32 {
    (initial_a(seed, seed_addend, add_offset).wrapping_add(initial_b(seed)) >> 32) as u32
}

fn read_packed(bytes: &[u8; 16], position: &mut usize) -> Option<u32> {
    let mut value = 0u32;
    let mut shift = 0;
    for _ in 0..5 {
        if *position + 8 > 128 {
            return None;
        }
        let byte_index = *position / 8;
        let bit_offset = *position % 8;
        let low = u16::from(bytes[byte_index]);
        let high = bytes
            .get(byte_index + 1)
            .copied()
            .map(u16::from)
            .unwrap_or(0);
        let next = ((low | (high << 8)) >> bit_offset) as u8;
        *position += 8;
        value |= u32::from(next >> 1) << shift;
        if next & 1 == 0 {
            return Some(value);
        }
        shift += 7;
    }
    None
}

fn score_payload(bytes: &[u8; 16], payload_bits: usize, score: &mut Score) {
    score.rows += 1;
    let mut position = 1usize;
    loop {
        let Some(handle) = read_packed(bytes, &mut position) else {
            return;
        };
        score.packed_values += 1;
        if handle == 0 {
            score.terminators += 1;
            score.valid_prefix += 1;
            return;
        }
        if handle > 256 {
            score.invalid_handle += 1;
            return;
        }
        score.fields_seen += 1;
        if position > 64 {
            score.fields_after_first_word += 1;
        }
        let Some(field_bits) = read_packed(bytes, &mut position) else {
            return;
        };
        score.packed_values += 1;
        if field_bits == 0 || field_bits as usize > payload_bits.saturating_sub(position) {
            score.invalid_payload_length += 1;
            return;
        }
        let next = position.saturating_add(field_bits as usize);
        if next >= 128 {
            score.valid_prefix += 1;
            return;
        }
        position = next;
    }
}

fn bytes16(value: &str) -> Option<[u8; 16]> {
    hex::decode(value).ok()?.get(..16)?.try_into().ok()
}

fn score_file(
    path: &str,
    candidate: Option<(u32, bool)>,
) -> Result<Score, Box<dyn std::error::Error>> {
    let mut score = Score::default();
    for line in BufReader::new(File::open(path)?).lines() {
        let sample: Sample = serde_json::from_str(&line?)?;
        if sample.export_group_path.is_none() || sample.payload_bit_count < 128 {
            continue;
        }
        let Some(mut bytes) = candidate
            .map(|_| bytes16(&sample.raw_ciphertext_hex))
            .unwrap_or_else(|| bytes16(&sample.decoded_hex))
        else {
            continue;
        };
        if let Some((seed_addend, add_offset)) = candidate {
            let first = u64::from_le_bytes(bytes[..8].try_into().unwrap());
            let second = u64::from_le_bytes(bytes[8..].try_into().unwrap());
            bytes[..8].copy_from_slice(&word64(first, sample.seed).to_le_bytes());
            bytes[8..].copy_from_slice(
                &word64(second, state2(sample.seed, seed_addend, add_offset)).to_le_bytes(),
            );
        }
        score_payload(&bytes, sample.payload_bit_count, &mut score);
    }
    Ok(score)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        return Err("usage: china_prng_score <global.jsonl> <china.jsonl>".into());
    }
    let mut candidates = serde_json::Map::new();
    for (seed_addend, add_offset) in CANDIDATES {
        candidates.insert(
            format!(
                "0x{seed_addend:08X}_{}",
                if *add_offset { "add" } else { "subtract" }
            ),
            serde_json::to_value(score_file(&args[2], Some((*seed_addend, *add_offset)))?)?,
        );
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "global_decoded_baseline": score_file(&args[1], None)?,
            "china_candidates": candidates,
        }))?
    );
    Ok(())
}
