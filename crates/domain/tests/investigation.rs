use chrono::{TimeZone, Utc};
use recovery_domain::{
    DiagnosticStatus, LogSeverity, ReleaseId, compare_release_configuration,
    database_connectivity_diagnostic, seeded_investigation_data,
};

#[test]
fn seeded_evidence_identifies_the_authentication_regression() {
    let now = Utc
        .with_ymd_and_hms(2026, 8, 26, 5, 0, 0)
        .single()
        .expect("fixture timestamp is valid");
    let evidence = seeded_investigation_data(now);

    let comparison = compare_release_configuration(
        &evidence.release_configuration,
        &ReleaseId::from_static("release_283"),
        &ReleaseId::from_static("release_284"),
    );
    assert_eq!(comparison.len(), 1);
    assert_eq!(comparison[0].key, "database.auth_mode");
    assert_eq!(comparison[0].baseline_value, "scram-sha-256");
    assert_eq!(comparison[0].candidate_value, "password");

    let auth_failures = evidence
        .logs
        .iter()
        .filter(|event| event.code == "DB_AUTH_METHOD_MISMATCH")
        .count();
    assert_eq!(auth_failures, 6);
    assert!(
        evidence
            .logs
            .iter()
            .any(|event| event.untrusted && event.severity == LogSeverity::Warn)
    );

    let diagnostic = database_connectivity_diagnostic(&ReleaseId::from_static("release_284"), now);
    assert_eq!(diagnostic.status, DiagnosticStatus::Failed);
    assert_eq!(diagnostic.code, "DB_AUTH_METHOD_MISMATCH");
    assert!(diagnostic.evidence.contains("scram-sha-256"));
}
