use chrono::{DateTime, Utc};
use recovery_domain::{
    DiagnosticStatus, EvidenceId, HumanDecision, IncidentId, IncidentStatus, PlanId,
    PrepareRecoveryCommand, RecoveryDiagnosticEvidence, RecoveryError, RecoveryEvidence,
    RecoveryEvidenceKind, RecoveryExecutionFacts, RecoveryFingerprint, RecoveryInvalidationReason,
    RecoveryPlanSpec, RecoveryPlanState, RecoveryTelemetryEvidence, RecoveryVerification,
    RecoveryVerificationFacts, RecoveryVerificationOutcome, ReleaseId, ServiceId, SessionId,
    apply_human_decision, canonical_recovery_fingerprint, complete_execution,
    database_connectivity_diagnostic, derive_verification, expire_recovery, prepare_recovery,
    seeded_scenario, validate_execution,
};
use serde::Serialize;
use sqlx::{Row, Sqlite, Transaction};
use uuid::Uuid;

use crate::{
    Store, StoreError, diagnostic_status_str, parse_health_status, parse_incident_status,
    parse_release_state, session::insert_session,
};

#[derive(Clone, Debug)]
pub struct RecoveryPreparation {
    pub target_release: ReleaseId,
    pub reason: String,
    pub evidence_refs: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedRecoveryPlan {
    pub spec: RecoveryPlanSpec,
    pub fingerprint: RecoveryFingerprint,
    pub state: RecoveryPlanState,
}

impl Store {
    /// Resolves evidence and stores one immutable plan with its audit event.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when validation or the transaction fails.
    #[allow(clippy::too_many_lines)]
    pub async fn prepare_recovery(
        &self,
        session_id: &SessionId,
        preparation: RecoveryPreparation,
        now: DateTime<Utc>,
    ) -> Result<PersistedRecoveryPlan, StoreError> {
        let mut tx = self.pool.begin().await?;
        lock_active_session(&mut tx, session_id, now).await?;
        expire_active_plan(&mut tx, session_id, now).await?;

        let row = sqlx::query(
            "SELECT demo_sessions.generation, incidents.id AS incident_id, incidents.service_id, incidents.status AS incident_status, services.current_release, releases.state AS target_state, service_releases.service_id AS target_service_id FROM demo_sessions JOIN incidents ON incidents.session_id = demo_sessions.id JOIN services ON services.session_id = demo_sessions.id AND services.id = incidents.service_id JOIN service_releases ON service_releases.session_id = demo_sessions.id AND service_releases.service_id = services.id AND service_releases.release_id = ? JOIN releases ON releases.session_id = service_releases.session_id AND releases.id = service_releases.release_id WHERE demo_sessions.id = ?",
        )
        .bind(preparation.target_release.as_str())
        .bind(session_text(session_id))
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| StoreError::Recovery(RecoveryError::InvalidTarget))?;
        if parse_incident_status(&row.try_get::<String, _>("incident_status")?)?
            != IncidentStatus::Active
        {
            return Err(StoreError::Recovery(RecoveryError::InvalidTransition));
        }

        let generation = row.try_get::<i64, _>("generation")?;
        let service_id = parse_service(row.try_get("service_id")?)?;
        let expected_release = parse_release(row.try_get("current_release")?)?;
        let evidence = resolve_evidence(
            &mut tx,
            session_id,
            &service_id,
            &expected_release,
            generation,
            &preparation.evidence_refs,
        )
        .await?;
        let active: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM recovery_plans WHERE session_id = ? AND status IN ('prepared', 'approved', 'executing')",
        )
        .bind(session_text(session_id))
        .fetch_one(&mut *tx)
        .await?;
        if active != 0 {
            return Err(StoreError::ActiveRecoveryExists);
        }
        let spec = prepare_recovery(PrepareRecoveryCommand {
            plan_id: PlanId::new(),
            session_id: session_id.clone(),
            incident_id: parse_incident(row.try_get("incident_id")?)?,
            service_id,
            scenario_generation: generation,
            expected_current_release: expected_release,
            target_release: preparation.target_release,
            target_service_id: parse_service(row.try_get("target_service_id")?)?,
            target_release_state: parse_release_state(&row.try_get::<String, _>("target_state")?)?,
            reason: preparation.reason,
            evidence,
            created_at: now,
        })?;
        let fingerprint = canonical_recovery_fingerprint(&spec);

        let inserted = sqlx::query(
            "INSERT INTO recovery_plans (session_id, id, incident_id, service_id, scenario_generation, expected_current_release, target_release, reason, policy_version, created_at, expires_at, fingerprint, status) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'prepared')",
        )
        .bind(session_text(session_id))
        .bind(spec.plan_id().as_uuid().to_string())
        .bind(spec.incident_id().as_str())
        .bind(spec.service_id().as_str())
        .bind(spec.scenario_generation())
        .bind(spec.expected_current_release().as_str())
        .bind(spec.target_release().as_str())
        .bind(spec.reason())
        .bind(spec.policy_version())
        .bind(spec.created_at())
        .bind(spec.expires_at())
        .bind(fingerprint.as_str())
        .execute(&mut *tx)
        .await;
        if let Err(error) = inserted {
            if error.to_string().contains("recovery_plans.session_id") {
                return Err(StoreError::ActiveRecoveryExists);
            }
            return Err(error.into());
        }

