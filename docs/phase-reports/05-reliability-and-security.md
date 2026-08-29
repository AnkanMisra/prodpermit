# Phase 5: Reliability and security

## Outcome

The reliability and security pass reviewed the complete repository against `docs/THREAT_MODEL.md`. A sealed standard Codex Security scan recorded three validated findings in the Phase 4 snapshot. The current tree fixes all three findings and includes focused regression tests.

## Scan

- Scan ID: `cdcd1165-48d1-4526-9e63-9df47b9aff3c`
- Coverage: complete, 10 security surfaces
- Findings: two medium and one low
- Report: `/tmp/codex-security-scans-z6aIde/webmcp-project/ee0451eefe01630ce5bff9ac9d2fd510dc6ee6a7_20260829T060842Z_r1yc0qm8/report.md`
- Execution mode: parent-only fallback because the participant requested direct work without implementation or review agents
- TAC advisory: unavailable because the access connector was not connected

The scan recorded the original snapshot. It also warned that the working tree changed while fixes were applied. The fix verification below uses the amended source and focused tests.

## Findings and fixes

### Revoked reset cookie recovered replacement authority

The reset lineage lookup returned the active replacement before it checked the old session. The reset operation now requires an active original session. A revoked cookie receives `SESSION_NOT_FOUND` and cannot recover the successor cookie.

Verification:

```text
cargo test -p recovery-persistence --test session_store reset_revokes_old_authority_without_reissuing_the_replacement -- --exact
```

### Revoked sessions retained investigation read access

Release comparison, log, and audit routes used only the parsed session ID. They now call one active-session boundary that checks both expiration and revocation before every session-scoped read.

Verification:

```text
cargo test -p recovery-api --test walking_skeleton recovery_capability_is_session_bound_and_reset_revokes_old_cookie -- --exact
```

### Anonymous session creation allowed unbounded SQLite growth

Session creation now prunes expired and revoked sessions before allocation and caps active sessions at 256. The API returns retryable `503 SESSION_CAPACITY_REACHED` at the ceiling. Hosting-layer traffic controls remain a deployment requirement for burst abuse.

Verification:

```text
cargo test -p recovery-persistence --test session_store session_creation_bounds_growth_and_prunes_expired_rows -- --exact
```

## Additional checks

The following commands passed after remediation:

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
bun audit --production
credential-pattern scan with ripgrep
```

`bun audit --production` reported no vulnerabilities. The credential-pattern scan found no private keys or common provider-token formats. Cargo advisory tooling is not installed, so Phase 6 records that limitation rather than claiming a Rust advisory-database result.

## Residual assumptions

- The public host must rate-limit anonymous session creation. The application ceiling bounds storage but cannot prevent every availability attack.
- Production must set `SECURE_COOKIE=true` and an exact `ALLOWED_ORIGIN`.
- SQLite remains a single-instance deployment constraint.
- WebMCP annotations classify untrusted data but do not control model behavior.
