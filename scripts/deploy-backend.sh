#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
repo_root=$(cd "$script_dir/.." && pwd -P)
compose_file="$repo_root/deploy/backend.compose.yml"
backend_env_file=${BACKEND_ENV_FILE:-"$repo_root/.env.backend"}

if [[ ! -f "$backend_env_file" ]]; then
  echo "Missing $backend_env_file. Copy deploy/backend.env.example and set the production origin." >&2
  exit 1
fi

for required_key in ALLOWED_ORIGIN DATABASE_URL SECURE_COOKIE PORT RUST_LOG; do
  if ! rg --quiet "^${required_key}=" "$backend_env_file"; then
    echo "Missing ${required_key} in $backend_env_file." >&2
    exit 1
  fi
done

if [[ -n $(git -C "$repo_root" status --porcelain) ]]; then
  echo "Refusing to deploy from a dirty Git worktree." >&2
  git -C "$repo_root" status --short >&2
  exit 1
fi

cd "$repo_root"

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

docker compose --env-file "$backend_env_file" -f "$compose_file" config --quiet
docker compose --env-file "$backend_env_file" -f "$compose_file" up -d --build

health_url="http://127.0.0.1:8080/api/health"
for attempt in $(seq 1 60); do
  if curl --fail --silent --show-error "$health_url" >/dev/null; then
    docker compose --env-file "$backend_env_file" -f "$compose_file" ps
    echo "Backend healthy at commit $(git rev-parse --short HEAD)."
    exit 0
  fi
  sleep 1
done

docker compose --env-file "$backend_env_file" -f "$compose_file" logs --tail 100 api >&2
echo "Backend did not become healthy within 60 seconds." >&2
exit 1
