use std::cmp::Reverse;

use crate::{config::AppConfig, domain::*};

const MARKET_WEIGHT: &[(&str, i32)] = &[("se", 300), ("us", 200), ("uk", 100)];

pub fn hydrate_event(seed: &EventSeed, config: &AppConfig) -> Event {
    let availabilities = build_availabilities(seed, config);
    let recommended = availabilities.first().cloned();

    Event {
        id: seed.id.clone(),
        sport: seed.sport.clone(),
        competition: seed.competition.clone(),
        title: seed.title.clone(),
        start_time: seed.start_time,
        end_time: seed.end_time,
        status: seed.status.clone(),
        venue: seed.venue.clone(),
        round_label: seed.round_label.clone(),
        participants: seed.participants.clone(),
        source: seed.source.clone(),
        source_url: seed.source_url.clone(),
        watch: EventWatch {
            recommended_market: recommended.as_ref().map(|a| a.market.clone()),
            recommended_provider: recommended.as_ref().map(|a| a.provider_label.clone()),
            availabilities,
        },
        search_metadata: SearchMetadata {
            queries: base_queries(seed),
            keywords: keywords(seed),
        },
    }
}

fn build_availabilities(seed: &EventSeed, config: &AppConfig) -> Vec<WatchAvailability> {
    let mut availabilities = config
        .rules
        .iter()
        .filter(|rule| rule.competition == seed.competition)
        .filter_map(|rule| {
            let provider = config
                .provider_lookup
                .get(&(rule.provider_family.clone(), rule.market.clone()))?;
            let provider_label = provider
                .aliases
                .first()
                .cloned()
                .unwrap_or_else(|| provider.family.clone());
            let channel_name = preferred_channel(seed, provider, &rule.watch_type);
            let search_hints =
                provider_search_hints(seed, &provider_label, channel_name.as_deref());
            let priority = availability_priority(&rule.market, &rule.watch_type, rule.confidence);

            Some(WatchAvailability {
                market: rule.market.clone(),
                provider_family: rule.provider_family.clone(),
                provider_label,
                channel_name,
                watch_type: rule.watch_type.clone(),
                priority,
                confidence: rule.confidence,
                source: "competition-rule".to_string(),
                search_hints,
            })
        })
        .collect::<Vec<_>>();

    availabilities.extend(
        config
            .watch_overlays
            .iter()
            .filter(|overlay| overlay_matches_event(overlay, seed))
            .map(|overlay| {
                let search_hints = provider_search_hints(
                    seed,
                    &overlay.provider_label,
                    overlay.channel_name.as_deref(),
                );
                WatchAvailability {
                    market: overlay.market.clone(),
                    provider_family: overlay.provider_family.clone(),
                    provider_label: overlay.provider_label.clone(),
                    channel_name: overlay.channel_name.clone(),
                    watch_type: overlay.watch_type.clone(),
                    priority: availability_priority(
                        &overlay.market,
                        &overlay.watch_type,
                        overlay.confidence,
                    ) + 40,
                    confidence: overlay.confidence,
                    source: overlay.source.clone(),
                    search_hints,
                }
            }),
    );

    availabilities.sort_by_key(|a| {
        (
            Reverse(a.priority),
            Reverse((a.confidence * 100.0) as i32),
            a.provider_label.clone(),
        )
    });
    availabilities.dedup_by(|a, b| {
        a.market == b.market
            && a.provider_family == b.provider_family
            && a.channel_name == b.channel_name
    });
    availabilities
}

fn availability_priority(market: &str, watch_type: &str, confidence: f32) -> i32 {
    market_weight(market) + watch_type_weight(watch_type) + (confidence * 100.0).round() as i32
}

fn market_weight(market: &str) -> i32 {
    MARKET_WEIGHT
        .iter()
        .find_map(|(candidate, weight)| (*candidate == market).then_some(*weight))
        .unwrap_or_default()
}

fn watch_type_weight(watch_type: &str) -> i32 {
    match watch_type {
        "ppv-event" => 50,
        "streaming" => 40,
        "streaming+linear" => 35,
        "linear-tv" => 25,
        "studio" => -20,
        _ => 10,
    }
}

fn preferred_channel(
    seed: &EventSeed,
    provider: &ProviderCatalogEntry,
    watch_type: &str,
) -> Option<String> {
    let label = provider.aliases.first()?.clone();
    if seed.sport == "golf" {
        return Some(format!(
            "{} {}",
            label,
            seed.round_label
                .clone()
                .unwrap_or_else(|| "Main Feed".into())
        ));
    }

    if watch_type == "ppv-event" {
        return Some(seed.title.clone());
    }

    match (provider.family.as_str(), seed.sport.as_str()) {
        ("tv4", "soccer") => Some("TV4 Fotboll".into()),
        ("tv4", "hockey") => Some("TV4 Hockey".into()),
        ("viaplay", "soccer") => Some("V Sport Football".into()),
        ("viaplay", "motorsport") => Some("Viaplay".into()),
        ("sky", "soccer") => Some("Sky Sports Premier League".into()),
        ("sky", "golf") => Some("Sky Sports Golf".into()),
        ("peacock", "soccer") => Some("Peacock".into()),
        ("peacock", "golf") => Some("Golf Channel".into()),
        ("paramount", "soccer") => Some("Paramount+".into()),
        ("tnt", "soccer") => Some("TNT Sports 1".into()),
        ("fox", "soccer") => Some("FOX Sports".into()),
        ("svt", "soccer") => Some("SVT Play".into()),
        _ => Some(label),
    }
}

