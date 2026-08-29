use chrono::{DateTime, Duration, TimeZone, Utc};
use recovery_domain::{
    DiagnosticStatus, EvidenceId, HealthStatus, HumanDecision, IncidentId, IncidentStatus, PlanId,
    PrepareRecoveryCommand, RecoveryDiagnosticEvidence, RecoveryError, RecoveryEvidence,
    RecoveryEvidenceKind, RecoveryExecutionFacts, RecoveryFingerprint, RecoveryInvalidationReason,
    RecoveryPlanSpec, RecoveryPlanState, RecoveryTelemetryEvidence, RecoveryVerificationFacts,
    RecoveryVerificationMismatch, RecoveryVerificationOutcome, ReleaseId, ReleaseState, ServiceId,
    SessionId, apply_human_decision, canonical_recovery_fingerprint, complete_execution,
    derive_verification, expire_recovery, invalidate_recovery, prepare_recovery,
    validate_execution,
};
use uuid::Uuid;

#[test]
fn preparation_accepts_two_to_eight_supported_unique_evidence_records() {
    let mut missing = valid_command();
    missing.evidence = vec![auth_log(&missing, "log_db_auth_1")];
    assert_eq!(
        prepare_recovery(missing).expect_err("the failed diagnostic is required"),
        RecoveryError::MissingEvidence(RecoveryEvidenceKind::FailedDatabaseConnectivityDiagnostic)
    );

    let mut unsupported = valid_command();
    unsupported.evidence[0] = RecoveryEvidence::Log {
        id: evidence_id("log_customer_note"),
        session_id: unsupported.session_id.clone(),
        service_id: unsupported.service_id.clone(),
        release_id: unsupported.expected_current_release.clone(),
        scenario_generation: unsupported.scenario_generation,
        code: "CUSTOMER_NOTE_CAPTURED".to_owned(),
    };
    assert_eq!(
        prepare_recovery(unsupported).expect_err("unrelated logs are not recovery evidence"),
        RecoveryError::UnsupportedEvidence(evidence_id("log_customer_note"))
    );

    let mut duplicate = valid_command();
    duplicate.evidence.insert(1, duplicate.evidence[0].clone());
    assert_eq!(
        prepare_recovery(duplicate).expect_err("duplicate evidence is rejected"),
        RecoveryError::DuplicateEvidence(evidence_id("log_db_auth_1"))
    );

    let mut eight = valid_command();
    eight.evidence = (1..=5)
        .map(|index| auth_log(&eight, &format!("log_db_auth_{index}")))
        .chain((1..=3).map(|index| diagnostic(&eight, &format!("diagnostic_db_{index}"))))
        .collect();
    let plan = prepare_recovery(eight.clone()).expect("eight relevant unique records are valid");
    assert_eq!(plan.evidence().iter().count(), 8);

    let ninth = diagnostic(&eight, "diagnostic_db_4");
    eight.evidence.push(ninth);
    assert_eq!(
        prepare_recovery(eight).expect_err("the ninth evidence record exceeds the policy bound"),
        RecoveryError::TooManyEvidence
    );
}

