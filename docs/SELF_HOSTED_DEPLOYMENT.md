# Euripus Sports API self-hosted deployment

This guide describes the lightweight GHCR-based deployment flow for the Sports API.

## Why use GHCR

Publishing the image once and pulling it on the server makes deployments much faster than rebuilding the Rust image directly on the host.

The flow is:

1. Build and push the image from your workstation.
2. Tag it with both the current git SHA and `selfhosted-latest`.
3. Pull the image on the server.
4. Start or update the service with `compose.selfhosted.yml`.

## Published image

Default image name:

- `ghcr.io/olivermarcusson/euripus-sports-api`

The publish script pushes two tags:

- the current git SHA
- `selfhosted-latest`

That gives you both:

- an immutable deploy target
- a convenient moving latest tag

## Prepare credentials

Copy:

```bash
cp .env.selfhosted-images.example .env.selfhosted-images
```

Then fill in:

- `GHCR_USERNAME`
- `GHCR_TOKEN`

For publishing, the token needs package write access.
For the server, the token only needs package read access.

## Publish from your workstation

Run:

```bash
bash scripts/publish-image.sh
```

The script:

- loads `.env.selfhosted-images` if present
- optionally logs in to `ghcr.io`
- builds a `linux/amd64` image with `docker buildx`
- pushes both the SHA tag and `selfhosted-latest`

Useful overrides:

```bash
SPORTS_API_IMAGE_TAG=staging bash scripts/publish-image.sh
SPORTS_API_IMAGE=ghcr.io/olivermarcusson/custom-sports-api bash scripts/publish-image.sh
SPORTS_API_PUBLISH_PLATFORM=linux/arm64 bash scripts/publish-image.sh
```

## Deploy on the server

Copy `.env.selfhosted-images` to the server and make sure it contains:

- `GHCR_USERNAME`
- `GHCR_TOKEN`
- optionally `SPORTS_API_IMAGE_TAG` if you want a specific SHA instead of `selfhosted-latest`

Log in once:

```bash
source .env.selfhosted-images
printf '%s' "$GHCR_TOKEN" | docker login ghcr.io --username "$GHCR_USERNAME" --password-stdin
```

Then deploy with:

```bash
docker compose --env-file .env.selfhosted-images -f compose.selfhosted.yml pull
docker compose --env-file .env.selfhosted-images -f compose.selfhosted.yml up -d
```

Or use the helper script:

```bash
bash scripts/deploy-selfhosted.sh
```

The deploy script prefers `docker` and automatically falls back to `podman` if `docker` is not installed.

To pin a specific build, set:

```bash
SPORTS_API_IMAGE_TAG=<git-sha>
```

in `.env.selfhosted-images` before pulling.

## Validation

Check the running image:

```bash
docker compose --env-file .env.selfhosted-images -f compose.selfhosted.yml images
```

Check health:

```bash
curl http://127.0.0.1:3000/health
```

## Notes

- `compose.yaml` remains the local build-from-source setup.
- `compose.selfhosted.yml` is the server/pull-from-GHCR setup.
- The image includes Chromium so browser-mode ingestion works on the server.
