CREATE TABLE recovery_plans (
    session_id TEXT NOT NULL,
    id TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('prepared', 'approved', 'executing', 'executed', 'rejected', 'expired', 'failed')),
    fingerprint TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    plan_json TEXT NOT NULL,
    PRIMARY KEY (session_id, id),
    FOREIGN KEY (session_id) REFERENCES demo_sessions(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX recovery_plans_one_active_per_session
ON recovery_plans(session_id)
WHERE status IN ('prepared', 'approved', 'executing');

