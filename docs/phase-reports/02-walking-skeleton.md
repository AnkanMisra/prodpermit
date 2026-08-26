# Phase 2: Walking skeleton

## Outcome

The browser now displays a critical `checkout-api` incident returned by the Rust service and stored in SQLite. The local workflow runs through Bun, and both deployment containers build.

## Files changed

- Added Rust domain types and a deterministic 30-minute incident scenario.
- Added normalized session, service, incident, release, and telemetry tables.
- Added the Axum health, session, and current-incident routes.
- Added the strict TypeScript contracts, API client, dark operational dashboard, and accessible telemetry chart.
- Added Next.js headers and the same-origin backend rewrite.
- Added Dockerfiles, Compose configuration, component tests, API tests, and a Chromium test.
- Kept the generated nested Next.js `AGENTS.md` because Next.js 16 uses it to route agents to version-matched local documentation.

## Decisions

- Store each complete scenario under one opaque cookie-bound session.
- Keep the first UI state small enough to verify before adding WebMCP registration.
- Use TypeScript 6.0.3 because the installed TypeScript ESLint parser does not support TypeScript 7.
- Use Bun for installation, scripts, and the web container.

## Verification

The following checks passed:

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
bun install --frozen-lockfile
bun run typecheck
bun run lint
bun run test:web
bun run build:web
bun run test:e2e
docker compose config
docker compose build
git diff --check
```

Rust has three passing behavior tests. Vitest has one passing dashboard test. Playwright loaded the real page in Chromium, checked the WebMCP headers, found no browser console errors, and saved `output/playwright/phase-2-walking-skeleton.png`.

## Remaining work

Release comparison, logs, diagnostics, WebMCP registration, recovery plans, approval, execution, verification, reset, security review, deployment, and submission assets remain.

## Risks and blockers

- The local Git checkpoint remains blocked by missing author configuration.
- The `agent-browser` executable was unavailable, so Playwright performed the required real-browser check.
- The real WebMCP client check remains a later release gate.

## Next phase

Implement the investigation data and six initial WebMCP tools. Keep `execute_approved_recovery` absent.
