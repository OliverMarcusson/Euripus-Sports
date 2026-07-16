use std::collections::{HashMap, VecDeque};

use time::{Date, Duration, Month, OffsetDateTime, Weekday};
use time_tz::OffsetDateTimeExt;

use crate::config::AppConfig;

pub fn resolve_date(label: &str, observed_at: OffsetDateTime, season: i32) -> Option<Date> {
    let stockholm = time_tz::timezones::db::europe::STOCKHOLM;
    let today = observed_at.to_timezone(stockholm).date();
    let normalized = label.to_lowercase();
    if normalized.contains("i morgon") || normalized.contains("imorgon") {
        return today.next_day();
    }
    if normalized.contains("live") {
        return Some(today);
    }

    let numeric = regex::Regex::new(r"(?P<day>\d{1,2})/(?P<month>\d{1,2})").ok()?;
    if let Some(captures) = numeric.captures(&normalized) {
        return Date::from_calendar_date(
            season,
            Month::try_from(captures.name("month")?.as_str().parse::<u8>().ok()?).ok()?,
            captures.name("day")?.as_str().parse().ok()?,
        )
        .ok();
    }
    let named = regex::Regex::new(
        r"(?P<day>\d{1,2})(?::e)?\s+(?P<month>jan(?:uari)?|feb(?:ruari)?|mar(?:s)?|apr(?:il)?|maj|jun(?:i)?|jul(?:i)?|aug(?:usti)?|sep(?:tember)?|okt(?:ober)?|nov(?:ember)?|dec(?:ember)?)",
    )
    .ok()?;
    if let Some(captures) = named.captures(&normalized) {
        return Date::from_calendar_date(
            season,
            swedish_month(captures.name("month")?.as_str())?,
            captures.name("day")?.as_str().parse().ok()?,
        )
        .ok();
    }
    for (name, weekday) in [
        ("måndag", Weekday::Monday),
        ("tisdag", Weekday::Tuesday),
        ("onsdag", Weekday::Wednesday),
        ("torsdag", Weekday::Thursday),
        ("fredag", Weekday::Friday),
        ("lördag", Weekday::Saturday),
        ("söndag", Weekday::Sunday),
    ] {
        if normalized.contains(name) {
            let days = (weekday.number_days_from_monday() as i64
                - today.weekday().number_days_from_monday() as i64)
                .rem_euclid(7);
            return today.checked_add(Duration::days(days));
        }
    }
    None
}

pub fn interval(
    date: Date,
    start_clock: &str,
    end_clock: Option<&str>,
) -> Option<(OffsetDateTime, OffsetDateTime)> {
    let stockholm = time_tz::timezones::db::europe::STOCKHOLM;
    let start = crate::time_utils::local_datetime(date, &normalize_clock(start_clock)?, stockholm)?;
    let end = match end_clock {
        Some(clock) => {
            crate::time_utils::local_datetime(date, &normalize_clock(clock)?, stockholm)?
        }
        None => start + Duration::hours(3),
    };
    Some((
        start,
        if end < start {
            end + Duration::days(1)
        } else {
            end
        },
    ))
}

fn normalize_clock(value: &str) -> Option<String> {
    let values = regex::Regex::new(r"(?P<hour>\d{1,2})[:.\s](?P<minute>\d{2})")
        .ok()?
        .captures(value)?;
    Some(format!(
        "{:02}:{:02}",
        values.name("hour")?.as_str().parse::<u8>().ok()?,
        values.name("minute")?.as_str().parse::<u8>().ok()?
    ))
}

pub fn enrich_tv4(
    overlays: &mut [crate::domain::WatchOverlay],
    input: &str,
    observed_at: OffsetDateTime,
    season: i32,
) {
    let clock_regex = regex::Regex::new(r"\d{1,2}:\d{2}").expect("valid listing clock regex");
    let lines_by_identity = input
        .lines()
        .filter_map(|line| {
            overlays.iter().find_map(|overlay| {
                let identity = tv4_identity(&overlay.source_url);
                line.contains(identity)
                    .then(|| (identity.to_string(), line))
            })
        })
        .collect::<HashMap<_, _>>();

    for overlay in overlays {
        let Some(line) = lines_by_identity.get(tv4_identity(&overlay.source_url)) else {
            continue;
        };
        if line.to_lowercase().contains("live") && !line.chars().any(|character| character == ':') {
            overlay.airing_start = Some(observed_at);
            overlay.airing_end = Some(observed_at);
            overlay.season = Some(season);
            continue;
        }
        let Some(date) = resolve_date(line, observed_at, season) else {
            continue;
        };
        let Some(clock) = clock_regex.find(line) else {
            continue;
        };
        if let Some((start, end)) = interval(date, clock.as_str(), None) {
            overlay.airing_start = Some(start);
            overlay.airing_end = Some(end);
            overlay.season = Some(season);
        }
    }
}

