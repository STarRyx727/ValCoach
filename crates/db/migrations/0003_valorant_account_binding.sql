CREATE UNIQUE INDEX idx_valorant_accounts_user_subject
    ON valorant_accounts(user_id, subject_id)
    WHERE subject_id IS NOT NULL;
