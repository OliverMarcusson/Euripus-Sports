use anyhow::Context;

use crate::{
    config::{AppConfig, ParserKind, SourceDefinition, SourceKind},
    domain::{EventSeed, WatchOverlay},
    ingest::{FetchRequest, SourceFetchMode, SourceFetcher},
    sources::{
        allsvenskan, champions_league, damallsvenskan, elitettan, elitserien, formula1,
        hockeyallsvenskan, lpga_tour, ndhl, pga_tour, premier_league, sdhl, shl, superettan,
        tv4play, viaplay, world_cup,
    },
};

pub async fn load_configured_sources(
    config: &AppConfig,
    fetch_mode_override: SourceFetchMode,
    fetcher: &dyn SourceFetcher,
) -> anyhow::Result<(Vec<EventSeed>, Vec<WatchOverlay>)> {
    let mut events = Vec::new();
    let mut watch_overlays = Vec::new();

    for source in &config.sources {
        let body = load_source_body(source, fetch_mode_override, fetcher).await?;
        let parsed = parse_source_body(source, &body, config);
        events.extend(parsed.events);
        watch_overlays.extend(parsed.watch_overlays);
    }

    Ok((events, watch_overlays))
}

#[derive(Debug, Default)]
struct ParsedSourceData {
    events: Vec<EventSeed>,
    watch_overlays: Vec<WatchOverlay>,
}

