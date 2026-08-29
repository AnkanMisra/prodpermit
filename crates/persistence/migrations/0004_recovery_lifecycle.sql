ALTER TABLE demo_sessions ADD COLUMN revoked_at TEXT;
ALTER TABLE audit_events ADD COLUMN dedup_key TEXT;

CREATE UNIQUE INDEX audit_events_session_dedup_idx
ON audit_events(session_id, dedup_key)
WHERE dedup_key IS NOT NULL;

CREATE TABLE session_resets (
    original_session_id TEXT PRIMARY KEY NOT NULL,
    replacement_session_id TEXT UNIQUE NOT NULL,
    reset_at TEXT NOT NULL,
    FOREIGN KEY (original_session_id) REFERENCES demo_sessions(id) ON DELETE CASCADE,
    FOREIGN KEY (replacement_session_id) REFERENCES demo_sessions(id) ON DELETE CASCADE,
    CHECK (original_session_id <> replacement_session_id)
);

CREATE TABLE service_releases (
    session_id TEXT NOT NULL,
    service_id TEXT NOT NULL,
    release_id TEXT NOT NULL,
    PRIMARY KEY (session_id, service_id, release_id),
    FOREIGN KEY (session_id, service_id) REFERENCES services(session_id, id) ON DELETE CASCADE,
    FOREIGN KEY (session_id, release_id) REFERENCES releases(session_id, id) ON DELETE CASCADE
);

INSERT INTO service_releases (session_id, service_id, release_id)
SELECT services.session_id, services.id, releases.id
FROM services
JOIN releases ON releases.session_id = services.session_id;

DROP INDEX recovery_plans_one_active_per_session;
DROP TABLE recovery_plans;

CREATE TABLE recovery_plans (
    session_id TEXT NOT NULL,
    id TEXT NOT NULL,
    incident_id TEXT NOT NULL,
    service_id TEXT NOT NULL,
    scenario_generation INTEGER NOT NULL CHECK (scenario_generation > 0),
    expected_current_release TEXT NOT NULL,
    target_release TEXT NOT NULL,
    reason TEXT NOT NULL CHECK (length(reason) BETWEEN 1 AND 240),
    policy_version INTEGER NOT NULL CHECK (policy_version > 0),
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    fingerprint TEXT NOT NULL CHECK (
        length(fingerprint) = 64
        AND fingerprint NOT GLOB '*[^0-9a-f]*'
    ),
    status TEXT NOT NULL CHECK (
        status IN ('prepared', 'approved', 'executing', 'executed', 'rejected', 'expired', 'invalidated')
    ),
    approved_at TEXT,
    approved_fingerprint TEXT CHECK (
        approved_fingerprint IS NULL
        OR (
            length(approved_fingerprint) = 64
            AND approved_fingerprint NOT GLOB '*[^0-9a-f]*'
        )
    ),
    execution_started_at TEXT,
    executed_at TEXT,
    rejected_at TEXT,
    invalidation_reason TEXT CHECK (
        invalidation_reason IS NULL
        OR invalidation_reason IN (
            'session_reset',
            'scenario_generation_changed',
            'active_release_changed',
            'target_became_ineligible'
        )
    ),
    invalidated_at TEXT,
    PRIMARY KEY (session_id, id),
    FOREIGN KEY (session_id) REFERENCES demo_sessions(id),
    FOREIGN KEY (session_id, incident_id) REFERENCES incidents(session_id, id),
    FOREIGN KEY (session_id, service_id) REFERENCES services(session_id, id),
    FOREIGN KEY (session_id, service_id, expected_current_release)
        REFERENCES service_releases(session_id, service_id, release_id),
    FOREIGN KEY (session_id, service_id, target_release)
        REFERENCES service_releases(session_id, service_id, release_id),
    CHECK (expected_current_release <> target_release),
    CHECK (expires_at > created_at),
    CHECK (
        (status = 'prepared'
            AND approved_at IS NULL
            AND approved_fingerprint IS NULL
            AND execution_started_at IS NULL
            AND executed_at IS NULL
            AND rejected_at IS NULL
            AND invalidation_reason IS NULL
            AND invalidated_at IS NULL)
        OR (status = 'approved'
            AND approved_at IS NOT NULL
            AND approved_fingerprint IS NOT NULL
            AND execution_started_at IS NULL
            AND executed_at IS NULL
            AND rejected_at IS NULL
            AND invalidation_reason IS NULL
            AND invalidated_at IS NULL)
        OR (status = 'executing'
            AND approved_at IS NOT NULL
            AND approved_fingerprint IS NOT NULL
            AND execution_started_at IS NOT NULL
            AND executed_at IS NULL
            AND rejected_at IS NULL
            AND invalidation_reason IS NULL
            AND invalidated_at IS NULL)
        OR (status = 'executed'
            AND approved_at IS NOT NULL
            AND approved_fingerprint IS NOT NULL
            AND execution_started_at IS NOT NULL
            AND executed_at IS NOT NULL
            AND rejected_at IS NULL
            AND invalidation_reason IS NULL
            AND invalidated_at IS NULL)
        OR (status = 'rejected'
            AND approved_at IS NULL
            AND approved_fingerprint IS NULL
            AND execution_started_at IS NULL
            AND executed_at IS NULL
            AND rejected_at IS NOT NULL
            AND invalidation_reason IS NULL
            AND invalidated_at IS NULL)
        OR (status = 'expired'
            AND approved_at IS NULL
            AND approved_fingerprint IS NULL
            AND execution_started_at IS NULL
            AND executed_at IS NULL
            AND rejected_at IS NULL
            AND invalidation_reason IS NULL
            AND invalidated_at IS NULL)
        OR (status = 'invalidated'
            AND approved_at IS NULL
            AND approved_fingerprint IS NULL
            AND execution_started_at IS NULL
            AND executed_at IS NULL
            AND rejected_at IS NULL
            AND invalidation_reason IS NOT NULL
            AND invalidated_at IS NOT NULL)
    )
);

