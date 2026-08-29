# Phase 4 through Phase 6 implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `subagent-driven-development` or `executing-plans` to implement this plan task by task. Steps use checkbox syntax for tracking.

**Goal:** Complete and verify the recovery lifecycle, reliability and security work, and all local release gates through Phase 6.

**Architecture:** Rust domain types and `Store` operations own recovery policy. SQLite stores normalized plan facts, ordered evidence, execution evidence, reset lineage, and atomic audit records. Next.js consumes typed server state and registers WebMCP execution only from the server capability envelope.

**Tech stack:** Rust 1.97.1, Axum 0.8.9, SQLx 0.9.0, SQLite, Bun 1.3.14, Next.js 16.3.3, React 19.2.8, TypeScript 6.0.3, Zod 4.4.3, Vitest 4.1.11, and Playwright 1.62.1.

**Spec:** `docs/superpowers/specs/2026-08-26-phase-4-6-design.md`

## Global constraints

- Complete Phase 4 before Phase 5 and Phase 5 before Phase 6.
- Write a failing test before each behavior change.
- Keep domain and authorization rules in Rust.
- Use Bun for JavaScript commands and Cargo for Rust commands.
- Use `apply_patch` for source and documentation edits.
- Preserve unrelated working-tree changes.
- Do not configure Git identity or create phase commits while identity is missing.
- Do not deploy, publish, push, upload, or submit.

---

### Task 1: Normalize recovery domain facts

**Files:**

- Create: `crates/domain/src/recovery.rs`
- Modify: `crates/domain/src/lib.rs`
- Modify: `crates/domain/tests/recovery_plan.rs`

**Produces:** `RecoveryPlanSpec`, `RecoveryEvidenceSet`, `RecoveryPlanState`, `PrepareRecoveryCommand`, `HumanDecision`, canonical fingerprinting, execution validation, and verification derivation.

- [ ] Add failing tests for required evidence types, duplicate evidence, wrong release and generation, canonical ordering, delimiter-safe fingerprints, expiry equality, idempotent approval, approval revocation, every execution precondition, and verification mismatch outcomes.
- [ ] Run `cargo test -p recovery-domain --test recovery_plan` and confirm the new tests fail for missing behavior.
- [ ] Implement the typed recovery module. Keep constructors private where invalid states must remain unrepresentable.
- [ ] Re-export the required public types from `lib.rs` and remove the legacy recovery implementation.
- [ ] Run `cargo fmt --all --check`, `cargo clippy -p recovery-domain --all-targets -- -D warnings`, and `cargo test -p recovery-domain`.

### Task 2: Add the normalized SQLite lifecycle

**Files:**

- Create: `crates/persistence/migrations/0004_recovery_lifecycle.sql`
- Create: `crates/persistence/src/recovery.rs`
- Create: `crates/persistence/src/session.rs`
- Modify: `crates/persistence/src/lib.rs`
- Expand: `crates/persistence/tests/session_store.rs`

**Produces:** normalized plan storage, ordered evidence, service-release ownership, diagnostic context, execution links, session revocation, reset lineage, and atomic lifecycle methods.

- [ ] Add file-backed migration tests that assert foreign keys, normalized legacy conversion, and fingerprint preservation.
- [ ] Add failing behavior tests for evidence resolution, one active plan, durable expiry, atomic audit rollback, conditional approval and rejection, one-winner execution, persisted after evidence, verification joins, session revocation, and retryable reset.
- [ ] Run the persistence tests and confirm that each new behavior fails for the expected missing mechanism.
- [ ] Implement `0004_recovery_lifecycle.sql` and transaction-local row parsers.
- [ ] Implement `prepare_recovery`, `current_recovery`, `decide_recovery`, `execute_recovery`, `verify_recovery`, and `reset_session` on `Store`.
- [ ] Use real SQLite constraints and triggers in tests. Do not add a production failure-injection API.
- [ ] Run `cargo fmt --all --check`, strict persistence Clippy, and `cargo test -p recovery-persistence`.

### Task 3: Enforce the complete Axum boundary

**Files:**

- Create: `crates/api/src/recovery_routes.rs`
- Create: `crates/api/src/session_routes.rs`
- Modify: `crates/api/src/lib.rs`
- Expand: `crates/api/tests/walking_skeleton.rs`

**Produces:** clock injection, active-session extraction, reset route, recovery envelope, body limit, structured extractor failures, and safe error mapping.

- [x] Add integration tests for every recovery route, reset cookie rotation, revoked-cookie denial, inactive sessions, unknown fields, 32 KiB bodies, cross-session privacy, wrong fingerprints, stale plans, replay, and parallel execution.
- [ ] Run the API integration test and confirm the new cases fail for the expected missing behavior.
- [ ] Add `Clock` and `SystemClock`, then pass deterministic time through the router in tests.
- [ ] Add an active-session boundary that rejects expired or revoked sessions on every scoped route.
- [ ] Route `POST /api/demo/session/reset` and return the replacement cookie only after commit.
- [ ] Map domain and persistence errors to the documented safe error envelope.
- [ ] Apply the body limit and JSON rejection mapping at the HTTP boundary.
- [ ] Run strict API Clippy and `cargo test -p recovery-api`.

### Task 4: Make browser capability state server-owned

**Files:**

