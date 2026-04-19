use regex::Regex;
use scraper::{Html, Selector};
use serde_json::Value;
use time::{
    format_description::well_known::Rfc3339,
    macros::{format_description, offset},
    Date, OffsetDateTime, PrimitiveDateTime, UtcOffset,
};

use crate::{
    config::AppConfig,
    domain::{EventSeed, EventStatus, Participants},
};

const DATE_FORMAT: &[time::format_description::FormatItem<'static>] =
    format_description!("[day] [month repr:long] [year]");
const STOCKHOLM: UtcOffset = offset!(+2);

#[derive(Debug, Clone, Copy)]
pub struct LeagueConfig<'a> {
    pub competition: &'a str,
    pub base_url: &'a str,
    pub source_prefix: &'a str,
    pub article_source_url: Option<&'a str>,
}

pub fn parse_document(
    input: &str,
    season: i32,
    config: &AppConfig,
    league: LeagueConfig<'_>,
) -> Vec<EventSeed> {
    if input.trim_start().starts_with('{') {
        return parse_graphql_response(input, season, config, league);
    }
    if input.contains("<html") || input.contains("<!DOCTYPE html") {
        return parse_html(input, season, config, league);
    }
    parse_markdown(input, season, config, league)
}

pub fn parse_markdown(
    input: &str,
    season: i32,
    config: &AppConfig,
    league: LeagueConfig<'_>,
) -> Vec<EventSeed> {
    let line_re = Regex::new(&format!(
        r#"^\[(?P<label>.+?)\]\((?P<url>https://{}(/matcher/\d{{4}}/\d+/[^)]+|/matcher/\d{{4}}/\d+/[^)]+))(?:\?live=true)?\)$"#,
        regex::escape(league.base_url)
    ))
    .unwrap();

    let mut current_round = None;
    let mut last_date = None;
    let mut events = Vec::new();

    for line in input.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if let Some(round) = line.strip_prefix("OMGÅNG ") {
            current_round = Some(format!("Round {}", round.trim()));
            continue;
        }

        let Some(caps) = line_re.captures(line) else {
            continue;
        };
        let label = caps.name("label").unwrap().as_str();
        let raw_url = caps.name("url").unwrap().as_str();
        let live = line.contains("?live=true") || label.starts_with("Idag ");

        let (date, remainder) = extract_date(label, season, last_date);
        last_date = date;
        let Some(date) = date else { continue };
        let teams = config.team_names_for_competition(league.competition);
        let parsed = split_fixture_parts(remainder, &teams)
            .map(|(venue, home, away, time)| {
                (venue.to_string(), home, away, Some(time.to_string()), live)
            })
            .or_else(|| {
                split_live_fixture_parts(remainder, &teams)
                    .map(|(venue, home, away)| (venue, home, away, None, true))
            });
        let Some((venue, home, away, time, live)) = parsed else {
            continue;
        };
        let home = config.canonical_team_name(league.competition, &home);
        let away = config.canonical_team_name(league.competition, &away);
        let start_time = time
            .as_deref()
            .map(|time| parse_datetime(date, time))
            .unwrap_or_else(|| OffsetDateTime::now_utc().to_offset(STOCKHOLM));
        let slug = raw_url.rsplit('/').next().unwrap_or("match");

        events.push(EventSeed {
            id: format!(
                "{}_{}_{}",
                league.competition,
                season,
                slug.replace('-', "_")
            ),
            sport: "soccer".into(),
            competition: league.competition.into(),
            title: format!("{} vs {}", home, away),
            start_time,
            end_time: Some(start_time + time::Duration::hours(2)),
            status: if live {
                EventStatus::Live
            } else {
                EventStatus::Upcoming
            },
            venue: Some(venue.trim().to_string()),
            round_label: current_round.clone(),
            participants: Participants {
                home: home.to_string(),
                away: away.to_string(),
            },
            source: format!("{}-fixture", league.source_prefix),
            source_url: raw_url.to_string(),
        });
    }

    events
}

