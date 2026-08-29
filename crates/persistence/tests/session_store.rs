use std::path::PathBuf;

use chrono::{DateTime, Duration, TimeZone, Utc};
use recovery_domain::{
    DiagnosticStatus, HealthStatus, HumanDecision, LogSeverity, RecoveryError, RecoveryPlanState,
    RecoveryVerificationOutcome, ReleaseId, SessionId, seeded_scenario,
};
use recovery_persistence::{RecoveryPreparation, Store, StoreError};
use sqlx::{Connection, Row, SqliteConnection};
use uuid::Uuid;

#[tokio::test]
async fn session_store_round_trips_the_seeded_incident() {
    let store = Store::connect("sqlite::memory:")
        .await
        .expect("in-memory store connects");
    let now = fixture_time();
    let scenario = seeded_scenario(SessionId::new(), now);

    store
        .create_session(&scenario)
        .await
        .expect("session seed commits");

    let loaded = store
        .load_snapshot(&scenario.session.id)
        .await
        .expect("snapshot query succeeds")
        .expect("session exists");

    assert_eq!(loaded, scenario);
    assert_eq!(loaded.health.status, HealthStatus::Critical);
    assert_eq!(loaded.telemetry.len(), 30);

    let comparison = store
        .compare_releases(
            &scenario.session.id,
            &ReleaseId::from_static("release_283"),
            &ReleaseId::from_static("release_284"),
        )
        .await
        .expect("release comparison succeeds");
    assert_eq!(comparison.configuration_diff.len(), 1);
    assert!(comparison.configuration_diff[0].suspected_regression);

    let logs = store
        .query_logs(
            &scenario.session.id,
            Some(LogSeverity::Error),
            now - Duration::minutes(30),
            25,
        )
        .await
        .expect("log query succeeds");
    assert_eq!(logs.len(), 6);
    assert!(
        logs.iter()
            .all(|event| event.severity == LogSeverity::Error)
    );

    let diagnostic = store
        .run_database_diagnostic(&scenario.session.id, now)
        .await
        .expect("diagnostic succeeds");
    assert_eq!(diagnostic.status, DiagnosticStatus::Failed);
}

#[tokio::test]
async fn fresh_migration_is_normalized_and_enforces_foreign_keys() {
    let database = TestDatabase::new();
    let store = Store::connect(database.url())
        .await
        .expect("file-backed store connects");
    let scenario = seed_session(&store, fixture_time()).await;
    let mut connection = database.connect().await;

    let columns = sqlx::query("PRAGMA table_info(recovery_plans)")
        .fetch_all(&mut connection)
        .await
        .expect("normalized table metadata loads")
        .into_iter()
        .map(|row| row.get::<String, _>("name"))
        .collect::<Vec<_>>();
    assert!(columns.contains(&"incident_id".to_owned()));
    assert!(columns.contains(&"scenario_generation".to_owned()));
    assert!(columns.contains(&"expected_current_release".to_owned()));
    assert!(!columns.contains(&"plan_json".to_owned()));

    let revoked_column: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('demo_sessions') WHERE name = 'revoked_at'",
    )
    .fetch_one(&mut connection)
    .await
    .expect("session columns load");
    assert_eq!(revoked_column, 1);

    let release_owners: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM service_releases WHERE session_id = ? AND service_id = ?",
    )
    .bind(scenario.session.id.as_uuid().to_string())
    .bind(scenario.incident.service_id.as_str())
    .fetch_one(&mut connection)
    .await
    .expect("release ownership rows load");
    assert_eq!(release_owners, 3);

    let foreign_key_error = sqlx::query(
        "INSERT INTO service_releases (session_id, service_id, release_id) VALUES ('missing', 'checkout-api', 'release_283')",
    )
    .execute(&mut connection)
    .await
    .expect_err("a missing session cannot own a release");
    assert!(foreign_key_error.as_database_error().is_some());

    let violations = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&mut connection)
        .await
        .expect("foreign-key check runs");
    assert!(violations.is_empty());
}

