use crate::{
    config::AppConfig,
    domain::EventSeed,
    sources::svenskfotboll_league::{self, LeagueConfig},
};

const LEAGUE: LeagueConfig<'static> = LeagueConfig {
    competition: "damallsvenskan",
    base_url: "www.obosdamallsvenskan.se",
    source_prefix: "damallsvenskan",
    article_source_url: Some("https://www.obosdamallsvenskan.se/"),
};

pub fn parse_document(input: &str, season: i32, config: &AppConfig) -> Vec<EventSeed> {
    svenskfotboll_league::parse_document(input, season, config, LEAGUE)
}

pub fn parse_svenskfotboll_article(input: &str, season: i32, config: &AppConfig) -> Vec<EventSeed> {
    svenskfotboll_league::parse_svenskfotboll_article(input, season, config, LEAGUE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_damallsvenskan_graphql_fixture() {
        let input = include_str!("../../tests/fixtures/damallsvenskan_graphql.json");
        let config = AppConfig::load(
            "config/providers.yaml",
            "config/competition_rules.yaml",
            "config/sample_events.yaml",
            "config/sources.yaml",
            "config/team_aliases.yaml",
        )
        .unwrap();
        let events = parse_document(input, 2026, &config);
        assert_eq!(events.len(), 2);
        assert!(events.iter().any(|event| {
            event.title == "Hammarby IF vs BK Häcken"
                && event.round_label.as_deref() == Some("Round 3")
        }));
    }
}