fn provider_search_hints(
    seed: &EventSeed,
    provider_label: &str,
    channel_name: Option<&str>,
) -> Vec<String> {
    let mut hints = vec![format!("{} {}", seed.title, provider_label)];

    if is_field_event(seed) {
        hints.push(format!("{} {}", readable_competition(&seed.competition), seed.title));
        if seed.competition == "formula_1" {
            hints.push(format!("Formula 1 {}", seed.title));
            hints.push(format!("F1 {} {}", seed.title, provider_label));
        }
    } else {
        hints.push(format!("{} {}", seed.participants.home, seed.participants.away));
        hints.push(format!(
            "{} {} {}",
            readable_competition(&seed.competition),
            seed.participants.home,
            seed.participants.away
        ));
    }

    if seed.sport == "golf" {
        hints.push(format!(
            "{} {} {}",
            seed.title,
            seed.round_label.clone().unwrap_or_default(),
            provider_label
        ));
        hints.push(format!("{} main feed {}", seed.title, provider_label));
    }

    if let Some(channel_name) = channel_name {
        hints.push(format!("{} {}", channel_name, seed.title));
    }

    hints.sort();
    hints.dedup();
    hints
}

fn base_queries(seed: &EventSeed) -> Vec<String> {
    let mut queries = vec![seed.title.clone()];

    if is_field_event(seed) {
        queries.push(format!("{} {}", readable_competition(&seed.competition), seed.title));
        if seed.competition == "formula_1" {
            queries.push(format!("Formula 1 {}", seed.title));
            queries.push(format!("F1 {}", seed.title));
        }
    } else {
        queries.push(format!("{} {}", seed.participants.home, seed.participants.away));
        queries.push(format!(
            "{} {} {}",
            readable_competition(&seed.competition),
            seed.participants.home,
            seed.participants.away
        ));
    }

    if seed.sport == "golf" {
        queries.push(format!(
            "{} {}",
            seed.title,
            seed.round_label.clone().unwrap_or_default()
        ));
    }

    queries.sort();
    queries.dedup();
    queries
}

fn keywords(seed: &EventSeed) -> Vec<String> {
    let mut keywords = vec![seed.sport.clone(), seed.competition.clone(), seed.title.clone()];

    if is_field_event(seed) {
        if seed.competition == "formula_1" {
            keywords.push("f1".into());
            keywords.push("formula 1".into());
        }
        keywords.push(seed.participants.home.clone());
    } else {
        keywords.push(seed.participants.home.clone());
        keywords.push(seed.participants.away.clone());
    }

    if let Some(round) = &seed.round_label {
        keywords.push(round.clone());
    }

    keywords.sort();
    keywords.dedup();
    keywords
}

fn is_field_event(seed: &EventSeed) -> bool {
    seed.participants.away == "Field"
}

fn overlay_matches_event(overlay: &WatchOverlay, seed: &EventSeed) -> bool {
    let participant_match = overlay.competition == seed.competition
        && normalize_name(&overlay.participants.home) == normalize_name(&seed.participants.home)
        && normalize_name(&overlay.participants.away) == normalize_name(&seed.participants.away);

    if !participant_match {
        return false;
    }

    if seed.sport == "golf" {
        return normalize_name(&overlay.title) == normalize_name(&seed.title)
            || overlay
                .channel_name
                .as_ref()
                .map(|name| normalize_name(name) == normalize_name(&seed.title))
                .unwrap_or(false)
            || overlay
                .title
                .contains(seed.round_label.as_deref().unwrap_or_default());
    }

    true
}

fn normalize_name(value: &str) -> String {
    value
        .to_lowercase()
        .replace("if", "")
        .replace('å', "a")
        .replace('ä', "a")
        .replace('ö', "o")
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .collect()
}

fn readable_competition(slug: &str) -> String {
    slug.split('_')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

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
    fn sweden_provider_is_ranked_first_for_allsvenskan() {
        let config = config();
        let seed = config
            .events
            .iter()
            .find(|event| event.competition == "allsvenskan")
            .unwrap();
        let event = hydrate_event(seed, &config);
        assert_eq!(event.watch.recommended_market.as_deref(), Some("se"));
        assert_eq!(
            event.watch.recommended_provider.as_deref(),
            Some("TV4 Play")
        );
    }

    #[test]
    fn sweden_market_beats_other_markets_when_available() {
        let config = config();
        let seed = config
            .events
            .iter()
            .find(|event| event.competition == "uefa_champions_league")
            .unwrap();
        let event = hydrate_event(seed, &config);
        assert_eq!(event.watch.recommended_market.as_deref(), Some("se"));
        assert_eq!(event.watch.recommended_provider.as_deref(), Some("Viaplay"));
    }
}