#[tokio::test]
async fn session_creation_bounds_growth_and_prunes_expired_rows() {
    let database = TestDatabase::new();
    let store = Store::connect(database.url())
        .await
        .expect("file-backed store connects");
    let now = fixture_time();
    let mut connection = database.connect().await;
    for _ in 0..256 {
        sqlx::query(
            "INSERT INTO demo_sessions (id, created_at, expires_at, generation, revoked_at) VALUES (?, ?, ?, 1, NULL)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(now)
        .bind(now + Duration::hours(1))
        .execute(&mut connection)
        .await
        .expect("capacity fixture inserts");
    }

    let scenario = seeded_scenario(SessionId::new(), now);
    let full = store
        .create_session(&scenario)
        .await
        .expect_err("the active session ceiling bounds database growth");
    assert!(matches!(full, StoreError::SessionCapacity));

    sqlx::query(
        "UPDATE demo_sessions SET expires_at = ? WHERE id = (SELECT id FROM demo_sessions LIMIT 1)",
    )
    .bind(now)
    .execute(&mut connection)
    .await
    .expect("one fixture session expires");
    store
        .create_session(&scenario)
        .await
        .expect("expired rows are pruned before enforcing capacity");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM demo_sessions")
        .fetch_one(&mut connection)
        .await
        .expect("session count loads");
    assert_eq!(count, 256);
}

#[tokio::test]
async fn preparation_resolves_evidence_and_preserves_the_normalized_fingerprint() {
    let database = TestDatabase::new();
    let store = Store::connect(database.url())
        .await
        .expect("file-backed store connects");
    let now = fixture_time();
    let scenario = seed_session(&store, now).await;
    let diagnostic = store
        .run_database_diagnostic(&scenario.session.id, now + Duration::seconds(1))
        .await
        .expect("failed diagnostic is persisted with context");

    let prepared = store
        .prepare_recovery(
            &scenario.session.id,
            RecoveryPreparation {
                target_release: ReleaseId::from_static("release_283"),
                reason: "  Rollback the database authentication regression.  ".to_owned(),
                evidence_refs: vec![diagnostic.id.clone(), "log_db_auth_1".to_owned()],
            },
            now + Duration::minutes(1),
        )
        .await
        .expect("related evidence prepares one normalized plan");

    assert_eq!(prepared.state, RecoveryPlanState::Prepared);
    assert_eq!(
        prepared.spec.reason(),
        "Rollback the database authentication regression."
    );
    assert_eq!(
        prepared
            .spec
            .evidence()
            .iter()
            .map(|item| item.id().as_str())
            .collect::<Vec<_>>(),
        vec!["log_db_auth_1", diagnostic.id.as_str()]
    );

    let restored = store
        .current_recovery(&scenario.session.id, now + Duration::minutes(1))
        .await
        .expect("current recovery query succeeds")
        .expect("prepared plan is current");
    assert_eq!(restored, prepared);

    let stored_fingerprint: String = sqlx::query_scalar(
        "SELECT fingerprint FROM recovery_plans WHERE session_id = ? AND id = ?",
    )
    .bind(scenario.session.id.as_uuid().to_string())
    .bind(prepared.spec.plan_id().as_uuid().to_string())
    .fetch_one(&mut database.connect().await)
    .await
    .expect("stored fingerprint loads");
    assert_eq!(stored_fingerprint, prepared.fingerprint.as_str());

    let unknown = store
        .prepare_recovery(
            &scenario.session.id,
            RecoveryPreparation {
                target_release: ReleaseId::from_static("release_283"),
                reason: "Unknown evidence must not resolve.".to_owned(),
                evidence_refs: vec!["log_db_auth_1".to_owned(), "missing".to_owned()],
            },
            now + Duration::minutes(2),
        )
        .await
        .expect_err("unknown evidence is rejected before persistence");
    assert!(matches!(unknown, StoreError::InvalidRecoveryEvidence));

    let active_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM recovery_plans WHERE session_id = ? AND status IN ('prepared', 'approved', 'executing')",
    )
    .bind(scenario.session.id.as_uuid().to_string())
    .fetch_one(&mut database.connect().await)
    .await
    .expect("active plan count loads");
    assert_eq!(active_count, 1);
}

#[tokio::test]
async fn current_recovery_persists_deadline_expiry_once() {
    let database = TestDatabase::new();
    let store = Store::connect(database.url())
        .await
        .expect("file-backed store connects");
    let now = fixture_time();
    let prepared = prepare_valid_recovery(&store, now).await;

    let expired = store
        .current_recovery(prepared.spec.session_id(), prepared.spec.expires_at())
        .await
        .expect("deadline query succeeds")
        .expect("expired plan remains current");
    assert_eq!(expired.state, RecoveryPlanState::Expired);

    let durable = store
        .current_recovery(
            prepared.spec.session_id(),
            prepared.spec.created_at() + Duration::seconds(1),
        )
        .await
        .expect("second query succeeds")
        .expect("terminal plan remains current");
    assert_eq!(durable.state, RecoveryPlanState::Expired);

    let audit_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_events WHERE session_id = ? AND event_type = 'recovery_expired'",
    )
    .bind(prepared.spec.session_id().as_uuid().to_string())
    .fetch_one(&mut database.connect().await)
    .await
    .expect("expiry audit count loads");
    assert_eq!(audit_count, 1);
}

