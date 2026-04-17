use std::collections::{BTreeMap, HashMap};

use regex::Regex;
use scraper::{Html, Selector};
use serde_json::Value;
use time::{macros::offset, Date, Month, OffsetDateTime, PrimitiveDateTime, Time, UtcOffset};

use crate::domain::{EventSeed, EventStatus, Participants, WatchOverlay};

const EASTERN: UtcOffset = offset!(-4);
const STOCKHOLM: UtcOffset = offset!(+2);

pub fn parse_schedule_document(input: &str, season: i32) -> Vec<EventSeed> {
    if input.contains("__NEXT_DATA__") {
        return parse_schedule_html_document(input, season);
    }

    let tournament_re = Regex::new(r"###\s+(.+?)Right Arrow").unwrap();
    let round_re = Regex::new(r"\bR(?P<round>\d)\b").unwrap();

    let tournament = tournament_re
        .captures(input)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().trim().to_string());
    let round = round_re
        .captures(input)
        .and_then(|caps| caps.name("round"))
        .and_then(|m| m.as_str().parse::<u8>().ok());

    build_current_round_event(tournament, round, season)
        .into_iter()
        .collect()
}

pub fn parse_broadcast_events_document(input: &str, season: i32) -> Vec<EventSeed> {
    let entries = parse_broadcast_entries(input, season);
    let mut grouped: BTreeMap<(String, u8), Vec<BroadcastEntry>> = BTreeMap::new();
    for entry in entries {
        grouped
            .entry((entry.tournament.clone(), entry.round))
            .or_default()
            .push(entry);
    }

    grouped
        .into_values()
        .filter_map(|entries| {
            let first = entries.iter().min_by_key(|entry| entry.start_time)?;
            let last = entries.iter().max_by_key(|entry| entry.end_time)?;
            Some(EventSeed {
                id: format!(
                    "pga_tour_{}_{}_round_{}",
                    season,
                    slugify(&first.tournament),
                    first.round
                ),
                sport: "golf".into(),
                competition: "pga_tour".into(),
                title: format!("{} Round {}", first.tournament, first.round),
                start_time: first.start_time.to_offset(STOCKHOLM),
                end_time: Some(last.end_time.to_offset(STOCKHOLM)),
                status: infer_status(first.start_time, last.end_time),
                venue: None,
                round_label: Some(format!("Round {}", first.round)),
                participants: Participants {
                    home: first.tournament.clone(),
                    away: "Field".into(),
                },
                source: "pga-tour-broadcast".into(),
                source_url: "https://pgatourmedia.pgatourhq.com/broadcast-schedule".into(),
            })
        })
        .collect()
}

pub fn parse_broadcast_watch_document(input: &str, season: i32) -> Vec<WatchOverlay> {
    let entries = parse_broadcast_entries(input, season);
    let network_to_provider: HashMap<&str, (&str, &str)> = HashMap::from([
        ("ESPN+", ("espn", "ESPN+")),
        ("GOLF Channel", ("peacock", "Golf Channel")),
        ("CBS", ("paramount", "CBS")),
        ("Paramount+", ("paramount", "Paramount+")),
    ]);

    entries
        .into_iter()
        .filter_map(|entry| {
            let (provider_family, provider_label) =
                network_to_provider.get(entry.network.as_str())?;
            Some(WatchOverlay {
                competition: "pga_tour".into(),
                market: "us".into(),
                provider_family: (*provider_family).into(),
                provider_label: (*provider_label).into(),
                title: format!("{} Round {}", entry.tournament, entry.round),
                participants: Participants {
                    home: entry.tournament.clone(),
                    away: "Field".into(),
                },
                channel_name: Some(entry.network.clone()),
                watch_type: "ppv-event".into(),
                confidence: 0.99,
                source: "pga-tour-broadcast".into(),
                source_url: "https://pgatourmedia.pgatourhq.com/broadcast-schedule".into(),
            })
        })
        .collect()
}

