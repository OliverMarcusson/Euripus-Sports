use regex::Regex;
use serde_json::Value;
use time::{
    format_description::well_known::Rfc3339, macros::offset, Date, Month, OffsetDateTime,
    PrimitiveDateTime, UtcOffset,
};

use crate::{
    config::AppConfig,
    domain::{EventSeed, EventStatus, Participants},
};

const STOCKHOLM: UtcOffset = offset!(+2);
const SOURCE_URL: &str = "https://www.hockeyallsvenskan.se/game-schedule";

pub fn parse_schedule_document(input: &str, season: i32, config: &AppConfig) -> Vec<EventSeed> {
    if input.trim_start().starts_with('{') {
        return parse_api_schedule_document(input, season, config);
    }
    let image_re = Regex::new(r"!\[[^\]]*\]\([^)]*\)").unwrap();
    let markdown_link_re = Regex::new(r"\[(?P<text>[^\]]+)\]\([^)]*\)").unwrap();
    let date_re =
        Regex::new(r"^(mån|tis|ons|tors|fre|lör|sön)\s+(\d{1,2})\s+([a-zåäö]+)\.?$").unwrap();
    let live_re = Regex::new(r"^HA\s+live\s+(?P<round>[A-Z0-9]+)\s+(?P<time>\d{1,2}:\d{2})\s+(?P<home>[A-ZÅÄÖ]{2,4})\s+\d+\s*-\s*\d+\s+(?P<away>[A-ZÅÄÖ]{2,4})$").unwrap();
    let upcoming_re = Regex::new(r"^HA\s+(?:(?P<round>[A-Z0-9]+)\s+)?(?P<home>[A-ZÅÄÖ]{2,4})\s+(?P<time>\d{1,2}:\d{2})\s+(?P<away>[A-ZÅÄÖ]{2,4})$").unwrap();

    let mut current_date = None;
    let mut events = Vec::new();

    for raw_line in input.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let line = clean_line(raw_line, &image_re, &markdown_link_re);
        if line.is_empty() {
            continue;
        }

        if let Some(caps) = date_re.captures(&line) {
            let day = caps.get(2).and_then(|m| m.as_str().parse::<u8>().ok());
            let month = caps.get(3).and_then(|m| parse_month(m.as_str()));
            current_date = match (day, month) {
                (Some(day), Some(month)) => {
                    Date::from_calendar_date(season_year(season, month), month, day).ok()
                }
                _ => None,
            };
            continue;
        }

        let Some(date) = current_date else { continue };

        if let Some(caps) = live_re.captures(&line) {
            let home = config
                .canonical_team_name("hockeyallsvenskan", caps.name("home").unwrap().as_str());
            let away = config
                .canonical_team_name("hockeyallsvenskan", caps.name("away").unwrap().as_str());
            let start_time = parse_datetime(date, caps.name("time").unwrap().as_str());
            events.push(build_event(
                home,
                away,
                start_time,
                Some(caps.name("round").unwrap().as_str().to_string()),
                EventStatus::Live,
                season,
                date,
            ));
            continue;
        }

        let Some(caps) = upcoming_re.captures(&line) else {
            continue;
        };
        let home =
            config.canonical_team_name("hockeyallsvenskan", caps.name("home").unwrap().as_str());
        let away =
            config.canonical_team_name("hockeyallsvenskan", caps.name("away").unwrap().as_str());
        let start_time = parse_datetime(date, caps.name("time").unwrap().as_str());
        events.push(build_event(
            home,
            away,
            start_time,
            caps.name("round").map(|m| m.as_str().to_string()),
            infer_status(start_time),
            season,
            date,
        ));
    }

    events
}

fn parse_api_schedule_document(input: &str, season: i32, config: &AppConfig) -> Vec<EventSeed> {
    let value: Value = match serde_json::from_str(input) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };
    value
        .get("gameInfo")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|game| {
            let home_raw = game
                .get("homeTeamInfo")?
                .get("names")?
                .get("long")?
                .as_str()?
                .trim();
            let away_raw = game
                .get("awayTeamInfo")?
                .get("names")?
                .get("long")?
                .as_str()?
                .trim();
            let home = config.canonical_team_name("hockeyallsvenskan", home_raw);
            let away = config.canonical_team_name("hockeyallsvenskan", away_raw);
            let start_raw = game.get("rawStartDateTime").and_then(Value::as_str)?;
            let start_time = OffsetDateTime::parse(start_raw, &Rfc3339)
                .ok()?
                .to_offset(STOCKHOLM);
            let date = start_time.date();
            Some(EventSeed {
                id: format!(
                    "hockeyallsvenskan_{}_{:02}_{:02}_{}_{}",
                    season,
                    date.month() as u8,
                    date.day(),
                    slugify(&home),
                    slugify(&away)
                ),
                sport: "hockey".into(),
                competition: "hockeyallsvenskan".into(),
                title: format!("{} vs {}", home, away),
                start_time,
                end_time: Some(start_time + time::Duration::minutes(150)),
                status: match game.get("state").and_then(Value::as_str) {
                    Some("pre-game") => EventStatus::Upcoming,
                    Some("in-game") => EventStatus::Live,
                    Some("post-game") => EventStatus::Finished,
                    _ => infer_status(start_time),
                },
                venue: game
                    .get("venueInfo")
                    .and_then(|value| value.get("name"))
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                round_label: game
                    .get("seriesInfo")
                    .and_then(|value| value.get("name"))
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                participants: Participants { home, away },
                source: "hockeyallsvenskan-api".into(),
                source_url: SOURCE_URL.into(),
            })
        })
        .collect()
}

