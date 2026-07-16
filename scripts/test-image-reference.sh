#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/image-reference.sh
source "$script_dir/image-reference.sh"

valid=(
  "ghcr.io/example/api@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  "ghcr.io/example/api:0123456789abcdef0123456789abcdef01234567"
)
invalid=(
  ""
  "ghcr.io/example/api:latest"
  "ghcr.io/example/api:selfhosted-latest"
  "ghcr.io/example/api:main"
  "ghcr.io/example/api:staging"
  "ghcr.io/example/api:0123456"
)
for reference in "${valid[@]}"; do
  validate_image_ref "$reference" >/dev/null
 done
for reference in "${invalid[@]}"; do
  if validate_image_ref "$reference" >/dev/null 2>&1; then
    echo "unexpectedly accepted '$reference'" >&2
    exit 1
  fi
done

echo "image reference validation passed"
