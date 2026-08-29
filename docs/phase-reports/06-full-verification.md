# Phase 6: Full verification

## Outcome

The final local release gate passes. The repository builds and tests in Rust, Bun, Next.js, Playwright, Docker, and native Chrome WebMCP. No deployment, publication, push, upload, or submission action occurred.

## Rust gate

The following commands passed on August 29, 2026:

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The workspace passed 26 behavior tests:

- 5 Axum integration tests
- 12 domain and scenario tests
- 9 file-backed and in-memory persistence tests

The persistence tests cover normalized migrations, foreign keys, evidence, fingerprints, deadline expiry, audit rollback, concurrent execution, persisted verification facts, reset revocation, retention cleanup, and session capacity.

## Web gate

The following commands passed:

```text
bun install --frozen-lockfile
bun run typecheck
bun run lint
bun run test:web
bun run build:web
bun run test:e2e
```

Bun verified 482 installs across 579 packages without changing the lockfile. Vitest passed 10 tests. The Next.js 16.3.3 production build passed. Playwright passed five Chromium journeys with no application console errors:

- Broken incident rendering and security headers
- Investigation through registered WebMCP tools
- Telemetry-chart containment
- Exact prepare, approve, execute, verify, and reset journey
- Approval restoration and revocation across reload

Playwright now starts and waits for the Rust and Next.js servers independently. This removes the cold-build race where the frontend became ready before the API.

## Native WebMCP gate

Chrome for Testing 151.0.7922.34 ran against the real HTTP application origin with:

```text
--enable-experimental-web-platform-features
--enable-features=WebMCPTesting,DevToolsWebMCPSupport
```

The native `document.modelContext` API discovered these six initial tools:

```text
compare_releases
inspect_incident
prepare_recovery
query_logs
run_diagnostic
verify_recovery
```

Native `executeTool` invoked the actual page callbacks. The verified lifecycle was:

```text
diagnostic=failed
prepared=prepared
execute_approved_recovery absent before approval
execute_approved_recovery present after the human approval click
executed=executed
execute_approved_recovery absent after execution
verification=passed
currentRelease=release_283
```

The native run exposed two browser-compatibility defects before passing. The registry now supplies a fallback execution signal when Chrome omits callback options and defers cleanup by one task so React development remounts do not remove a valid native registration.

## Container gate

The following commands passed:

```text
docker compose config
docker compose build
docker compose build web
```

The final local images are `webmcp-project-api:latest` and `webmcp-project-web:latest`. They were not pushed or started as deployed services. Compose emitted one local tooling warning because Docker Bake is configured but `buildx` is not installed; the standard Docker builder completed both images.

## Security and repository checks

- The sealed standard security scan recorded three findings. All three current-tree fixes passed static fix verification and focused regression tests.
- `bun audit --production` reported no vulnerabilities.
- The credential-pattern scan found no private keys or common provider token formats.
- `git diff --check` passed.
- Cargo advisory tooling is not installed, so no RustSec database result is claimed.

## Visual evidence

`output/playwright/phase-4-recovery.png` was inspected at full resolution. It shows the resolved incident, healthy `release_283`, full fingerprint, supporting evidence, absent post-execution capability, persisted verification details, and four recovery audit events. The full-page screenshot renderer displays the off-canvas skip link once during stitching; the link remains hidden during normal unfocused use and is available to keyboard users.

## Remaining human-controlled work

Phase 7 remains pending. The participant must authorize hosting credentials, deployment, public product naming, and any Devpost preparation or submission. The official rules acknowledgment also remains pending.
