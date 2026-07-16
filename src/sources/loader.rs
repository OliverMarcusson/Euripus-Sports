use std::time::Duration;

use anyhow::Context;
use time::OffsetDateTime;

const SOURCE_FETCH_TIMEOUT: Duration = Duration::from_secs(90);

use crate::{
    config::{AppConfig, ParserKind, SourceDefinition, SourceKind},
    domain::{EventSeed, WatchOverlay},
    ingest::{FetchRequest, SourceFetchMode, SourceFetcher},
    sources::{
        allsvenskan, champions_league, damallsvenskan, elitettan, elitserien, formula1,
        hockeyallsvenskan, listing_time, lpga_tour, ndhl, pga_tour, premier_league, sdhl, shl,
        superettan, tv4play, viaplay, world_cup,
    },
};

pub async fn load_configured_sources(
    config: &AppConfig,
    fetch_mode_override: SourceFetchMode,
    fetcher: &dyn SourceFetcher,
) -> Vec<SourceOutcome> {
    load_configured_sources_with_timeout(config, fetch_mode_override, fetcher, SOURCE_FETCH_TIMEOUT)
        .await
}

async fn load_configured_sources_with_timeout(
    config: &AppConfig,
    fetch_mode_override: SourceFetchMode,
    fetcher: &dyn SourceFetcher,
    source_timeout: Duration,
) -> Vec<SourceOutcome> {
    let observed_at = OffsetDateTime::now_utc();
    let mut outcomes = Vec::with_capacity(config.sources.len());

    for source in &config.sources {
        let started_at = OffsetDateTime::now_utc();
        if fetch_mode_override != SourceFetchMode::Fixture && !source.enabled_in_live {
            outcomes.push(SourceOutcome::new(
                source,
                started_at,
                SourceOutcomeStatus::Skipped,
                Some("source disabled in live mode".into()),
                ParsedSourceData::default(),
            ));
            continue;
        }

        let outcome =
            match load_source_body(source, fetch_mode_override, fetcher, source_timeout).await {
                Ok(body) => {
                    let parsed = parse_source_body(source, &body, config, observed_at);
                    let count = parsed.item_count(source.kind.clone());
                    let status = if count == 0 && !source.allow_empty {
                        SourceOutcomeStatus::Empty
                    } else {
                        SourceOutcomeStatus::Success
                    };
                    SourceOutcome::new(source, started_at, status, None, parsed)
                }
                Err(error) => SourceOutcome::new(
                    source,
                    started_at,
                    SourceOutcomeStatus::Failed,
                    Some(error.to_string()),
                    ParsedSourceData::default(),
                ),
            };
        outcomes.push(outcome);
    }

    outcomes
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceOutcomeStatus {
    Success,
    Empty,
    Failed,
    Skipped,
}

impl SourceOutcomeStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Empty => "empty",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(Debug)]
pub struct SourceOutcome {
    pub source_name: String,
    pub competition: String,
    pub kind: SourceKind,
    pub priority: i32,
    pub started_at: OffsetDateTime,
    pub finished_at: OffsetDateTime,
    pub status: SourceOutcomeStatus,
    pub error: Option<String>,
    pub events: Vec<EventSeed>,
    pub watch_overlays: Vec<WatchOverlay>,
}

impl SourceOutcome {
    fn new(
        source: &SourceDefinition,
        started_at: OffsetDateTime,
        status: SourceOutcomeStatus,
        error: Option<String>,
        parsed: ParsedSourceData,
    ) -> Self {
        Self {
            source_name: source.name.clone(),
            competition: source.competition.clone(),
            kind: source.kind.clone(),
            priority: source.priority,
            started_at,
            finished_at: OffsetDateTime::now_utc(),
            status,
            error,
            events: parsed.events,
            watch_overlays: parsed.watch_overlays,
        }
    }

    pub fn item_count(&self) -> usize {
        match self.kind {
            SourceKind::Event => self.events.len(),
            SourceKind::Watch => self.watch_overlays.len(),
        }
    }
}

#[derive(Debug, Default)]
struct ParsedSourceData {
    events: Vec<EventSeed>,
    watch_overlays: Vec<WatchOverlay>,
}

impl ParsedSourceData {
    fn item_count(&self, kind: SourceKind) -> usize {
        match kind {
            SourceKind::Event => self.events.len(),
            SourceKind::Watch => self.watch_overlays.len(),
        }
    }
}