#[test]
fn preparation_rejects_evidence_with_unrelated_context() {
    let mut wrong_release = valid_command();
    let release_evidence_id = evidence_id("log_db_auth_1");
    wrong_release.evidence[0] = RecoveryEvidence::Log {
        id: release_evidence_id.clone(),
        session_id: wrong_release.session_id.clone(),
        service_id: wrong_release.service_id.clone(),
        release_id: wrong_release.target_release.clone(),
        scenario_generation: wrong_release.scenario_generation,
        code: "DB_AUTH_METHOD_MISMATCH".to_owned(),
    };
    assert_eq!(
        prepare_recovery(wrong_release).expect_err("target-release evidence is unrelated"),
        RecoveryError::EvidenceReleaseMismatch(release_evidence_id)
    );

    let mut wrong_generation = valid_command();
    let generation_evidence_id = evidence_id("diagnostic_db");
    wrong_generation.evidence[1] = RecoveryEvidence::Diagnostic {
        id: generation_evidence_id.clone(),
        session_id: wrong_generation.session_id.clone(),
        service_id: wrong_generation.service_id.clone(),
        release_id: wrong_generation.expected_current_release.clone(),
        scenario_generation: wrong_generation.scenario_generation + 1,
        kind: "database_connectivity".to_owned(),
        status: DiagnosticStatus::Failed,
        code: "DB_AUTH_METHOD_MISMATCH".to_owned(),
    };
    assert_eq!(
        prepare_recovery(wrong_generation).expect_err("old-generation evidence is stale"),
        RecoveryError::EvidenceGenerationMismatch(generation_evidence_id)
    );

    let mut wrong_session = valid_command();
    let session_evidence_id = evidence_id("log_db_auth_1");
    wrong_session.evidence[0] = RecoveryEvidence::Log {
        id: session_evidence_id.clone(),
        session_id: SessionId::new(),
        service_id: wrong_session.service_id.clone(),
        release_id: wrong_session.expected_current_release.clone(),
        scenario_generation: wrong_session.scenario_generation,
        code: "DB_AUTH_METHOD_MISMATCH".to_owned(),
    };
    assert_eq!(
        prepare_recovery(wrong_session).expect_err("foreign-session evidence is unrelated"),
        RecoveryError::EvidenceSessionMismatch(session_evidence_id)
    );

    let mut wrong_service = valid_command();
    let service_evidence_id = evidence_id("diagnostic_db");
    wrong_service.evidence[1] = RecoveryEvidence::Diagnostic {
        id: service_evidence_id.clone(),
        session_id: wrong_service.session_id.clone(),
        service_id: service_id("payments-api"),
        release_id: wrong_service.expected_current_release.clone(),
        scenario_generation: wrong_service.scenario_generation,
        kind: "database_connectivity".to_owned(),
        status: DiagnosticStatus::Failed,
        code: "DB_AUTH_METHOD_MISMATCH".to_owned(),
    };
    assert_eq!(
        prepare_recovery(wrong_service).expect_err("foreign-service evidence is unrelated"),
        RecoveryError::EvidenceServiceMismatch(service_evidence_id)
    );
}

#[test]
fn preparation_canonicalizes_order_and_fingerprints_field_boundaries() {
    let command = valid_command();
    let mut reversed = command.clone();
    reversed.evidence.reverse();
    let ordered = prepare_recovery(command).expect("canonical plan is prepared");
    let reordered = prepare_recovery(reversed).expect("input order is not authoritative");

    assert_eq!(ordered, reordered);
    assert_eq!(
        ordered.reason(),
        "Rollback the database authentication regression."
    );
    assert_eq!(
        ordered
            .evidence()
            .iter()
            .map(RecoveryEvidence::kind)
            .collect::<Vec<_>>(),
        vec![
            RecoveryEvidenceKind::DatabaseAuthenticationFailureLog,
            RecoveryEvidenceKind::FailedDatabaseConnectivityDiagnostic,
        ]
    );
    assert_eq!(
        canonical_recovery_fingerprint(&ordered),
        canonical_recovery_fingerprint(&reordered)
    );

    let mut delimiter_in_reason = valid_command();
    delimiter_in_reason.reason = "Rollback|database".to_owned();
    delimiter_in_reason.evidence = vec![
        auth_log(&delimiter_in_reason, "auth"),
        diagnostic(&delimiter_in_reason, "diagnostic"),
    ];
    let mut delimiter_in_id = valid_command();
    delimiter_in_id.reason = "Rollback".to_owned();
    delimiter_in_id.evidence = vec![
        auth_log(&delimiter_in_id, "database|auth"),
        diagnostic(&delimiter_in_id, "diagnostic"),
    ];
    assert_ne!(
        canonical_recovery_fingerprint(
            &prepare_recovery(delimiter_in_reason).expect("first delimiter fixture is valid")
        ),
        canonical_recovery_fingerprint(
            &prepare_recovery(delimiter_in_id).expect("second delimiter fixture is valid")
        )
    );
}

