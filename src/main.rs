mod api;
mod config;
mod db;
mod domain;
mod inference;
mod ingest;
mod jobs;
mod repository;
mod sources;
mod time_utils;

use std::{net::SocketAddr, sync::Arc, time::Duration};

use anyhow::Context;
use axum::{
    http::{header, HeaderValue, Method, Uri},
    Router,
};
use clap::Parser;
use config::AppConfig;
use ingest::SourceFetchMode;
use repository::AppState;
use sqlx::SqlitePool;
use tower_http::{
    compression::CompressionLayer,
    cors::{AllowOrigin, CorsLayer},
    trace::TraceLayer,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Debug, Parser)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(long, default_value = "127.0.0.1:3000")]
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

    #[arg(long, value_name = "DURATION", default_value = "30m", value_parser = parse_refresh_interval)]
    readiness_max_refresh_age: Duration,

    #[arg(long, value_name = "ORIGIN")]
    cors_origin: Vec<String>,
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
        tracing::info!(event_count = summary.event_count, status = ?summary.status, started_at = %summary.started_at, finished_at = %summary.finished_at, "refresh completed");
        return Ok(());
    }

    let state = Arc::new(
        AppState::initialize(&config, &cli.database_url, cli.readiness_max_refresh_age).await?,
    );
    let app = app_router(state.clone(), &cli.cors_origin)?;

    tracing::info!(listen = %cli.listen, "starting sports api");
    let listener = tokio::net::TcpListener::bind(cli.listen).await?;
    spawn_refresh_worker(
        state.pool.clone(),
        config,
        cli.source_fetch_mode,
        cli.browser_command.clone(),
        cli.refresh_interval,
    );
    axum::serve(listener, app).await?;
    Ok(())
}

fn app_router(state: Arc<AppState>, cors_origins: &[String]) -> anyhow::Result<Router> {
    let mut router = api::router(state)
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http());
    if !cors_origins.is_empty() {
        let origins = cors_origins
            .iter()
            .map(|origin| parse_cors_origin(origin))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let cors = CorsLayer::new()
            .allow_origin(AllowOrigin::list(origins))
            .allow_methods([Method::GET, Method::HEAD, Method::OPTIONS])
            .allow_headers([header::ACCEPT, header::CONTENT_TYPE]);
        router = router.layer(cors);
    }
    Ok(router)
}

fn parse_cors_origin(value: &str) -> anyhow::Result<HeaderValue> {
    let uri = value
        .parse::<Uri>()
        .with_context(|| format!("invalid CORS origin '{value}'"))?;
    if uri.scheme().is_none()
        || uri.authority().is_none()
        || uri.path() != "/"
        || uri.query().is_some()
    {
        anyhow::bail!("invalid CORS origin '{value}'");
    }
    HeaderValue::from_str(value).with_context(|| format!("invalid CORS origin '{value}'"))
}

fn parse_refresh_interval(value: &str) -> Result<Duration, String> {
    let duration = humantime::parse_duration(value)
        .map_err(|error| format!("invalid refresh interval '{value}': {error}"))?;
    if duration.is_zero() {
        return Err("refresh interval must be greater than zero".into());
    }
    Ok(duration)
}

fn spawn_refresh_worker(
    pool: SqlitePool,
    config: AppConfig,
    source_fetch_mode: SourceFetchMode,
    browser_command: String,
    refresh_interval: Option<Duration>,
) {
    tracing::info!(refresh_interval = ?refresh_interval, source_fetch_mode = ?source_fetch_mode, "starting refresh worker");
    tokio::spawn(async move {
        let mut interval = refresh_interval.map(|refresh_interval| {
            let mut interval = tokio::time::interval_at(
                tokio::time::Instant::now() + refresh_interval,
                refresh_interval,
            );
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            interval
        });
        loop {
            match jobs::refresh_sources(&pool, &config, source_fetch_mode, &browser_command).await {
                Ok(summary) => tracing::info!(
                    event_count = summary.event_count,
                    status = ?summary.status,
                    started_at = %summary.started_at,
                    finished_at = %summary.finished_at,
                    "refresh completed"
                ),
                Err(error) => tracing::error!(error = %error, "refresh failed"),
            }

            let Some(interval) = interval.as_mut() else {
                break;
            };
            interval.tick().await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    #[test]
    fn parses_refresh_interval() {
        assert_eq!(parse_refresh_interval("10m").unwrap().as_secs(), 600);
        assert!(parse_refresh_interval("0s").is_err());
        assert!(parse_refresh_interval("banana").is_err());
    }

    #[test]
    fn cli_defaults_to_loopback_and_validates_cors_origins() {
        let cli = Cli::try_parse_from(["sports-api"]).unwrap();
        assert_eq!(cli.listen, "127.0.0.1:3000".parse().unwrap());
        assert_eq!(cli.readiness_max_refresh_age, Duration::from_secs(1800));
        assert!(parse_cors_origin("https://euripus.example").is_ok());
        assert!(parse_cors_origin("not an origin").is_err());
        assert!(parse_cors_origin("https://euripus.example/path").is_err());
    }

    #[tokio::test]
    async fn cors_is_disabled_by_default_and_exact_when_configured() {
        let config = AppConfig::load(
            "config/providers.yaml",
            "config/competition_rules.yaml",
            "config/sample_events.yaml",
            "config/sources.yaml",
            "config/team_aliases.yaml",
        )
        .unwrap();
        let state = Arc::new(
            AppState::initialize(&config, "sqlite::memory:", Duration::from_secs(1800))
                .await
                .unwrap(),
        );
        let request = |origin: &'static str| {
            Request::builder()
                .uri("/health/live")
                .header(header::ORIGIN, origin)
                .body(Body::empty())
                .unwrap()
        };

        let response = app_router(state.clone(), &[])
            .unwrap()
            .oneshot(request("https://euripus.example"))
            .await
            .unwrap();
        assert!(response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none());

        let origins = vec!["https://euripus.example".to_string()];
        let allowed = app_router(state.clone(), &origins)
            .unwrap()
            .oneshot(request("https://euripus.example"))
            .await
            .unwrap();
        assert_eq!(
            allowed
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .unwrap(),
            "https://euripus.example"
        );
        let denied = app_router(state, &origins)
            .unwrap()
            .oneshot(request("https://other.example"))
            .await
            .unwrap();
        assert!(denied
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none());
    }
}
