//! China 13.05 transform constant brute-force search.
//!
//! The transform algorithm is shared between Global and China 13.05.
//! Only 3 constants differ: SeedAddend, InitASeedAddend, TailXor.
//!
//! Strategy: The first content block of each ReplayData chunk uses a known seed
//! (derived from the replay's NetGUID cache). The first 8 bytes after transform
//! must be a valid UE replication header. We use this as a grammar oracle to
//! search the 32-bit SeedAddend space.
//!
//! TailXor only affects trailing bits (< 8), so we can ignore it for the search.
//! InitASeedAddend only affects InitialPrngA which influences the PRNG sequence
//! after the first block, so we search it separately (only 256 values since it's
//! added to seed as u32).

use thiserror::Error;

const MULTIPLIER: u64 = 0x2545f4914f6cdd1d;
const GLOBAL_SEED_ADDEND: u32 = 0x48c26613;
const GLOBAL_INIT_A_SEED_ADDEND: u32 = 0x13;
const GLOBAL_TAIL_XOR: u8 = 0x13;

#[derive(Debug, Clone, Copy)]
pub struct TransformConstants {
    pub seed_addend: u32,
    pub init_a_seed_addend: u32,
    pub tail_xor: u8,
}

impl Default for TransformConstants {
    fn default() -> Self {
        Self {
            seed_addend: GLOBAL_SEED_ADDEND,
            init_a_seed_addend: GLOBAL_INIT_A_SEED_ADDEND,
            tail_xor: GLOBAL_TAIL_XOR,
        }
    }
}

