use scraper::{Html, Selector};
use time::{Date, Month, OffsetDateTime};
use time_tz::Tz;

use crate::domain::{EventSeed, EventStatus, Participants};

const SOURCE_URL: &str = "https://www.formula1.com/en/latest/article/official-grand-prix-start-times-for-2026-f1-season-confirmed.2UgPfArqH76tzlOYh21jSG";

pub fn parse_race_times_document_at(
    input: &str,
    season: i32,
    observed_at: OffsetDateTime,
) -> Vec<EventSeed> {
    let document = Html::parse_document(input);
    let table_selector = Selector::parse("table").unwrap();
    let header_selector = Selector::parse("thead th").unwrap();
    let row_selector = Selector::parse("tbody tr").unwrap();
    let cell_selector = Selector::parse("td").unwrap();

    let mut events = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();

    for table in document.select(&table_selector) {
        let headers = table
            .select(&header_selector)
            .map(text_content)
            .collect::<Vec<_>>();
        if headers.len() < 4
            || headers.first().map(String::as_str) != Some("Venue, race date")
            || !headers.iter().any(|header| header == "Race (local time)")
        {
            continue;
        }

        for row in table.select(&row_selector) {
            let cells = row
                .select(&cell_selector)
                .map(text_content)
                .collect::<Vec<_>>();
            if cells.len() < 4 {
                continue;
            }

            let Some((venue_key, date)) = parse_venue_date(&cells[0], season) else {
                continue;
            };
            let Some(metadata) = metadata_for_venue(venue_key) else {
                continue;
            };
            let Some(start_time) = parse_local_start(date, &cells[3], metadata.timezone) else {
                continue;
            };

            let id = format!(
                "formula_1_{}_{:02}_{:02}_{}",
                season,
                date.month() as u8,
                date.day(),
                metadata.slug
            );
            if !seen_ids.insert(id.clone()) {
                continue;
            }

            let round = events.len() + 1;
            let title = format!("{} Grand Prix", metadata.title_prefix);
            events.push(EventSeed {
                id,
                sport: "motorsport".into(),
                competition: "formula_1".into(),
                title: title.clone(),
                start_time,
                end_time: Some(start_time + time::Duration::hours(3)),
                status: infer_status(start_time, observed_at),
                venue: Some(metadata.venue.to_string()),
                round_label: Some(format!("Round {}", round)),
                participants: Participants {
                    home: title,
                    away: "Field".into(),
                },
                source: "formula1-race-times".into(),
                source_url: SOURCE_URL.into(),
            });
        }
    }

    events.sort_by_key(|event| event.start_time);
    events
}

#[derive(Clone, Copy)]
struct Formula1RaceMetadata {
    slug: &'static str,
    title_prefix: &'static str,
    venue: &'static str,
    timezone: &'static Tz,
}

