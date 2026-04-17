use regex::Regex;
use scraper::{Html, Selector};

use crate::{
    config::AppConfig,
    domain::{Participants, WatchOverlay},
};

pub fn parse_document(input: &str, config: &AppConfig) -> Vec<WatchOverlay> {
    if input.contains("<html") || input.contains("<!DOCTYPE html") {
        return parse_category_html(
            input,
            config,
            "allsvenskan",
            "Allsvenskan",
            "https://www.tv4play.se/kategorier/allsvenskan",
        );
    }
    parse_allsvenskan_markdown(input, config)
}

pub fn parse_shl_document(input: &str, config: &AppConfig) -> Vec<WatchOverlay> {
    if input.contains("<html") || input.contains("<!DOCTYPE html") {
        return parse_category_html(
            input,
            config,
            "shl",
            "SHL",
            "https://www.tv4play.se/kategorier/shl",
        );
    }
    parse_hockey_markdown(
        input,
        config,
        "shl",
        "SHL",
        "https://www.tv4play.se/kategorier/shl",
    )
}

pub fn parse_hockeyallsvenskan_document(input: &str, config: &AppConfig) -> Vec<WatchOverlay> {
    if input.contains("<html") || input.contains("<!DOCTYPE html") {
        return parse_category_html(
            input,
            config,
            "hockeyallsvenskan",
            "HockeyAllsvenskan",
            "https://www.tv4play.se/kategorier/hockeyallsvenskan",
        );
    }
    parse_hockey_markdown(
        input,
        config,
        "hockeyallsvenskan",
        "HockeyAllsvenskan",
        "https://www.tv4play.se/kategorier/hockeyallsvenskan",
    )
}

pub fn parse_allsvenskan_markdown(input: &str, config: &AppConfig) -> Vec<WatchOverlay> {
    let title_re =
        Regex::new(r#"(?P<title>(?:Studio:\s+)?[A-ZÅÄÖ][^•\[]+?\s-\s[^•\[]+?)\s+Allsvenskan"#)
            .unwrap();
    let mut overlays = Vec::new();

    for line in input
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with('['))
    {
        if !line.contains("tv4play.se/program/") || !line.contains("Allsvenskan") {
            continue;
        }

        let url = extract_url(line);
        let label = extract_label(line, &url);

        let Some(caps) = title_re.captures(&label) else {
            continue;
        };
        let title = clean_title(caps.name("title").unwrap().as_str());
        if title.starts_with("Studio:") {
            continue;
        }
        let Some((home, away)) = title.split_once(" - ") else {
            continue;
        };

        let home = config.canonical_team_name("allsvenskan", home.trim());
        let away = config.canonical_team_name("allsvenskan", away.trim());
        let title = format!("{home} - {away}");

        overlays.push(WatchOverlay {
            competition: "allsvenskan".into(),
            market: "se".into(),
            provider_family: "tv4".into(),
            provider_label: "TV4 Play".into(),
            title: title.clone(),
            participants: Participants { home, away },
            channel_name: Some(title),
            watch_type: "ppv-event".into(),
            confidence: 0.99,
            source: "tv4play-listing".into(),
            source_url: url,
        });
    }

    overlays
}

fn parse_hockey_markdown(
    input: &str,
    config: &AppConfig,
    competition: &str,
    competition_label: &str,
    default_source_url: &str,
) -> Vec<WatchOverlay> {
    let teams = config.team_names_for_competition(competition);
    let mut overlays = Vec::new();

    for line in input
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with('['))
    {
        if !line.contains("tv4play.se/program/") || !line.contains(competition_label) {
            continue;
        }

        let url = extract_url(line);
        let label = extract_label(line, &url);
        let Some(prefix) = label
            .split_once(competition_label)
            .map(|(prefix, _)| prefix)
        else {
            continue;
        };
        let Some((home, away)) = extract_matchup(prefix, &teams) else {
            continue;
        };

        let home = config.canonical_team_name(competition, &home);
        let away = config.canonical_team_name(competition, &away);
        let title = format!("{home} - {away}");

        overlays.push(WatchOverlay {
            competition: competition.into(),
            market: "se".into(),
            provider_family: "tv4".into(),
            provider_label: "TV4 Play".into(),
            title: title.clone(),
            participants: Participants { home, away },
            channel_name: Some(title),
            watch_type: "ppv-event".into(),
            confidence: 0.99,
            source: "tv4play-listing".into(),
            source_url: if url.is_empty() {
                default_source_url.into()
            } else {
                url
            },
        });
    }

    overlays
}

