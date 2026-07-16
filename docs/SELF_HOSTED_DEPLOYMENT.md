# Euripus Sports API self-hosted deployment

The production Compose flow pulls a prebuilt GHCR image and requires an immutable image selection. A registry digest is preferred; a tag containing the full 40-character Git SHA is also accepted. `selfhosted-latest` is published only as a discovery convenience and is never a deployment default.

## Credentials and configuration

Prefer `docker login ghcr.io` with a credential helper or a CI secret store. If an env file is required, keep it outside the repository and restrict it to the account running deployment:

```bash
install -d -m 0700 "$HOME/.config/euripus-sports"
install -m 0600 .env.selfhosted-images.example "$HOME/.config/euripus-sports/images.env"
$EDITOR "$HOME/.config/euripus-sports/images.env"
```

The scripts default to `${XDG_CONFIG_HOME:-$HOME/.config}/euripus-sports/images.env`. `SPORTS_API_PUBLISH_ENV_FILE` and `SPORTS_API_DEPLOY_ENV_FILE` override that location. The old repository-root `.env.selfhosted-images` is accepted temporarily with a deprecation warning and is excluded from Docker build context.

Set `SPORTS_API_IMAGE_REF` to one of:

```text
ghcr.io/olivermarcusson/euripus-sports-api@sha256:<64-hex-digest>
ghcr.io/olivermarcusson/euripus-sports-api:<full-40-character-git-sha>
```

Missing references, moving tags (`latest`, `selfhosted-latest`, `main`, `staging`), arbitrary tags, and short SHAs are rejected. A one-run emergency exception requires the explicit `SPORTS_API_ALLOW_MUTABLE_IMAGE=true` environment variable and prints a warning; do not store that override in configuration.

## Publish

```bash
bash scripts/publish-image.sh
```

The publisher pushes the full Git SHA tag and the `selfhosted-latest` convenience tag. Publishing credentials need package write access. The server needs only package read access.

## Deploy and roll back

```bash
bash scripts/deploy-selfhosted.sh
```

The deploy script validates the image reference before login or pull, prints the pulled image ID and registry digest, starts the service, and verifies that the container image ID matches what was pulled. For digest input it also verifies that the requested digest appears in the resolved repository digests.

Rollback means restoring the previous immutable `SPORTS_API_IMAGE_REF` and running the deploy script again. Preserve prior references in deployment records rather than relying on a moving tag.

## Existing data volume migration

The runtime now uses UID/GID `10001:10001`. Before upgrading an existing root-owned database volume:

1. Stop the service and take a tested backup of the SQLite database/volume.
2. Change ownership of the mounted `/data` contents to `10001:10001` using a narrowly scoped one-off maintenance command.
3. Start the hardened service and confirm readiness.

Fresh named volumes inherit `/data` ownership from the image. Bind mounts require host-side ownership to be set explicitly.

## Runtime hardening and validation

Both Compose definitions retain host-loopback port publication and add an unprivileged user, read-only root filesystem, all-capability drop, `no-new-privileges`, an init process, a PID limit, and bounded writable `/tmp` and `/dev/shm` tmpfs mounts. `/data` is the only persistent writable mount. Chromium runs without `--no-sandbox`.

The deployment kernel/runtime and seccomp policy must support Chromium's unprivileged user-namespace sandbox. A host sysctl enabling user namespaces is not sufficient if the container runtime's default seccomp profile blocks the required namespace operation. Validate the exact target environment; if necessary, use a narrowly reviewed Chromium-compatible seccomp profile or isolate browser ingestion. Never compensate with `--no-sandbox`, privileged mode, `seccomp:unconfined`, or broad capabilities:

```bash
docker exec sports-api id
docker exec sports-api sh -c 'test "$(id -u)" = 10001 && ! touch /app/rootfs-write-test'
docker exec sports-api chromium --headless=new --dump-dom 'data:text/html,<p>sandbox-ok</p>'
curl -fsS http://127.0.0.1:3000/health/live
curl -fsS http://127.0.0.1:3000/health/ready
docker inspect sports-api
```

`/health/live` is process liveness. `/health/ready` checks SQLite plus successful refresh freshness and is used by Compose healthchecks. The default maximum refresh age is 30 minutes; operators using manual or less frequent refreshes must set a larger `SPORTS_API_READINESS_MAX_REFRESH_AGE` explicitly.
