use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::OnceLock,
};

use anyhow::{bail, Context};
use sqlx::SqlitePool;
use time::OffsetDateTime;
use tokio::sync::Mutex;

use crate::{
    config::{AppConfig, SourceKind},
    db::{self, SourceRunWrite},
    domain::{Event, EventSeed, WatchOverlay},
    inference::hydrate_event,
    ingest::{BrowserFallbackFetcher, SourceFetchMode, SourceFetcher},
    sources::loader::{self, SourceOutcome, SourceOutcomeStatus},
};

static REFRESH_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub async fn refresh_sources(
    pool: &SqlitePool,
    config: &AppConfig,
    source_fetch_mode: SourceFetchMode,
    browser_command: &str,
) -> anyhow::Result<RefreshSummary> {
    let fetcher = BrowserFallbackFetcher::new(browser_command)?;
    refresh_sources_with_fetcher(pool, config, source_fetch_mode, &fetcher).await
}

pub async fn refresh_sources_with_fetcher(
    pool: &SqlitePool,
    config: &AppConfig,
    source_fetch_mode: SourceFetchMode,
    fetcher: &dyn SourceFetcher,
) -> anyhow::Result<RefreshSummary> {
    let _guard = REFRESH_LOCK.get_or_init(|| Mutex::new(())).lock().await;
    let started_at = OffsetDateTime::now_utc();
    let run_id = db::insert_source_run(pool, "configured_sources", started_at).await?;
    let outcomes = loader::load_configured_sources(config, source_fetch_mode, fetcher).await;
    let finished_at = OffsetDateTime::now_utc();

    let publication = build_publication(config, &outcomes);
    let (competition_events, composition_errors) = match publication {
        Ok(value) => value,
        Err(error) => {
            db::finish_source_run(
                pool,
                run_id,
                finished_at,
                "failed",
                0,
                Some(&error.to_string()),
            )
            .await?;
            return Err(error).context("refresh composition failed");
        }
    };
    let degraded = outcomes
        .iter()
        .any(|outcome| outcome.status != SourceOutcomeStatus::Success)
        || !composition_errors.is_empty();
    let mut errors = outcomes
        .iter()
        .filter(|outcome| outcome.status != SourceOutcomeStatus::Success)
        .map(|outcome| {
            format!(
                "{}: {}{}",
                outcome.source_name,
                outcome.status.as_str(),
                outcome
                    .error
                    .as_deref()
                    .map(|error| format!(" ({error})"))
                    .unwrap_or_default()
            )
        })
        .collect::<Vec<_>>();
    errors.extend(composition_errors);
    let error_text = (!errors.is_empty()).then(|| errors.join("; "));
    let source_runs = outcomes
        .iter()
        .map(|outcome| SourceRunWrite {
            source_name: &outcome.source_name,
            started_at: outcome.started_at,
            finished_at: outcome.finished_at,
            status: outcome.status.as_str(),
            item_count: outcome.item_count(),
            error_text: outcome.error.as_deref(),
        })
        .collect::<Vec<_>>();
    let status = if degraded { "degraded" } else { "success" };

    match db::replace_snapshot(
        pool,
        &config.providers,
        &config.rules,
        &competition_events,
        &source_runs,
        run_id,
        status,
        error_text.as_deref(),
        finished_at,
    )
    .await
    {
        Ok(event_count) => Ok(RefreshSummary {
            started_at,
            finished_at,
            event_count,
            status: if degraded {
                RefreshStatus::Degraded
            } else {
                RefreshStatus::Success
            },
        }),
        Err(error) => {
            let publication_error = error.context("atomic snapshot publication failed");
            let _ = db::finish_source_run(
                pool,
                run_id,
                OffsetDateTime::now_utc(),
                "failed",
                0,
                Some(&publication_error.to_string()),
            )
            .await;
            Err(publication_error)
        }
    }
}

type Publication = (HashMap<String, Vec<Event>>, Vec<String>);