        for (ordinal, item) in spec.evidence().iter().enumerate() {
            let (kind, log_id, diagnostic_id) = match item.kind() {
                RecoveryEvidenceKind::DatabaseAuthenticationFailureLog => (
                    "database_authentication_failure_log",
                    Some(item.id().as_str()),
                    None,
                ),
                RecoveryEvidenceKind::FailedDatabaseConnectivityDiagnostic => (
                    "failed_database_connectivity_diagnostic",
                    None,
                    Some(item.id().as_str()),
                ),
            };
            sqlx::query(
                "INSERT INTO recovery_plan_evidence (session_id, plan_id, ordinal, kind, evidence_id, log_event_id, diagnostic_id) VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(session_text(session_id))
            .bind(spec.plan_id().as_uuid().to_string())
            .bind(i64::try_from(ordinal).map_err(|error| StoreError::InvalidData(error.to_string()))?)
            .bind(kind)
            .bind(item.id().as_str())
            .bind(log_id)
            .bind(diagnostic_id)
            .execute(&mut *tx)
            .await?;
        }
        insert_audit(
            &mut tx,
            session_id,
            "recovery_prepared",
            Some(spec.plan_id().as_uuid().to_string()),
            "succeeded",
            "Recovery plan prepared. Production state did not change.",
            now,
            Some(format!("plan:{}:prepared", spec.plan_id().as_uuid())),
        )
        .await?;
        tx.commit().await?;
        Ok(PersistedRecoveryPlan {
            spec,
            fingerprint,
            state: RecoveryPlanState::Prepared,
        })
    }

    /// Returns the newest plan after durably applying deadline expiry.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the session is inactive or persistence is invalid.
    pub async fn current_recovery(
        &self,
        session_id: &SessionId,
        now: DateTime<Utc>,
    ) -> Result<Option<PersistedRecoveryPlan>, StoreError> {
        let mut tx = self.pool.begin().await?;
        lock_active_session(&mut tx, session_id, now).await?;
        expire_active_plan(&mut tx, session_id, now).await?;
        let plan_id = sqlx::query_scalar::<_, String>(
            "SELECT id FROM recovery_plans WHERE session_id = ? ORDER BY created_at DESC, id DESC LIMIT 1",
        )
        .bind(session_text(session_id))
        .fetch_optional(&mut *tx)
        .await?;
        let plan = match plan_id {
            Some(value) => Some(load_plan(&mut tx, session_id, &parse_plan(value)?).await?),
            None => None,
        };
        tx.commit().await?;
        Ok(plan)
    }