fn parse_source_body(
    source: &SourceDefinition,
    body: &str,
    config: &AppConfig,
    observed_at: OffsetDateTime,
) -> ParsedSourceData {
    let mut parsed = ParsedSourceData::default();

    match (source.kind.clone(), source.parser.clone()) {
        (SourceKind::Event, ParserKind::Allsvenskan) => {
            parsed.events.extend(allsvenskan::parse_document_at(
                body,
                source.season.unwrap_or(observed_at.year()),
                config,
                observed_at,
            ));
        }
        (SourceKind::Event, ParserKind::Damallsvenskan) => {
            parsed.events.extend(damallsvenskan::parse_document_at(
                body,
                source.season.unwrap_or(observed_at.year()),
                config,
                observed_at,
            ));
        }
        (SourceKind::Event, ParserKind::Elitettan) => {
            parsed.events.extend(elitettan::parse_document_at(
                body,
                source.season.unwrap_or(observed_at.year()),
                config,
                observed_at,
            ));
        }
        (SourceKind::Watch, ParserKind::Tv4playAllsvenskan) => {
            parsed
                .watch_overlays
                .extend(tv4play::parse_document(body, config));
            listing_time::enrich_tv4(
                &mut parsed.watch_overlays,
                body,
                observed_at,
                source.season.unwrap_or(observed_at.year()),
            );
        }
        (SourceKind::Watch, ParserKind::Tv4playShl) => {
            parsed
                .watch_overlays
                .extend(tv4play::parse_shl_document(body, config));
            listing_time::enrich_tv4(
                &mut parsed.watch_overlays,
                body,
                observed_at,
                source.season.unwrap_or(observed_at.year()),
            );
        }
        (SourceKind::Watch, ParserKind::Tv4playHockeyallsvenskan) => {
            parsed
                .watch_overlays
                .extend(tv4play::parse_hockeyallsvenskan_document(body, config));
            listing_time::enrich_tv4(
                &mut parsed.watch_overlays,
                body,
                observed_at,
                source.season.unwrap_or(observed_at.year()),
            );
        }
        (SourceKind::Event, ParserKind::PgaTourBroadcastEvents) => {
            parsed
                .events
                .extend(pga_tour::parse_broadcast_events_document_at(
                    body,
                    source.season.unwrap_or(observed_at.year()),
                    observed_at,
                ));
        }
        (SourceKind::Watch, ParserKind::PgaTourBroadcastWatch) => {
            parsed
                .watch_overlays
                .extend(pga_tour::parse_broadcast_watch_document(
                    body,
                    source.season.unwrap_or(observed_at.year()),
                ));
        }
        (SourceKind::Watch, ParserKind::PgaTourSvenskGolfWatch) => {
            parsed
                .watch_overlays
                .extend(pga_tour::parse_svensk_golf_watch_document_at(
                    body,
                    source.season.unwrap_or(observed_at.year()),
                    observed_at,
                ));
        }
        (SourceKind::Event, ParserKind::LpgaTourSchedule) => {
            parsed.events.extend(lpga_tour::parse_schedule_document_at(
                body,
                source.season.unwrap_or(observed_at.year()),
                observed_at,
            ));
        }
        (SourceKind::Watch, ParserKind::LpgaTourSvenskGolfWatch) => {
            parsed
                .watch_overlays
                .extend(lpga_tour::parse_svensk_golf_watch_document(
                    body,
                    source.season.unwrap_or(observed_at.year()),
                ));
        }
        (SourceKind::Event, ParserKind::Formula1RaceTimes) => {
            parsed.events.extend(formula1::parse_race_times_document_at(
                body,
                source.season.unwrap_or(observed_at.year()),
                observed_at,
            ));
        }
        (SourceKind::Event, ParserKind::PremierLeagueBbc) => {
            parsed.events.extend(premier_league::parse_bbc_fixtures_at(
                body,
                source.season.unwrap_or(observed_at.year()),
                observed_at,
            ));
        }
        (SourceKind::Watch, ParserKind::ViaplayPremierLeague) => {
            parsed
                .watch_overlays
                .extend(viaplay::parse_premier_league_document(body, config));
            listing_time::enrich_viaplay(
                &mut parsed.watch_overlays,
                body,
                observed_at,
                source.season.unwrap_or(observed_at.year()),
                &source.competition,
                config,
            );
        }
        (SourceKind::Watch, ParserKind::ViaplayChampionsLeague) => {
            parsed
                .watch_overlays
                .extend(viaplay::parse_champions_league_document(body, config));
            listing_time::enrich_viaplay(
                &mut parsed.watch_overlays,
                body,
                observed_at,
                source.season.unwrap_or(observed_at.year()),
                &source.competition,
                config,
            );
        }
        (SourceKind::Event, ParserKind::ChampionsLeagueBbc) => {
            parsed
                .events
                .extend(champions_league::parse_bbc_fixtures_at(
                    body,
                    source.season.unwrap_or(observed_at.year()),
                    observed_at,
                ));
        }
        (SourceKind::Event, ParserKind::FifaWorldCupFifa) => {
            parsed
                .events
                .extend(world_cup::parse_fifa_fixtures_at(body, observed_at));
        }
        (SourceKind::Event, ParserKind::Shl) => {
            parsed.events.extend(shl::parse_schedule_document_at(
                body,
                source.season.unwrap_or(observed_at.year()),
                observed_at,
            ));
        }
        (SourceKind::Event, ParserKind::Sdhl) => {
            parsed.events.extend(sdhl::parse_schedule_document_at(
                body,
                source.season.unwrap_or(observed_at.year()),
                config,
                observed_at,
            ));
        }
        (SourceKind::Event, ParserKind::Ndhl) => {
            parsed.events.extend(ndhl::parse_schedule_document_at(
                body,
                source.season.unwrap_or(observed_at.year()),
                config,
                observed_at,
            ));
        }
        (SourceKind::Event, ParserKind::Hockeyallsvenskan) => {
            parsed
                .events
                .extend(hockeyallsvenskan::parse_schedule_document_at(
                    body,
                    source.season.unwrap_or(observed_at.year()),
                    config,
                    observed_at,
                ));
        }
        (SourceKind::Event, ParserKind::Elitserien) => {
            parsed.events.extend(elitserien::parse_schedule_document_at(
                body,
                source.season.unwrap_or(observed_at.year()),
                observed_at,
            ));
        }
        (SourceKind::Event, ParserKind::ElitserienDam) => {
            parsed
                .events
                .extend(elitserien::parse_schedule_document_for_competition_at(
                    body,
                    source.season.unwrap_or(observed_at.year()),
                    "bandy_elitserien_dam",
                    "tr.women-team",
                    "elitserien-dam-schedule",
                    observed_at,
                ));
        }
        (SourceKind::Event, ParserKind::Superettan) => {
            parsed.events.extend(superettan::parse_document(
                body,
                source.season.unwrap_or(observed_at.year()),
                config,
            ));
        }
        (SourceKind::Event, ParserKind::SuperettanSvenskfotboll) => {
            parsed
                .events
                .extend(superettan::parse_svenskfotboll_article(
                    body,
                    source.season.unwrap_or(observed_at.year()),
                    config,
                ));
        }
        _ => {}
    }

    parsed
}

