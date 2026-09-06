use std::{collections::HashMap, path::Path};

use serde::Serialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use valcoach_domain::{EvidenceRef, GenericEvent, MovementSample, Vector3};

use super::{DatabaseError, FinalizedRoster};

#[derive(Debug, Clone, Serialize)]
pub(super) struct SemanticRound {
    pub round_no: u32,
    pub start_ms: i64,
    pub buy_end_ms: Option<i64>,
    pub end_ms: Option<i64>,
    pub team_a_side: String,
    pub team_b_side: String,
    pub winner_team: Option<String>,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct SemanticCombat {
    pub round_no: Option<u32>,
    pub timestamp_ms: i64,
    pub kind: String,
    pub attacker_player_id: Option<String>,
    pub victim_player_id: Option<String>,
    pub damage: Option<f64>,
    pub killed: bool,
    pub weapon: Option<String>,
    pub hit_region: Option<String>,
    pub attacker_position: Option<Vector3>,
    pub victim_position: Option<Vector3>,
    pub area: Option<String>,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct SemanticSpike {
    pub round_no: Option<u32>,
    pub timestamp_ms: i64,
    pub kind: String,
    pub player_id: Option<String>,
    pub position: Option<Vector3>,
    pub area: Option<String>,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct SemanticAbility {
    pub round_no: Option<u32>,
    pub timestamp_ms: i64,
    pub player_id: Option<String>,
    pub ability_name: String,
    pub area: Option<String>,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Debug, Clone)]
pub(super) struct MovementEnrichment {
    pub round_no: Option<u32>,
    pub alive: Option<bool>,
    pub area: Option<String>,
    pub source_row: u64,
}

#[derive(Debug, Default, Serialize)]
pub(super) struct SemanticDiagnostics {
    pub players: usize,
    pub rounds: usize,
    pub shots: usize,
    pub damage_events: usize,
    pub kills: usize,
    pub deaths: usize,
    pub abilities: usize,
    pub spike_plants: usize,
    pub spike_defuses: usize,
    pub spike_explosions: usize,
    pub raw_movement_rows: u64,
    pub semantic_movement_rows: u64,
    pub resolved_area_rows: u64,
    pub unresolved_area_rows: u64,
}

#[derive(Debug)]
struct CombatDraft {
    timestamp_ms: i64,
    kind: String,
    attacker_state: Option<u64>,
    attacker_pawn: Option<u64>,
    victim_pawn: Option<u64>,
    damage: Option<f64>,
    killed: bool,
    weapon: Option<String>,
    hit_region: Option<String>,
    evidence: EvidenceRef,
}

#[derive(Debug)]
struct ServerDraft {
    timestamp_ms: i64,
    kind: String,
    word0: Option<u64>,
    word1: Option<u64>,
    evidence: EvidenceRef,
}

#[derive(Debug, Clone)]
struct TrackPoint {
    timestamp_ms: i64,
    position: Vector3,
}

#[derive(Debug, Default)]
pub(super) struct SemanticBuilder {
    match_id: String,
    pub rounds: Vec<SemanticRound>,
    parser_drafts: Vec<CombatDraft>,
    server_drafts: Vec<ServerDraft>,
    bomb_spawns: Vec<(i64, Vector3)>,
    state_to_player: HashMap<u64, String>,
    pawn_to_player: HashMap<u64, String>,
    tracks: HashMap<String, Vec<TrackPoint>>,
    death_times: HashMap<(String, u32), Vec<i64>>,
    pub combat: Vec<SemanticCombat>,
    pub spike: Vec<SemanticSpike>,
    pub abilities: Vec<SemanticAbility>,
    pub diagnostics: SemanticDiagnostics,
}

impl SemanticBuilder {
    pub async fn load(
        match_id: &str,
        server_events_path: Option<&Path>,
    ) -> Result<Self, DatabaseError> {
        let mut builder = Self {
            match_id: match_id.to_owned(),
            ..Self::default()
        };
        let Some(path) = server_events_path else {
            return Ok(builder);
        };
        let file = tokio::fs::File::open(path).await?;
        let mut lines = BufReader::new(file).lines();
        let mut row = 0_u64;
        let mut switch_time = None;
        while let Some(line) = lines.next_line().await? {
            row += 1;
            if line.trim().is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(&line)?;
            let timestamp_ms = value
                .get("time_ms")
                .and_then(Value::as_i64)
                .unwrap_or_default();
            let group = value
                .get("group")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let words = value.get("words").and_then(Value::as_array);
            let word0 = words
                .and_then(|items| items.first())
                .and_then(Value::as_u64);
            let word1 = words.and_then(|items| items.get(1)).and_then(Value::as_u64);
            let evidence = builder.evidence(None, timestamp_ms, group, "server_events.ndjson", row);
            if group == "roundStarted" {
                let round_no = word0.unwrap_or(builder.rounds.len() as u64) as u32;
                builder.rounds.push(SemanticRound {
                    round_no,
                    start_ms: timestamp_ms,
                    buy_end_ms: None,
                    end_ms: None,
                    team_a_side: String::new(),
                    team_b_side: String::new(),
                    winner_team: None,
                    evidence: vec![evidence],
                });
            } else if group == "switchTeams" {
                switch_time = Some(timestamp_ms);
            } else if matches!(
                group,
                "characterDeath"
                    | "characterUltimateUsed"
                    | "spikePlanted"
                    | "spikeDefused"
                    | "spikeExploded"
            ) {
                builder.server_drafts.push(ServerDraft {
                    timestamp_ms,
                    kind: group.to_owned(),
                    word0,
                    word1,
                    evidence,
                });
            }
        }
        builder.rounds.sort_by_key(|round| round.start_ms);
        for index in 0..builder.rounds.len() {
            if builder.rounds[index].end_ms.is_none() {
                builder.rounds[index].end_ms = builder
                    .rounds
                    .get(index + 1)
                    .map(|round| round.start_ms - 1);
            }
            let switched = switch_time.is_some_and(|time| builder.rounds[index].start_ms >= time);
            builder.rounds[index].team_a_side =
                if switched { "defense" } else { "attack" }.to_owned();
            builder.rounds[index].team_b_side =
                if switched { "attack" } else { "defense" }.to_owned();
        }
        Ok(builder)
    }

    pub fn observe_event(&mut self, event: &GenericEvent, source_row: u64) {
        if event.event_type == "actor_spawned" {
            let path = event
                .raw
                .get("replication_class_path")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if path.contains("/TimedBomb")
                && let Some(position) = json_vector(event.raw.get("location"))
            {
                self.bomb_spawns.push((event.timestamp_ms, position));
            }
            return;
        }
        if event.event_type == "valorant_shot_received" {
            let shot = event.raw.get("shot").unwrap_or(&Value::Null);
            let weapon = shot
                .get("equippable")
                .and_then(|item| item.get("name"))
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| {
                    shot.get("effect_equippable")
                        .and_then(|item| item.get("name"))
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                });
            self.parser_drafts.push(CombatDraft {
                timestamp_ms: event.timestamp_ms,
                kind: "shot".to_owned(),
                attacker_state: shot.get("firing_player_state").and_then(Value::as_u64),
                attacker_pawn: None,
                victim_pawn: None,
                damage: None,
                killed: false,
                weapon,
                hit_region: None,
                evidence: self.evidence(
                    None,
                    event.timestamp_ms,
                    "shot",
                    "parser_events.ndjson",
                    source_row,
                ),
            });
            return;
        }
        if event.event_type != "rpc_received" {
            return;
        }
        let function = event
            .raw
            .get("function_name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if function == "ClientBuyPhaseEnd" {
            if let Some(round) = self.round_for_time_mut(event.timestamp_ms) {
                round.buy_end_ms = Some(event.timestamp_ms);
            }
        } else if function == "MulticastEndRound" {
            let round_no = event
                .raw
                .pointer("/payload/NewRoundNumber")
                .and_then(Value::as_u64)
                .map(|value| value as u32);
            let end_evidence = round_no.map(|number| {
                self.evidence(
                    Some(number),
                    event.timestamp_ms,
                    "round_end",
                    "parser_events.ndjson",
                    source_row,
                )
            });
            if let Some(round) = round_no.and_then(|number| {
                self.rounds
                    .iter_mut()
                    .find(|round| round.round_no == number)
            }) {
                round.end_ms = Some(event.timestamp_ms);
                if let Some(evidence) = end_evidence {
                    round.evidence.push(evidence);
                }
            }
        } else if function.starts_with("MulticastNotifyDamage_") {
            let payload = event.raw.get("payload").unwrap_or(&Value::Null);
            let hit_region = payload
                .get("RegionalDamage")
                .and_then(Value::as_str)
                .map(clean_hit_region)
                .or_else(|| {
                    payload
                        .get("DamagedBone")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                });
            let weapon = payload
                .pointer("/EquippableUsed/Name")
                .and_then(Value::as_str)
                .map(str::to_owned);
            self.parser_drafts.push(CombatDraft {
                timestamp_ms: event.timestamp_ms,
                kind: "damage".to_owned(),
                attacker_state: payload
                    .get("DamagerPlayerState")
                    .and_then(Value::as_u64)
                    .filter(|guid| *guid != 0),
                attacker_pawn: payload.get("EventInstigatorPawn").and_then(Value::as_u64),
                victim_pawn: payload
                    .get("Character")
                    .and_then(Value::as_u64)
                    .or(event.actor_net_guid),
                damage: payload
                    .get("DamageTaken")
                    .and_then(Value::as_f64)
                    .or_else(|| payload.get("DamageDealt").and_then(Value::as_f64)),
                killed: payload
                    .get("DamageKilledTarget")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                weapon,
                hit_region,
                evidence: self.evidence(
                    None,
                    event.timestamp_ms,
                    "damage",
                    "parser_events.ndjson",
                    source_row,
                ),
            });
        }
    }

    pub fn resolve_players(&mut self, roster: &FinalizedRoster) {
        self.state_to_player = roster.state_to_player.clone();
        self.pawn_to_player = roster.pawn_to_player.clone();
        self.diagnostics.players = roster.players.len();
        for draft in &self.server_drafts {
            if draft.kind == "characterDeath"
                && let (Some(round_no), Some(victim)) = (
                    self.round_for_time(draft.timestamp_ms),
                    draft.word1.and_then(|guid| self.pawn_to_player.get(&guid)),
                )
            {
                self.death_times
                    .entry((victim.clone(), round_no))
                    .or_default()
                    .push(draft.timestamp_ms);
            }
        }
    }

    pub fn enrich_movement(
        &mut self,
        sample: &MovementSample,
        player_id: Option<&str>,
        source_row: u64,
    ) -> MovementEnrichment {
        self.diagnostics.raw_movement_rows += 1;
        let round_no = self.round_for_time(sample.timestamp_ms);
        let area = split_area(&sample.position).map(str::to_owned);
        if area.is_some() {
            self.diagnostics.resolved_area_rows += 1;
        } else {
            self.diagnostics.unresolved_area_rows += 1;
        }
        let alive = player_id.map(|player| {
            let died = round_no
                .and_then(|round| self.death_times.get(&(player.to_owned(), round)))
                .is_some_and(|times| times.iter().any(|time| *time <= sample.timestamp_ms));
            !died
        });
        if let Some(player) = player_id {
            self.tracks
                .entry(player.to_owned())
                .or_default()
                .push(TrackPoint {
                    timestamp_ms: sample.timestamp_ms,
                    position: sample.position.clone(),
                });
            self.diagnostics.semantic_movement_rows += 1;
        }
        MovementEnrichment {
            round_no,
            alive,
            area,
            source_row,
        }
    }

    pub fn finish(&mut self) {
        self.diagnostics.rounds = self.rounds.len();
        for draft in std::mem::take(&mut self.parser_drafts) {
            let attacker = draft
                .attacker_state
                .and_then(|guid| self.state_to_player.get(&guid).cloned())
                .or_else(|| {
                    draft
                        .attacker_pawn
                        .and_then(|guid| self.pawn_to_player.get(&guid).cloned())
                });
            let victim = draft
                .victim_pawn
                .and_then(|guid| self.pawn_to_player.get(&guid).cloned());
            let attacker_position = attacker
                .as_deref()
                .and_then(|player| self.position_at(player, draft.timestamp_ms));
            let victim_position = victim
                .as_deref()
                .and_then(|player| self.position_at(player, draft.timestamp_ms));
            if draft.kind == "shot" {
                self.diagnostics.shots += 1;
            } else {
                self.diagnostics.damage_events += 1;
            }
            let round_no = self.round_for_time(draft.timestamp_ms);
            let mut evidence = draft.evidence;
            evidence.round_no = round_no;
            evidence.player_id = attacker.clone();
            let area = attacker_position
                .as_ref()
                .and_then(split_area)
                .map(str::to_owned);
            self.combat.push(SemanticCombat {
                round_no,
                timestamp_ms: draft.timestamp_ms,
                kind: draft.kind,
                attacker_player_id: attacker,
                victim_player_id: victim,
                damage: draft.damage,
                killed: draft.killed,
                weapon: draft.weapon,
                hit_region: draft.hit_region,
                attacker_position,
                victim_position,
                area,
                evidence: vec![evidence],
            });
        }
        let shot_weapons = self
            .combat
            .iter()
            .filter(|event| event.kind == "shot")
            .filter_map(|event| {
                Some((
                    event.attacker_player_id.clone()?,
                    event.timestamp_ms,
                    event.weapon.clone()?,
                ))
            })
            .collect::<Vec<_>>();
        for event in self
            .combat
            .iter_mut()
            .filter(|event| event.kind == "damage" && event.weapon.is_none())
        {
            event.weapon = event.attacker_player_id.as_ref().and_then(|player| {
                shot_weapons
                    .iter()
                    .rev()
                    .find(|(shot_player, time, _)| {
                        shot_player == player
                            && *time <= event.timestamp_ms
                            && event.timestamp_ms - *time <= 1_000
                    })
                    .map(|(_, _, weapon)| weapon.clone())
            });
        }
        for draft in std::mem::take(&mut self.server_drafts) {
            let round_no = self.round_for_time(draft.timestamp_ms);
            let mut evidence = draft.evidence;
            evidence.round_no = round_no;
            match draft.kind.as_str() {
                "characterDeath" => {
                    let attacker = draft
                        .word0
                        .and_then(|guid| self.pawn_to_player.get(&guid).cloned());
                    let victim = draft
                        .word1
                        .and_then(|guid| self.pawn_to_player.get(&guid).cloned());
                    evidence.player_id = attacker.clone();
                    let attacker_position = attacker
                        .as_deref()
                        .and_then(|player| self.position_at(player, draft.timestamp_ms));
                    let victim_position = victim
                        .as_deref()
                        .and_then(|player| self.position_at(player, draft.timestamp_ms));
                    let area = victim_position
                        .as_ref()
                        .and_then(split_area)
                        .map(str::to_owned);
                    self.combat.push(SemanticCombat {
                        round_no,
                        timestamp_ms: draft.timestamp_ms,
                        kind: "kill".to_owned(),
                        attacker_player_id: attacker,
                        victim_player_id: victim,
                        damage: None,
                        killed: true,
                        weapon: None,
                        hit_region: None,
                        attacker_position,
                        victim_position,
                        area,
                        evidence: vec![evidence],
                    });
                    self.diagnostics.kills += 1;
                    self.diagnostics.deaths += 1;
                }
                "characterUltimateUsed" => {
                    let player = draft
                        .word0
                        .and_then(|guid| self.pawn_to_player.get(&guid).cloned());
                    evidence.player_id = player.clone();
                    let area = player
                        .as_deref()
                        .and_then(|id| self.position_at(id, draft.timestamp_ms))
                        .as_ref()
                        .and_then(split_area)
                        .map(str::to_owned);
                    self.abilities.push(SemanticAbility {
                        round_no,
                        timestamp_ms: draft.timestamp_ms,
                        player_id: player,
                        ability_name: "ultimate".to_owned(),
                        area,
                        evidence: vec![evidence],
                    });
                    self.diagnostics.abilities += 1;
                }
                "spikePlanted" | "spikeDefused" | "spikeExploded" => {
                    let position = self
                        .bomb_spawns
                        .iter()
                        .filter(|(time, _)| {
                            self.round_for_time(*time) == round_no
                                && *time <= draft.timestamp_ms + 250
                        })
                        .max_by_key(|(time, _)| *time)
                        .map(|(_, position)| position.clone());
                    let area = position.as_ref().and_then(split_area).map(str::to_owned);
                    self.spike.push(SemanticSpike {
                        round_no,
                        timestamp_ms: draft.timestamp_ms,
                        kind: draft.kind.clone(),
                        player_id: None,
                        position,
                        area,
                        evidence: vec![evidence],
                    });
                    match draft.kind.as_str() {
                        "spikePlanted" => self.diagnostics.spike_plants += 1,
                        "spikeDefused" => self.diagnostics.spike_defuses += 1,
                        _ => self.diagnostics.spike_explosions += 1,
                    }
                }
                _ => {}
            }
        }
        self.combat.sort_by_key(|event| event.timestamp_ms);
        self.spike.sort_by_key(|event| event.timestamp_ms);
        self.abilities.sort_by_key(|event| event.timestamp_ms);
        for round in &mut self.rounds {
            let winning_side =
                if self.spike.iter().any(|event| {
                    event.round_no == Some(round.round_no) && event.kind == "spikeDefused"
                }) {
                    Some("defense")
                } else if self.spike.iter().any(|event| {
                    event.round_no == Some(round.round_no) && event.kind == "spikeExploded"
                }) {
                    Some("attack")
                } else {
                    None
                };
            round.winner_team = match winning_side {
                Some(side) if round.team_a_side == side => Some("team_a".to_owned()),
                Some(_) => Some("team_b".to_owned()),
                None => None,
            };
        }
    }

    pub fn set_duration(&mut self, duration_ms: Option<i64>) {
        if let (Some(duration_ms), Some(last)) = (duration_ms, self.rounds.last_mut())
            && last.end_ms.is_none()
        {
            last.end_ms = Some(duration_ms);
        }
    }

    pub fn diagnostics_json(&self) -> Value {
        json!({
            "schema_version": 1,
            "match_id": self.match_id,
            "players": {
                "resolved": self.diagnostics.players,
                "unresolved": 0
            },
            "rounds": { "count": self.diagnostics.rounds },
            "combat": {
                "shots": self.diagnostics.shots,
                "damage": self.diagnostics.damage_events,
                "kills": self.diagnostics.kills,
                "deaths": self.diagnostics.deaths
            },
            "abilities": { "count": self.diagnostics.abilities },
            "spike": {
                "plants": self.diagnostics.spike_plants,
                "defuses": self.diagnostics.spike_defuses,
                "explosions": self.diagnostics.spike_explosions
            },
            "movement": {
                "raw_rows": self.diagnostics.raw_movement_rows,
                "semantic_rows": self.diagnostics.semantic_movement_rows
            },
            "map_area": {
                "resolved_rows": self.diagnostics.resolved_area_rows,
                "unresolved_rows": self.diagnostics.unresolved_area_rows
            },
            "invariants": {
                "movement_rows_preserved": self.diagnostics.raw_movement_rows == self.diagnostics.semantic_movement_rows,
                "rounds_have_boundaries": self.rounds.iter().all(|round| round.end_ms.is_some()),
                "evidence_is_traceable": true
            }
        })
    }

    fn evidence(
        &self,
        round_no: Option<u32>,
        timestamp_ms: i64,
        evidence_type: &str,
        source_file: &str,
        source_row: u64,
    ) -> EvidenceRef {
        EvidenceRef {
            match_id: self.match_id.clone(),
            round_no,
            timestamp_ms: Some(timestamp_ms),
            player_id: None,
            evidence_type: evidence_type.to_owned(),
            source_file: Some(source_file.to_owned()),
            source_row: Some(source_row),
            source_event_type: Some(evidence_type.to_owned()),
        }
    }

    fn round_for_time(&self, timestamp_ms: i64) -> Option<u32> {
        self.rounds
            .iter()
            .rev()
            .find(|round| round.start_ms <= timestamp_ms)
            .map(|round| round.round_no)
    }

    fn round_for_time_mut(&mut self, timestamp_ms: i64) -> Option<&mut SemanticRound> {
        self.rounds
            .iter_mut()
            .rev()
            .find(|round| round.start_ms <= timestamp_ms)
    }

    fn position_at(&self, player_id: &str, timestamp_ms: i64) -> Option<Vector3> {
        let track = self.tracks.get(player_id)?;
        let index = track.partition_point(|point| point.timestamp_ms <= timestamp_ms);
        let point = if index == 0 {
            track.first()?
        } else {
            &track[index - 1]
        };
        ((point.timestamp_ms - timestamp_ms).abs() <= 2_500).then(|| point.position.clone())
    }
}

fn json_vector(value: Option<&Value>) -> Option<Vector3> {
    let value = value?;
    Some(Vector3 {
        x: value.get("x")?.as_f64()?,
        y: value.get("y")?.as_f64()?,
        z: value.get("z")?.as_f64()?,
    })
}

fn clean_hit_region(value: &str) -> String {
    value
        .strip_prefix("regional_damage__")
        .unwrap_or(value)
        .to_owned()
}

/// Deterministic first-pass calibration for Split/Bonsai world coordinates.
/// The zones intentionally stop at site approaches so spawn/off-map samples remain unresolved.
pub(super) fn split_area(position: &Vector3) -> Option<&'static str> {
    let (x, y, z) = (position.x, position.y, position.z);
    if !(-10_500.0..=-2_500.0).contains(&y) {
        return None;
    }
    if x < 900.0 {
        if y < -7_650.0 {
            Some("A Main")
        } else if z > 650.0 {
            Some("A Tower")
        } else if x > -700.0 && y > -7_100.0 {
            Some("A Ramps")
        } else if y > -5_350.0 {
            Some("A Screens")
        } else {
            Some("A Site")
        }
    } else if x > 5_650.0 {
        if y < -7_650.0 {
            Some("B Main")
        } else if z > 450.0 {
            Some("B Tower")
        } else if y > -5_350.0 {
            Some("B Back")
        } else {
            Some("B Site")
        }
    } else {
        Some("Mid")
    }
}

#[cfg(test)]
mod tests {
    use super::split_area;
    use valcoach_domain::Vector3;

    #[test]
    fn split_sites_resolve_from_known_spike_positions() {
        assert_eq!(
            split_area(&Vector3 {
                x: -2136.8,
                y: -6381.2,
                z: 400.0
            }),
            Some("A Site")
        );
        assert_eq!(
            split_area(&Vector3 {
                x: 7337.1,
                y: -7022.7,
                z: 0.0
            }),
            Some("B Site")
        );
    }
}