#[test]
fn expiry_approval_replay_and_revocation_have_exact_semantics() {
    let plan = prepared_plan();
    let approved_at = plan.created_at() + Duration::minutes(1);
    let approved = approve(&plan, RecoveryPlanState::Prepared, approved_at);

    assert_eq!(
        expire_recovery(
            &plan,
            approved.clone(),
            plan.expires_at() - Duration::nanoseconds(1)
        ),
        approved
    );
    assert_eq!(
        expire_recovery(&plan, approved.clone(), plan.expires_at()),
        RecoveryPlanState::Expired
    );

    let replayed = approve(&plan, approved.clone(), approved_at + Duration::minutes(1));
    assert_eq!(replayed, approved);
    assert_eq!(replayed.approved_at(), Some(approved_at));

    let rejected_at = approved_at + Duration::minutes(2);
    assert_eq!(
        apply_human_decision(&plan, approved, HumanDecision::Reject, rejected_at)
            .expect("the human can revoke approval before execution"),
        RecoveryPlanState::Rejected { rejected_at }
    );
}

#[test]
fn invalidation_retains_the_authority_fact_that_changed() {
    let invalidated_at = fixture_time() + Duration::minutes(2);
    for reason in [
        RecoveryInvalidationReason::SessionReset,
        RecoveryInvalidationReason::ScenarioGenerationChanged,
        RecoveryInvalidationReason::ActiveReleaseChanged,
        RecoveryInvalidationReason::TargetBecameIneligible,
    ] {
        assert_eq!(
            invalidate_recovery(RecoveryPlanState::Prepared, reason, invalidated_at),
            RecoveryPlanState::Invalidated {
                reason,
                invalidated_at,
            }
        );
    }
}

#[test]
fn persisted_fingerprints_require_lowercase_sha256_hex() {
    let canonical = canonical_recovery_fingerprint(&prepared_plan());
    assert_eq!(
        RecoveryFingerprint::parse(canonical.as_str().to_owned())
            .expect("a canonical fingerprint reconstructs safely"),
        canonical
    );
    for invalid in ["f".repeat(63), "g".repeat(64), "A".repeat(64)] {
        assert_eq!(
            RecoveryFingerprint::parse(invalid),
            Err(RecoveryError::InvalidFingerprint)
        );
    }
}

#[test]
fn execution_rechecks_every_authority_precondition() {
    let plan = prepared_plan();
    let approved_at = plan.created_at() + Duration::minutes(1);
    let approved = approve(&plan, RecoveryPlanState::Prepared, approved_at);
    let valid_facts = valid_execution_facts(&plan);
    let execution_time = approved_at + Duration::minutes(1);

    let mut foreign = valid_facts.clone();
    foreign.session_id = SessionId::new();
    let mut stale_generation = valid_facts.clone();
    stale_generation.scenario_generation += 1;
    let mut stale_release = valid_facts.clone();
    stale_release.active_release = plan.target_release().clone();
    let mut wrong_target = valid_facts.clone();
    wrong_target.target_release = ReleaseId::from_static("release_282");
    let mut foreign_target = valid_facts.clone();
    foreign_target.target_service_id = service_id("payments-api");
    let mut ineligible = valid_facts.clone();
    ineligible.target_release_state = ReleaseState::Staged;
    let mut changed_fingerprint = valid_facts.clone();
    changed_fingerprint.stored_fingerprint = "0".repeat(64);
    let invalidated = invalidate_recovery(
        approved.clone(),
        RecoveryInvalidationReason::SessionReset,
        execution_time,
    );

    let executing = validate_execution(&plan, approved.clone(), &valid_facts, execution_time)
        .expect("valid execution facts claim the plan");
    let executed =
        complete_execution(executing, execution_time).expect("a claimed plan can complete once");

    let cases = vec![
        (
            RecoveryPlanState::Prepared,
            valid_facts.clone(),
            execution_time,
            RecoveryError::NotApproved,
        ),
        (
            approved.clone(),
            valid_facts.clone(),
            plan.expires_at(),
            RecoveryError::Expired,
        ),
        (
            approved.clone(),
            foreign,
            execution_time,
            RecoveryError::CrossSession,
        ),
        (
            approved.clone(),
            stale_generation,
            execution_time,
            RecoveryError::StaleGeneration,
        ),
        (
            approved.clone(),
            stale_release,
            execution_time,
            RecoveryError::StaleActiveRelease,
        ),
        (
            approved.clone(),
            wrong_target,
            execution_time,
            RecoveryError::TargetReleaseMismatch,
        ),
        (
            approved.clone(),
            foreign_target,
            execution_time,
            RecoveryError::TargetServiceMismatch,
        ),
        (
            approved.clone(),
            ineligible,
            execution_time,
            RecoveryError::TargetIneligible,
        ),
        (
            approved,
            changed_fingerprint,
            execution_time,
            RecoveryError::StoredFingerprintMismatch,
        ),
        (
            invalidated,
            valid_facts.clone(),
            execution_time,
            RecoveryError::Invalidated,
        ),
        (
            executed,
            valid_facts,
            execution_time + Duration::seconds(1),
            RecoveryError::AlreadyExecuted,
        ),
    ];

    for (state, facts, now, expected) in cases {
        assert_eq!(validate_execution(&plan, state, &facts, now), Err(expected));
    }
}

