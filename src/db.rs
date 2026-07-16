use anyhow::Context;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    QueryBuilder, Row, Sqlite, SqliteConnection, SqlitePool,
};
use std::{collections::HashMap, str::FromStr, time::Duration};
use time::OffsetDateTime;

use crate::domain::{CompetitionRule, Event, ProviderCatalogEntry, WatchAvailability};

pub async fn connect(database_url: &str) -> anyhow::Result<SqlitePool> {
    let options = SqliteConnectOptions::from_str(database_url)
        .with_context(|| format!("parsing database url {database_url}"))?
        .create_if_missing(true)
        .busy_timeout(Duration::from_secs(10));

    SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .with_context(|| format!("connecting to database {database_url}"))
}

pub async fn init(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS providers (
            family TEXT NOT NULL,
            market TEXT NOT NULL,
            aliases_json TEXT NOT NULL,
            PRIMARY KEY (family, market)
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS competition_rules (
            competition TEXT NOT NULL,
            market TEXT NOT NULL,
            provider_family TEXT NOT NULL,
            watch_type TEXT NOT NULL,
            confidence REAL NOT NULL,
            PRIMARY KEY (competition, market, provider_family)
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS events (
            id TEXT PRIMARY KEY,
            sport TEXT NOT NULL,
            competition TEXT NOT NULL,
            title TEXT NOT NULL,
            start_time TEXT NOT NULL,
            end_time TEXT,
            status TEXT NOT NULL,
            venue TEXT,
            round_label TEXT,
            participants_json TEXT NOT NULL,
            source TEXT NOT NULL,
            source_url TEXT NOT NULL,
            search_metadata_json TEXT NOT NULL,
            recommended_market TEXT,
            recommended_provider TEXT
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_events_competition_start_time ON events (competition, start_time)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_events_start_time_julianday ON events (julianday(start_time))",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_events_end_time_julianday ON events (julianday(end_time))",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS watch_availabilities (
            event_id TEXT NOT NULL,
            ordinal INTEGER NOT NULL,
            market TEXT NOT NULL,
            provider_family TEXT NOT NULL,
            provider_label TEXT NOT NULL,
            channel_name TEXT,
            watch_type TEXT NOT NULL,
            priority INTEGER NOT NULL,
            confidence REAL NOT NULL,
            source TEXT NOT NULL,
            search_hints_json TEXT NOT NULL,
            PRIMARY KEY (event_id, ordinal),
            FOREIGN KEY (event_id) REFERENCES events(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS source_runs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source_name TEXT NOT NULL,
            started_at TEXT NOT NULL,
            finished_at TEXT,
            status TEXT NOT NULL,
            item_count INTEGER NOT NULL DEFAULT 0,
            error_text TEXT
        )
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}

#[derive(Debug)]
pub struct SourceRunWrite<'a> {
    pub source_name: &'a str,
    pub started_at: OffsetDateTime,
    pub finished_at: OffsetDateTime,
    pub status: &'a str,
    pub item_count: usize,
    pub error_text: Option<&'a str>,
}

#[allow(clippy::too_many_arguments)]
pub async fn replace_snapshot(
    pool: &SqlitePool,
    providers: &[ProviderCatalogEntry],
    rules: &[CompetitionRule],
    competition_events: &HashMap<String, Vec<Event>>,
    source_runs: &[SourceRunWrite<'_>],
    aggregate_run_id: i64,
    aggregate_status: &str,
    aggregate_error: Option<&str>,
    finished_at: OffsetDateTime,
) -> anyhow::Result<usize> {
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    let connection = &mut *transaction;

    replace_reference_data(connection, providers, rules).await?;
    let mut event_count = 0;
    for (competition, events) in competition_events {
        sqlx::query(
            "DELETE FROM watch_availabilities WHERE event_id IN (SELECT id FROM events WHERE competition = ?)",
        )
        .bind(competition)
        .execute(&mut *connection)
        .await?;
        sqlx::query("DELETE FROM events WHERE competition = ?")
            .bind(competition)
            .execute(&mut *connection)
            .await?;
        for event in events {
            insert_event(connection, event).await?;
            event_count += 1;
        }
    }

    for run in source_runs {
        sqlx::query(
            "INSERT INTO source_runs (source_name, started_at, finished_at, status, item_count, error_text) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(run.source_name)
        .bind(format_time(run.started_at)?)
        .bind(format_time(run.finished_at)?)
        .bind(run.status)
        .bind(run.item_count as i64)
        .bind(run.error_text)
        .execute(&mut *connection)
        .await?;
    }
    sqlx::query("UPDATE source_runs SET finished_at = ?, status = ?, item_count = ?, error_text = ? WHERE id = ?")
        .bind(format_time(finished_at)?)
        .bind(aggregate_status)
        .bind(event_count as i64)
        .bind(aggregate_error)
        .bind(aggregate_run_id)
        .execute(&mut *connection)
        .await?;

    transaction.commit().await?;
    Ok(event_count)
}

async fn replace_reference_data(
    connection: &mut SqliteConnection,
    providers: &[ProviderCatalogEntry],
    rules: &[CompetitionRule],
) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM providers")
        .execute(&mut *connection)
        .await?;
    sqlx::query("DELETE FROM competition_rules")
        .execute(&mut *connection)
        .await?;
    for provider in providers {
        sqlx::query("INSERT INTO providers (family, market, aliases_json) VALUES (?, ?, ?)")
            .bind(&provider.family)
            .bind(&provider.market)
            .bind(serde_json::to_string(&provider.aliases)?)
            .execute(&mut *connection)
            .await?;
    }
    for rule in rules {
        sqlx::query("INSERT INTO competition_rules (competition, market, provider_family, watch_type, confidence) VALUES (?, ?, ?, ?, ?)")
            .bind(&rule.competition)
            .bind(&rule.market)
            .bind(&rule.provider_family)
            .bind(&rule.watch_type)
            .bind(rule.confidence)
            .execute(&mut *connection)
            .await?;
    }
    Ok(())
}

async fn insert_event(connection: &mut SqliteConnection, event: &Event) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO events (
            id, sport, competition, title, start_time, end_time, status, venue, round_label,
            participants_json, source, source_url, search_metadata_json, recommended_market, recommended_provider
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&event.id)
    .bind(&event.sport)
    .bind(&event.competition)
    .bind(&event.title)
    .bind(format_time(event.start_time)?)
    .bind(event.end_time.map(format_time).transpose()?)
    .bind(serde_json::to_string(&event.status)?)
    .bind(&event.venue)
    .bind(&event.round_label)
    .bind(serde_json::to_string(&event.participants)?)
    .bind(&event.source)
    .bind(&event.source_url)
    .bind(serde_json::to_string(&event.search_metadata)?)
    .bind(&event.watch.recommended_market)
    .bind(&event.watch.recommended_provider)
    .execute(&mut *connection)
    .await?;

    for (ordinal, availability) in event.watch.availabilities.iter().enumerate() {
        sqlx::query(
            r#"
            INSERT INTO watch_availabilities (
                event_id, ordinal, market, provider_family, provider_label, channel_name,
                watch_type, priority, confidence, source, search_hints_json
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&event.id)
        .bind(ordinal as i64)
        .bind(&availability.market)
        .bind(&availability.provider_family)
        .bind(&availability.provider_label)
        .bind(&availability.channel_name)
        .bind(&availability.watch_type)
        .bind(availability.priority)
        .bind(availability.confidence)
        .bind(&availability.source)
        .bind(serde_json::to_string(&availability.search_hints)?)
        .execute(&mut *connection)
        .await?;
    }
    Ok(())
}

pub enum EventFilter<'a> {
    Id(&'a str),
    Competition(&'a str),
    StartsBetween(OffsetDateTime, OffsetDateTime),
    ActiveAt(OffsetDateTime),
    Overlaps(OffsetDateTime, OffsetDateTime),
}

pub async fn load_events(pool: &SqlitePool, filter: EventFilter<'_>) -> anyhow::Result<Vec<Event>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT id, sport, competition, title, start_time, end_time, status, venue, round_label, participants_json, source, source_url, search_metadata_json, recommended_market, recommended_provider FROM events",
    );
    match filter {
        EventFilter::Id(id) => {
            query.push(" WHERE id = ").push_bind(id);
        }
        EventFilter::Competition(competition) => {
            query.push(" WHERE competition = ").push_bind(competition);
        }
        EventFilter::StartsBetween(start, end) => {
            query
                .push(" WHERE julianday(start_time) >= julianday(")
                .push_bind(format_time(start)?)
                .push(") AND julianday(start_time) <= julianday(")
                .push_bind(format_time(end)?)
                .push(")");
        }
        EventFilter::ActiveAt(now) => {
            let now = format_time(now)?;
            query
                .push(" WHERE julianday(start_time) <= julianday(")
                .push_bind(now.clone())
                .push(") AND end_time IS NOT NULL AND julianday(end_time) > julianday(")
                .push_bind(now)
                .push(")");
        }
        EventFilter::Overlaps(start, end) => {
            query
                .push(" WHERE julianday(start_time) < julianday(")
                .push_bind(format_time(end)?)
                .push(") AND ((end_time IS NOT NULL AND julianday(end_time) > julianday(")
                .push_bind(format_time(start)?)
                .push(")) OR (end_time IS NULL AND julianday(start_time) >= julianday(")
                .push_bind(format_time(start)?)
                .push(")))");
        }
    }
    query.push(" ORDER BY start_time ASC, id ASC");
    let rows = query.build().fetch_all(pool).await?;
    let ids = rows
        .iter()
        .map(|row| row.try_get::<String, _>("id"))
        .collect::<Result<Vec<_>, _>>()?;
    let mut availabilities = load_availabilities_batch(pool, &ids).await?;

    rows.into_iter()
        .map(|row| {
            let id: String = row.try_get("id")?;
            Ok(Event {
                watch: crate::domain::EventWatch {
                    recommended_market: row.try_get("recommended_market")?,
                    recommended_provider: row.try_get("recommended_provider")?,
                    availabilities: availabilities.remove(&id).unwrap_or_default(),
                },
                id,
                sport: row.try_get("sport")?,
                competition: row.try_get("competition")?,
                title: row.try_get("title")?,
                start_time: parse_time(&row.try_get::<String, _>("start_time")?)?,
                end_time: row
                    .try_get::<Option<String>, _>("end_time")?
                    .map(|value| parse_time(&value))
                    .transpose()?,
                status: serde_json::from_str(&row.try_get::<String, _>("status")?)?,
                venue: row.try_get("venue")?,
                round_label: row.try_get("round_label")?,
                participants: serde_json::from_str(
                    &row.try_get::<String, _>("participants_json")?,
                )?,
                source: row.try_get("source")?,
                source_url: row.try_get("source_url")?,
                search_metadata: serde_json::from_str(
                    &row.try_get::<String, _>("search_metadata_json")?,
                )?,
            })
        })
        .collect()
}

async fn load_availabilities_batch(
    pool: &SqlitePool,
    event_ids: &[String],
) -> anyhow::Result<HashMap<String, Vec<WatchAvailability>>> {
    if event_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT event_id, market, provider_family, provider_label, channel_name, watch_type, priority, confidence, source, search_hints_json FROM watch_availabilities WHERE event_id IN (",
    );
    let mut separated = query.separated(", ");
    for id in event_ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(") ORDER BY event_id, ordinal ASC");
    let rows = query.build().fetch_all(pool).await?;
    let mut grouped: HashMap<String, Vec<WatchAvailability>> = HashMap::new();
    for row in rows {
        grouped
            .entry(row.try_get("event_id")?)
            .or_default()
            .push(WatchAvailability {
                market: row.try_get("market")?,
                provider_family: row.try_get("provider_family")?,
                provider_label: row.try_get("provider_label")?,
                channel_name: row.try_get("channel_name")?,
                watch_type: row.try_get("watch_type")?,
                priority: row.try_get("priority")?,
                confidence: row.try_get("confidence")?,
                source: row.try_get("source")?,
                search_hints: serde_json::from_str(
                    &row.try_get::<String, _>("search_hints_json")?,
                )?,
            });
    }
    Ok(grouped)
}

pub async fn insert_source_run(
    pool: &SqlitePool,
    source_name: &str,
    started_at: OffsetDateTime,
) -> anyhow::Result<i64> {
    let result = sqlx::query(
        "INSERT INTO source_runs (source_name, started_at, status) VALUES (?, ?, 'running')",
    )
    .bind(source_name)
    .bind(started_at.format(&time::format_description::well_known::Rfc3339)?)
    .execute(pool)
    .await?;
    Ok(result.last_insert_rowid())
}

pub async fn finish_source_run(
    pool: &SqlitePool,
    id: i64,
    finished_at: OffsetDateTime,
    status: &str,
    item_count: i64,
    error_text: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE source_runs SET finished_at = ?, status = ?, item_count = ?, error_text = ? WHERE id = ?")
        .bind(finished_at.format(&time::format_description::well_known::Rfc3339)?)
        .bind(status)
        .bind(item_count)
        .bind(error_text)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

#[derive(Debug)]
pub struct RefreshHealth {
    pub latest_status: Option<String>,
    pub last_success_at: Option<OffsetDateTime>,
}

pub async fn load_refresh_health(pool: &SqlitePool) -> anyhow::Result<RefreshHealth> {
    sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(pool)
        .await?;
    let latest_status = sqlx::query_scalar::<_, String>(
        "SELECT status FROM source_runs WHERE source_name = 'configured_sources' AND finished_at IS NOT NULL ORDER BY id DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    let last_success_raw = sqlx::query_scalar::<_, String>(
        "SELECT finished_at FROM source_runs WHERE source_name = 'configured_sources' AND finished_at IS NOT NULL AND status IN ('success', 'degraded') AND item_count > 0 ORDER BY id DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    let last_success_at = last_success_raw.as_deref().map(parse_time).transpose()?;
    Ok(RefreshHealth {
        latest_status,
        last_success_at,
    })
}

fn format_time(value: OffsetDateTime) -> anyhow::Result<String> {
    Ok(value.format(&time::format_description::well_known::Rfc3339)?)
}

fn parse_time(value: &str) -> anyhow::Result<OffsetDateTime> {
    Ok(OffsetDateTime::parse(
        value,
        &time::format_description::well_known::Rfc3339,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{EventStatus, EventWatch, Participants, SearchMetadata};
    use time::macros::datetime;

    fn event(id: &str, title: &str) -> Event {
        Event {
            id: id.into(),
            sport: "test".into(),
            competition: "test_competition".into(),
            title: title.into(),
            start_time: datetime!(2026-04-01 10:00 UTC),
            end_time: Some(datetime!(2026-04-01 12:00 UTC)),
            status: EventStatus::Upcoming,
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

    #[tokio::test]
    async fn failed_snapshot_replacement_rolls_back_deletes() {
        let path = std::env::temp_dir().join(format!(
            "euripus-rollback-{}-{}.db",
            std::process::id(),
            OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        let pool = connect(&format!("sqlite://{}", path.display()))
            .await
            .unwrap();
        init(&pool).await.unwrap();
        let run_a = insert_source_run(&pool, "aggregate", datetime!(2026-04-01 08:00 UTC))
            .await
            .unwrap();
        let snapshot_a = HashMap::from([("test_competition".into(), vec![event("old", "Old")])]);
        replace_snapshot(
            &pool,
            &[],
            &[],
            &snapshot_a,
            &[],
            run_a,
            "success",
            None,
            datetime!(2026-04-01 08:01 UTC),
        )
        .await
        .unwrap();

        let run_b = insert_source_run(&pool, "aggregate", datetime!(2026-04-01 09:00 UTC))
            .await
            .unwrap();
        let snapshot_b = HashMap::from([(
            "test_competition".into(),
            vec![event("duplicate", "One"), event("duplicate", "Two")],
        )]);
        assert!(replace_snapshot(
            &pool,
            &[],
            &[],
            &snapshot_b,
            &[],
            run_b,
            "success",
            None,
            datetime!(2026-04-01 09:01 UTC)
        )
        .await
        .is_err());

        let events = load_events(&pool, EventFilter::Competition("test_competition"))
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, "old");
        assert_eq!(events[0].title, "Old");
        pool.close().await;
        let _ = std::fs::remove_file(path);
    }
}
