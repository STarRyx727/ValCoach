CREATE TABLE compact_replays (
    match_id TEXT PRIMARY KEY NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    replay_sha256 TEXT,
    semantic_version TEXT NOT NULL DEFAULT 'v1',
    compact_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_compact_replays_match ON compact_replays(match_id);
