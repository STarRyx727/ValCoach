//! Mine cross-fixture second-word constraints for the candidate China transform.

use std::{collections::HashSet, fs::File, io::BufRead, io::BufReader};

use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize)]
struct Sample {
    export_group_path: Option<String>,
    class_path: Option<String>,
    payload_bit_count: usize,
    seed: u32,
    raw_ciphertext_hex: String,
    decoded_hex: String,
}

#[derive(Serialize)]
struct Match {
    export_group_path: Option<String>,
    class_path: Option<String>,
    payload_bit_count: usize,
    seed: u32,
    china_first_word_ciphertext: String,
    common_first_word_plaintext: String,
    china_second_word_ciphertexts: Vec<String>,
    global_second_word_plaintext: String,
}

fn read(path: &str) -> Result<Vec<Sample>, Box<dyn std::error::Error>> {
    BufReader::new(File::open(path)?)
        .lines()
        .map(|line| Ok(serde_json::from_str(&line?)?))
        .collect()
}

fn first_word(value: &str) -> Option<u64> {
    let bytes = hex::decode(value).ok()?;
    Some(u64::from_le_bytes(bytes.get(..8)?.try_into().ok()?))
}

fn first_hex(value: &str) -> Option<String> {
    (value.len() >= 16).then(|| value[..16].to_uppercase())
}

fn second_hex(value: &str) -> Option<String> {
    (value.len() >= 32).then(|| value[16..32].to_uppercase())
}

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

fn same_shape(left: &Sample, right: &Sample) -> bool {
    left.export_group_path == right.export_group_path
        && left.class_path == right.class_path
        && left.payload_bit_count == right.payload_bit_count
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 6 {
        return Err("usage: china_pair_mine <global.jsonl> <china1.jsonl> <china2.jsonl> <china3.jsonl> <china4.jsonl>".into());
    }
    let global = read(&args[1])?;
    let china: Vec<Vec<Sample>> = args[2..]
        .iter()
        .map(|path| read(path))
        .collect::<Result<_, _>>()?;
    let mut seen = HashSet::new();
    let mut matches = Vec::new();

    for sample in china[0].iter().filter(|row| row.payload_bit_count >= 128) {
        let Some(cipher_word) = first_word(&sample.raw_ciphertext_hex) else {
            continue;
        };
        let cipher_hex = first_hex(&sample.raw_ciphertext_hex).unwrap();
        let signature = format!(
            "{}|{}|{}|{}|{}",
            sample.export_group_path.as_deref().unwrap_or_default(),
            sample.class_path.as_deref().unwrap_or_default(),
            sample.payload_bit_count,
            sample.seed,
            cipher_hex
        );
        if !seen.insert(signature) {
            continue;
        }
        let siblings: Vec<&Sample> = china
            .iter()
            .skip(1)
            .filter_map(|rows| {
                rows.iter().find(|row| {
                    same_shape(sample, row)
                        && row.seed == sample.seed
                        && first_hex(&row.raw_ciphertext_hex).as_deref()
                            == Some(cipher_hex.as_str())
                })
            })
            .collect();
        if siblings.len() != 3 {
            continue;
        }
        let plaintext = candidate_word64(cipher_word, sample.seed);
        let plaintext_hex = hex::encode_upper(plaintext.to_le_bytes());
        let global_matches: Vec<&Sample> = global
            .iter()
            .filter(|row| {
                same_shape(sample, row)
                    && first_hex(&row.decoded_hex).as_deref() == Some(plaintext_hex.as_str())
            })
            .collect();
        if global_matches.len() != 1 {
            continue;
        }
        let mut second_words = vec![second_hex(&sample.raw_ciphertext_hex).unwrap()];
        second_words.extend(
            siblings
                .iter()
                .map(|row| second_hex(&row.raw_ciphertext_hex).unwrap()),
        );
        matches.push(Match {
            export_group_path: sample.export_group_path.clone(),
            class_path: sample.class_path.clone(),
            payload_bit_count: sample.payload_bit_count,
            seed: sample.seed,
            china_first_word_ciphertext: cipher_hex,
            common_first_word_plaintext: plaintext_hex,
            china_second_word_ciphertexts: second_words,
            global_second_word_plaintext: second_hex(&global_matches[0].decoded_hex).unwrap(),
        });
    }

    println!("{}", serde_json::to_string_pretty(&matches)?);
    Ok(())
}
