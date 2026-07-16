use crate::{
    config::AppConfig,
    domain::EventSeed,
    sources::svenskfotboll_league::{self, LeagueConfig},
};

const LEAGUE: LeagueConfig<'static> = LeagueConfig {
    competition: "damallsvenskan",
    base_url: "www.obosdamallsvenskan.se",
    source_prefix: "damallsvenskan",
};

pub fn parse_document_at(
    input: &str,
    season: i32,
    config: &AppConfig,
    observed_at: time::OffsetDateTime,
) -> Vec<EventSeed> {
    svenskfotboll_league::parse_document_at(input, season, config, LEAGUE, observed_at)
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
        let events = parse_document_at(
            input,
            2026,
            &config,
            time::macros::datetime!(2026-01-01 00:00 UTC),
        );
        assert_eq!(events.len(), 2);
        assert!(events.iter().any(|event| {
            event.title == "Hammarby IF vs BK Häcken"
                && event.round_label.as_deref() == Some("Round 3")
        }));
    }
}
