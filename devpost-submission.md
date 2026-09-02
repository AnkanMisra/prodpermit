# Title

Recovery Control Room

## One-line Summary

An incident workspace where agents investigate through WebMCP, but a person must approve the exact recovery before execution.

## Problem

Incident response forces operators to move between dashboards, logs, release history, diagnostics, and change controls while production is already failing. An AI agent can collect evidence quickly, but giving an agent permanent write access creates a new risk. The operator still needs to understand and approve the exact production change.

## Solution

Recovery Control Room turns one checkout incident into a complete human-agent recovery workflow. The website exposes structured investigation tools through `document.modelContext.registerTool`. An agent can inspect the incident, compare releases, query bounded logs, run a safe diagnostic, and prepare a rollback plan.

Preparation does not change production. Rust resolves the cited evidence, checks that the target is a known healthy release, creates an immutable plan, and returns its fingerprint. The browser shows the complete plan to the operator. Only the approval click can cause Rust to expose `execute_approved_recovery`. The capability accepts only the approved plan ID and disappears after execution, rejection, revocation, reset, or expiry.

The demo then verifies the persisted before-and-after facts and shows the audit history.

## Why This Matters

The project focuses on a real boundary in AI-assisted operations. Agents are useful at gathering evidence and forming a recommendation. People remain responsible for authorizing a production change.

WebMCP fits this problem because it gives the agent typed, purpose-built operations instead of making it scrape labels or guess which button to click. It also lets the website change the available capability set as authority changes. The agent can investigate from the start, but execution does not exist until the person approves one exact plan.

The result is faster investigation without permanent agent write access.

## How We Used AI

Recovery Control Room does not hide an AI model behind a chat box. It exposes a workflow that any compatible agent can use through the browser's WebMCP interface.

Six tools exist when a session starts:

- `inspect_incident`
- `compare_releases`
- `query_logs`
- `run_diagnostic`
- `prepare_recovery`
- `verify_recovery`

The seventh tool, `execute_approved_recovery`, exists only while the backend reports an approved, unexpired plan for the current cookie-bound session. Rust owns that decision. TypeScript registers or removes the browser capability from the backend response.

Tool results are bounded and structured. Customer-like log text is marked as untrusted content. Diagnostic cancellation propagates through `AbortSignal`. Recovery evidence and authorization remain server-side.

## How We Used Codex

Codex helped turn the initial incident-recovery brief into the scope, product requirements, technical specification, threat model, and phased build checklist.

During implementation, Codex:

- built the Rust domain, Axum API, SQLite migrations, Next.js interface, and WebMCP registry;
- used browser tests to find a cookie authentication regression and verify that execution was absent before approval;
- compared recovery-state designs before moving all authorization decisions into Rust;
- ran a repository security scan, validated three findings, and fixed stale credential reissue, stale-session reads, and unbounded anonymous session growth;
- tested the app with unit, integration, component, Playwright, container, and native Chrome WebMCP checks;
- deployed the frontend, backend, persistent database, and HTTPS ingress, then ran all five browser journeys against production.

The repository keeps phase reports and an audit log so each design and verification claim can be checked.

## Key Features

- Six structured WebMCP investigation and planning tools.
- One execution capability that appears only after exact human approval.
- A SHA-256 plan fingerprint over canonical recovery facts.
- Evidence-bound rollback preparation with a ten-minute expiry.
- Atomic SQLite execution, verification facts, and audit writes.
- Cookie-bound session isolation with reset revocation and bounded session growth.
- Untrusted log labeling and redacted release comparison.
- Visible tool registry, invocation status, plan state, and audit timeline.
- A deterministic reset that restores the broken incident for another judge.

## Architecture

The Next.js 16 frontend runs on Vercel. It calls `/api/backend/*` on its own origin. A Next.js rewrite sends those requests through Cloudflare Tunnel to the Rust service on Ankan-Linux. Tailscale Funnel remains a fallback ingress.

Rust and Axum own domain rules, authorization, plan fingerprints, session validity, and recovery transitions. SQLite stores isolated demo sessions, evidence, recovery plans, verification facts, and audit events in a persistent Docker volume.

The browser validates wire data with Zod and manages `document.modelContext` registrations. It does not decide whether execution is authorized.

## Testing Instructions

No account or credentials are required.

