# Euripus Sports API: how it works and how to integrate it into Euripus

## 1. What this service does

The Sports API is a **Sweden-first sports discovery and watch-guidance service**.

It does **not** try to deep-link directly into exact playback inside Euripus yet.
Instead, it gives Euripus enough structured information to:

- show live and upcoming sports events
- recommend the most likely provider for each event
- suggest the best event label or channel label to search for
- expose fallback watch options for other markets
- keep scraping and source complexity outside of Euripus itself

In practice, Euripus should treat this service as a **sports metadata + watch guidance backend**.

---

## 2. Current implementation status

### Implemented competition ingestion

Fully or mostly implemented:
- PGA Tour
- Allsvenskan
- Premier League
- UEFA Champions League
- FIFA World Cup
- SHL
- HockeyAllsvenskan
- Bandy Elitserien
- Superettan (currently reliable via Svensk Fotboll article-style source rather than the league matcher page)

Configured in rules, but canonical ingestion is not finished yet:
- no major remaining competition in the original high-priority list besides deeper watch-source coverage and source hardening

### Implemented watch guidance

Implemented:
- TV4 Play overlays for Allsvenskan
- TV4 Play overlays for SHL
- TV4 Play overlays for HockeyAllsvenskan
- Viaplay overlays for Premier League
- Viaplay overlays for Champions League
- PGA Tour US watch windows from the PGA Tour broadcast schedule
- PGA Tour Sweden watch guidance from Svensk Golf
- competition-level fallback rules for Sweden / US / UK
- Bandy provider guidance via Bonnier-family competition rules

---

## 3. How the API works

The service has two separate responsibilities:

1. **Ingest and normalize sports data**
2. **Serve stable API responses from SQLite**

The API does **not** scrape sources during normal request handling.
Scraping happens during refresh. Requests read from the local database.

### 3.1 Data flow

The flow is:

1. Load config from YAML:
   - `config/providers.yaml`
   - `config/competition_rules.yaml`
   - `config/sample_events.yaml`
   - `config/sources.yaml`
   - `config/team_aliases.yaml`
2. Load configured sources from `config/sources.yaml`
3. Fetch each source using one of four modes:
   - `fixture`
   - `http`
   - `browser`
   - `auto`
4. Parse sources into one of two internal forms:
   - `EventSeed` for canonical events
   - `WatchOverlay` for source-specific watch clues
5. Merge source-derived events and watch overlays into the effective config
6. Hydrate each event into a final API `Event` by:
   - applying competition rules
   - matching overlays to events
   - generating ranked watch availabilities
   - generating search hints
7. Store final events and watch availabilities in SQLite
8. Serve HTTP requests from SQLite

### 3.2 Why SQLite is used

SQLite is the serving layer for v1. That gives:

- stable API responses even if source pages are temporarily down
- separation between refresh jobs and request handling
- easy local development
- a simple operational model for Euripus to depend on

### 3.3 Fetch modes

The fetch mode is controlled by `--source-fetch-mode`.

- `fixture`
  - reads saved fixture content from `tests/fixtures/`
  - best for tests and deterministic local development
- `http`
  - fetches source pages directly with `reqwest`
- `browser`
  - fetches via a browser renderer
  - supports `agent-browser` and Chromium-style `--dump-dom` rendering
  - useful for JS-heavy pages or Cloudflare-protected pages
- `auto`
  - tries HTTP first
  - falls back to browser automation on failure

### 3.4 Browser fallback

Some sports sites are Cloudflare-protected or render poorly to plain HTTP.
When `auto` mode is enabled, the fetcher:

1. tries a normal HTTP request
2. detects failures such as Cloudflare block pages
3. retries through browser rendering (`agent-browser` or Chromium, depending on `--browser-command`)

This keeps scraping logic out of Euripus.

### 3.5 Ranking logic

Each event gets a ranked list of `watch.availabilities`.

Ranking currently favors:

1. Sweden over US over UK
2. `ppv-event` over streaming over generic linear references
3. higher confidence sources over weaker ones
4. source-specific overlays above generic competition rules

That means, for example:
- a TV4 Play event listing for a specific Allsvenskan match will beat a generic `TV4 Fotboll` rule
- a Viaplay event listing for a Champions League match will beat a generic `V Sport Football` rule
- golf round-specific overlays are matched with round/title awareness so Round 3 data does not incorrectly attach to Round 1

