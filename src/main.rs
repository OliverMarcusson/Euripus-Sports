mod api;
mod config;
mod db;
mod domain;
mod inference;
mod ingest;
mod jobs;
mod repository;
mod sources;

use std::{net::SocketAddr, sync::Arc, time::Duration};

use anyhow::Context;
use axum::Router;
use clap::Parser;
use config::AppConfig;
use ingest::SourceFetchMode;
use repository::AppState;
use sqlx::SqlitePool;
use tower_http::{compression::CompressionLayer, cors::CorsLayer, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Debug, Parser)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(long, default_value = "0.0.0.0:3000")]
    listen: SocketAddr,

    #[arg(long, default_value = "config/providers.yaml")]
    providers: String,

    #[arg(long, default_value = "config/competition_rules.yaml")]
    rules: String,

    #[arg(long, default_value = "config/sample_events.yaml")]
    events: String,

    #[arg(long, default_value = "config/sources.yaml")]
    sources: String,

    #[arg(long, default_value = "config/team_aliases.yaml")]
    team_aliases: String,

    #[arg(long, default_value = "sqlite://sports-api.db")]
    database_url: String,

    #[arg(long, value_enum, default_value_t = SourceFetchMode::Fixture)]
    source_fetch_mode: SourceFetchMode,

    #[arg(long, default_value = "agent-browser")]
    browser_command: String,

    #[arg(long, value_name = "DURATION", value_parser = parse_refresh_interval)]
    refresh_interval: Option<Duration>,
}

#[derive(Debug, clap::Subcommand)]
enum Commands {
    Refresh,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "sports_api=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();
    let config = AppConfig::load(
        &cli.providers,
        &cli.rules,
        &cli.events,
        &cli.sources,
        &cli.team_aliases,
    )
    .with_context(|| "failed to load config files")?;

    if matches!(cli.command, Some(Commands::Refresh)) {
        let pool = db::connect(&cli.database_url).await?;
        db::init(&pool).await?;
        let summary =
            jobs::refresh_sources(&pool, &config, cli.source_fetch_mode, &cli.browser_command)
                .await?;
        tracing::info!(event_count = summary.event_count, started_at = %summary.started_at, finished_at = %summary.finished_at, "refresh completed");
        return Ok(());
    }

    let state = Arc::new(
        AppState::new(
            config.clone(),
            &cli.database_url,
            cli.source_fetch_mode,
            &cli.browser_command,
        )
        .await?,
    );

    if let Some(refresh_interval) = cli.refresh_interval {
        spawn_refresh_loop(
            state.pool.clone(),
            config,
            cli.source_fetch_mode,
            cli.browser_command.clone(),
            refresh_interval,
        );
    }

    let app = app_router(state);

    tracing::info!(listen = %cli.listen, "starting sports api");
    let listener = tokio::net::TcpListener::bind(cli.listen).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn app_router(state: Arc<AppState>) -> Router {
    api::router(state)
        .layer(CompressionLayer::new())
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
}

fn parse_refresh_interval(value: &str) -> Result<Duration, String> {
    let duration = humantime::parse_duration(value)
        .map_err(|error| format!("invalid refresh interval '{value}': {error}"))?;
    if duration.is_zero() {
        return Err("refresh interval must be greater than zero".into());
    }
    Ok(duration)
}

fn spawn_refresh_loop(
    pool: SqlitePool,
    config: AppConfig,
    source_fetch_mode: SourceFetchMode,
    browser_command: String,
    refresh_interval: Duration,
) {
    tracing::info!(refresh_interval = ?refresh_interval, source_fetch_mode = ?source_fetch_mode, "starting periodic refresh loop");
    tokio::spawn(async move {
        let mut interval = tokio::time::interval_at(
            tokio::time::Instant::now() + refresh_interval,
            refresh_interval,
        );
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            interval.tick().await;
            match jobs::refresh_sources(&pool, &config, source_fetch_mode, &browser_command).await {
                Ok(summary) => tracing::info!(
                    event_count = summary.event_count,
                    started_at = %summary.started_at,
                    finished_at = %summary.finished_at,
                    "periodic refresh completed"
                ),
                Err(error) => tracing::error!(error = %error, "periodic refresh failed"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::parse_refresh_interval;

    #[test]
    fn parses_refresh_interval() {
        assert_eq!(parse_refresh_interval("10m").unwrap().as_secs(), 600);
        assert!(parse_refresh_interval("0s").is_err());
        assert!(parse_refresh_interval("banana").is_err());
    }
}
