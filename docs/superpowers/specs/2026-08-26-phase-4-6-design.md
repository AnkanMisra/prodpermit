# Phase 4 through Phase 6 design

## Problem

The recovery happy path exists, but the persisted model does not yet prove the authority guarantees in the product requirements. `recovery_plans` stores status and immutable plan fields in both columns and mutable JSON. Evidence has no child rows. Several plan changes and their audit events use separate transactions. Verification reports a passing diagnostic without reading one from SQLite. Reset and durable expiry are absent.

Phases 4 through 6 must close those gaps without changing the fixed product scope. Rust remains the authorization boundary. TypeScript parses wire data, manages browser state, and registers WebMCP tools.

## Definition of done

The work is complete when these predicates hold:

- Phase 4 implements prepare, review, exact approval, rejection, expiry, execution, verification, audit, and reset.
- Rust rejects unknown evidence, unrelated evidence, wrong targets, wrong fingerprints, unapproved plans, expired plans, stale plans, invalidated plans, replays, and cross-session access.
- Every recovery mutation and its audit event commit in one SQLite transaction.
- Execution persists the release change, healthy service state, resolved incident, healthy telemetry, passing diagnostic, and links to the plan.
- Verification derives its before-and-after response from persisted facts.
- Reset revokes the old session, invalidates old authority, and creates one replacement session. A revoked cookie cannot retrieve that replacement.
- The browser registers execution only from a Rust-produced capability decision.
- Phase 5 isolation, race, boundary, and abuse tests pass. A standard repository security scan has no unresolved validated finding in scope.
- Phase 6 local formatting, linting, tests, builds, browser checks, container checks, and repository checks pass.
- The project is not deployed, published, pushed, uploaded, or submitted.

## Usage

Axum parses transport input, extracts the cookie-bound session, and calls one deep `Store` operation.

```rust
let current = store.current_recovery(&session_id, now).await?;
let prepared = store.prepare_recovery(&session_id, command, now).await?;
let decided = store.decide_recovery(&session_id, &plan_id, decision, now).await?;
let executed = store.execute_recovery(&session_id, &plan_id, now).await?;
let verified = store.verify_recovery(&session_id, &plan_id, now).await?;
let replacement = store.reset_session(&session_id, now).await?;
```

The browser consumes one server envelope.

```ts
type CurrentRecovery = {
  plan: RecoveryPlan | null;
  executionCapability:
    | { kind: "available"; planId: PlanId; fingerprint: string; expiresAt: string }
    | { kind: "absent"; reason: CapabilityAbsenceReason };
};
```

Only `executionCapability.kind === "available"` permits registration of `execute_approved_recovery`. The tool input remains `{ planId }`. Rust repeats every authorization check inside the execution transaction.

## Domain shape

`RecoveryPlanSpec` contains the immutable facts that the human reviews:

- plan, session, incident, and service IDs;
- scenario generation;
- expected and target releases;
- trimmed reason;
- ordered evidence;
- policy version;
- creation and expiry timestamps.

`RecoveryEvidence` is a closed enum. Version 1 accepts a database-authentication failure log and a failed database-connectivity diagnostic for the same session, service, release, and generation. Unknown, duplicate, ambiguous, and unrelated evidence fails preparation.

`RecoveryPlanState` models prepared, approved, executing, executed, rejected, expired, and invalidated states. The domain exposes pure functions for preparation, fingerprinting, expiry, human decisions, execution validation, and verification derivation.

The canonical fingerprint uses a versioned, length-prefixed byte encoding. It does not hash response JSON or delimiter-joined strings. Input evidence order does not change the fingerprint. Any reviewed fact does.

## Persistence shape

Add `0004_recovery_lifecycle.sql`. Do not edit the already used `0003` migration.

The migration adds:

- `revoked_at` on `demo_sessions`;
- `session_resets` for audit lineage without credential reissuance;
- `service_releases` for explicit release ownership;
- `diagnostic_contexts` for service, release, and generation evidence;
- normalized recovery-plan columns with no authoritative `plan_json`;
- `recovery_plan_evidence` with canonical ordinals and source foreign keys;
- `recovery_plan_executions` linking the exact telemetry and diagnostic produced by execution;
- audit deduplication keys for retry-safe lifecycle records.