async fn load_source_body(
    source: &SourceDefinition,
    fetch_mode_override: SourceFetchMode,
    fetcher: &dyn SourceFetcher,
    source_timeout: Duration,
) -> anyhow::Result<String> {
    let mode = if fetch_mode_override == SourceFetchMode::Fixture {
        SourceFetchMode::Fixture
    } else {
        fetch_mode_override
    };

    if mode == SourceFetchMode::Fixture {
        let path = source
            .fixture_path
            .as_ref()
            .context("fixture mode requires fixture_path")?;
        return std::fs::read_to_string(path).with_context(|| format!("reading fixture {path}"));
    }

    let request_mode = if fetch_mode_override == SourceFetchMode::Auto {
        source.fetch_mode
    } else {
        fetch_mode_override
    };
    let request = FetchRequest {
        source_name: source.name.clone(),
        url: source.url.clone(),
        method: source.request_method,
        body: source.request_body.clone(),
        mode: request_mode,
    };
    let page = tokio::time::timeout(source_timeout, fetcher.fetch(&request))
        .await
        .with_context(|| {
            format!(
                "loading source {} timed out after {} seconds",
                source.name,
                source_timeout.as_secs_f64()
            )
        })?
        .with_context(|| format!("loading source {}", source.name))?;

    tracing::info!(source = page.source_name, competition = source.competition, url = %page.url, method = ?page.method, "loaded source");
    Ok(page.body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    struct FirstFetchHangs {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl SourceFetcher for FirstFetchHangs {
        async fn fetch(
            &self,
            request: &FetchRequest,
        ) -> anyhow::Result<crate::ingest::FetchedPage> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if call == 0 {
                std::future::pending::<()>().await;
            }
            Ok(crate::ingest::FetchedPage {
                source_name: request.source_name.clone(),
                url: request.url.clone(),
                body: String::new(),
                method: crate::ingest::FetchMethod::Http,
            })
        }
    }

    #[tokio::test]
    async fn source_timeout_does_not_prevent_later_sources() {
        let mut config = AppConfig::load(
            "config/providers.yaml",
            "config/competition_rules.yaml",
            "config/sample_events.yaml",
            "config/sources.yaml",
            "config/team_aliases.yaml",
        )
        .unwrap();
        config.sources.truncate(2);
        for source in &mut config.sources {
            source.enabled_in_live = true;
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let outcomes = load_configured_sources_with_timeout(
            &config,
            SourceFetchMode::Http,
            &FirstFetchHangs {
                calls: calls.clone(),
            },
            Duration::from_millis(20),
        )
        .await;

        assert_eq!(outcomes.len(), 2);
        assert_eq!(outcomes[0].status, SourceOutcomeStatus::Failed);
        assert!(outcomes[0]
            .error
            .as_deref()
            .is_some_and(|error| error.contains("timed out")));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
