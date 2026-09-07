//! Vector-constrained search for the China 13.05 byte transform.

use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader},
};

use serde::Deserialize;

#[derive(Clone, Copy)]
struct Pair {
    ciphertext: u8,
    plaintext: u8,
    state: u32,
}

#[derive(Deserialize)]
struct Sample {
    payload_bit_count: usize,
    seed: u32,
    raw_ciphertext_hex: String,
}

#[derive(Clone, Copy)]
enum Source {
    Mul(u32),
    Rol(u32),
    Ror(u32),
}

#[derive(Clone, Copy)]
enum Primitive {
    Add(Source),
    Sub(Source),
    Xor(Source),
    Rol(Source),
    Ror(Source),
    Not,
    Swap,
    Reverse,
    Substitute,
    InverseSubstitute,
}

#[derive(Clone)]
struct NamedSource {
    name: String,
    source: Source,
    vector: Vec<u8>,
}

fn source_value(source: Source, state: u32) -> u32 {
    match source {
        Source::Mul(value) => state.wrapping_mul(value),
        Source::Rol(value) => state.rotate_left(value),
        Source::Ror(value) => state.rotate_right(value),
    }
}

fn swap(value: u8) -> u8 {
    ((value & 0x55) << 1) | ((value >> 1) & 0x55)
}