#[test]
fn execution_rechecks_the_active_incident() {
    let plan = prepared_plan();
    let now = plan.created_at() + Duration::minutes(1);
    let approved = approve(&plan, RecoveryPlanState::Prepared, now);

    let mut wrong_incident = valid_execution_facts(&plan);
    wrong_incident.active_incident_id = incident_id("inc_other");
    assert_eq!(
        validate_execution(&plan, approved.clone(), &wrong_incident, now),
        Err(RecoveryError::ActiveIncidentMismatch)
    );

    let mut resolved_incident = valid_execution_facts(&plan);
    resolved_incident.active_incident_status = IncidentStatus::Resolved;
    assert_eq!(
        validate_execution(&plan, approved, &resolved_incident, now),
        Err(RecoveryError::IncidentNotActive)
    );
}

#[test]
fn verification_is_derived_from_persisted_before_and_after_facts() {
    let plan = prepared_plan();
    let state = executed_state(&plan);
    let verified_at = plan.created_at() + Duration::minutes(3);
    let facts = valid_verification_facts(&plan);
    let expected_telemetry = facts.telemetry.clone();
    let expected_diagnostic = facts.diagnostic.clone();
    let verification = derive_verification(&plan, &state, facts, verified_at)
        .expect("matching persisted facts verify the recovery");

    assert_eq!(verification.outcome, RecoveryVerificationOutcome::Passed);
    assert_eq!(
        verification.before.release,
        *plan.expected_current_release()
    );
    assert_eq!(verification.before.evidence, plan.evidence().clone());
    assert_eq!(verification.after.release, *plan.target_release());
    assert_eq!(verification.after.health_status, HealthStatus::Healthy);
    assert_eq!(verification.after.telemetry, expected_telemetry);
    assert_eq!(verification.after.diagnostic, expected_diagnostic);
    assert_eq!(verification.verified_at, verified_at);
}

