use regex::Regex;
use scraper::{Html, Selector};
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
const SOURCE_URL: &str = "https://www.sdhl.se/game-schedule";

pub fn parse_schedule_document(input: &str, season: i32, config: &AppConfig) -> Vec<EventSeed> {
    if input.trim_start().starts_with('{') {
        return parse_api_schedule_document(input, season, config);
    }
    if input.contains("<html") || input.contains("<!DOCTYPE html") {
        return parse_html_schedule_document(input, season, config);
    }

    let date_re = Regex::new(
        r"^(MÅNDAG|TISDAG|ONSDAG|TORSDAG|FREDAG|LÖRDAG|SÖNDAG)\s+(\d{1,2})\s+([A-ZÅÄÖ]+)$",
    )
    .unwrap();
    let fixture_re = Regex::new(r"^(?P<home>.+?)\s+[–-]\s+(?P<away>.+)$").unwrap();
    let time_re = Regex::new(r"^\d{1,2}:\d{2}$").unwrap();
    let result_re = Regex::new(r"^\d+\s*-\s*\d+$").unwrap();

    let lines = input
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>();
    let mut current_date = None;
    let mut events = Vec::new();
    let mut index = 0;

    while index < lines.len() {
        let line = lines[index];
        if let Some(caps) = date_re.captures(line) {
            let day = caps.get(2).and_then(|m| m.as_str().parse::<u8>().ok());
            let month = caps.get(3).and_then(|m| parse_month(m.as_str()));
            current_date = match (day, month) {
                (Some(day), Some(month)) => {
                    Date::from_calendar_date(season_year(season, month), month, day).ok()
                }
                _ => None,
            };
            index += 1;
            continue;
        }

        let Some(date) = current_date else {
            index += 1;
            continue;
        };
        let Some(caps) = fixture_re.captures(line) else {
            index += 1;
            continue;
        };
        if index + 5 >= lines.len() {
            break;
        }

        let home_raw = caps.name("home").unwrap().as_str().trim();
        let away_raw = caps.name("away").unwrap().as_str().trim();
        let home = config.canonical_team_name("sdhl", home_raw);
        let away = config.canonical_team_name("sdhl", away_raw);
        let repeated_home = lines[index + 1];
        let timing_or_result = lines[index + 2];
        let repeated_away = lines[index + 3];
        let venue = lines[index + 4];
        let status_line = lines[index + 5];

        if repeated_home != home_raw
            || repeated_away != away_raw
            || (!time_re.is_match(timing_or_result) && !result_re.is_match(timing_or_result))
        {
            index += 1;
            continue;
        }

        let (start_time, status) = if time_re.is_match(timing_or_result) {
            let start_time = parse_datetime(date, timing_or_result);
            let status = match status_line {
                "Efter match" => EventStatus::Finished,
                "Live" | "Pågår" => EventStatus::Live,
                _ => infer_status(start_time),
            };
            (start_time, status)
        } else {
            (parse_datetime(date, "19:00"), EventStatus::Finished)
        };

        events.push(build_event(
            home,
            away,
            start_time,
            Some(venue.to_string()),
            None,
            status,
            season,
        ));
        index += 6;
    }

    events
}

fn parse_html_schedule_document(input: &str, season: i32, config: &AppConfig) -> Vec<EventSeed> {
    let document = Html::parse_document(input);
    let section_selector = Selector::parse("section.list").unwrap();
    let date_selector = Selector::parse("h2").unwrap();
    let row_selector = Selector::parse("li.game-schedule-row article").unwrap();
    let label_selector = Selector::parse("h3").unwrap();
    let venue_selector = Selector::parse(".arena-container span").unwrap();
    let time_result_selector = Selector::parse(".time-result").unwrap();
    let action_selector =
        Selector::parse(".action-button a, .action-button button, .action-button").unwrap();

    let mut events = Vec::new();
    for section in document.select(&section_selector) {
        let Some(date_label) = section.select(&date_selector).next().map(text_content) else {
            continue;
        };
        let Some(date) = parse_swedish_date_label(&date_label, season) else {
            continue;
        };

        for row in section.select(&row_selector) {
            let Some(label) = row.select(&label_selector).next().map(text_content) else {
                continue;
            };
            let Some((home_raw, away_raw)) = split_matchup(&label) else {
                continue;
            };
            let home = config.canonical_team_name("sdhl", &home_raw);
            let away = config.canonical_team_name("sdhl", &away_raw);
            let venue = row
                .select(&venue_selector)
                .next()
                .map(text_content)
                .filter(|value| !value.is_empty());
            let time_or_result = row
                .select(&time_result_selector)
                .next()
                .map(text_content)
                .unwrap_or_default();
            let action = row
                .select(&action_selector)
                .next()
                .map(text_content)
                .unwrap_or_default();

            let (start_time, status) =
                if looks_finished_score(&time_or_result) || action.contains("Efter match") {
                    (parse_datetime(date, "19:00"), EventStatus::Finished)
                } else if looks_like_time(&time_or_result) {
                    let start_time = parse_datetime(date, &time_or_result);
                    let status = if action.contains("Live") || action.contains("Pågår") {
                        EventStatus::Live
                    } else {
                        infer_status(start_time)
                    };
                    (start_time, status)
                } else {
                    let start_time = parse_datetime(date, "19:00");
                    (start_time, infer_status(start_time))
                };

            events.push(build_event(
                home, away, start_time, venue, None, status, season,
            ));
        }
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
            let home = config.canonical_team_name("sdhl", home_raw);
            let away = config.canonical_team_name("sdhl", away_raw);
            let start_raw = game.get("rawStartDateTime").and_then(Value::as_str)?;
            let start_time = OffsetDateTime::parse(start_raw, &Rfc3339)
                .ok()?
                .to_offset(STOCKHOLM);
            Some(build_event(
                home,
                away,
                start_time,
                game.get("venueInfo")
                    .and_then(|v| v.get("name"))
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                game.get("seriesInfo")
                    .and_then(|v| v.get("name"))
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                match game.get("state").and_then(Value::as_str) {
                    Some("pre-game") => EventStatus::Upcoming,
                    Some("in-game") => EventStatus::Live,
                    Some("post-game") => EventStatus::Finished,
                    _ => infer_status(start_time),
                },
                season,
            ))
        })
        .collect()
}

