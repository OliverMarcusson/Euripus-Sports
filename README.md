# Euripus Sports API

A Sweden-first sports discovery and watch-guidance API for Euripus.

This service ingests sports schedules and watch-source hints from configured sources, normalizes them, stores them in SQLite, and serves a stable HTTP API for live and upcoming events.

## What it does

- tracks live and upcoming sports events
- ranks likely watch providers with Sweden-first priority
- returns search hints Euripus can use to find playable content
- refreshes source data on startup and optionally on an interval
- supports fixture, HTTP, browser, and auto fetch modes

## Current competition coverage

Implemented or largely implemented:

- PGA Tour
- Allsvenskan
- Superettan
- Premier League
- UEFA Champions League
- FIFA World Cup 2026
- Formula 1
- SHL
- HockeyAllsvenskan
- Bandy Elitserien

## API endpoints

- `GET /health`
- `GET /v1/events/live`
- `GET /v1/events/upcoming?hours=72`
- `GET /v1/events/today`
- `GET /v1/events/{id}`
- `GET /v1/competitions/{slug}`
- `GET /v1/providers`

## Tech stack

- Rust
- Axum
- SQLite + `sqlx`
- YAML config
- `reqwest` + browser fallback for ingestion

## Project layout

- `src/` - API, ingestion, inference, persistence
- `config/` - providers, rules, sources, team aliases
- `tests/fixtures/` - deterministic test/dev fixtures
- `docs/api-and-euripus.md` - integration notes for Euripus
- `v1.md` - v1 scope and implementation status

## Run locally

### Fixture mode

Best for deterministic local development:

```bash
cargo run -- --listen 127.0.0.1:3000 --source-fetch-mode fixture
```

### Live/auto mode

Uses HTTP first and falls back to browser rendering when needed:

```bash
cargo run -- --listen 127.0.0.1:3000 --source-fetch-mode auto --browser-command chromium
```

### Periodic refresh every 10 minutes

```bash
cargo run -- --listen 127.0.0.1:3000 --source-fetch-mode auto --browser-command chromium --refresh-interval 10m
```

## Refresh without starting the server

```bash
cargo run -- --source-fetch-mode auto --browser-command chromium refresh
```

Note: flags must come before `refresh`.

## Docker

### Local build-from-source compose

```bash
docker compose up --build
```

The local compose setup binds the API to:

- `127.0.0.1:3000`

Current local compose defaults:

- `--source-fetch-mode auto`
- `--browser-command chromium`
- `--refresh-interval 10m`

### Publish to GHCR

To speed up deployments, this repo also supports publishing a prebuilt image to GitHub Container Registry.

Default image:

- `ghcr.io/olivermarcusson/euripus-sports-api`

Copy the example env file:

```bash
cp .env.selfhosted-images.example .env.selfhosted-images
```

Then publish:

```bash
bash scripts/publish-image.sh
```

Or just push to `main` and let GitHub Actions publish automatically to GHCR.

This pushes two tags:

- the current git SHA
- `selfhosted-latest`

### Server pull/deploy flow

Use the GHCR-based compose file on your server:

```bash
docker compose --env-file .env.selfhosted-images -f compose.selfhosted.yml pull
docker compose --env-file .env.selfhosted-images -f compose.selfhosted.yml up -d
```

Or use the helper script:

```bash
bash scripts/deploy-selfhosted.sh
```

See:

- `docs/SELF_HOSTED_DEPLOYMENT.md`

If pull fails with `not found`, the image has not been published yet. Publish it first, or set `SPORTS_API_IMAGE_TAG` to an existing SHA tag.

## Example requests

```bash
curl http://127.0.0.1:3000/health
curl http://127.0.0.1:3000/v1/events/live
curl "http://127.0.0.1:3000/v1/events/upcoming?hours=72"
curl http://127.0.0.1:3000/v1/competitions/pga_tour
```

## Configuration

Main config files:

- `config/providers.yaml`
- `config/competition_rules.yaml`
- `config/sample_events.yaml`
- `config/sources.yaml`
- `config/team_aliases.yaml`

The system is intentionally config-driven where possible so provider/rule/source behavior is not unnecessarily hardcoded.

## Database

Default local database:

```text
sqlite://sports-api.db
```

Override with:

```bash
cargo run -- --database-url sqlite:///tmp/sports-api.db --source-fetch-mode fixture
```

## Testing

```bash
cargo test -q
```

## Euripus integration

Euripus should use this service as a sports metadata and watch-guidance backend, not as a direct playback resolver.

See:

- `docs/api-and-euripus.md`

## Status / roadmap

See:

- `v1.md`

for completed, partial, and remaining v1 work.