### 3.6 Search hints

Each watch availability includes `search_hints`.
These are intended to be used by Euripus when trying to find the right playable item in its own catalog.

Examples:
- `Halmstads BK - IFK Göteborg`
- `Halmstads BK vs IFK Göteborg TV4 Play`
- `TV4 Fotboll Halmstads BK vs IFK Göteborg`
- `Paris Saint-Germain vs Bayern Munich Viaplay`

This is the main bridge between the sports API and Euripus until exact playback mapping exists.

---

## 4. Running the API

## 4.1 Start the server

```bash
cargo run -- --source-fetch-mode fixture
```

Custom listen address:

```bash
cargo run -- --listen 127.0.0.1:3000 --source-fetch-mode fixture
```

Using browser fallback in production-like mode:

```bash
cargo run -- --source-fetch-mode auto --browser-command chromium
```

Using built-in periodic refresh every 10 minutes:

```bash
cargo run -- --source-fetch-mode auto --refresh-interval 10m --browser-command chromium
```

`--refresh-interval` enables an in-process background refresh loop. The server still refreshes once on startup, then continues refreshing on the configured interval.

## 4.2 Run a refresh without starting the server

```bash
cargo run -- --source-fetch-mode fixture refresh
```

Important: CLI flags must appear **before** the `refresh` subcommand.
This works:

```bash
cargo run -- --source-fetch-mode fixture refresh
```

This does **not** work:

```bash
cargo run -- refresh --source-fetch-mode fixture
```

## 4.3 Default local database

By default the server uses:

```text
sqlite://sports-api.db
```

You can override it:

```bash
cargo run -- --database-url sqlite:///tmp/sports-api.db --source-fetch-mode fixture
```

---

## 5. HTTP API reference

Base URL examples assume local development:

```text
http://127.0.0.1:3000
```

## 5.1 Health

### `GET /health`

Returns service liveness.

Example response:

```json
{
  "status": "ok"
}
```

## 5.2 Live events

### `GET /v1/events/live`

Returns events that are currently live or whose current time falls between `start_time` and `end_time`.

Response shape:

```json
{
  "count": 1,
  "events": [ ... ]
}
```

## 5.3 Upcoming events

### `GET /v1/events/upcoming?hours=72`

Returns events starting between now and `now + hours`.

Query params:
- `hours` optional, default `72`

Example:

```bash
curl 'http://127.0.0.1:3000/v1/events/upcoming?hours=168'
```

## 5.4 Today events

### `GET /v1/events/today`

Currently implemented as a **rolling next 24 hours** view.
It is not a strict local-calendar-day endpoint.

## 5.5 Event detail

### `GET /v1/events/{id}`

Returns one event object.

Example:

```bash
curl 'http://127.0.0.1:3000/v1/events/pga_tour_2026_rbc_heritage_round_1'
```

## 5.6 Competition view

### `GET /v1/competitions/{slug}`

Returns all stored events for a competition.

Example slugs currently useful:
- `allsvenskan`
- `superettan`
- `premier_league`
- `uefa_champions_league`
- `pga_tour`

Example:

```bash
curl 'http://127.0.0.1:3000/v1/competitions/allsvenskan'
```

## 5.7 Provider catalog

### `GET /v1/providers`

Returns the configured provider catalog.
This is useful if Euripus wants to map provider families to local branding or icons.

Example response shape:

```json
{
  "count": 11,
  "providers": [
    {
      "family": "tv4",
      "market": "se",
      "aliases": ["TV4 Play", "TV4 Sport", "TV4 Hockey", "TV4 Fotboll", "Sportkanalen"]
    }
  ]
}
```

---

## 6. Response model

The central object is `Event`.

## 6.1 Event fields

```json
{
  "id": "allsvenskan_2026_halmstads_bk_mot_ifk_goteborg",
  "sport": "soccer",
  "competition": "allsvenskan",
  "title": "Halmstads BK vs IFK Göteborg",
  "start_time": "2026-04-18T13:00:00+02:00",
  "end_time": "2026-04-18T15:00:00+02:00",
  "status": "upcoming",
  "venue": "örjans vall",
  "round_label": "Round 3",
  "participants": {
    "home": "Halmstads BK",
    "away": "IFK Göteborg"
  },
  "source": "allsvenskan-fixture",
  "source_url": "https://allsvenskan.se/matcher/2026/6529852/halmstads-bk-mot-ifk-goteborg",
  "watch": {
    "recommended_market": "se",
    "recommended_provider": "TV4 Play",
    "availabilities": [ ... ]
  },
  "search_metadata": {
    "queries": [ ... ],
    "keywords": [ ... ]
  }
}
```

