use regex::Regex;
use serde_json::Value;
use time::{
    format_description::well_known::Rfc3339, macros::offset, Date, Month, OffsetDateTime,
    PrimitiveDateTime, UtcOffset,
};

use crate::domain::{EventSeed, EventStatus, Participants};

const STOCKHOLM: UtcOffset = offset!(+2);
const SOURCE_URL: &str = "https://www.shl.se/game-schedule";

pub fn parse_schedule_document(input: &str, season: i32) -> Vec<EventSeed> {
    if input.trim_start().starts_with('{') {
        return parse_api_schedule_document(input, season);
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
        .filter(|line| !line.is_empty())
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

        let home = caps.name("home").unwrap().as_str().trim();
        let away = caps.name("away").unwrap().as_str().trim();
        if index + 5 >= lines.len() {
            break;
        }

        let repeated_home = lines[index + 1];
        let timing_or_result = lines[index + 2];
        let repeated_away = lines[index + 3];
        let venue = lines[index + 4];
        let status_line = lines[index + 5];

        if repeated_home != home
            || repeated_away != away
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
            let start_time = parse_datetime(date, "19:00");
            (start_time, EventStatus::Finished)
        };

        events.push(EventSeed {
            id: format!(
                "shl_{}_{:02}_{:02}_{}_{}",
                season,
                date.month() as u8,
                date.day(),
                slugify(home),
                slugify(away)
            ),
            sport: "hockey".into(),
            competition: "shl".into(),
            title: format!("{} vs {}", home, away),
            start_time,
            end_time: Some(start_time + time::Duration::minutes(150)),
            status,
            venue: Some(venue.to_string()),
            round_label: None,
            participants: Participants {
                home: home.into(),
                away: away.into(),
            },
            source: "shl-schedule".into(),
            source_url: SOURCE_URL.into(),
        });

        index += 6;
    }

    events
}

fn parse_api_schedule_document(input: &str, season: i32) -> Vec<EventSeed> {
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
            let home = game
                .get("homeTeamInfo")?
                .get("names")?
                .get("long")?
                .as_str()?
                .trim()
                .to_string();
            let away = game
                .get("awayTeamInfo")?
                .get("names")?
                .get("long")?
                .as_str()?
                .trim()
                .to_string();
            let start_raw = game.get("rawStartDateTime").and_then(Value::as_str)?;
            let start_time = OffsetDateTime::parse(start_raw, &Rfc3339)
                .ok()?
                .to_offset(STOCKHOLM);
            let date = start_time.date();
            Some(EventSeed {
                id: format!(
                    "shl_{}_{:02}_{:02}_{}_{}",
                    season,
                    date.month() as u8,
                    date.day(),
                    slugify(&home),
                    slugify(&away)
                ),
                sport: "hockey".into(),
                competition: "shl".into(),
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
                source: "shl-api".into(),
                source_url: SOURCE_URL.into(),
            })
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_shl_schedule() {
        let input = include_str!("../../tests/fixtures/shl_game_schedule.md");
        let events = parse_schedule_document(input, 2026);
        assert_eq!(events.len(), 4);
        assert!(events.iter().any(|event| {
            event.title == "Skellefteå AIK vs Rögle BK"
                && event.venue.as_deref() == Some("Skellefteå Kraft Arena")
                && event.status == EventStatus::Upcoming
        }));
    }
}
