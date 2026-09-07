//! Read-only static scanner for constants anchoring VALORANT payload-transform functions.

use std::{collections::BTreeMap, env, fs, path::Path};

use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Serialize)]
struct BinaryScan {
    path: String,
    size_bytes: usize,
    sha256: String,
    anchors: BTreeMap<&'static str, AnchorHits>,
}

#[derive(Serialize)]
struct AnchorHits {
    count: usize,
    first_offsets: Vec<String>,
    first_context_hex: Option<String>,
}

const ANCHORS: &[(&str, &[u8])] = &[
    (
        "prng_multiplier_u64",
        &0x2545_f491_4f6c_dd1du64.to_le_bytes(),
    ),
    ("prng_multiplier_low_u32", &0x4f6c_dd1du32.to_le_bytes()),
    ("prng_multiplier_high_u32", &0x2545_f491u32.to_le_bytes()),
    ("seed_addend_12_10", &0x12fd_0ee5u32.to_le_bytes()),
    ("seed_addend_12_11", &0x409d_36a3u32.to_le_bytes()),
    ("seed_addend_13_00", &0x2949_b6efu32.to_le_bytes()),
    ("seed_addend_13_01", &0xe62f_cd5cu32.to_le_bytes()),
    ("seed_addend_13_02", &0x9e81_a37cu32.to_le_bytes()),
    ("seed_addend_13_04", &0xb6e1_3c58u32.to_le_bytes()),
    ("seed_addend_13_05", &0x48c2_6613u32.to_le_bytes()),
    ("byte_mix_1b0829", &0x001b_0829u32.to_le_bytes()),
];

fn find_all(data: &[u8], needle: &[u8]) -> AnchorHits {
    if needle.is_empty() || needle.len() > data.len() {
        return AnchorHits {
            count: 0,
            first_offsets: Vec::new(),
            first_context_hex: None,
        };
    }
    let mut count = 0;
    let mut first_offsets = Vec::new();
    let mut first_context_hex = None;
    for (offset, window) in data.windows(needle.len()).enumerate() {
        if window == needle {
            count += 1;
            if first_context_hex.is_none() {
                let start = offset.saturating_sub(32);
                let end = (offset + needle.len() + 32).min(data.len());
                first_context_hex = Some(hex::encode_upper(&data[start..end]));
            }
            if first_offsets.len() < 32 {
                first_offsets.push(format!("0x{offset:08X}"));
            }
        }
    }
    AnchorHits {
        count,
        first_offsets,
        first_context_hex,
    }
}

fn scan(path: &Path) -> Result<BinaryScan, Box<dyn std::error::Error>> {
    let data = fs::read(path)?;
    let anchors = ANCHORS
        .iter()
        .map(|(name, pattern)| (*name, find_all(&data, pattern)))
        .collect();
    Ok(BinaryScan {
        path: path.display().to_string(),
        size_bytes: data.len(),
        sha256: hex::encode(Sha256::digest(&data)),
        anchors,
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let paths: Vec<_> = env::args_os().skip(1).collect();
    if paths.is_empty() {
        return Err("usage: binary_anchor_scan <global.exe> [china.exe]".into());
    }
    let reports = paths
        .iter()
        .map(|path| scan(Path::new(path)))
        .collect::<Result<Vec<_>, _>>()?;
    println!("{}", serde_json::to_string_pretty(&reports)?);
    Ok(())
}
