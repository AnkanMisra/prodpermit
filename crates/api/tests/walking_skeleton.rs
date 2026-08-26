use axum::{Router, body::Body, http::Request, response::Response};
use http::{StatusCode, header};
use http_body_util::BodyExt;
use recovery_api::{AppConfig, build_router};
use recovery_persistence::Store;
use serde_json::Value;
use tower::ServiceExt;

#[tokio::test]
async fn session_creation_exposes_the_broken_incident() {
    let app = test_app().await;
    let health = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .expect("health request builds"),
        )
        .await
        .expect("health request succeeds");
    assert_eq!(health.status(), StatusCode::OK);

    let cookie = create_session(&app).await;
    let incident = app
        .oneshot(
            Request::builder()
                .uri("/api/incidents/current")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .expect("incident request builds"),
        )
        .await
        .expect("incident request succeeds");
    assert_eq!(incident.status(), StatusCode::OK);
    let payload = response_json(incident).await;

    assert_eq!(payload["data"]["incident"]["status"], "active");
    assert_eq!(payload["data"]["health"]["status"], "critical");
    assert_eq!(payload["data"]["health"]["currentRelease"], "release_284");
    assert_eq!(
        payload["data"]["telemetry"].as_array().map(Vec::len),
        Some(30)
    );
}

#[tokio::test]
async fn investigation_endpoints_expose_the_authentication_regression() {
    let app = test_app().await;
    let cookie = create_session(&app).await;

    let comparison = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/releases/compare?baselineRelease=release_283&candidateRelease=release_284")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .expect("comparison request builds"),
        )
        .await
        .expect("comparison request succeeds");
    assert_eq!(comparison.status(), StatusCode::OK);
    let comparison_payload = response_json(comparison).await;
    assert_eq!(
        comparison_payload["data"]["configurationDiff"][0]["key"],
        "database.auth_mode"
    );

    let logs = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/logs?severity=error&limit=25")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .expect("log request builds"),
        )
        .await
        .expect("log request succeeds");
    assert_eq!(logs.status(), StatusCode::OK);
    let logs_payload = response_json(logs).await;
    assert_eq!(logs_payload["data"].as_array().map(Vec::len), Some(6));

    let diagnostic = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/diagnostics")
                .header(header::ORIGIN, "http://localhost:3000")
                .header(header::COOKIE, cookie)
                .header(header::CONTENT_TYPE, "application/json")
                .header("x-demo-request", "1")
                .body(Body::from(r#"{"kind":"database_connectivity"}"#))
                .expect("diagnostic request builds"),
        )
        .await
        .expect("diagnostic request succeeds");
    assert_eq!(diagnostic.status(), StatusCode::OK);
    let diagnostic_payload = response_json(diagnostic).await;
    assert_eq!(diagnostic_payload["data"]["status"], "failed");
    assert_eq!(
        diagnostic_payload["data"]["code"],
        "DB_AUTH_METHOD_MISMATCH"
    );
}

#[tokio::test]
async fn recovery_requires_exact_approval_and_rejects_replay() {
    let app = test_app().await;
    let cookie = create_session(&app).await;
    let diagnostic = mutation_request(
        &app,
        &cookie,
        "/api/diagnostics",
        r#"{"kind":"database_connectivity"}"#,
    )
    .await;
    let diagnostic_payload = response_json(diagnostic).await;
    let diagnostic_id = diagnostic_payload["data"]["id"]
        .as_str()
        .expect("diagnostic id is text");
    let prepare_body = serde_json::json!({
        "targetRelease": "release_283",
        "reason": "Rollback the database authentication regression.",
        "evidenceRefs": ["log_db_auth_1", diagnostic_id]
    })
    .to_string();
    let prepared = mutation_request(&app, &cookie, "/api/recovery-plans", &prepare_body).await;
    assert_eq!(prepared.status(), StatusCode::CREATED);
    let prepared_payload = response_json(prepared).await;
    let plan_id = prepared_payload["data"]["planId"]
        .as_str()
        .expect("plan id is text");
    let fingerprint = prepared_payload["data"]["fingerprint"]
        .as_str()
        .expect("fingerprint is text");

    let unapproved = mutation_request(
        &app,
        &cookie,
        &format!("/api/recovery-plans/{plan_id}/execute"),
        "{}",
    )
    .await;
    assert_eq!(unapproved.status(), StatusCode::CONFLICT);

    let approve_body = serde_json::json!({ "fingerprint": fingerprint }).to_string();
    let approved = mutation_request(
        &app,
        &cookie,
        &format!("/api/recovery-plans/{plan_id}/approve"),
        &approve_body,
    )
    .await;
    assert_eq!(approved.status(), StatusCode::OK);

    let executed = mutation_request(
        &app,
        &cookie,
        &format!("/api/recovery-plans/{plan_id}/execute"),
        "{}",
    )
    .await;
    assert_eq!(executed.status(), StatusCode::OK);
    let executed_payload = response_json(executed).await;
    assert_eq!(executed_payload["data"]["status"], "executed");

    let replay = mutation_request(
        &app,
        &cookie,
        &format!("/api/recovery-plans/{plan_id}/execute"),
        "{}",
    )
    .await;
    assert_eq!(replay.status(), StatusCode::CONFLICT);

    let verified = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/recovery-plans/{plan_id}/verify"))
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .expect("verification request builds"),
        )
        .await
        .expect("verification request succeeds");
    assert_eq!(verified.status(), StatusCode::OK);
    let verified_payload = response_json(verified).await;
    assert_eq!(verified_payload["data"]["currentRelease"], "release_283");
    assert_eq!(verified_payload["data"]["healthStatus"], "healthy");

    let audit = app
        .oneshot(
            Request::builder()
                .uri("/api/audit-events")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .expect("audit request builds"),
        )
        .await
        .expect("audit request succeeds");
    assert_eq!(audit.status(), StatusCode::OK);
    let audit_payload = response_json(audit).await;
    assert_eq!(audit_payload["data"].as_array().map(Vec::len), Some(4));
}

async fn test_app() -> Router {
    let store = Store::connect("sqlite::memory:")
        .await
        .expect("in-memory store connects");
    build_router(
        store,
        AppConfig {
            allowed_origin: "http://localhost:3000".to_owned(),
            secure_cookie: false,
        },
    )
}

async fn create_session(app: &Router) -> String {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/demo/sessions")
                .header(header::ORIGIN, "http://localhost:3000")
                .header("x-demo-request", "1")
                .body(Body::empty())
                .expect("session request builds"),
        )
        .await
        .expect("session request succeeds");
    assert_eq!(response.status(), StatusCode::CREATED);
    response
        .headers()
        .get(header::SET_COOKIE)
        .expect("session response sets a cookie")
        .to_str()
        .expect("cookie is valid text")
        .split(';')
        .next()
        .expect("cookie contains a value")
        .to_owned()
}

async fn response_json(response: Response) -> Value {
    let body = response
        .into_body()
        .collect()
        .await
        .expect("response body reads")
        .to_bytes();
    serde_json::from_slice(&body).expect("response body is JSON")
}

async fn mutation_request(app: &Router, cookie: &str, uri: &str, body: &str) -> Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header(header::ORIGIN, "http://localhost:3000")
                .header(header::COOKIE, cookie)
                .header(header::CONTENT_TYPE, "application/json")
                .header("x-demo-request", "1")
                .body(Body::from(body.to_owned()))
                .expect("mutation request builds"),
        )
        .await
        .expect("mutation request succeeds")
}
