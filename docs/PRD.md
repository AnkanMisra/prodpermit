# Product requirements

## Product outcome

An incident commander opens the app and sees a failing checkout service. A browser agent can inspect evidence and prepare a rollback. The human reviews the exact change and grants a temporary execution capability by approving it. The agent executes that one recovery and verifies the result.

## First-run experience

1. The app creates or resumes an isolated demo session.
2. The dashboard shows `checkout-api` as critical on `release_284`.
3. The tool inspector lists six initial tools and does not list `execute_approved_recovery`.
4. Unsupported clients receive a clear WebMCP message while the human dashboard remains usable.

## Investigation

The agent can inspect the incident, compare releases, query bounded logs, and run a database-connectivity diagnostic.

Acceptance criteria:

- Incident inspection returns the current release, health, recent releases, and no more than 60 minutes of telemetry.
- Release comparison redacts secret values and highlights the authentication-mode regression.
- Log queries return at most 25 events and label external text as untrusted.
- The prompt-injection-shaped log cannot approve or execute any action.
- Cancelling a diagnostic aborts the browser request and does not record a completed result.
- Each tool call updates the visible inspector and relevant dashboard panel.

## Recovery preparation

The agent can prepare a rollback to `release_283` with a short reason and evidence references.

Acceptance criteria:

- Plan preparation never changes the active release.
- The plan panel shows before and after releases, evidence, risk, preconditions, expiry, and exact fingerprint.
- The panel states `Production changed: No` before execution.
- `release_285`, unknown releases, and unrelated evidence are rejected.
- Only one nonterminal plan exists in a session.

## Human authority

The human can approve or reject the displayed plan with the keyboard or pointer.

Acceptance criteria:

- Approval includes the displayed plan fingerprint.
- Rejection is terminal and removes any execution capability.
- Approval causes `execute_approved_recovery` to appear without a page reload.
- Refresh restores the tool only if the approved plan remains valid.
- Expiry, reset, rejection, execution, or invalidation removes the tool.

## Execution and verification

The agent can execute only the exact approved plan and then verify recovery.

Acceptance criteria:

- The backend rejects execution before approval.
- The backend rejects altered, stale, expired, replayed, and cross-session plans.
- Successful execution changes the active release to `release_283` in SQLite.
- Health becomes healthy, the incident resolves, and the database diagnostic passes.
- Verification returns before-and-after evidence.
- The audit timeline records the tool call, approval, execution, and verification.

## Reset

The human can reset the scenario at any time.

Acceptance criteria:

- Reset rotates the session and restores the broken state.
- Every old plan becomes unusable.
- One judge's reset does not affect another judge's session.

## Quality requirements

- Every interactive control is keyboard usable.
- Status never relies on color alone.
- The interface has visible focus states and an `aria-live` incident summary.
- Reduced-motion preferences disable nonessential motion.
- Every failure path has a safe message and a retry or reset action when appropriate.
- The complete demo fits within three minutes.

