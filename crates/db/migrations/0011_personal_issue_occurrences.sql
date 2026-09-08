-- Older builds inserted the first observation with the schema default of zero.
-- A persisted issue has, by definition, been observed at least once.
UPDATE player_issues
SET occurrences = 1
WHERE occurrences < 1;
