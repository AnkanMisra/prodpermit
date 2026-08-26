CREATE TABLE release_configuration (
    session_id TEXT NOT NULL,
    release_id TEXT NOT NULL,
    config_key TEXT NOT NULL,
    config_value TEXT NOT NULL,
    redacted INTEGER NOT NULL CHECK (redacted IN (0, 1)),
    PRIMARY KEY (session_id, release_id, config_key),
    FOREIGN KEY (session_id, release_id) REFERENCES releases(session_id, id) ON DELETE CASCADE
);

CREATE TABLE log_events (
    session_id TEXT NOT NULL,
    id TEXT NOT NULL,
    recorded_at TEXT NOT NULL,
    severity TEXT NOT NULL CHECK (severity IN ('info', 'warn', 'error')),
    code TEXT NOT NULL,
    component TEXT NOT NULL,
    message TEXT NOT NULL,
    untrusted INTEGER NOT NULL CHECK (untrusted IN (0, 1)),
    PRIMARY KEY (session_id, id),
    FOREIGN KEY (session_id) REFERENCES demo_sessions(id) ON DELETE CASCADE
);

CREATE INDEX log_events_session_time_idx ON log_events(session_id, recorded_at DESC);

CREATE TABLE diagnostic_results (
    session_id TEXT NOT NULL,
    id TEXT NOT NULL,
    kind TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('passed', 'failed')),
    code TEXT NOT NULL,
    summary TEXT NOT NULL,
    evidence TEXT NOT NULL,
    checked_at TEXT NOT NULL,
    PRIMARY KEY (session_id, id),
    FOREIGN KEY (session_id) REFERENCES demo_sessions(id) ON DELETE CASCADE
);

CREATE TABLE audit_events (
    session_id TEXT NOT NULL,
    id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    subject_id TEXT,
    outcome TEXT NOT NULL,
    detail TEXT NOT NULL,
    recorded_at TEXT NOT NULL,
    PRIMARY KEY (session_id, id),
    FOREIGN KEY (session_id) REFERENCES demo_sessions(id) ON DELETE CASCADE
);

CREATE INDEX audit_events_session_time_idx ON audit_events(session_id, recorded_at DESC);

