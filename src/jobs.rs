use anyhow::Context;
use sqlx::SqlitePool;
use time::OffsetDateTime;

use crate::{
    config::AppConfig,
    db,
    inference::hydrate_event,
    ingest::{BrowserFallbackFetcher, SourceFetchMode},
    sources,
};

pub async fn refresh_sources(
    pool: &SqlitePool,
    config: &AppConfig,
    source_fetch_mode: SourceFetchMode,
    browser_command: &str,
) -> anyhow::Result<RefreshSummary> {
    let started_at = OffsetDateTime::now_utc();
    let run_id = db::insert_source_run(pool, "configured_sources", started_at).await?;

    let result = async {
        let fetcher = BrowserFallbackFetcher::new(browser_command)?;
        let (source_events, source_overlays) =
            sources::loader::load_configured_sources(config, source_fetch_mode, &fetcher).await?;
        let effective_config = config
            .clone()
            .with_source_data(source_events, source_overlays);
        let hydrated_events = effective_config
            .events
            .iter()
            .map(|seed| hydrate_event(seed, &effective_config))
            .collect::<Vec<_>>();
        db::seed_reference_data(pool, &config.providers, &config.rules).await?;
        db::seed_events(pool, &hydrated_events).await?;
        Ok::<_, anyhow::Error>(RefreshSummary {
            started_at,
            finished_at: OffsetDateTime::now_utc(),
            event_count: hydrated_events.len(),
        })
    }
    .await;

    match result {
        Ok(summary) => {
            db::finish_source_run(
                pool,
                run_id,
                summary.finished_at,
                "success",
                summary.event_count as i64,
                None,
            )
            .await?;
            Ok(summary)
        }
        Err(error) => {
            let finished_at = OffsetDateTime::now_utc();
            db::finish_source_run(
                pool,
                run_id,
                finished_at,
                "failed",
                0,
                Some(&error.to_string()),
            )
            .await?;
            Err(error).context("refresh failed")
        }
    }
}

#[derive(Debug, Clone)]
pub struct RefreshSummary {
    pub started_at: OffsetDateTime,
    pub finished_at: OffsetDateTime,
    pub event_count: usize,
}
