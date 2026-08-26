# Threat model

## Protected assets

- Human authority over recovery execution
- Session-specific incident and plan state
- Integrity of the approved plan
- Release and health state
- Audit history
- Configuration secrets

## Trust boundaries

The external browser agent is untrusted. Tool arguments are untrusted. Log text and customer content are untrusted. The Next.js UI is not an authorization boundary. The Rust service and its SQLite transaction are the enforcement boundary.

## Threats and controls

### Prompt injection in logs

An attacker places instructions in operational text and asks the agent to execute them.

Controls:

- Mark the complete `query_logs` tool with `untrustedContentHint: true`.
- Return structured fields and explicit `untrusted: true` values.
- Treat text as evidence data only.
- Require backend plan validation and human approval regardless of agent reasoning.

### Tool argument manipulation

An agent submits unknown releases, long input, altered plan fields, or foreign evidence.

Controls:

- Use strict JSON schemas and Zod at the browser boundary.
- Validate every request again in Rust.
- Resolve releases and evidence from the cookie-bound session.
- Let execution accept only `planId`.

### Unapproved execution

An agent calls the HTTP endpoint directly before approval.

Controls:

- Omit the execution tool until approval.
- Require `approved` status in the execution transaction.
- Treat WebMCP registration as user experience, not authorization.

### Replay and duplicate execution

An agent repeats a previously successful call or two callers race.

Controls:

- Use a conditional status transition in one SQLite write transaction.
- Make `executed` terminal.
- Verify that one concurrent caller changes the state.

### Stale plans

The active release or scenario changes after plan creation.

Controls:

- Store `expected_current_release` and scenario generation.
- Check both inside the execution transaction.
- Expire plans after ten minutes.

### Cross-session access

One judge attempts to use another judge's plan ID.

Controls:

- Bind every lookup to the opaque session cookie.
- Do not accept session IDs in route parameters or tool input.
- Rotate the session on reset.

### Secret exposure

Logs, diffs, errors, or traces reveal credentials.

Controls:

- Seed no genuine credentials.
- Store redacted configuration projections.
- Use safe error messages and structured tracing fields.
- Run a secret scan before any public push.

### Browser refresh during approval

The page refreshes while a plan is approved.

Controls:

- Load current plan state from Rust after startup.
- Register the execution tool only for a still-valid approved plan.
- Let the backend reject state that expires during refresh.

### Session interference

One judge resolves or resets a shared global incident.

Controls:

- Seed complete state per session.
- Scope every query and mutation to the session.
- Replace reset sessions instead of mutating a global scenario.

## Residual risks

- SQLite and a persistent disk limit the backend to one instance.
- An always-on host can still restart during a judge session.
- WebMCP is experimental and browser behavior may change before judging.
- Tool annotations help clients classify data but do not guarantee model behavior.

