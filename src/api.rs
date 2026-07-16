use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::Deserialize;

use crate::{
    domain::{CompetitionEventsResponse, EventsResponse},
    repository::{
        SharedState, UpcomingHours, UPCOMING_HOURS_DEFAULT, UPCOMING_HOURS_MAX, UPCOMING_HOURS_MIN,
    },
};

pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/health/live", get(health))
        .route("/health/ready", get(readiness))
        .route("/v1/events/live", get(live_events))
        .route("/v1/events/upcoming", get(upcoming_events))
        .route("/v1/events/today", get(today_events))
        .route("/v1/events/{id}", get(event_by_id))
        .route("/v1/competitions/{slug}", get(competition_events))
        .route("/v1/providers", get(providers))
        .with_state(state)
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({"status": "ok"}))
}

async fn readiness(State(state): State<SharedState>) -> impl IntoResponse {
    let readiness = state.readiness().await;
    let status = if readiness.status == "ready" {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(readiness))
}

async fn live_events(
    State(state): State<SharedState>,
) -> Result<Json<EventsResponse>, (StatusCode, Json<serde_json::Value>)> {
    let mut events = state.live_events().await.map_err(internal_error)?;
    sort_events(&mut events);
    Ok(Json(EventsResponse {
        count: events.len(),
        events,
    }))
}

#[derive(Debug, Deserialize)]
struct UpcomingQuery {
    hours: Option<i64>,
}

async fn upcoming_events(
    State(state): State<SharedState>,
    Query(query): Query<UpcomingQuery>,
) -> Result<Json<EventsResponse>, (StatusCode, Json<serde_json::Value>)> {
    let hours = UpcomingHours::try_from(query.hours.unwrap_or(UPCOMING_HOURS_DEFAULT))
        .map_err(|_| invalid_hours())?;
    let mut events = state.upcoming_events(hours).await.map_err(internal_error)?;
    sort_events(&mut events);
    Ok(Json(EventsResponse {
        count: events.len(),
        events,
    }))
}

async fn today_events(
    State(state): State<SharedState>,
) -> Result<Json<EventsResponse>, (StatusCode, Json<serde_json::Value>)> {
    let mut events = state.today_events().await.map_err(internal_error)?;
    sort_events(&mut events);
    Ok(Json(EventsResponse {
        count: events.len(),
        events,
    }))
}

async fn event_by_id(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let event = state
        .event_by_id(&id)
        .await
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    Ok(Json(serde_json::to_value(event).expect("event serializes")))
}

async fn competition_events(
    State(state): State<SharedState>,
    Path(slug): Path<String>,
) -> Result<Json<CompetitionEventsResponse>, (StatusCode, Json<serde_json::Value>)> {
    let mut events = state
        .events_for_competition(&slug)
        .await
        .map_err(internal_error)?;
    if events.is_empty() {
        return Err(not_found());
    }
    sort_events(&mut events);
    Ok(Json(CompetitionEventsResponse {
        competition: slug,
        events,
    }))
}

async fn providers(State(state): State<SharedState>) -> impl IntoResponse {
    Json(serde_json::json!({
        "count": state.providers.len(),
        "providers": state.providers,
    }))
}

fn invalid_hours() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({
            "error": "invalid_hours",
            "min": UPCOMING_HOURS_MIN,
            "max": UPCOMING_HOURS_MAX,
        })),
    )
}

fn not_found() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"error": "not_found"})),
    )
}

fn internal_error(error: anyhow::Error) -> (StatusCode, Json<serde_json::Value>) {
    tracing::error!(error = %error, "request failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": "internal_error"})),
    )
}

fn sort_events(events: &mut [crate::domain::Event]) {
    events.sort_by_key(|event| event.start_time);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::AppConfig, ingest::SourceFetchMode, jobs, repository::AppState};
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use std::time::Duration;
    use tower::ServiceExt;

    fn test_config() -> AppConfig {
        AppConfig::load(
            "config/providers.yaml",
            "config/competition_rules.yaml",
            "config/sample_events.yaml",
            "config/sources.yaml",
            "config/team_aliases.yaml",
        )
        .unwrap()
    }

    async fn state(config: &AppConfig) -> AppState {
        AppState::initialize(config, "sqlite::memory:", Duration::from_secs(1800))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn providers_endpoint_works() {
        let config = test_config();
        let app = router(std::sync::Arc::new(state(&config).await));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/providers")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn formula1_competition_endpoint_works() {
        let config = test_config();
        let state = state(&config).await;
        jobs::refresh_sources(
            &state.pool,
            &config,
            SourceFetchMode::Fixture,
            "agent-browser",
        )
        .await
        .unwrap();
        let app = router(std::sync::Arc::new(state));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/competitions/formula_1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn upcoming_hours_boundaries_and_extremes_are_validated() {
        let config = test_config();
        let app = router(std::sync::Arc::new(state(&config).await));

        for hours in [None, Some("1"), Some("72"), Some("8760")] {
            let uri = hours.map_or_else(
                || "/v1/events/upcoming".to_string(),
                |hours| format!("/v1/events/upcoming?hours={hours}"),
            );
            let response = app
                .clone()
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }

        for hours in [
            "0",
            "-1",
            "8761",
            "-9223372036854775808",
            "9223372036854775807",
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/v1/events/upcoming?hours={hours}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            let body = response.into_body().collect().await.unwrap().to_bytes();
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(json["error"], "invalid_hours");
            assert_eq!(json["min"], 1);
            assert_eq!(json["max"], 8760);
        }

        let malformed = app
            .oneshot(
                Request::builder()
                    .uri("/v1/events/upcoming?hours=nope")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn liveness_is_available_before_readiness() {
        let config = test_config();
        let app = router(std::sync::Arc::new(state(&config).await));
        let live = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/health/live")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(live.status(), StatusCode::OK);
        let ready = app
            .oneshot(
                Request::builder()
                    .uri("/health/ready")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ready.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
