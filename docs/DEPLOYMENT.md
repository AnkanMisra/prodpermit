# Deploy Recovery Control Room for free

This runbook deploys the Next.js frontend to Vercel Hobby and keeps the Rust API with SQLite on Ankan-Linux. Tailscale Funnel publishes the API over HTTPS. The setup uses provider domains and requires no paid storage or database migration.

## Public topology

| Component | Location | Public address |
|---|---|---|
| Source | GitHub | `https://github.com/AnkanMisra/webmcp-project` |
| Frontend | Vercel Hobby | `https://recovery-control-room.vercel.app` or the assigned production hostname |
| Rust and SQLite | Docker on Ankan-Linux | Loopback port `127.0.0.1:8080` |
| API ingress | Tailscale Funnel | `https://ankan-linux.tailf04855.ts.net` |
| Demo video | YouTube | Public video with audio under three minutes |

SQLite must stay with the Rust service. The recovery transaction depends on a single local database writer and a persistent filesystem.

## Prepare the machine

Keep Ankan-Linux powered and connected through judging. Disable automatic suspension while the machine is plugged in.

Enable Docker and Tailscale at boot:

```bash
sudo systemctl enable --now docker
sudo systemctl enable --now tailscaled
```

Create the ignored production environment file:

```bash
cp deploy/backend.env.example .env.backend
```

Set `ALLOWED_ORIGIN` to the exact Vercel production origin. Do not add a trailing slash. Keep `SECURE_COOKIE=true`.

## Deploy the backend

Deploy only from a clean, verified `main` checkout:

```bash
git pull --ff-only origin main
./scripts/deploy-backend.sh
```

The script runs the Rust gate, builds the API image, starts it with a persistent Docker volume, and waits for the health endpoint.

Check local state:

```bash
curl --fail http://127.0.0.1:8080/api/health
docker compose --env-file .env.backend -f deploy/backend.compose.yml ps
docker volume inspect recovery-control-room-api-data
```

## Publish the API with Funnel

Enable the HTTPS Funnel once:

```bash
sudo tailscale funnel --bg http://127.0.0.1:8080
tailscale funnel status
curl --fail https://ankan-linux.tailf04855.ts.net/api/health
```

The `--bg` configuration resumes after Tailscale or the machine restarts. Port 8080 remains bound to loopback, so Funnel is the only public API ingress.

## Configure Vercel

Import `AnkanMisra/webmcp-project` as a Vercel project with these settings:

- Project name: `recovery-control-room`
- Production branch: `main`
- Root Directory: `apps/web`
- Include source files outside Root Directory: enabled
- Framework: Next.js
- Node.js: 24.x
- Build command: `bun run build`
- Output directory: Next.js default

Set this Production and Preview environment variable:

```text
BACKEND_URL=https://ankan-linux.tailf04855.ts.net
```

Store `BACKEND_URL` as a Vercel Config value. It is a public hostname, not a secret.

Preview builds are build checks only. The Rust API accepts the exact production Vercel origin.

If Vercel assigns a different production hostname, update `ALLOWED_ORIGIN` in `.env.backend`, redeploy the backend, and then redeploy Vercel.

## Verify production

Check the two public paths:

```bash
curl --fail https://ankan-linux.tailf04855.ts.net/api/health
curl --fail https://recovery-control-room.vercel.app/api/backend/health
```

The machine's MagicDNS resolver maps the Funnel hostname to its private `100.x` address. That result does not prove that the public Funnel edge works. Resolve the public address through a public DNS server and test it explicitly:

```bash
for public_ip in $(dig @1.1.1.1 +short ankan-linux.tailf04855.ts.net A); do
  curl --resolve "ankan-linux.tailf04855.ts.net:443:$public_ip" \
    --fail https://ankan-linux.tailf04855.ts.net/api/health
done
```

Test every returned address. A failure on one address with a success on another identifies a Tailscale public-edge problem, not a Rust or SQLite failure. The Vercel rewrite health check remains the judge-path check.

If local health passes but the public edge or Vercel rewrite stalls, refresh the Funnel relay state:

```bash
tailscale funnel --https=443 off
tailscale funnel --bg http://127.0.0.1:8080
```

Then repeat both public health checks before asking an agent to invoke a tool.

Confirm the frontend sends these headers:

```text
Origin-Agent-Cluster: ?1
Permissions-Policy: tools=(self)
X-Content-Type-Options: nosniff
```

Run the browser journey:

1. Confirm six initial tools and no execution tool.
2. Run the database diagnostic.
3. Prepare the rollback to `release_283`.
4. Approve the exact displayed fingerprint.
5. Confirm the execution tool appears.
6. Execute and confirm the tool disappears.
7. Verify healthy `release_283` and the audit timeline.
8. Reset and confirm the broken scenario returns.

Confirm persistence by restarting the container and reloading the same browser session:

```bash
docker compose --env-file .env.backend -f deploy/backend.compose.yml restart api
curl --fail https://recovery-control-room.vercel.app/api/backend/health
```

## Update and roll back

Frontend deployments follow verified pushes to `main`. Backend deployments remain manual:

```bash
git pull --ff-only origin main
./scripts/deploy-backend.sh
```

Record `git rev-parse HEAD` before updating. If the backend smoke test fails, check out that recorded commit, run the deployment script, then return the checkout to `main` after service recovery.

Vercel can instantly move the production alias back to a previously verified deployment from the dashboard.

The first production deployment used the connected Vercel integration. The project now connects to `AnkanMisra/webmcp-project` with `apps/web` as its root directory. Verified pushes to `main` trigger production deployments.

## Stop public access

Turn off Funnel without deleting SQLite data:

```bash
sudo tailscale funnel --https=443 off
```

Stop the backend while preserving the Docker volume:

```bash
docker compose --env-file .env.backend -f deploy/backend.compose.yml down
```

Do not add `--volumes` unless permanent deletion of the SQLite database is intended.
