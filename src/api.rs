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
    repository::SharedState,
};

pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/health", get(health))
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
    let mut events = state
        .upcoming_events(query.hours.unwrap_or(72))
        .await
        .map_err(internal_error)?;
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
    use crate::{config::AppConfig, ingest::SourceFetchMode, repository::AppState};
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    #[tokio::test]
    async fn providers_endpoint_works() {
        let config = AppConfig::load(
            "config/providers.yaml",
            "config/competition_rules.yaml",
            "config/sample_events.yaml",
            "config/sources.yaml",
            "config/team_aliases.yaml",
        )
        .unwrap();
        let app = router(std::sync::Arc::new(
            AppState::new(
                config,
                "sqlite::memory:",
                SourceFetchMode::Fixture,
                "agent-browser",
            )
            .await
            .unwrap(),
        ));
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
}
