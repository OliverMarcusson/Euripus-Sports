#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
default_env_file="${XDG_CONFIG_HOME:-$HOME/.config}/euripus-sports/images.env"
env_file="${SPORTS_API_DEPLOY_ENV_FILE:-$default_env_file}"
legacy_env_file="$repo_root/.env.selfhosted-images"
compose_file="$repo_root/compose.selfhosted.yml"
compose_args=( -f "$compose_file" )
# shellcheck source=scripts/image-reference.sh
source "$repo_root/scripts/image-reference.sh"

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
    [[ -z "$trimmed_line" || "${trimmed_line:0:1}" == "#" ]] && continue
    separator_index="$(expr index "$trimmed_line" '=')"
    [[ "$separator_index" -le 1 ]] && continue
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
  elif command -v podman >/dev/null 2>&1; then
    printf 'podman'
  else
    echo "Neither docker nor podman was found on PATH." >&2
    exit 1
  fi
}

login_ghcr_if_configured() {
  local container_cli="$1"
  if [[ -z "${GHCR_USERNAME:-}" || -z "${GHCR_TOKEN:-}" ]]; then
    echo "GHCR credentials are not set; using the existing registry login." >&2
    return
  fi
  printf '%s' "$GHCR_TOKEN" | "$container_cli" login ghcr.io --username "$GHCR_USERNAME" --password-stdin
}

if [[ ! -f "$env_file" && -z "${SPORTS_API_DEPLOY_ENV_FILE:-}" && -f "$legacy_env_file" ]]; then
  echo "WARNING: $legacy_env_file is deprecated; move it to $default_env_file with mode 0600." >&2
  env_file="$legacy_env_file"
fi
if [[ -f "$env_file" ]]; then
  import_env_file "$env_file"
  compose_args=( --env-file "$env_file" -f "$compose_file" )
else
  echo "No env file found at $env_file; using exported environment variables." >&2
fi

image_ref="${SPORTS_API_IMAGE_REF:-}"
validate_image_ref "$image_ref"
container_cli="$(pick_container_cli)"
assert_command_available "$container_cli"
"$container_cli" compose version >/dev/null
login_ghcr_if_configured "$container_cli"

"$container_cli" compose "${compose_args[@]}" pull
pulled_id="$("$container_cli" image inspect --format '{{.Id}}' "$image_ref")"
repo_digests="$("$container_cli" image inspect --format '{{join .RepoDigests "\n"}}' "$image_ref")"
echo "Pulled image ID: $pulled_id"
echo "Registry digest(s):"
printf '%s\n' "$repo_digests"
if [[ "$image_ref" == *@sha256:* ]]; then
  requested_digest="${image_ref##*@}"
  if [[ "$repo_digests" != *"@$requested_digest"* ]]; then
    echo "Pulled image does not report requested digest $requested_digest" >&2
    exit 1
  fi
fi

"$container_cli" compose "${compose_args[@]}" up -d
container_id="$("$container_cli" compose "${compose_args[@]}" ps -q sports-api)"
running_id="$("$container_cli" inspect --format '{{.Image}}' "$container_id")"
if [[ "$running_id" != "$pulled_id" ]]; then
  echo "Running container image $running_id differs from pulled image $pulled_id" >&2
  exit 1
fi

echo "Deployment complete. Running image ID: $running_id"
"$container_cli" compose "${compose_args[@]}" images