fn parse_source_body(
    source: &SourceDefinition,
    body: &str,
    config: &AppConfig,
) -> ParsedSourceData {
    let mut parsed = ParsedSourceData::default();

    match (source.kind.clone(), source.parser.clone()) {
        (SourceKind::Event, ParserKind::Allsvenskan) => {
            parsed.events.extend(allsvenskan::parse_document(
                body,
                source.season.unwrap_or(current_season()),
                config,
            ));
        }
        (SourceKind::Event, ParserKind::Damallsvenskan) => {
            parsed.events.extend(damallsvenskan::parse_document(
                body,
                source.season.unwrap_or(current_season()),
                config,
            ));
        }
        (SourceKind::Event, ParserKind::Elitettan) => {
            parsed.events.extend(elitettan::parse_document(
                body,
                source.season.unwrap_or(current_season()),
                config,
            ));
        }
        (SourceKind::Watch, ParserKind::Tv4playAllsvenskan) => {
            parsed
                .watch_overlays
                .extend(tv4play::parse_document(body, config));
        }
        (SourceKind::Watch, ParserKind::Tv4playShl) => {
            parsed
                .watch_overlays
                .extend(tv4play::parse_shl_document(body, config));
        }
        (SourceKind::Watch, ParserKind::Tv4playHockeyallsvenskan) => {
            parsed
                .watch_overlays
                .extend(tv4play::parse_hockeyallsvenskan_document(body, config));
        }
        (SourceKind::Event, ParserKind::PgaTourSchedule) => {
            parsed.events.extend(pga_tour::parse_schedule_document(
                body,
                source.season.unwrap_or(current_season()),
            ));
        }
        (SourceKind::Event, ParserKind::PgaTourBroadcastEvents) => {
            parsed
                .events
                .extend(pga_tour::parse_broadcast_events_document(
                    body,
                    source.season.unwrap_or(current_season()),
                ));
        }
        (SourceKind::Watch, ParserKind::PgaTourBroadcastWatch) => {
            parsed
                .watch_overlays
                .extend(pga_tour::parse_broadcast_watch_document(
                    body,
                    source.season.unwrap_or(current_season()),
                ));
        }
        (SourceKind::Watch, ParserKind::PgaTourSvenskGolfWatch) => {
            parsed
                .watch_overlays
                .extend(pga_tour::parse_svensk_golf_watch_document(
                    body,
                    source.season.unwrap_or(current_season()),
                ));
        }
        (SourceKind::Event, ParserKind::LpgaTourSchedule) => {
            parsed.events.extend(lpga_tour::parse_schedule_document(
                body,
                source.season.unwrap_or(current_season()),
            ));
        }
        (SourceKind::Watch, ParserKind::LpgaTourSvenskGolfWatch) => {
            parsed
                .watch_overlays
                .extend(lpga_tour::parse_svensk_golf_watch_document(
                    body,
                    source.season.unwrap_or(current_season()),
                ));
        }
        (SourceKind::Event, ParserKind::Formula1RaceTimes) => {
            parsed.events.extend(formula1::parse_race_times_document(
                body,
                source.season.unwrap_or(current_season()),
            ));
        }
        (SourceKind::Event, ParserKind::PremierLeagueBbc) => {
            parsed.events.extend(premier_league::parse_bbc_fixtures(
                body,
                source.season.unwrap_or(current_season()),
            ));
        }
        (SourceKind::Watch, ParserKind::ViaplayPremierLeague) => {
            parsed
                .watch_overlays
                .extend(viaplay::parse_premier_league_document(body, config));
        }
        (SourceKind::Watch, ParserKind::ViaplayChampionsLeague) => {
            parsed
                .watch_overlays
                .extend(viaplay::parse_champions_league_document(body, config));
        }
        (SourceKind::Event, ParserKind::ChampionsLeagueBbc) => {
            parsed.events.extend(champions_league::parse_bbc_fixtures(
                body,
                source.season.unwrap_or(current_season()),
            ));
        }
        (SourceKind::Event, ParserKind::FifaWorldCupFifa) => {
            parsed.events.extend(world_cup::parse_fifa_fixtures(body));
        }
        (SourceKind::Event, ParserKind::Shl) => {
            parsed.events.extend(shl::parse_schedule_document(
                body,
                source.season.unwrap_or(current_season()),
            ));
        }
        (SourceKind::Event, ParserKind::Sdhl) => {
            parsed.events.extend(sdhl::parse_schedule_document(
                body,
                source.season.unwrap_or(current_season()),
                config,
            ));
        }
        (SourceKind::Event, ParserKind::Ndhl) => {
            parsed.events.extend(ndhl::parse_schedule_document(
                body,
                source.season.unwrap_or(current_season()),
                config,
            ));
        }
        (SourceKind::Event, ParserKind::Hockeyallsvenskan) => {
            parsed
                .events
                .extend(hockeyallsvenskan::parse_schedule_document(
                    body,
                    source.season.unwrap_or(current_season()),
                    config,
                ));
        }
        (SourceKind::Event, ParserKind::Elitserien) => {
            parsed.events.extend(elitserien::parse_schedule_document(
                body,
                source.season.unwrap_or(current_season()),
            ));
        }
        (SourceKind::Event, ParserKind::ElitserienDam) => {
            parsed
                .events
                .extend(elitserien::parse_schedule_document_for_competition(
                    body,
                    source.season.unwrap_or(current_season()),
                    "bandy_elitserien_dam",
                    "tr.women-team",
                    "elitserien-dam-schedule",
                ));
        }
        (SourceKind::Event, ParserKind::Superettan) => {
            parsed.events.extend(superettan::parse_document(
                body,
                source.season.unwrap_or(current_season()),
                config,
            ));
        }
        (SourceKind::Event, ParserKind::SuperettanSvenskfotboll) => {
            parsed
                .events
                .extend(superettan::parse_svenskfotboll_article(
                    body,
                    source.season.unwrap_or(current_season()),
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
    let page = fetcher
        .fetch(&FetchRequest {
            source_name: source.name.clone(),
            url: source.url.clone(),
            method: source.request_method,
            body: source.request_body.clone(),
            mode: request_mode,
        })
        .await
        .with_context(|| format!("loading source {}", source.name))?;

    tracing::info!(source = page.source_name, competition = source.competition, url = %page.url, method = ?page.method, "loaded source");
    Ok(page.body)
}

fn current_season() -> i32 {
    time::OffsetDateTime::now_utc().year()
}