fn build_event(
    home: String,
    away: String,
    start_time: OffsetDateTime,
    venue: Option<String>,
    round_label: Option<String>,
    status: EventStatus,
    season: i32,
) -> EventSeed {
    let date = start_time.date();
    EventSeed {
        id: format!(
            "sdhl_{}_{:02}_{:02}_{}_{}",
            season,
            date.month() as u8,
            date.day(),
            slugify(&home),
            slugify(&away)
        ),
        sport: "hockey".into(),
        competition: "sdhl".into(),
        title: format!("{} vs {}", home, away),
        start_time,
        end_time: Some(start_time + time::Duration::minutes(150)),
        status,
        venue,
        round_label,
        participants: Participants { home, away },
        source: "sdhl-schedule".into(),
        source_url: SOURCE_URL.into(),
    }
}

fn parse_month(value: &str) -> Option<Month> {
    match value.to_ascii_uppercase().as_str() {
        "JANUARI" => Some(Month::January),
        "FEBRUARI" => Some(Month::February),
        "MARS" => Some(Month::March),
        "APRIL" => Some(Month::April),
        "MAJ" => Some(Month::May),
        "JUNI" => Some(Month::June),
        "JULI" => Some(Month::July),
        "AUGUSTI" => Some(Month::August),
        "SEPTEMBER" => Some(Month::September),
        "OKTOBER" => Some(Month::October),
        "NOVEMBER" => Some(Month::November),
        "DECEMBER" => Some(Month::December),
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

fn text_content(element: scraper::ElementRef<'_>) -> String {
    element
        .text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn split_matchup(value: &str) -> Option<(String, String)> {
    let (home, away) = value.split_once('–').or_else(|| value.split_once('-'))?;
    Some((home.trim().to_string(), away.trim().to_string()))
}

fn looks_like_time(value: &str) -> bool {
    Regex::new(r"^\d{1,2}:\d{2}$")
        .unwrap()
        .is_match(value.trim())
}

fn looks_finished_score(value: &str) -> bool {
    let mut parts = value.split('-').map(str::trim);
    matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some(left), Some(right), None) if left.parse::<u8>().is_ok() && right.parse::<u8>().is_ok()
    )
}

fn parse_swedish_date_label(value: &str, season: i32) -> Option<Date> {
    let re = Regex::new(r"(?i)^[a-zåäö]+\s+(?P<day>\d{1,2})\s+(?P<month>[a-zåäö]+)$").unwrap();
    let caps = re.captures(value.trim())?;
    let day = caps.name("day")?.as_str().parse::<u8>().ok()?;
    let month = parse_month(&caps.name("month")?.as_str().to_ascii_uppercase())?;
    Date::from_calendar_date(season_year(season, month), month, day).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sdhl_schedule() {
        let input = include_str!("../../tests/fixtures/sdhl_game_schedule.json");
        let config = AppConfig::load(
            "config/providers.yaml",
            "config/competition_rules.yaml",
            "config/sample_events.yaml",
            "config/sources.yaml",
            "config/team_aliases.yaml",
        )
        .unwrap();
        let events = parse_schedule_document(input, 2026, &config);
        assert_eq!(events.len(), 2);
        assert!(events
            .iter()
            .any(|event| event.title == "Luleå Hockey/MSSK vs Frölunda HC"
                && event.round_label.as_deref() == Some("Grundserie")));
    }
}
