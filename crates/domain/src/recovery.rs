//! Pure recovery-plan facts and authority transitions.

use std::collections::HashSet;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    DiagnosticStatus, HealthStatus, IncidentId, IncidentStatus, PlanId, ReleaseId, ReleaseState,
    ServiceId, SessionId,
};

const RECOVERY_POLICY_VERSION: u32 = 1;
const FINGERPRINT_ENCODING_VERSION: u32 = 1;
const PLAN_LIFETIME: Duration = Duration::minutes(10);
const MAX_REASON_BYTES: usize = 240;
const DATABASE_AUTH_FAILURE_CODE: &str = "DB_AUTH_METHOD_MISMATCH";
const DATABASE_CONNECTIVITY_KIND: &str = "database_connectivity";

/// Identifier for one persisted evidence record.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct EvidenceId(String);

impl EvidenceId {
    /// Parses a persisted evidence identifier.
    ///
    /// # Errors
    ///
    /// Returns [`RecoveryError::InvalidEvidenceId`] when `value` is empty or whitespace.
    pub fn parse(value: String) -> Result<Self, RecoveryError> {
        if value.trim().is_empty() {
            return Err(RecoveryError::InvalidEvidenceId);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The two evidence roles accepted by recovery policy version 1.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryEvidenceKind {
    DatabaseAuthenticationFailureLog,
    FailedDatabaseConnectivityDiagnostic,
}

/// An evidence candidate resolved from an authoritative source row.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum RecoveryEvidence {
    Log {
        id: EvidenceId,
        session_id: SessionId,
        service_id: ServiceId,
        release_id: ReleaseId,
        scenario_generation: i64,
        code: String,
    },
    Diagnostic {
        id: EvidenceId,
        session_id: SessionId,
        service_id: ServiceId,
        release_id: ReleaseId,
        scenario_generation: i64,
        kind: String,
        status: DiagnosticStatus,
        code: String,
    },
}

impl RecoveryEvidence {
    #[must_use]
    pub const fn id(&self) -> &EvidenceId {
        match self {
            Self::Log { id, .. } | Self::Diagnostic { id, .. } => id,
        }
    }

    #[must_use]
    pub const fn session_id(&self) -> &SessionId {
        match self {
            Self::Log { session_id, .. } | Self::Diagnostic { session_id, .. } => session_id,
        }
    }

    #[must_use]
    pub const fn service_id(&self) -> &ServiceId {
        match self {
            Self::Log { service_id, .. } | Self::Diagnostic { service_id, .. } => service_id,
        }
    }

    #[must_use]
    pub const fn release_id(&self) -> &ReleaseId {
        match self {
            Self::Log { release_id, .. } | Self::Diagnostic { release_id, .. } => release_id,
        }
    }

    #[must_use]
    pub const fn scenario_generation(&self) -> i64 {
        match self {
            Self::Log {
                scenario_generation,
                ..
            }
            | Self::Diagnostic {
                scenario_generation,
                ..
            } => *scenario_generation,
        }
    }

    #[must_use]
    pub const fn kind(&self) -> RecoveryEvidenceKind {
        match self {
            Self::Log { .. } => RecoveryEvidenceKind::DatabaseAuthenticationFailureLog,
            Self::Diagnostic { .. } => RecoveryEvidenceKind::FailedDatabaseConnectivityDiagnostic,
        }
    }

    fn is_supported(&self) -> bool {
        match self {
            Self::Log { code, .. } => code == DATABASE_AUTH_FAILURE_CODE,
            Self::Diagnostic {
                kind, status, code, ..
            } => {
                kind == DATABASE_CONNECTIVITY_KIND
                    && *status == DiagnosticStatus::Failed
                    && code == DATABASE_AUTH_FAILURE_CODE
            }
        }
    }

    fn write_canonical(&self, encoder: &mut CanonicalEncoder) {
        match self {
            Self::Log {
                id,
                session_id,
                service_id,
                release_id,
                scenario_generation,
                code,
            } => {
                encoder.push(b"database_authentication_failure_log");
                encoder.push(id.as_str().as_bytes());
                encoder.push(session_id.as_uuid().as_bytes());
                encoder.push(service_id.as_str().as_bytes());
                encoder.push(release_id.as_str().as_bytes());
                encoder.push(&scenario_generation.to_be_bytes());
                encoder.push(code.as_bytes());
            }
            Self::Diagnostic {
                id,
                session_id,
                service_id,
                release_id,
                scenario_generation,
                kind,
                status,
                code,
            } => {
                encoder.push(b"failed_database_connectivity_diagnostic");
                encoder.push(id.as_str().as_bytes());
                encoder.push(session_id.as_uuid().as_bytes());
                encoder.push(service_id.as_str().as_bytes());
                encoder.push(release_id.as_str().as_bytes());
                encoder.push(&scenario_generation.to_be_bytes());
                encoder.push(kind.as_bytes());
                encoder.push(diagnostic_status_bytes(*status));
                encoder.push(code.as_bytes());
            }
        }
    }
}

/// The exact, canonical evidence pair reviewed with a recovery plan.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct RecoveryEvidenceSet([RecoveryEvidence; 2]);

impl RecoveryEvidenceSet {
    fn try_new(
        mut evidence: Vec<RecoveryEvidence>,
        session_id: &SessionId,
        service_id: &ServiceId,
        release_id: &ReleaseId,
        scenario_generation: i64,
    ) -> Result<Self, RecoveryError> {
        let mut ids = HashSet::with_capacity(evidence.len());
        for item in &evidence {
            if !item.is_supported() {
                return Err(RecoveryError::UnsupportedEvidence(item.id().clone()));
            }
            if !ids.insert(item.id().clone()) {
                return Err(RecoveryError::DuplicateEvidence(item.id().clone()));
            }
            if item.session_id() != session_id {
                return Err(RecoveryError::EvidenceSessionMismatch(item.id().clone()));
            }
            if item.service_id() != service_id {
                return Err(RecoveryError::EvidenceServiceMismatch(item.id().clone()));
            }
            if item.release_id() != release_id {
                return Err(RecoveryError::EvidenceReleaseMismatch(item.id().clone()));
            }
            if item.scenario_generation() != scenario_generation {
                return Err(RecoveryError::EvidenceGenerationMismatch(item.id().clone()));
            }
        }

        for kind in [
            RecoveryEvidenceKind::DatabaseAuthenticationFailureLog,
            RecoveryEvidenceKind::FailedDatabaseConnectivityDiagnostic,
        ] {
            match evidence.iter().filter(|item| item.kind() == kind).count() {
                0 => return Err(RecoveryError::MissingEvidence(kind)),
                1 => {}
                _ => return Err(RecoveryError::AmbiguousEvidence(kind)),
            }
        }

        evidence.sort_by(|left, right| {
            left.kind()
                .cmp(&right.kind())
                .then_with(|| left.id().cmp(right.id()))
        });
        match <[RecoveryEvidence; 2]>::try_from(evidence) {
            Ok(ordered) => Ok(Self(ordered)),
            Err(_) => unreachable!("one item of each evidence kind is exactly two items"),
        }
    }

    pub fn iter(&self) -> std::slice::Iter<'_, RecoveryEvidence> {
        self.0.iter()
    }
}

impl<'a> IntoIterator for &'a RecoveryEvidenceSet {
    type Item = &'a RecoveryEvidence;
    type IntoIter = std::slice::Iter<'a, RecoveryEvidence>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

/// Untrusted preparation input assembled from request and persisted facts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrepareRecoveryCommand {
    pub plan_id: PlanId,
    pub session_id: SessionId,
    pub incident_id: IncidentId,
    pub service_id: ServiceId,
    pub scenario_generation: i64,
    pub expected_current_release: ReleaseId,
    pub target_release: ReleaseId,
    pub target_service_id: ServiceId,
    pub target_release_state: ReleaseState,
    pub reason: String,
    pub evidence: Vec<RecoveryEvidence>,
    pub created_at: DateTime<Utc>,
}

/// Immutable normalized facts bound to one human-reviewed recovery plan.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryPlanSpec {
    plan_id: PlanId,
    session_id: SessionId,
    incident_id: IncidentId,
    service_id: ServiceId,
    scenario_generation: i64,
    expected_current_release: ReleaseId,
    target_release: ReleaseId,
    reason: String,
    evidence: RecoveryEvidenceSet,
    policy_version: u32,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

impl RecoveryPlanSpec {
    #[must_use]
    pub const fn plan_id(&self) -> &PlanId {
        &self.plan_id
    }

