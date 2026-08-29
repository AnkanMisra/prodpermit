use std::{sync::Arc, time::Duration};

use axum::{
    Json, Router,
    extract::{
        DefaultBodyLimit, FromRequest, Path, Query, Request, State, rejection::JsonRejection,
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use recovery_domain::{
    AuditEvent, DiagnosticResult, HumanDecision, IncidentSnapshot, LogEvent, LogSeverity, PlanId,
    RecoveryError, RecoveryPlanState, RecoveryVerification, RecoveryVerificationAfter,
    RecoveryVerificationBefore, RecoveryVerificationOutcome, ReleaseComparison, ReleaseId,
    SessionId, seeded_scenario,
};
use recovery_persistence::{PersistedRecoveryPlan, RecoveryPreparation, Store, StoreError};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tower_http::{catch_panic::CatchPanicLayer, trace::TraceLayer};
use uuid::Uuid;

const SESSION_COOKIE_NAME: &str = "recovery_demo_session";

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub allowed_origin: String,
    pub secure_cookie: bool,
}

pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> DateTime<Utc>;
}

#[derive(Debug)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

#[derive(Clone)]
struct AppState {
    store: Store,
    config: AppConfig,
    clock: Arc<dyn Clock>,
}

#[derive(Debug, Error)]
enum ApiError {
    #[error("a demo session is required")]
    SessionRequired,
    #[error("the request origin is not allowed")]
    OriginNotAllowed,
    #[error("the demo session no longer exists")]
    SessionNotFound,
    #[error("request input is invalid")]
    InvalidInput,
    #[error("request body is too large")]
    PayloadTooLarge,
    #[error("persistence failed")]
    Store(#[from] StoreError),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorBody {
    code: &'static str,
    message: &'static str,
    request_id: Uuid,
    retryable: bool,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: ErrorBody,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message, retryable) = match self {
            Self::SessionRequired => (
                StatusCode::UNAUTHORIZED,
                "SESSION_REQUIRED",
                "Create a demo session before requesting incident data.",
                false,
            ),
            Self::OriginNotAllowed => (
                StatusCode::FORBIDDEN,
                "ORIGIN_NOT_ALLOWED",
                "The request origin is not allowed.",
                false,
            ),
            Self::SessionNotFound
            | Self::Store(StoreError::SessionNotFound | StoreError::SessionInactive) => (
                StatusCode::UNAUTHORIZED,
                "SESSION_NOT_FOUND",
                "The demo session no longer exists. Create a new session.",
                true,
            ),
            Self::InvalidInput => (
                StatusCode::BAD_REQUEST,
                "INVALID_INPUT",
                "The request input is invalid.",
                false,
            ),
            Self::PayloadTooLarge => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "PAYLOAD_TOO_LARGE",
                "The request body exceeds 32 KiB.",
                false,
            ),
            Self::Store(StoreError::Recovery(error)) => recovery_error_response(&error),
            Self::Store(StoreError::RecoveryNotFound) => (
                StatusCode::NOT_FOUND,
                "PLAN_NOT_FOUND",
                "The recovery plan was not found in this session.",
                false,
            ),
            Self::Store(StoreError::InvalidRecoveryEvidence) => (
                StatusCode::BAD_REQUEST,
                "INVALID_RECOVERY_EVIDENCE",
                "Recovery evidence is missing, unrelated, or unavailable.",
                false,
            ),
            Self::Store(StoreError::ActiveRecoveryExists) => (
                StatusCode::CONFLICT,
                "PLAN_ALREADY_ACTIVE",
                "Complete or invalidate the current recovery plan first.",
                false,
            ),
            Self::Store(StoreError::SessionCapacity) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "SESSION_CAPACITY_REACHED",
                "The demo is at session capacity. Retry after an existing session expires.",
                true,
            ),
            Self::Store(error) => {
                tracing::error!(error = ?error, "persistence request failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL_ERROR",
                    "The request could not be completed.",
                    true,
                )
            }
        };
        (
            status,
            Json(ErrorResponse {
                error: ErrorBody {
                    code,
                    message,
                    request_id: Uuid::new_v4(),
                    retryable,
                },
            }),
        )
            .into_response()
    }
}

