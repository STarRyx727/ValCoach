//! Score candidate China first-word transforms against the RepLayout field grammar.

use std::{fs::File, io::BufRead, io::BufReader};

use serde::{Deserialize, Serialize};
use valcoach_vrf_probe::china_transform::transform_uint64 as global_word64;

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
    valid_first_handle: usize,
    valid_first_payload_length: usize,
    valid_prefix: usize,
    terminators: usize,
    fields_seen: usize,
    first_handle_sum: u64,
    first_handle_max: u32,
    first_payload_bits_sum: u64,
    first_payload_bits_max: u32,
    total_points: u64,
}

fn ror_term(state: u32, rotation: u32) -> u64 {
    state.rotate_right(rotation) as u64
}

fn candidate_common(value: u64, state: u32) -> u64 {
    let value = value.wrapping_add(ror_term(state, 8));
    let value = value.rotate_left((ror_term(state, 6) % 63 + 1) as u32);
    let value = value.rotate_right((ror_term(state, 7) % 63 + 1) as u32);
    let value = value.rotate_left((ror_term(state, 4) % 63 + 1) as u32);
    value.rotate_right((ror_term(state, 2) % 63 + 1) as u32)
}

fn candidate_sub(value: u64, state: u32) -> u64 {
    candidate_common(value, state).wrapping_sub(ror_term(state, 1))
}

fn candidate_xor(value: u64, state: u32) -> u64 {
    candidate_common(value, state) ^ ror_term(state, 1)
}

fn read_packed(word: u64, position: &mut usize) -> Option<u32> {
    let mut value = 0u32;
    let mut shift = 0;
    for _ in 0..5 {
        if *position + 8 > 64 {
            return None;
        }
        let next = ((word >> *position) & 0xff) as u8;
        *position += 8;
        value |= u32::from(next >> 1) << shift;
        if next & 1 == 0 {
            return Some(value);
        }
        shift += 7;
    }
    None
}

fn score_word(word: u64, payload_bits: usize, score: &mut Score) {
    score.rows += 1;
    score.total_points += 1; // The checksum bit always exists for RepLayout payloads.
    let mut position = 1usize;
    let mut fields = 0usize;
    loop {
        let Some(encoded_handle) = read_packed(word, &mut position) else {
            return;
        };
        if fields == 0 && encoded_handle <= 4096 {
            score.valid_first_handle += 1;
            score.first_handle_sum += u64::from(encoded_handle);
            score.first_handle_max = score.first_handle_max.max(encoded_handle);
            score.total_points += 3;
        }
        if encoded_handle == 0 {
            score.terminators += 1;
            score.valid_prefix += 1;
            score.total_points += 4;
            return;
        }
        if encoded_handle > 4096 {
            return;
        }
        fields += 1;
        let Some(field_bits) = read_packed(word, &mut position) else {
            return;
        };
        if field_bits == 0 || field_bits as usize > payload_bits.saturating_sub(position) {
            return;
        }
        if fields == 1 {
            score.valid_first_payload_length += 1;
            score.first_payload_bits_sum += u64::from(field_bits);
            score.first_payload_bits_max = score.first_payload_bits_max.max(field_bits);
            score.total_points += 4;
        }
        score.fields_seen += 1;
        let next = position.saturating_add(field_bits as usize);
        if next >= 64 {
            score.valid_prefix += 1;
            score.total_points += 5;
            return;
        }
        position = next;
    }
}

fn first_word(hex_text: &str) -> Option<u64> {
    let bytes = hex::decode(hex_text).ok()?;
    let first: [u8; 8] = bytes.get(..8)?.try_into().ok()?;
    Some(u64::from_le_bytes(first))
}

fn score_samples(
    path: &str,
    source: fn(&Sample) -> Option<u64>,
    transform: fn(u64, u32) -> u64,
) -> Result<Score, Box<dyn std::error::Error>> {
    let mut score = Score::default();
    for line in BufReader::new(File::open(path)?).lines() {
        let sample: Sample = serde_json::from_str(&line?)?;
        if sample.export_group_path.is_none() || sample.payload_bit_count < 64 {
            continue;
        }
        if let Some(value) = source(&sample) {
            score_word(
                transform(value, sample.seed),
                sample.payload_bit_count,
                &mut score,
            );
        }
    }
    Ok(score)
}

fn identity(value: u64, _: u32) -> u64 {
    value
}

fn raw(sample: &Sample) -> Option<u64> {
    first_word(&sample.raw_ciphertext_hex)
}

fn decoded(sample: &Sample) -> Option<u64> {
    first_word(&sample.decoded_hex)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        return Err("usage: china_word64_score <global.jsonl> <china.jsonl>".into());
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "global_decoded_baseline": score_samples(&args[1], decoded, identity)?,
            "china_with_global_word64": score_samples(&args[2], raw, global_word64)?,
            "china_candidate_sub": score_samples(&args[2], raw, candidate_sub)?,
            "china_candidate_xor": score_samples(&args[2], raw, candidate_xor)?,
        }))?
    );
    Ok(())
}
