# Women's competitions implementation plan

## Scope

Add ingestion support for:

- `damallsvenskan`
- `elitettan`
- `sdhl`
- `ndhl`
- `bandy_elitserien_dam`
- `lpga_tour`

Treat this as **event ingestion first**, with **watch/provider inference added where the source is clear enough**.

---

## Recommended competition model

Use these slugs:

- `damallsvenskan`
- `elitettan`
- `sdhl`
- `ndhl`
- `bandy_elitserien_dam`
- `lpga_tour`

Notes:

- Keep `bandy_elitserien_dam` separate from existing `bandy_elitserien`.
- For `ndhl`, aggregate multiple regional/phase sources under one competition slug, and encode region/phase in `round_label`.

---

## Source strategy by competition

| Competition | Event source | Watch/source inference | Implementation approach |
|---|---|---|---|
| Damallsvenskan | `gql.sportomedia.se/graphql` if it supports `configLeagueName=damallsvenskan`; fallback to official EFD/league fixture page | Viaplay rule-only initially | Reuse/generalize Allsvenskan/Superettan parser |
| Elitettan | Same GraphQL approach if available; fallback to official EFD fixture page/article | Leave watch unset in v1 unless rights confirmed | Reuse/generalize Superettan parser |
| SDHL | Official `sdhl.se/game-schedule`, ideally direct schedule endpoint if available | Add `youtube`/`sdhl_play` provider family and rule | New parser, likely close to SHL/HockeyAllsvenskan shape |
| NDHL | Official Swehockey stats schedule pages, likely multiple regional URLs | No watch rule initially | New parser that aggregates regional series |
| Bandy Elitserien Dam | Official `elitserien.se/spelprogram/` | Reuse `bandy_bonnier` if confirmed; otherwise event-only first | Parameterize current Elitserien parser |
| LPGA Tour | Official `lpga.com/tournaments?year=2026` | Svensk Golf weekly guide + Viaplay rule | New parser, but extract reusable golf-guide helpers from PGA |

---

## What to build

## 1) Add config/parser scaffolding

### Files
- `src/config.rs`
- `src/sources/mod.rs`
- `src/sources/loader.rs`

### Changes
Add parser enum variants for the new sources.

Suggested additions in `ParserKind`:
- `Damallsvenskan`
- `Elitettan`
- `Sdhl`
- `Ndhl`
- `ElitserienDam`
- `LpgaTourSchedule`
- `LpgaTourSvenskGolfWatch`

Then wire them in `loader.rs`.

---

## 2) Swedish women’s football: Damallsvenskan + Elitettan

## Recommendation
Do a **small refactor** of the current men’s Swedish football ingestion instead of copying two more near-identical files.

Right now:
- `src/sources/allsvenskan.rs`
- `src/sources/superettan.rs`

are mostly the same parser with hardcoded:
- competition slug
- source URL domain
- team alias lookup key
- event ID prefix

## Plan
Extract a shared helper, e.g.:

- `src/sources/svenskfotboll_league.rs`

with a generic function like:
- parse graphql response
- parse fallback markdown/html
- accept:
  - `competition`
  - `sport`
  - `url_base`
  - `id_prefix`

Then make thin wrappers:
- `src/sources/damallsvenskan.rs`
- `src/sources/elitettan.rs`

or reuse one generic parser directly from `loader.rs`.

## Source assumptions
First spike to verify:
- `configLeagueName: "damallsvenskan"`
- `configLeagueName: "elitettan"`

on the same Sportomedia GraphQL endpoint already used for:
- `allsvenskan`
- `superettan`

If GraphQL works, these are low-effort adds.

If GraphQL does **not** work:
- use official EFD pages/articles as fixture-backed fallback
- but keep the parser abstraction, since the domain model is the same

## Watch/provider
- `damallsvenskan`: add SE rule for `viaplay`  
  Viaplay’s own press release confirms rights through 2026.
- `elitettan`: hold off on provider rule until verified

## Files to touch
- `config/sources.yaml`
- `config/competition_rules.yaml`
- `config/team_aliases.yaml`
- `tests/fixtures/*`
- `config/sample_events.yaml`

---

## 3) SDHL

## Source
Official schedule exists at:
- `https://www.sdhl.se/game-schedule`

This looks close to the SHL/HockeyAllsvenskan schedule model.

## Plan
Create:
- `src/sources/sdhl.rs`

Parser should:
- prefer direct structured schedule data if available
- otherwise parse the official schedule page HTML
- support full season, playoffs, and promotion/relegation if exposed cleanly
- use season semantics like SHL: autumn dates belong to `season - 1`, spring dates to `season`

## Refactor opportunity
If SDHL exposes the same sports-v2 shape as SHL/HockeyAllsvenskan, extract a shared parser:
- `src/sources/swedish_hockey_schedule.rs`

and move:
- SHL
- HockeyAllsvenskan
- SDHL

onto one helper.

If not, keep `sdhl.rs` standalone for now.

## Watch/provider
SDHL currently points to YouTube/SDHL Play.

So add a new provider family in:

- `config/providers.yaml`

Suggested:
- family: `youtube`
- market: `se`
- aliases: `[YouTube, SDHL Play, SDHL YouTube]`

Then add:
- `sdhl` + `se` + `youtube`

No overlay parser needed initially; a competition rule is enough for baseline inference.

## Also add
- SDHL team aliases in `config/team_aliases.yaml`

---

## 4) NDHL

## Important modeling decision
NDHL is the trickiest one.

It appears to be split across:
- regional series
- continuation phases
- qualification structure

So the cleanest API choice is:

### Keep one public competition slug
- `ndhl`

