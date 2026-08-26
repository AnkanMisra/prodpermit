use std::time::Duration;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{Duration as ChronoDuration, Utc};
use recovery_domain::{
    AuditEvent, DiagnosticResult, IncidentSnapshot, LogEvent, LogSeverity, PlanError, PlanId,
    RecoveryPlan, RecoveryVerification, ReleaseComparison, ReleaseId, SessionId, seeded_scenario,
};
use recovery_persistence::{Store, StoreError};
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

#[derive(Clone, Debug)]
struct AppState {
    store: Store,
    config: AppConfig,
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
            Self::SessionNotFound => (
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
            Self::Store(StoreError::Plan(plan_error)) => plan_error_response(&plan_error),
            Self::Store(error) => {
                tracing::error!(error = %error, "persistence request failed");
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
    let state = AppState { store, config };
    Router::new()
        .route("/api/health", get(health))
        .route("/api/demo/sessions", post(create_or_resume_session))
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
        && snapshot.session.expires_at > Utc::now()
    {
        return Ok((StatusCode::OK, Json(DataResponse { data: snapshot })).into_response());
    }

    let snapshot = seeded_scenario(SessionId::new(), Utc::now());
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
    let session_id = session_id_from_headers(&headers).ok_or(ApiError::SessionRequired)?;
    let snapshot = state
        .store
        .load_snapshot(&session_id)
        .await?
        .ok_or(ApiError::SessionNotFound)?;
    if snapshot.session.expires_at <= Utc::now() {
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
    let session_id = required_session_id(&headers)?;
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
    let session_id = required_session_id(&headers)?;
    let since = Utc::now() - ChronoDuration::minutes(query.window_minutes);
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
    Json(request): Json<DiagnosticRequest>,
) -> Result<Json<DataResponse<DiagnosticResult>>, ApiError> {
    validate_mutation_headers(&headers, &state.config)?;
    let session_id = required_session_id(&headers)?;
    match request.kind {
        DiagnosticKind::DatabaseConnectivity => {
            tokio::time::sleep(Duration::from_millis(300)).await;
            let result = state
                .store
                .run_database_diagnostic(&session_id, Utc::now())
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

async fn create_recovery_plan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<PrepareRecoveryRequest>,
) -> Result<Response, ApiError> {
    validate_mutation_headers(&headers, &state.config)?;
    let session_id = required_session_id(&headers)?;
    let target = ReleaseId::parse(request.target_release).map_err(|_| ApiError::InvalidInput)?;
    let plan = state
        .store
        .create_recovery_plan(
            &session_id,
            target,
            request.reason,
            request.evidence_refs,
            Utc::now(),
        )
        .await?;
    Ok((StatusCode::CREATED, Json(DataResponse { data: plan })).into_response())
}

async fn current_recovery_plan(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<DataResponse<Option<RecoveryPlan>>>, ApiError> {
    let session_id = required_session_id(&headers)?;
    let plan = state.store.current_recovery_plan(&session_id).await?;
    Ok(Json(DataResponse { data: plan }))
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
    Json(request): Json<ApprovalRequest>,
) -> Result<Json<DataResponse<RecoveryPlan>>, ApiError> {
    validate_mutation_headers(&headers, &state.config)?;
    let session_id = required_session_id(&headers)?;
    let plan_id = PlanId::parse(&id).map_err(|_| ApiError::InvalidInput)?;
    let plan = state
        .store
        .approve_recovery_plan(&session_id, &plan_id, &request.fingerprint, Utc::now())
        .await?;
    Ok(Json(DataResponse { data: plan }))
}

async fn reject_recovery_plan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<DataResponse<RecoveryPlan>>, ApiError> {
    validate_mutation_headers(&headers, &state.config)?;
    let session_id = required_session_id(&headers)?;
    let plan_id = PlanId::parse(&id).map_err(|_| ApiError::InvalidInput)?;
    let plan = state
        .store
        .reject_recovery_plan(&session_id, &plan_id, Utc::now())
        .await?;
    Ok(Json(DataResponse { data: plan }))
}

async fn execute_recovery_plan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<DataResponse<RecoveryPlan>>, ApiError> {
    validate_mutation_headers(&headers, &state.config)?;
    let session_id = required_session_id(&headers)?;
    let plan_id = PlanId::parse(&id).map_err(|_| ApiError::InvalidInput)?;
    let plan = state
        .store
        .execute_recovery_plan(&session_id, &plan_id, Utc::now())
        .await?;
    Ok(Json(DataResponse { data: plan }))
}

async fn verify_recovery(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<DataResponse<RecoveryVerification>>, ApiError> {
    let session_id = required_session_id(&headers)?;
    let plan_id = PlanId::parse(&id).map_err(|_| ApiError::InvalidInput)?;
    let verification = state
        .store
        .verify_recovery(&session_id, &plan_id, Utc::now())
        .await?;
    Ok(Json(DataResponse { data: verification }))
}

async fn audit_events(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<DataResponse<Vec<AuditEvent>>>, ApiError> {
    let session_id = required_session_id(&headers)?;
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

fn session_cookie(session_id: &SessionId, secure: bool) -> String {
    let secure_attribute = if secure { "; Secure" } else { "" };
    format!(
        "{SESSION_COOKIE_NAME}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}{}",
        session_id.as_uuid(),
        Duration::from_hours(24).as_secs(),
        secure_attribute
    )
}

fn plan_error_response(error: &PlanError) -> (StatusCode, &'static str, &'static str, bool) {
    match error {
        PlanError::InvalidTarget => (
            StatusCode::BAD_REQUEST,
            "INVALID_TARGET_RELEASE",
            "The target release is not an eligible rollback.",
            false,
        ),
        PlanError::InvalidInput => (
            StatusCode::BAD_REQUEST,
            "INVALID_PLAN_INPUT",
            "The recovery reason or evidence is invalid.",
            false,
        ),
        PlanError::FingerprintMismatch => (
            StatusCode::CONFLICT,
            "PLAN_FINGERPRINT_MISMATCH",
            "The approval does not match the displayed recovery plan.",
            false,
        ),
        PlanError::NotApproved => (
            StatusCode::CONFLICT,
            "PLAN_NOT_APPROVED",
            "The recovery plan has not been approved.",
            false,
        ),
        PlanError::Expired => (
            StatusCode::CONFLICT,
            "PLAN_EXPIRED",
            "The recovery plan has expired.",
            false,
        ),
        PlanError::Stale => (
            StatusCode::CONFLICT,
            "PLAN_STALE",
            "The active release no longer matches the approved plan.",
            false,
        ),
        PlanError::CrossSession => (
            StatusCode::FORBIDDEN,
            "PLAN_SESSION_MISMATCH",
            "The recovery plan does not belong to this session.",
            false,
        ),
        PlanError::AlreadyExecuted => (
            StatusCode::CONFLICT,
            "PLAN_ALREADY_EXECUTED",
            "The recovery plan has already executed.",
            false,
        ),
        PlanError::InvalidTransition => (
            StatusCode::CONFLICT,
            "INVALID_PLAN_TRANSITION",
            "The recovery plan cannot perform this transition.",
            false,
        ),
    }
}
