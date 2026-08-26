PRAGMA foreign_keys = ON;

CREATE TABLE demo_sessions (
    id TEXT PRIMARY KEY NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    generation INTEGER NOT NULL CHECK (generation > 0)
);

CREATE TABLE services (
    session_id TEXT NOT NULL,
    id TEXT NOT NULL,
    health_status TEXT NOT NULL CHECK (health_status IN ('healthy', 'critical')),
    error_rate_percent REAL NOT NULL CHECK (error_rate_percent >= 0),
    p95_latency_ms INTEGER NOT NULL CHECK (p95_latency_ms >= 0),
    request_rate_rps INTEGER NOT NULL CHECK (request_rate_rps >= 0),
    current_release TEXT NOT NULL,
    PRIMARY KEY (session_id, id),
    FOREIGN KEY (session_id) REFERENCES demo_sessions(id) ON DELETE CASCADE
);

CREATE TABLE incidents (
    session_id TEXT NOT NULL,
    id TEXT NOT NULL,
    service_id TEXT NOT NULL,
    title TEXT NOT NULL,
    summary TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('active', 'resolved')),
    started_at TEXT NOT NULL,
    PRIMARY KEY (session_id, id),
    FOREIGN KEY (session_id, service_id) REFERENCES services(session_id, id) ON DELETE CASCADE
);

CREATE TABLE releases (
    session_id TEXT NOT NULL,
    id TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('healthy_baseline', 'deployed_faulty', 'staged')),
    commit_sha TEXT NOT NULL,
    description TEXT NOT NULL,
    deployed_at TEXT,
    PRIMARY KEY (session_id, id),
    FOREIGN KEY (session_id) REFERENCES demo_sessions(id) ON DELETE CASCADE
);

CREATE TABLE telemetry_points (
    session_id TEXT NOT NULL,
    service_id TEXT NOT NULL,
    recorded_at TEXT NOT NULL,
    error_rate_percent REAL NOT NULL CHECK (error_rate_percent >= 0),
    p95_latency_ms INTEGER NOT NULL CHECK (p95_latency_ms >= 0),
    request_rate_rps INTEGER NOT NULL CHECK (request_rate_rps >= 0),
    PRIMARY KEY (session_id, service_id, recorded_at),
    FOREIGN KEY (session_id, service_id) REFERENCES services(session_id, id) ON DELETE CASCADE
);