    #[must_use]
    pub const fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    #[must_use]
    pub const fn incident_id(&self) -> &IncidentId {
        &self.incident_id
    }

    #[must_use]
    pub const fn service_id(&self) -> &ServiceId {
        &self.service_id
    }

    #[must_use]
    pub const fn scenario_generation(&self) -> i64 {
        self.scenario_generation
    }

    #[must_use]
    pub const fn expected_current_release(&self) -> &ReleaseId {
        &self.expected_current_release
    }

    #[must_use]
    pub const fn target_release(&self) -> &ReleaseId {
        &self.target_release
    }

    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }

    #[must_use]
    pub const fn evidence(&self) -> &RecoveryEvidenceSet {
        &self.evidence
    }

    #[must_use]
    pub const fn policy_version(&self) -> u32 {
        self.policy_version
    }

    #[must_use]
    pub const fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    #[must_use]
    pub const fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }
}

/// Creates a normalized plan without changing production state.
///
/// # Errors
///
/// Returns [`RecoveryError`] when the target, generation, reason, timestamp, or evidence is invalid.
pub fn prepare_recovery(
    command: PrepareRecoveryCommand,
) -> Result<RecoveryPlanSpec, RecoveryError> {
    if command.scenario_generation <= 0 {
        return Err(RecoveryError::InvalidGeneration);
    }
    if command.target_release == command.expected_current_release
        || command.target_service_id != command.service_id
        || command.target_release_state != ReleaseState::HealthyBaseline
    {
        return Err(RecoveryError::InvalidTarget);
    }
    let reason = command.reason.trim();
    if reason.is_empty() || reason.len() > MAX_REASON_BYTES {
        return Err(RecoveryError::InvalidReason);
    }
    let evidence = RecoveryEvidenceSet::try_new(
        command.evidence,
        &command.session_id,
        &command.service_id,
        &command.expected_current_release,
        command.scenario_generation,
    )?;
    let expires_at = command
        .created_at
        .checked_add_signed(PLAN_LIFETIME)
        .ok_or(RecoveryError::TimestampOverflow)?;

    Ok(RecoveryPlanSpec {
        plan_id: command.plan_id,
        session_id: command.session_id,
        incident_id: command.incident_id,
        service_id: command.service_id,
        scenario_generation: command.scenario_generation,
        expected_current_release: command.expected_current_release,
        target_release: command.target_release,
        reason: reason.to_owned(),
        evidence,
        policy_version: RECOVERY_POLICY_VERSION,
        created_at: command.created_at,
        expires_at,
    })
}

