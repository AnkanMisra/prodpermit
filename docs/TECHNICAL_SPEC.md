# Technical specification

## System shape

The browser loads a Next.js App Router application from Vercel. The frontend calls `/api/backend/*`. A Next.js external rewrite sends those requests to the Rust service on Render. The browser therefore uses one visible origin and stores the Rust session cookie against the frontend origin.

The Rust service uses one SQLite database and one service instance. Each session owns a complete scenario copy. A reset creates a new session instead of rewriting shared global state.

## Workspace

- `apps/web` contains the Next.js UI, API client, WebMCP registry, component tests, and Playwright configuration.
- `crates/domain` contains domain types and pure state-transition functions.
- `crates/persistence` contains SQLx migrations, repositories, seed transactions, and execution transactions.
- `crates/api` contains Axum routes, extractors, validation, middleware, configuration, and tracing.
- `tests` contains cross-system fixtures and WebMCP evaluation cases.

## Database

Use migrations for these tables:

- `demo_sessions`
- `services`
- `incidents`
- `releases`
- `telemetry_points`
- `log_events`
- `diagnostic_results`
- `recovery_plans`
- `recovery_plan_evidence`
- `audit_events`

Enable foreign keys, WAL mode, and a busy timeout. Use a file-backed temporary database for integration tests.

Store timestamps as UTC RFC 3339 strings. Store enums as checked text values. Keep plan evidence in a child table so the plan fingerprint uses an ordered canonical representation.

## Domain types

Use newtypes for `SessionId`, `IncidentId`, `ServiceId`, `ReleaseId`, `PlanId`, and `EvidenceId`.

Use enums for incident status, health status, release state, diagnostic kind, risk level, plan status, log severity, and audit outcome.

Pure transition functions receive `now` as an argument. Tests control time without a global clock.

## Plan creation

The request contains `targetRelease`, `reason`, and `evidenceRefs`.

The backend loads the session's active incident and release. It verifies that the target is a prior known-healthy release for the same service. It resolves evidence references from the same session and requires the database-authentication failure plus the database diagnostic.

The backend creates a ten-minute plan, calculates a canonical fingerprint, and stores the immutable fields in one transaction.

## Atomic execution

Execution receives only a plan ID. The transaction performs these checks before any state change:

1. The plan exists in the cookie-bound session.
2. The status is `approved`.
3. The current time is before `expires_at`.
4. The scenario generation still matches.
5. The active release equals `expected_current_release`.
6. The target release belongs to the service and is an eligible rollback target.
7. The stored fingerprint still matches the canonical plan.

The transaction conditionally moves the plan to `executing`, changes the active release, writes healthy telemetry and diagnostic state, resolves the incident, writes audit events, and marks the plan `executed`.

Any failed check rolls back the transaction. A second execution observes a terminal plan and returns `PLAN_ALREADY_EXECUTED`.

## API contracts

All success responses contain a `data` property. All errors contain `code`, `message`, `requestId`, and `retryable`.

The API provides:

- `POST /api/demo/sessions`
- `POST /api/demo/session/reset`
- `GET /api/incidents/current`
- `GET /api/releases/compare`
- `GET /api/logs`
- `POST /api/diagnostics`
- `POST /api/recovery-plans`
- `GET /api/recovery-plans/current`
- `POST /api/recovery-plans/{id}/approve`
- `POST /api/recovery-plans/{id}/reject`
- `POST /api/recovery-plans/{id}/execute`
- `GET /api/recovery-plans/{id}/verify`
- `GET /api/audit-events`
- `GET /api/health`

Mutation routes require the session cookie, an allowed `Origin`, and `X-Demo-Request: 1`. Limit request bodies to 32 KiB. Limit telemetry windows to 5 through 60 minutes and log results to 25 rows.

## WebMCP integration

Use `document.modelContext.registerTool`. Each registration receives an `AbortSignal` owned by the registry.

The registry tracks registration name, definition fingerprint, promise, controller, and owner count. It defers zero-owner cleanup by one task so React development remounts do not race browser unregistration.

Tool callbacks parse arguments with Zod before calling Rust. They return a serializable `ToolOutcome` with a short summary and bounded data. Domain errors return structured safe errors. Cancellation rethrows `AbortError`.

Register these tools at startup:

- `inspect_incident`
- `compare_releases`
- `query_logs`
- `run_diagnostic`
- `prepare_recovery`
- `verify_recovery`

Register `execute_approved_recovery` only when the backend reports one approved, unexpired plan. Its input contains only `planId`.

Set `untrustedContentHint: true` on `query_logs`. Do not claim a native `outputSchema`, confirmation API, or MCP result envelope.

## Frontend state

Use a client-side controller for this single-page operational workspace. Keep server responses as the source of truth. After any mutation, fetch the current incident and current plan before updating capability state.

The page contains:

- Header and reset control
- Service health and telemetry
- Release comparison
- Logs and diagnostic results
- Recovery-plan review
- WebMCP tool inspector
- Audit timeline

The plan panel is an accessible in-page region. Move focus to its heading when preparation succeeds. Announce capability changes and incident status through polite live regions.

## Deployment

Deploy the frontend to Vercel with Node 24.x. Bun installs and builds the application.

Deploy the Rust service as a reproducible container on one always-on Render instance. Attach a small persistent disk for SQLite and configure `/api/health` as the health check.

Set `Origin-Agent-Cluster: ?1` and `Permissions-Policy: tools=(self)` on the frontend. Do not enable cross-origin credentials on the Rust service.

## Verification

Rust unit tests cover pure domain rules. Axum integration tests cover the cookie-bound API workflow. Vitest covers component and registry lifecycles. Playwright drives the full browser workflow with an injected standards-shaped model context.

The release gate also requires a real run in ChatGPT's in-app browser or Chrome 149 or later with WebMCP testing enabled.

