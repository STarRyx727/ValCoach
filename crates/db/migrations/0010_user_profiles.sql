CREATE TABLE user_profiles (
    user_id TEXT PRIMARY KEY NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    rank_name TEXT,
    main_role TEXT,
    main_agents_json TEXT NOT NULL DEFAULT '[]',
    training_goals_json TEXT NOT NULL DEFAULT '[]',
    goal_notes TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