pub fn parse_svenskfotboll_article(
    input: &str,
    season: i32,
    config: &AppConfig,
    league: LeagueConfig<'_>,
) -> Vec<EventSeed> {
    let date_re = Regex::new(r"^\*\*.+?,\s+(?P<day>\d{1,2})\s+(?P<month>[a-zåäö]+)\*\*$").unwrap();
    let match_re =
        Regex::new(r"^(?P<time>\d{1,2}\.\d{2})\s+(?P<home>.+?)\s+[–-]\s+(?P<away>.+)$").unwrap();
    let round_re = Regex::new(r"omgång\s+(?P<round>\d+)").unwrap();

    let mut current_date = None;
    let mut round_label = round_re
        .captures(input)
        .and_then(|caps| caps.name("round"))
        .map(|m| format!("Round {}", m.as_str()));
    let mut events = Vec::new();

    for line in input.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if let Some(caps) = date_re.captures(line) {
            let day = caps.name("day").and_then(|m| m.as_str().parse::<u8>().ok());
            let month = caps
                .name("month")
                .and_then(|m| parse_swedish_month(m.as_str()));
            current_date = day.and_then(|day| {
                month.and_then(|month| Date::from_calendar_date(season, month, day).ok())
            });
            continue;
        }

        if round_label.is_none() {
            round_label = round_re
                .captures(line)
                .and_then(|caps| caps.name("round"))
                .map(|m| format!("Round {}", m.as_str()));
        }

        let Some(caps) = match_re.captures(line) else {
            continue;
        };
        let Some(date) = current_date else { continue };
        let time = caps.name("time").unwrap().as_str().replace('.', ":");
        let home = config.canonical_team_name(
            league.competition,
            caps.name("home").unwrap().as_str().trim(),
        );
        let away = config.canonical_team_name(
            league.competition,
            caps.name("away").unwrap().as_str().trim(),
        );
        let start_time = parse_datetime(date, &time);

        events.push(EventSeed {
            id: format!(
                "{}_{}_{}_{}",
                league.competition,
                season,
                slugify(&home),
                slugify(&away)
            ),
            sport: "soccer".into(),
            competition: league.competition.into(),
            title: format!("{} vs {}", home, away),
            start_time,
            end_time: Some(start_time + time::Duration::hours(2)),
            status: infer_status(start_time),
            venue: None,
            round_label: round_label.clone(),
            participants: Participants { home, away },
            source: format!("svenskfotboll-{}-fixture", league.source_prefix),
            source_url: league.article_source_url.unwrap_or_default().into(),
        });
    }

    events
}

fn extract_date<'a>(
    label: &'a str,
    season: i32,
    fallback: Option<Date>,
) -> (Option<Date>, &'a str) {
    let mut parts = label.splitn(4, ' ');
    let day_word = parts.next().unwrap_or_default();
    if day_word == "Idag" {
        return (fallback, label.trim_start_matches("Idag "));
    }

    let Some(day) = parts.next() else {
        return (fallback, label);
    };
    let Some(month) = parts.next() else {
        return (fallback, label);
    };
    let remainder = parts.next().unwrap_or_default();
    let full = format!("{} {} {}", day, title_case(&month.to_lowercase()), season);
    let parsed = Date::parse(&full, DATE_FORMAT).ok();
    (parsed.or(fallback), remainder)
}

fn split_fixture_parts<'a>(
    input: &'a str,
    teams: &[String],
) -> Option<(&'a str, String, String, &'a str)> {
    let time = input.rsplit_once(' ')?.1;
    if !time.contains(':') {
        return None;
    }
    let body = input.strip_suffix(time)?.trim_end();

    let mut matched = None;
    for team in teams {
        let needle = format!(" {team} - ");
        if let Some(index) = body.find(&needle) {
            matched = Some((index, team.as_str()));
            break;
        }
    }
    let (index, home_team) = matched?;
    let venue = body[..index].trim();
    let away = body[index + 1 + home_team.len() + 3..].trim();
    Some((venue, home_team.to_string(), away.to_string(), time))
}

