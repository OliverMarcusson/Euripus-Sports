use std::time::Duration as StdDuration;

use anyhow::Context;
use serde::Serialize;
use sqlx::SqlitePool;
use time::{Duration, OffsetDateTime};

use crate::{
    config::AppConfig,
    db::{self, EventFilter},
    domain::{Event, EventStatus},
};

pub const UPCOMING_HOURS_MIN: i64 = 1;
pub const UPCOMING_HOURS_MAX: i64 = 8760;
pub const UPCOMING_HOURS_DEFAULT: i64 = 72;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpcomingHours(i64);

impl UpcomingHours {
    pub const fn get(self) -> i64 {
        self.0
    }
}

impl TryFrom<i64> for UpcomingHours {
    type Error = ();

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        if (UPCOMING_HOURS_MIN..=UPCOMING_HOURS_MAX).contains(&value) {
            Ok(Self(value))
        } else {
            Err(())
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ReadinessStatus {
    pub status: &'static str,
    pub latest_refresh_status: Option<String>,
    pub last_success_at: Option<String>,
    pub age_seconds: Option<i64>,
}

#[derive(Debug)]
pub struct AppState {
    pub pool: SqlitePool,
    pub providers: Vec<crate::domain::ProviderCatalogEntry>,
    readiness_max_refresh_age: StdDuration,
}

impl AppState {
    pub async fn initialize(
        config: &AppConfig,
        database_url: &str,
        readiness_max_refresh_age: StdDuration,
    ) -> anyhow::Result<Self> {
        let pool = db::connect(database_url).await?;
        db::init(&pool).await?;
        Ok(Self {
            pool,
            providers: config.providers.clone(),
            readiness_max_refresh_age,
        })
    }

    pub async fn live_events(&self) -> anyhow::Result<Vec<Event>> {
        self.live_events_at(OffsetDateTime::now_utc()).await
    }

    pub(crate) async fn live_events_at(&self, now: OffsetDateTime) -> anyhow::Result<Vec<Event>> {
        let events = db::load_events(&self.pool, EventFilter::ActiveAt(now)).await?;
        Ok(effective_events(events, now)
            .into_iter()
            .filter(|event| event.status == EventStatus::Live)
            .collect())
    }

    pub async fn upcoming_events(&self, hours: UpcomingHours) -> anyhow::Result<Vec<Event>> {
        self.upcoming_events_at(hours, OffsetDateTime::now_utc())
            .await
    }

    pub(crate) async fn upcoming_events_at(
        &self,
        hours: UpcomingHours,
        now: OffsetDateTime,
    ) -> anyhow::Result<Vec<Event>> {
        let seconds = hours
            .get()
            .checked_mul(60 * 60)
            .context("invalid upcoming duration")?;
        let duration = Duration::seconds(seconds);
        let end = now
            .checked_add(duration)
            .context("upcoming range overflow")?;
        let events = db::load_events(&self.pool, EventFilter::StartsBetween(now, end)).await?;
        Ok(effective_events(events, now))
    }

    pub async fn today_events(&self) -> anyhow::Result<Vec<Event>> {
        self.today_events_at(OffsetDateTime::now_utc()).await
    }

    pub(crate) async fn today_events_at(&self, now: OffsetDateTime) -> anyhow::Result<Vec<Event>> {
        let (day_start, day_end) = crate::time_utils::stockholm_day_bounds(now)
            .context("could not resolve Stockholm calendar day")?;
        let events = db::load_events(&self.pool, EventFilter::Overlaps(day_start, day_end)).await?;
        Ok(effective_events(events, now))
    }

    pub async fn event_by_id(&self, id: &str) -> anyhow::Result<Option<Event>> {
        let now = OffsetDateTime::now_utc();
        let events = db::load_events(&self.pool, EventFilter::Id(id)).await?;
        Ok(effective_events(events, now).into_iter().next())
    }

    pub async fn events_for_competition(&self, competition: &str) -> anyhow::Result<Vec<Event>> {
        let now = OffsetDateTime::now_utc();
        let events = db::load_events(&self.pool, EventFilter::Competition(competition)).await?;
        Ok(effective_events(events, now))
    }

    pub async fn readiness(&self) -> ReadinessStatus {
        self.readiness_at(OffsetDateTime::now_utc()).await
    }

    pub(crate) async fn readiness_at(&self, now: OffsetDateTime) -> ReadinessStatus {
        let Ok(refresh) = db::load_refresh_health(&self.pool).await else {
            return ReadinessStatus {
                status: "unready",
                latest_refresh_status: None,
                last_success_at: None,
                age_seconds: None,
            };
        };
        let Some(last_success) = refresh.last_success_at else {
            return ReadinessStatus {
                status: "unready",
                latest_refresh_status: refresh.latest_status,
                last_success_at: None,
                age_seconds: None,
            };
        };
        let age_seconds = (now - last_success).whole_seconds().max(0);
        let ready = u64::try_from(age_seconds)
            .ok()
            .is_some_and(|age| age <= self.readiness_max_refresh_age.as_secs());
        ReadinessStatus {
            status: if ready { "ready" } else { "unready" },
            latest_refresh_status: refresh.latest_status,
            last_success_at: last_success
                .format(&time::format_description::well_known::Rfc3339)
                .ok(),
            age_seconds: Some(age_seconds),
        }
    }
}

fn effective_events(mut events: Vec<Event>, now: OffsetDateTime) -> Vec<Event> {
    for event in &mut events {
        event.apply_effective_status(now);
    }
    events
}

pub type SharedState = std::sync::Arc<AppState>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{EventStatus, EventWatch, Participants, SearchMetadata};
    use std::collections::HashMap;
    use time::macros::datetime;

    fn event(id: &str, start: OffsetDateTime, end: OffsetDateTime, status: EventStatus) -> Event {
        Event {
            id: id.into(),
            sport: "hockey".into(),
            competition: "test".into(),
            title: id.into(),
            start_time: start,
            end_time: Some(end),
            status,
            venue: None,
            round_label: None,
            participants: Participants {
                home: "Home".into(),
                away: "Away".into(),
            },
            source: "test".into(),
            source_url: "https://example.test".into(),
            watch: EventWatch {
                recommended_market: None,
                recommended_provider: None,
                availabilities: Vec::new(),
            },
            search_metadata: SearchMetadata {
                queries: Vec::new(),
                keywords: Vec::new(),
            },
        }
    }

    async fn state_with(events: Vec<Event>) -> (AppState, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "euripus-repository-{}-{}.db",
            std::process::id(),
            OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        let pool = db::connect(&format!("sqlite://{}", path.display()))
            .await
            .unwrap();
        db::init(&pool).await.unwrap();
        let run = db::insert_source_run(&pool, "test", datetime!(2026-04-01 00:00 UTC))
            .await
            .unwrap();
        db::replace_snapshot(
            &pool,
            &[],
            &[],
            &HashMap::from([("test".into(), events)]),
            &[],
            run,
            "success",
            None,
            datetime!(2026-04-01 00:01 UTC),
        )
        .await
        .unwrap();
        (
            AppState {
                pool,
                providers: Vec::new(),
                readiness_max_refresh_age: StdDuration::from_secs(1800),
            },
            path,
        )
    }

    #[tokio::test]
    async fn live_uses_effective_status_instead_of_stored_status() {
        let now = datetime!(2026-04-01 11:00 UTC);
        let (state, path) = state_with(vec![
            event(
                "stale-live",
                datetime!(2026-04-01 08:00 UTC),
                datetime!(2026-04-01 09:00 UTC),
                EventStatus::Live,
            ),
            event(
                "stale-upcoming",
                datetime!(2026-04-01 10:00 UTC),
                datetime!(2026-04-01 12:00 UTC),
                EventStatus::Upcoming,
            ),
        ])
        .await;
        let events = state.live_events_at(now).await.unwrap();
        assert_eq!(
            events
                .iter()
                .map(|event| event.id.as_str())
                .collect::<Vec<_>>(),
            vec!["stale-upcoming"]
        );
        assert_eq!(events[0].status, EventStatus::Live);
        state.pool.close().await;
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn upcoming_window_uses_bounded_checked_hours() {
        assert!(UpcomingHours::try_from(UPCOMING_HOURS_MIN).is_ok());
        assert!(UpcomingHours::try_from(UPCOMING_HOURS_MAX).is_ok());
        for invalid in [0, -1, 8761, i64::MIN, i64::MAX] {
            assert!(UpcomingHours::try_from(invalid).is_err());
        }
        let (state, path) = state_with(Vec::new()).await;
        let events = state
            .upcoming_events_at(
                UpcomingHours::try_from(UPCOMING_HOURS_MAX).unwrap(),
                datetime!(2026-04-01 00:00 UTC),
            )
            .await
            .unwrap();
        assert!(events.is_empty());
        state.pool.close().await;
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn readiness_requires_nonzero_publication_and_preserves_last_success() {
        let (state, path) = state_with(Vec::new()).await;
        let before = state.readiness_at(datetime!(2026-04-01 00:00 UTC)).await;
        assert_eq!(before.status, "unready");

        let empty_first = db::insert_source_run(
            &state.pool,
            "configured_sources",
            datetime!(2026-04-01 00:00 UTC),
        )
        .await
        .unwrap();
        db::finish_source_run(
            &state.pool,
            empty_first,
            datetime!(2026-04-01 00:01 UTC),
            "degraded",
            0,
            Some("all sources failed"),
        )
        .await
        .unwrap();
        let still_unready = state.readiness_at(datetime!(2026-04-01 00:02 UTC)).await;
        assert_eq!(still_unready.status, "unready");
        assert_eq!(
            still_unready.latest_refresh_status.as_deref(),
            Some("degraded")
        );
        assert!(still_unready.last_success_at.is_none());

        let success = db::insert_source_run(
            &state.pool,
            "configured_sources",
            datetime!(2026-04-01 00:00 UTC),
        )
        .await
        .unwrap();
        db::finish_source_run(
            &state.pool,
            success,
            datetime!(2026-04-01 00:01 UTC),
            "success",
            1,
            None,
        )
        .await
        .unwrap();
        let ready = state.readiness_at(datetime!(2026-04-01 00:10 UTC)).await;
        assert_eq!(ready.status, "ready");
        let successful_timestamp = ready.last_success_at.clone();

        let degraded_zero = db::insert_source_run(
            &state.pool,
            "configured_sources",
            datetime!(2026-04-01 00:11 UTC),
        )
        .await
        .unwrap();
        db::finish_source_run(
            &state.pool,
            degraded_zero,
            datetime!(2026-04-01 00:12 UTC),
            "degraded",
            0,
            Some("no competitions published"),
        )
        .await
        .unwrap();
        let degraded = state.readiness_at(datetime!(2026-04-01 00:20 UTC)).await;
        assert_eq!(degraded.status, "ready");
        assert_eq!(degraded.latest_refresh_status.as_deref(), Some("degraded"));
        assert_eq!(degraded.last_success_at, successful_timestamp);
        let stale = state.readiness_at(datetime!(2026-04-01 01:00 UTC)).await;
        assert_eq!(stale.status, "unready");

        state.pool.close().await;
        let closed = state.readiness_at(datetime!(2026-04-01 00:20 UTC)).await;
        assert_eq!(closed.status, "unready");
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn today_is_stockholm_calendar_day_and_includes_overlaps() {
        let now = datetime!(2026-04-01 13:00 UTC);
        let (state, path) = state_with(vec![
            event(
                "morning",
                datetime!(2026-04-01 06:00 UTC),
                datetime!(2026-04-01 07:00 UTC),
                EventStatus::Upcoming,
            ),
            event(
                "spanning",
                datetime!(2026-03-31 21:30 UTC),
                datetime!(2026-04-01 01:00 UTC),
                EventStatus::Live,
            ),
            event(
                "tomorrow",
                datetime!(2026-04-02 12:00 UTC),
                datetime!(2026-04-02 14:00 UTC),
                EventStatus::Upcoming,
            ),
        ])
        .await;
        let events = state.today_events_at(now).await.unwrap();
        let ids = events
            .iter()
            .map(|event| event.id.as_str())
            .collect::<Vec<_>>();
        assert!(ids.contains(&"morning"));
        assert!(ids.contains(&"spanning"));
        assert!(!ids.contains(&"tomorrow"));
        state.pool.close().await;
        let _ = std::fs::remove_file(path);
    }
}
