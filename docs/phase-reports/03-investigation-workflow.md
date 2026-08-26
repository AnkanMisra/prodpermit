# Phase 3: Investigation workflow

## Outcome

A WebMCP-aware browser can inspect the incident, compare releases, query bounded logs, and run the database diagnostic. The page updates with every result and never exposes an execution capability.

## Files changed

- Added release configuration, structured logs, diagnostic results, and audit-event tables.
- Added the internally consistent configuration regression and prompt-injection-shaped customer log.
- Added release comparison, log query, and diagnostic API routes.
- Added a typed, idempotent WebMCP registry with abort-based cleanup and activity reporting.
- Added four investigative tools and the release, log, diagnostic, and tool-inspector panels.
- Extended Playwright with a standards-shaped browser adapter that invokes the registered callbacks.

## Decisions

- Annotate the complete log tool with `untrustedContentHint` because annotation is currently tool-level.
- Return only one changed configuration field. Secret configuration remains redacted and unchanged.
- Run diagnostics inline. Aborting the browser fetch drops the Rust handler before it stores a completed result.
- Add preparation and verification tools with the recovery backend in Phase 4 rather than registering nonfunctional capabilities.

## Verification

The following checks passed:

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
bun run typecheck
bun run lint
bun run test:web
bun run build:web
bun run test:e2e
git diff --check
```

Rust now has five passing behavior tests. Vitest has four passing tests. Playwright has two passing Chromium journeys. The investigation journey called all four tools, displayed `database.auth_mode`, returned authentication failures plus the untrusted customer log, failed the database diagnostic for `release_284`, and confirmed that `execute_approved_recovery` is absent.

Visual evidence is stored at `output/playwright/phase-3-investigation.png`.

## Remaining work

Recovery planning, exact approval, rejection, dynamic execution registration, atomic recovery, verification, reset, security review, deployment, and submission preparation remain.

## Risks and blockers

- WebMCP is still tested through a Playwright adapter. A real supported client remains a release gate.
- The local Git checkpoint remains blocked by missing author configuration.

## Next phase

Implement the recovery-plan state machine and the exact human approval boundary.