## 6.2 Important fields for Euripus

### `id`
Stable event identifier inside this service.
Use it as the foreign key in Euripus if you cache sports events.

### `title`
Human-friendly event title.
Good for UI cards.

### `competition`
Slug for grouping and routing.

### `start_time` / `end_time`
RFC3339 timestamps.
Euripus should parse them as timezone-aware values.

### `watch.recommended_provider`
Best current provider recommendation for the event.
Useful for the first CTA or badge.

### `watch.availabilities`
Sorted list of all known watch options.
Use this in detail views and fallback logic.

### `watch.availabilities[].channel_name`
Often the best direct search string for Euripus.
Examples:
- exact event feed label
- `TV4 Fotboll`
- `V Sport Football`
- `Eurosport 2`

### `watch.availabilities[].watch_type`
Current values include:
- `ppv-event`
- `streaming`
- `streaming+linear`
- `linear-tv`
- `studio`

### `watch.availabilities[].search_hints`
Ordered search candidates Euripus can try when matching against its own catalog.

### `source` and `source_url`
Debugging and observability fields.
Useful in admin or internal QA screens.

## 6.3 Response wrappers

List endpoints return:

```json
{
  "count": 6,
  "events": [ ... ]
}
```

Competition endpoint returns:

```json
{
  "competition": "allsvenskan",
  "events": [ ... ]
}
```

---

## 7. How Euripus should integrate with this API

## 7.1 Recommended architecture

The recommended architecture is:

1. Run the Sports API as its own service
2. Refresh sports sources on a schedule
3. Let Euripus call the Sports API over HTTP
4. Let Euripus use the response to search or open matching content in its own catalog

In other words:

- **Sports API owns ingestion, normalization, ranking and scraping**
- **Euripus owns UI, user context and final playback resolution**

Euripus should not scrape sports pages directly.

## 7.2 Minimum useful Euripus integration

At minimum, Euripus should use these endpoints:

- `GET /v1/events/live`
  - render a "Live now" row
- `GET /v1/events/today`
  - render a "Today" row
- `GET /v1/events/upcoming?hours=72`
  - render a "Coming up" row
- `GET /v1/competitions/{slug}`
  - render competition pages
- `GET /v1/events/{id}`
  - render event detail modals/pages

## 7.3 Suggested Euripus UI mapping

### Event card

For each event card, show:
- `title`
- `competition`
- formatted `start_time`
- `watch.recommended_provider`
- optional top `channel_name`

Example subtitle:

```text
TV4 Play · Halmstads BK - IFK Göteborg
```

or

```text
Viaplay · Paris Saint-Germain - Bayern Munich
```

### Event detail page

Show:
- title
- competition
- start time
- venue
- all `watch.availabilities`
- source/debug metadata in an internal or admin section

Each availability row can show:
- provider label
- channel name
- watch type
- market
- confidence/source if desired

## 7.4 Search strategy inside Euripus

The recommended matching order is:

1. Try the top availability `channel_name`
2. Try each `search_hints` entry from the same availability
3. If nothing matches, move to the next availability
4. If no availability matches, fall back to `search_metadata.queries`

Pseudo-flow:

```text
for availability in event.watch.availabilities:
  try search(availability.channel_name)
  for hint in availability.search_hints:
    try search(hint)
if no result:
  for query in event.search_metadata.queries:
    try search(query)
```

This works especially well for:
- event-specific feeds on TV4 Play
- event-specific Viaplay listings
- named golf windows and channels

## 7.5 Provider-aware search

If Euripus can filter by provider internally, use:

- `provider_family` as the stable machine key
- `provider_label` for display
- `channel_name` or `search_hints` for the actual search text

Example:
- provider family: `tv4`
- provider label: `TV4 Play`
- search text: `Hammarby IF - Örgryte IS`

## 7.6 Market handling

The API already ranks Sweden first, then US, then UK.

Recommended Euripus behavior:
- if the user market is Sweden, show the list as-is
- if the user market is known and not Sweden, optionally filter or boost availabilities for that market
- if the user market is unknown, keep the API ordering and label each row with its market

