//! `SQLite` persistence for session-isolated incident scenarios.

mod recovery;
mod session;

pub use recovery::{PersistedRecoveryPlan, RecoveryPreparation};

use std::str::FromStr;

use recovery_domain::{
    AuditEvent, ConfigDifference, DemoSession, DiagnosticResult, DiagnosticStatus, HealthStatus,
    Incident, IncidentId, IncidentSnapshot, IncidentStatus, LogEvent, LogSeverity, RecoveryError,
    ReleaseComparison, ReleaseConfiguration, ReleaseId, ReleaseState, ReleaseSummary,
    ServiceHealth, ServiceId, SessionId, TelemetryPoint, compare_release_configuration,
    database_connectivity_diagnostic,
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
    #[error("recovery operation rejected: {0}")]
    Recovery(#[from] RecoveryError),
    #[error("demo session not found")]
    SessionNotFound,
    #[error("demo session is expired or revoked")]
    SessionInactive,
    #[error("recovery plan not found")]
    RecoveryNotFound,
    #[error("recovery evidence is unknown, ambiguous, or belongs to another session")]
    InvalidRecoveryEvidence,
    #[error("one active recovery plan already exists")]
    ActiveRecoveryExists,
    #[error("the demo session capacity is temporarily full")]
    SessionCapacity,
}

#[derive(Clone, Debug)]
pub struct Store {
    pub(crate) pool: SqlitePool,
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

    /// Reports whether a cookie-bound session is present, unexpired, and not revoked.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the lookup fails.
    pub async fn session_is_active(
        &self,
        session_id: &SessionId,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool, StoreError> {
        let active: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM demo_sessions WHERE id = ? AND revoked_at IS NULL AND expires_at > ?",
        )
        .bind(session_id.as_uuid().to_string())
        .bind(now)
        .fetch_one(&self.pool)
        .await?;
        Ok(active == 1)
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
            "SELECT created_at, expires_at, generation FROM demo_sessions WHERE id = ? AND revoked_at IS NULL",
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
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT services.id AS service_id, services.current_release, demo_sessions.generation, demo_sessions.expires_at, demo_sessions.revoked_at FROM services JOIN demo_sessions ON demo_sessions.id = services.session_id WHERE services.session_id = ?",
        )
            .bind(&id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(StoreError::SessionNotFound)?;
        let expires_at: chrono::DateTime<chrono::Utc> = row.try_get("expires_at")?;
        let revoked_at: Option<chrono::DateTime<chrono::Utc>> = row.try_get("revoked_at")?;
        if revoked_at.is_some() || now >= expires_at {
            return Err(StoreError::SessionInactive);
        }
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
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO diagnostic_contexts (session_id, diagnostic_id, service_id, release_id, scenario_generation, plan_id) VALUES (?, ?, ?, ?, ?, NULL)",
        )
        .bind(&id)
        .bind(&result.id)
        .bind(row.try_get::<String, _>("service_id")?)
        .bind(release_id.as_str())
        .bind(row.try_get::<i64, _>("generation")?)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
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
