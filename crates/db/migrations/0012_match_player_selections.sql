-- A user's identity in one replay must not change another replay's selection.
CREATE TABLE match_player_selections (
    match_id TEXT PRIMARY KEY NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    player_id TEXT NOT NULL REFERENCES players(id) ON DELETE CASCADE
);

-- Preserve the effective legacy selection for each already-imported match.
INSERT INTO match_player_selections (match_id, player_id)
SELECT matches.id,
       (SELECT players.id
        FROM players
        JOIN valorant_accounts
          ON valorant_accounts.user_id = matches.user_id
         AND valorant_accounts.subject_id = players.stable_player_id
        WHERE players.match_id = matches.id
        ORDER BY valorant_accounts.id, players.id
        LIMIT 1)
FROM matches
WHERE EXISTS (
    SELECT 1
    FROM players
    JOIN valorant_accounts
      ON valorant_accounts.user_id = matches.user_id
     AND valorant_accounts.subject_id = players.stable_player_id
    WHERE players.match_id = matches.id
);