pub fn parse_svensk_golf_watch_document(input: &str, season: i32) -> Vec<WatchOverlay> {
    if input.contains("<html") || input.contains("<!DOCTYPE html") {
        return parse_svensk_golf_watch_html(input, season);
    }

    let tournament_re =
        Regex::new(r"(?m)^(?P<name>[A-ZÅÄÖ][A-Za-zÅÄÖåäö0-9'&+\- ]+?)\s+är årets").unwrap();
    let weekly_heading_re =
        Regex::new(r"(?m)^([A-ZÅÄÖ][A-Za-zÅÄÖåäö0-9'&+\- ]+)\n\n(?:Discovery\+|Eurosport|HBO Max)")
            .unwrap();
    let day_re = Regex::new(
        r"^\*\s+(?P<day>\d{1,2})\s+[a-zåäö]+\.,\s+(?P<label>torsdag|fredag|lördag|söndag)$",
    )
    .unwrap();
    let time_re =
        Regex::new(r"^\*\s+(?P<time>\d{1,2}:\d{2})\s+(?P<label>Feeder|Huvudsändning|Eurosport 2)$")
            .unwrap();

    let tournament = tournament_re
        .captures(input)
        .and_then(|caps| caps.name("name"))
        .map(|m| m.as_str().trim().to_string())
        .or_else(|| {
            weekly_heading_re
                .captures(input)
                .and_then(|caps| caps.get(1))
                .map(|m| m.as_str().trim().to_string())
        })
        .unwrap_or_else(|| "PGA Tour".to_string());

    let mut overlays = Vec::new();
    let mut current_day: Option<(u8, u8)> = None;

    for line in input.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if let Some(caps) = day_re.captures(line) {
            let day = caps.name("day").and_then(|m| m.as_str().parse::<u8>().ok());
            let round = caps.name("label").and_then(|m| match m.as_str() {
                "torsdag" => Some(1),
                "fredag" => Some(2),
                "lördag" => Some(3),
                "söndag" => Some(4),
                _ => None,
            });
            current_day = day.zip(round);
            continue;
        }

        let Some((_, round)) = current_day else {
            continue;
        };
        let Some(caps) = time_re.captures(line) else {
            continue;
        };
        let label = caps.name("label").unwrap().as_str();
        overlays.push(build_svensk_golf_overlay(&tournament, round, label));
    }

    overlays
}

