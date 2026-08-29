# Phase 4: Recovery lifecycle

## Outcome

The browser agent can prepare an exact rollback from related evidence. A human can approve or reject the plan. Rust exposes one execution capability only for an approved, unexpired plan and performs the complete recovery in one SQLite transaction.

Execution changes `checkout-api` from `release_284` to `release_283`, resolves the incident, writes healthy telemetry, stores a passing database diagnostic, and records the audit event. Verification reads those persisted facts. Reset revokes the old session and creates one retry-safe replacement scenario.

## Domain and persistence

- Added a normalized recovery domain with typed evidence, lifecycle states, invalidation reasons, and persisted-fact verification.
- Replaced delimiter-based hashing with versioned, length-prefixed SHA-256 input.
- Added normalized recovery evidence, execution links, diagnostic context, service-release ownership, session revocation, and reset lineage.
- Coupled every lifecycle mutation to its audit write in one transaction.
- Made concurrent execution converge on one success and one replay error.

## API and browser

- Added the reset route and safe cookie rotation.
- Added a 32 KiB mutation-body limit and structured JSON rejection errors.
- Returned an explicit Rust-owned execution-capability envelope.
- Registered, replaced, restored, expired, revoked, and removed the execution tool from server state.
- Displayed the full fingerprint, supporting evidence, current authority state, and persisted verification details.
- Added focus movement and live incident status updates.

## Verification

The following commands passed on August 29, 2026:

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
bun run typecheck
bun run lint
bun run test:web
bun run build:web
bun run test:e2e
```

Rust passed 25 behavior tests: 5 API, 12 domain and scenario, and 8 persistence tests. Vitest passed 8 tests. Playwright passed 5 Chromium journeys, including the complete recovery, reset, reload restoration, and approval revocation paths.

## Remaining work

Phase 5 must extend the abuse and isolation matrix, run the standard security scan, validate its findings, and fix every confirmed issue. Phase 6 must rerun the complete local release gate, including container and secret checks.

## Blockers

No engineering blocker is active. Git checkpoints remain unavailable because author identity is not configured. The real supported WebMCP-client check remains a Phase 6 human-assisted gate.
