#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
env_file="${SPORTS_API_PUBLISH_ENV_FILE:-$repo_root/.env.selfhosted-images}"

platform="${SPORTS_API_PUBLISH_PLATFORM:-linux/amd64}"
moving_tag="${SPORTS_API_IMAGE_TAG:-selfhosted-latest}"
image_name="${SPORTS_API_IMAGE:-ghcr.io/olivermarcusson/euripus-sports-api}"
dockerfile_path="${SPORTS_API_DOCKERFILE:-Dockerfile}"

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

get_git_sha() {
  local sha
  sha="$(git -C "$repo_root" rev-parse HEAD | head -n 1 | tr -d '\r\n')"

  if [[ -z "$sha" ]]; then
    echo "Unable to resolve the current git SHA." >&2
    exit 1
  fi

  printf '%s' "$sha"
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

invoke_ghcr_login_if_configured() {
  if [[ -z "${GHCR_USERNAME:-}" || -z "${GHCR_TOKEN:-}" ]]; then
    echo "GHCR_USERNAME and GHCR_TOKEN are not both set in the environment or $env_file. Assuming you already ran 'docker login ghcr.io'." >&2
    return
  fi

  echo "Attempting GHCR login with configured credentials..."
  if ! printf '%s' "$GHCR_TOKEN" | docker login ghcr.io --username "$GHCR_USERNAME" --password-stdin; then
    echo "Configured GHCR credentials were rejected. Continuing with any existing Docker credential store entry for ghcr.io." >&2
  fi
}

publish_image() {
  local sha_tag="$1"
  local moving_tag_value="$2"

  echo "Publishing ${image_name}:${sha_tag} and ${image_name}:${moving_tag_value}..."
  docker buildx build \
    --platform "$platform" \
    --file "$repo_root/$dockerfile_path" \
    --tag "${image_name}:${sha_tag}" \
    --tag "${image_name}:${moving_tag_value}" \
    --push \
    "$repo_root"
}

assert_command_available docker
assert_command_available git

import_env_file "$env_file"

platform="${SPORTS_API_PUBLISH_PLATFORM:-$platform}"
moving_tag="${SPORTS_API_IMAGE_TAG:-$moving_tag}"
image_name="${SPORTS_API_IMAGE:-$image_name}"
dockerfile_path="${SPORTS_API_DOCKERFILE:-$dockerfile_path}"

sha_tag="$(get_git_sha)"

invoke_ghcr_login_if_configured
publish_image "$sha_tag" "$moving_tag"

echo
echo "Published Sports API image."
echo "Image: ${image_name}:${sha_tag}"
echo "Image: ${image_name}:${moving_tag}"
