use anyhow::Context;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    Row, SqlitePool,
};
use std::str::FromStr;
use time::OffsetDateTime;

use crate::domain::{CompetitionRule, Event, ProviderCatalogEntry, WatchAvailability};

pub async fn connect(database_url: &str) -> anyhow::Result<SqlitePool> {
    let options = SqliteConnectOptions::from_str(database_url)
        .with_context(|| format!("parsing database url {database_url}"))?
        .create_if_missing(true);

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

pub async fn seed_reference_data(
    pool: &SqlitePool,
    providers: &[ProviderCatalogEntry],
    rules: &[CompetitionRule],
) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM providers").execute(pool).await?;
    sqlx::query("DELETE FROM competition_rules")
        .execute(pool)
        .await?;

    for provider in providers {
        sqlx::query("INSERT INTO providers (family, market, aliases_json) VALUES (?, ?, ?)")
            .bind(&provider.family)
            .bind(&provider.market)
            .bind(serde_json::to_string(&provider.aliases)?)
            .execute(pool)
            .await?;
    }

    for rule in rules {
        sqlx::query(
            "INSERT INTO competition_rules (competition, market, provider_family, watch_type, confidence) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&rule.competition)
        .bind(&rule.market)
        .bind(&rule.provider_family)
        .bind(&rule.watch_type)
        .bind(rule.confidence)
        .execute(pool)
        .await?;
    }

    Ok(())
}

pub async fn seed_events(pool: &SqlitePool, events: &[Event]) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM watch_availabilities")
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM events").execute(pool).await?;

    for event in events {
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
        .bind(event.start_time.format(&time::format_description::well_known::Rfc3339)?)
        .bind(event.end_time.map(|dt| dt.format(&time::format_description::well_known::Rfc3339)).transpose()?)
        .bind(serde_json::to_string(&event.status)?)
        .bind(&event.venue)
        .bind(&event.round_label)
        .bind(serde_json::to_string(&event.participants)?)
        .bind(&event.source)
        .bind(&event.source_url)
        .bind(serde_json::to_string(&event.search_metadata)?)
        .bind(&event.watch.recommended_market)
        .bind(&event.watch.recommended_provider)
        .execute(pool)
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
            .execute(pool)
            .await?;
        }
    }

    Ok(())
}

pub async fn load_events(pool: &SqlitePool) -> anyhow::Result<Vec<Event>> {
    let rows = sqlx::query(
        r#"
        SELECT id, sport, competition, title, start_time, end_time, status, venue, round_label,
               participants_json, source, source_url, search_metadata_json, recommended_market, recommended_provider
        FROM events
        ORDER BY start_time ASC, id ASC
        "#,
    )
    .fetch_all(pool)
    .await?;

    let mut events = Vec::with_capacity(rows.len());
    for row in rows {
        let id: String = row.try_get("id")?;
        let availabilities = load_availabilities(pool, &id).await?;
        let start_time = parse_time(&row.try_get::<String, _>("start_time")?)?;
        let end_time = row
            .try_get::<Option<String>, _>("end_time")?
            .map(|value| parse_time(&value))
            .transpose()?;

        events.push(Event {
            id,
            sport: row.try_get("sport")?,
            competition: row.try_get("competition")?,
            title: row.try_get("title")?,
            start_time,
            end_time,
            status: serde_json::from_str(&row.try_get::<String, _>("status")?)?,
            venue: row.try_get("venue")?,
            round_label: row.try_get("round_label")?,
            participants: serde_json::from_str(&row.try_get::<String, _>("participants_json")?)?,
            source: row.try_get("source")?,
            source_url: row.try_get("source_url")?,
            watch: crate::domain::EventWatch {
                recommended_market: row.try_get("recommended_market")?,
                recommended_provider: row.try_get("recommended_provider")?,
                availabilities,
            },
            search_metadata: serde_json::from_str(
                &row.try_get::<String, _>("search_metadata_json")?,
            )?,
        });
    }

    Ok(events)
}

async fn load_availabilities(
    pool: &SqlitePool,
    event_id: &str,
) -> anyhow::Result<Vec<WatchAvailability>> {
    let rows = sqlx::query(
        r#"
        SELECT market, provider_family, provider_label, channel_name, watch_type, priority, confidence, source, search_hints_json
        FROM watch_availabilities
        WHERE event_id = ?
        ORDER BY ordinal ASC
        "#,
    )
    .bind(event_id)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| {
            Ok(WatchAvailability {
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
            })
        })
        .collect()
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

pub async fn load_providers(pool: &SqlitePool) -> anyhow::Result<Vec<ProviderCatalogEntry>> {
    let rows =
        sqlx::query("SELECT family, market, aliases_json FROM providers ORDER BY market, family")
            .fetch_all(pool)
            .await?;

    rows.into_iter()
        .map(|row| {
            Ok(ProviderCatalogEntry {
                family: row.try_get("family")?,
                market: row.try_get("market")?,
                aliases: serde_json::from_str(&row.try_get::<String, _>("aliases_json")?)?,
            })
        })
        .collect()
}

fn parse_time(value: &str) -> anyhow::Result<OffsetDateTime> {
    Ok(OffsetDateTime::parse(
        value,
        &time::format_description::well_known::Rfc3339,
    )?)
}