fn build_publication(
    config: &AppConfig,
    outcomes: &[SourceOutcome],
) -> anyhow::Result<Publication> {
    let mut grouped: BTreeMap<&str, Vec<&SourceOutcome>> = BTreeMap::new();
    for outcome in outcomes {
        grouped
            .entry(&outcome.competition)
            .or_default()
            .push(outcome);
    }

    let mut publication: HashMap<String, Vec<Event>> = HashMap::new();
    let mut errors = Vec::new();
    for (competition, group) in grouped {
        if group
            .iter()
            .any(|outcome| outcome.status != SourceOutcomeStatus::Success)
        {
            continue;
        }
        let candidates = group
            .iter()
            .flat_map(|outcome| {
                outcome
                    .events
                    .iter()
                    .cloned()
                    .map(|event| (event, outcome.priority, outcome.source_name.as_str()))
            })
            .collect::<Vec<_>>();
        let events = match merge_event_candidates(candidates) {
            Ok(events) => events,
            Err(error) => {
                errors.push(format!("{competition}: {error}"));
                continue;
            }
        };
        let overlays = group
            .iter()
            .flat_map(|outcome| outcome.watch_overlays.iter().cloned())
            .collect::<Vec<WatchOverlay>>();
        let effective = config.for_publication(events, overlays);
        publication.insert(
            competition.to_string(),
            effective
                .events
                .iter()
                .map(|seed| hydrate_event(seed, &effective))
                .collect(),
        );
    }

    let configured_event_competitions = config
        .sources
        .iter()
        .filter(|source| source.kind == SourceKind::Event)
        .map(|source| source.competition.as_str())
        .collect::<HashSet<_>>();
    for seed in &config.events {
        if configured_event_competitions.contains(seed.competition.as_str()) {
            continue;
        }
        let effective = config.for_publication(vec![seed.clone()], Vec::new());
        publication
            .entry(seed.competition.clone())
            .or_default()
            .push(hydrate_event(seed, &effective));
    }
    Ok((publication, errors))
}

fn merge_event_candidates(
    candidates: Vec<(EventSeed, i32, &str)>,
) -> anyhow::Result<Vec<EventSeed>> {
    let mut grouped: BTreeMap<String, Vec<(EventSeed, i32, &str)>> = BTreeMap::new();
    for candidate in candidates {
        grouped
            .entry(candidate.0.id.clone())
            .or_default()
            .push(candidate);
    }
    let mut merged = Vec::with_capacity(grouped.len());
    for (id, mut candidates) in grouped {
        let first = &candidates[0].0;
        if candidates.iter().skip(1).any(|(candidate, _, _)| {
            candidate.sport != first.sport
                || candidate.competition != first.competition
                || normalize(&candidate.participants.home) != normalize(&first.participants.home)
                || normalize(&candidate.participants.away) != normalize(&first.participants.away)
        }) {
            bail!("duplicate event {id} has conflicting identity");
        }
        candidates.sort_by(|left, right| {
            right
                .1
                .cmp(&left.1)
                .then_with(|| right.0.end_time.is_some().cmp(&left.0.end_time.is_some()))
                .then_with(|| left.2.cmp(right.2))
        });
        let mut canonical = candidates.remove(0).0;
        for (candidate, _, _) in candidates {
            if canonical.venue.is_none() {
                canonical.venue = candidate.venue;
            }
            if canonical.round_label.is_none() {
                canonical.round_label = candidate.round_label;
            }
        }
        merged.push(canonical);
    }
    Ok(merged)
}