struct ApiJson<T>(T);

impl<S, T> FromRequest<S> for ApiJson<T>
where
    S: Send + Sync,
    T: serde::de::DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        Json::<T>::from_request(request, state)
            .await
            .map(|Json(value)| Self(value))
            .map_err(|rejection: JsonRejection| {
                if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
                    ApiError::PayloadTooLarge
                } else {
                    ApiError::InvalidInput
                }
            })
    }
}

#[derive(Serialize)]
struct DataResponse<T> {
    data: T,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HealthResponse {
    status: &'static str,
}

pub fn build_router(store: Store, config: AppConfig) -> Router {
    build_router_with_clock(store, config, Arc::new(SystemClock))
}

pub fn build_router_with_clock(store: Store, config: AppConfig, clock: Arc<dyn Clock>) -> Router {
    let state = AppState {
        store,
        config,
        clock,
    };
    Router::new()
        .route("/api/health", get(health))
        .route("/api/demo/sessions", post(create_or_resume_session))
        .route("/api/demo/session/reset", post(reset_session))
        .route("/api/incidents/current", get(current_incident))
        .route("/api/releases/compare", get(compare_releases))
        .route("/api/logs", get(query_logs))
        .route("/api/diagnostics", post(run_diagnostic))
        .route("/api/recovery-plans", post(create_recovery_plan))
        .route("/api/recovery-plans/current", get(current_recovery_plan))
        .route(
            "/api/recovery-plans/{id}/approve",
            post(approve_recovery_plan),
        )
        .route(
            "/api/recovery-plans/{id}/reject",
            post(reject_recovery_plan),
        )
        .route(
            "/api/recovery-plans/{id}/execute",
            post(execute_recovery_plan),
        )
        .route("/api/recovery-plans/{id}/verify", get(verify_recovery))
        .route("/api/audit-events", get(audit_events))
        .layer(DefaultBodyLimit::max(32 * 1024))
        .layer(CatchPanicLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health() -> Json<DataResponse<HealthResponse>> {
    Json(DataResponse {
        data: HealthResponse { status: "ok" },
    })
}

async fn create_or_resume_session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    validate_mutation_headers(&headers, &state.config)?;

    if let Some(session_id) = session_id_from_headers(&headers)
        && let Some(snapshot) = state.store.load_snapshot(&session_id).await?
        && snapshot.session.expires_at > state.clock.now()
    {
        return Ok((StatusCode::OK, Json(DataResponse { data: snapshot })).into_response());
    }

    let snapshot = seeded_scenario(SessionId::new(), state.clock.now());
    state.store.create_session(&snapshot).await?;
    let cookie = session_cookie(&snapshot.session.id, state.config.secure_cookie);
    let mut response = (StatusCode::CREATED, Json(DataResponse { data: snapshot })).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).map_err(|_| ApiError::SessionNotFound)?,
    );
    Ok(response)
}

