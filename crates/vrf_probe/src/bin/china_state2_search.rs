//! Exhaustive state search for the second China 13.05 word.
//!
//! The first five bytes of the first BaseReplayController payload are stable across all four China
//! fixtures. Matching the analogous Global plaintext supplies a 40-bit constraint over a 32-bit
//! state. A result remains a hypothesis until it also satisfies independent payloads.

use std::{sync::Arc, thread, time::Instant};

fn ror_term(state: u32, rotation: u32) -> u64 {
    state.rotate_right(rotation) as u64
}

fn candidate_word64(value: u64, state: u32) -> u64 {
    let value = value.wrapping_add(ror_term(state, 8));
    let value = value.rotate_left((ror_term(state, 6) % 63 + 1) as u32);
    let value = value.rotate_right((ror_term(state, 7) % 63 + 1) as u32);
    let value = value.rotate_left((ror_term(state, 4) % 63 + 1) as u32);
    let value = value.rotate_right((ror_term(state, 2) % 63 + 1) as u32);
    value.wrapping_sub(ror_term(state, 1))
}

fn parse_u64_le(value: &str) -> Result<u64, Box<dyn std::error::Error>> {
    let bytes = hex::decode(value)?;
    let array: [u8; 8] = bytes.as_slice().try_into()?;
    Ok(u64::from_le_bytes(array))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        return Err(
            "usage: china_state2_search <ciphertext-8-byte-hex> <plaintext-prefix-hex> <threads>"
                .into(),
        );
    }
    let ciphertext = parse_u64_le(&args[1])?;
    let prefix = hex::decode(&args[2])?;
    if prefix.is_empty() || prefix.len() > 8 {
        return Err("plaintext prefix must contain 1 to 8 bytes".into());
    }
    let mut target_bytes = [0u8; 8];
    target_bytes[..prefix.len()].copy_from_slice(&prefix);
    let target = u64::from_le_bytes(target_bytes);
    let mask = if prefix.len() == 8 {
        u64::MAX
    } else {
        (1u64 << (prefix.len() * 8)) - 1
    };
    let threads: u64 = args[3].parse()?;
    if threads == 0 || threads > 256 {
        return Err("threads must be between 1 and 256".into());
    }

    let started = Instant::now();
    let mut workers = Vec::new();
    let target = Arc::new((ciphertext, target, mask));
    let total = u64::from(u32::MAX) + 1;
    for worker in 0..threads {
        let shared = Arc::clone(&target);
        let start = total * worker / threads;
        let end = total * (worker + 1) / threads;
        workers.push(thread::spawn(move || {
            let mut matches = Vec::new();
            for candidate in start..end {
                let decoded = candidate_word64(shared.0, candidate as u32);
                if decoded & shared.2 == shared.1 & shared.2 {
                    matches.push((candidate as u32, decoded));
                }
            }
            matches
        }));
    }

    let mut matches = Vec::new();
    for worker in workers {
        matches.extend(worker.join().map_err(|_| "state-search worker panicked")?);
    }
    matches.sort_unstable();
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "ciphertext_hex": args[1].to_uppercase(),
            "plaintext_prefix_hex": args[2].to_uppercase(),
            "constraint_bits": prefix.len() * 8,
            "searched_states": total,
            "threads": threads,
            "elapsed_milliseconds": started.elapsed().as_millis(),
            "matches": matches.into_iter().map(|(state, decoded)| serde_json::json!({
                "state": format!("0x{state:08X}"),
                "decoded_hex": hex::encode_upper(decoded.to_le_bytes()),
            })).collect::<Vec<_>>(),
        }))?
    );
    Ok(())
}
