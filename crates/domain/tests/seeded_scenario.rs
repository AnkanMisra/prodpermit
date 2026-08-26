use chrono::{TimeZone, Utc};
use recovery_domain::{HealthStatus, IncidentStatus, ReleaseState, SessionId, seeded_scenario};

#[test]
fn seeded_scenario_starts_with_a_consistent_broken_release() {
    let now = Utc
        .with_ymd_and_hms(2026, 8, 26, 5, 0, 0)
        .single()
        .expect("fixture timestamp is valid");
    let session_id = SessionId::new();

    let scenario = seeded_scenario(session_id.clone(), now);

    assert_eq!(scenario.session.id, session_id);
    assert_eq!(scenario.incident.status, IncidentStatus::Active);
    assert_eq!(scenario.health.status, HealthStatus::Critical);
    assert_eq!(scenario.health.current_release.as_str(), "release_284");
    assert!((scenario.health.error_rate_percent - 18.7).abs() < f64::EPSILON);
    assert_eq!(scenario.health.p95_latency_ms, 1_420);
    assert_eq!(scenario.releases.len(), 3);
    assert_eq!(scenario.telemetry.len(), 30);

    let baseline = scenario
        .releases
        .iter()
        .find(|release| release.id.as_str() == "release_283")
        .expect("baseline release exists");
    assert_eq!(baseline.state, ReleaseState::HealthyBaseline);

    let current = scenario
        .releases
        .iter()
        .find(|release| release.id.as_str() == "release_284")
        .expect("current release exists");
    assert_eq!(current.state, ReleaseState::DeployedFaulty);
    assert_eq!(
        current.deployed_at,
        Some(now - chrono::Duration::minutes(12))
    );

    assert!(scenario.telemetry[17].error_rate_percent < scenario.telemetry[29].error_rate_percent);
}
