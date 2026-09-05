ALTER TABLE players ADD COLUMN team TEXT;
ALTER TABLE players ADD COLUMN agent_name TEXT;
ALTER TABLE players ADD COLUMN player_slot INTEGER;

CREATE INDEX idx_players_match_team_slot
    ON players(match_id, team, player_slot);