pub fn enrich_viaplay(
    overlays: &mut [crate::domain::WatchOverlay],
    input: &str,
    observed_at: OffsetDateTime,
    season: i32,
    competition: &str,
    config: &AppConfig,
) {
    let mut blocks = input
        .split("\n\n")
        .filter_map(|block| {
            let lines = block
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>();
            if lines.len() < 3 {
                return None;
            }
            let date = resolve_date(lines[0], observed_at, season)?;
            let clocks = regex::Regex::new(r"\d{1,2}[.:]\d{2}")
                .ok()?
                .find_iter(lines[1])
                .map(|value| value.as_str())
                .collect::<Vec<_>>();
            let (start, mut end) = interval(date, clocks.first()?, clocks.get(1).copied())?;
            if clocks.len() == 1 {
                if let Some(duration) = regex::Regex::new(r"(?P<h>\d+)h\s*(?P<m>\d+)m")
                    .ok()?
                    .captures(lines[1])
                {
                    end = start
                        + Duration::hours(duration.name("h")?.as_str().parse().ok()?)
                        + Duration::minutes(duration.name("m")?.as_str().parse().ok()?);
                }
            }
            let title = canonical_listing_title(lines[2], competition, config)?;
            Some((normalize(&title), (start, end)))
        })
        .fold(
            HashMap::<String, VecDeque<(OffsetDateTime, OffsetDateTime)>>::new(),
            |mut blocks, (title, interval)| {
                blocks.entry(title).or_default().push_back(interval);
                blocks
            },
        );

    for overlay in overlays {
        let title = normalize(&overlay.title);
        if let Some((start, end)) = blocks.get_mut(&title).and_then(VecDeque::pop_front) {
            overlay.airing_start = Some(start);
            overlay.airing_end = Some(end);
            overlay.season = Some(season);
        }
    }
}

fn tv4_identity(source_url: &str) -> &str {
    source_url
        .strip_prefix("https://www.tv4play.se")
        .unwrap_or(source_url)
}

fn canonical_listing_title(line: &str, competition: &str, config: &AppConfig) -> Option<String> {
    let title = line
        .strip_prefix("PL-studion:")
        .or_else(|| line.strip_prefix("CL-studion:"))
        .unwrap_or(line)
        .split(',')
        .next()?
        .trim();
    let (home, away) = title.split_once(" - ")?;
    Some(format!(
        "{} - {}",
        config.canonical_team_name(competition, home),
        config.canonical_team_name(competition, away)
    ))
}

fn normalize(value: &str) -> String {
    value
        .to_lowercase()
        .replace("if", "")
        .replace(['å', 'ä'], "a")
        .replace('ö', "o")
        .chars()
        .filter(|character| character.is_alphanumeric())
        .collect()
}

fn swedish_month(value: &str) -> Option<Month> {
    Some(match value {
        value if value.starts_with("jan") => Month::January,
        value if value.starts_with("feb") => Month::February,
        value if value.starts_with("mar") => Month::March,
        value if value.starts_with("apr") => Month::April,
        "maj" => Month::May,
        value if value.starts_with("jun") => Month::June,
        value if value.starts_with("jul") => Month::July,
        value if value.starts_with("aug") => Month::August,
        value if value.starts_with("sep") => Month::September,
        value if value.starts_with("okt") => Month::October,
        value if value.starts_with("nov") => Month::November,
        value if value.starts_with("dec") => Month::December,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn config() -> AppConfig {
        AppConfig::load(
            "config/providers.yaml",
            "config/competition_rules.yaml",
            "config/sample_events.yaml",
            "config/sources.yaml",
            "config/team_aliases.yaml",
        )
        .unwrap()
    }

    #[test]
    fn resolves_relative_and_explicit_stockholm_listing_times() {
        let observed = datetime!(2026-04-17 10:00 UTC);
        let tomorrow = resolve_date("I morgon", observed, 2026).unwrap();
        assert_eq!(tomorrow, time::macros::date!(2026 - 04 - 18));
        let explicit = resolve_date("28 apr", observed, 2026).unwrap();
        let (start, end) = interval(explicit, "18.50", Some("22.00")).unwrap();
        assert_eq!(
            start.to_offset(time::UtcOffset::UTC),
            datetime!(2026-04-28 16:50 UTC)
        );
        assert_eq!(
            end.to_offset(time::UtcOffset::UTC),
            datetime!(2026-04-28 20:00 UTC)
        );
    }

    #[test]
    fn tv4_repeated_pairings_keep_their_listing_times() {
        let input = include_str!("../../tests/fixtures/tv4_shl_readability.md");
        let config = config();
        let mut overlays = crate::sources::tv4play::parse_shl_document(input, &config);
        enrich_tv4(&mut overlays, input, datetime!(2026-04-17 10:00 UTC), 2026);

        let repeated = overlays
            .iter()
            .filter(|overlay| overlay.title == "Skellefteå AIK - Rögle BK")
            .collect::<Vec<_>>();
        assert_eq!(repeated.len(), 2);
        assert_eq!(
            repeated[0]
                .airing_start
                .unwrap()
                .to_offset(time::UtcOffset::UTC),
            datetime!(2026-04-23 16:30 UTC)
        );
        assert_eq!(
            repeated[1]
                .airing_start
                .unwrap()
                .to_offset(time::UtcOffset::UTC),
            datetime!(2026-04-25 12:30 UTC)
        );
        assert_ne!(repeated[0].source_url, repeated[1].source_url);
    }

    #[test]
    fn viaplay_canonicalized_titles_keep_each_blocks_time() {
        let input = include_str!("../../tests/fixtures/viaplay_champions_league_index.md");
        let config = config();
        let mut overlays = crate::sources::viaplay::parse_champions_league_document(input, &config);
        enrich_viaplay(
            &mut overlays,
            input,
            datetime!(2026-04-17 10:00 UTC),
            2026,
            "uefa_champions_league",
            &config,
        );

        let repeated = overlays
            .iter()
            .filter(|overlay| overlay.title == "Paris Saint-Germain - Bayern Munich")
            .collect::<Vec<_>>();
        assert_eq!(repeated.len(), 2);
        assert_eq!(
            repeated[0]
                .airing_start
                .unwrap()
                .to_offset(time::UtcOffset::UTC),
            datetime!(2026-04-28 16:00 UTC)
        );
        assert_eq!(
            repeated[1]
                .airing_start
                .unwrap()
                .to_offset(time::UtcOffset::UTC),
            datetime!(2026-04-28 16:50 UTC)
        );
    }
}