/// A SHA-256 fingerprint over versioned, length-prefixed normalized facts.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct RecoveryFingerprint(String);

impl RecoveryFingerprint {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Calculates the authority fingerprint for every reviewed plan fact.
#[must_use]
pub fn canonical_recovery_fingerprint(spec: &RecoveryPlanSpec) -> RecoveryFingerprint {
    let mut encoder = CanonicalEncoder::default();
    encoder.push(b"recovery_plan");
    encoder.push(&FINGERPRINT_ENCODING_VERSION.to_be_bytes());
    encoder.push(spec.plan_id.as_uuid().as_bytes());
    encoder.push(spec.session_id.as_uuid().as_bytes());
    encoder.push(spec.incident_id.as_str().as_bytes());
    encoder.push(spec.service_id.as_str().as_bytes());
    encoder.push(&spec.scenario_generation.to_be_bytes());
    encoder.push(spec.expected_current_release.as_str().as_bytes());
    encoder.push(spec.target_release.as_str().as_bytes());
    encoder.push(spec.reason.as_bytes());
    encoder.push(&spec.policy_version.to_be_bytes());
    push_timestamp(&mut encoder, spec.created_at);
    push_timestamp(&mut encoder, spec.expires_at);
    encoder.push(&(spec.evidence.0.len() as u64).to_be_bytes());
    for evidence in &spec.evidence.0 {
        evidence.write_canonical(&mut encoder);
    }
    RecoveryFingerprint(format!("{:x}", Sha256::digest(encoder.bytes)))
}

#[derive(Default)]
struct CanonicalEncoder {
    bytes: Vec<u8>,
}

impl CanonicalEncoder {
    fn push(&mut self, value: &[u8]) {
        let length = u64::try_from(value.len()).expect("in-memory field length fits in u64");
        self.bytes.extend_from_slice(&length.to_be_bytes());
        self.bytes.extend_from_slice(value);
    }
}

fn push_timestamp(encoder: &mut CanonicalEncoder, value: DateTime<Utc>) {
    encoder.push(&value.timestamp().to_be_bytes());
    encoder.push(&value.timestamp_subsec_nanos().to_be_bytes());
}

const fn diagnostic_status_bytes(status: DiagnosticStatus) -> &'static [u8] {
    match status {
        DiagnosticStatus::Passed => b"passed",
        DiagnosticStatus::Failed => b"failed",
    }
}

