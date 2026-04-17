#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
env_file="${SPORTS_API_DEPLOY_ENV_FILE:-$repo_root/.env.selfhosted-images}"
compose_file="$repo_root/compose.selfhosted.yml"
compose_args=( -f "$compose_file" )

trim() {
  local value="$1"
  value="${value#"${value%%[![:space:]]*}"}"
  value="${value%"${value##*[![:space:]]}"}"
  printf '%s' "$value"
}

assert_command_available() {
  local command_name="$1"

  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "Required command '$command_name' was not found on PATH." >&2
    exit 1
  fi
}

import_env_file() {
  local path="$1"

  [[ -f "$path" ]] || return 0

  while IFS= read -r line || [[ -n "$line" ]]; do
    local trimmed_line separator_index name value

    trimmed_line="$(trim "$line")"

    if [[ -z "$trimmed_line" || "${trimmed_line:0:1}" == "#" ]]; then
      continue
    fi

    separator_index="$(expr index "$trimmed_line" '=')"
    if [[ "$separator_index" -le 1 ]]; then
      continue
    fi

    name="$(trim "${trimmed_line::separator_index-1}")"
    value="$(trim "${trimmed_line:separator_index}")"

    if [[ "$value" == \"*\" && "$value" == *\" ]]; then
      value="${value:1:${#value}-2}"
    elif [[ "$value" == \'*\' && "$value" == *\' ]]; then
      value="${value:1:${#value}-2}"
    fi

    export "$name=$value"
  done < "$path"
}

pick_container_cli() {
  if command -v docker >/dev/null 2>&1; then
    printf 'docker'
    return
  fi

  if command -v podman >/dev/null 2>&1; then
    printf 'podman'
    return
  fi

  echo "Neither docker nor podman was found on PATH." >&2
  exit 1
}

login_ghcr_if_configured() {
  local container_cli="$1"

  if [[ -z "${GHCR_USERNAME:-}" || -z "${GHCR_TOKEN:-}" ]]; then
    echo "GHCR_USERNAME and GHCR_TOKEN are not both set in the environment or $env_file. Assuming you already logged in to ghcr.io." >&2
    return
  fi

  echo "Logging in to ghcr.io..."
  printf '%s' "$GHCR_TOKEN" | "$container_cli" login ghcr.io --username "$GHCR_USERNAME" --password-stdin
}

if [[ -f "$env_file" ]]; then
  import_env_file "$env_file"
  compose_args=( --env-file "$env_file" -f "$compose_file" )
else
  echo "No env file found at $env_file. Using compose defaults and any existing container registry login." >&2
fi

container_cli="$(pick_container_cli)"
assert_command_available "$container_cli"
"$container_cli" compose version >/dev/null

login_ghcr_if_configured "$container_cli"

echo "Pulling image(s)..."
"$container_cli" compose "${compose_args[@]}" pull

echo "Starting container(s)..."
"$container_cli" compose "${compose_args[@]}" up -d

echo
echo "Deployment complete."
"$container_cli" compose "${compose_args[@]}" images
