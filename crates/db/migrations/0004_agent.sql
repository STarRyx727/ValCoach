ALTER TABLE conversations ADD COLUMN match_id TEXT REFERENCES matches(id) ON DELETE CASCADE;
ALTER TABLE conversations ADD COLUMN provider TEXT;
ALTER TABLE conversations ADD COLUMN model TEXT;

ALTER TABLE messages ADD COLUMN evidence_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE messages ADD COLUMN limitations_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE messages ADD COLUMN provider_request_id TEXT;

ALTER TABLE llm_usage ADD COLUMN provider TEXT NOT NULL DEFAULT 'legacy';
ALTER TABLE llm_usage ADD COLUMN total_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE llm_usage ADD COLUMN cost_is_estimate INTEGER NOT NULL DEFAULT 0;

CREATE INDEX idx_conversations_user_match
    ON conversations(user_id, match_id, created_at);