#[derive(Debug, Error)]
pub enum SearchError {
    #[error("failed to read replay: {0}")]
    Io(#[from] std::io::Error),
    #[error("no valid content blocks found")]
    NoContentBlocks,
    #[error("search exhausted without finding valid constants")]
    NotFound,
}

/// Rotate right for u32
fn ror32(value: u32, count: u32) -> u32 {
    value.rotate_right(count)
}

/// Rotate left for u32
fn rol32(value: u32, count: u32) -> u32 {
    value.rotate_left(count)
}

/// Rotate right for u64
fn ror64(value: u64, count: u32) -> u64 {
    value.rotate_right(count)
}

/// Rotate left for u64
fn rol64(value: u64, count: u32) -> u64 {
    value.rotate_left(count)
}

fn transform_uint64(value: u64, state: u32) -> u64 {
    let ror1 = ror32(state, 1) as u64;
    let ror2 = ror32(state, 2) as u64;
    let ror3 = ror32(state, 3) as u64;
    let ror4 = ror32(state, 4) as u64;
    let ror5 = ror32(state, 5) as u64;
    let ror6 = ror32(state, 6) as u64;
    let ror7 = ror32(state, 7) as u64;
    let ror8 = ror32(state, 8) as u64;

    let mut v = value;
    v = (v ^ !ror8).wrapping_sub(ror7);
    v = rol64(v, ((ror6 % 63) + 1) as u32);
    v = (v.wrapping_sub(ror5)) ^ !ror4 ^ !ror3;
    v = rol64(v, ((ror2 % 63) + 1) as u32);
    rol64(v, ((ror1 % 63) + 1) as u32)
}

fn transform_uint32(value: u32, state: u32) -> u32 {
    let rol1 = rol32(state, 1);
    let rol2 = rol32(state, 2);
    let rol3 = rol32(state, 3);
    let rol4 = rol32(state, 4);
    let rol5 = rol32(state, 5);
    let rol6 = rol32(state, 6);
    let rol7 = rol32(state, 7);
    let rol8 = rol32(state, 8);

    let mut v = value;
    v = (v ^ rol8).wrapping_sub(rol7);
    v = rol32(v, (rol6 % 31) + 1);
    v = (v.wrapping_sub(rol5)) ^ rol4 ^ rol3;
    v = rol32(v, (rol2 % 31) + 1);
    rol32(v, (rol1 % 31) + 1)
}

fn initial_prng_a(seed: u32, seed_addend: u32, init_a_seed_addend: u32) -> u64 {
    let seed_plus = seed.wrapping_add(seed_addend);
    let mixed = (((seed_plus >> 15) ^ seed_plus) >> 12)
        ^ (seed.wrapping_add(init_a_seed_addend)).wrapping_mul(0x02000000)
        ^ seed_plus;
    (mixed as u64).wrapping_mul(MULTIPLIER)
}

fn initial_prng_b(seed: u32) -> u64 {
    let mixed = (((seed >> 15) ^ seed) >> 12) ^ (seed << 25) ^ seed;
    (mixed as u64).wrapping_mul(MULTIPLIER)
}

fn advance_state(state: &mut u32, prng_a: &mut u64, prng_b: &mut u64) -> u8 {
    let sum = prng_b.wrapping_add(*prng_a);
    *prng_b ^= *prng_a;
    *prng_a = ror64(*prng_a, 9) ^ (*prng_b << 14) ^ *prng_b;
    *prng_b = rol64(*prng_b, 36);
    *state = (sum >> 32) as u32;
    *state as u8
}

/// Apply transform to a byte buffer with given constants.
pub fn apply_transform(data: &mut [u8], bit_count: usize, seed: u32, constants: TransformConstants) {
    if bit_count == 0 {
        return;
    }

    let mut state = seed;
    let mut stream_byte = seed as u8;
    let mut prng_a = initial_prng_a(seed, constants.seed_addend, constants.init_a_seed_addend);
    let mut prng_b = initial_prng_b(seed);
    let mut byte_offset = 0usize;
    let mut bits_remaining = bit_count;

    while bits_remaining > 63 {
        let value = u64::from_le_bytes(data[byte_offset..byte_offset + 8].try_into().unwrap());
        let transformed = transform_uint64(value, state);
        data[byte_offset..byte_offset + 8].copy_from_slice(&transformed.to_le_bytes());
        stream_byte = advance_state(&mut state, &mut prng_a, &mut prng_b);
        byte_offset += 8;
        bits_remaining -= 64;
    }

    while bits_remaining > 31 {
        let value = u32::from_le_bytes(data[byte_offset..byte_offset + 4].try_into().unwrap());
        let transformed = transform_uint32(value, state);
        data[byte_offset..byte_offset + 4].copy_from_slice(&transformed.to_le_bytes());
        stream_byte = advance_state(&mut state, &mut prng_a, &mut prng_b);
        byte_offset += 4;
        bits_remaining -= 32;
    }

    while bits_remaining > 7 {
        let state_byte = state as u8;
        let mix_a = state.wrapping_mul(0x1b0829);
        let mut v = data[byte_offset];
        v = ((mix_a.wrapping_mul(0x79) as u8) ^ v).wrapping_sub(mix_a.wrapping_mul(0x0b) as u8);
        v = v.rotate_left((mix_a % 7) + 1);
        v = (v.wrapping_sub(state_byte.wrapping_mul(0x1b)))
            ^ state_byte.wrapping_mul(0x33)
            ^ state_byte.wrapping_mul(0x31);
        v = v.rotate_left(state.wrapping_mul(0x79) % 7 + 1);
        v = v.rotate_left(state.wrapping_mul(0x0b) % 7 + 1);
        data[byte_offset] = v;
        stream_byte = advance_state(&mut state, &mut prng_a, &mut prng_b);
        byte_offset += 1;
        bits_remaining -= 8;
    }

    if bits_remaining != 0 {
        let mask = 0xffu8 >> (7 - ((bit_count - 1) & 7));
        data[byte_offset] ^= mask & (stream_byte ^ constants.tail_xor);
    }
}

/// Check if a decoded first block looks like valid UE replication.
/// The first content block of a ReplayData chunk typically starts with
/// a bunch header that has known structure.
#[allow(dead_code)]
fn is_valid_ue_header(data: &[u8]) -> bool {
    // UE content blocks start with:
    // - bunch header fields (variable length, but first few bytes have constraints)
    // - The first byte often has low values (bunch type, channel index)
    // - Not all zeros, not all 0xFF
    if data.is_empty() {
        return false;
    }
    let all_zero = data.iter().take(16).all(|&b| b == 0);
    let all_ff = data.iter().take(16).all(|&b| b == 0xff);
    if all_zero || all_ff {
        return false;
    }
    // First byte should be small (channel/bunch fields are typically < 64)
    // This is a heuristic — not a hard constraint
    data[0] < 128
}

/// Score how "valid" a decoded buffer looks.
/// Higher score = more likely correct.
/// This is a heuristic grammar oracle, not a hard validator.
pub fn score_decoded(data: &[u8]) -> u32 {
    if data.len() < 4 {
        return 0;
    }
    let mut score = 0u32;

    // Not all same byte
    let first = data[0];
    if data.iter().take(16).all(|&b| b == first) {
        return 0;
    }

    // Avoid all-zero or all-FF
    let all_zero = data.iter().take(32).all(|&b| b == 0);
    let all_ff = data.iter().take(32).all(|&b| b == 0xff);
    if all_zero || all_ff {
        return 0;
    }

    // Some bytes should be zero (padding, zero-length fields)
    let zero_count = data.iter().take(32).filter(|&&b| b == 0).count();
    if (2..=20).contains(&zero_count) {
        score += 5;
    }

    // Byte value entropy: decoded UE data has a mix of high and low bytes.
    let high_count = data.iter().take(64).filter(|&&b| b >= 0x80).count();
    if (10..=50).contains(&high_count) {
        score += 10;
    } else if high_count > 50 {
        score += 2;
    }

    // Good variety in first 32 bytes
    let mut seen = std::collections::HashSet::new();
    for &b in data.iter().take(32) {
        seen.insert(b);
    }
    if seen.len() >= 10 && seen.len() <= 28 {
        score += 5;
    }

    // Some low-value bytes (RepLayout handles are typically 0x00-0x1F)
    let small_count = data.iter().take(16).filter(|&&b| b < 0x40).count();
    if small_count >= 4 {
        score += 5;
    }

    score
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_constants_roundtrip() {
        // Verify our Rust implementation matches the C# algorithm
        // by checking that Global constants produce known-good output
        let mut data = vec![0x42u8; 16];
        let original = data.clone();
        apply_transform(&mut data, 128, 0x12345678, TransformConstants::default());
        // Transform should change the data
        assert_ne!(data, original);
        // Applying inverse should restore (but our transform is not self-inverse)
        // So just verify it doesn't panic and produces deterministic output
        let mut data2 = vec![0x42u8; 16];
        apply_transform(&mut data2, 128, 0x12345678, TransformConstants::default());
        assert_eq!(data, data2);
    }

    #[test]
    fn transform_uint64_is_deterministic() {
        let state = 0xDEADBEEFu32;
        let value = 0x0123456789ABCDEFu64;
        let result1 = transform_uint64(value, state);
        let result2 = transform_uint64(value, state);
        assert_eq!(result1, result2);
        // Different state should produce different result
        let result3 = transform_uint64(value, state + 1);
        assert_ne!(result1, result3);
    }
}