#[tokio::test]
async fn decisions_are_conditional_and_audit_failure_rolls_back_state() {
    let database = TestDatabase::new();
    let store = Store::connect(database.url())
        .await
        .expect("file-backed store connects");
    let now = fixture_time();
    let prepared = prepare_valid_recovery(&store, now).await;
    let plan_id = prepared.spec.plan_id().clone();
    let session_id = prepared.spec.session_id().clone();

    let wrong_fingerprint = store
        .decide_recovery(
            &session_id,
            &plan_id,
            HumanDecision::Approve {
                fingerprint: "0".repeat(64),
            },
            now + Duration::minutes(2),
        )
        .await
        .expect_err("approval is bound to the canonical fingerprint");
    assert!(matches!(
        wrong_fingerprint,
        StoreError::Recovery(RecoveryError::FingerprintMismatch)
    ));

    let mut connection = database.connect().await;
    sqlx::query(
        "CREATE TRIGGER fail_human_approval BEFORE INSERT ON audit_events WHEN NEW.event_type = 'human_approval' BEGIN SELECT RAISE(ABORT, 'audit unavailable'); END",
    )
    .execute(&mut connection)
    .await
    .expect("real SQLite trigger installs");
    let failed = store
        .decide_recovery(
            &session_id,
            &plan_id,
            HumanDecision::Approve {
                fingerprint: prepared.fingerprint.as_str().to_owned(),
            },
            now + Duration::minutes(2),
        )
        .await;
    assert!(matches!(failed, Err(StoreError::Database(_))));
    let after_failure = store
        .current_recovery(&session_id, now + Duration::minutes(2))
        .await
        .expect("plan reload succeeds")
        .expect("plan remains current");
    assert_eq!(after_failure.state, RecoveryPlanState::Prepared);

    sqlx::query("DROP TRIGGER fail_human_approval")
        .execute(&mut connection)
        .await
        .expect("test trigger drops");
    let approved = store
        .decide_recovery(
            &session_id,
            &plan_id,
            HumanDecision::Approve {
                fingerprint: prepared.fingerprint.as_str().to_owned(),
            },
            now + Duration::minutes(2),
        )
        .await
        .expect("exact approval commits with its audit");
    let replayed = store
        .decide_recovery(
            &session_id,
            &plan_id,
            HumanDecision::Approve {
                fingerprint: prepared.fingerprint.as_str().to_owned(),
            },
            now + Duration::minutes(3),
        )
        .await
        .expect("exact approval is idempotent");
    assert_eq!(replayed.state, approved.state);

    let rejected = store
        .decide_recovery(
            &session_id,
            &plan_id,
            HumanDecision::Reject,
            now + Duration::minutes(4),
        )
        .await
        .expect("rejection revokes unexecuted approval");
    assert!(matches!(rejected.state, RecoveryPlanState::Rejected { .. }));

    let decision_audits: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_events WHERE session_id = ? AND event_type IN ('human_approval', 'human_rejection')",
    )
    .bind(session_id.as_uuid().to_string())
    .fetch_one(&mut connection)
    .await
    .expect("decision audit count loads");
    assert_eq!(decision_audits, 2);
}

