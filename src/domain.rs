use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Participants {
    pub home: String,
    pub away: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventSeed {
    pub id: String,
    pub sport: String,
    pub competition: String,
    pub title: String,
    #[serde(with = "time::serde::rfc3339")]
    pub start_time: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub end_time: Option<OffsetDateTime>,
    pub status: EventStatus,
    pub venue: Option<String>,
    pub round_label: Option<String>,
    pub participants: Participants,
    pub source: String,
    pub source_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventStatus {
    Upcoming,
    Live,
    Finished,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompetitionRule {
    pub competition: String,
    pub market: String,
    pub provider_family: String,
    pub watch_type: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCatalogEntry {
    pub family: String,
    pub market: String,
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchOverlay {
    pub competition: String,
    pub market: String,
    pub provider_family: String,
    pub provider_label: String,
    pub title: String,
    pub participants: Participants,
    pub channel_name: Option<String>,
    pub watch_type: String,
    pub confidence: f32,
    pub source: String,
    pub source_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchMetadata {
    pub queries: Vec<String>,
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchAvailability {
    pub market: String,
    pub provider_family: String,
    pub provider_label: String,
    pub channel_name: Option<String>,
    pub watch_type: String,
    pub priority: i32,
    pub confidence: f32,
    pub source: String,
    pub search_hints: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub sport: String,
    pub competition: String,
    pub title: String,
    #[serde(with = "time::serde::rfc3339")]
    pub start_time: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub end_time: Option<OffsetDateTime>,
    pub status: EventStatus,
    pub venue: Option<String>,
    pub round_label: Option<String>,
    pub participants: Participants,
    pub source: String,
    pub source_url: String,
    pub watch: EventWatch,
    pub search_metadata: SearchMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventWatch {
    pub recommended_market: Option<String>,
    pub recommended_provider: Option<String>,
    pub availabilities: Vec<WatchAvailability>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompetitionEventsResponse {
    pub competition: String,
    pub events: Vec<Event>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventsResponse {
    pub count: usize,
    pub events: Vec<Event>,
}
