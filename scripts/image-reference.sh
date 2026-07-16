#!/usr/bin/env bash

is_immutable_image_ref() {
  local reference="${1:-}"
  [[ "$reference" =~ ^[^[:space:]]+@sha256:[0-9a-fA-F]{64}$ ]] \
    || [[ "$reference" =~ ^[^[:space:]]+:[0-9a-fA-F]{40}$ ]]
}

validate_image_ref() {
  local reference="${1:-}"
  if [[ -z "$reference" ]]; then
    echo "SPORTS_API_IMAGE_REF is required." >&2
    return 1
  fi
  if is_immutable_image_ref "$reference"; then
    return 0
  fi
  if [[ "${SPORTS_API_ALLOW_MUTABLE_IMAGE:-false}" == "true" ]]; then
    echo "WARNING: allowing mutable image reference for this run: $reference" >&2
    return 0
  fi
  echo "Refusing mutable or abbreviated image reference: $reference" >&2
  echo "Use an @sha256 digest or a tag containing a full 40-character Git SHA." >&2
  return 1
}
