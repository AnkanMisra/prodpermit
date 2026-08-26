# Recovery control room scope

## Product thesis

A browser-native incident workspace lets an AI agent investigate a simulated failure through structured WebMCP tools. The agent prepares an exact recovery plan, but it receives an execution capability only after a human approves that plan.

## Audience

The primary user is an incident commander or on-call engineer who wants an agent to gather evidence without surrendering authority over a production change.

## Build target

Build one complete deterministic incident for `checkout-api`:

- `release_283` is the healthy baseline.
- `release_284` is active and fails database authentication.
- `release_285` is staged and unrelated.
- A rollback to `release_283` restores health.

The application must make the invisible WebMCP capability lifecycle visible to a judge. The execution tool is absent at first, appears after exact human approval, and disappears after execution or invalidation.

## Included

- Session-isolated seeded incident state
- Service health, telemetry, releases, logs, and diagnostics
- Seven semantic WebMCP tools
- Recovery-plan state machine and exact approval fingerprint
- Atomic backend execution and verification
- Tool inspector and audit timeline
- Resettable judge sessions
- Automated tests and real supported-client verification
- Vercel frontend and containerized Rust backend deployment
- Public documentation and Devpost draft materials

## Excluded

- Real cloud, Kubernetes, database, or deployment-provider mutations
- User accounts, organization management, or OAuth
- Multiple incident scenarios
- Embedded chat or a model API
- Arbitrary shell commands, queries, or release mutations
- Mobile-native applications
- Horizontal scaling of the SQLite backend

## Success criteria

The build is complete when the deployed app demonstrates the full investigate, prepare, approve, execute, and verify journey in under three minutes. The backend must reject execution without approval and reject stale, expired, replayed, modified, or cross-session plans.

## Time constraint

The official deadline is September 3, 2026 at 1:00 p.m. Pacific Time. Scope remains fixed to one incident and one recovery action until the submission packet is ready.