/// One explicit human decision at the plan review boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum HumanDecision {
    Approve { fingerprint: String },
    Reject,
}

/// Valid recovery lifecycle states without contradictory optional fields.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RecoveryPlanState {
    Prepared,
    Approved {
        approved_at: DateTime<Utc>,
        approved_fingerprint: RecoveryFingerprint,
    },
    Executing {
        approved_at: DateTime<Utc>,
        approved_fingerprint: RecoveryFingerprint,
        execution_started_at: DateTime<Utc>,
    },
    Executed {
        approved_at: DateTime<Utc>,
        approved_fingerprint: RecoveryFingerprint,
        execution_started_at: DateTime<Utc>,
        executed_at: DateTime<Utc>,
    },
    Rejected {
        rejected_at: DateTime<Utc>,
    },
    Expired,
    Invalidated {
        invalidated_at: DateTime<Utc>,
    },
}

impl RecoveryPlanState {
    #[must_use]
    pub const fn approved_at(&self) -> Option<DateTime<Utc>> {
        match self {
            Self::Approved { approved_at, .. }
            | Self::Executing { approved_at, .. }
            | Self::Executed { approved_at, .. } => Some(*approved_at),
            Self::Prepared | Self::Rejected { .. } | Self::Expired | Self::Invalidated { .. } => {
                None
            }
        }
    }
}

/// Applies deadline expiry to a prepared or approved plan.
#[must_use]
pub fn expire_recovery(
    spec: &RecoveryPlanSpec,
    state: RecoveryPlanState,
    now: DateTime<Utc>,
) -> RecoveryPlanState {
    if now >= spec.expires_at
        && matches!(
            state,
            RecoveryPlanState::Prepared | RecoveryPlanState::Approved { .. }
        )
    {
        RecoveryPlanState::Expired
    } else {
        state
    }
}

/// Applies an exact approval or revokes approval through rejection.
///
/// # Errors
///
/// Returns [`RecoveryError`] when the deadline or requested transition is invalid.
pub fn apply_human_decision(
    spec: &RecoveryPlanSpec,
    state: RecoveryPlanState,
    decision: HumanDecision,
    now: DateTime<Utc>,
) -> Result<RecoveryPlanState, RecoveryError> {
    if now >= spec.expires_at
        && matches!(
            state,
            RecoveryPlanState::Prepared | RecoveryPlanState::Approved { .. }
        )
    {
        return Err(RecoveryError::Expired);
    }
    match decision {
        HumanDecision::Approve { fingerprint } => {
            let canonical = canonical_recovery_fingerprint(spec);
            if fingerprint != canonical.as_str() {
                return Err(RecoveryError::FingerprintMismatch);
            }
            match state {
                RecoveryPlanState::Prepared => Ok(RecoveryPlanState::Approved {
                    approved_at: now,
                    approved_fingerprint: canonical,
                }),
                approved @ RecoveryPlanState::Approved { .. } => Ok(approved),
                _ => Err(RecoveryError::InvalidTransition),
            }
        }
        HumanDecision::Reject => match state {
            RecoveryPlanState::Prepared | RecoveryPlanState::Approved { .. } => {
                Ok(RecoveryPlanState::Rejected { rejected_at: now })
            }
            rejected @ RecoveryPlanState::Rejected { .. } => Ok(rejected),
            _ => Err(RecoveryError::InvalidTransition),
        },
    }
}