fn parse_schedule_html_document(input: &str, season: i32) -> Vec<EventSeed> {
    let next_data_re =
        Regex::new(r#"<script[^>]+id="__NEXT_DATA__"[^>]*>(?P<json>.*?)</script>"#).unwrap();
    let Some(json) = next_data_re
        .captures(input)
        .and_then(|caps| caps.name("json"))
        .map(|value| value.as_str())
    else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let Some(queries) = value
        .get("props")
        .and_then(|value| value.get("pageProps"))
        .and_then(|value| value.get("dehydratedState"))
        .and_then(|value| value.get("queries"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let tournament = queries
        .iter()
        .find(|query| {
            query
                .get("queryKey")
                .and_then(Value::as_array)
                .and_then(|value| value.first())
                .and_then(Value::as_str)
                == Some("tournament")
        })
        .and_then(|query| query.get("state"))
        .and_then(|value| value.get("data"));
    let tournament_name = tournament
        .and_then(|value| value.get("tournamentName"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let round = tournament
        .and_then(|value| value.get("currentRound"))
        .and_then(Value::as_u64)
        .and_then(|value| u8::try_from(value).ok());

    build_current_round_event(tournament_name, round, season)
        .into_iter()
        .collect()
}

fn build_current_round_event(
    tournament: Option<String>,
    round: Option<u8>,
    season: i32,
) -> Option<EventSeed> {
    let tournament = tournament?;
    let round = round?;

    Some(EventSeed {
        id: format!(
            "pga_tour_{}_{}_round_{}",
            season,
            slugify(&tournament),
            round
        ),
        sport: "golf".into(),
        competition: "pga_tour".into(),
        title: format!("{} Round {}", tournament, round),
        start_time: OffsetDateTime::now_utc(),
        end_time: None,
        status: EventStatus::Live,
        venue: None,
        round_label: Some(format!("Round {}", round)),
        participants: Participants {
            home: tournament.clone(),
            away: "Field".into(),
        },
        source: "pga-tour-schedule".into(),
        source_url: "https://www.pgatour.com/schedule".into(),
    })
}

fn parse_broadcast_entries_html(input: &str, season: i32) -> Vec<BroadcastEntry> {
    let document = Html::parse_document(input);
    let row_selector = Selector::parse("table.table tbody tr").unwrap();
    let mut entries = Vec::new();

    for row in document.select(&row_selector) {
        let text = row.text().collect::<Vec<_>>().join(" ");
        let clean = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let tournament = extract_labeled_value(&clean, "Tournament", &["Round"]);
        let round = extract_labeled_value(&clean, "Round", &["Date"])
            .and_then(|value| value.parse::<u8>().ok());
        let date = extract_labeled_value(&clean, "Date", &["Airtime"])
            .and_then(|value| parse_date(&value, season));
        let airtime = extract_labeled_value(&clean, "Airtime", &["Network"])
            .and_then(|value| parse_airtime(&value));
        let network = extract_labeled_value(&clean, "Network", &["Content Type"]);

        if let (Some(tournament), Some(round), Some(date), Some((start, end)), Some(network)) =
            (tournament, round, date, airtime, network)
        {
            entries.push(BroadcastEntry {
                tournament,
                round,
                start_time: PrimitiveDateTime::new(date, start).assume_offset(EASTERN),
                end_time: PrimitiveDateTime::new(date, end).assume_offset(EASTERN),
                network,
            });
        }
    }

    entries
}

fn extract_labeled_value(input: &str, label: &str, next_labels: &[&str]) -> Option<String> {
    let marker = format!("{label}:");
    let start = input.find(&marker)? + marker.len();
    let rest = input[start..].trim();
    let end = next_labels
        .iter()
        .filter_map(|next| rest.find(&format!("{next}:")))
        .min()
        .unwrap_or(rest.len());
    let value = rest[..end].trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn parse_svensk_golf_watch_html(input: &str, season: i32) -> Vec<WatchOverlay> {
    let document = Html::parse_document(input);
    let block_selector =
        Selector::parse("div.flex.flex-wrap.gap-5.py-8.border-t.border-t-grey-100").unwrap();
    let title_selector = Selector::parse("h2.text-xl").unwrap();
    let date_item_selector =
        Selector::parse("div.flex.flex-col.flex-shrink.gap-6 > ul[role='list'] > li").unwrap();
    let date_label_selector = Selector::parse("span.whitespace-nowrap").unwrap();
    let nested_item_selector = Selector::parse("ul[role='list'] > li").unwrap();
    let span_selector = Selector::parse("span").unwrap();
    let mut overlays = Vec::new();

    for block in document.select(&block_selector) {
        let Some(tournament) = block
            .select(&title_selector)
            .next()
            .map(text_content)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            continue;
        };

        for date_item in block.select(&date_item_selector) {
            let Some(date_label) = date_item
                .select(&date_label_selector)
                .next()
                .map(text_content)
            else {
                continue;
            };
            let Some(round) = round_from_swedish_date_label(&date_label) else {
                continue;
            };

            for item in date_item.select(&nested_item_selector) {
                let values = item
                    .select(&span_selector)
                    .map(text_content)
                    .collect::<Vec<_>>();
                if values.len() < 2 {
                    continue;
                }
                let label = values[1].trim();
                if !matches!(label, "Feeder" | "Huvudsändning" | "Eurosport 2") {
                    continue;
                }
                let _season = season;
                overlays.push(build_svensk_golf_overlay(&tournament, round, label));
            }
        }
    }

    overlays
}

fn round_from_swedish_date_label(input: &str) -> Option<u8> {
    let label = input.to_ascii_lowercase();
    if label.contains("torsdag") {
        Some(1)
    } else if label.contains("fredag") {
        Some(2)
    } else if label.contains("lördag") {
        Some(3)
    } else if label.contains("söndag") {
        Some(4)
    } else {
        None
    }
}

fn build_svensk_golf_overlay(tournament: &str, round: u8, label: &str) -> WatchOverlay {
    WatchOverlay {
        competition: "pga_tour".into(),
        market: "se".into(),
        provider_family: "max".into(),
        provider_label: "Max".into(),
        title: format!("{} Round {}", tournament, round),
        participants: Participants {
            home: tournament.to_string(),
            away: "Field".into(),
        },
        channel_name: Some(label.to_string()),
        watch_type: "streaming+linear".into(),
        confidence: if label == "Eurosport 2" { 0.97 } else { 0.99 },
        source: "svensk-golf-tv-guide".into(),
        source_url: "https://www.svenskgolf.se/sidor/har-ar-veckans-livesandningar/".into(),
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

#[derive(Debug, Clone)]
struct BroadcastEntry {
    tournament: String,
    round: u8,
    start_time: OffsetDateTime,
    end_time: OffsetDateTime,
    network: String,
}

fn parse_broadcast_entries(input: &str, season: i32) -> Vec<BroadcastEntry> {
    if input.contains("<html") || input.contains("<!DOCTYPE html") {
        return parse_broadcast_entries_html(input, season);
    }

    let mut entries = Vec::new();
    for chunk in input.split("\n\n") {
        let mut tournament = None;
        let mut round = None;
        let mut date = None;
        let mut airtime = None;
        let mut network = None;

        for line in chunk.lines().map(str::trim).filter(|line| !line.is_empty()) {
            let clean = line.trim_end_matches("  ");
            if let Some(value) = clean.strip_prefix("Tournament: ") {
                tournament = Some(value.to_string());
            } else if let Some(value) = clean.strip_prefix("Round: ") {
                round = value.parse::<u8>().ok();
            } else if let Some(value) = clean.strip_prefix("Date: ") {
                date = parse_date(value, season);
            } else if let Some(value) = clean.strip_prefix("Airtime: ") {
                airtime = parse_airtime(value);
            } else if let Some(value) = clean.strip_prefix("Network: ") {
                network = Some(value.to_string());
            }
        }

        if let (Some(tournament), Some(round), Some(date), Some((start, end)), Some(network)) =
            (tournament, round, date, airtime, network)
        {
            let start_time = PrimitiveDateTime::new(date, start).assume_offset(EASTERN);
            let end_time = PrimitiveDateTime::new(date, end).assume_offset(EASTERN);
            entries.push(BroadcastEntry {
                tournament,
                round,
                start_time,
                end_time,
                network,
            });
        }
    }
    entries
}

fn parse_date(value: &str, season: i32) -> Option<Date> {
    let (_, md) = value.split_once(' ')?;
    let (month, day) = md.split_once('/')?;
    Date::from_calendar_date(
        season,
        Month::try_from(month.parse::<u8>().ok()?).ok()?,
        day.parse::<u8>().ok()?,
    )
    .ok()
}

fn parse_airtime(value: &str) -> Option<(Time, Time)> {
    let cleaned = value.trim_end_matches(" Eastern");
    let (start, end) = cleaned.split_once(" - ")?;
    Some((parse_time(start)?, parse_time(end)?))
}

fn parse_time(value: &str) -> Option<Time> {
    let (time, meridiem) = value.trim().split_once(' ')?;
    let (hour, minute) = time.split_once(':')?;
    let mut hour = hour.parse::<u8>().ok()?;
    let minute = minute.parse::<u8>().ok()?;
    match meridiem {
        "PM" if hour != 12 => hour += 12,
        "AM" if hour == 12 => hour = 0,
        _ => {}
    }
    Time::from_hms(hour, minute, 0).ok()
}

fn slugify(value: &str) -> String {
    value
        .to_lowercase()
        .replace('&', "and")
        .chars()
        .map(|ch| if ch.is_alphanumeric() { ch } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

fn infer_status(start: OffsetDateTime, end: OffsetDateTime) -> EventStatus {
    let now = OffsetDateTime::now_utc();
    if now < start {
        EventStatus::Upcoming
    } else if now <= end {
        EventStatus::Live
    } else {
        EventStatus::Finished
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_schedule_document() {
        let input = include_str!("../../tests/fixtures/pga_schedule_readability.md");
        let events = parse_schedule_document(input, 2026);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].title, "RBC Heritage Round 2");
    }

    #[test]
    fn parses_schedule_html_document() {
        let input = r#"<html><body><script id="__NEXT_DATA__" type="application/json">{"props":{"pageProps":{"dehydratedState":{"queries":[{"queryKey":["tournament",{"id":"R2026012"}],"state":{"data":{"tournamentName":"RBC Heritage","currentRound":2}}}]}}}}</script></body></html>"#;
        let events = parse_schedule_document(input, 2026);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].title, "RBC Heritage Round 2");
    }

    #[test]
    fn parses_broadcast_events() {
        let input = include_str!("../../tests/fixtures/pga_broadcast_schedule_readability.md");
        let events = parse_broadcast_events_document(input, 2026);
        assert_eq!(events.len(), 4);
        assert!(events
            .iter()
            .any(|event| event.title == "RBC Heritage Round 3"));
    }

    #[test]
    fn parses_broadcast_watch() {
        let input = include_str!("../../tests/fixtures/pga_broadcast_schedule_readability.md");
        let overlays = parse_broadcast_watch_document(input, 2026);
        assert!(overlays
            .iter()
            .any(|overlay| overlay.provider_label == "ESPN+"));
        assert!(overlays
            .iter()
            .any(|overlay| overlay.provider_label == "Paramount+"));
    }

    #[test]
    fn parses_broadcast_html() {
        let input = r#"<html><body><table class="table"><tbody><tr><td><span class="font-weight-bold">Tournament:</span> RBC Heritage<br/><span class="font-weight-bold">Round:</span> 1<br/><span class="font-weight-bold">Date:</span> TH 04/16<br/><span class="font-weight-bold">Airtime:</span> 7:00 AM - 2:00 PM Eastern<br/><span class="font-weight-bold">Network:</span> ESPN+<br/><span class="font-weight-bold">Content Type:</span> Full Telecast<br/></td></tr></tbody></table></body></html>"#;
        let overlays = parse_broadcast_watch_document(input, 2026);
        assert_eq!(overlays.len(), 1);
        assert_eq!(overlays[0].provider_label, "ESPN+");
    }

    #[test]
    fn parses_svensk_golf_watch() {
        let input = include_str!("../../tests/fixtures/pga_svenskgolf_rbc_heritage.md");
        let overlays = parse_svensk_golf_watch_document(input, 2026);
        assert!(overlays
            .iter()
            .any(|overlay| overlay.title == "RBC Heritage Round 3"));
        assert!(overlays
            .iter()
            .all(|overlay| overlay.provider_family == "max"));
    }

    #[test]
    fn parses_svensk_golf_weekly_watch() {
        let input = include_str!("../../tests/fixtures/pga_svenskgolf_weekly.md");
        let overlays = parse_svensk_golf_watch_document(input, 2026);
        assert!(overlays
            .iter()
            .any(|overlay| overlay.title == "RBC Heritage Round 4"));
    }

    #[test]
    fn parses_svensk_golf_html_watch() {
        let input = r#"<html><body><div class="flex flex-wrap gap-5 py-8 border-t border-t-grey-100"><div class="flex flex-col items-start gap-5 flex-auto w-[60%]"><h2 class="text-xl">RBC Heritage</h2></div><div class="flex flex-col flex-shrink gap-6"><ul role="list"><li class="flex gap-4 py-3 border-t border-t-grey-100"><span class="whitespace-nowrap w-[112px]"><span class="font-bold">16</span> apr., torsdag</span><ul role="list"><li class="flex gap-2.5"><span class="w-[5ch]">13:00</span><span>Feeder</span></li><li class="flex gap-2.5"><span class="w-[5ch]">20:00</span><span>Eurosport 2</span></li></ul></li></ul></div></div></body></html>"#;
        let overlays = parse_svensk_golf_watch_document(input, 2026);
        assert_eq!(overlays.len(), 2);
        assert!(overlays
            .iter()
            .any(|overlay| overlay.title == "RBC Heritage Round 1"));
    }
}
