CREATE TABLE users (
    id TEXT PRIMARY KEY NOT NULL,
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE sessions (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at TEXT NOT NULL
);

CREATE TABLE valorant_accounts (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    region TEXT NOT NULL,
    subject_id TEXT,
    display_name TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE matches (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    parser_source TEXT NOT NULL,
    metadata_json TEXT NOT NULL,
    capabilities_json TEXT NOT NULL,
    summary_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE players (
    id TEXT PRIMARY KEY NOT NULL,
    match_id TEXT NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    stable_player_id TEXT,
    display_name TEXT
);

CREATE TABLE rounds (
    id TEXT PRIMARY KEY NOT NULL,
    match_id TEXT NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    round_no INTEGER NOT NULL
);

CREATE TABLE movement_samples (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    match_id TEXT NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    player_id TEXT,
    timestamp_ms INTEGER NOT NULL,
    x REAL NOT NULL,
    y REAL NOT NULL,
    z REAL NOT NULL,
    velocity_x REAL,
    velocity_y REAL,
    velocity_z REAL
);

CREATE TABLE events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    match_id TEXT NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    timestamp_ms INTEGER NOT NULL,
    event_type TEXT NOT NULL,
    actor_net_guid TEXT,
    payload_json TEXT NOT NULL
);

CREATE TABLE shots (
    id TEXT PRIMARY KEY NOT NULL,
    match_id TEXT NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    player_id TEXT,
    timestamp_ms INTEGER NOT NULL,
    payload_json TEXT NOT NULL
);

CREATE TABLE duels (
    id TEXT PRIMARY KEY NOT NULL,
    match_id TEXT NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    player_id TEXT,
    timestamp_ms INTEGER,
    payload_json TEXT NOT NULL
);

CREATE TABLE ability_events (
    id TEXT PRIMARY KEY NOT NULL,
    match_id TEXT NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    player_id TEXT,
    timestamp_ms INTEGER,
    payload_json TEXT NOT NULL
);

CREATE TABLE round_metrics (
    id TEXT PRIMARY KEY NOT NULL,
    match_id TEXT NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    round_id TEXT REFERENCES rounds(id) ON DELETE CASCADE,
    metric_name TEXT NOT NULL,
    value_json TEXT NOT NULL
);

CREATE TABLE match_metrics (
    id TEXT PRIMARY KEY NOT NULL,
    match_id TEXT NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    metric_name TEXT NOT NULL,
    value_json TEXT NOT NULL
);

CREATE TABLE player_profile_snapshots (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    scope TEXT NOT NULL,
    profile_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE parse_jobs (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    status TEXT NOT NULL,
    source_name TEXT NOT NULL,
    error_message TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE conversations (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE messages (
    id TEXT PRIMARY KEY NOT NULL,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE tool_calls (
    id TEXT PRIMARY KEY NOT NULL,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    tool_name TEXT NOT NULL,
    arguments_json TEXT NOT NULL,
    result_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE llm_usage (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    conversation_id TEXT REFERENCES conversations(id) ON DELETE SET NULL,
    model TEXT NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    estimated_cost_micros INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_matches_user_id ON matches(user_id);
CREATE INDEX idx_players_match_id ON players(match_id);
CREATE INDEX idx_rounds_match_id ON rounds(match_id);
CREATE INDEX idx_movement_samples_match_player_time ON movement_samples(match_id, player_id, timestamp_ms);
CREATE INDEX idx_events_match_time ON events(match_id, timestamp_ms);
CREATE INDEX idx_shots_match_player_time ON shots(match_id, player_id, timestamp_ms);
