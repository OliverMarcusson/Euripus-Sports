use regex::Regex;
use scraper::{Html, Selector};
use time::{macros::offset, Date, Month, OffsetDateTime, PrimitiveDateTime, UtcOffset};

use crate::domain::{EventSeed, EventStatus, Participants};

const STOCKHOLM: UtcOffset = offset!(+2);
const SOURCE_URL: &str =
    "https://www.fifa.com/en/tournaments/mens/worldcup/canadamexicousa2026/scores-fixtures";

pub fn parse_fifa_fixtures(input: &str) -> Vec<EventSeed> {
    if input.contains("/match-centre/match/") && input.contains("matches-container_title__") {
        return parse_rendered_html(input);
    }

    let image_re = Regex::new(r"!\[[^\]]*\]\([^)]*\)").unwrap();
    let markdown_link_re = Regex::new(r"^\[(?P<text>.+)\]\([^)]*\)$").unwrap();
    let date_re = Regex::new(r"^(Monday|Tuesday|Wednesday|Thursday|Friday|Saturday|Sunday)\s+(\d{1,2})\s+([A-Za-z]+)\s+(\d{4})$").unwrap();
    let match_re = Regex::new(
        r"^(?P<home_code>[A-Z]{3})\s+(?P<home>.+?)\s+(?P<time>\d{2}:\d{2})\s+(?P<away_code>[A-Z]{3})\s+(?P<away>.+?)\s+(?P<stage>(?:First Stage|Round of 32|Round of 16|Quarter-finals?|Semi-finals?|Third-place play-off|Final)(?:· Group [A-Z])?)·\s*(?P<venue>.+?)\((?P<city>.+?)\)$",
    )
    .unwrap();

    let mut current_date = None;
    let mut events = Vec::new();

    for raw_line in input.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let line = clean_line(raw_line, &image_re, &markdown_link_re);
        if line.is_empty() || line == "View groups" {
            continue;
        }

        if let Some(caps) = date_re.captures(&line) {
            let day = caps.get(2).and_then(|m| m.as_str().parse::<u8>().ok());
            let month = caps.get(3).and_then(|m| parse_month(m.as_str()));
            let year = caps.get(4).and_then(|m| m.as_str().parse::<i32>().ok());
            current_date = match (year, month, day) {
                (Some(year), Some(month), Some(day)) => {
                    Date::from_calendar_date(year, month, day).ok()
                }
                _ => None,
            };
            continue;
        }

        let Some(caps) = match_re.captures(&line) else {
            continue;
        };
        let Some(date) = current_date else { continue };

        let home = caps.name("home").unwrap().as_str().trim();
        let away = caps.name("away").unwrap().as_str().trim();
        let time = caps.name("time").unwrap().as_str();
        let stage = caps.name("stage").unwrap().as_str().trim();
        let venue = format!(
            "{} ({})",
            caps.name("venue").unwrap().as_str().trim(),
            caps.name("city").unwrap().as_str().trim()
        );
        let start_time = parse_datetime(date, time);

        events.push(build_event(
            date,
            home,
            away,
            start_time,
            Some(stage.into()),
            Some(venue),
            SOURCE_URL.into(),
        ));
    }

    events
}

fn parse_rendered_html(input: &str) -> Vec<EventSeed> {
    let document = Html::parse_document(input);
    let block_selector = Selector::parse("div[class*='ff-text-blue-dark']").unwrap();
    let title_selector = Selector::parse("div[class*='matches-container_title__']").unwrap();
    let match_selector = Selector::parse("a[href^='/en/match-centre/match/']").unwrap();
    let team_selector = Selector::parse("div[class*='match-row_team__']").unwrap();
    let desktop_name_selector = Selector::parse("span.d-none.d-md-block").unwrap();
    let time_selector = Selector::parse("span[class*='match-row_matchTime__']").unwrap();
    let bottom_label_selector = Selector::parse("span[class*='match-row_bottomLabel__']").unwrap();
    let stadium_selector =
        Selector::parse("div[class*='match-row_stadiumCityLabels__'] span").unwrap();
    let mut events = Vec::new();

    for block in document.select(&block_selector) {
        let Some(date_label) = block.select(&title_selector).next().map(text_content) else {
            continue;
        };
        let Some(date) = parse_rendered_date(&date_label) else {
            continue;
        };

        for game in block.select(&match_selector) {
            let teams = game
                .select(&team_selector)
                .filter_map(|team| team.select(&desktop_name_selector).next().map(text_content))
                .collect::<Vec<_>>();
            if teams.len() < 2 {
                continue;
            }
            let home = teams[0].trim();
            let away = teams[1].trim();
            let Some(time) = game.select(&time_selector).next().map(text_content) else {
                continue;
            };
            if !time.contains(':') {
                continue;
            }
            let start_time = parse_datetime(date, time.trim());
            let labels = game
                .select(&bottom_label_selector)
                .map(text_content)
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>();
            let stage = match labels.as_slice() {
                [stage, group, ..] => Some(format!("{}· {}", stage.trim(), group.trim())),
                [stage] => Some(stage.trim().to_string()),
                _ => None,
            };
            let stadium = game
                .select(&stadium_selector)
                .map(text_content)
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>();
            let venue = if stadium.len() >= 2 {
                Some(format!("{} {}", stadium[0].trim(), stadium[1].trim()))
            } else {
                None
            };
            let href = game.value().attr("href").unwrap_or(SOURCE_URL);
            let source_url = if href.starts_with("http") {
                href.to_string()
            } else {
                format!("https://www.fifa.com{href}")
            };

            events.push(build_event(
                date, home, away, start_time, stage, venue, source_url,
            ));
        }
    }

    events
}

