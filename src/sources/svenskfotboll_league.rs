use regex::Regex;
use scraper::{Html, Selector};
use serde_json::Value;
use time::{
    format_description::well_known::Rfc3339, macros::format_description, Date, OffsetDateTime,
};

use crate::{
    config::AppConfig,
    domain::{EventSeed, EventStatus, Participants},
};

const DATE_FORMAT: &[time::format_description::FormatItem<'static>] =
    format_description!("[day] [month repr:long] [year]");

#[derive(Debug, Clone, Copy)]
pub struct LeagueConfig<'a> {
    pub competition: &'a str,
    pub base_url: &'a str,
    pub source_prefix: &'a str,
}

pub fn parse_document_at(
    input: &str,
    season: i32,
    config: &AppConfig,
    league: LeagueConfig<'_>,
    observed_at: OffsetDateTime,
) -> Vec<EventSeed> {
    if input.trim_start().starts_with('{') {
        return parse_graphql_response(input, season, config, league, observed_at);
    }
    if input.contains("<html") || input.contains("<!DOCTYPE html") {
        return parse_html(input, season, config, league, observed_at);
    }
    parse_markdown(input, season, config, league, observed_at)
}

pub fn parse_markdown(
    input: &str,
    season: i32,
    config: &AppConfig,
    league: LeagueConfig<'_>,
    observed_at: OffsetDateTime,
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
        let start_time = match time.as_deref() {
            Some(value) => {
                let Some(start_time) = parse_datetime(date, value) else {
                    continue;
                };
                start_time
            }
            None => observed_at,
        };
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

fn extract_date(label: &str, season: i32, fallback: Option<Date>) -> (Option<Date>, &str) {
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
    observed_at: OffsetDateTime,
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

    parse_markdown(&lines.join("\n"), season, config, league, observed_at)
}

fn parse_graphql_response(
    input: &str,
    season: i32,
    config: &AppConfig,
    league: LeagueConfig<'_>,
    observed_at: OffsetDateTime,
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
            let start_time = OffsetDateTime::parse(start_raw, &Rfc3339).ok()?;
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
                status: status_from_graphql(&game, start_time, observed_at),
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

fn status_from_graphql(
    game: &Value,
    start_time: OffsetDateTime,
    observed_at: OffsetDateTime,
) -> EventStatus {
    match game.get("status").and_then(|value| value.as_str()) {
        Some("PreEvent") => EventStatus::Upcoming,
        Some("PostEvent") | Some("Finished") | Some("FINISHED") => EventStatus::Finished,
        Some("Live") | Some("Ongoing") => EventStatus::Live,
        _ => infer_status(start_time, observed_at),
    }
}

fn parse_datetime(date: Date, value: &str) -> Option<OffsetDateTime> {
    super::parse_clock_in_timezone(date, value, time_tz::timezones::db::europe::STOCKHOLM)
}

fn infer_status(start_time: OffsetDateTime, observed_at: OffsetDateTime) -> EventStatus {
    crate::time_utils::infer_status_at(
        observed_at,
        start_time,
        start_time + time::Duration::hours(2),
    )
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