fn substitute_table() -> Vec<u8> {
    hex::decode(concat!(
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
    .unwrap()
}

fn primitive_name(primitive: Primitive) -> String {
    let source_name = |source| match source {
        Source::Mul(value) => format!("mul(0x{value:X})"),
        Source::Rol(value) => format!("rol32({value})"),
        Source::Ror(value) => format!("ror32({value})"),
    };
    match primitive {
        Primitive::Add(source) => format!("add({})", source_name(source)),
        Primitive::Sub(source) => format!("sub({})", source_name(source)),
        Primitive::Xor(source) => format!("xor({})", source_name(source)),
        Primitive::Rol(source) => format!("rol({})", source_name(source)),
        Primitive::Ror(source) => format!("ror({})", source_name(source)),
        Primitive::Not => "not".into(),
        Primitive::Swap => "swap".into(),
        Primitive::Reverse => "reverse".into(),
        Primitive::Substitute => "substitute".into(),
        Primitive::InverseSubstitute => "inverse_substitute".into(),
    }
}

fn inverse_primitive(primitive: Primitive) -> Primitive {
    match primitive {
        Primitive::Add(source) => Primitive::Sub(source),
        Primitive::Sub(source) => Primitive::Add(source),
        Primitive::Xor(source) => Primitive::Xor(source),
        Primitive::Rol(source) => Primitive::Ror(source),
        Primitive::Ror(source) => Primitive::Rol(source),
        Primitive::Not => Primitive::Not,
        Primitive::Swap => Primitive::Swap,
        Primitive::Reverse => Primitive::Reverse,
        Primitive::Substitute => Primitive::InverseSubstitute,
        Primitive::InverseSubstitute => Primitive::Substitute,
    }
}

fn apply_primitive(
    value: u8,
    state: u32,
    primitive: Primitive,
    table: &[u8],
    inverse_table: &[u8; 256],
) -> u8 {
    match primitive {
        Primitive::Add(source) => value.wrapping_add(source_value(source, state) as u8),
        Primitive::Sub(source) => value.wrapping_sub(source_value(source, state) as u8),
        Primitive::Xor(source) => value ^ source_value(source, state) as u8,
        Primitive::Rol(source) => value.rotate_left(source_value(source, state) % 7 + 1),
        Primitive::Ror(source) => value.rotate_right(source_value(source, state) % 7 + 1),
        Primitive::Not => !value,
        Primitive::Swap => swap(value),
        Primitive::Reverse => value.reverse_bits(),
        Primitive::Substitute => table[value as usize],
        Primitive::InverseSubstitute => inverse_table[value as usize],
    }
}

fn apply_program(
    pairs: &[Pair],
    program: &[Primitive],
    table: &[u8],
    inverse_table: &[u8; 256],
) -> bool {
    pairs.iter().all(|pair| {
        let value = program.iter().fold(pair.plaintext, |value, primitive| {
            apply_primitive(value, pair.state, *primitive, table, inverse_table)
        });
        value == pair.ciphertext
    })
}

fn known_sources(pairs: &[Pair], counts: bool) -> Vec<NamedSource> {
    let mut raw_sources = vec![
        Source::Mul(0x0b),
        Source::Mul(0x79),
        Source::Mul(0x533),
        Source::Mul(0x1b0829),
        Source::Mul(0x2751b),
        Source::Mul(0x0cc6db61),
        Source::Mul(0x1b),
        Source::Mul(0x33),
        Source::Mul(0x31),
        Source::Mul(0x29),
        Source::Mul(0x23),
        Source::Mul(0x61),
        Source::Mul(0xc9),
        Source::Mul(0xf67761c9),
    ];
    for rotation in 1..=8 {
        raw_sources.push(Source::Rol(rotation));
        raw_sources.push(Source::Ror(rotation));
    }
    let mut unique = HashMap::<Vec<u8>, NamedSource>::new();
    for source in raw_sources {
        let vector = pairs
            .iter()
            .map(|pair| {
                let value = source_value(source, pair.state);
                if counts {
                    (value % 7 + 1) as u8
                } else {
                    value as u8
                }
            })
            .collect::<Vec<_>>();
        unique.entry(vector.clone()).or_insert(NamedSource {
            name: primitive_name(if counts {
                Primitive::Rol(source)
            } else {
                Primitive::Add(source)
            }),
            source,
            vector,
        });
    }
    unique.into_values().collect()
}

fn search_13_05_combined(pairs: &[Pair]) -> Option<String> {
    let compact = &pairs[..pairs.len().min(12)];
    let terms = known_sources(compact, false);
    let counts = known_sources(compact, true);
    let mut forward = HashMap::<Vec<u8>, (usize, usize, usize)>::new();
    for (xor_index, xor) in terms.iter().enumerate() {
        for (subtract_index, subtract) in terms.iter().enumerate() {
            for (count_index, count) in counts.iter().enumerate() {
                let vector = compact
                    .iter()
                    .map(|pair| {
                        let value = (pair.ciphertext ^ apply_source(xor.source, pair.state))
                            .wrapping_sub(apply_source(subtract.source, pair.state));
                        value.rotate_left(apply_count(count.source, pair.state))
                    })
                    .collect();
                forward
                    .entry(vector)
                    .or_insert((xor_index, subtract_index, count_index));
            }
        }
    }

    let mut xor_pairs = HashMap::<Vec<u8>, (usize, usize)>::new();
    for (left_index, left) in terms.iter().enumerate() {
        for (right_index, right) in terms.iter().enumerate().skip(left_index) {
            let vector = left
                .vector
                .iter()
                .zip(&right.vector)
                .map(|(left, right)| left ^ right)
                .collect();
            xor_pairs.entry(vector).or_insert((left_index, right_index));
        }
    }
    let mut count_pairs = HashMap::<Vec<u8>, (usize, usize)>::new();
    for (left_index, left) in counts.iter().enumerate() {
        for (right_index, right) in counts.iter().enumerate().skip(left_index) {
            let vector = left
                .vector
                .iter()
                .zip(&right.vector)
                .map(|(left, right)| (left + right) & 7)
                .collect();
            count_pairs
                .entry(vector)
                .or_insert((left_index, right_index));
        }
    }

    for (count_vector, (count2_index, count3_index)) in count_pairs {
        for (xor_vector, (xor2_index, xor3_index)) in &xor_pairs {
            for add in &terms {
                let required = compact
                    .iter()
                    .enumerate()
                    .map(|(index, pair)| {
                        pair.plaintext.rotate_right(u32::from(count_vector[index]))
                            ^ xor_vector[index]
                    })
                    .enumerate()
                    .map(|(index, value)| value.wrapping_add(add.vector[index]))
                    .collect::<Vec<_>>();
                let Some((xor1_index, subtract1_index, count1_index)) =
                    forward.get(&required).copied()
                else {
                    continue;
                };
                let xor1 = &terms[xor1_index];
                let subtract1 = &terms[subtract1_index];
                let count1 = &counts[count1_index];
                let xor2 = &terms[*xor2_index];
                let xor3 = &terms[*xor3_index];
                let count2 = &counts[count2_index];
                let count3 = &counts[count3_index];
                let valid = pairs.iter().all(|pair| {
                    let mut value = (pair.ciphertext ^ apply_source(xor1.source, pair.state))
                        .wrapping_sub(apply_source(subtract1.source, pair.state));
                    value = value.rotate_left(apply_count(count1.source, pair.state));
                    value = value.wrapping_sub(apply_source(add.source, pair.state));
                    value ^= apply_source(xor2.source, pair.state)
                        ^ apply_source(xor3.source, pair.state);
                    value = value.rotate_left(apply_count(count2.source, pair.state));
                    value.rotate_left(apply_count(count3.source, pair.state)) == pair.plaintext
                });
                if valid {
                    return Some(format!(
                        "xor({}); sub({}); rol({}); sub({}); xor({}); xor({}); rol({}); rol({})",
                        xor1.name,
                        subtract1.name,
                        count1.name,
                        add.name,
                        xor2.name,
                        xor3.name,
                        count2.name,
                        count3.name,
                    ));
                }
            }
        }
    }
    None
}

fn synthesize_program(pairs: &[Pair]) -> Option<Vec<String>> {
    let compact = &pairs[..pairs.len().min(12)];
    let table = substitute_table();
    let mut inverse_table = [0u8; 256];
    for (index, value) in table.iter().copied().enumerate() {
        inverse_table[value as usize] = index as u8;
    }
    let raw_sources = known_sources(compact, false)
        .into_iter()
        .map(|source| source.source)
        .collect::<Vec<_>>();
    let mut unique_terms = HashMap::<Vec<u8>, Source>::new();
    let mut unique_counts = HashMap::<Vec<u8>, Source>::new();
    for source in raw_sources {
        unique_terms
            .entry(
                compact
                    .iter()
                    .map(|pair| source_value(source, pair.state) as u8)
                    .collect(),
            )
            .or_insert(source);
        unique_counts
            .entry(
                compact
                    .iter()
                    .map(|pair| (source_value(source, pair.state) % 7 + 1) as u8)
                    .collect(),
            )
            .or_insert(source);
    }
    let mut primitives = Vec::new();
    for source in unique_terms.into_values() {
        primitives.push(Primitive::Add(source));
        primitives.push(Primitive::Sub(source));
        primitives.push(Primitive::Xor(source));
    }
    for source in unique_counts.into_values() {
        primitives.push(Primitive::Rol(source));
        primitives.push(Primitive::Ror(source));
    }
    primitives.extend([
        Primitive::Not,
        Primitive::Swap,
        Primitive::Reverse,
        Primitive::Substitute,
        Primitive::InverseSubstitute,
    ]);

    let initial: Vec<u8> = compact.iter().map(|pair| pair.plaintext).collect();
    let target: Vec<u8> = compact.iter().map(|pair| pair.ciphertext).collect();
    let apply_vector = |vector: &[u8], primitive: Primitive| {
        vector
            .iter()
            .zip(compact)
            .map(|(value, pair)| {
                apply_primitive(*value, pair.state, primitive, &table, &inverse_table)
            })
            .collect::<Vec<_>>()
    };

    let append_code = |code: u32, index: usize| {
        let depth = (0..3)
            .find(|offset| (code >> (offset * 8)) & 0xff == 0)
            .unwrap();
        code | (((index + 1) as u32) << (depth * 8))
    };
    let mut forward = HashMap::<Vec<u8>, u32>::new();
    forward.insert(initial.clone(), 0);
    let mut layer = vec![(initial, 0u32)];
    for _ in 0..3 {
        let mut next = Vec::new();
        for (vector, code) in &layer {
            for (index, primitive) in primitives.iter().enumerate() {
                let output = apply_vector(vector, *primitive);
                if forward.contains_key(&output) {
                    continue;
                }
                let output_code = append_code(*code, index);
                forward.insert(output.clone(), output_code);
                next.push((output, output_code));
            }
        }
        layer = next;
    }

    #[allow(clippy::too_many_arguments)]
    fn walk_backward(
        depth: usize,
        vector: Vec<u8>,
        suffix: Vec<Primitive>,
        compact: &[Pair],
        pairs: &[Pair],
        primitives: &[Primitive],
        forward: &HashMap<Vec<u8>, u32>,
        primitive_list: &[Primitive],
        table: &[u8],
        inverse_table: &[u8; 256],
    ) -> Option<Vec<Primitive>> {
        if let Some(code) = forward.get(&vector) {
            let mut program = (0..3)
                .filter_map(|offset| {
                    let value = ((code >> (offset * 8)) & 0xff) as usize;
                    (value != 0).then(|| primitive_list[value - 1])
                })
                .collect::<Vec<_>>();
            program.extend_from_slice(&suffix);
            if apply_program(pairs, &program, table, inverse_table) {
                return Some(program);
            }
        }
        if depth == 0 {
            return None;
        }
        for primitive in primitives {
            let inverse = inverse_primitive(*primitive);
            let previous = vector
                .iter()
                .zip(compact)
                .map(|(value, pair)| {
                    apply_primitive(*value, pair.state, inverse, table, inverse_table)
                })
                .collect::<Vec<_>>();
            let mut previous_suffix = Vec::with_capacity(suffix.len() + 1);
            previous_suffix.push(*primitive);
            previous_suffix.extend_from_slice(&suffix);
            if let Some(program) = walk_backward(
                depth - 1,
                previous,
                previous_suffix,
                compact,
                pairs,
                primitives,
                forward,
                primitive_list,
                table,
                inverse_table,
            ) {
                return Some(program);
            }
        }
        None
    }

    let program = walk_backward(
        3,
        target,
        Vec::new(),
        compact,
        pairs,
        &primitives,
        &forward,
        &primitives,
        &table,
        &inverse_table,
    )?;
    let decrypt = program
        .iter()
        .rev()
        .map(|primitive| primitive_name(inverse_primitive(*primitive)))
        .collect();
    Some(decrypt)
}

fn sources(pairs: &[Pair], counts: bool) -> Vec<NamedSource> {
    let mut candidates: Vec<(String, Source)> = (0u32..=255)
        .map(|value| (format!("mul(0x{value:02X})"), Source::Mul(value)))
        .collect();
    for value in [0x533, 0x1b0829, 0x2751b, 0x0cc6db61, 0xf67761c9] {
        candidates.push((format!("mul(0x{value:08X})"), Source::Mul(value)));
    }
    for value in 1..=8 {
        candidates.push((format!("rol32({value})"), Source::Rol(value)));
        candidates.push((format!("ror32({value})"), Source::Ror(value)));
    }

    let mut unique = HashMap::<Vec<u8>, NamedSource>::new();
    for (name, source) in candidates {
        let vector = pairs
            .iter()
            .map(|pair| {
                let value = source_value(source, pair.state);
                if counts {
                    (value % 7 + 1) as u8
                } else {
                    value as u8
                }
            })
            .collect::<Vec<_>>();
        unique.entry(vector.clone()).or_insert(NamedSource {
            name,
            source,
            vector,
        });
    }
    unique.into_values().collect()
}

fn read_pairs(paths: &[String]) -> Result<Vec<Pair>, Box<dyn std::error::Error>> {
    let mut unique = HashMap::<(u8, u8, u32), Pair>::new();
    for path in paths {
        for line in BufReader::new(File::open(path)?).lines() {
            let sample: Sample = serde_json::from_str(&line?)?;
            if sample.payload_bit_count == 9 {
                let ciphertext = hex::decode(sample.raw_ciphertext_hex)?[0];
                let pair = Pair {
                    ciphertext,
                    plaintext: 0,
                    state: sample.seed,
                };
                unique.insert((ciphertext, 0, sample.seed), pair);
            }
        }
    }
    for pair in [
        Pair {
            ciphertext: 0x04,
            plaintext: 0x00,
            state: 11,
        },
        Pair {
            ciphertext: 0xfa,
            plaintext: 0x40,
            state: 13,
        },
        Pair {
            ciphertext: 0xc7,
            plaintext: 0x68,
            state: 26,
        },
        Pair {
            ciphertext: 0x17,
            plaintext: 0x09,
            state: 284_543_723,
        },
        Pair {
            ciphertext: 0x40,
            plaintext: 0x00,
            state: 1_460_500_957,
        },
    ] {
        unique.insert((pair.ciphertext, pair.plaintext, pair.state), pair);
    }
    let mut pairs: Vec<_> = unique.into_values().collect();
    pairs.sort_by_key(|pair| pair.state);
    Ok(pairs)
}

fn apply_source(source: Source, state: u32) -> u8 {
    source_value(source, state) as u8
}

fn apply_count(source: Source, state: u32) -> u32 {
    source_value(source, state) % 7 + 1
}

fn all_match(pairs: &[Pair], transform: impl Fn(Pair) -> u8) -> bool {
    pairs
        .iter()
        .copied()
        .all(|pair| transform(pair) == pair.plaintext)
}

fn search_12_10(
    training: &[Pair],
    all: &[Pair],
    terms: &[NamedSource],
    counts: &[NamedSource],
) -> Vec<String> {
    let term_lookup: HashMap<Vec<u8>, &NamedSource> = terms
        .iter()
        .map(|term| (term.vector.clone(), term))
        .collect();
    let mut matches = Vec::new();
    for count1 in counts {
        for subtract in terms {
            for count2 in counts {
                let required = training
                    .iter()
                    .enumerate()
                    .map(|(index, pair)| {
                        let mut value = pair
                            .ciphertext
                            .rotate_right(u32::from(count1.vector[index]));
                        value = swap(value).wrapping_sub(subtract.vector[index]);
                        value = value.rotate_right(u32::from(count2.vector[index]));
                        value ^ swap(pair.plaintext)
                    })
                    .collect::<Vec<_>>();
                let Some(xor) = term_lookup.get(&required) else {
                    continue;
                };
                if all_match(all, |pair| {
                    let mut value = pair
                        .ciphertext
                        .rotate_right(apply_count(count1.source, pair.state));
                    value = swap(value).wrapping_sub(apply_source(subtract.source, pair.state));
                    value = value.rotate_right(apply_count(count2.source, pair.state));
                    swap(value ^ apply_source(xor.source, pair.state))
                }) {
                    matches.push(format!(
                        "ror({}); swap; sub({}); ror({}); xor({}); swap",
                        count1.name, subtract.name, count2.name, xor.name
                    ));
                }
            }
        }
    }
    matches
}

fn search_12_11(
    training: &[Pair],
    all: &[Pair],
    terms: &[NamedSource],
    counts: &[NamedSource],
) -> Vec<String> {
    let term_lookup: HashMap<Vec<u8>, &NamedSource> = terms
        .iter()
        .map(|term| (term.vector.clone(), term))
        .collect();
    let mut matches = Vec::new();
    for count in counts {
        for add1 in terms {
            let required = training
                .iter()
                .enumerate()
                .map(|(index, pair)| {
                    let mut value = pair.ciphertext.rotate_right(u32::from(count.vector[index]));
                    value = swap(value).wrapping_add(add1.vector[index]);
                    swap(pair.plaintext).wrapping_sub(value.reverse_bits())
                })
                .collect::<Vec<_>>();
            let Some(add2) = term_lookup.get(&required) else {
                continue;
            };
            if all_match(all, |pair| {
                let mut value = pair
                    .ciphertext
                    .rotate_right(apply_count(count.source, pair.state));
                value = swap(value).wrapping_add(apply_source(add1.source, pair.state));
                value = value
                    .reverse_bits()
                    .wrapping_add(apply_source(add2.source, pair.state));
                swap(value)
            }) {
                matches.push(format!(
                    "ror({}); swap; add({}); reverse; add({}); swap",
                    count.name, add1.name, add2.name
                ));
            }
        }
    }
    matches
}

fn search_13_01(
    training: &[Pair],
    all: &[Pair],
    terms: &[NamedSource],
    counts: &[NamedSource],
) -> Vec<String> {
    let term_lookup: HashMap<Vec<u8>, &NamedSource> = terms
        .iter()
        .map(|term| (term.vector.clone(), term))
        .collect();
    let mut matches = Vec::new();
    for xor in terms {
        for count in counts {
            let required = training
                .iter()
                .enumerate()
                .map(|(index, pair)| {
                    let value = (swap(!pair.ciphertext) ^ xor.vector[index])
                        .rotate_right(u32::from(count.vector[index]));
                    pair.plaintext.wrapping_sub(!value)
                })
                .collect::<Vec<_>>();
            let Some(add) = term_lookup.get(&required) else {
                continue;
            };
            if all_match(all, |pair| {
                let value = (swap(!pair.ciphertext) ^ apply_source(xor.source, pair.state))
                    .rotate_right(apply_count(count.source, pair.state));
                (!value).wrapping_add(apply_source(add.source, pair.state))
            }) {
                matches.push(format!(
                    "not; swap; xor({}); ror({}); not; add({})",
                    xor.name, count.name, add.name
                ));
            }
        }
    }
    matches
}

fn search_13_00(
    training: &[Pair],
    all: &[Pair],
    terms: &[NamedSource],
    counts: &[NamedSource],
) -> Vec<String> {
    let table = substitute_table();
    let mut inverse_table = [0u8; 256];
    for (index, value) in table.iter().copied().enumerate() {
        inverse_table[value as usize] = index as u8;
    }
    let mut forward = HashMap::<Vec<u8>, (usize, usize)>::new();
    for (add1_index, add1) in terms.iter().enumerate() {
        for (add2_index, add2) in terms.iter().enumerate() {
            let vector = training
                .iter()
                .enumerate()
                .map(|(index, pair)| {
                    pair.ciphertext
                        .wrapping_add(add1.vector[index])
                        .reverse_bits()
                        .wrapping_add(add2.vector[index])
                })
                .collect();
            forward.entry(vector).or_insert((add1_index, add2_index));
        }
    }
    let mut matches = Vec::new();
    for count in counts {
        for xor in terms {
            let required = training
                .iter()
                .enumerate()
                .map(|(index, pair)| {
                    let before_rotation =
                        pair.plaintext.rotate_left(u32::from(count.vector[index]));
                    !(inverse_table[before_rotation as usize] ^ xor.vector[index])
                })
                .collect::<Vec<_>>();
            let Some((add1_index, add2_index)) = forward.get(&required).copied() else {
                continue;
            };
            let add1 = &terms[add1_index];
            let add2 = &terms[add2_index];
            if all_match(all, |pair| {
                let mut value = pair
                    .ciphertext
                    .wrapping_add(apply_source(add1.source, pair.state));
                value = value
                    .reverse_bits()
                    .wrapping_add(apply_source(add2.source, pair.state));
                value = !value ^ apply_source(xor.source, pair.state);
                table[value as usize].rotate_right(apply_count(count.source, pair.state))
            }) {
                matches.push(format!(
                    "add({}); reverse; add({}); not-xor({}); substitute; ror({})",
                    add1.name, add2.name, xor.name, count.name
                ));
            }
        }
    }
    matches
}

fn search_13_02(
    training: &[Pair],
    all: &[Pair],
    terms: &[NamedSource],
    counts: &[NamedSource],
) -> Vec<String> {
    let table = substitute_table();
    let mut matches = Vec::new();
    for subtract in terms {
        for count1 in counts {
            for count2 in counts {
                let valid_training = training.iter().enumerate().all(|(index, pair)| {
                    let mut value = table[pair.ciphertext as usize].reverse_bits();
                    value = !value.wrapping_sub(subtract.vector[index]);
                    value = value
                        .reverse_bits()
                        .rotate_left(u32::from(count1.vector[index]));
                    value.rotate_right(u32::from(count2.vector[index])) == pair.plaintext
                });
                if !valid_training {
                    continue;
                }
                if all_match(all, |pair| {
                    let mut value = table[pair.ciphertext as usize].reverse_bits();
                    value = !value.wrapping_sub(apply_source(subtract.source, pair.state));
                    value = value
                        .reverse_bits()
                        .rotate_left(apply_count(count1.source, pair.state));
                    value.rotate_right(apply_count(count2.source, pair.state))
                }) {
                    matches.push(format!(
                        "substitute; reverse; sub({}); not; reverse; rol({}); ror({})",
                        subtract.name, count1.name, count2.name
                    ));
                }
            }
        }
    }
    matches
}

fn search_13_04(
    training: &[Pair],
    all: &[Pair],
    terms: &[NamedSource],
    counts: &[NamedSource],
) -> Vec<String> {
    let compact = &training[..training.len().min(20)];
    let mut forward = HashMap::<Vec<u8>, (usize, usize, usize)>::new();
    for (count_index, count) in counts.iter().enumerate() {
        for (xor_index, xor) in terms.iter().enumerate() {
            for (subtract_index, subtract) in terms.iter().enumerate() {
                let vector = compact
                    .iter()
                    .map(|pair| {
                        let mut value = pair
                            .ciphertext
                            .rotate_right(apply_count(count.source, pair.state));
                        value = (value ^ apply_source(xor.source, pair.state))
                            .wrapping_sub(apply_source(subtract.source, pair.state));
                        swap(value)
                    })
                    .collect();
                forward
                    .entry(vector)
                    .or_insert((count_index, xor_index, subtract_index));
            }
        }
    }
    let mut matches = Vec::new();
    for count2 in counts {
        for add in terms {
            let required = compact
                .iter()
                .map(|pair| {
                    pair.plaintext
                        .rotate_right(apply_count(count2.source, pair.state))
                        .wrapping_sub(apply_source(add.source, pair.state))
                })
                .collect::<Vec<_>>();
            let Some((count1_index, xor_index, subtract_index)) = forward.get(&required).copied()
            else {
                continue;
            };
            let count1 = &counts[count1_index];
            let xor = &terms[xor_index];
            let subtract = &terms[subtract_index];
            if all_match(all, |pair| {
                let mut value = pair
                    .ciphertext
                    .rotate_right(apply_count(count1.source, pair.state));
                value = (value ^ apply_source(xor.source, pair.state))
                    .wrapping_sub(apply_source(subtract.source, pair.state));
                value = swap(value).wrapping_add(apply_source(add.source, pair.state));
                value.rotate_left(apply_count(count2.source, pair.state))
            }) {
                matches.push(format!(
                    "ror({}); xor({}); sub({}); swap; add({}); rol({})",
                    count1.name, xor.name, subtract.name, add.name, count2.name
                ));
            }
        }
    }
    matches
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        return Err("usage: china_byte_vector_search <china13.05.jsonl> [more.jsonl ...]".into());
    }
    let all = read_pairs(&paths)?;
    let training: Vec<_> = all
        .iter()
        .copied()
        .filter(|pair| pair.plaintext == 0 && pair.state < 100_000)
        .collect();
    let terms = sources(&training, false);
    let counts = sources(&training, true);
    let result = serde_json::json!({
        "pair_count": all.len(),
        "training_pair_count": training.len(),
        "term_vector_count": terms.len(),
        "count_vector_count": counts.len(),
        "skeleton_12_10": search_12_10(&training, &all, &terms, &counts),
        "skeleton_12_11": search_12_11(&training, &all, &terms, &counts),
        "skeleton_13_01": search_13_01(&training, &all, &terms, &counts),
        "skeleton_13_00": search_13_00(&training, &all, &terms, &counts),
        "skeleton_13_02": search_13_02(&training, &all, &terms, &counts),
        "skeleton_13_04": search_13_04(&training, &all, &terms, &counts),
        "synthesized_decrypt_program": synthesize_program(&all),
        "skeleton_13_05_combined": search_13_05_combined(&all),
    });
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
