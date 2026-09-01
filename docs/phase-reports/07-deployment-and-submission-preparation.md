# Phase 7 deployment and submission preparation

Status: `in_progress`

## Outcome

The application, backend, and repository are public. The production recovery workflow passes all five Chromium journeys. The participant acknowledged the official rules, and `devpost-submission.md` now contains the verified project story, official form fields, testing instructions, screenshot list, and video outline.

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
- ChatGPT's in-app browser then completed the public recovery flow after human approval and independently verified healthy `release_283`.
- Restarting the API container preserved the SQLite file at the same volume inode and size.
- The container returned to the `healthy` state after restart.
- GitHub's unauthenticated API reported public repository visibility and the MIT license.

## Deployment record

- Backend source checkpoint: `808a063`
- Vercel project: `recovery-control-room`
- Vercel project ID: `prj_iITAQdIP6yUbpTrvCKmgEuNWrdTF`
- Production deployment ID: `dpl_CZGvpE1VWfk8mWyMgJXzLD97aH69`
- Production runtime: Node.js 24.x

The connected Vercel integration created the first production deployment. GitHub now connects to the same project with `apps/web` as its root directory, Bun frozen installs, Node.js 24.x, and `BACKEND_URL` in the Production and Preview environments.

## Remaining gates

1. The participant confirms the submitter type, country, learning level, and career AI value.
2. The participant publishes a narrated YouTube video under three minutes.
3. Codex adds the video URL and runs the final readiness check.
4. The participant explicitly confirms the Devpost submission.

Nothing has been sent to Devpost.