#[test]
fn verification_reports_all_persisted_fact_mismatches() {
    let plan = prepared_plan();
    let state = executed_state(&plan);
    let mut facts = valid_verification_facts(&plan);
    facts.execution_plan_id = fixed_plan_id("00000000-0000-4000-8000-000000000099");
    facts.scenario_generation += 1;
    facts.previous_release = ReleaseId::from_static("release_282");
    facts.current_release = plan.expected_current_release().clone();
    facts.service_id = service_id("payments-api");
    facts.health_status = HealthStatus::Critical;
    facts.incident_id = incident_id("inc_other");
    facts.incident_status = IncidentStatus::Active;
    facts.telemetry.plan_id = fixed_plan_id("00000000-0000-4000-8000-000000000098");
    facts.telemetry.service_id = service_id("payments-api");
    facts.telemetry.release_id = plan.expected_current_release().clone();
    facts.telemetry.scenario_generation += 1;
    facts.diagnostic.plan_id = fixed_plan_id("00000000-0000-4000-8000-000000000097");
    facts.diagnostic.service_id = service_id("payments-api");
    facts.diagnostic.release_id = plan.expected_current_release().clone();
    facts.diagnostic.scenario_generation += 1;
    facts.diagnostic.kind = "cache_connectivity".to_owned();
    facts.diagnostic.status = DiagnosticStatus::Failed;
    facts.diagnostic.code = "CACHE_CONNECTION_OK".to_owned();
    facts.stored_fingerprint = "f".repeat(64);

    let verification = derive_verification(
        &plan,
        &state,
        facts,
        plan.created_at() + Duration::minutes(3),
    )
    .expect("mismatches are a result, not an authorization error");
    assert_eq!(
        verification.outcome,
        RecoveryVerificationOutcome::Mismatch {
            mismatches: vec![
                RecoveryVerificationMismatch::ExecutionPlan,
                RecoveryVerificationMismatch::ScenarioGeneration,
                RecoveryVerificationMismatch::PreviousRelease,
                RecoveryVerificationMismatch::CurrentRelease,
                RecoveryVerificationMismatch::Service,
                RecoveryVerificationMismatch::Health,
                RecoveryVerificationMismatch::Incident,
                RecoveryVerificationMismatch::IncidentStatus,
                RecoveryVerificationMismatch::TelemetryPlan,
                RecoveryVerificationMismatch::TelemetryService,
                RecoveryVerificationMismatch::TelemetryRelease,
                RecoveryVerificationMismatch::TelemetryGeneration,
                RecoveryVerificationMismatch::DiagnosticPlan,
                RecoveryVerificationMismatch::DiagnosticService,
                RecoveryVerificationMismatch::DiagnosticRelease,
                RecoveryVerificationMismatch::DiagnosticGeneration,
                RecoveryVerificationMismatch::DiagnosticKind,
                RecoveryVerificationMismatch::DiagnosticStatus,
                RecoveryVerificationMismatch::DiagnosticCode,
                RecoveryVerificationMismatch::Fingerprint,
            ]
        }
    );
    assert_eq!(
        derive_verification(
            &plan,
            &RecoveryPlanState::Prepared,
            valid_verification_facts(&plan),
            plan.created_at() + Duration::minutes(1)
        ),
        Err(RecoveryError::NotExecuted)
    );
}

fn valid_command() -> PrepareRecoveryCommand {
    let mut command = PrepareRecoveryCommand {
        plan_id: fixed_plan_id("00000000-0000-4000-8000-000000000001"),
        session_id: fixed_session_id("00000000-0000-4000-8000-000000000002"),
        incident_id: IncidentId::checkout_failures(),
        service_id: ServiceId::checkout_api(),
        scenario_generation: 7,
        expected_current_release: ReleaseId::from_static("release_284"),
        target_release: ReleaseId::from_static("release_283"),
        target_service_id: ServiceId::checkout_api(),
        target_release_state: ReleaseState::HealthyBaseline,
        reason: "  Rollback the database authentication regression.  ".to_owned(),
        evidence: Vec::new(),
        created_at: fixture_time(),
    };
    command.evidence = vec![
        auth_log(&command, "log_db_auth_1"),
        diagnostic(&command, "diagnostic_db"),
    ];
    command
}

fn prepared_plan() -> RecoveryPlanSpec {
    prepare_recovery(valid_command()).expect("fixture plan is valid")
}

fn auth_log(command: &PrepareRecoveryCommand, id: &str) -> RecoveryEvidence {
    RecoveryEvidence::Log {
        id: evidence_id(id),
        session_id: command.session_id.clone(),
        service_id: command.service_id.clone(),
        release_id: command.expected_current_release.clone(),
        scenario_generation: command.scenario_generation,
        code: "DB_AUTH_METHOD_MISMATCH".to_owned(),
    }
}

fn diagnostic(command: &PrepareRecoveryCommand, id: &str) -> RecoveryEvidence {
    RecoveryEvidence::Diagnostic {
        id: evidence_id(id),
        session_id: command.session_id.clone(),
        service_id: command.service_id.clone(),
        release_id: command.expected_current_release.clone(),
        scenario_generation: command.scenario_generation,
        kind: "database_connectivity".to_owned(),
        status: DiagnosticStatus::Failed,
        code: "DB_AUTH_METHOD_MISMATCH".to_owned(),
    }
}

