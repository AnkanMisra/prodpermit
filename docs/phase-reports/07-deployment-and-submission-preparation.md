# Phase 7 deployment and submission preparation

Status: `in_progress`

## Outcome

The application, backend, and repository are public. The production recovery workflow passes all five Chromium journeys. Submission drafting remains blocked until the participant acknowledges the official Devpost rules.

## Live services

| Component | Address | State |
|---|---|---|
| Frontend | `https://recovery-control-room.vercel.app` | Ready |
| Backend ingress | `https://ankan-linux.tailf04855.ts.net` | Healthy |
| Source | `https://github.com/AnkanMisra/webmcp-project` | Public, MIT |

The API container binds only to `127.0.0.1:8080`. Tailscale Funnel terminates public HTTPS and proxies to the loopback service. The named Docker volume `recovery-control-room-api-data` stores SQLite.

## Production verification

The following checks passed on September 1, 2026:

- Both public health paths returned `{"data":{"status":"ok"}}`.
- The frontend returned the required WebMCP and security headers.
- Five Chromium journeys passed against the production URL.
- The journeys covered initial tool registration, investigation, human approval, execution, verification, reset, and capability revocation across reload.
- Restarting the API container preserved the SQLite file at the same volume inode and size.
- The container returned to the `healthy` state after restart.
- GitHub's unauthenticated API reported public repository visibility and the MIT license.

## Deployment record

- Backend source checkpoint: `808a063`
- Vercel project: `recovery-control-room`
- Vercel project ID: `prj_iITAQdIP6yUbpTrvCKmgEuNWrdTF`
- Production deployment ID: `dpl_CZGvpE1VWfk8mWyMgJXzLD97aH69`
- Production runtime: Node.js 24.x

The connected Vercel integration created the first production deployment. The project still needs a Git connection before pushes to `main` can deploy automatically.

## Remaining gates

1. The participant reviews the official terms and replies `yes`.
2. Codex prepares `devpost-submission.md` from the verified project and official form fields.
3. The participant publishes a narrated YouTube video under three minutes.
4. Codex runs the final readiness check.
5. The participant explicitly confirms the Devpost submission.

Nothing has been sent to Devpost.
