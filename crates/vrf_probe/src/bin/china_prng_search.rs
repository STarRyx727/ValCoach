//! Search China 13.05 PRNG initialization constants from a recovered second-word state.

use std::{thread, time::Instant};

const MULTIPLIER: u64 = 0x2545_f491_4f6c_dd1d;

fn initial_b(seed: u32) -> u64 {
    let mixed = (((seed >> 15) ^ seed) >> 12) ^ seed.wrapping_shl(25) ^ seed;
    u64::from(mixed).wrapping_mul(MULTIPLIER)
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        return Err("usage: china_prng_search <seed> <state2-hex> <threads>".into());
    }
    let seed: u32 = args[1].parse()?;
    let target_state = u32::from_str_radix(args[2].trim_start_matches("0x"), 16)?;
    let thread_count: u64 = args[3].parse()?;
    if thread_count == 0 || thread_count > 256 {
        return Err("threads must be between 1 and 256".into());
    }
    let total = u64::from(u32::MAX) + 1;
    let b = initial_b(seed);
    let started = Instant::now();
    let mut workers = Vec::new();
    for worker in 0..thread_count {
        let start = total * worker / thread_count;
        let end = total * (worker + 1) / thread_count;
        workers.push(thread::spawn(move || {
            let mut matches = Vec::new();
            for raw in start..end {
                let seed_addend = raw as u32;
                for add_offset in [false, true] {
                    let state =
                        (b.wrapping_add(initial_a(seed, seed_addend, add_offset)) >> 32) as u32;
                    if state == target_state {
                        matches.push((seed_addend, add_offset));
                    }
                }
            }
            matches
        }));
    }
    let mut matches = Vec::new();
    for worker in workers {
        matches.extend(worker.join().map_err(|_| "PRNG-search worker panicked")?);
    }
    matches.sort_unstable();
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "seed": seed,
            "target_state2": format!("0x{target_state:08X}"),
            "assumptions": {
                "multiplier": format!("0x{MULTIPLIER:016X}"),
                "initial_b": "shared historical helper",
                "init_a_offset": "low byte of seed addend",
                "offset_signs_tested": ["subtract", "add"],
            },
            "searched_seed_addends": total,
            "elapsed_milliseconds": started.elapsed().as_millis(),
            "matches": matches.into_iter().map(|(seed_addend, add_offset)| serde_json::json!({
                "seed_addend": format!("0x{seed_addend:08X}"),
                "init_a_offset": format!("0x{:02X}", seed_addend & 0xff),
                "offset_operation": if add_offset { "add" } else { "subtract" },
                "tail_xor_historical_hypothesis": format!("0x{:02X}", seed_addend & 0xff),
            })).collect::<Vec<_>>(),
        }))?
    );
    Ok(())
}