fn approve(
    plan: &RecoveryPlanSpec,
    state: RecoveryPlanState,
    now: DateTime<Utc>,
) -> RecoveryPlanState {
    apply_human_decision(
        plan,
        state,
        HumanDecision::Approve {
            fingerprint: canonical_recovery_fingerprint(plan).as_str().to_owned(),
        },
        now,
    )
    .expect("fixture approval is valid")
}

fn valid_execution_facts(plan: &RecoveryPlanSpec) -> RecoveryExecutionFacts {
    RecoveryExecutionFacts {
        session_id: plan.session_id().clone(),
        scenario_generation: plan.scenario_generation(),
        active_incident_id: plan.incident_id().clone(),
        active_incident_status: IncidentStatus::Active,
        active_release: plan.expected_current_release().clone(),
        target_release: plan.target_release().clone(),
        target_service_id: plan.service_id().clone(),
        target_release_state: ReleaseState::HealthyBaseline,
        stored_fingerprint: canonical_recovery_fingerprint(plan).as_str().to_owned(),
    }
}

fn executed_state(plan: &RecoveryPlanSpec) -> RecoveryPlanState {
    let approved_at = plan.created_at() + Duration::minutes(1);
    let executing = validate_execution(
        plan,
        approve(plan, RecoveryPlanState::Prepared, approved_at),
        &valid_execution_facts(plan),
        approved_at + Duration::minutes(1),
    )
    .expect("fixture execution is authorized");
    complete_execution(executing, approved_at + Duration::minutes(1))
        .expect("fixture execution completes")
}

fn valid_verification_facts(plan: &RecoveryPlanSpec) -> RecoveryVerificationFacts {
    RecoveryVerificationFacts {
        execution_plan_id: plan.plan_id().clone(),
        scenario_generation: plan.scenario_generation(),
        previous_release: plan.expected_current_release().clone(),
        current_release: plan.target_release().clone(),
        service_id: plan.service_id().clone(),
        health_status: HealthStatus::Healthy,
        incident_id: plan.incident_id().clone(),
        incident_status: IncidentStatus::Resolved,
        telemetry: RecoveryTelemetryEvidence {
            plan_id: plan.plan_id().clone(),
            service_id: plan.service_id().clone(),
            release_id: plan.target_release().clone(),
            scenario_generation: plan.scenario_generation(),
            recorded_at: plan.created_at() + Duration::minutes(2),
            error_rate_percent: 0.3,
            p95_latency_ms: 182,
            request_rate_rps: 221,
        },
        diagnostic: RecoveryDiagnosticEvidence {
            plan_id: plan.plan_id().clone(),
            id: evidence_id("diagnostic_after_recovery"),
            service_id: plan.service_id().clone(),
            release_id: plan.target_release().clone(),
            scenario_generation: plan.scenario_generation(),
            kind: "database_connectivity".to_owned(),
            status: DiagnosticStatus::Passed,
            code: "DB_CONNECTION_OK".to_owned(),
            summary: "checkout-api can authenticate to the primary database.".to_owned(),
            evidence: "release_283 uses scram-sha-256 authentication.".to_owned(),
            checked_at: plan.created_at() + Duration::minutes(2),
        },
        stored_fingerprint: canonical_recovery_fingerprint(plan).as_str().to_owned(),
    }
}

fn evidence_id(value: &str) -> EvidenceId {
    EvidenceId::parse(value.to_owned()).expect("fixture evidence ID is valid")
}

fn service_id(value: &str) -> ServiceId {
    ServiceId::parse(value.to_owned()).expect("fixture service ID is valid")
}

fn incident_id(value: &str) -> IncidentId {
    IncidentId::parse(value.to_owned()).expect("fixture incident ID is valid")
}

fn fixed_plan_id(value: &str) -> PlanId {
    PlanId::from(Uuid::parse_str(value).expect("fixture plan UUID is valid"))
}

fn fixed_session_id(value: &str) -> SessionId {
    SessionId::from(Uuid::parse_str(value).expect("fixture session UUID is valid"))
}

fn fixture_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 26, 5, 0, 0)
        .single()
        .expect("fixture timestamp is valid")
}