fn normalize(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .filter(|character| character.is_alphanumeric())
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshStatus {
    Success,
    Degraded,
}

#[derive(Debug, Clone)]
pub struct RefreshSummary {
    pub started_at: OffsetDateTime,
    pub finished_at: OffsetDateTime,
    pub event_count: usize,
    pub status: RefreshStatus,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{EventStatus, Participants};
    use time::macros::datetime;

    fn seed(source: &str, start_hour: u8, end: bool) -> EventSeed {
        let start = datetime!(2026-04-17 00:00 UTC) + time::Duration::hours(i64::from(start_hour));
        EventSeed {
            id: "pga_round_2".into(),
            sport: "golf".into(),
            competition: "pga_tour".into(),
            title: "Tournament Round 2".into(),
            start_time: start,
            end_time: end.then_some(start + time::Duration::hours(4)),
            status: EventStatus::Upcoming,
            venue: None,
            round_label: Some("Round 2".into()),
            participants: Participants {
                home: "Tournament".into(),
                away: "Field".into(),
            },
            source: source.into(),
            source_url: "https://example.test".into(),
        }
    }

    #[test]
    fn duplicate_merge_uses_priority_not_input_order() {
        let low = seed("low", 10, false);
        let high = seed("high", 11, true);
        let forward =
            merge_event_candidates(vec![(low.clone(), 0, "low"), (high.clone(), 100, "high")])
                .unwrap();
        let reverse = merge_event_candidates(vec![(high, 100, "high"), (low, 0, "low")]).unwrap();
        assert_eq!(forward[0].start_time, datetime!(2026-04-17 11:00 UTC));
        assert_eq!(forward[0].start_time, reverse[0].start_time);
        assert!(forward[0].end_time.is_some());
    }

    #[test]
    fn duplicate_merge_rejects_identity_conflicts() {
        let one = seed("one", 10, true);
        let mut conflicting = seed("two", 11, true);
        conflicting.participants.home = "Other Tournament".into();
        assert!(merge_event_candidates(vec![(one, 0, "one"), (conflicting, 0, "two")]).is_err());
    }

    struct PartialFetcher;

    #[async_trait::async_trait]
    impl SourceFetcher for PartialFetcher {
        async fn fetch(
            &self,
            request: &crate::ingest::FetchRequest,
        ) -> anyhow::Result<crate::ingest::FetchedPage> {
            if request.source_name == "ndhl_schedule" {
                anyhow::bail!("simulated outage");
            }
            Ok(crate::ingest::FetchedPage {
                source_name: request.source_name.clone(),
                url: request.url.clone(),
                body: std::fs::read_to_string("tests/fixtures/elitserien_spelprogram.html")?,
                method: crate::ingest::FetchMethod::Http,
            })
        }
    }

    struct FailedFetcher;

    #[async_trait::async_trait]
    impl SourceFetcher for FailedFetcher {
        async fn fetch(
            &self,
            _request: &crate::ingest::FetchRequest,
        ) -> anyhow::Result<crate::ingest::FetchedPage> {
            anyhow::bail!("simulated total outage")
        }
    }

    #[tokio::test]
    async fn fixture_refresh_persists_tv4_and_viaplay_listings() {
        let config = AppConfig::load(
            "config/providers.yaml",
            "config/competition_rules.yaml",
            "config/sample_events.yaml",
            "config/sources.yaml",
            "config/team_aliases.yaml",
        )
        .unwrap();
        let path = std::env::temp_dir().join(format!(
            "euripus-listing-refresh-{}-{}.db",
            std::process::id(),
            OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        let pool = db::connect(&format!("sqlite://{}", path.display()))
            .await
            .unwrap();
        db::init(&pool).await.unwrap();

        let summary =
            refresh_sources_with_fetcher(&pool, &config, SourceFetchMode::Fixture, &FailedFetcher)
                .await
                .unwrap();
        assert!(summary.event_count > 0);
        let shl = db::load_events(&pool, db::EventFilter::Competition("shl"))
            .await
            .unwrap();
        let champions_league =
            db::load_events(&pool, db::EventFilter::Competition("uefa_champions_league"))
                .await
                .unwrap();
        assert!(shl.iter().any(|event| event
            .watch
            .availabilities
            .iter()
            .any(|availability| availability.source == "tv4play-listing")));
        assert!(champions_league.iter().any(|event| event
            .watch
            .availabilities
            .iter()
            .any(|availability| availability.source == "viaplay-listing")));

        pool.close().await;
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn all_sources_failed_first_refresh_does_not_establish_success() {
        let mut config = AppConfig::load(
            "config/providers.yaml",
            "config/competition_rules.yaml",
            "config/sample_events.yaml",
            "config/sources.yaml",
            "config/team_aliases.yaml",
        )
        .unwrap();
        config.sources.retain(|source| {
            matches!(
                source.name.as_str(),
                "ndhl_schedule" | "elitserien_schedule"
            )
        });
        for source in &mut config.sources {
            source.enabled_in_live = true;
        }
        config.events.clear();
        let path = std::env::temp_dir().join(format!(
            "euripus-total-outage-{}-{}.db",
            std::process::id(),
            OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        let pool = db::connect(&format!("sqlite://{}", path.display()))
            .await
            .unwrap();
        db::init(&pool).await.unwrap();

        let summary =
            refresh_sources_with_fetcher(&pool, &config, SourceFetchMode::Http, &FailedFetcher)
                .await
                .unwrap();
        assert_eq!(summary.status, RefreshStatus::Degraded);
        assert_eq!(summary.event_count, 0);
        let health = db::load_refresh_health(&pool).await.unwrap();
        assert_eq!(health.latest_status.as_deref(), Some("degraded"));
        assert!(health.last_success_at.is_none());

        pool.close().await;
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn failed_competition_preserves_lkg_while_successful_competition_replaces() {
        let mut config = AppConfig::load(
            "config/providers.yaml",
            "config/competition_rules.yaml",
            "config/sample_events.yaml",
            "config/sources.yaml",
            "config/team_aliases.yaml",
        )
        .unwrap();
        config.sources.retain(|source| {
            matches!(
                source.name.as_str(),
                "ndhl_schedule" | "elitserien_schedule"
            )
        });
        for source in &mut config.sources {
            source.enabled_in_live = true;
        }
        config.events.clear();
        let path = std::env::temp_dir().join(format!(
            "euripus-lkg-{}-{}.db",
            std::process::id(),
            OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        let pool = db::connect(&format!("sqlite://{}", path.display()))
            .await
            .unwrap();
        db::init(&pool).await.unwrap();
        let old_seed = |id: &str, competition: &str| EventSeed {
            id: id.into(),
            sport: "hockey".into(),
            competition: competition.into(),
            title: id.into(),
            start_time: datetime!(2026-01-01 10:00 UTC),
            end_time: Some(datetime!(2026-01-01 12:00 UTC)),
            status: EventStatus::Finished,
            venue: None,
            round_label: None,
            participants: Participants {
                home: "Old Home".into(),
                away: "Old Away".into(),
            },
            source: "old".into(),
            source_url: "https://example.test".into(),
        };
        let old = HashMap::from([
            (
                "ndhl".into(),
                vec![hydrate_event(&old_seed("old_ndhl", "ndhl"), &config)],
            ),
            (
                "bandy_elitserien".into(),
                vec![hydrate_event(
                    &old_seed("old_elitserien", "bandy_elitserien"),
                    &config,
                )],
            ),
        ]);
        let run = db::insert_source_run(&pool, "seed", datetime!(2026-01-01 00:00 UTC))
            .await
            .unwrap();
        db::replace_snapshot(
            &pool,
            &config.providers,
            &config.rules,
            &old,
            &[],
            run,
            "success",
            None,
            datetime!(2026-01-01 00:01 UTC),
        )
        .await
        .unwrap();

        let summary =
            refresh_sources_with_fetcher(&pool, &config, SourceFetchMode::Http, &PartialFetcher)
                .await
                .unwrap();
        assert_eq!(summary.status, RefreshStatus::Degraded);
        let ndhl = db::load_events(&pool, db::EventFilter::Competition("ndhl"))
            .await
            .unwrap();
        let elitserien = db::load_events(&pool, db::EventFilter::Competition("bandy_elitserien"))
            .await
            .unwrap();
        assert_eq!(
            ndhl.iter()
                .map(|event| event.id.as_str())
                .collect::<Vec<_>>(),
            vec!["old_ndhl"]
        );
        assert!(!elitserien.is_empty());
        assert!(elitserien.iter().all(|event| event.id != "old_elitserien"));
        pool.close().await;
        let _ = std::fs::remove_file(path);
    }
}