/// Invalidates authority after its scenario or session is superseded.
#[must_use]
pub fn invalidate_recovery(
    state: RecoveryPlanState,
    invalidated_at: DateTime<Utc>,
) -> RecoveryPlanState {
    match state {
        RecoveryPlanState::Prepared
        | RecoveryPlanState::Approved { .. }
        | RecoveryPlanState::Executing { .. } => RecoveryPlanState::Invalidated { invalidated_at },
        terminal => terminal,
    }
}

/// Authoritative facts reread inside the execution transaction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryExecutionFacts {
    pub session_id: SessionId,
    pub scenario_generation: i64,
    pub active_release: ReleaseId,
    pub target_release: ReleaseId,
    pub target_service_id: ServiceId,
    pub target_release_state: ReleaseState,
    pub stored_fingerprint: String,
}

/// Validates every execution precondition and claims an approved plan.
///
/// # Errors
///
/// Returns [`RecoveryError`] when any authority fact no longer matches the reviewed plan.
pub fn validate_execution(
    spec: &RecoveryPlanSpec,
    state: RecoveryPlanState,
    facts: &RecoveryExecutionFacts,
    now: DateTime<Utc>,
) -> Result<RecoveryPlanState, RecoveryError> {
    let (approved_at, approved_fingerprint) = match state {
        RecoveryPlanState::Approved {
            approved_at,
            approved_fingerprint,
        } => (approved_at, approved_fingerprint),
        RecoveryPlanState::Executed { .. } => return Err(RecoveryError::AlreadyExecuted),
        RecoveryPlanState::Invalidated { .. } => return Err(RecoveryError::Invalidated),
        RecoveryPlanState::Expired => return Err(RecoveryError::Expired),
        RecoveryPlanState::Prepared
        | RecoveryPlanState::Executing { .. }
        | RecoveryPlanState::Rejected { .. } => return Err(RecoveryError::NotApproved),
    };
    if facts.session_id != spec.session_id {
        return Err(RecoveryError::CrossSession);
    }
    if now >= spec.expires_at {
        return Err(RecoveryError::Expired);
    }
    if facts.scenario_generation != spec.scenario_generation {
        return Err(RecoveryError::StaleGeneration);
    }
    if facts.active_release != spec.expected_current_release {
        return Err(RecoveryError::StaleActiveRelease);
    }
    if facts.target_release != spec.target_release {
        return Err(RecoveryError::TargetReleaseMismatch);
    }
    if facts.target_service_id != spec.service_id {
        return Err(RecoveryError::TargetServiceMismatch);
    }
    if facts.target_release_state != ReleaseState::HealthyBaseline {
        return Err(RecoveryError::TargetIneligible);
    }
    let canonical = canonical_recovery_fingerprint(spec);
    if approved_fingerprint != canonical {
        return Err(RecoveryError::ApprovedFingerprintMismatch);
    }
    if facts.stored_fingerprint != canonical.as_str() {
        return Err(RecoveryError::StoredFingerprintMismatch);
    }

    Ok(RecoveryPlanState::Executing {
        approved_at,
        approved_fingerprint,
        execution_started_at: now,
    })
}

/// Marks a claimed execution complete.
///
/// # Errors
///
/// Returns [`RecoveryError::InvalidTransition`] unless the state is executing.
pub fn complete_execution(
    state: RecoveryPlanState,
    executed_at: DateTime<Utc>,
) -> Result<RecoveryPlanState, RecoveryError> {
    match state {
        RecoveryPlanState::Executing {
            approved_at,
            approved_fingerprint,
            execution_started_at,
        } if executed_at >= execution_started_at => Ok(RecoveryPlanState::Executed {
            approved_at,
            approved_fingerprint,
            execution_started_at,
            executed_at,
        }),
        _ => Err(RecoveryError::InvalidTransition),
    }
}

/// Persisted execution and after-state facts used to derive verification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryVerificationFacts {
    pub execution_plan_id: PlanId,
    pub scenario_generation: i64,
    pub previous_release: ReleaseId,
    pub current_release: ReleaseId,
    pub service_id: ServiceId,
    pub health_status: HealthStatus,
    pub incident_id: IncidentId,
    pub incident_status: IncidentStatus,
    pub telemetry_release: ReleaseId,
    pub telemetry_generation: i64,
    pub diagnostic_service_id: ServiceId,
    pub diagnostic_release: ReleaseId,
    pub diagnostic_generation: i64,
    pub diagnostic_status: DiagnosticStatus,
    pub stored_fingerprint: String,
}