The migration converts any existing `0003` rows and then removes JSON from recovery reads and writes. A migration test proves that the stored fingerprint still matches the normalized facts.

## Transaction rules

Each operation owns its transaction from the first authoritative read through the audit write.

- Prepare expires or invalidates an old active plan, resolves evidence, inserts the plan and evidence rows, and writes `recovery_prepared`.
- Current recovery persists due expiry or invalidation before it returns the capability envelope.
- Approval and rejection recompute the fingerprint, use a conditional transition, and write one audit event.
- Execution claims the approved row before mutable reads. It rechecks session activity, generation, active release, target ownership, target eligibility, and all fingerprint copies. It then writes every recovery effect and the audit event before commit.
- Verification joins the plan, before evidence, execution links, service, incident, telemetry, diagnostic, and diagnostic context. It writes one deduplicated audit result.
- Reset revokes the old session, invalidates nonterminal plans, seeds one replacement scenario, records lineage, and writes audit events. A retry with the old cookie returns the same replacement.

Use an explicit immediate SQLite write transaction where SQLx permits it. The two-connection race tests decide the exact implementation.

## API boundaries

Mutation routes require the allowed `Origin`, `X-Demo-Request: 1`, an active session cookie, a body no larger than 32 KiB, and structs with `deny_unknown_fields`.

Unknown and foreign plan IDs both return `PLAN_NOT_FOUND`. Unknown and foreign evidence IDs both return `INVALID_RECOVERY_EVIDENCE`. Responses do not reveal another session's data.

The API injects a clock so expiry tests do not wait ten minutes. Production uses `SystemClock`.

## Browser state

After every mutation, the controller fetches the current incident, current recovery envelope, and audit events. It applies them as one state update.

Reset increments a session epoch and unregisters execution before the request. Responses captured under an older epoch are discarded. An expiry timer unregisters the tool and refreshes server state, but the timer is not an authorization check.

The plan panel shows the full fingerprint, ordered evidence, risk, preconditions, expiry, and production-change state. Preparation moves focus to the panel heading. Live regions announce incident and capability changes. Header and inspector text derive from current state.

The registry replaces an existing dynamic tool when its definition fingerprint changes. Replacement aborts the old registration signal before publishing the new callback.

## Verification strategy

Use strict red-green cycles. Each new behavior starts with a test that fails for the missing behavior.

- Domain tests prove evidence rules, canonical fingerprints, lifecycle transitions, execution checks, and verification outcomes.
- File-backed SQLite tests prove migration integrity, transaction rollback, expiry, reset, audit coupling, and two-connection races.
- Axum tests prove cookies, origins, body limits, error envelopes, session expiry, cross-session privacy, and the complete workflow.
- Vitest proves wire parsing, epoch handling, capability registration, replacement, expiry cleanup, reset, focus, and display.
- Playwright proves the complete browser journey, reload restoration, rejection, replay, reset isolation, keyboard behavior, and absence of browser errors.
- Phase 5 runs the standard repository security scan and fixes validated findings.
- Phase 6 reruns every gate from a fresh local state.

## Synthesis decision

The normalized candidate is the base because it removes the current duplicate JSON authority and fits the existing `Store` seam. The snapshot and event candidate added more lifecycle machinery than this single-instance demo needs.

Three parts were adapted from the alternate candidate:

- Keep the existing plan view and add a small `executionCapability` envelope instead of replacing every frontend state with a larger protocol.
- Acquire the SQLite writer position explicitly before race-sensitive recovery mutations.
- Replace the dynamic execution registration when its definition fingerprint changes.

The design rejects an append-only plan event log, a browser-owned expiry rule, deletion of old sessions on reset, and mutation of the existing `0003` migration.

## Constraints

- Use Bun for JavaScript commands and Cargo for Rust commands.
- Keep one incident and one recovery action.
- Do not add real infrastructure mutation or a model API.
- Do not configure Git identity.
- Do not deploy, publish, push, upload, or submit.