#[tokio::test]
async fn execution_has_one_winner_and_rolls_back_every_effect_when_audit_fails() {
    let database = TestDatabase::new();
    let first_store = Store::connect(database.url())
        .await
        .expect("first store connects");
    let now = fixture_time();
    let prepared = prepare_valid_recovery(&first_store, now).await;
    let approved = first_store
        .decide_recovery(
            prepared.spec.session_id(),
            prepared.spec.plan_id(),
            HumanDecision::Approve {
                fingerprint: prepared.fingerprint.as_str().to_owned(),
            },
            now + Duration::minutes(2),
        )
        .await
        .expect("plan is approved");

    let mut connection = database.connect().await;
    sqlx::query(
        "CREATE TRIGGER fail_recovery_execution BEFORE INSERT ON audit_events WHEN NEW.event_type = 'recovery_execution' BEGIN SELECT RAISE(ABORT, 'audit unavailable'); END",
    )
    .execute(&mut connection)
    .await
    .expect("execution audit trigger installs");
    let failed = first_store
        .execute_recovery(
            approved.spec.session_id(),
            approved.spec.plan_id(),
            now + Duration::minutes(3),
        )
        .await;
    assert!(matches!(failed, Err(StoreError::Database(_))));
    let snapshot = first_store
        .load_snapshot(approved.spec.session_id())
        .await
        .expect("snapshot reload succeeds")
        .expect("session remains active");
    assert_eq!(snapshot.health.current_release.as_str(), "release_284");
    assert_eq!(snapshot.health.status, HealthStatus::Critical);
    let after_failure = first_store
        .current_recovery(approved.spec.session_id(), now + Duration::minutes(3))
        .await
        .expect("plan reload succeeds")
        .expect("plan remains current");
    assert!(matches!(
        after_failure.state,
        RecoveryPlanState::Approved { .. }
    ));

    sqlx::query("DROP TRIGGER fail_recovery_execution")
        .execute(&mut connection)
        .await
        .expect("test trigger drops");
    let second_store = Store::connect(database.url())
        .await
        .expect("second independent pool connects");
    let session_id = approved.spec.session_id().clone();
    let plan_id = approved.spec.plan_id().clone();
    let execution_time = now + Duration::minutes(3);
    let (first, second) = tokio::join!(
        first_store.execute_recovery(&session_id, &plan_id, execution_time),
        second_store.execute_recovery(&session_id, &plan_id, execution_time),
    );
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    let loser = if first.is_err() { first } else { second };
    assert!(matches!(
        loser,
        Err(StoreError::Recovery(RecoveryError::AlreadyExecuted))
    ));

    let execution_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM recovery_plan_executions WHERE session_id = ? AND plan_id = ?",
    )
    .bind(session_id.as_uuid().to_string())
    .bind(plan_id.as_uuid().to_string())
    .fetch_one(&mut connection)
    .await
    .expect("execution link count loads");
    assert_eq!(execution_count, 1);
    let diagnostic_status: String = sqlx::query_scalar(
        "SELECT d.status FROM recovery_plan_executions e JOIN diagnostic_results d ON d.session_id = e.session_id AND d.id = e.diagnostic_id WHERE e.session_id = ? AND e.plan_id = ?",
    )
    .bind(session_id.as_uuid().to_string())
    .bind(plan_id.as_uuid().to_string())
    .fetch_one(&mut connection)
    .await
    .expect("linked diagnostic loads");
    assert_eq!(diagnostic_status, "passed");
}

#[tokio::test]
async fn verification_joins_the_exact_before_and_after_evidence() {
    let database = TestDatabase::new();
    let store = Store::connect(database.url())
        .await
        .expect("file-backed store connects");
    let now = fixture_time();
    let prepared = prepare_valid_recovery(&store, now).await;
    let approved = store
        .decide_recovery(
            prepared.spec.session_id(),
            prepared.spec.plan_id(),
            HumanDecision::Approve {
                fingerprint: prepared.fingerprint.as_str().to_owned(),
            },
            now + Duration::minutes(2),
        )
        .await
        .expect("plan is approved");
    let executed = store
        .execute_recovery(
            approved.spec.session_id(),
            approved.spec.plan_id(),
            now + Duration::minutes(3),
        )
        .await
        .expect("approved plan executes");

    let first = store
        .verify_recovery(
            executed.spec.session_id(),
            executed.spec.plan_id(),
            now + Duration::minutes(4),
        )
        .await
        .expect("persisted recovery verifies");
    assert_eq!(first.outcome, RecoveryVerificationOutcome::Passed);
    assert_eq!(first.before.evidence.iter().count(), 2);
    assert_eq!(first.after.release.as_str(), "release_283");
    assert_eq!(first.after.health_status, HealthStatus::Healthy);
    assert_eq!(first.after.diagnostic.status, DiagnosticStatus::Passed);
    assert_eq!(first.after.diagnostic.plan_id, *executed.spec.plan_id());
    assert_eq!(first.after.telemetry.plan_id, *executed.spec.plan_id());

    let second = store
        .verify_recovery(
            executed.spec.session_id(),
            executed.spec.plan_id(),
            now + Duration::minutes(5),
        )
        .await
        .expect("verification retry returns persisted facts");
    assert_eq!(second.outcome, RecoveryVerificationOutcome::Passed);

    let audit_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_events WHERE session_id = ? AND event_type = 'recovery_verification'",
    )
    .bind(executed.spec.session_id().as_uuid().to_string())
    .fetch_one(&mut database.connect().await)
    .await
    .expect("verification audit count loads");
    assert_eq!(audit_count, 1);
}