async fn current_incident(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<DataResponse<IncidentSnapshot>>, ApiError> {
    let session_id = required_active_session(&state, &headers).await?;
    let snapshot = state
        .store
        .load_snapshot(&session_id)
        .await?
        .ok_or(ApiError::SessionNotFound)?;
    if snapshot.session.expires_at <= state.clock.now() {
        return Err(ApiError::SessionNotFound);
    }
    Ok(Json(DataResponse { data: snapshot }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReleaseComparisonQuery {
    baseline_release: String,
    candidate_release: String,
}

async fn compare_releases(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ReleaseComparisonQuery>,
) -> Result<Json<DataResponse<ReleaseComparison>>, ApiError> {
    let session_id = required_active_session(&state, &headers).await?;
    let baseline = ReleaseId::parse(query.baseline_release).map_err(|_| ApiError::InvalidInput)?;
    let candidate =
        ReleaseId::parse(query.candidate_release).map_err(|_| ApiError::InvalidInput)?;
    let comparison = state
        .store
        .compare_releases(&session_id, &baseline, &candidate)
        .await?;
    Ok(Json(DataResponse { data: comparison }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LogQuery {
    severity: Option<LogSeverity>,
    #[serde(default = "default_window_minutes")]
    window_minutes: i64,
    #[serde(default = "default_log_limit")]
    limit: i64,
}

async fn query_logs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<LogQuery>,
) -> Result<Json<DataResponse<Vec<LogEvent>>>, ApiError> {
    if !(5..=60).contains(&query.window_minutes) || !(1..=25).contains(&query.limit) {
        return Err(ApiError::InvalidInput);
    }
    let session_id = required_active_session(&state, &headers).await?;
    let since = state.clock.now() - ChronoDuration::minutes(query.window_minutes);
    let logs = state
        .store
        .query_logs(&session_id, query.severity, since, query.limit)
        .await?;
    Ok(Json(DataResponse { data: logs }))
}

const fn default_window_minutes() -> i64 {
    30
}

const fn default_log_limit() -> i64 {
    20
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum DiagnosticKind {
    DatabaseConnectivity,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DiagnosticRequest {
    kind: DiagnosticKind,
}

async fn run_diagnostic(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<DiagnosticRequest>,
) -> Result<Json<DataResponse<DiagnosticResult>>, ApiError> {
    validate_mutation_headers(&headers, &state.config)?;
    let session_id = required_session_id(&headers)?;
    match request.kind {
        DiagnosticKind::DatabaseConnectivity => {
            tokio::time::sleep(Duration::from_millis(300)).await;
            let result = state
                .store
                .run_database_diagnostic(&session_id, state.clock.now())
                .await?;
            Ok(Json(DataResponse { data: result }))
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PrepareRecoveryRequest {
    target_release: String,
    reason: String,
    evidence_refs: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecoveryPlanView {
    plan_id: PlanId,
    session_id: SessionId,
    incident_id: recovery_domain::IncidentId,
    service_id: recovery_domain::ServiceId,
    current_release: ReleaseId,
    target_release: ReleaseId,
    expected_current_release: ReleaseId,
    scenario_generation: i64,
    reason: String,
    supporting_evidence: Vec<String>,
    risk_level: &'static str,
    preconditions: Vec<String>,
    fingerprint: String,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    approved_at: Option<DateTime<Utc>>,
    executed_at: Option<DateTime<Utc>>,
    status: &'static str,
}

impl From<PersistedRecoveryPlan> for RecoveryPlanView {
    fn from(plan: PersistedRecoveryPlan) -> Self {
        let (status, approved_at, executed_at) = plan_state_fields(&plan.state);
        let expected = plan.spec.expected_current_release().clone();
        Self {
            plan_id: plan.spec.plan_id().clone(),
            session_id: plan.spec.session_id().clone(),
            incident_id: plan.spec.incident_id().clone(),
            service_id: plan.spec.service_id().clone(),
            current_release: expected.clone(),
            target_release: plan.spec.target_release().clone(),
            expected_current_release: expected.clone(),
            scenario_generation: plan.spec.scenario_generation(),
            reason: plan.spec.reason().to_owned(),
            supporting_evidence: plan
                .spec
                .evidence()
                .iter()
                .map(|item| item.id().as_str().to_owned())
                .collect(),
            risk_level: "low",
            preconditions: vec![
                format!("The active release remains {}.", expected.as_str()),
                "The plan remains approved and unexpired.".to_owned(),
                "The target remains the known healthy baseline.".to_owned(),
            ],
            fingerprint: plan.fingerprint.as_str().to_owned(),
            created_at: plan.spec.created_at(),
            expires_at: plan.spec.expires_at(),
            approved_at,
            executed_at,
            status,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CurrentRecoveryView {
    plan: Option<RecoveryPlanView>,
    execution_capability: ExecutionCapabilityView,
}

#[derive(Serialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
enum ExecutionCapabilityView {
    Available {
        plan_id: PlanId,
        fingerprint: String,
        expires_at: DateTime<Utc>,
    },
    Absent {
        reason: &'static str,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecoveryVerificationView {
    plan_id: PlanId,
    outcome: RecoveryVerificationOutcome,
    previous_release: ReleaseId,
    current_release: ReleaseId,
    health_status: recovery_domain::HealthStatus,
    diagnostic_status: recovery_domain::DiagnosticStatus,
    before: RecoveryVerificationBefore,
    after: RecoveryVerificationAfter,
    verified_at: DateTime<Utc>,
}

impl From<RecoveryVerification> for RecoveryVerificationView {
    fn from(value: RecoveryVerification) -> Self {
        Self {
            plan_id: value.plan_id,
            outcome: value.outcome,
            previous_release: value.before.release.clone(),
            current_release: value.after.release.clone(),
            health_status: value.after.health_status,
            diagnostic_status: value.after.diagnostic.status,
            before: value.before,
            after: value.after,
            verified_at: value.verified_at,
        }
    }
}

async fn create_recovery_plan(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<PrepareRecoveryRequest>,
) -> Result<Response, ApiError> {
    validate_mutation_headers(&headers, &state.config)?;
    let session_id = required_session_id(&headers)?;
    let target = ReleaseId::parse(request.target_release).map_err(|_| ApiError::InvalidInput)?;
    let plan = state
        .store
        .prepare_recovery(
            &session_id,
            RecoveryPreparation {
                target_release: target,
                reason: request.reason,
                evidence_refs: request.evidence_refs,
            },
            state.clock.now(),
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(DataResponse {
            data: RecoveryPlanView::from(plan),
        }),
    )
        .into_response())
}

async fn current_recovery_plan(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<DataResponse<CurrentRecoveryView>>, ApiError> {
    let session_id = required_session_id(&headers)?;
    let plan = state
        .store
        .current_recovery(&session_id, state.clock.now())
        .await?;
    let execution_capability = match plan.as_ref() {
        Some(PersistedRecoveryPlan {
            spec,
            fingerprint,
            state: RecoveryPlanState::Approved { .. },
        }) => ExecutionCapabilityView::Available {
            plan_id: spec.plan_id().clone(),
            fingerprint: fingerprint.as_str().to_owned(),
            expires_at: spec.expires_at(),
        },
        Some(PersistedRecoveryPlan { state, .. }) => ExecutionCapabilityView::Absent {
            reason: capability_absence_reason(state),
        },
        None => ExecutionCapabilityView::Absent { reason: "no_plan" },
    };
    Ok(Json(DataResponse {
        data: CurrentRecoveryView {
            plan: plan.map(RecoveryPlanView::from),
            execution_capability,
        },
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovalRequest {
    fingerprint: String,
}

async fn approve_recovery_plan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<ApprovalRequest>,
) -> Result<Json<DataResponse<RecoveryPlanView>>, ApiError> {
    validate_mutation_headers(&headers, &state.config)?;
    let session_id = required_session_id(&headers)?;
    let plan_id = PlanId::parse(&id).map_err(|_| ApiError::InvalidInput)?;
    let plan = state
        .store
        .decide_recovery(
            &session_id,
            &plan_id,
            HumanDecision::Approve {
                fingerprint: request.fingerprint,
            },
            state.clock.now(),
        )
        .await?;
    Ok(Json(DataResponse {
        data: RecoveryPlanView::from(plan),
    }))
}

async fn reject_recovery_plan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<DataResponse<RecoveryPlanView>>, ApiError> {
    validate_mutation_headers(&headers, &state.config)?;
    let session_id = required_session_id(&headers)?;
    let plan_id = PlanId::parse(&id).map_err(|_| ApiError::InvalidInput)?;
    let plan = state
        .store
        .decide_recovery(
            &session_id,
            &plan_id,
            HumanDecision::Reject,
            state.clock.now(),
        )
        .await?;
    Ok(Json(DataResponse {
        data: RecoveryPlanView::from(plan),
    }))
}

async fn execute_recovery_plan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<DataResponse<RecoveryPlanView>>, ApiError> {
    validate_mutation_headers(&headers, &state.config)?;
    let session_id = required_session_id(&headers)?;
    let plan_id = PlanId::parse(&id).map_err(|_| ApiError::InvalidInput)?;
    let plan = state
        .store
        .execute_recovery(&session_id, &plan_id, state.clock.now())
        .await?;
    Ok(Json(DataResponse {
        data: RecoveryPlanView::from(plan),
    }))
}

async fn verify_recovery(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<DataResponse<RecoveryVerificationView>>, ApiError> {
    let session_id = required_session_id(&headers)?;
    let plan_id = PlanId::parse(&id).map_err(|_| ApiError::InvalidInput)?;
    let verification = state
        .store
        .verify_recovery(&session_id, &plan_id, state.clock.now())
        .await?;
    Ok(Json(DataResponse {
        data: RecoveryVerificationView::from(verification),
    }))
}

async fn reset_session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    validate_mutation_headers(&headers, &state.config)?;
    let session_id = required_session_id(&headers)?;
    let snapshot = state
        .store
        .reset_session(&session_id, state.clock.now())
        .await?;
    let cookie = session_cookie(&snapshot.session.id, state.config.secure_cookie);
    let mut response = (StatusCode::OK, Json(DataResponse { data: snapshot })).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).map_err(|_| ApiError::SessionNotFound)?,
    );
    Ok(response)
}

async fn audit_events(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<DataResponse<Vec<AuditEvent>>>, ApiError> {
    let session_id = required_active_session(&state, &headers).await?;
    let events = state.store.audit_events(&session_id, 100).await?;
    Ok(Json(DataResponse { data: events }))
}

fn validate_mutation_headers(headers: &HeaderMap, config: &AppConfig) -> Result<(), ApiError> {
    let origin_matches = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == config.allowed_origin);
    let marker_matches = headers
        .get("x-demo-request")
        .and_then(|value| value.to_str().ok())
        == Some("1");
    if origin_matches && marker_matches {
        Ok(())
    } else {
        Err(ApiError::OriginNotAllowed)
    }
}

fn session_id_from_headers(headers: &HeaderMap) -> Option<SessionId> {
    let cookie_header = headers.get(header::COOKIE)?.to_str().ok()?;
    let value = cookie_header.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name == SESSION_COOKIE_NAME).then_some(value)
    })?;
    Uuid::parse_str(value).ok().map(SessionId::from)
}

fn required_session_id(headers: &HeaderMap) -> Result<SessionId, ApiError> {
    session_id_from_headers(headers).ok_or(ApiError::SessionRequired)
}

async fn required_active_session(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<SessionId, ApiError> {
    let session_id = required_session_id(headers)?;
    if state
        .store
        .session_is_active(&session_id, state.clock.now())
        .await?
    {
        Ok(session_id)
    } else {
        Err(ApiError::SessionNotFound)
    }
}

fn session_cookie(session_id: &SessionId, secure: bool) -> String {
    let secure_attribute = if secure { "; Secure" } else { "" };
    format!(
        "{SESSION_COOKIE_NAME}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}{}",
        session_id.as_uuid(),
        Duration::from_hours(24).as_secs(),
        secure_attribute
    )
}

fn plan_state_fields(
    state: &RecoveryPlanState,
) -> (&'static str, Option<DateTime<Utc>>, Option<DateTime<Utc>>) {
    match state {
        RecoveryPlanState::Prepared => ("prepared", None, None),
        RecoveryPlanState::Approved { approved_at, .. } => ("approved", Some(*approved_at), None),
        RecoveryPlanState::Executing { approved_at, .. } => ("executing", Some(*approved_at), None),
        RecoveryPlanState::Executed {
            approved_at,
            executed_at,
            ..
        } => ("executed", Some(*approved_at), Some(*executed_at)),
        RecoveryPlanState::Rejected { .. } => ("rejected", None, None),
        RecoveryPlanState::Expired => ("expired", None, None),
        RecoveryPlanState::Invalidated { .. } => ("invalidated", None, None),
    }
}

fn capability_absence_reason(state: &RecoveryPlanState) -> &'static str {
    match state {
        RecoveryPlanState::Prepared => "not_approved",
        RecoveryPlanState::Approved { .. } => "available",
        RecoveryPlanState::Executing { .. }
        | RecoveryPlanState::Executed { .. }
        | RecoveryPlanState::Rejected { .. } => "terminal",
        RecoveryPlanState::Expired => "expired",
        RecoveryPlanState::Invalidated { .. } => "invalidated",
    }
}

fn recovery_error_response(
    error: &RecoveryError,
) -> (StatusCode, &'static str, &'static str, bool) {
    match error {
        RecoveryError::InvalidTarget => (
            StatusCode::BAD_REQUEST,
            "INVALID_TARGET_RELEASE",
            "The target release is not an eligible rollback.",
            false,
        ),
        RecoveryError::InvalidEvidenceId
        | RecoveryError::InvalidGeneration
        | RecoveryError::InvalidReason
        | RecoveryError::TimestampOverflow
        | RecoveryError::UnsupportedEvidence(_)
        | RecoveryError::DuplicateEvidence(_)
        | RecoveryError::MissingEvidence(_)
        | RecoveryError::TooManyEvidence
        | RecoveryError::EvidenceSessionMismatch(_)
        | RecoveryError::EvidenceServiceMismatch(_)
        | RecoveryError::EvidenceReleaseMismatch(_)
        | RecoveryError::EvidenceGenerationMismatch(_) => (
            StatusCode::BAD_REQUEST,
            "INVALID_PLAN_INPUT",
            "The recovery reason or evidence is invalid.",
            false,
        ),
        RecoveryError::FingerprintMismatch | RecoveryError::InvalidFingerprint => (
            StatusCode::CONFLICT,
            "PLAN_FINGERPRINT_MISMATCH",
            "The approval does not match the displayed recovery plan.",
            false,
        ),
        RecoveryError::NotApproved => (
            StatusCode::CONFLICT,
            "PLAN_NOT_APPROVED",
            "The recovery plan has not been approved.",
            false,
        ),
        RecoveryError::Expired => (
            StatusCode::CONFLICT,
            "PLAN_EXPIRED",
            "The recovery plan has expired.",
            false,
        ),
        RecoveryError::StaleGeneration
        | RecoveryError::ActiveIncidentMismatch
        | RecoveryError::IncidentNotActive
        | RecoveryError::StaleActiveRelease
        | RecoveryError::TargetReleaseMismatch
        | RecoveryError::TargetServiceMismatch
        | RecoveryError::TargetIneligible => (
            StatusCode::CONFLICT,
            "PLAN_STALE",
            "The active release no longer matches the approved plan.",
            false,
        ),
        RecoveryError::CrossSession => (
            StatusCode::NOT_FOUND,
            "PLAN_NOT_FOUND",
            "The recovery plan was not found in this session.",
            false,
        ),
        RecoveryError::AlreadyExecuted => (
            StatusCode::CONFLICT,
            "PLAN_ALREADY_EXECUTED",
            "The recovery plan has already executed.",
            false,
        ),
        RecoveryError::Invalidated => (
            StatusCode::CONFLICT,
            "PLAN_INVALIDATED",
            "The recovery plan is no longer valid.",
            false,
        ),
        RecoveryError::ApprovedFingerprintMismatch | RecoveryError::StoredFingerprintMismatch => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "RECOVERY_INTEGRITY_ERROR",
            "Stored recovery authority failed its integrity check.",
            true,
        ),
        RecoveryError::InvalidTransition | RecoveryError::NotExecuted => (
            StatusCode::CONFLICT,
            "INVALID_PLAN_TRANSITION",
            "The recovery plan cannot perform this transition.",
            false,
        ),
    }
}
