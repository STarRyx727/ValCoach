CREATE TABLE player_issues (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    issue_key TEXT NOT NULL,
    category TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT,
    map_name TEXT,
    side TEXT,
    area TEXT,
    severity REAL NOT NULL DEFAULT 0.5,
    confidence REAL NOT NULL DEFAULT 0.5,
    status TEXT NOT NULL DEFAULT 'active',
    occurrences INTEGER NOT NULL DEFAULT 0,
    last_match_id TEXT,
    last_round_no INTEGER,
    last_timestamp_ms INTEGER,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(user_id, issue_key)
);

CREATE INDEX idx_player_issues_user ON player_issues(user_id, status);

CREATE TABLE issue_occurrences (
    id TEXT PRIMARY KEY NOT NULL,
    issue_id TEXT NOT NULL REFERENCES player_issues(id) ON DELETE CASCADE,
    match_id TEXT NOT NULL,
    round_no INTEGER,
    timestamp_ms INTEGER,
    severity REAL,
    evidence_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_issue_occurrences_issue ON issue_occurrences(issue_id);
CREATE INDEX idx_issue_occurrences_match ON issue_occurrences(match_id);
