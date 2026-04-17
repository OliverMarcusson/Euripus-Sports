use regex::Regex;

use crate::{
    config::AppConfig,
    domain::{Participants, WatchOverlay},
};

pub fn parse_premier_league_document(input: &str, config: &AppConfig) -> Vec<WatchOverlay> {
    let match_re = Regex::new(
        r"^(?:PL-studion:\s+)?(?P<home>[\p{L}][\p{L}'&\- ]+)\s+-\s+(?P<away>[\p{L}][\p{L}'&\- ]+)$",
    )
    .unwrap();
    let mut overlays = Vec::new();

    for line in input.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let Some(caps) = match_re.captures(line) else {
            continue;
        };
        let home = config
            .canonical_team_name("premier_league", caps.name("home").unwrap().as_str().trim());
        let away = config
            .canonical_team_name("premier_league", caps.name("away").unwrap().as_str().trim());
        let title = format!("{} - {}", home, away);
        let is_studio = line.starts_with("PL-studion:");

        overlays.push(build_overlay(
            "premier_league",
            &home,
            &away,
            title,
            is_studio,
            "https://viaplay.se/sport/fotboll/premier-league",
        ));
    }

    overlays
}

pub fn parse_champions_league_document(input: &str, config: &AppConfig) -> Vec<WatchOverlay> {
    let match_re = Regex::new(r"^(?:CL-studion:\s+)?(?P<home>[\p{L}][\p{L}'&\- ]+)\s+-\s+(?P<away>[\p{L}][\p{L}'&\- ]+)(?:,|$)").unwrap();
    let mut overlays = Vec::new();

    for line in input.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let Some(caps) = match_re.captures(line) else {
            continue;
        };
        let home = config.canonical_team_name(
            "uefa_champions_league",
            caps.name("home").unwrap().as_str().trim(),
        );
        let away = config.canonical_team_name(
            "uefa_champions_league",
            caps.name("away").unwrap().as_str().trim(),
        );
        let title = format!("{} - {}", home, away);
        let is_studio = line.starts_with("CL-studion:");
        overlays.push(build_overlay(
            "uefa_champions_league",
            &home,
            &away,
            title,
            is_studio,
            "https://viaplay.se/sport/fotboll/uefa-champions-league",
        ));
    }

    overlays
}

fn build_overlay(
    competition: &str,
    home: &str,
    away: &str,
    title: String,
    is_studio: bool,
    source_url: &str,
) -> WatchOverlay {
    let studio_prefix = match competition {
        "uefa_champions_league" => "CL-studion",
        _ => "PL-studion",
    };

    WatchOverlay {
        competition: competition.into(),
        market: "se".into(),
        provider_family: "viaplay".into(),
        provider_label: "Viaplay".into(),
        title: title.clone(),
        participants: Participants {
            home: home.into(),
            away: away.into(),
        },
        channel_name: Some(if is_studio {
            format!("{studio_prefix}: {title}")
        } else {
            title.clone()
        }),
        watch_type: if is_studio {
            "studio".into()
        } else {
            "ppv-event".into()
        },
        confidence: if is_studio { 0.96 } else { 0.99 },
        source: "viaplay-listing".into(),
        source_url: source_url.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_viaplay_premier_league_listings() {
        let input = include_str!("../../tests/fixtures/viaplay_premier_league_listings.md");
        let config = AppConfig::load(
            "config/providers.yaml",
            "config/competition_rules.yaml",
            "config/sample_events.yaml",
            "config/sources.yaml",
            "config/team_aliases.yaml",
        )
        .unwrap();
        let overlays = parse_premier_league_document(input, &config);
        assert!(overlays
            .iter()
            .any(|overlay| overlay.title == "AFC Bournemouth - Leeds United"));
        assert!(overlays
            .iter()
            .any(|overlay| overlay.watch_type == "studio"));
    }

    #[test]
    fn parses_viaplay_champions_league_listing() {
        let input = include_str!("../../tests/fixtures/viaplay_champions_league_index.md");
        let config = AppConfig::load(
            "config/providers.yaml",
            "config/competition_rules.yaml",
            "config/sample_events.yaml",
            "config/sources.yaml",
            "config/team_aliases.yaml",
        )
        .unwrap();
        let overlays = parse_champions_league_document(input, &config);
        assert!(overlays.iter().any(|overlay| overlay.title
            == "Paris Saint-Germain - Bayern Munich"
            && overlay.watch_type == "ppv-event"));
        assert!(overlays.iter().any(|overlay| overlay.title
            == "Paris Saint-Germain - Bayern Munich"
            && overlay.watch_type == "studio"));
    }
}