1. Open https://recovery-control-room.vercel.app in the ChatGPT desktop in-app browser or Chrome 149 or later.
2. In Chrome, enable `chrome://flags/#enable-webmcp-testing` and restart the browser before opening the app.
3. Confirm that the page shows the critical `checkout-api` incident on `release_284` and reports WebMCP support.
4. Ask the agent to inspect the incident, compare `release_283` with `release_284`, query the recent logs, and run the database connectivity diagnostic.
5. Ask the agent to prepare a rollback to `release_283` using the database authentication log and diagnostic as evidence.
6. Review the target, reason, evidence, expiry, and full fingerprint. Confirm that "Production changed" still says "No".
7. Click "Approve exact plan". Confirm that `execute_approved_recovery` appears in the tool registry.
8. Ask the agent to execute the approved plan and then verify recovery.
9. Confirm that the service is healthy on `release_283`, the execution tool is gone, "Production changed" says "Yes", and the audit timeline contains preparation, approval, execution, and verification.
10. Click "Reset scenario" to restore the original failure.

The production release also passed five automated Chromium journeys covering this entire lifecycle.

## Public Demo Link

https://recovery-control-room.vercel.app

Backend health endpoint: https://ankan-linux.tailf04855.ts.net/api/health

## Public Repository Link

https://github.com/AnkanMisra/webmcp-project

The repository is public. GitHub detects the MIT license and the README contains local run and verification commands.

## Demo Video

TODO: Add the public YouTube URL. The official requirement is a public video with audio that is shorter than three minutes.

Suggested 155-second outline:

1. 0:00-0:12. Open on the critical checkout incident and state the problem.
2. 0:12-0:42. Show the six initial tools. Invoke incident inspection, release comparison, logs, and the database diagnostic.
3. 0:42-1:12. Prepare the rollback. Show the evidence, target release, fingerprint, and "Production changed: No".
4. 1:12-1:32. Click "Approve exact plan". Show the execution tool appearing only after approval.
5. 1:32-1:58. Execute and verify. Show healthy `release_283` and the execution tool disappearing.
6. 1:58-2:20. Show the audit timeline and explain that Rust owns authorization and atomic state changes.
7. 2:20-2:35. Close with the WebMCP value: agents investigate through structured tools while people authorize production changes.

## Screenshot Shot List

1. Broken incident and initial service telemetry: `output/playwright/phase-2-walking-skeleton.png`
2. Release comparison, untrusted logs, failed diagnostic, and tool activity: `output/playwright/phase-3-investigation.png`
3. Healthy release, verified recovery, removed execution capability, and audit timeline: `output/playwright/phase-4-recovery.png`

All three screenshots came from the production browser verification on September 1, 2026.

## Submission Readiness Notes

Ready:

- The public application and backend health endpoints respond over HTTPS.
- The complete recovery workflow passes against production.
- The repository is public and contains an MIT license, source, screenshots, and run instructions.
- Chrome for Testing 151 discovered and invoked the native WebMCP tools.
- ChatGPT's in-app browser investigated the live incident, prepared plan `743b1fee-7ab4-4144-b06c-88382f99ddcf`, waited for human approval, executed it, and independently verified the recovery.
- The rules are acknowledged and the participant is registered.

Still required:

- Confirm the participant-specific form selections listed below.
- Record and publish the public narrated demo video.
- Add the video URL.
- Run `$submit-project` for the final preview and explicit confirmation.

## Known Limitations

- The app demonstrates one deterministic checkout incident and one safe rollback target.
- The recovery changes demo state. It does not connect to a real production deployment system.
- The Rust service and SQLite database run on an always-on personal Linux machine. The demo is unavailable if that machine or the active tunnel is offline.
- The app depends on a WebMCP-capable client. Firefox can display the workspace but cannot invoke its agent tools.
- Preview deployments cannot execute recovery mutations because the backend accepts only the stable production origin.

## TODO Official Form Fields

The live Devpost form requires these values:

| Field | Draft value |
|---|---|
| Submitter Type | TODO: Confirm `Individual`, `Team of Individuals`, or `Organization` |
| Country of residence | TODO: Confirm the participant and team countries |
| Organization name | Not applicable unless submitting for an organization |
| App Status | `New`. The repository was initialized during the submission period |
| Existing-project changes | Not applicable for a new project |
| Live URL | https://recovery-control-room.vercel.app |
| Testing instructions | Use the steps in this draft. No credentials are required |
| Public code repository | https://github.com/AnkanMisra/webmcp-project |
| Tested agents or clients | ChatGPT desktop in-app browser; Chrome for Testing 151 with native WebMCP; automated Playwright model-context harness |
| AI tools used | OpenAI Codex |
| Learning level | TODO: Confirm `None`, `Moderate`, or `Significant` |
| Career AI value | TODO: Confirm `Yes` or `No` |
| Demo video URL | TODO: Add the public YouTube URL |

The official form does not ask for a Codex session ID.
