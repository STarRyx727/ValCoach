ALTER TABLE players ADD COLUMN player_state_net_guid TEXT;
ALTER TABLE players ADD COLUMN character_net_guids_json TEXT NOT NULL DEFAULT '[]';

ALTER TABLE rounds ADD COLUMN start_ms INTEGER;
ALTER TABLE rounds ADD COLUMN buy_end_ms INTEGER;
ALTER TABLE rounds ADD COLUMN end_ms INTEGER;
ALTER TABLE rounds ADD COLUMN team_a_side TEXT;
ALTER TABLE rounds ADD COLUMN team_b_side TEXT;
ALTER TABLE rounds ADD COLUMN winner_team TEXT;
ALTER TABLE rounds ADD COLUMN evidence_json TEXT NOT NULL DEFAULT '[]';

ALTER TABLE movement_samples ADD COLUMN round_no INTEGER;
ALTER TABLE movement_samples ADD COLUMN yaw REAL;
ALTER TABLE movement_samples ADD COLUMN pitch REAL;
ALTER TABLE movement_samples ADD COLUMN alive INTEGER;
ALTER TABLE movement_samples ADD COLUMN area TEXT;
ALTER TABLE movement_samples ADD COLUMN source_file TEXT;
ALTER TABLE movement_samples ADD COLUMN source_row INTEGER;

CREATE TABLE combat_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    match_id TEXT NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    round_no INTEGER,
    timestamp_ms INTEGER NOT NULL,
    kind TEXT NOT NULL,
    attacker_player_id TEXT,
    victim_player_id TEXT,
    damage REAL,
    killed INTEGER NOT NULL DEFAULT 0,
    weapon TEXT,
    hit_region TEXT,
    attacker_x REAL,
    attacker_y REAL,
    attacker_z REAL,
    victim_x REAL,
    victim_y REAL,
    victim_z REAL,
    area TEXT,
    evidence_json TEXT NOT NULL
);

CREATE TABLE spike_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    match_id TEXT NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    round_no INTEGER,
    timestamp_ms INTEGER NOT NULL,
    kind TEXT NOT NULL,
    player_id TEXT,
    x REAL,
    y REAL,
    z REAL,
    area TEXT,
    evidence_json TEXT NOT NULL
);

ALTER TABLE ability_events ADD COLUMN round_no INTEGER;
ALTER TABLE ability_events ADD COLUMN ability_name TEXT;
ALTER TABLE ability_events ADD COLUMN area TEXT;
ALTER TABLE ability_events ADD COLUMN evidence_json TEXT NOT NULL DEFAULT '[]';

CREATE TABLE semantic_diagnostics (
    match_id TEXT PRIMARY KEY NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    value_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_rounds_match_round ON rounds(match_id, round_no);
CREATE INDEX idx_movement_semantic_lookup
    ON movement_samples(match_id, player_id, round_no, area, timestamp_ms);
CREATE INDEX idx_movement_match_time
    ON movement_samples(match_id, timestamp_ms);
CREATE INDEX idx_combat_match_round_player
    ON combat_events(match_id, round_no, attacker_player_id, victim_player_id, timestamp_ms);
CREATE INDEX idx_spike_match_round ON spike_events(match_id, round_no, timestamp_ms);
CREATE INDEX idx_ability_match_round_player
    ON ability_events(match_id, round_no, player_id, timestamp_ms);