/// A persisted fact that does not match the executed plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryVerificationMismatch {
    ExecutionPlan,
    ScenarioGeneration,
    PreviousRelease,
    CurrentRelease,
    Service,
    Health,
    Incident,
    IncidentStatus,
    TelemetryRelease,
    TelemetryGeneration,
    DiagnosticService,
    DiagnosticRelease,
    DiagnosticGeneration,
    DiagnosticStatus,
    Fingerprint,
}

/// Whether persisted facts prove the requested recovery.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecoveryVerificationOutcome {
    Passed,
    Mismatch {
        mismatches: Vec<RecoveryVerificationMismatch>,
    },
}

/// The before-and-after evidence returned by verification.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryVerificationBefore {
    pub release: ReleaseId,
    pub evidence: RecoveryEvidenceSet,
}

/// The persisted after-state returned by verification.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryVerificationAfter {
    pub release: ReleaseId,
    pub health_status: HealthStatus,
    pub incident_status: IncidentStatus,
    pub diagnostic_status: DiagnosticStatus,
}

/// Verification derived solely from the normalized plan and persisted effects.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryVerification {
    pub plan_id: PlanId,
    pub outcome: RecoveryVerificationOutcome,
    pub before: RecoveryVerificationBefore,
    pub after: RecoveryVerificationAfter,
    pub verified_at: DateTime<Utc>,
}

/// Derives a complete verification result from persisted facts.
///
/// # Errors
///
/// Returns [`RecoveryError::NotExecuted`] until execution has completed.
pub fn derive_verification(
    spec: &RecoveryPlanSpec,
    state: &RecoveryPlanState,
    facts: RecoveryVerificationFacts,
    verified_at: DateTime<Utc>,
) -> Result<RecoveryVerification, RecoveryError> {
    let RecoveryPlanState::Executed {
        approved_fingerprint,
        ..
    } = state
    else {
        return Err(RecoveryError::NotExecuted);
    };
    let mismatches = verification_mismatches(spec, approved_fingerprint, &facts);
    let outcome = if mismatches.is_empty() {
        RecoveryVerificationOutcome::Passed
    } else {
        RecoveryVerificationOutcome::Mismatch { mismatches }
    };
    Ok(RecoveryVerification {
        plan_id: spec.plan_id.clone(),
        outcome,
        before: RecoveryVerificationBefore {
            release: spec.expected_current_release.clone(),
            evidence: spec.evidence.clone(),
        },
        after: RecoveryVerificationAfter {
            release: facts.current_release,
            health_status: facts.health_status,
            incident_status: facts.incident_status,
            diagnostic_status: facts.diagnostic_status,
        },
        verified_at,
    })
}