## 7.7 Caching in Euripus

Suggested cache policy:
- `/health`: 30s to 60s
- `/v1/events/live`: 1 to 5 minutes
- `/v1/events/today`: 5 minutes
- `/v1/events/upcoming`: 15 to 30 minutes
- `/v1/competitions/{slug}`: 15 to 30 minutes
- `/v1/providers`: cache for hours or until deployment

Because the Sports API itself is DB-backed, Euripus does not need aggressive polling.

## 7.8 Failure handling

If the Sports API is temporarily unavailable:
- keep the last successful Euripus cache if possible
- mark sports rows as stale rather than empty if UX allows
- do not treat a source outage as a playback failure in Euripus

---

## 8. Example Euripus implementation plan

## 8.1 Backend adapter in Euripus

Create a small adapter module, for example:

```text
sportsApi.getLiveEvents()
sportsApi.getTodayEvents()
sportsApi.getUpcomingEvents(hours)
sportsApi.getCompetition(slug)
sportsApi.getEvent(id)
```

This adapter should:
- call the Sports API
- parse RFC3339 timestamps
- return typed event objects to the rest of Euripus

## 8.2 Resolver from sports event to Euripus content

Create a resolver such as:

```text
resolveSportsEventToPlayable(event)
```

Suggested logic:
- take the sorted `watch.availabilities`
- try provider-aware search if available
- otherwise use text search with `channel_name` and `search_hints`
- return the first confident playable match
- if no match is found, return a structured "search manually" result using the best query text

## 8.3 Suggested output states in Euripus

Each event should end up in one of these states:

- `playable_exact`
  - Euripus found a confident item to open
- `playable_search`
  - Euripus can open a provider or prefilled search query
- `info_only`
  - show the event and provider guidance, but no resolvable item exists yet

This keeps the integration useful before exact mapping is complete.

## 8.4 Competition pages

Competition pages in Euripus can be built directly from competition slugs:

- `/sports/allsvenskan`
- `/sports/premier-league`
- `/sports/champions-league`
- `/sports/pga-tour`

Back them with:

```text
GET /v1/competitions/{slug}
```

---

## 9. Operational setup for production

## 9.1 Recommended deployment pattern

Run the Sports API as a small internal service with:
- one SQLite database file on persistent storage
- built-in periodic refreshes via `--refresh-interval`
- normal HTTP access from Euripus

## 9.2 Refresh schedule

A reasonable starting point:
- refresh every 5-10 minutes during active sports windows
- refresh every 15-30 minutes otherwise

The current Docker compose setup uses:
- `--source-fetch-mode auto`
- `--refresh-interval 10m`

Because sources are scraped during refresh rather than request time, this keeps requests fast and predictable.

## 9.3 Source run tracking

Refresh executions are recorded in the `source_runs` table.
Each run stores:
- source name
- started time
- finished time
- status
- item count
- error text

This should eventually back an admin endpoint, but today it already exists in SQLite for operational inspection.

---

## 10. Known limitations Euripus should account for

- Exact playback mapping is not implemented yet
- Some competitions are still only partially implemented at the watch-source layer, especially Bandy Elitserien where canonical fixtures exist but event-specific Bonnier watch overlays are not emitted yet
- `/v1/events/today` is a rolling 24-hour view, not a strict local-date calendar view
- Some source pages may require browser fallback in production
- Superettan is currently reliable through a fallback source, not the ideal official matcher page
- OpenAPI documentation is not generated yet

---

## 11. Recommended next integration steps

For Euripus, the best next steps are:

1. Add a Sports API client module
2. Build Live / Today / Upcoming rows from the existing endpoints
3. Use `recommended_provider` and top `channel_name` on cards
4. Use `search_hints` to resolve playable items inside Euripus
5. Add event detail views showing all availabilities
6. Add background refresh scheduling for the Sports API service
7. Later, add exact item matching and persistent overrides

---

## 12. Summary

This API should be integrated into Euripus as a **sports intelligence service**, not as a direct playback service.

The contract is:
- the Sports API tells Euripus **what is on**, **where it is likely available**, and **what to search for**
- Euripus uses that information to present sports rows, event pages, and search/play actions in its own UI

That split keeps the system simple:
- source scraping and ranking stay in one place
- Euripus gets a stable HTTP API
- exact playback mapping can be added later without redesigning the entire integration
