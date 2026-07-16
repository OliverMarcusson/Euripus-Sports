use scraper::{Html, Selector};
use time::{Date, Month, OffsetDateTime};

use crate::domain::{EventSeed, EventStatus, Participants};

const SOURCE_URL: &str = "https://elitserien.se/spelprogram/";

pub fn parse_schedule_document_at(
    input: &str,
    season: i32,
    observed_at: OffsetDateTime,
) -> Vec<EventSeed> {
    parse_schedule_document_for_competition_at(
        input,
        season,
        "bandy_elitserien",
        "tr.men-team",
        "elitserien-schedule",
        observed_at,
    )
}

pub fn parse_schedule_document_for_competition_at(
    input: &str,
    season: i32,
    competition: &str,
    row_selector: &str,
    source_name: &str,
    observed_at: OffsetDateTime,
) -> Vec<EventSeed> {
    let document = Html::parse_document(input);
    let row_selector = Selector::parse(row_selector).unwrap();
    let cell_selector = Selector::parse("td").unwrap();
    let link_selector = Selector::parse("a").unwrap();

    let mut events = Vec::new();

    for row in document.select(&row_selector) {
        let cells = row.select(&cell_selector).collect::<Vec<_>>();
        if cells.len() < 5 {
            continue;
        }

        let time_cell = cell_text(&cells[0]);
        let Some((date, time)) = parse_date_time(&time_cell) else {
            continue;
        };
        let home = extract_team_name(&cells[1]);
        let result = cell_text(&cells[2]);
        let away = extract_team_name(&cells[3]);
        let report_url = cells[4]
            .select(&link_selector)
            .next()
            .and_then(|link| link.value().attr("href"))
            .unwrap_or(SOURCE_URL)
            .to_string();

        if home.is_empty() || away.is_empty() {
            continue;
        }

        let Some(start_time) = parse_datetime(date, time) else {
            continue;
        };
        let status = if looks_finished_score(&result) {
            EventStatus::Finished
        } else {
            infer_status(start_time, observed_at)
        };

        events.push(EventSeed {
            id: format!(
                "{}_{}_{:02}_{:02}_{}_{}",
                competition,
                season,
                date.month() as u8,
                date.day(),
                slugify(&home),
                slugify(&away)
            ),
            sport: "bandy".into(),
            competition: competition.into(),
            title: format!("{} vs {}", home, away),
            start_time,
            end_time: Some(start_time + time::Duration::hours(2)),
            status,
            venue: None,
            round_label: None,
            participants: Participants { home, away },
            source: source_name.into(),
            source_url: report_url,
        });
    }

    events
}

fn cell_text(cell: &scraper::element_ref::ElementRef<'_>) -> String {
    cell.text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn extract_team_name(cell: &scraper::element_ref::ElementRef<'_>) -> String {
    let text = cell_text(cell);
    text.split_whitespace()
        .skip_while(|part| part.starts_with("http"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_date_time(value: &str) -> Option<(Date, &str)> {
    let (date_part, time_part) = value.split_once(' ')?;
    let mut parts = date_part.split('-');
    let year = parts.next()?.parse::<i32>().ok()?;
    let month = Month::try_from(parts.next()?.parse::<u8>().ok()?).ok()?;
    let day = parts.next()?.parse::<u8>().ok()?;
    Some((
        Date::from_calendar_date(year, month, day).ok()?,
        time_part.trim(),
    ))
}

fn parse_datetime(date: Date, value: &str) -> Option<OffsetDateTime> {
    super::parse_clock_in_timezone(date, value, time_tz::timezones::db::europe::STOCKHOLM)
}

fn looks_finished_score(value: &str) -> bool {
    let mut parts = value.split('-').map(str::trim);
    matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some(left), Some(right), None) if left.parse::<u8>().is_ok() && right.parse::<u8>().is_ok()
    )
}

fn infer_status(start_time: OffsetDateTime, observed_at: OffsetDateTime) -> EventStatus {
    crate::time_utils::infer_status_at(
        observed_at,
        start_time,
        start_time + time::Duration::hours(2),
    )
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
    fn parses_elitserien_schedule_table() {
        let input = include_str!("../../tests/fixtures/elitserien_spelprogram.html");
        let events =
            parse_schedule_document_at(input, 2026, time::macros::datetime!(2026-01-01 00:00 UTC));
        assert_eq!(events.len(), 4);
        assert!(events.iter().any(|event| {
            event.title == "Villa-Lidköping BK vs Västerås SK"
                && event.status == EventStatus::Finished
        }));
        assert!(events.iter().any(|event| {
            event.title == "Västerås SK vs Villa-Lidköping BK"
                && event.start_time.to_string().contains("2026-03-02")
        }));
    }

    #[test]
    fn parses_elitserien_dam_schedule_table() {
        let input = include_str!("../../tests/fixtures/elitserien_dam_spelprogram.html");
        let events = parse_schedule_document_for_competition_at(
            input,
            2026,
            "bandy_elitserien_dam",
            "tr.women-team",
            "elitserien-dam-schedule",
            time::macros::datetime!(2026-02-01 00:00 UTC),
        );
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0].start_time.to_offset(time::UtcOffset::UTC),
            time::macros::datetime!(2026-02-28 13:00 UTC)
        );
        assert!(events.iter().any(|event| {
            event.title == "Västerås SK vs Villa-Lidköping BK"
                && event.competition == "bandy_elitserien_dam"
        }));
    }
}
