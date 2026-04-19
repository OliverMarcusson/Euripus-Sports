use regex::Regex;
use time::{macros::offset, Date, Month, OffsetDateTime, PrimitiveDateTime, UtcOffset};

use crate::{
    config::AppConfig,
    domain::{EventSeed, EventStatus, Participants},
};

const STOCKHOLM: UtcOffset = offset!(+2);
const SOURCE_URL: &str = "https://stats.swehockey.se/";

pub fn parse_schedule_document(input: &str, season: i32, config: &AppConfig) -> Vec<EventSeed> {
    let line_re = Regex::new(r"^(?P<phase>[^|]+)\|(?P<date>\d{4}-\d{2}-\d{2})\|(?P<time>\d{1,2}:\d{2})\|(?P<home>[^|]+)\|(?P<away>[^|]+)(?:\|(?P<venue>[^|]+))?$").unwrap();

    input
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let caps = line_re.captures(line)?;
            let phase = caps.name("phase")?.as_str().trim().to_string();
            let date = parse_date(caps.name("date")?.as_str(), season)?;
            let start_time = parse_datetime(date, caps.name("time")?.as_str());
            let home = config.canonical_team_name("ndhl", caps.name("home")?.as_str().trim());
            let away = config.canonical_team_name("ndhl", caps.name("away")?.as_str().trim());
            Some(EventSeed {
                id: format!(
                    "ndhl_{}_{:02}_{:02}_{}_{}",
                    season,
                    date.month() as u8,
                    date.day(),
                    slugify(&home),
                    slugify(&away)
                ),
                sport: "hockey".into(),
                competition: "ndhl".into(),
                title: format!("{} vs {}", home, away),
                start_time,
                end_time: Some(start_time + time::Duration::minutes(150)),
                status: infer_status(start_time),
                venue: caps
                    .name("venue")
                    .map(|m| m.as_str().trim().to_string())
                    .filter(|v| !v.is_empty()),
                round_label: Some(phase),
                participants: Participants { home, away },
                source: "ndhl-schedule".into(),
                source_url: SOURCE_URL.into(),
            })
        })
        .collect()
}

fn parse_date(value: &str, season: i32) -> Option<Date> {
    let mut parts = value.split('-');
    let year = parts.next()?.parse::<i32>().ok()?;
    let month = Month::try_from(parts.next()?.parse::<u8>().ok()?).ok()?;
    let day = parts.next()?.parse::<u8>().ok()?;
    let year = if matches!(
        month,
        Month::August | Month::September | Month::October | Month::November | Month::December
    ) {
        season - 1
    } else {
        year
    };
    Date::from_calendar_date(year, month, day).ok()
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
    fn parses_ndhl_schedule() {
        let input = include_str!("../../tests/fixtures/ndhl_schedule.txt");
        let config = AppConfig::load(
            "config/providers.yaml",
            "config/competition_rules.yaml",
            "config/sample_events.yaml",
            "config/sources.yaml",
            "config/team_aliases.yaml",
        )
        .unwrap();
        let events = parse_schedule_document(input, 2026, &config);
        assert_eq!(events.len(), 3);
        assert!(events
            .iter()
            .any(|event| event.round_label.as_deref() == Some("Östra")));
        assert!(events
            .iter()
            .any(|event| event.round_label.as_deref() == Some("Kval")));
    }
}