### Aggregate multiple source feeds into it
Examples:
- `ndhl_ostra`
- `ndhl_vastra`
- `ndhl_sodra`
- continuation/kval feeds if needed

Each source entry still maps to:
- `competition: ndhl`

## Parser
Create:
- `src/sources/ndhl.rs`

Responsibilities:
- parse Swehockey schedule tables/pages
- produce stable IDs
- include region/phase in `round_label`
  - e.g. `Östra`
  - `Södra Vår`
  - `Kval`

## Risk
These Swehockey URLs may have season-specific IDs.  
That’s acceptable because this repo already hardcodes season UUIDs in `config/sources.yaml` for other leagues.

So plan for:
- annual URL refresh in config
- fixture coverage to catch breakage

## Watch/provider
Skip in first pass.  
NDHL watch mapping is too unclear for a good default rule.

---

## 5) Bandy Elitserien Dam

## Source
Use the same official page already used for men:
- `https://elitserien.se/spelprogram/`

## Plan
Refactor current:
- `src/sources/elitserien.rs`

so it can parse either:
- men’s rows
- women’s rows

Likely approach:
- parameterize the row selector
- parameterize competition slug
- parameterize source name / ID prefix

Then add:
- `bandy_elitserien_dam`

without copying the entire parser.

## Watch/provider
If business is okay with a first-pass assumption, use the same provider family as men:
- `bandy_bonnier`

If not, land event ingestion first and defer watch rules one PR.

---

## 6) LPGA Tour

## Source strategy
Unlike PGA, LPGA likely needs a different combination:

### Event source
Official:
- `https://www.lpga.com/tournaments?year=2026`

### Swedish watch source
Use the same kind of weekly Swedish guide already used for PGA:
- Svensk Golf weekly live-broadcast page

## Recommendation
Do **not** try to force LPGA into the exact PGA source stack.

Current PGA implementation is unusually tailored:
- official schedule page
- PGA broadcast media page
- Svensk Golf weekly guide

For LPGA, do this instead:

### Phase 1
- Parse official LPGA tournament list
- Create tournament events or round events where day info is available
- Add Swedish watch overlays from Svensk Golf if LPGA entries are present

### Phase 2
- If official LPGA tournament pages expose enough daily/session detail, upgrade to true round-level parity

## Parser plan
Create:
- `src/sources/lpga_tour.rs`

Extract shared helper functions from `src/sources/pga_tour.rs` for:
- Swedish weekly guide parsing
- round detection from weekday labels
- tournament title normalization

Then parameterize:
- competition slug
- provider family
- source URLs

## Watch/provider
Add SE rule:
- `lpga_tour` -> `viaplay`

based on Svensk Golf / Viaplay Golf references.

---

## Config changes

## `config/sources.yaml`
Add new sources for:
- damallsvenskan event feed
- elitettan event feed
- sdhl schedule
- ndhl regional schedules
- bandy_elitserien_dam schedule
- lpga_tour official schedule
- optionally `lpga_tour` Svensk Golf watch source

## `config/competition_rules.yaml`
Add:
- `damallsvenskan` / `se` / `viaplay`
- `sdhl` / `se` / `youtube`
- `bandy_elitserien_dam` / `se` / `bandy_bonnier` if confirmed
- `lpga_tour` / `se` / `viaplay`

Hold off on:
- `elitettan`
- `ndhl`

until rights are verified

## `config/providers.yaml`
Add:
- `youtube` provider family for Sweden

## `config/team_aliases.yaml`
Add aliases for all six new competitions.

This is mandatory for:
- women’s football parsing
- SDHL/NDHL normalization
- bandy women matchup normalization

## `config/sample_events.yaml`
Add at least one representative sample event per new competition.

---

## Test plan

## Fixtures to add
Under `tests/fixtures/`:

- `damallsvenskan_*.json|md|html`
- `elitettan_*.json|md|html`
- `sdhl_game_schedule.*`
- `ndhl_*.html|md`
- `elitserien_dam_spelprogram.html`
- `lpga_schedule_2026.html`
- `lpga_svenskgolf_weekly.*`

## Unit tests
Add parser tests for:
- event count
- title normalization
- team alias resolution
- season rollover for winter leagues
- status inference
- region labeling for NDHL

## Integration checks
Verify in fixture mode:
- `/v1/competitions/damallsvenskan`
- `/v1/competitions/elitettan`
- `/v1/competitions/sdhl`
- `/v1/competitions/ndhl`
- `/v1/competitions/bandy_elitserien_dam`
- `/v1/competitions/lpga_tour`

all return non-empty data.

---

## PR breakdown

## PR 1 — low-risk additions
- Damallsvenskan
- Elitettan
- Bandy Elitserien Dam
- shared Swedish football/bandy refactors
- sample events + fixtures + tests

## PR 2 — women’s hockey
- SDHL
- NDHL
- `youtube` provider family
- hockey fixtures + tests

## PR 3 — golf
- LPGA Tour
- shared golf-guide helpers
- LPGA rules + watch overlays
- fixtures + tests

This keeps risk isolated:
- football/bandy first
- hockey second
- golf last

---

## Main risks

1. **Damallsvenskan/Elitettan GraphQL support may differ**
   - mitigation: validate source before coding parser

2. **NDHL source fragmentation**
   - mitigation: aggregate multiple sources under one slug

3. **Season-specific IDs/UUIDs**
   - mitigation: keep them config-driven and fixture-tested

4. **Watch rights differ from men’s equivalents**
   - mitigation: treat rules independently, not inherited from men’s comps

---

## Recommended implementation order

1. `damallsvenskan`
2. `elitettan`
3. `bandy_elitserien_dam`
4. `sdhl`
5. `ndhl`
6. `lpga_tour`

This gives the most coverage quickly with the least parser risk.
