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

#[derive(Debug)]
struct AbilityDraft {
    timestamp_ms: i64,
    ability_name: String,
    position: Option<Vector3>,
    evidence: EvidenceRef,
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
    pub ability_spawns: usize,
    pub spike_plants: usize,
    pub spike_defuses: usize,
    pub spike_explosions: usize,
    pub raw_movement_rows: u64,
    pub semantic_movement_rows: u64,
    pub resolved_area_rows: u64,
    pub unresolved_area_rows: u64,
    pub buy_phase_rounds: usize,
    pub economy_inferred: bool,
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
    map_asset_path: Option<String>,
    pub rounds: Vec<SemanticRound>,
    parser_drafts: Vec<CombatDraft>,
    server_drafts: Vec<ServerDraft>,
    bomb_spawns: Vec<(i64, Vector3)>,
    ability_drafts: Vec<AbilityDraft>,
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

    pub fn set_map(&mut self, map_asset_path: &str) {
        self.map_asset_path = Some(map_asset_path.to_owned());
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
            if let Some(ability_name) = extract_ability_name(path) {
                self.diagnostics.ability_spawns += 1;
                self.ability_drafts.push(AbilityDraft {
                    timestamp_ms: event.timestamp_ms,
                    ability_name,
                    position: json_vector(event.raw.get("location")),
                    evidence: self.evidence(
                        None,
                        event.timestamp_ms,
                        "ability_spawn",
                        "parser_events.ndjson",
                        source_row,
                    ),
                });
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
                self.diagnostics.buy_phase_rounds += 1;
                self.diagnostics.economy_inferred = true;
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
        } else if function == "MulticastNotifyKilledEnemy" {
            let payload = event.raw.get("payload").unwrap_or(&Value::Null);
            let killer = payload.get("KillerCharacter").and_then(Value::as_u64);
            let killed_char = payload.get("KilledCharacter").and_then(Value::as_u64);
            if let (Some(killer_guid), Some(victim_guid)) = (killer, killed_char) {
                self.parser_drafts.push(CombatDraft {
                    timestamp_ms: event.timestamp_ms,
                    kind: "kill".to_owned(),
                    attacker_state: None,
                    attacker_pawn: Some(killer_guid),
                    victim_pawn: Some(victim_guid),
                    damage: None,
                    killed: true,
                    weapon: None,
                    hit_region: None,
                    evidence: self.evidence(
                        None,
                        event.timestamp_ms,
                        "kill",
                        "parser_events.ndjson",
                        source_row,
                    ),
                });
            }
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
        let area = resolve_area(&sample.position, self.map_asset_path.as_deref(), None);
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
        for event in self
            .combat
            .iter_mut()
            .filter(|event| event.kind == "kill" && event.weapon.is_none())
        {
            event.weapon = event.attacker_player_id.as_ref().and_then(|player| {
                shot_weapons
                    .iter()
                    .rev()
                    .find(|(shot_player, time, _)| {
                        shot_player == player
                            && *time <= event.timestamp_ms
                            && event.timestamp_ms - *time <= 2_000
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
                    let (weapon, hit_region) = self
                        .combat
                        .iter()
                        .rev()
                        .find(|e| {
                            e.kind == "damage"
                                && e.timestamp_ms <= draft.timestamp_ms
                                && draft.timestamp_ms - e.timestamp_ms <= 2_000
                                && (e.attacker_player_id.as_deref() == attacker.as_deref()
                                    || e.victim_player_id.as_deref() == victim.as_deref())
                        })
                        .map(|e| (e.weapon.clone(), e.hit_region.clone()))
                        .unwrap_or((None, None));
                    self.combat.push(SemanticCombat {
                        round_no,
                        timestamp_ms: draft.timestamp_ms,
                        kind: "kill".to_owned(),
                        attacker_player_id: attacker,
                        victim_player_id: victim,
                        damage: None,
                        killed: true,
                        weapon,
                        hit_region,
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
        for draft in std::mem::take(&mut self.ability_drafts) {
            let round_no = self.round_for_time(draft.timestamp_ms);
            let mut evidence = draft.evidence;
            evidence.round_no = round_no;
            let player_id = draft
                .position
                .as_ref()
                .and_then(|pos| self.find_nearest_player(pos, draft.timestamp_ms));
            let area = draft
                .position
                .as_ref()
                .and_then(|pos| resolve_area(pos, self.map_asset_path.as_deref(), None));
            self.abilities.push(SemanticAbility {
                round_no,
                timestamp_ms: draft.timestamp_ms,
                player_id,
                ability_name: draft.ability_name,
                area,
                evidence: vec![evidence],
            });
            self.diagnostics.abilities += 1;
        }
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
            "abilities": {
                "count": self.diagnostics.abilities,
                "ability_spawns": self.diagnostics.ability_spawns,
            },
            "spike": {
                "plants": self.diagnostics.spike_plants,
                "defuses": self.diagnostics.spike_defuses,
                "explosions": self.diagnostics.spike_explosions
            },
            "economy": {
                "buy_phase_rounds": self.diagnostics.buy_phase_rounds,
                "inferred": self.diagnostics.economy_inferred,
                "credits_available": false,
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

    fn find_nearest_player(&self, pos: &Vector3, timestamp_ms: i64) -> Option<String> {
        self.tracks
            .iter()
            .filter_map(|(player_id, track)| {
                let point = track
                    .iter()
                    .rev()
                    .find(|p| p.timestamp_ms <= timestamp_ms + 1_000)?;
                let dx = point.position.x - pos.x;
                let dy = point.position.y - pos.y;
                let dz = point.position.z - pos.z;
                let dist = (dx * dx + dy * dy + dz * dz).sqrt();
                if dist < 3_000.0 {
                    Some((player_id.clone(), dist))
                } else {
                    None
                }
            })
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(id, _)| id)
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

/// Extract a human-readable ability name from a replication class path.
/// Examples: "/Game/Characters/Hunter/Q/Ability_Hunter_Q_SonarPing" -> "Sova Q (SonarPing)"
///           "/Game/Characters/Smonk/NewSmoke/GameObject_Smonk_NewSmoke" -> "Clove E (NewSmoke)"
fn extract_ability_name(path: &str) -> Option<String> {
    if !path.contains("Ability_") && !path.contains("GameObject_") && !path.contains("Projectile_") {
        return None;
    }
    if path.contains("Melee_Base") || path.contains("EquippablePickup") {
        return None;
    }
    let agent = path
        .rsplit('/')
        .nth(1)
        .and_then(|segment| {
            let codename = segment.strip_suffix("_PC").unwrap_or(segment);
            let display = match codename {
                "Hunter" => "Sova",
                "Clay" => "Raze",
                "Sprinter" => "Neon",
                "Vampire" => "Reyna",
                "Sarge" => "Brimstone",
                "Smonk" => "Clove",
                "Wushu" => "Jett",
                "Pine" => "Vyse",
                "Deadeye" => "Chamber",
                "AggroBot" => "Gekko",
                _ => codename,
            };
            (!display.is_empty() && display != "Characters").then(|| display.to_owned())
        });
    let ability_slot = if path.contains("/Q/") {
        Some("Q")
    } else if path.contains("/E/") {
        Some("E")
    } else if path.contains("/4/") {
        Some("C")
    } else if path.contains("/X/") {
        Some("X")
    } else {
        None
    };
    let effect_name = path
        .rsplit('/')
        .next()
        .and_then(|name| {
            let stripped = name
                .strip_prefix("Ability_")
                .or_else(|| name.strip_prefix("GameObject_"))
                .or_else(|| name.strip_prefix("Projectile_"))
                .unwrap_or(name);
            let cleaned = stripped
                .trim_end_matches("_C")
                .trim_end_matches("_Production")
                .trim_end_matches("_ProductionNEW");
            (!cleaned.is_empty()).then(|| cleaned.to_owned())
        });
    match (agent, ability_slot, effect_name) {
        (Some(a), Some(slot), Some(effect)) => Some(format!("{a} {slot} ({effect})")),
        (Some(a), None, Some(effect)) => Some(format!("{a} ({effect})")),
        (None, Some(slot), Some(effect)) => Some(format!("{slot} ({effect})")),
        (_, _, Some(effect)) => Some(effect),
        _ => None,
    }
}

/// Resolve area from world coordinates using the map registry.
/// Falls back to hardcoded Split zones if no map data is loaded.
pub(super) fn resolve_area(position: &Vector3, map_asset_path: Option<&str>, registry: Option<&valcoach_maps::MapRegistry>) -> Option<String> {
    if let (Some(registry), Some(map_path)) = (registry, map_asset_path)
        && let Some(area) = registry.resolve_area(map_path, position)
    {
        return Some(area);
    }
    split_area(position).map(str::to_owned)
}

/// Legacy deterministic first-pass calibration for Split/Bonsai world coordinates.
/// Used as fallback when no Valorant-API map data is available.
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