fn split_live_fixture_parts(input: &str, teams: &[String]) -> Option<(String, String, String)> {
    let body = input
        .strip_suffix("Följ match")
        .or_else(|| input.strip_suffix("Pågår"))?
        .trim_end();
    let score_re = Regex::new(r#"\s+\d+\s+\d+\s+Pågår$"#).unwrap();
    let body = score_re.replace(body, "");
    let body = body.as_ref().trim_end();

    let mut matched = None;
    for team in teams {
        let needle = format!(" {team} - ");
        if let Some(index) = body.find(&needle) {
            matched = Some((index, team.as_str()));
            break;
        }
    }
    let (index, home_team) = matched?;
    let venue = body[..index].trim().to_string();
    let away = body[index + 1 + home_team.len() + 3..].trim().to_string();
    Some((venue, home_team.to_string(), away))
}

fn parse_html(
    input: &str,
    season: i32,
    config: &AppConfig,
    league: LeagueConfig<'_>,
) -> Vec<EventSeed> {
    let document = Html::parse_document(input);
    let selector = Selector::parse("a").unwrap();
    let mut lines = Vec::new();

    for link in document.select(&selector) {
        let Some(href) = link.value().attr("href") else {
            continue;
        };
        if !href.contains("/matcher/") {
            continue;
        }
        let text = link.text().collect::<Vec<_>>().join(" ");
        let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if normalized.is_empty() {
            continue;
        }
        let absolute = if href.starts_with("http") {
            href.to_string()
        } else {
            format!("https://{}{}", league.base_url, href)
        };
        lines.push(format!("[{normalized}]({absolute})"));
    }

    parse_markdown(&lines.join("\n"), season, config, league)
}

fn parse_graphql_response(
    input: &str,
    season: i32,
    config: &AppConfig,
    league: LeagueConfig<'_>,
) -> Vec<EventSeed> {
    let value: Value = match serde_json::from_str(input) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };
    let matches = value
        .get("data")
        .and_then(|data| data.get("matchesForLeague"))
        .and_then(|data| data.get("matches"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    matches
        .into_iter()
        .filter_map(|game| {
            let home = game.get("homeTeamName")?.as_str()?.trim();
            let away = game.get("visitingTeamName")?.as_str()?.trim();
            let start_raw = game.get("startDate")?.as_str()?;
            let start_time = OffsetDateTime::parse(start_raw, &Rfc3339)
                .ok()?
                .to_offset(STOCKHOLM);
            let home = config.canonical_team_name(league.competition, home);
            let away = config.canonical_team_name(league.competition, away);
            let fogis_id = game
                .get("fogisId")
                .and_then(|value| value.as_i64())
                .unwrap_or_default();
            let round = game
                .get("round")
                .and_then(|value| value.as_i64())
                .map(|round| format!("Round {round}"));
            let venue = game
                .get("arenaName")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string);
            Some(EventSeed {
                id: format!("{}_{}_{}", league.competition, season, fogis_id),
                sport: "soccer".into(),
                competition: league.competition.into(),
                title: format!("{} vs {}", home, away),
                start_time,
                end_time: Some(start_time + time::Duration::hours(2)),
                status: status_from_graphql(&game, start_time),
                venue,
                round_label: round,
                participants: Participants { home, away },
                source: format!("{}-graphql", league.source_prefix),
                source_url: format!(
                    "https://{}/matcher/{}/{}",
                    league.base_url, season, fogis_id
                ),
            })
        })
        .collect()
}

fn status_from_graphql(game: &Value, start_time: OffsetDateTime) -> EventStatus {
    match game.get("status").and_then(|value| value.as_str()) {
        Some("PreEvent") => EventStatus::Upcoming,
        Some("PostEvent") | Some("Finished") | Some("FINISHED") => EventStatus::Finished,
        Some("Live") | Some("Ongoing") => EventStatus::Live,
        _ => infer_status(start_time),
    }
}

fn parse_swedish_month(value: &str) -> Option<time::Month> {
    match value.to_ascii_lowercase().as_str() {
        "januari" => Some(time::Month::January),
        "februari" => Some(time::Month::February),
        "mars" => Some(time::Month::March),
        "april" => Some(time::Month::April),
        "maj" => Some(time::Month::May),
        "juni" => Some(time::Month::June),
        "juli" => Some(time::Month::July),
        "augusti" => Some(time::Month::August),
        "september" => Some(time::Month::September),
        "oktober" => Some(time::Month::October),
        "november" => Some(time::Month::November),
        "december" => Some(time::Month::December),
        _ => None,
    }
}

fn parse_datetime(date: Date, time: &str) -> OffsetDateTime {
    let (hour, minute) = time.split_once(':').unwrap();
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
    } else if now <= start_time + time::Duration::hours(2) {
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

fn title_case(input: &str) -> String {
    input
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