fn parse_rendered_date(input: &str) -> Option<Date> {
    let parts = input.split_whitespace().collect::<Vec<_>>();
    if parts.len() != 4 {
        return None;
    }
    let day = parts[1].parse::<u8>().ok()?;
    let month = parse_month(parts[2])?;
    let year = parts[3].parse::<i32>().ok()?;
    Date::from_calendar_date(year, month, day).ok()
}

fn build_event(
    date: Date,
    home: &str,
    away: &str,
    start_time: OffsetDateTime,
    round_label: Option<String>,
    venue: Option<String>,
    source_url: String,
) -> EventSeed {
    EventSeed {
        id: format!(
            "fifa_world_cup_2026_{:02}_{:02}_{}_{}",
            date.month() as u8,
            date.day(),
            slugify(home),
            slugify(away)
        ),
        sport: "soccer".into(),
        competition: "fifa_world_cup_2026".into(),
        title: format!("{} vs {}", home, away),
        start_time,
        end_time: Some(start_time + time::Duration::hours(2)),
        status: infer_status(start_time),
        venue,
        round_label,
        participants: Participants {
            home: home.into(),
            away: away.into(),
        },
        source: "fifa-world-cup-fixture".into(),
        source_url,
    }
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

fn clean_line(input: &str, image_re: &Regex, markdown_link_re: &Regex) -> String {
    let without_images = image_re.replace_all(input, "");
    let collapsed = without_images
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(" ](", "](");

    markdown_link_re
        .captures(&collapsed)
        .and_then(|caps| caps.name("text").map(|m| m.as_str().trim().to_string()))
        .unwrap_or_else(|| collapsed.trim().to_string())
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
    .assume_offset(STOCKHOLM)
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
    fn parses_fifa_world_cup_fixtures() {
        let input = include_str!("../../tests/fixtures/fifa_world_cup_fifa_fixtures.md");
        let events = parse_fifa_fixtures(input);
        assert_eq!(events.len(), 17);
        assert!(events.iter().any(|event| {
            event.title == "Sweden vs Tunisia"
                && event.round_label.as_deref() == Some("First Stage· Group F")
                && event.venue.as_deref() == Some("Monterrey Stadium (Monterrey)")
        }));
    }

    #[test]
    fn parses_rendered_world_cup_html() {
        let input = r#"<html><body><div class="col-xl-12 col-lg-12 ff-pb-24 ff-text-blue-dark col-md-12 col-sm-12"><div class="matches-container_header__yYA5H"><div class="matches-container_title__ATLsl">Monday 15 June 2026</div></div><div class="row"><a href="/en/match-centre/match/17/285023/289273/400021474"><div class="match-row_matchRowContainer__NoCRI"><div class="match-row_matchRowBody__yc8mV"><div class="match-row_team__y5Rva justify-content-end"><span class="d-none d-md-block">Sweden</span></div><div class="match-row_matchRowStatus__AJE7s"><span class="match-row_matchTime__9QJXJ">04:00</span></div><div class="match-row_team__y5Rva"><span class="d-none d-md-block">Tunisia</span></div></div><div class="match-row_bottomLabelWrapper__9iAmu"><span class="match-row_bottomLabel__ni63b justify-content-end">First Stage</span><span class="match-row_bottomLabel__ni63b">Group F</span><div class="match-row_stadiumCityLabels__zjXUq"><span>Monterrey Stadium</span><span>(Monterrey)</span></div></div></div></a></div></div></body></html>"#;
        let events = parse_fifa_fixtures(input);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].title, "Sweden vs Tunisia");
        assert_eq!(
            events[0].round_label.as_deref(),
            Some("First Stage· Group F")
        );
        assert_eq!(
            events[0].venue.as_deref(),
            Some("Monterrey Stadium (Monterrey)")
        );
    }
}