fn metadata_for_venue(venue: &str) -> Option<Formula1RaceMetadata> {
    match venue {
        "Australia" => Some(Formula1RaceMetadata {
            slug: "australia",
            title_prefix: "Australian",
            venue: "Melbourne",
            timezone: time_tz::timezones::db::australia::MELBOURNE,
        }),
        "China" => Some(Formula1RaceMetadata {
            slug: "china",
            title_prefix: "Chinese",
            venue: "Shanghai",
            timezone: time_tz::timezones::db::asia::SHANGHAI,
        }),
        "Japan" => Some(Formula1RaceMetadata {
            slug: "japan",
            title_prefix: "Japanese",
            venue: "Suzuka",
            timezone: time_tz::timezones::db::asia::TOKYO,
        }),
        "Bahrain" => Some(Formula1RaceMetadata {
            slug: "bahrain",
            title_prefix: "Bahrain",
            venue: "Bahrain International Circuit",
            timezone: time_tz::timezones::db::asia::BAHRAIN,
        }),
        "Saudi Arabia" => Some(Formula1RaceMetadata {
            slug: "saudi_arabia",
            title_prefix: "Saudi Arabian",
            venue: "Jeddah Corniche Circuit",
            timezone: time_tz::timezones::db::asia::RIYADH,
        }),
        "Miami" => Some(Formula1RaceMetadata {
            slug: "miami",
            title_prefix: "Miami",
            venue: "Miami International Autodrome",
            timezone: time_tz::timezones::db::america::NEW_YORK,
        }),
        "Canada" => Some(Formula1RaceMetadata {
            slug: "canada",
            title_prefix: "Canadian",
            venue: "Circuit Gilles Villeneuve",
            timezone: time_tz::timezones::db::america::TORONTO,
        }),
        "Monaco" => Some(Formula1RaceMetadata {
            slug: "monaco",
            title_prefix: "Monaco",
            venue: "Circuit de Monaco",
            timezone: time_tz::timezones::db::europe::PARIS,
        }),
        "Barcelona" | "Barcelona-Catalunya" => Some(Formula1RaceMetadata {
            slug: "barcelona",
            title_prefix: "Spanish",
            venue: "Circuit de Barcelona-Catalunya",
            timezone: time_tz::timezones::db::europe::MADRID,
        }),
        "Austria" => Some(Formula1RaceMetadata {
            slug: "austria",
            title_prefix: "Austrian",
            venue: "Red Bull Ring",
            timezone: time_tz::timezones::db::europe::VIENNA,
        }),
        "Great Britain" => Some(Formula1RaceMetadata {
            slug: "great_britain",
            title_prefix: "British",
            venue: "Silverstone Circuit",
            timezone: time_tz::timezones::db::europe::LONDON,
        }),
        "Belgium" => Some(Formula1RaceMetadata {
            slug: "belgium",
            title_prefix: "Belgian",
            venue: "Circuit de Spa-Francorchamps",
            timezone: time_tz::timezones::db::europe::BRUSSELS,
        }),
        "Hungary" => Some(Formula1RaceMetadata {
            slug: "hungary",
            title_prefix: "Hungarian",
            venue: "Hungaroring",
            timezone: time_tz::timezones::db::europe::BUDAPEST,
        }),
        "Netherlands" => Some(Formula1RaceMetadata {
            slug: "netherlands",
            title_prefix: "Dutch",
            venue: "Circuit Zandvoort",
            timezone: time_tz::timezones::db::europe::AMSTERDAM,
        }),
        "Italy" => Some(Formula1RaceMetadata {
            slug: "italy",
            title_prefix: "Italian",
            venue: "Autodromo Nazionale Monza",
            timezone: time_tz::timezones::db::europe::ROME,
        }),
        "Spain" => Some(Formula1RaceMetadata {
            slug: "spain",
            title_prefix: "Spanish",
            venue: "Madring",
            timezone: time_tz::timezones::db::europe::MADRID,
        }),
        "Azerbaijan" => Some(Formula1RaceMetadata {
            slug: "azerbaijan",
            title_prefix: "Azerbaijan",
            venue: "Baku City Circuit",
            timezone: time_tz::timezones::db::asia::BAKU,
        }),
        "Singapore" => Some(Formula1RaceMetadata {
            slug: "singapore",
            title_prefix: "Singapore",
            venue: "Marina Bay Street Circuit",
            timezone: time_tz::timezones::db::SINGAPORE,
        }),
        "United States" => Some(Formula1RaceMetadata {
            slug: "united_states",
            title_prefix: "United States",
            venue: "Circuit of The Americas",
            timezone: time_tz::timezones::db::america::CHICAGO,
        }),
        "Mexico" => Some(Formula1RaceMetadata {
            slug: "mexico",
            title_prefix: "Mexico City",
            venue: "Autódromo Hermanos Rodríguez",
            timezone: time_tz::timezones::db::america::MEXICO_CITY,
        }),
        "Brazil" => Some(Formula1RaceMetadata {
            slug: "brazil",
            title_prefix: "São Paulo",
            venue: "Interlagos",
            timezone: time_tz::timezones::db::america::SAO_PAULO,
        }),
        "Las Vegas" => Some(Formula1RaceMetadata {
            slug: "las_vegas",
            title_prefix: "Las Vegas",
            venue: "Las Vegas Strip Circuit",
            timezone: time_tz::timezones::db::america::LOS_ANGELES,
        }),
        "Qatar" => Some(Formula1RaceMetadata {
            slug: "qatar",
            title_prefix: "Qatar",
            venue: "Lusail International Circuit",
            timezone: time_tz::timezones::db::asia::QATAR,
        }),
        "Abu Dhabi" => Some(Formula1RaceMetadata {
            slug: "abu_dhabi",
            title_prefix: "Abu Dhabi",
            venue: "Yas Marina Circuit",
            timezone: time_tz::timezones::db::asia::DUBAI,
        }),
        _ => None,
    }
}

fn parse_venue_date(input: &str, season: i32) -> Option<(&str, Date)> {
    let (venue, raw_date) = input.split_once(',')?;
    let mut parts = raw_date.split_whitespace();
    let month = parse_month(parts.next()?)?;
    let day = parts.next()?.parse::<u8>().ok()?;
    let date = Date::from_calendar_date(season, month, day).ok()?;
    Some((venue.trim(), date))
}

fn parse_month(value: &str) -> Option<Month> {
    match value {
        "Jan" => Some(Month::January),
        "Feb" => Some(Month::February),
        "Mar" => Some(Month::March),
        "Apr" => Some(Month::April),
        "May" => Some(Month::May),
        "Jun" => Some(Month::June),
        "Jul" => Some(Month::July),
        "Aug" => Some(Month::August),
        "Sep" => Some(Month::September),
        "Oct" => Some(Month::October),
        "Nov" => Some(Month::November),
        "Dec" => Some(Month::December),
        _ => None,
    }
}

fn parse_local_start(date: Date, value: &str, timezone: &'static Tz) -> Option<OffsetDateTime> {
    let value = value.trim();
    if value.len() != 4 || value == "-" {
        return None;
    }
    crate::time_utils::local_datetime(date, &format!("{}:{}", &value[..2], &value[2..]), timezone)
}

fn infer_status(start_time: OffsetDateTime, observed_at: OffsetDateTime) -> EventStatus {
    crate::time_utils::infer_status_at(
        observed_at,
        start_time,
        start_time + time::Duration::hours(3),
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_formula1_race_times_fixture() {
        let input = include_str!("../../tests/fixtures/formula1_2026_start_times.html");
        let events = parse_race_times_document_at(
            input,
            2026,
            time::macros::datetime!(2026-01-01 00:00 UTC),
        );
        assert_eq!(events.len(), 24);

        let miami = events
            .iter()
            .find(|event| event.id == "formula_1_2026_05_03_miami")
            .unwrap();
        assert_eq!(miami.title, "Miami Grand Prix");
        assert_eq!(
            miami.start_time.to_offset(time::UtcOffset::UTC),
            time::macros::datetime!(2026-05-03 20:00 UTC)
        );
        assert_eq!(miami.round_label.as_deref(), Some("Round 6"));
        assert_eq!(
            miami.venue.as_deref(),
            Some("Miami International Autodrome")
        );
    }
}