#[tokio::test]
async fn reset_revokes_old_authority_without_reissuing_the_replacement() {
    let database = TestDatabase::new();
    let store = Store::connect(database.url())
        .await
        .expect("file-backed store connects");
    let now = fixture_time();
    let prepared = prepare_valid_recovery(&store, now).await;
    let old_session = prepared.spec.session_id().clone();

    let replacement = store
        .reset_session(&old_session, now + Duration::minutes(2))
        .await
        .expect("first reset creates a replacement scenario");
    assert_ne!(replacement.session.id, old_session);
    assert_eq!(replacement.session.generation, 1);
    assert_eq!(replacement.health.status, HealthStatus::Critical);
    assert_eq!(replacement.health.current_release.as_str(), "release_284");
    assert!(
        store
            .load_snapshot(&old_session)
            .await
            .expect("old snapshot query succeeds")
            .is_none()
    );

    let revoked_execution = store
        .execute_recovery(
            &old_session,
            prepared.spec.plan_id(),
            now + Duration::minutes(3),
        )
        .await
        .expect_err("revoked session cannot use its old authority");
    assert!(matches!(revoked_execution, StoreError::SessionInactive));

    let retried = store
        .reset_session(&old_session, now + Duration::minutes(3))
        .await
        .expect_err("a revoked cookie cannot recover replacement authority");
    assert!(matches!(retried, StoreError::SessionInactive));

    let mut connection = database.connect().await;
    let lineage_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM session_resets WHERE original_session_id = ? AND replacement_session_id = ?",
    )
    .bind(old_session.as_uuid().to_string())
    .bind(replacement.session.id.as_uuid().to_string())
    .fetch_one(&mut connection)
    .await
    .expect("reset lineage count loads");
    assert_eq!(lineage_count, 1);
    let (status, reason): (String, Option<String>) = sqlx::query_as(
        "SELECT status, invalidation_reason FROM recovery_plans WHERE session_id = ? AND id = ?",
    )
    .bind(old_session.as_uuid().to_string())
    .bind(prepared.spec.plan_id().as_uuid().to_string())
    .fetch_one(&mut connection)
    .await
    .expect("old plan state loads");
    assert_eq!(status, "invalidated");
    assert_eq!(reason.as_deref(), Some("session_reset"));

    let fresh = seeded_scenario(SessionId::new(), now + Duration::minutes(3));
    store
        .create_session(&fresh)
        .await
        .expect("creating a session prunes the complete revoked scenario");
    let old_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM demo_sessions WHERE id = ?")
        .bind(old_session.as_uuid().to_string())
        .fetch_one(&mut connection)
        .await
        .expect("old session count loads");
    assert_eq!(old_rows, 0);
}

async fn prepare_valid_recovery(
    store: &Store,
    now: DateTime<Utc>,
) -> recovery_persistence::PersistedRecoveryPlan {
    let scenario = seed_session(store, now).await;
    let diagnostic = store
        .run_database_diagnostic(&scenario.session.id, now + Duration::seconds(1))
        .await
        .expect("failed diagnostic is persisted");
    store
        .prepare_recovery(
            &scenario.session.id,
            RecoveryPreparation {
                target_release: ReleaseId::from_static("release_283"),
                reason: "Rollback the database authentication regression.".to_owned(),
                evidence_refs: vec!["log_db_auth_1".to_owned(), diagnostic.id],
            },
            now + Duration::minutes(1),
        )
        .await
        .expect("valid recovery is prepared")
}

async fn seed_session(store: &Store, now: DateTime<Utc>) -> recovery_domain::IncidentSnapshot {
    let scenario = seeded_scenario(SessionId::new(), now);
    store
        .create_session(&scenario)
        .await
        .expect("session seed commits");
    scenario
}

fn fixture_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 26, 5, 0, 0)
        .single()
        .expect("fixture timestamp is valid")
}

struct TestDatabase {
    path: PathBuf,
    url: String,
}

impl TestDatabase {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("recovery-persistence-{}.sqlite", Uuid::new_v4()));
        let url = format!("sqlite://{}", path.display());
        Self { path, url }
    }

    fn url(&self) -> &str {
        &self.url
    }

    async fn connect(&self) -> SqliteConnection {
        let mut connection = SqliteConnection::connect(&self.url)
            .await
            .expect("raw test connection opens");
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&mut connection)
            .await
            .expect("foreign keys enable on raw connection");
        connection
    }
}

impl Drop for TestDatabase {
    fn drop(&mut self) {
        for path in [
            self.path.clone(),
            PathBuf::from(format!("{}-shm", self.path.display())),
            PathBuf::from(format!("{}-wal", self.path.display())),
        ] {
            let _ = std::fs::remove_file(path);
        }
    }
}
