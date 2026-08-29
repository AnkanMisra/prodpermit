CREATE TABLE session_resets_v2 (
    original_session_id TEXT PRIMARY KEY NOT NULL,
    replacement_session_id TEXT UNIQUE NOT NULL,
    reset_at TEXT NOT NULL,
    FOREIGN KEY (original_session_id) REFERENCES demo_sessions(id) ON DELETE CASCADE,
    FOREIGN KEY (replacement_session_id) REFERENCES demo_sessions(id) ON DELETE CASCADE,
    CHECK (original_session_id <> replacement_session_id)
);

INSERT INTO session_resets_v2 (original_session_id, replacement_session_id, reset_at)
SELECT original_session_id, replacement_session_id, reset_at
FROM session_resets;

DROP TABLE session_resets;
ALTER TABLE session_resets_v2 RENAME TO session_resets;

DROP TRIGGER recovery_plan_evidence_immutable_delete;
