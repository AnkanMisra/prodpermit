# Build checklist

This file is the only task-status source. Use `pending`, `in_progress`, `blocked`, or `completed`.

| Phase | Status | Exit condition | Report |
|---|---|---|---|
| 0. Inspect and verify | completed | Repository, toolchains, official rules, and current WebMCP API verified | [Phase 0](phase-reports/00-inspect.md) |
| 1. Initialize and specify | completed | Workspaces and decision-complete documents exist and pass structural checks | [Phase 1](phase-reports/01-initialize-and-specify.md) |
| 2. Walking skeleton | completed | The browser displays the seeded broken incident from Rust | [Phase 2](phase-reports/02-walking-skeleton.md) |
| 3. Investigation workflow | completed | WebMCP tools identify the root cause without an execution capability | [Phase 3](phase-reports/03-investigation-workflow.md) |
| 4. Recovery lifecycle | completed | Human approval gates one atomic recovery and verification | [Phase 4](phase-reports/04-recovery-lifecycle.md) |
| 5. Reliability and security | completed | Isolation and abuse tests pass; validated security findings are fixed | [Phase 5](phase-reports/05-reliability-and-security.md) |
| 6. Full verification | completed | Tests, builds, browser checks, and a real WebMCP client pass | [Phase 6](phase-reports/06-full-verification.md) |
| 7. Deployment and submission preparation | pending | Live URLs and submission packet are complete | Pending |

## Phase 1 tasks

| Task | Status | Verification |
|---|---|---|
| Initialize Git and directory structure | completed | `git status --short --branch` |
| Add agent entry point and current-status handoff | completed | Links resolve and name the next task |
| Write scope and product requirements | completed | Acceptance criteria cover the complete human-agent journey |
| Write technical specification and threat model | completed | Interfaces, transitions, data flow, and mitigations are explicit |
| Add Cargo and Bun workspace manifests | completed | `cargo metadata --no-deps` and `bun install --frozen-lockfile` |
| Write Phase 1 report | completed | [Phase 1 report](phase-reports/01-initialize-and-specify.md) |
| Create Phase 1 Git checkpoint | blocked | Git author name and email are not configured |

## Phase 2 tasks

| Task | Status | Verification |
|---|---|---|
| Implement Axum configuration, tracing, and health route | completed | `cargo test -p recovery-api` |
| Add SQLite migrations and deterministic session seed | completed | Persistence test round-trips the seeded scenario |
| Add the current-incident endpoint | completed | API test returns the broken scenario |
| Build the Next.js operational shell | completed | Production build and component test pass |
| Connect Next.js to Rust through the development rewrite | completed | Chromium shows the Rust incident |
| Add Docker and local development commands | completed | Compose configuration and both images build |
| Write Phase 2 report | completed | [Phase 2 report](phase-reports/02-walking-skeleton.md) |
| Create Phase 2 Git checkpoint | blocked | Git author name and email are not configured |

## Phase 3 tasks

| Task | Status | Verification |
|---|---|---|
| Model and persist release configuration, logs, diagnostics, and audit events | completed | Domain and persistence tests pass |
| Add investigation API routes | completed | Axum integration tests pass |
| Add the typed WebMCP registry and four investigative tools | completed | Registry and tool-definition tests pass |
| Add release comparison, logs, diagnostics, and tool inspector UI | completed | TypeScript, ESLint, component tests, and production build pass |
| Prove the investigation journey and execution-tool absence | completed | Playwright WebMCP adapter test passes |
| Write Phase 3 report | completed | [Phase 3 report](phase-reports/03-investigation-workflow.md) |

## Phase 4 tasks

| Task | Status | Verification |
|---|---|---|
| Implement recovery-plan types, fingerprint, and state transitions | completed | Required domain invariant tests |
| Persist plans, evidence, and audit events | completed | Persistence transition tests |
| Add prepare, current, approve, reject, execute, verify, and audit routes | completed | Complete Axum workflow test |
| Add plan review and audit UI | completed | Component tests and production build |
| Register preparation and verification tools | completed | Initial registry test |
| Dynamically gate the execution tool | completed | Lifecycle and Playwright tests |
| Write Phase 4 report | completed | [Phase 4 report](phase-reports/04-recovery-lifecycle.md) |