- Modify: `apps/web/src/lib/contracts.ts`
- Modify: `apps/web/src/lib/api.ts`
- Create: `apps/web/src/lib/recovery-controller.ts`
- Modify: `apps/web/src/lib/webmcp/registry.ts`
- Modify: `apps/web/src/lib/webmcp/recovery-tools.ts`
- Modify: `apps/web/src/components/control-room.tsx`
- Add or expand matching Vitest files.

**Produces:** discriminated wire schemas, session epochs, refresh-after-mutation, expiry cleanup, reset handling, safe tool outcomes, and registry replacement.

- [ ] Read the relevant Next.js 16 local documentation under `apps/web/node_modules/next/dist/docs/` before editing client code.
- [ ] Add failing Vitest cases for impossible wire states, old-epoch responses, server-owned capability registration, replacement on changed fingerprint, expiry cleanup, reset cleanup, and structured safe tool errors.
- [ ] Run the focused Vitest files and confirm each new test fails for the missing behavior.
- [ ] Implement the Zod schemas and derive TypeScript types from them.
- [ ] Implement the pure controller reducer and one refresh operation that loads incident, recovery, and audit state together.
- [ ] Implement registry replacement and abort the old registration before exposing a changed callback.
- [ ] Wire preparation, decisions, execution, verification, expiry, and reset through the controller.
- [ ] Run `bun run typecheck`, `bun run lint`, and `bun run test:web`.

### Task 5: Complete the recovery review experience

**Files:**

- Modify: `apps/web/src/components/incident-dashboard.tsx`
- Modify: `apps/web/src/components/incident-dashboard.test.tsx`
- Modify: `apps/web/src/app/globals.css`
- Modify: `apps/web/e2e/walking-skeleton.spec.ts`

**Produces:** reset control, full approval evidence, accurate status text, focus movement, live announcements, and complete before-and-after verification.

- [ ] Add failing component tests for the full fingerprint, ordered evidence, expiry, accurate incident and capability copy, reset, keyboard controls, focus movement, live regions, and before-and-after verification.
- [ ] Run the focused component tests and confirm expected failures.
- [ ] Render every required plan and verification fact without duplicating authorization rules.
- [ ] Add reset and retry behavior, visible focus, and reduced-motion-safe transitions.
- [ ] Add Playwright cases for rejection, reload restoration, expiry, replay, reset isolation, two browser contexts, keyboard use, and console errors.
- [ ] Run `bun run test:web`, `bun run build:web`, and `bun run test:e2e`.

### Task 6: Close Phase 4

**Files:**

- Modify: `docs/BUILD_CHECKLIST.md`
- Modify: `docs/STATUS.md`
- Create: `docs/phase-reports/04-recovery-lifecycle.md`
- Append: `.audit/recovery-control-room.tsv`
- Append: `docs/hackathon-build/build-notes.md`

- [ ] Run the complete Phase 4 Rust and browser gate.
- [ ] Check every Phase 4 acceptance criterion against a test or running-browser observation.
- [ ] Record exact commands, counts, remaining risks, and artifacts in the report.
- [ ] Mark Phase 4 complete only after every exit condition passes.

### Task 7: Complete Phase 5 reliability and security

**Files:**

- Expand Rust, API, Vitest, and Playwright tests where Phase 4 coverage does not prove the threat model.
- Create: `docs/phase-reports/05-reliability-and-security.md`
- Modify: `docs/BUILD_CHECKLIST.md`
- Modify: `docs/STATUS.md`
- Append: `.audit/recovery-control-room.tsv`

- [ ] Add or confirm tests for session isolation, reset isolation, race behavior, replay, stale generation, expired sessions, malformed input, request size, prompt-injection-shaped logs, secret redaction, and safe errors.
- [ ] Run the complete local test gate.
- [ ] Run the standard repository security scan against the current repository and threat model.
- [ ] Validate each candidate finding against source and an attack path.
- [ ] Add a failing regression test for each validated finding, implement the smallest root-cause fix, and rerun the affected gate.
- [ ] Run a final standard scan or verification required by the scan workflow.
- [ ] Mark Phase 5 complete only when validated findings are fixed and the abuse tests pass.

### Task 8: Complete Phase 6 local verification

**Files:**

- Create: `docs/phase-reports/06-full-verification.md`
- Modify: `docs/BUILD_CHECKLIST.md`
- Modify: `docs/STATUS.md`
- Append: `.audit/recovery-control-room.tsv`

- [ ] Run `cargo fmt --all --check`.
- [ ] Run `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] Run `cargo test --workspace`.
- [ ] Run `bun install --frozen-lockfile`.
- [ ] Run `bun run typecheck`.
- [ ] Run `bun run lint`.
- [ ] Run `bun run test:web`.
- [ ] Run `bun run build:web`.
- [ ] Run `bun run test:e2e`.
- [ ] Run `docker compose config` and `docker compose build`.
- [ ] Run a local secret scan and `git diff --check`.
- [ ] Inspect the final browser screenshots and browser console output.
- [ ] Document the real supported WebMCP-client check for the participant if this environment cannot perform it.
- [ ] Mark Phase 6 complete only when every locally executable gate passes. If the real-client check remains human-owned, record it explicitly rather than claiming it passed.

### Task 9: Audit and hand back

**Files:**

- Review: `.audit/recovery-control-room.tsv`
- Review: all changed source, tests, status files, and phase reports.

- [ ] Reconcile the decision trail with the actual diff and command outputs.
- [ ] Run a cross-model review of the audit trail and unresolved risks.
- [ ] Confirm that no deploy, publish, push, upload, or submission action occurred.
- [ ] Hand back the local project, exact verification evidence, and any human-only real-client test.
