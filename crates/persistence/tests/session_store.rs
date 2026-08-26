use chrono::{TimeZone, Utc};
use recovery_domain::{
    DiagnosticStatus, HealthStatus, LogSeverity, ReleaseId, SessionId, seeded_scenario,
};
use recovery_persistence::Store;

#[tokio::test]
async fn session_store_round_trips_the_seeded_incident() {
    let store = Store::connect("sqlite::memory:")
        .await
        .expect("in-memory store connects");
    let now = Utc
        .with_ymd_and_hms(2026, 8, 26, 5, 0, 0)
        .single()
        .expect("fixture timestamp is valid");
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
            now - chrono::Duration::minutes(30),
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
