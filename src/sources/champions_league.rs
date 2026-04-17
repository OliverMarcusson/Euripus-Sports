use regex::Regex;
use serde_json::Value;
use time::{
    format_description::well_known::Rfc3339, macros::offset, Date, Month, OffsetDateTime,
    PrimitiveDateTime, UtcOffset,
};

use crate::domain::{EventSeed, EventStatus, Participants};

const UK_SUMMER: UtcOffset = offset!(+1);
const STOCKHOLM: UtcOffset = offset!(+2);

pub fn parse_bbc_fixtures(input: &str, season: i32) -> Vec<EventSeed> {
    if input.contains("window.__INITIAL_DATA__=") {
        return parse_bbc_html(input, season);
    }
    let day_header_re = Regex::new(r"^##\s+(?:Monday|Tuesday|Wednesday|Thursday|Friday|Saturday|Sunday)\s+(\d{1,2})(?:st|nd|rd|th)\s+([A-Za-z]+)$").unwrap();
    let stage_re = Regex::new(r"^###\s+(.+)$").unwrap();
    let fixture_re = Regex::new(
        r"^\*\s+(?P<home>.+?)\s+versus\s+(?P<away>.+?)\s+kick off\s+(?P<time>\d{1,2}:\d{2})",
    )
    .unwrap();

    let mut current_date = None;
    let mut current_stage: Option<String> = None;
    let mut events = Vec::new();

    for line in input.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if let Some(caps) = day_header_re.captures(line) {
            let day = caps.get(1).and_then(|m| m.as_str().parse::<u8>().ok());
            let month = caps.get(2).and_then(|m| parse_month(m.as_str()));
            current_date = day.and_then(|day| {
                month.and_then(|month| Date::from_calendar_date(season, month, day).ok())
            });
            continue;
        }
        if let Some(caps) = stage_re.captures(line) {
            current_stage = caps.get(1).map(|m| m.as_str().trim().to_string());
            continue;
        }

        let Some(caps) = fixture_re.captures(line) else {
            continue;
        };
        let Some(date) = current_date else { continue };
        let home = caps.name("home").unwrap().as_str().trim();
        let away = caps.name("away").unwrap().as_str().trim();
        let time = caps.name("time").unwrap().as_str();
        let start_time = parse_datetime(date, time);

        events.push(EventSeed {
            id: format!(
                "uefa_champions_league_{}_{}_{}",
                season,
                slugify(home),
                slugify(away)
            ),
            sport: "soccer".into(),
            competition: "uefa_champions_league".into(),
            title: format!("{} vs {}", home, away),
            start_time,
            end_time: Some(start_time + time::Duration::hours(2)),
            status: infer_status(start_time),
            venue: None,
            round_label: current_stage.clone(),
            participants: Participants {
                home: home.into(),
                away: away.into(),
            },
            source: "bbc-champions-league-fixture".into(),
            source_url: "https://www.bbc.com/sport/football/champions-league/scores-fixtures"
                .into(),
        });
    }

    events
}

fn parse_bbc_html(input: &str, season: i32) -> Vec<EventSeed> {
    let initial_re = Regex::new(r#"window\.__INITIAL_DATA__=(".*?");</script>"#).unwrap();
    let Some(caps) = initial_re.captures(input) else {
        return Vec::new();
    };
    let Some(raw) = caps.get(1).map(|m| m.as_str()) else {
        return Vec::new();
    };
    let Ok(decoded) = serde_json::from_str::<String>(raw) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&decoded) else {
        return Vec::new();
    };
    let Some(data) = value.get("data").and_then(Value::as_object) else {
        return Vec::new();
    };
    let Some(fixtures) = data
        .iter()
        .find(|(key, _)| key.contains("sport-data-scores-fixtures"))
        .and_then(|(_, value)| value.get("data"))
        .and_then(|value| value.get("eventGroups"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };

    fixtures
        .iter()
        .flat_map(|group| {
            group
                .get("secondaryGroups")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .flat_map(|group| {
            let stage = group
                .get("displayLabel")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string);
            group
                .get("events")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(move |event| (stage.clone(), event))
                .collect::<Vec<_>>()
        })
        .filter_map(|(stage, event)| {
            let home = event
                .get("home")
                .and_then(|team| team.get("fullName"))
                .and_then(Value::as_str)?
                .trim()
                .to_string();
            let away = event
                .get("away")
                .and_then(|team| team.get("fullName"))
                .and_then(Value::as_str)?
                .trim()
                .to_string();
            let start_raw = event.get("startDateTime").and_then(Value::as_str)?;
            let start_time = OffsetDateTime::parse(start_raw, &Rfc3339)
                .ok()?
                .to_offset(STOCKHOLM);
            Some(EventSeed {
                id: format!(
                    "uefa_champions_league_{}_{}_{}",
                    season,
                    slugify(&home),
                    slugify(&away)
                ),
                sport: "soccer".into(),
                competition: "uefa_champions_league".into(),
                title: format!("{} vs {}", home, away),
                start_time,
                end_time: Some(start_time + time::Duration::hours(2)),
                status: infer_status(start_time),
                venue: None,
                round_label: stage,
                participants: Participants { home, away },
                source: "bbc-champions-league-fixture".into(),
                source_url: "https://www.bbc.com/sport/football/champions-league/scores-fixtures"
                    .into(),
            })
        })
        .collect()
}

fn parse_month(value: &str) -> Option<Month> {
    match value.to_ascii_lowercase().as_str() {
        "january" => Some(Month::January),
        "february" => Some(Month::February),
        "march" => Some(Month::March),
        "april" => Some(Month::April),
        "may" => Some(Month::May),
        "june" => Some(Month::June),
        "july" => Some(Month::July),
        "august" => Some(Month::August),
        "september" => Some(Month::September),
        "october" => Some(Month::October),
        "november" => Some(Month::November),
        "december" => Some(Month::December),
        _ => None,
    }
}

fn parse_datetime(date: Date, value: &str) -> OffsetDateTime {
    let (hour, minute) = value.split_once(':').unwrap();
    PrimitiveDateTime::new(
        date,
        time::Time::from_hms(hour.parse().unwrap(), minute.parse().unwrap(), 0).unwrap(),
    )
    .assume_offset(UK_SUMMER)
    .to_offset(STOCKHOLM)
}

fn slugify(value: &str) -> String {
    value
        .to_lowercase()
        .replace("saint-germain", "saint_germain")
        .chars()
        .map(|ch| if ch.is_alphanumeric() { ch } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

fn infer_status(start_time: OffsetDateTime) -> EventStatus {
    let now = OffsetDateTime::now_utc();
    if now < start_time {
        EventStatus::Upcoming
    } else if now <= start_time + time::Duration::hours(2) {
        EventStatus::Live
    } else {
        EventStatus::Finished
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bbc_champions_league_fixtures() {
        let input = include_str!("../../tests/fixtures/champions_league_bbc_fixtures.md");
        let events = parse_bbc_fixtures(input, 2026);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].round_label.as_deref(), Some("Semi-finals"));
    }
}
