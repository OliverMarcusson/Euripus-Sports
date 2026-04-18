use std::{collections::HashMap, fs};

use anyhow::Context;
use serde::Deserialize;

use crate::{
    domain::{CompetitionRule, EventSeed, ProviderCatalogEntry, WatchOverlay},
    ingest::SourceFetchMode,
};

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub providers: Vec<ProviderCatalogEntry>,
    pub provider_lookup: HashMap<(String, String), ProviderCatalogEntry>,
    pub rules: Vec<CompetitionRule>,
    pub events: Vec<EventSeed>,
    pub watch_overlays: Vec<WatchOverlay>,
    pub sources: Vec<SourceDefinition>,
    pub team_aliases: Vec<TeamAlias>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SourceDefinition {
    pub name: String,
    pub competition: String,
    pub kind: SourceKind,
    pub parser: ParserKind,
    pub url: String,
    pub fixture_path: Option<String>,
    pub fetch_mode: SourceFetchMode,
    #[serde(default)]
    pub request_method: SourceRequestMethod,
    pub request_body: Option<String>,
    pub season: Option<i32>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Event,
    Watch,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SourceRequestMethod {
    #[default]
    Get,
    Post,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ParserKind {
    Allsvenskan,
    Tv4playAllsvenskan,
    Tv4playShl,
    Tv4playHockeyallsvenskan,
    PgaTourSchedule,
    PgaTourBroadcastEvents,
    PgaTourBroadcastWatch,
    PgaTourSvenskGolfWatch,
    Formula1RaceTimes,
    PremierLeagueBbc,
    ViaplayPremierLeague,
    ViaplayChampionsLeague,
    ChampionsLeagueBbc,
    FifaWorldCupFifa,
    Shl,
    Hockeyallsvenskan,
    Elitserien,
    Superettan,
    SuperettanSvenskfotboll,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TeamAlias {
    pub competition: String,
    pub canonical: String,
    pub aliases: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ProvidersFile {
    providers: Vec<ProviderCatalogEntry>,
}

#[derive(Debug, Deserialize)]
struct RulesFile {
    rules: Vec<CompetitionRule>,
}

#[derive(Debug, Deserialize)]
struct EventsFile {
    events: Vec<EventSeed>,
}

#[derive(Debug, Deserialize)]
struct SourcesFile {
    sources: Vec<SourceDefinition>,
}

#[derive(Debug, Deserialize)]
struct TeamAliasesFile {
    teams: Vec<TeamAlias>,
}

impl AppConfig {
    pub fn load(
        providers_path: &str,
        rules_path: &str,
        events_path: &str,
        sources_path: &str,
        team_aliases_path: &str,
    ) -> anyhow::Result<Self> {
        let providers: ProvidersFile = serde_yaml::from_str(
            &fs::read_to_string(providers_path)
                .with_context(|| format!("reading {providers_path}"))?,
        )
        .with_context(|| format!("parsing {providers_path}"))?;

        let rules: RulesFile = serde_yaml::from_str(
            &fs::read_to_string(rules_path).with_context(|| format!("reading {rules_path}"))?,
        )
        .with_context(|| format!("parsing {rules_path}"))?;

        let events: EventsFile = serde_yaml::from_str(
            &fs::read_to_string(events_path).with_context(|| format!("reading {events_path}"))?,
        )
        .with_context(|| format!("parsing {events_path}"))?;

        let sources: SourcesFile = serde_yaml::from_str(
            &fs::read_to_string(sources_path).with_context(|| format!("reading {sources_path}"))?,
        )
        .with_context(|| format!("parsing {sources_path}"))?;

        let team_aliases: TeamAliasesFile = serde_yaml::from_str(
            &fs::read_to_string(team_aliases_path)
                .with_context(|| format!("reading {team_aliases_path}"))?,
        )
        .with_context(|| format!("parsing {team_aliases_path}"))?;

        let provider_lookup = providers
            .providers
            .iter()
            .cloned()
            .map(|provider| ((provider.family.clone(), provider.market.clone()), provider))
            .collect();

        Ok(Self {
            providers: providers.providers,
            provider_lookup,
            rules: rules.rules,
            events: events.events,
            watch_overlays: Vec::new(),
            sources: sources.sources,
            team_aliases: team_aliases.teams,
        })
    }

    pub fn with_source_data(
        mut self,
        events: Vec<EventSeed>,
        watch_overlays: Vec<WatchOverlay>,
    ) -> Self {
        if !events.is_empty() {
            let competitions = events
                .iter()
                .map(|event| event.competition.clone())
                .collect::<std::collections::HashSet<_>>();
            self.events
                .retain(|event| !competitions.contains(&event.competition));
            let mut seen_ids = std::collections::HashSet::new();
            self.events.extend(
                events
                    .into_iter()
                    .filter(|event| seen_ids.insert(event.id.clone())),
            );
        }
        self.watch_overlays = watch_overlays;
        self
    }

    pub fn canonical_team_name(&self, competition: &str, raw: &str) -> String {
        self.team_aliases
            .iter()
            .find(|team| {
                team.competition == competition
                    && team.aliases.iter().any(|alias| alias == raw.trim())
            })
            .map(|team| team.canonical.clone())
            .unwrap_or_else(|| raw.trim().to_string())
    }

    pub fn team_names_for_competition(&self, competition: &str) -> Vec<String> {
        let mut teams = self
            .team_aliases
            .iter()
            .filter(|team| team.competition == competition)
            .flat_map(|team| team.aliases.clone())
            .collect::<Vec<_>>();
        teams.sort_by_key(|name| std::cmp::Reverse(name.len()));
        teams.dedup();
        teams
    }
}