fn parse_category_html(
    input: &str,
    config: &AppConfig,
    competition: &str,
    competition_label: &str,
    default_source_url: &str,
) -> Vec<WatchOverlay> {
    let document = Html::parse_document(input);
    let selector = Selector::parse("a").unwrap();
    let mut lines = Vec::new();

    for link in document.select(&selector) {
        let Some(href) = link.value().attr("href") else {
            continue;
        };
        if !href.contains("/program/") {
            continue;
        }
        let text = link.text().collect::<Vec<_>>().join(" ");
        let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if !normalized.contains(competition_label) {
            continue;
        }
        let absolute = if href.starts_with("http") {
            href.to_string()
        } else {
            format!("https://www.tv4play.se{href}")
        };
        lines.push(format!("[{normalized}]({absolute})"));
    }

    if competition == "allsvenskan" {
        parse_allsvenskan_markdown(&lines.join("\n"), config)
    } else {
        parse_hockey_markdown(
            &lines.join("\n"),
            config,
            competition,
            competition_label,
            default_source_url,
        )
    }
}

fn extract_matchup(label_prefix: &str, teams: &[String]) -> Option<(String, String)> {
    let normalized = label_prefix.replace('•', " ");
    let mut best: Option<(usize, usize, String, String)> = None;

    for home in teams {
        let needle = format!("{home} - ");
        let Some(start) = normalized.rfind(&needle) else {
            continue;
        };
        let tail = normalized[start + needle.len()..].trim();

        for away in teams {
            if !tail.starts_with(away) {
                continue;
            }
            let remainder = tail[away.len()..].chars().next();
            if remainder.is_some_and(|ch| !ch.is_whitespace()) {
                continue;
            }
            let score = start + home.len() + away.len();
            match &best {
                Some((best_score, _, _, _)) if *best_score >= score => {}
                _ => best = Some((score, start, home.clone(), away.clone())),
            }
        }
    }

    best.map(|(_, _, home, away)| (home, away))
}

fn extract_url(line: &str) -> String {
    line.rsplit_once('(')
        .and_then(|(_, tail)| tail.strip_suffix(')'))
        .unwrap_or_default()
        .to_string()
}

fn extract_label(line: &str, url: &str) -> String {
    line.split_once(") ")
        .map(|(_, rest)| rest.trim_end_matches(&format!("]({url})")))
        .unwrap_or(line)
        .to_string()
}

fn clean_title(raw: &str) -> String {
    let mut value = raw.trim().to_string();
    if let Some((_, tail)) = value.rsplit_once(" Imorgon ") {
        value = Regex::new(r#"^\d{1,2}(?::|\s)\d{2}\s+"#)
            .unwrap()
            .replace(tail, "")
            .to_string();
    }
    if let Some((_, tail)) = value.rsplit_once(" Live ") {
        value = tail.to_string();
    }
    value.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tv4_allsvenskan_listings() {
        let input = include_str!("../../tests/fixtures/tv4_allsvenskan_readability.md");
        let config = AppConfig::load(
            "config/providers.yaml",
            "config/competition_rules.yaml",
            "config/sample_events.yaml",
            "config/sources.yaml",
            "config/team_aliases.yaml",
        )
        .unwrap();
        let overlays = parse_allsvenskan_markdown(input, &config);
        assert!(overlays
            .iter()
            .any(|overlay| overlay.title == "Hammarby IF - Örgryte IS"));
        assert!(overlays
            .iter()
            .all(|overlay| !overlay.title.starts_with("Studio:")));
    }

    #[test]
    fn parses_tv4_shl_listings() {
        let input = include_str!("../../tests/fixtures/tv4_shl_readability.md");
        let config = AppConfig::load(
            "config/providers.yaml",
            "config/competition_rules.yaml",
            "config/sample_events.yaml",
            "config/sources.yaml",
            "config/team_aliases.yaml",
        )
        .unwrap();
        let overlays = parse_shl_document(input, &config);
        assert!(overlays
            .iter()
            .any(|overlay| overlay.title == "Skellefteå AIK - Rögle BK"));
        assert!(overlays
            .iter()
            .any(|overlay| overlay.title == "Skellefteå AIK - Luleå Hockey"));
    }

    #[test]
    fn parses_tv4_hockeyallsvenskan_listings() {
        let input = include_str!("../../tests/fixtures/tv4_hockeyallsvenskan_readability.md");
        let config = AppConfig::load(
            "config/providers.yaml",
            "config/competition_rules.yaml",
            "config/sample_events.yaml",
            "config/sources.yaml",
            "config/team_aliases.yaml",
        )
        .unwrap();
        let overlays = parse_hockeyallsvenskan_document(input, &config);
        assert!(overlays
            .iter()
            .any(|overlay| overlay.title == "Björklöven - BIK Karlskoga"));
        assert!(overlays
            .iter()
            .any(|overlay| overlay.title == "BIK Karlskoga - Björklöven"));
    }
}
