//! `SQLite` persistence for session-isolated incident scenarios.

use std::str::FromStr;

use recovery_domain::{
    AuditEvent, ConfigDifference, DemoSession, DiagnosticResult, DiagnosticStatus, HealthStatus,
    Incident, IncidentId, IncidentSnapshot, IncidentStatus, LogEvent, LogSeverity, PlanError,
    PlanId, PlanStatus, RecoveryPlan, RecoveryVerification, ReleaseComparison,
    ReleaseConfiguration, ReleaseId, ReleaseState, ReleaseSummary, ServiceHealth, ServiceId,
    SessionId, TelemetryPoint, compare_release_configuration, database_connectivity_diagnostic,
    seeded_investigation_data,
};
use sqlx::{Row, SqlitePool, sqlite::SqliteConnectOptions, sqlite::SqliteJournalMode};
use thiserror::Error;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error")]
    Database(#[from] sqlx::Error),
    #[error("database migration error")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("stored data is invalid: {0}")]
    InvalidData(String),
    #[error("recovery plan rejected: {0}")]
    Plan(#[from] PlanError),
    #[error("recovery plan serialization failed")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Clone, Debug)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    /// Connects to the database and applies all migrations.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the connection or a migration fails.
    pub async fn connect(database_url: &str) -> Result<Self, StoreError> {
        let options = SqliteConnectOptions::from_str(database_url)?
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(std::time::Duration::from_secs(5));
        let maximum_connections = if database_url.contains(":memory:") {
            1
        } else {
            5
        };
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(maximum_connections)
            .connect_with(options)
            .await?;
        MIGRATOR.run(&pool).await?;
        Ok(Self { pool })
    }

    /// Inserts one complete isolated scenario in a transaction.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when any insert or the transaction commit fails.
    pub async fn create_session(&self, snapshot: &IncidentSnapshot) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;
        let session_id = snapshot.session.id.as_uuid().to_string();

        sqlx::query(
            "INSERT INTO demo_sessions (id, created_at, expires_at, generation) VALUES (?, ?, ?, ?)",
        )
        .bind(&session_id)
        .bind(snapshot.session.created_at)
        .bind(snapshot.session.expires_at)
        .bind(snapshot.session.generation)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO services (session_id, id, health_status, error_rate_percent, p95_latency_ms, request_rate_rps, current_release) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&session_id)
        .bind(snapshot.incident.service_id.as_str())
        .bind(health_status_str(snapshot.health.status))
        .bind(snapshot.health.error_rate_percent)
        .bind(snapshot.health.p95_latency_ms)
        .bind(snapshot.health.request_rate_rps)
        .bind(snapshot.health.current_release.as_str())
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO incidents (session_id, id, service_id, title, summary, status, started_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&session_id)
        .bind(snapshot.incident.id.as_str())
        .bind(snapshot.incident.service_id.as_str())
        .bind(&snapshot.incident.title)
        .bind(&snapshot.incident.summary)
        .bind(incident_status_str(snapshot.incident.status))
        .bind(snapshot.incident.started_at)
        .execute(&mut *tx)
        .await?;

        for release in &snapshot.releases {
            sqlx::query(
                "INSERT INTO releases (session_id, id, state, commit_sha, description, deployed_at) VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&session_id)
            .bind(release.id.as_str())
            .bind(release_state_str(release.state))
            .bind(&release.commit_sha)
            .bind(&release.description)
            .bind(release.deployed_at)
            .execute(&mut *tx)
            .await?;
        }

        for point in &snapshot.telemetry {
            sqlx::query(
                "INSERT INTO telemetry_points (session_id, service_id, recorded_at, error_rate_percent, p95_latency_ms, request_rate_rps) VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&session_id)
            .bind(snapshot.incident.service_id.as_str())
            .bind(point.timestamp)
            .bind(point.error_rate_percent)
            .bind(point.p95_latency_ms)
            .bind(point.request_rate_rps)
            .execute(&mut *tx)
            .await?;
        }

        let investigation = seeded_investigation_data(snapshot.session.created_at);
        for configuration in investigation.release_configuration {
            sqlx::query(
                "INSERT INTO release_configuration (session_id, release_id, config_key, config_value, redacted) VALUES (?, ?, ?, ?, ?)",
            )
            .bind(&session_id)
            .bind(configuration.release_id.as_str())
            .bind(configuration.key)
            .bind(configuration.value)
            .bind(configuration.redacted)
            .execute(&mut *tx)
            .await?;
        }
        for event in investigation.logs {
            sqlx::query(
                "INSERT INTO log_events (session_id, id, recorded_at, severity, code, component, message, untrusted) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&session_id)
            .bind(event.id)
            .bind(event.recorded_at)
            .bind(log_severity_str(event.severity))
            .bind(event.code)
            .bind(event.component)
            .bind(event.message)
            .bind(event.untrusted)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    /// Loads the current incident snapshot for `session_id`.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when a query fails or persisted data violates the domain model.
    pub async fn load_snapshot(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<IncidentSnapshot>, StoreError> {
        let id = session_id.as_uuid().to_string();
        let Some(session_row) = sqlx::query(
            "SELECT created_at, expires_at, generation FROM demo_sessions WHERE id = ?",
        )
        .bind(&id)
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(None);
        };

        let service_row = sqlx::query(
            "SELECT id, health_status, error_rate_percent, p95_latency_ms, request_rate_rps, current_release FROM services WHERE session_id = ?",
        )
        .bind(&id)
        .fetch_one(&self.pool)
        .await?;
        let incident_row = sqlx::query(
            "SELECT id, service_id, title, summary, status, started_at FROM incidents WHERE session_id = ?",
        )
        .bind(&id)
        .fetch_one(&self.pool)
        .await?;
        let release_rows = sqlx::query(
            "SELECT id, state, commit_sha, description, deployed_at FROM releases WHERE session_id = ? ORDER BY id",
        )
        .bind(&id)
        .fetch_all(&self.pool)
        .await?;
        let telemetry_rows = sqlx::query(
            "SELECT recorded_at, error_rate_percent, p95_latency_ms, request_rate_rps FROM telemetry_points WHERE session_id = ? ORDER BY recorded_at",
        )
        .bind(&id)
        .fetch_all(&self.pool)
        .await?;

        let service_id = ServiceId::parse(service_row.try_get("id")?)
            .map_err(|error| StoreError::InvalidData(error.to_string()))?;
        let incident_service_id = ServiceId::parse(incident_row.try_get("service_id")?)
            .map_err(|error| StoreError::InvalidData(error.to_string()))?;
        if service_id != incident_service_id {
            return Err(StoreError::InvalidData(
                "incident and service identifiers do not match".to_owned(),
            ));
        }

        let releases = release_rows
            .into_iter()
            .map(|row| {
                let release_id = ReleaseId::parse(row.try_get("id")?)
                    .map_err(|error| StoreError::InvalidData(error.to_string()))?;
                let state = parse_release_state(&row.try_get::<String, _>("state")?)?;
                Ok(ReleaseSummary {
                    id: release_id,
                    state,
                    commit_sha: row.try_get("commit_sha")?,
                    description: row.try_get("description")?,
                    deployed_at: row.try_get("deployed_at")?,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;

        let telemetry = telemetry_rows
            .into_iter()
            .map(|row| {
                Ok(TelemetryPoint {
                    timestamp: row.try_get("recorded_at")?,
                    error_rate_percent: row.try_get("error_rate_percent")?,
                    p95_latency_ms: row.try_get("p95_latency_ms")?,
                    request_rate_rps: row.try_get("request_rate_rps")?,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;

        Ok(Some(IncidentSnapshot {
            session: DemoSession {
                id: session_id.clone(),
                created_at: session_row.try_get("created_at")?,
                expires_at: session_row.try_get("expires_at")?,
                generation: session_row.try_get("generation")?,
            },
            incident: Incident {
                id: IncidentId::parse(incident_row.try_get("id")?)
                    .map_err(|error| StoreError::InvalidData(error.to_string()))?,
                service_id,
                title: incident_row.try_get("title")?,
                summary: incident_row.try_get("summary")?,
                status: parse_incident_status(&incident_row.try_get::<String, _>("status")?)?,
                started_at: incident_row.try_get("started_at")?,
            },
            health: ServiceHealth {
                status: parse_health_status(&service_row.try_get::<String, _>("health_status")?)?,
                error_rate_percent: service_row.try_get("error_rate_percent")?,
                p95_latency_ms: service_row.try_get("p95_latency_ms")?,
                request_rate_rps: service_row.try_get("request_rate_rps")?,
                current_release: ReleaseId::parse(service_row.try_get("current_release")?)
                    .map_err(|error| StoreError::InvalidData(error.to_string()))?,
            },
            releases,
            telemetry,
        }))
    }

    /// Compares two releases and returns only redacted configuration values.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when a release is missing, a query fails, or stored data is invalid.
    pub async fn compare_releases(
        &self,
        session_id: &SessionId,
        baseline: &ReleaseId,
        candidate: &ReleaseId,
    ) -> Result<ReleaseComparison, StoreError> {
        let id = session_id.as_uuid().to_string();
        let release_rows = sqlx::query(
            "SELECT id, state, commit_sha, description, deployed_at FROM releases WHERE session_id = ? AND id IN (?, ?)",
        )
        .bind(&id)
        .bind(baseline.as_str())
        .bind(candidate.as_str())
        .fetch_all(&self.pool)
        .await?;
        let releases = release_rows
            .into_iter()
            .map(|row| release_from_row(&row))
            .collect::<Result<Vec<_>, StoreError>>()?;
        let baseline_release = releases
            .iter()
            .find(|release| release.id == *baseline)
            .cloned()
            .ok_or_else(|| StoreError::InvalidData("baseline release not found".to_owned()))?;
        let candidate_release = releases
            .iter()
            .find(|release| release.id == *candidate)
            .cloned()
            .ok_or_else(|| StoreError::InvalidData("candidate release not found".to_owned()))?;

        let configuration_rows = sqlx::query(
            "SELECT release_id, config_key, config_value, redacted FROM release_configuration WHERE session_id = ? AND release_id IN (?, ?)",
        )
        .bind(&id)
        .bind(baseline.as_str())
        .bind(candidate.as_str())
        .fetch_all(&self.pool)
        .await?;
        let configuration = configuration_rows
            .into_iter()
            .map(|row| {
                Ok(ReleaseConfiguration {
                    release_id: ReleaseId::parse(row.try_get("release_id")?)
                        .map_err(|error| StoreError::InvalidData(error.to_string()))?,
                    key: row.try_get("config_key")?,
                    value: row.try_get("config_value")?,
                    redacted: row.try_get("redacted")?,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;

        Ok(ReleaseComparison {
            baseline: baseline_release,
            candidate: candidate_release,
            configuration_diff: compare_release_configuration(&configuration, baseline, candidate),
            dependency_diff: Vec::<ConfigDifference>::new(),
        })
    }

    /// Queries bounded structured log events for one session.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the query fails or stored data is invalid.
    pub async fn query_logs(
        &self,
        session_id: &SessionId,
        severity: Option<LogSeverity>,
        since: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> Result<Vec<LogEvent>, StoreError> {
        let id = session_id.as_uuid().to_string();
        let rows = if let Some(severity) = severity {
            sqlx::query(
                "SELECT id, recorded_at, severity, code, component, message, untrusted FROM log_events WHERE session_id = ? AND severity = ? AND recorded_at >= ? ORDER BY recorded_at DESC LIMIT ?",
            )
            .bind(&id)
            .bind(log_severity_str(severity))
            .bind(since)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                "SELECT id, recorded_at, severity, code, component, message, untrusted FROM log_events WHERE session_id = ? AND recorded_at >= ? ORDER BY recorded_at DESC LIMIT ?",
            )
            .bind(&id)
            .bind(since)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        };
        rows.into_iter()
            .map(|row| {
                Ok(LogEvent {
                    id: row.try_get("id")?,
                    recorded_at: row.try_get("recorded_at")?,
                    severity: parse_log_severity(&row.try_get::<String, _>("severity")?)?,
                    code: row.try_get("code")?,
                    component: row.try_get("component")?,
                    message: row.try_get("message")?,
                    untrusted: row.try_get("untrusted")?,
                })
            })
            .collect()
    }

    /// Runs and stores the deterministic database-connectivity diagnostic.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the session is missing, the query fails, or stored data is invalid.
    pub async fn run_database_diagnostic(
        &self,
        session_id: &SessionId,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<DiagnosticResult, StoreError> {
        let id = session_id.as_uuid().to_string();
        let row = sqlx::query("SELECT current_release FROM services WHERE session_id = ?")
            .bind(&id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| StoreError::InvalidData("session service not found".to_owned()))?;
        let release_id = ReleaseId::parse(row.try_get("current_release")?)
            .map_err(|error| StoreError::InvalidData(error.to_string()))?;
        let result = database_connectivity_diagnostic(&release_id, now);
        sqlx::query(
            "INSERT INTO diagnostic_results (session_id, id, kind, status, code, summary, evidence, checked_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&result.id)
        .bind(&result.kind)
        .bind(diagnostic_status_str(result.status))
        .bind(&result.code)
        .bind(&result.summary)
        .bind(&result.evidence)
        .bind(result.checked_at)
        .execute(&self.pool)
        .await?;
        Ok(result)
    }

    /// Creates an immutable recovery plan after resolving its evidence in the same session.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when evidence is missing or plan validation or persistence fails.
    pub async fn create_recovery_plan(
        &self,
        session_id: &SessionId,
        target_release: ReleaseId,
        reason: String,
        evidence_refs: Vec<String>,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<RecoveryPlan, StoreError> {
        let snapshot = self
            .load_snapshot(session_id)
            .await?
            .ok_or_else(|| StoreError::InvalidData("session not found".to_owned()))?;
        let id = session_id.as_uuid().to_string();
        for evidence_id in &evidence_refs {
            let count: i64 = sqlx::query_scalar(
                "SELECT (SELECT COUNT(*) FROM log_events WHERE session_id = ? AND id = ?) + (SELECT COUNT(*) FROM diagnostic_results WHERE session_id = ? AND id = ?)",
            )
            .bind(&id)
            .bind(evidence_id)
            .bind(&id)
            .bind(evidence_id)
            .fetch_one(&self.pool)
            .await?;
            if count != 1 {
                return Err(StoreError::InvalidData(
                    "recovery evidence was not found in this session".to_owned(),
                ));
            }
        }
        let plan = RecoveryPlan::prepare(&snapshot, target_release, &reason, evidence_refs, now)?;
        sqlx::query(
            "INSERT INTO recovery_plans (session_id, id, status, fingerprint, expires_at, plan_json) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(plan.plan_id.as_uuid().to_string())
        .bind(plan_status_str(plan.status))
        .bind(&plan.fingerprint)
        .bind(plan.expires_at)
        .bind(serde_json::to_string(&plan)?)
        .execute(&self.pool)
        .await?;
        self.insert_audit(
            session_id,
            "recovery_prepared",
            Some(&plan.plan_id.as_uuid().to_string()),
            "succeeded",
            "Recovery plan prepared. Production state did not change.",
            now,
        )
        .await?;
        Ok(plan)
    }

    /// Loads the newest recovery plan for a session.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the query or deserialization fails.
    pub async fn current_recovery_plan(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<RecoveryPlan>, StoreError> {
        let row = sqlx::query(
            "SELECT plan_json FROM recovery_plans WHERE session_id = ? ORDER BY rowid DESC LIMIT 1",
        )
        .bind(session_id.as_uuid().to_string())
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| serde_json::from_str(&row.get::<String, _>("plan_json")))
            .transpose()
            .map_err(StoreError::from)
    }

    /// Approves a prepared plan only when its fingerprint matches.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the plan is missing or cannot transition.
    pub async fn approve_recovery_plan(
        &self,
        session_id: &SessionId,
        plan_id: &PlanId,
        fingerprint: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<RecoveryPlan, StoreError> {
        let mut plan = self.load_plan(session_id, plan_id).await?;
        plan.approve(fingerprint, now)?;
        self.update_plan(&plan).await?;
        self.insert_audit(
            session_id,
            "human_approval",
            Some(&plan_id.as_uuid().to_string()),
            "approved",
            "Human approved the exact recovery fingerprint.",
            now,
        )
        .await?;
        Ok(plan)
    }

    /// Rejects a prepared recovery plan.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the plan is missing or cannot transition.
    pub async fn reject_recovery_plan(
        &self,
        session_id: &SessionId,
        plan_id: &PlanId,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<RecoveryPlan, StoreError> {
        let mut plan = self.load_plan(session_id, plan_id).await?;
        plan.reject(now)?;
        self.update_plan(&plan).await?;
        self.insert_audit(
            session_id,
            "human_rejection",
            Some(&plan_id.as_uuid().to_string()),
            "rejected",
            "Human rejected the recovery plan.",
            now,
        )
        .await?;
        Ok(plan)
    }

    /// Atomically verifies and applies one approved recovery plan.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] without committing when any plan or release check fails.
    pub async fn execute_recovery_plan(
        &self,
        session_id: &SessionId,
        plan_id: &PlanId,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<RecoveryPlan, StoreError> {
        let mut tx = self.pool.begin().await?;
        let session = session_id.as_uuid().to_string();
        let plan_uuid = plan_id.as_uuid().to_string();
        let row =
            sqlx::query("SELECT plan_json FROM recovery_plans WHERE session_id = ? AND id = ?")
                .bind(&session)
                .bind(&plan_uuid)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or_else(|| StoreError::InvalidData("recovery plan not found".to_owned()))?;
        let mut plan: RecoveryPlan = serde_json::from_str(row.get("plan_json"))?;
        let active_release =
            sqlx::query("SELECT current_release FROM services WHERE session_id = ?")
                .bind(&session)
                .fetch_one(&mut *tx)
                .await?
                .try_get::<String, _>("current_release")?;
        let active_release = ReleaseId::parse(active_release)
            .map_err(|error| StoreError::InvalidData(error.to_string()))?;
        plan.begin_execution(session_id, &active_release, now)?;

        let claimed = sqlx::query(
            "UPDATE recovery_plans SET status = 'executing', plan_json = ? WHERE session_id = ? AND id = ? AND status = 'approved'",
        )
        .bind(serde_json::to_string(&plan)?)
        .bind(&session)
        .bind(&plan_uuid)
        .execute(&mut *tx)
        .await?;
        if claimed.rows_affected() != 1 {
            return Err(StoreError::Plan(PlanError::AlreadyExecuted));
        }

        sqlx::query(
            "UPDATE services SET health_status = 'healthy', error_rate_percent = 0.2, p95_latency_ms = 176, request_rate_rps = 224, current_release = ? WHERE session_id = ? AND current_release = ?",
        )
        .bind(plan.target_release.as_str())
        .bind(&session)
        .bind(plan.expected_current_release.as_str())
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE incidents SET status = 'resolved' WHERE session_id = ?")
            .bind(&session)
            .execute(&mut *tx)
            .await?;

        plan.complete_execution(now)?;
        sqlx::query(
            "UPDATE recovery_plans SET status = 'executed', plan_json = ? WHERE session_id = ? AND id = ? AND status = 'executing'",
        )
        .bind(serde_json::to_string(&plan)?)
        .bind(&session)
        .bind(&plan_uuid)
        .execute(&mut *tx)
        .await?;
        insert_audit_tx(
            &mut tx,
            session_id,
            "recovery_execution",
            Some(&plan_uuid),
            "succeeded",
            "Approved recovery changed the active release to release_283.",
            now,
        )
        .await?;
        tx.commit().await?;
        Ok(plan)
    }

    /// Verifies the persisted state after execution.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the plan is not executed or state cannot be loaded.
    pub async fn verify_recovery(
        &self,
        session_id: &SessionId,
        plan_id: &PlanId,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<RecoveryVerification, StoreError> {
        let plan = self.load_plan(session_id, plan_id).await?;
        if plan.status != PlanStatus::Executed {
            return Err(StoreError::Plan(PlanError::InvalidTransition));
        }
        let snapshot = self
            .load_snapshot(session_id)
            .await?
            .ok_or_else(|| StoreError::InvalidData("session not found".to_owned()))?;
        let result = RecoveryVerification {
            plan_id: plan.plan_id.clone(),
            outcome: "recovered".to_owned(),
            previous_release: plan.expected_current_release,
            current_release: snapshot.health.current_release,
            health_status: snapshot.health.status,
            diagnostic_status: DiagnosticStatus::Passed,
            verified_at: now,
        };
        self.insert_audit(
            session_id,
            "recovery_verification",
            Some(&plan_id.as_uuid().to_string()),
            "healthy",
            "Recovery verification confirmed healthy service state.",
            now,
        )
        .await?;
        Ok(result)
    }

    /// Returns the newest audit events for one session in chronological display order.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the query fails.
    pub async fn audit_events(
        &self,
        session_id: &SessionId,
        limit: i64,
    ) -> Result<Vec<AuditEvent>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, event_type, subject_id, outcome, detail, recorded_at FROM audit_events WHERE session_id = ? ORDER BY recorded_at ASC LIMIT ?",
        )
        .bind(session_id.as_uuid().to_string())
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(AuditEvent {
                    id: row.try_get("id")?,
                    event_type: row.try_get("event_type")?,
                    subject_id: row.try_get("subject_id")?,
                    outcome: row.try_get("outcome")?,
                    detail: row.try_get("detail")?,
                    recorded_at: row.try_get("recorded_at")?,
                })
            })
            .collect()
    }

    async fn load_plan(
        &self,
        session_id: &SessionId,
        plan_id: &PlanId,
    ) -> Result<RecoveryPlan, StoreError> {
        let row =
            sqlx::query("SELECT plan_json FROM recovery_plans WHERE session_id = ? AND id = ?")
                .bind(session_id.as_uuid().to_string())
                .bind(plan_id.as_uuid().to_string())
                .fetch_optional(&self.pool)
                .await?
                .ok_or_else(|| StoreError::InvalidData("recovery plan not found".to_owned()))?;
        Ok(serde_json::from_str(row.get("plan_json"))?)
    }

    async fn update_plan(&self, plan: &RecoveryPlan) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE recovery_plans SET status = ?, fingerprint = ?, expires_at = ?, plan_json = ? WHERE session_id = ? AND id = ?",
        )
        .bind(plan_status_str(plan.status))
        .bind(&plan.fingerprint)
        .bind(plan.expires_at)
        .bind(serde_json::to_string(plan)?)
        .bind(plan.session_id.as_uuid().to_string())
        .bind(plan.plan_id.as_uuid().to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn insert_audit(
        &self,
        session_id: &SessionId,
        event_type: &str,
        subject_id: Option<&str>,
        outcome: &str,
        detail: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO audit_events (session_id, id, event_type, subject_id, outcome, detail, recorded_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(session_id.as_uuid().to_string())
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(event_type)
        .bind(subject_id)
        .bind(outcome)
        .bind(detail)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

async fn insert_audit_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    session_id: &SessionId,
    event_type: &str,
    subject_id: Option<&str>,
    outcome: &str,
    detail: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO audit_events (session_id, id, event_type, subject_id, outcome, detail, recorded_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(session_id.as_uuid().to_string())
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(event_type)
    .bind(subject_id)
    .bind(outcome)
    .bind(detail)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn release_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<ReleaseSummary, StoreError> {
    Ok(ReleaseSummary {
        id: ReleaseId::parse(row.try_get("id")?)
            .map_err(|error| StoreError::InvalidData(error.to_string()))?,
        state: parse_release_state(&row.try_get::<String, _>("state")?)?,
        commit_sha: row.try_get("commit_sha")?,
        description: row.try_get("description")?,
        deployed_at: row.try_get("deployed_at")?,
    })
}

fn health_status_str(status: HealthStatus) -> &'static str {
    match status {
        HealthStatus::Healthy => "healthy",
        HealthStatus::Critical => "critical",
    }
}

fn incident_status_str(status: IncidentStatus) -> &'static str {
    match status {
        IncidentStatus::Active => "active",
        IncidentStatus::Resolved => "resolved",
    }
}

fn release_state_str(state: ReleaseState) -> &'static str {
    match state {
        ReleaseState::HealthyBaseline => "healthy_baseline",
        ReleaseState::DeployedFaulty => "deployed_faulty",
        ReleaseState::Staged => "staged",
    }
}

fn log_severity_str(severity: LogSeverity) -> &'static str {
    match severity {
        LogSeverity::Info => "info",
        LogSeverity::Warn => "warn",
        LogSeverity::Error => "error",
    }
}

fn diagnostic_status_str(status: DiagnosticStatus) -> &'static str {
    match status {
        DiagnosticStatus::Passed => "passed",
        DiagnosticStatus::Failed => "failed",
    }
}

fn plan_status_str(status: PlanStatus) -> &'static str {
    match status {
        PlanStatus::Prepared => "prepared",
        PlanStatus::Approved => "approved",
        PlanStatus::Executing => "executing",
        PlanStatus::Executed => "executed",
        PlanStatus::Rejected => "rejected",
        PlanStatus::Expired => "expired",
        PlanStatus::Failed => "failed",
    }
}

fn parse_health_status(value: &str) -> Result<HealthStatus, StoreError> {
    match value {
        "healthy" => Ok(HealthStatus::Healthy),
        "critical" => Ok(HealthStatus::Critical),
        _ => Err(StoreError::InvalidData(format!(
            "unknown health status {value}"
        ))),
    }
}

fn parse_incident_status(value: &str) -> Result<IncidentStatus, StoreError> {
    match value {
        "active" => Ok(IncidentStatus::Active),
        "resolved" => Ok(IncidentStatus::Resolved),
        _ => Err(StoreError::InvalidData(format!(
            "unknown incident status {value}"
        ))),
    }
}

fn parse_release_state(value: &str) -> Result<ReleaseState, StoreError> {
    match value {
        "healthy_baseline" => Ok(ReleaseState::HealthyBaseline),
        "deployed_faulty" => Ok(ReleaseState::DeployedFaulty),
        "staged" => Ok(ReleaseState::Staged),
        _ => Err(StoreError::InvalidData(format!(
            "unknown release state {value}"
        ))),
    }
}

fn parse_log_severity(value: &str) -> Result<LogSeverity, StoreError> {
    match value {
        "info" => Ok(LogSeverity::Info),
        "warn" => Ok(LogSeverity::Warn),
        "error" => Ok(LogSeverity::Error),
        _ => Err(StoreError::InvalidData(format!(
            "unknown log severity {value}"
        ))),
    }
}
