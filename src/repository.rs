use sqlx::SqlitePool;
use time::{Duration, OffsetDateTime};

use crate::{config::AppConfig, db, domain::Event, ingest::SourceFetchMode, jobs};

#[derive(Debug)]
pub struct AppState {
    pub pool: SqlitePool,
    pub providers: Vec<crate::domain::ProviderCatalogEntry>,
}

impl AppState {
    pub async fn new(
        config: AppConfig,
        database_url: &str,
        source_fetch_mode: SourceFetchMode,
        browser_command: &str,
    ) -> anyhow::Result<Self> {
        let pool = db::connect(database_url).await?;
        db::init(&pool).await?;
        jobs::refresh_sources(&pool, &config, source_fetch_mode, browser_command).await?;

        let providers = db::load_providers(&pool).await?;
        Ok(Self { pool, providers })
    }

    pub async fn live_events(&self) -> anyhow::Result<Vec<Event>> {
        let now = OffsetDateTime::now_utc();
        let events = db::load_events(&self.pool).await?;
        Ok(events
            .into_iter()
            .filter(|event| {
                event.status == crate::domain::EventStatus::Live
                    || in_window(event.start_time, event.end_time, now)
            })
            .collect())
    }

    pub async fn upcoming_events(&self, hours: i64) -> anyhow::Result<Vec<Event>> {
        let now = OffsetDateTime::now_utc();
        let end = now + Duration::hours(hours);
        let events = db::load_events(&self.pool).await?;
        Ok(events
            .into_iter()
            .filter(|event| event.start_time >= now && event.start_time <= end)
            .collect())
    }

    pub async fn today_events(&self) -> anyhow::Result<Vec<Event>> {
        self.upcoming_events(24).await
    }

    pub async fn event_by_id(&self, id: &str) -> anyhow::Result<Option<Event>> {
        let events = db::load_events(&self.pool).await?;
        Ok(events.into_iter().find(|event| event.id == id))
    }

    pub async fn events_for_competition(&self, competition: &str) -> anyhow::Result<Vec<Event>> {
        let events = db::load_events(&self.pool).await?;
        Ok(events
            .into_iter()
            .filter(|event| event.competition == competition)
            .collect())
    }
}

fn in_window(start: OffsetDateTime, end: Option<OffsetDateTime>, now: OffsetDateTime) -> bool {
    if start > now {
        return false;
    }
    end.map(|end| now <= end).unwrap_or(false)
}

pub type SharedState = std::sync::Arc<AppState>;
