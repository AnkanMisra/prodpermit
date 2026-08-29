use recovery_domain::{IncidentSnapshot, seeded_investigation_data};
use sqlx::{Sqlite, Transaction};

use crate::{
    Store, StoreError, health_status_str, incident_status_str, log_severity_str, release_state_str,
};

const MAX_ACTIVE_SESSIONS: i64 = 256;

impl Store {
    /// Inserts one complete isolated scenario in a transaction.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when any insert or the transaction commit fails.
    pub async fn create_session(&self, snapshot: &IncidentSnapshot) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM demo_sessions WHERE revoked_at IS NOT NULL OR expires_at <= ?")
            .bind(snapshot.session.created_at)
            .execute(&mut *tx)
            .await?;
        let active_sessions: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM demo_sessions WHERE revoked_at IS NULL")
                .fetch_one(&mut *tx)
                .await?;
        if active_sessions >= MAX_ACTIVE_SESSIONS {
            return Err(StoreError::SessionCapacity);
        }
        insert_session(&mut tx, snapshot).await?;
        tx.commit().await?;
        Ok(())
    }
}

pub(crate) async fn insert_session(
    tx: &mut Transaction<'_, Sqlite>,
    snapshot: &IncidentSnapshot,
) -> Result<(), StoreError> {
    let session_id = snapshot.session.id.as_uuid().to_string();

    sqlx::query(
        "INSERT INTO demo_sessions (id, created_at, expires_at, generation, revoked_at) VALUES (?, ?, ?, ?, NULL)",
    )
    .bind(&session_id)
    .bind(snapshot.session.created_at)
    .bind(snapshot.session.expires_at)
    .bind(snapshot.session.generation)
    .execute(&mut **tx)
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
    .execute(&mut **tx)
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
    .execute(&mut **tx)
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
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            "INSERT INTO service_releases (session_id, service_id, release_id) VALUES (?, ?, ?)",
        )
        .bind(&session_id)
        .bind(snapshot.incident.service_id.as_str())
        .bind(release.id.as_str())
        .execute(&mut **tx)
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
        .execute(&mut **tx)
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
        .execute(&mut **tx)
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
        .execute(&mut **tx)
        .await?;
    }

    Ok(())
}
