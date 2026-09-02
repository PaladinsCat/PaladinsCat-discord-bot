//! Champion name autocomplete — replaces championAutocompleteChoices() from commands.ts
//! refs: none

use unicode_normalization::UnicodeNormalization;

use crate::api::ApiClient;

const SCORE_EXACT: u8 = 0;
const SCORE_STARTS: u8 = 1;
const SCORE_WORD_STARTS: u8 = 2;
const SCORE_CONTAINS: u8 = 3;
const MAX_CHOICES: usize = 25;
const CHOICE_LEN: usize = 100;

/// Simple in-memory champion list holder.
/// (External caching via `cache::RenderCache` handles HTTP deduplication.)
/// refs: none
#[allow(dead_code)] // Kept for potential future autocomplete optimization
/// Define ChampionList.
///
/// Contract: accepts the arguments shown in the signature and returns the documented result; side effects follow the implementation.
///
/// refs: none
pub struct ChampionList {
    names: Option<Vec<String>>,
}

#[allow(dead_code)] // Kept for potential future autocomplete optimization
impl ChampionList {
    /// Create an empty champion-name cache.
    ///
    /// I/O: () -> `ChampionList`
/// refs: none
    pub fn new() -> Self {
        Self { names: None }
    }

    /// Return the cached champion names, fetching from the API on first use.
    ///
    /// I/O: `&ApiClient` -> `&[String]`
/// refs: none
    pub async fn get(&mut self, api: &ApiClient) -> &[String] {
        if self.names.is_none() {
            self.names = api.champion_names().await.ok();
        }
        self.names
            .as_ref()
            .map(|v| v.as_slice())
            .unwrap_or_default()
    }
}

fn normalize(input: &str) -> String {
    input
        .nfkd()
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_ascii_alphanumeric())
        .collect()
}

fn score(name: &str, query: &str) -> u8 {
    let key = normalize(name);
    if query.is_empty() {
        return 4;
    }
    if key == query {
        return SCORE_EXACT;
    }
    if key.starts_with(query) {
        return SCORE_STARTS;
    }
    let words: Vec<String> = name
        .split(|c: char| !c.is_alphanumeric())
        .map(normalize)
        .filter(|s| !s.is_empty())
        .collect();
    for word in &words {
        if word.starts_with(query) {
            return SCORE_WORD_STARTS;
        }
    }
    if key.contains(query) {
        return SCORE_CONTAINS;
    }
    u8::MAX
}

/// Filter champion names into Discord autocomplete choices for a query.
///
/// I/O: `&[String]` (names), `&str` (query) -> `Vec<(String, String)>`
/// refs: none
pub fn champion_autocomplete_choices(names: &[String], query: &str) -> Vec<(String, String)> {
    let query = normalize(query);
    let mut seen = std::collections::HashMap::new();
    for name in names {
        let key = normalize(name);
        seen.entry(key).or_insert_with(|| name.clone());
    }

    let mut scored: Vec<(String, u8)> = seen
        .into_values()
        .filter_map(|name| {
            let s = score(&name, &query);
            (s != u8::MAX).then_some((name.clone(), s))
        })
        .collect();

    scored.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    scored
        .iter()
        .take(MAX_CHOICES)
        .map(|(name, _)| {
            let capped: String = name.chars().take(CHOICE_LEN).collect();
            (capped.clone(), capped)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_basic() {
        assert_eq!(normalize("Akali"), "akali");
        assert_eq!(normalize("123ABC"), "123abc");
    }

    #[test]
    fn exact_match_score_zero() {
        assert_eq!(score("Akali", "akali"), SCORE_EXACT);
    }

    #[test]
    fn starts_with_score() {
        assert_eq!(score("Akali", "aka"), SCORE_STARTS);
    }

    #[test]
    fn choice_name_length_capped() {
        let long_name: String = "A".repeat(200);
        let names = vec![long_name];
        let result = champion_autocomplete_choices(&names, "a");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0.len(), CHOICE_LEN);
        assert_eq!(result[0].1.len(), CHOICE_LEN);
    }

    #[test]
    fn empty_query_returns_alphabetical_catalog_like_ts() {
        let names = vec!["Zhin".into(), "Androxus".into()];
        let result = champion_autocomplete_choices(&names, "");
        assert_eq!(
            result,
            vec![
                ("Androxus".into(), "Androxus".into()),
                ("Zhin".into(), "Zhin".into())
            ]
        );
    }

    #[test]
    fn word_prefix_uses_original_name_boundaries() {
        let names = vec!["Mal'Damba".into()];
        assert_eq!(champion_autocomplete_choices(&names, "dam").len(), 1);
    }
}