CREATE UNIQUE INDEX recovery_plans_one_active_per_session
ON recovery_plans(session_id)
WHERE status IN ('prepared', 'approved', 'executing');

CREATE INDEX recovery_plans_session_created_idx
ON recovery_plans(session_id, created_at DESC, id DESC);

CREATE TRIGGER recovery_plans_immutable_facts
BEFORE UPDATE OF
    session_id,
    id,
    incident_id,
    service_id,
    scenario_generation,
    expected_current_release,
    target_release,
    reason,
    policy_version,
    created_at,
    expires_at,
    fingerprint
ON recovery_plans
BEGIN
    SELECT RAISE(ABORT, 'recovery plan facts are immutable');
END;

CREATE TABLE recovery_plan_evidence (
    session_id TEXT NOT NULL,
    plan_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal BETWEEN 0 AND 7),
    kind TEXT NOT NULL CHECK (
        kind IN ('database_authentication_failure_log', 'failed_database_connectivity_diagnostic')
    ),
    evidence_id TEXT NOT NULL,
    log_event_id TEXT,
    diagnostic_id TEXT,
    PRIMARY KEY (session_id, plan_id, ordinal),
    UNIQUE (session_id, plan_id, evidence_id),
    FOREIGN KEY (session_id, plan_id) REFERENCES recovery_plans(session_id, id) ON DELETE CASCADE,
    FOREIGN KEY (session_id, log_event_id) REFERENCES log_events(session_id, id),
    FOREIGN KEY (session_id, diagnostic_id) REFERENCES diagnostic_results(session_id, id),
    CHECK (
        (kind = 'database_authentication_failure_log'
            AND evidence_id = log_event_id
            AND log_event_id IS NOT NULL
            AND diagnostic_id IS NULL)
        OR (kind = 'failed_database_connectivity_diagnostic'
            AND evidence_id = diagnostic_id
            AND diagnostic_id IS NOT NULL
            AND log_event_id IS NULL)
    )
);

CREATE TRIGGER recovery_plan_evidence_immutable_update
BEFORE UPDATE ON recovery_plan_evidence
BEGIN
    SELECT RAISE(ABORT, 'recovery plan evidence is immutable');
END;

CREATE TABLE diagnostic_contexts (
    session_id TEXT NOT NULL,
    diagnostic_id TEXT NOT NULL,
    service_id TEXT NOT NULL,
    release_id TEXT NOT NULL,
    scenario_generation INTEGER NOT NULL CHECK (scenario_generation > 0),
    plan_id TEXT,
    PRIMARY KEY (session_id, diagnostic_id),
    FOREIGN KEY (session_id, diagnostic_id) REFERENCES diagnostic_results(session_id, id) ON DELETE CASCADE,
    FOREIGN KEY (session_id, service_id, release_id) REFERENCES service_releases(session_id, service_id, release_id),
    FOREIGN KEY (session_id, plan_id) REFERENCES recovery_plans(session_id, id)
);

CREATE TABLE recovery_plan_executions (
    session_id TEXT NOT NULL,
    plan_id TEXT NOT NULL,
    service_id TEXT NOT NULL,
    scenario_generation INTEGER NOT NULL CHECK (scenario_generation > 0),
    previous_release TEXT NOT NULL,
    current_release TEXT NOT NULL,
    telemetry_recorded_at TEXT NOT NULL,
    diagnostic_id TEXT NOT NULL,
    executed_at TEXT NOT NULL,
    PRIMARY KEY (session_id, plan_id),
    UNIQUE (session_id, diagnostic_id),
    FOREIGN KEY (session_id, plan_id) REFERENCES recovery_plans(session_id, id),
    FOREIGN KEY (session_id, service_id, previous_release)
        REFERENCES service_releases(session_id, service_id, release_id),
    FOREIGN KEY (session_id, service_id, current_release)
        REFERENCES service_releases(session_id, service_id, release_id),
    FOREIGN KEY (session_id, service_id, telemetry_recorded_at)
        REFERENCES telemetry_points(session_id, service_id, recorded_at),
    FOREIGN KEY (session_id, diagnostic_id)
        REFERENCES diagnostic_results(session_id, id),
    CHECK (previous_release <> current_release)
);