fn build_event(
    home: String,
    away: String,
    start_time: OffsetDateTime,
    round_label: Option<String>,
    status: EventStatus,
    season: i32,
    date: Date,
) -> EventSeed {
    EventSeed {
        id: format!(
            "hockeyallsvenskan_{}_{:02}_{:02}_{}_{}",
            season,
            date.month() as u8,
            date.day(),
            slugify(&home),
            slugify(&away)
        ),
        sport: "hockey".into(),
        competition: "hockeyallsvenskan".into(),
        title: format!("{} vs {}", home, away),
        start_time,
        end_time: Some(start_time + time::Duration::minutes(150)),
        status,
        venue: None,
        round_label,
        participants: Participants { home, away },
        source: "hockeyallsvenskan-schedule".into(),
        source_url: SOURCE_URL.into(),
    }
}

fn clean_line(input: &str, image_re: &Regex, markdown_link_re: &Regex) -> String {
    let without_images = image_re.replace_all(input, " ");
    let extracted_links = markdown_link_re
        .captures_iter(&without_images)
        .filter_map(|caps| caps.name("text").map(|m| m.as_str().trim().to_string()))
        .collect::<Vec<_>>();

    let base = if let Some(first_link) = extracted_links.first() {
        first_link.clone()
    } else {
        without_images.to_string()
    };

    base.trim_start_matches('*')
        .trim()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_month(value: &str) -> Option<Month> {
    match value.to_ascii_lowercase().trim_end_matches('.') {
        "jan" => Some(Month::January),
        "feb" => Some(Month::February),
        "mar" => Some(Month::March),
        "apr" => Some(Month::April),
        "maj" => Some(Month::May),
        "jun" => Some(Month::June),
        "jul" => Some(Month::July),
        "aug" => Some(Month::August),
        "sep" => Some(Month::September),
        "okt" => Some(Month::October),
        "nov" => Some(Month::November),
        "dec" => Some(Month::December),
        _ => None,
    }
}

fn season_year(season: i32, month: Month) -> i32 {
    match month {
        Month::August | Month::September | Month::October | Month::November | Month::December => {
            season - 1
        }
        _ => season,
    }
}

fn parse_datetime(date: Date, value: &str) -> OffsetDateTime {
    let (hour, minute) = value.split_once(':').unwrap();
    PrimitiveDateTime::new(
        date,
        time::Time::from_hms(hour.parse().unwrap(), minute.parse().unwrap(), 0).unwrap(),
    )
    .assume_offset(STOCKHOLM)
}

fn infer_status(start_time: OffsetDateTime) -> EventStatus {
    let now = OffsetDateTime::now_utc();
    if now < start_time {
        EventStatus::Upcoming
    } else if now <= start_time + time::Duration::minutes(150) {
        EventStatus::Live
    } else {
        EventStatus::Finished
    }
}

fn slugify(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .map(|ch| if ch.is_alphanumeric() { ch } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hockeyallsvenskan_schedule() {
        let input = include_str!("../../tests/fixtures/hockeyallsvenskan_game_schedule.md");
        let config = AppConfig::load(
            "config/providers.yaml",
            "config/competition_rules.yaml",
            "config/sample_events.yaml",
            "config/sources.yaml",
            "config/team_aliases.yaml",
        )
        .unwrap();
        let events = parse_schedule_document(input, 2026, &config);
        assert_eq!(events.len(), 5);
        assert!(events.iter().any(|event| {
            event.title == "BIK Karlskoga vs Björklöven" && event.status == EventStatus::Upcoming
        }));
        assert!(events.iter().any(|event| {
            event.title == "Björklöven vs BIK Karlskoga"
                && event.status == EventStatus::Live
                && event.round_label.as_deref() == Some("P3")
        }));
    }
}