fn verification_mismatches(
    spec: &RecoveryPlanSpec,
    approved_fingerprint: &RecoveryFingerprint,
    facts: &RecoveryVerificationFacts,
) -> Vec<RecoveryVerificationMismatch> {
    let mut mismatches = Vec::new();
    push_mismatch(
        &mut mismatches,
        facts.execution_plan_id != spec.plan_id,
        RecoveryVerificationMismatch::ExecutionPlan,
    );
    push_mismatch(
        &mut mismatches,
        facts.scenario_generation != spec.scenario_generation,
        RecoveryVerificationMismatch::ScenarioGeneration,
    );
    push_mismatch(
        &mut mismatches,
        facts.previous_release != spec.expected_current_release,
        RecoveryVerificationMismatch::PreviousRelease,
    );
    push_mismatch(
        &mut mismatches,
        facts.current_release != spec.target_release,
        RecoveryVerificationMismatch::CurrentRelease,
    );
    push_mismatch(
        &mut mismatches,
        facts.service_id != spec.service_id,
        RecoveryVerificationMismatch::Service,
    );
    push_mismatch(
        &mut mismatches,
        facts.health_status != HealthStatus::Healthy,
        RecoveryVerificationMismatch::Health,
    );
    push_mismatch(
        &mut mismatches,
        facts.incident_id != spec.incident_id,
        RecoveryVerificationMismatch::Incident,
    );
    push_mismatch(
        &mut mismatches,
        facts.incident_status != IncidentStatus::Resolved,
        RecoveryVerificationMismatch::IncidentStatus,
    );
    push_mismatch(
        &mut mismatches,
        facts.telemetry_release != spec.target_release,
        RecoveryVerificationMismatch::TelemetryRelease,
    );
    push_mismatch(
        &mut mismatches,
        facts.telemetry_generation != spec.scenario_generation,
        RecoveryVerificationMismatch::TelemetryGeneration,
    );
    push_mismatch(
        &mut mismatches,
        facts.diagnostic_service_id != spec.service_id,
        RecoveryVerificationMismatch::DiagnosticService,
    );
    push_mismatch(
        &mut mismatches,
        facts.diagnostic_release != spec.target_release,
        RecoveryVerificationMismatch::DiagnosticRelease,
    );
    push_mismatch(
        &mut mismatches,
        facts.diagnostic_generation != spec.scenario_generation,
        RecoveryVerificationMismatch::DiagnosticGeneration,
    );
    push_mismatch(
        &mut mismatches,
        facts.diagnostic_status != DiagnosticStatus::Passed,
        RecoveryVerificationMismatch::DiagnosticStatus,
    );
    let canonical = canonical_recovery_fingerprint(spec);
    push_mismatch(
        &mut mismatches,
        facts.stored_fingerprint != canonical.as_str() || approved_fingerprint != &canonical,
        RecoveryVerificationMismatch::Fingerprint,
    );
    mismatches
}

fn push_mismatch(
    mismatches: &mut Vec<RecoveryVerificationMismatch>,
    condition: bool,
    mismatch: RecoveryVerificationMismatch,
) {
    if condition {
        mismatches.push(mismatch);
    }
}

/// Recovery policy or authority validation failure.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RecoveryError {
    #[error("evidence identifier must not be empty")]
    InvalidEvidenceId,
    #[error("target release is not an eligible rollback")]
    InvalidTarget,
    #[error("scenario generation must be positive")]
    InvalidGeneration,
    #[error("recovery reason is empty or too long")]
    InvalidReason,
    #[error("recovery expiry overflows the timestamp range")]
    TimestampOverflow,
    #[error("evidence {0:?} is not supported by recovery policy")]
    UnsupportedEvidence(EvidenceId),
    #[error("evidence {0:?} is duplicated")]
    DuplicateEvidence(EvidenceId),
    #[error("required evidence kind {0:?} is missing")]
    MissingEvidence(RecoveryEvidenceKind),
    #[error("evidence kind {0:?} is ambiguous")]
    AmbiguousEvidence(RecoveryEvidenceKind),
    #[error("evidence {0:?} belongs to another session")]
    EvidenceSessionMismatch(EvidenceId),
    #[error("evidence {0:?} belongs to another service")]
    EvidenceServiceMismatch(EvidenceId),
    #[error("evidence {0:?} belongs to another release")]
    EvidenceReleaseMismatch(EvidenceId),
    #[error("evidence {0:?} belongs to another scenario generation")]
    EvidenceGenerationMismatch(EvidenceId),
    #[error("plan fingerprint does not match")]
    FingerprintMismatch,
    #[error("plan transition is not allowed")]
    InvalidTransition,
    #[error("plan is not approved")]
    NotApproved,
    #[error("plan has expired")]
    Expired,
    #[error("plan belongs to another session")]
    CrossSession,
    #[error("scenario generation is stale")]
    StaleGeneration,
    #[error("active release no longer matches the plan")]
    StaleActiveRelease,
    #[error("target release no longer matches the plan")]
    TargetReleaseMismatch,
    #[error("target release belongs to another service")]
    TargetServiceMismatch,
    #[error("target release is no longer eligible")]
    TargetIneligible,
    #[error("approved fingerprint does not match normalized facts")]
    ApprovedFingerprintMismatch,
    #[error("stored fingerprint does not match normalized facts")]
    StoredFingerprintMismatch,
    #[error("plan was invalidated")]
    Invalidated,
    #[error("plan has already executed")]
    AlreadyExecuted,
    #[error("plan has not completed execution")]
    NotExecuted,
}
