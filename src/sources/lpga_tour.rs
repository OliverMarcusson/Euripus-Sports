use regex::Regex;
use scraper::{Html, Selector};
use time::{macros::offset, Date, Month, OffsetDateTime, PrimitiveDateTime, UtcOffset};

use crate::domain::{EventSeed, EventStatus, Participants, WatchOverlay};

const STOCKHOLM: UtcOffset = offset!(+2);
const SOURCE_URL: &str = "https://www.lpga.com/tournaments?year=2026";
const SVENSK_GOLF_URL: &str = "https://www.svenskgolf.se/sidor/har-ar-veckans-livesandningar/";

pub fn parse_schedule_document(input: &str, season: i32) -> Vec<EventSeed> {
    if input.contains("<html") || input.contains("<!DOCTYPE html") {
        return parse_schedule_html_document(input, season);
    }

    let line_re = Regex::new(r"^(?P<name>[^|]+)\|(?P<month>[A-Za-z]+)\s+(?P<start>\d{1,2})-(?P<end>\d{1,2})\|(?P<venue>[^|]+)$").unwrap();
    input
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let caps = line_re.captures(line)?;
            build_event(
                caps.name("name")?.as_str().trim(),
                parse_month(caps.name("month")?.as_str())?,
                caps.name("start")?.as_str().parse::<u8>().ok()?,
                caps.name("venue")?.as_str().trim(),
                season,
            )
        })
        .collect()
}

fn parse_schedule_html_document(input: &str, season: i32) -> Vec<EventSeed> {
    let document = Html::parse_document(input);
    let item_selector =
        Selector::parse("article[data-tournament], .tournament-card, li[data-tournament]").unwrap();
    let name_selector = Selector::parse("h2, h3, .title").unwrap();
    let date_selector = Selector::parse(".date, time, .tournament-date").unwrap();
    let venue_selector = Selector::parse(".venue, .location, .tournament-location").unwrap();

    let mut events = Vec::new();
    for item in document.select(&item_selector) {
        let name = item
            .select(&name_selector)
            .next()
            .map(text)
            .or_else(|| item.value().attr("data-tournament").map(str::to_string));
        let date_text = item.select(&date_selector).next().map(text);
        let venue = item
            .select(&venue_selector)
            .next()
            .map(text)
            .unwrap_or_default();
        let Some(name) = name else { continue };
        let Some(date_text) = date_text else { continue };
        let Some((month, day)) = parse_date_label(&date_text) else {
            continue;
        };
        if let Some(event) = build_event(&name, month, day, &venue, season) {
            events.push(event);
        }
    }
    events
}

pub fn parse_svensk_golf_watch_document(input: &str, _season: i32) -> Vec<WatchOverlay> {
    let line_re =
        Regex::new(r"^(?P<name>.+?)\s+Round\s+(?P<round>\d)\s*[|:-]\s*(?P<label>.+)$").unwrap();
    input
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let caps = line_re.captures(line)?;
            let tournament = caps.name("name")?.as_str().trim();
            let round = caps.name("round")?.as_str().parse::<u8>().ok()?;
            let label = caps.name("label")?.as_str().trim();
            Some(WatchOverlay {
                competition: "lpga_tour".into(),
                market: "se".into(),
                provider_family: "viaplay".into(),
                provider_label: "Viaplay".into(),
                title: format!("{} Round {}", tournament, round),
                participants: Participants {
                    home: tournament.to_string(),
                    away: "Field".into(),
                },
                channel_name: Some(label.to_string()),
                watch_type: "streaming+linear".into(),
                confidence: 0.97,
                source: "svensk-golf-tv-guide".into(),
                source_url: SVENSK_GOLF_URL.into(),
            })
        })
        .collect()
}

fn build_event(name: &str, month: Month, day: u8, venue: &str, season: i32) -> Option<EventSeed> {
    let date = Date::from_calendar_date(season, month, day).ok()?;
    let start_time = PrimitiveDateTime::new(date, time::Time::from_hms(13, 0, 0).unwrap())
        .assume_offset(STOCKHOLM);
    Some(EventSeed {
        id: format!("lpga_tour_{}_{}", season, slugify(name)),
        sport: "golf".into(),
        competition: "lpga_tour".into(),
        title: name.to_string(),
        start_time,
        end_time: Some(start_time + time::Duration::days(4)),
        status: infer_status(start_time),
        venue: if venue.is_empty() {
            None
        } else {
            Some(venue.to_string())
        },
        round_label: Some("Tournament".into()),
        participants: Participants {
            home: name.to_string(),
            away: "Field".into(),
        },
        source: "lpga-tour-schedule".into(),
        source_url: SOURCE_URL.into(),
    })
}

fn parse_date_label(input: &str) -> Option<(Month, u8)> {
    let re = Regex::new(r"(?i)(Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)[a-z]*\s+(\d{1,2})")
        .unwrap();
    let caps = re.captures(input)?;
    Some((
        parse_month(caps.get(1)?.as_str())?,
        caps.get(2)?.as_str().parse::<u8>().ok()?,
    ))
}

fn parse_month(value: &str) -> Option<Month> {
    match value.to_ascii_lowercase().as_str() {
        "jan" | "january" => Some(Month::January),
        "feb" | "february" => Some(Month::February),
        "mar" | "march" => Some(Month::March),
        "apr" | "april" => Some(Month::April),
        "may" => Some(Month::May),
        "jun" | "june" => Some(Month::June),
        "jul" | "july" => Some(Month::July),
        "aug" | "august" => Some(Month::August),
        "sep" | "september" => Some(Month::September),
        "oct" | "october" => Some(Month::October),
        "nov" | "november" => Some(Month::November),
        "dec" | "december" => Some(Month::December),
        _ => None,
    }
}
fn infer_status(start_time: OffsetDateTime) -> EventStatus {
    let now = OffsetDateTime::now_utc();
    if now < start_time {
        EventStatus::Upcoming
    } else if now <= start_time + time::Duration::days(4) {
        EventStatus::Live
    } else {
        EventStatus::Finished
    }
}
fn slugify(value: &str) -> String {
    value
        .to_lowercase()
        .replace('&', "and")
        .chars()
        .map(|ch| if ch.is_alphanumeric() { ch } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}
fn text(element: scraper::ElementRef<'_>) -> String {
    element
        .text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lpga_schedule() {
        let input = include_str!("../../tests/fixtures/lpga_schedule_2026.html");
        let events = parse_schedule_document(input, 2026);
        assert_eq!(events.len(), 2);
        assert!(events
            .iter()
            .any(|event| event.title == "JM Eagle LA Championship"));
    }

    #[test]
    fn parses_lpga_watch_overlays() {
        let input = include_str!("../../tests/fixtures/lpga_svenskgolf_weekly.md");
        let overlays = parse_svensk_golf_watch_document(input, 2026);
        assert_eq!(overlays.len(), 2);
        assert!(overlays
            .iter()
            .all(|overlay| overlay.provider_family == "viaplay"));
    }
}
