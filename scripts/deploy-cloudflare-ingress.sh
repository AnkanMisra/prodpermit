#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
compose_file="$repo_root/deploy/cloudflare-quick-tunnel.compose.yml"
compose_project="recovery-control-room-cloudflare"
vercel_project="prj_iITAQdIP6yUbpTrvCKmgEuNWrdTF"
vercel_scope="ankanmisras-projects"
vercel_origin="https://recovery-control-room.vercel.app"
vercel_package="vercel@59.11.1"

for required_command in bunx curl dig docker jq; do
  if ! command -v "$required_command" >/dev/null 2>&1; then
    echo "Missing required command: $required_command" >&2
    exit 1
  fi
done

docker compose \
  --project-name "$compose_project" \
  --file "$compose_file" \
  up --detach --force-recreate --pull always

tunnel_url=""
for _ in $(seq 1 45); do
  tunnel_url="$(
    docker compose \
      --project-name "$compose_project" \
      --file "$compose_file" \
      logs --no-color cloudflared 2>&1 |
      sed -nE 's#.*(https://[a-z0-9-]+\.trycloudflare\.com).*#\1#p' |
      tail -1
  )"
  if [[ -n "$tunnel_url" ]]; then
    break
  fi
  sleep 1
done

if [[ ! "$tunnel_url" =~ ^https://[a-z0-9-]+\.trycloudflare\.com$ ]]; then
  echo "Cloudflare did not return a valid Quick Tunnel URL." >&2
  exit 1
fi

tunnel_ready="false"
tunnel_host="${tunnel_url#https://}"
for _ in $(seq 1 45); do
  while read -r public_ip; do
    if curl --resolve "$tunnel_host:443:$public_ip" \
      --fail --silent --show-error --max-time 5 \
      "$tunnel_url/api/health" >/dev/null 2>&1; then
      tunnel_ready="true"
      break 2
    fi
  done < <(dig @1.1.1.1 +short "$tunnel_host" A)
  sleep 1
done

if [[ "$tunnel_ready" != "true" ]]; then
  echo "Cloudflare did not route the Quick Tunnel before the timeout." >&2
  exit 1
fi

latest_production_url="$(
  bunx "$vercel_package" list recovery-control-room \
    --prod \
    --format json \
    --scope "$vercel_scope" |
    jq -er '.deployments | map(select(.target == "production" and .state == "READY")) | first | .url'
)"

bunx "$vercel_package" env add BACKEND_URL production,preview \
  --project "$vercel_project" \
  --value "$tunnel_url" \
  --force \
  --no-sensitive \
  --yes \
  --scope "$vercel_scope"

redeploy_output="$(
  bunx "$vercel_package" redeploy "$latest_production_url" \
    --target production \
    --no-wait \
    --scope "$vercel_scope" 2>&1
)"
printf '%s\n' "$redeploy_output"

deployment_url="$(
  sed -nE 's#.*(https://recovery-control-room-[a-z0-9-]+\.vercel\.app).*#\1#p' \
    <<<"$redeploy_output" |
    tail -1
)"
if [[ ! "$deployment_url" =~ ^https://recovery-control-room-[a-z0-9-]+\.vercel\.app$ ]]; then
  echo "Vercel did not return the new production deployment URL." >&2
  exit 1
fi

bunx "$vercel_package" inspect "$deployment_url" \
  --wait \
  --timeout 3m \
  --scope "$vercel_scope" >/dev/null

for _ in $(seq 1 45); do
  if curl --fail --silent --show-error --max-time 5 \
    "$vercel_origin/api/backend/health" >/dev/null 2>&1; then
    printf 'cloudflare_url=%s\nvercel_health=%s/api/backend/health\n' \
      "$tunnel_url" "$vercel_origin"
    exit 0
  fi
  sleep 1
done

echo "Vercel did not reach the Cloudflare-backed API before the timeout." >&2
exit 1
