#!/usr/bin/env bash
set -euo pipefail

vercel_project="prj_iITAQdIP6yUbpTrvCKmgEuNWrdTF"
vercel_scope="ankanmisras-projects"
vercel_origin="https://recovery-control-room.vercel.app"
vercel_package="vercel@59.11.1"
cloudflared_image="cloudflare/cloudflared@sha256:51c9cefcb4569df44e1ad403ab1d3d8065aa8e84339bcfc6aee75502e1140339"
new_container="recovery-control-room-cloudflared-$(date +%s%N)"

for required_command in bunx curl dig docker jq; do
  if ! command -v "$required_command" >/dev/null 2>&1; then
    echo "Missing required command: $required_command" >&2
    exit 1
  fi
done

docker run --detach \
  --name "$new_container" \
  --restart unless-stopped \
  --network host \
  --read-only \
  --cap-drop ALL \
  --security-opt no-new-privileges:true \
  --log-driver json-file \
  --log-opt max-size=10m \
  --log-opt max-file=3 \
  "$cloudflared_image" \
  tunnel --no-autoupdate --protocol http2 --url http://127.0.0.1:8080 >/dev/null

tunnel_url=""
for _ in $(seq 1 45); do
  tunnel_url="$(
    docker logs "$new_container" 2>&1 |
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

current_production_json="$(
  bunx "$vercel_package" inspect "$vercel_origin" \
    --json \
    --scope "$vercel_scope"
)"
current_production_url="$(jq -er '.url' <<<"$current_production_json")"

bunx "$vercel_package" env add BACKEND_URL production,preview \
  --project "$vercel_project" \
  --value "$tunnel_url" \
  --force \
  --no-sensitive \
  --yes \
  --scope "$vercel_scope"

redeploy_output="$(
  bunx "$vercel_package" redeploy "$current_production_url" \
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

new_deployment_json="$(
  bunx "$vercel_package" inspect "$deployment_url" \
    --wait \
    --timeout 3m \
    --json \
    --scope "$vercel_scope"
)"
new_deployment_id="$(jq -er '.id' <<<"$new_deployment_json")"

alias_ready="false"
for _ in $(seq 1 30); do
  alias_deployment_id="$(
    bunx "$vercel_package" inspect "$vercel_origin" \
      --json \
      --scope "$vercel_scope" |
      jq -er '.id'
  )"
  if [[ "$alias_deployment_id" == "$new_deployment_id" ]]; then
    alias_ready="true"
    break
  fi
  sleep 2
done

if [[ "$alias_ready" != "true" ]]; then
  echo "The production alias did not move to the new deployment." >&2
  exit 1
fi

for _ in $(seq 1 45); do
  if curl --fail --silent --show-error --max-time 5 \
    "$vercel_origin/api/backend/health" >/dev/null 2>&1; then
    while read -r old_container; do
      if [[ -n "$old_container" && "$old_container" != "$new_container" ]]; then
        docker rm --force "$old_container" >/dev/null
      fi
    done < <(
      docker ps --all --format '{{.Names}}' |
        grep -E '^recovery-control-room-cloudflared-[0-9]+$|^recovery-control-room-cloudflare-cloudflared-1$' ||
        true
    )
    printf 'cloudflare_url=%s\nvercel_health=%s/api/backend/health\n' \
      "$tunnel_url" "$vercel_origin"
    exit 0
  fi
  sleep 1
done

echo "Vercel did not reach the Cloudflare-backed API before the timeout." >&2
exit 1