    /// Applies one exact human approval or rejection with its audit event.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when validation or the transaction fails.
    pub async fn decide_recovery(
        &self,
        session_id: &SessionId,
        plan_id: &PlanId,
        decision: HumanDecision,
        now: DateTime<Utc>,
    ) -> Result<PersistedRecoveryPlan, StoreError> {
        let mut tx = self.pool.begin().await?;
        lock_active_session(&mut tx, session_id, now).await?;
        let mut plan = load_plan(&mut tx, session_id, plan_id).await?;
        if now >= plan.spec.expires_at()
            && matches!(
                plan.state,
                RecoveryPlanState::Prepared | RecoveryPlanState::Approved { .. }
            )
        {
            plan.state = RecoveryPlanState::Expired;
            write_state(&mut tx, &plan).await?;
            insert_audit(
                &mut tx,
                session_id,
                "recovery_expired",
                Some(plan_id.as_uuid().to_string()),
                "expired",
                "Recovery approval window expired.",
                now,
                Some(format!("plan:{}:expired", plan_id.as_uuid())),
            )
            .await?;
            tx.commit().await?;
            return Err(StoreError::Recovery(RecoveryError::Expired));
        }
        let next = apply_human_decision(&plan.spec, plan.state.clone(), decision, now)?;
        if next != plan.state {
            let (event, outcome, detail) = match next {
                RecoveryPlanState::Approved { .. } => (
                    "human_approval",
                    "approved",
                    "Human approved the exact recovery fingerprint.",
                ),
                RecoveryPlanState::Rejected { .. } => (
                    "human_rejection",
                    "rejected",
                    "Human rejected the recovery plan.",
                ),
                _ => return Err(StoreError::Recovery(RecoveryError::InvalidTransition)),
            };
            plan.state = next;
            write_state(&mut tx, &plan).await?;
            insert_audit(
                &mut tx,
                session_id,
                event,
                Some(plan_id.as_uuid().to_string()),
                outcome,
                detail,
                now,
                Some(format!("plan:{}:{event}", plan_id.as_uuid())),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(plan)
    }

    /// Revalidates and atomically executes one approved recovery.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when any authority fact or transaction fails.
    #[allow(clippy::too_many_lines)]
    pub async fn execute_recovery(
        &self,
        session_id: &SessionId,
        plan_id: &PlanId,
        now: DateTime<Utc>,
    ) -> Result<PersistedRecoveryPlan, StoreError> {
        let mut tx = self.pool.begin().await?;
        lock_active_session(&mut tx, session_id, now).await?;
        let mut plan = load_plan(&mut tx, session_id, plan_id).await?;
        if now >= plan.spec.expires_at() && matches!(plan.state, RecoveryPlanState::Approved { .. })
        {
            plan.state = RecoveryPlanState::Expired;
            write_state(&mut tx, &plan).await?;
            insert_audit(
                &mut tx,
                session_id,
                "recovery_expired",
                Some(plan_id.as_uuid().to_string()),
                "expired",
                "Recovery approval window expired.",
                now,
                Some(format!("plan:{}:expired", plan_id.as_uuid())),
            )
            .await?;
            tx.commit().await?;
            return Err(StoreError::Recovery(RecoveryError::Expired));
        }
        let facts_row = sqlx::query(
            "SELECT demo_sessions.generation, incidents.id AS incident_id, incidents.status AS incident_status, services.current_release, releases.state AS target_state, service_releases.service_id AS target_service_id, recovery_plans.fingerprint FROM demo_sessions JOIN incidents ON incidents.session_id = demo_sessions.id JOIN services ON services.session_id = demo_sessions.id AND services.id = incidents.service_id JOIN recovery_plans ON recovery_plans.session_id = demo_sessions.id AND recovery_plans.id = ? JOIN service_releases ON service_releases.session_id = demo_sessions.id AND service_releases.service_id = services.id AND service_releases.release_id = recovery_plans.target_release JOIN releases ON releases.session_id = service_releases.session_id AND releases.id = service_releases.release_id WHERE demo_sessions.id = ?",
        )
        .bind(plan_id.as_uuid().to_string())
        .bind(session_text(session_id))
        .fetch_one(&mut *tx)
        .await?;
        let executing = validate_execution(
            &plan.spec,
            plan.state.clone(),
            &RecoveryExecutionFacts {
                session_id: session_id.clone(),
                scenario_generation: facts_row.try_get("generation")?,
                active_incident_id: parse_incident(facts_row.try_get("incident_id")?)?,
                active_incident_status: parse_incident_status(
                    &facts_row.try_get::<String, _>("incident_status")?,
                )?,
                active_release: parse_release(facts_row.try_get("current_release")?)?,
                target_release: plan.spec.target_release().clone(),
                target_service_id: parse_service(facts_row.try_get("target_service_id")?)?,
                target_release_state: parse_release_state(
                    &facts_row.try_get::<String, _>("target_state")?,
                )?,
                stored_fingerprint: facts_row.try_get("fingerprint")?,
            },
            now,
        )?;
        plan.state = executing;
        let claimed = write_state_conditionally(&mut tx, &plan, "approved").await?;
        if claimed != 1 {
            let latest = load_plan(&mut tx, session_id, plan_id).await?;
            return if matches!(latest.state, RecoveryPlanState::Executed { .. }) {
                Err(StoreError::Recovery(RecoveryError::AlreadyExecuted))
            } else {
                Err(StoreError::Recovery(RecoveryError::InvalidTransition))
            };
        }

        let service_changed = sqlx::query(
            "UPDATE services SET health_status = 'healthy', error_rate_percent = 0.2, p95_latency_ms = 176, request_rate_rps = 224, current_release = ? WHERE session_id = ? AND id = ? AND current_release = ?",
        )
        .bind(plan.spec.target_release().as_str())
        .bind(session_text(session_id))
        .bind(plan.spec.service_id().as_str())
        .bind(plan.spec.expected_current_release().as_str())
        .execute(&mut *tx)
        .await?;
        if service_changed.rows_affected() != 1 {
            return Err(StoreError::Recovery(RecoveryError::StaleActiveRelease));
        }
        let incident_changed = sqlx::query(
            "UPDATE incidents SET status = 'resolved' WHERE session_id = ? AND id = ? AND status = 'active'",
        )
        .bind(session_text(session_id))
        .bind(plan.spec.incident_id().as_str())
        .execute(&mut *tx)
        .await?;
        if incident_changed.rows_affected() != 1 {
            return Err(StoreError::Recovery(RecoveryError::IncidentNotActive));
        }
        sqlx::query(
            "INSERT INTO telemetry_points (session_id, service_id, recorded_at, error_rate_percent, p95_latency_ms, request_rate_rps) VALUES (?, ?, ?, 0.2, 176, 224)",
        )
        .bind(session_text(session_id))
        .bind(plan.spec.service_id().as_str())
        .bind(now)
        .execute(&mut *tx)
        .await?;
        let diagnostic = database_connectivity_diagnostic(plan.spec.target_release(), now);
        sqlx::query(
            "INSERT INTO diagnostic_results (session_id, id, kind, status, code, summary, evidence, checked_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(session_text(session_id))
        .bind(&diagnostic.id)
        .bind(&diagnostic.kind)
        .bind(diagnostic_status_str(diagnostic.status))
        .bind(&diagnostic.code)
        .bind(&diagnostic.summary)
        .bind(&diagnostic.evidence)
        .bind(diagnostic.checked_at)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO diagnostic_contexts (session_id, diagnostic_id, service_id, release_id, scenario_generation, plan_id) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(session_text(session_id))
        .bind(&diagnostic.id)
        .bind(plan.spec.service_id().as_str())
        .bind(plan.spec.target_release().as_str())
        .bind(plan.spec.scenario_generation())
        .bind(plan_id.as_uuid().to_string())
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO recovery_plan_executions (session_id, plan_id, service_id, scenario_generation, previous_release, current_release, telemetry_recorded_at, diagnostic_id, executed_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(session_text(session_id))
        .bind(plan_id.as_uuid().to_string())
        .bind(plan.spec.service_id().as_str())
        .bind(plan.spec.scenario_generation())
        .bind(plan.spec.expected_current_release().as_str())
        .bind(plan.spec.target_release().as_str())
        .bind(now)
        .bind(&diagnostic.id)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        plan.state = complete_execution(plan.state, now)?;
        if write_state_conditionally(&mut tx, &plan, "executing").await? != 1 {
            return Err(StoreError::Recovery(RecoveryError::InvalidTransition));
        }
        insert_audit(
            &mut tx,
            session_id,
            "recovery_execution",
            Some(plan_id.as_uuid().to_string()),
            "succeeded",
            "Approved recovery changed the active release to release_283.",
            now,
            Some(format!("plan:{}:executed", plan_id.as_uuid())),
        )
        .await?;
        tx.commit().await?;
        Ok(plan)
    }

    /// Derives verification from linked persisted before-and-after facts.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the plan has not executed or persistence is invalid.
    pub async fn verify_recovery(
        &self,
        session_id: &SessionId,
        plan_id: &PlanId,
        now: DateTime<Utc>,
    ) -> Result<RecoveryVerification, StoreError> {
        let mut tx = self.pool.begin().await?;
        lock_active_session(&mut tx, session_id, now).await?;
        let plan = load_plan(&mut tx, session_id, plan_id).await?;
        let row = sqlx::query(
            "SELECT e.plan_id, e.scenario_generation, e.previous_release, e.current_release, e.service_id, e.telemetry_recorded_at, e.diagnostic_id, s.health_status, i.id AS incident_id, i.status AS incident_status, t.error_rate_percent, t.p95_latency_ms, t.request_rate_rps, d.kind, d.status AS diagnostic_status, d.code, d.summary, d.evidence, d.checked_at, dc.release_id AS diagnostic_release, dc.scenario_generation AS diagnostic_generation, dc.plan_id AS diagnostic_plan_id, rp.fingerprint FROM recovery_plan_executions e JOIN services s ON s.session_id = e.session_id AND s.id = e.service_id JOIN incidents i ON i.session_id = e.session_id AND i.service_id = e.service_id JOIN telemetry_points t ON t.session_id = e.session_id AND t.service_id = e.service_id AND t.recorded_at = e.telemetry_recorded_at JOIN diagnostic_results d ON d.session_id = e.session_id AND d.id = e.diagnostic_id JOIN diagnostic_contexts dc ON dc.session_id = e.session_id AND dc.diagnostic_id = e.diagnostic_id JOIN recovery_plans rp ON rp.session_id = e.session_id AND rp.id = e.plan_id WHERE e.session_id = ? AND e.plan_id = ?",
        )
        .bind(session_text(session_id))
        .bind(plan_id.as_uuid().to_string())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| StoreError::Recovery(RecoveryError::NotExecuted))?;
        let telemetry = RecoveryTelemetryEvidence {
            plan_id: parse_plan(row.try_get("plan_id")?)?,
            service_id: parse_service(row.try_get("service_id")?)?,
            release_id: parse_release(row.try_get("current_release")?)?,
            scenario_generation: row.try_get("scenario_generation")?,
            recorded_at: row.try_get("telemetry_recorded_at")?,
            error_rate_percent: row.try_get("error_rate_percent")?,
            p95_latency_ms: row.try_get("p95_latency_ms")?,
            request_rate_rps: row.try_get("request_rate_rps")?,
        };
        let diagnostic = RecoveryDiagnosticEvidence {
            plan_id: parse_plan(
                row.try_get::<Option<String>, _>("diagnostic_plan_id")?
                    .ok_or_else(|| {
                        StoreError::InvalidData("execution diagnostic has no plan".to_owned())
                    })?,
            )?,
            id: EvidenceId::parse(row.try_get("diagnostic_id")?)?,
            service_id: parse_service(row.try_get("service_id")?)?,
            release_id: parse_release(row.try_get("diagnostic_release")?)?,
            scenario_generation: row.try_get("diagnostic_generation")?,
            kind: row.try_get("kind")?,
            status: parse_diagnostic_status(&row.try_get::<String, _>("diagnostic_status")?)?,
            code: row.try_get("code")?,
            summary: row.try_get("summary")?,
            evidence: row.try_get("evidence")?,
            checked_at: row.try_get("checked_at")?,
        };
        let result = derive_verification(
            &plan.spec,
            &plan.state,
            RecoveryVerificationFacts {
                execution_plan_id: parse_plan(row.try_get("plan_id")?)?,
                scenario_generation: row.try_get("scenario_generation")?,
                previous_release: parse_release(row.try_get("previous_release")?)?,
                current_release: parse_release(row.try_get("current_release")?)?,
                service_id: parse_service(row.try_get("service_id")?)?,
                health_status: parse_health_status(&row.try_get::<String, _>("health_status")?)?,
                incident_id: parse_incident(row.try_get("incident_id")?)?,
                incident_status: parse_incident_status(
                    &row.try_get::<String, _>("incident_status")?,
                )?,
                telemetry,
                diagnostic,
                stored_fingerprint: row.try_get("fingerprint")?,
            },
            now,
        )?;
        let outcome = match result.outcome {
            RecoveryVerificationOutcome::Passed => "healthy",
            RecoveryVerificationOutcome::Mismatch { .. } => "mismatch",
        };
        insert_audit(
            &mut tx,
            session_id,
            "recovery_verification",
            Some(plan_id.as_uuid().to_string()),
            outcome,
            "Recovery verification read persisted before and after evidence.",
            now,
            Some(format!("plan:{}:verification:{outcome}", plan_id.as_uuid())),
        )
        .await?;
        tx.commit().await?;
        Ok(result)
    }

    /// Revokes a session and returns its single retry-safe replacement.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the session is inactive or the transaction fails.
    pub async fn reset_session(
        &self,
        original_session_id: &SessionId,
        now: DateTime<Utc>,
    ) -> Result<recovery_domain::IncidentSnapshot, StoreError> {
        let mut tx = self.pool.begin().await?;
        lock_active_session(&mut tx, original_session_id, now).await?;
        let replacement = seeded_scenario(SessionId::new(), now);
        insert_session(&mut tx, &replacement).await?;
        sqlx::query(
            "UPDATE recovery_plans SET status = 'invalidated', approved_at = NULL, approved_fingerprint = NULL, execution_started_at = NULL, executed_at = NULL, rejected_at = NULL, invalidation_reason = 'session_reset', invalidated_at = ? WHERE session_id = ? AND status IN ('prepared', 'approved', 'executing')",
        )
        .bind(now)
        .bind(session_text(original_session_id))
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO session_resets (original_session_id, replacement_session_id, reset_at) VALUES (?, ?, ?)",
        )
        .bind(session_text(original_session_id))
        .bind(session_text(&replacement.session.id))
        .bind(now)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE demo_sessions SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL")
            .bind(now)
            .bind(session_text(original_session_id))
            .execute(&mut *tx)
            .await?;
        insert_audit(
            &mut tx,
            original_session_id,
            "session_reset",
            Some(replacement.session.id.as_uuid().to_string()),
            "succeeded",
            "The session was revoked and replaced with a broken scenario.",
            now,
            Some(format!("session:{}:reset", original_session_id.as_uuid())),
        )
        .await?;
        tx.commit().await?;
        self.load_snapshot(&replacement.session.id)
            .await?
            .ok_or(StoreError::SessionNotFound)
    }
}

async fn lock_active_session(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: &SessionId,
    now: DateTime<Utc>,
) -> Result<i64, StoreError> {
    sqlx::query("UPDATE demo_sessions SET revoked_at = revoked_at WHERE id = ?")
        .bind(session_text(session_id))
        .execute(&mut **tx)
        .await?;
    let row =
        sqlx::query("SELECT generation, expires_at, revoked_at FROM demo_sessions WHERE id = ?")
            .bind(session_text(session_id))
            .fetch_optional(&mut **tx)
            .await?
            .ok_or(StoreError::SessionNotFound)?;
    let expires_at: DateTime<Utc> = row.try_get("expires_at")?;
    let revoked_at: Option<DateTime<Utc>> = row.try_get("revoked_at")?;
    if revoked_at.is_some() || now >= expires_at {
        return Err(StoreError::SessionInactive);
    }
    Ok(row.try_get("generation")?)
}

async fn resolve_evidence(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: &SessionId,
    service_id: &ServiceId,
    release_id: &ReleaseId,
    generation: i64,
    references: &[String],
) -> Result<Vec<RecoveryEvidence>, StoreError> {
    let mut resolved = Vec::with_capacity(references.len());
    for reference in references {
        let rows = sqlx::query(
            "SELECT 'log' AS source, id, code, NULL AS kind, NULL AS status, NULL AS context_service, NULL AS context_release, NULL AS context_generation FROM log_events WHERE session_id = ? AND id = ? UNION ALL SELECT 'diagnostic' AS source, d.id, d.code, d.kind, d.status, dc.service_id, dc.release_id, dc.scenario_generation FROM diagnostic_results d JOIN diagnostic_contexts dc ON dc.session_id = d.session_id AND dc.diagnostic_id = d.id WHERE d.session_id = ? AND d.id = ?",
        )
        .bind(session_text(session_id))
        .bind(reference)
        .bind(session_text(session_id))
        .bind(reference)
        .fetch_all(&mut **tx)
        .await?;
        if rows.len() != 1 {
            return Err(StoreError::InvalidRecoveryEvidence);
        }
        let row = &rows[0];
        let id = EvidenceId::parse(row.try_get("id")?)?;
        let item = match row.try_get::<String, _>("source")?.as_str() {
            "log" => RecoveryEvidence::Log {
                id,
                session_id: session_id.clone(),
                service_id: service_id.clone(),
                release_id: release_id.clone(),
                scenario_generation: generation,
                code: row.try_get("code")?,
            },
            "diagnostic" => RecoveryEvidence::Diagnostic {
                id,
                session_id: session_id.clone(),
                service_id: parse_service(row.try_get("context_service")?)?,
                release_id: parse_release(row.try_get("context_release")?)?,
                scenario_generation: row.try_get("context_generation")?,
                kind: row.try_get("kind")?,
                status: parse_diagnostic_status(&row.try_get::<String, _>("status")?)?,
                code: row.try_get("code")?,
            },
            _ => return Err(StoreError::InvalidRecoveryEvidence),
        };
        resolved.push(item);
    }
    Ok(resolved)
}

async fn load_plan(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: &SessionId,
    plan_id: &PlanId,
) -> Result<PersistedRecoveryPlan, StoreError> {
    let row = sqlx::query(
        "SELECT rp.*, r.state AS target_state FROM recovery_plans rp JOIN releases r ON r.session_id = rp.session_id AND r.id = rp.target_release WHERE rp.session_id = ? AND rp.id = ?",
    )
    .bind(session_text(session_id))
    .bind(plan_id.as_uuid().to_string())
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(StoreError::RecoveryNotFound)?;
    let service_id = parse_service(row.try_get("service_id")?)?;
    let expected_release = parse_release(row.try_get("expected_current_release")?)?;
    let generation = row.try_get("scenario_generation")?;
    let evidence_rows = sqlx::query(
        "SELECT pe.kind, pe.evidence_id, l.code AS log_code, d.kind AS diagnostic_kind, d.status AS diagnostic_status, d.code AS diagnostic_code, dc.service_id AS diagnostic_service, dc.release_id AS diagnostic_release, dc.scenario_generation AS diagnostic_generation FROM recovery_plan_evidence pe LEFT JOIN log_events l ON l.session_id = pe.session_id AND l.id = pe.log_event_id LEFT JOIN diagnostic_results d ON d.session_id = pe.session_id AND d.id = pe.diagnostic_id LEFT JOIN diagnostic_contexts dc ON dc.session_id = pe.session_id AND dc.diagnostic_id = pe.diagnostic_id WHERE pe.session_id = ? AND pe.plan_id = ? ORDER BY pe.ordinal",
    )
    .bind(session_text(session_id))
    .bind(plan_id.as_uuid().to_string())
    .fetch_all(&mut **tx)
    .await?;
    let mut evidence = Vec::with_capacity(evidence_rows.len());
    for evidence_row in evidence_rows {
        let id = EvidenceId::parse(evidence_row.try_get("evidence_id")?)?;
        match evidence_row.try_get::<String, _>("kind")?.as_str() {
            "database_authentication_failure_log" => evidence.push(RecoveryEvidence::Log {
                id,
                session_id: session_id.clone(),
                service_id: service_id.clone(),
                release_id: expected_release.clone(),
                scenario_generation: generation,
                code: evidence_row.try_get("log_code")?,
            }),
            "failed_database_connectivity_diagnostic" => {
                evidence.push(RecoveryEvidence::Diagnostic {
                    id,
                    session_id: session_id.clone(),
                    service_id: parse_service(evidence_row.try_get("diagnostic_service")?)?,
                    release_id: parse_release(evidence_row.try_get("diagnostic_release")?)?,
                    scenario_generation: evidence_row.try_get("diagnostic_generation")?,
                    kind: evidence_row.try_get("diagnostic_kind")?,
                    status: parse_diagnostic_status(
                        &evidence_row.try_get::<String, _>("diagnostic_status")?,
                    )?,
                    code: evidence_row.try_get("diagnostic_code")?,
                });
            }
            _ => return Err(StoreError::InvalidData("unknown evidence kind".to_owned())),
        }
    }
    let spec = prepare_recovery(PrepareRecoveryCommand {
        plan_id: plan_id.clone(),
        session_id: session_id.clone(),
        incident_id: parse_incident(row.try_get("incident_id")?)?,
        service_id: service_id.clone(),
        scenario_generation: generation,
        expected_current_release: expected_release,
        target_release: parse_release(row.try_get("target_release")?)?,
        target_service_id: service_id,
        target_release_state: parse_release_state(&row.try_get::<String, _>("target_state")?)?,
        reason: row.try_get("reason")?,
        evidence,
        created_at: row.try_get("created_at")?,
    })?;
    let stored_expiry: DateTime<Utc> = row.try_get("expires_at")?;
    if stored_expiry != spec.expires_at() {
        return Err(StoreError::InvalidData(
            "stored expiry is not canonical".to_owned(),
        ));
    }
    Ok(PersistedRecoveryPlan {
        fingerprint: RecoveryFingerprint::parse(row.try_get("fingerprint")?)?,
        state: parse_state(&row)?,
        spec,
    })
}

fn parse_state(row: &sqlx::sqlite::SqliteRow) -> Result<RecoveryPlanState, StoreError> {
    let approved = || -> Result<_, StoreError> {
        Ok((
            row.try_get::<DateTime<Utc>, _>("approved_at")?,
            RecoveryFingerprint::parse(row.try_get("approved_fingerprint")?)?,
        ))
    };
    match row.try_get::<String, _>("status")?.as_str() {
        "prepared" => Ok(RecoveryPlanState::Prepared),
        "approved" => {
            let (approved_at, approved_fingerprint) = approved()?;
            Ok(RecoveryPlanState::Approved {
                approved_at,
                approved_fingerprint,
            })
        }
        "executing" => {
            let (approved_at, approved_fingerprint) = approved()?;
            Ok(RecoveryPlanState::Executing {
                approved_at,
                approved_fingerprint,
                execution_started_at: row.try_get("execution_started_at")?,
            })
        }
        "executed" => {
            let (approved_at, approved_fingerprint) = approved()?;
            Ok(RecoveryPlanState::Executed {
                approved_at,
                approved_fingerprint,
                execution_started_at: row.try_get("execution_started_at")?,
                executed_at: row.try_get("executed_at")?,
            })
        }
        "rejected" => Ok(RecoveryPlanState::Rejected {
            rejected_at: row.try_get("rejected_at")?,
        }),
        "expired" => Ok(RecoveryPlanState::Expired),
        "invalidated" => Ok(RecoveryPlanState::Invalidated {
            reason: parse_invalidation(row.try_get("invalidation_reason")?)?,
            invalidated_at: row.try_get("invalidated_at")?,
        }),
        value => Err(StoreError::InvalidData(format!(
            "unknown plan status {value}"
        ))),
    }
}

async fn write_state(
    tx: &mut Transaction<'_, Sqlite>,
    plan: &PersistedRecoveryPlan,
) -> Result<u64, StoreError> {
    write_state_inner(tx, plan, None).await
}

async fn write_state_conditionally(
    tx: &mut Transaction<'_, Sqlite>,
    plan: &PersistedRecoveryPlan,
    expected: &str,
) -> Result<u64, StoreError> {
    write_state_inner(tx, plan, Some(expected)).await
}

async fn write_state_inner(
    tx: &mut Transaction<'_, Sqlite>,
    plan: &PersistedRecoveryPlan,
    expected: Option<&str>,
) -> Result<u64, StoreError> {
    let (
        status,
        approved_at,
        approved_fingerprint,
        started_at,
        executed_at,
        rejected_at,
        reason,
        invalidated_at,
    ) = state_columns(&plan.state);
    let query = if expected.is_some() {
        "UPDATE recovery_plans SET status = ?, approved_at = ?, approved_fingerprint = ?, execution_started_at = ?, executed_at = ?, rejected_at = ?, invalidation_reason = ?, invalidated_at = ? WHERE session_id = ? AND id = ? AND status = ?"
    } else {
        "UPDATE recovery_plans SET status = ?, approved_at = ?, approved_fingerprint = ?, execution_started_at = ?, executed_at = ?, rejected_at = ?, invalidation_reason = ?, invalidated_at = ? WHERE session_id = ? AND id = ?"
    };
    let mut statement = sqlx::query(query)
        .bind(status)
        .bind(approved_at)
        .bind(approved_fingerprint)
        .bind(started_at)
        .bind(executed_at)
        .bind(rejected_at)
        .bind(reason)
        .bind(invalidated_at)
        .bind(session_text(plan.spec.session_id()))
        .bind(plan.spec.plan_id().as_uuid().to_string());
    if let Some(expected) = expected {
        statement = statement.bind(expected);
    }
    Ok(statement.execute(&mut **tx).await?.rows_affected())
}

type StateColumns = (
    &'static str,
    Option<DateTime<Utc>>,
    Option<String>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
    Option<&'static str>,
    Option<DateTime<Utc>>,
);

fn state_columns(state: &RecoveryPlanState) -> StateColumns {
    match state {
        RecoveryPlanState::Prepared => ("prepared", None, None, None, None, None, None, None),
        RecoveryPlanState::Approved {
            approved_at,
            approved_fingerprint,
        } => (
            "approved",
            Some(*approved_at),
            Some(approved_fingerprint.as_str().to_owned()),
            None,
            None,
            None,
            None,
            None,
        ),
        RecoveryPlanState::Executing {
            approved_at,
            approved_fingerprint,
            execution_started_at,
        } => (
            "executing",
            Some(*approved_at),
            Some(approved_fingerprint.as_str().to_owned()),
            Some(*execution_started_at),
            None,
            None,
            None,
            None,
        ),
        RecoveryPlanState::Executed {
            approved_at,
            approved_fingerprint,
            execution_started_at,
            executed_at,
        } => (
            "executed",
            Some(*approved_at),
            Some(approved_fingerprint.as_str().to_owned()),
            Some(*execution_started_at),
            Some(*executed_at),
            None,
            None,
            None,
        ),
        RecoveryPlanState::Rejected { rejected_at } => (
            "rejected",
            None,
            None,
            None,
            None,
            Some(*rejected_at),
            None,
            None,
        ),
        RecoveryPlanState::Expired => ("expired", None, None, None, None, None, None, None),
        RecoveryPlanState::Invalidated {
            reason,
            invalidated_at,
        } => (
            "invalidated",
            None,
            None,
            None,
            None,
            None,
            Some(invalidation_text(*reason)),
            Some(*invalidated_at),
        ),
    }
}

async fn expire_active_plan(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: &SessionId,
    now: DateTime<Utc>,
) -> Result<(), StoreError> {
    let plan_id = sqlx::query_scalar::<_, String>(
        "SELECT id FROM recovery_plans WHERE session_id = ? AND status IN ('prepared', 'approved') AND expires_at <= ? LIMIT 1",
    )
    .bind(session_text(session_id))
    .bind(now)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(value) = plan_id {
        let plan_id = parse_plan(value)?;
        let mut plan = load_plan(tx, session_id, &plan_id).await?;
        plan.state = expire_recovery(&plan.spec, plan.state, now);
        write_state(tx, &plan).await?;
        insert_audit(
            tx,
            session_id,
            "recovery_expired",
            Some(plan_id.as_uuid().to_string()),
            "expired",
            "Recovery approval window expired.",
            now,
            Some(format!("plan:{}:expired", plan_id.as_uuid())),
        )
        .await?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn insert_audit(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: &SessionId,
    event_type: &str,
    subject_id: Option<String>,
    outcome: &str,
    detail: &str,
    now: DateTime<Utc>,
    dedup_key: Option<String>,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT OR IGNORE INTO audit_events (session_id, id, event_type, subject_id, outcome, detail, recorded_at, dedup_key) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(session_text(session_id))
    .bind(Uuid::new_v4().to_string())
    .bind(event_type)
    .bind(subject_id)
    .bind(outcome)
    .bind(detail)
    .bind(now)
    .bind(dedup_key)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[allow(clippy::needless_pass_by_value)]
fn parse_plan(value: String) -> Result<PlanId, StoreError> {
    PlanId::parse(&value).map_err(|error| StoreError::InvalidData(error.to_string()))
}

#[allow(clippy::needless_pass_by_value)]
fn parse_incident(value: String) -> Result<IncidentId, StoreError> {
    IncidentId::parse(value).map_err(|error| StoreError::InvalidData(error.to_string()))
}

fn parse_service(value: String) -> Result<ServiceId, StoreError> {
    ServiceId::parse(value).map_err(|error| StoreError::InvalidData(error.to_string()))
}

fn parse_release(value: String) -> Result<ReleaseId, StoreError> {
    ReleaseId::parse(value).map_err(|error| StoreError::InvalidData(error.to_string()))
}

fn parse_diagnostic_status(value: &str) -> Result<DiagnosticStatus, StoreError> {
    match value {
        "passed" => Ok(DiagnosticStatus::Passed),
        "failed" => Ok(DiagnosticStatus::Failed),
        _ => Err(StoreError::InvalidData(format!(
            "unknown diagnostic status {value}"
        ))),
    }
}

#[allow(clippy::needless_pass_by_value)]
fn parse_invalidation(value: String) -> Result<RecoveryInvalidationReason, StoreError> {
    match value.as_str() {
        "session_reset" => Ok(RecoveryInvalidationReason::SessionReset),
        "scenario_generation_changed" => Ok(RecoveryInvalidationReason::ScenarioGenerationChanged),
        "active_release_changed" => Ok(RecoveryInvalidationReason::ActiveReleaseChanged),
        "target_became_ineligible" => Ok(RecoveryInvalidationReason::TargetBecameIneligible),
        _ => Err(StoreError::InvalidData(format!(
            "unknown invalidation reason {value}"
        ))),
    }
}

const fn invalidation_text(value: RecoveryInvalidationReason) -> &'static str {
    match value {
        RecoveryInvalidationReason::SessionReset => "session_reset",
        RecoveryInvalidationReason::ScenarioGenerationChanged => "scenario_generation_changed",
        RecoveryInvalidationReason::ActiveReleaseChanged => "active_release_changed",
        RecoveryInvalidationReason::TargetBecameIneligible => "target_became_ineligible",
    }
}

fn session_text(value: &SessionId) -> String {
    value.as_uuid().to_string()
}
