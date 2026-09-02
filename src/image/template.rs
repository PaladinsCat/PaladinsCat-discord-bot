//! HTML template data binding — builds data-bound scoreboard/loadout documents.
//! refs: none

use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};

use base64::Engine as _;

use super::asset_catalog::AssetCatalog;

static ASSET_DATA_URL_CACHE: OnceLock<RwLock<HashMap<PathBuf, String>>> = OnceLock::new();
const PLAYER_TAG_MINIMUM_COUNT: i64 = 5;

#[derive(Debug, Clone)]
/// Define TemplateConfig.
///
/// Contract: accepts the arguments shown in the signature and returns the documented result; side effects follow the implementation.
///
/// refs: none
pub struct TemplateConfig {
    pub match_template_path: String,
    pub canonical_match_css_path: Option<String>,
    pub loadout_template_path: String,
    pub cheater_pattern_path: String,
    pub asset_root_path: String,
}

impl TemplateConfig {
    /// Default template config for development.
    ///
    /// I/O: () -> `TemplateConfig`
/// refs: none
    pub fn dev_defaults() -> Self {
        let workspace_css = "../paladinscat-frontend/app/globals.css";
        Self {
            match_template_path: "assets/templates/match-result-scoreboard.html".into(),
            canonical_match_css_path: Some(
                if std::path::Path::new(workspace_css).is_file() {
                    workspace_css
                } else {
                    "src/frontend/app/globals.css"
                }
                .into(),
            ),
            loadout_template_path: "assets/templates/loadout-card-layout.html".into(),
            cheater_pattern_path: "assets/templates/cheater-police-line.svg".into(),
            asset_root_path: "src/frontend/public/images".into(),
        }
    }
}

#[derive(Clone)]
/// Define TemplateEngine.
///
/// Contract: accepts the arguments shown in the signature and returns the documented result; side effects follow the implementation.
///
/// refs: none
pub struct TemplateEngine {
    match_template: Arc<String>,
    canonical_match_css: Option<Arc<String>>,
    loadout_template: Arc<String>,
    cheater_pattern_url: String,
    assets: AssetCatalog,
}

// ---------------------------------------------------------------------------
// Scoreboard field helpers — mirror the TS match-renderer.ts formatting.
// ---------------------------------------------------------------------------

fn num(value: Option<&serde_json::Value>) -> i64 {
    match value {
        Some(serde_json::Value::Number(value)) => value
            .as_i64()
            .or_else(|| value.as_f64().map(|v| v as i64))
            .unwrap_or(0),
        Some(serde_json::Value::String(value)) => {
            value.parse::<f64>().map(|v| v as i64).unwrap_or(0)
        }
        Some(serde_json::Value::Bool(value)) => i64::from(*value),
        _ => 0,
    }
}

fn num_f64(value: Option<&serde_json::Value>) -> f64 {
    match value {
        Some(serde_json::Value::Number(value)) => value.as_f64().unwrap_or(0.0),
        Some(serde_json::Value::String(value)) => value.parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

fn str_of(value: Option<&serde_json::Value>) -> String {
    match value {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

fn bool_of(value: Option<&serde_json::Value>) -> bool {
    match value {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(n) => num(Some(n)) != 0,
        None => false,
    }
}

fn asset_mime(path: &std::path::Path, bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return "image/png";
    }
    if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        return "image/webp";
    }
    if bytes.starts_with(b"\xff\xd8\xff") {
        return "image/jpeg";
    }
    if bytes.get(4..8) == Some(b"ftyp")
        && bytes
            .get(8..32)
            .is_some_and(|brands| brands.windows(4).any(|brand| brand == b"avif"))
    {
        return "image/avif";
    }
    let trimmed = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .map(|index| &bytes[index..])
        .unwrap_or(bytes);
    if trimmed.starts_with(b"<svg") || trimmed.starts_with(b"<?xml") {
        return "image/svg+xml";
    }
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

fn compact(value: i64) -> String {
    if value.abs() >= 1000 {
        format!("{:.1}k", value as f64 / 1000.0)
    } else {
        number(value)
    }
}

fn number(value: i64) -> String {
    let negative = value < 0;
    let digits = value.unsigned_abs().to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    if negative {
        format!("-{grouped}")
    } else {
        grouped
    }
}

fn format_scaled_number(value: f64) -> String {
    let rounded = (value * 100.0).round() / 100.0;
    if rounded.fract().abs() < f64::EPSILON {
        return number(rounded as i64);
    }
    let text = format!("{rounded:.2}").trim_end_matches('0').to_string();
    let (whole, decimal) = text.split_once('.').unwrap_or((&text, ""));
    let grouped = number(whole.parse::<i64>().unwrap_or(0));
    format!("{grouped}.{decimal}")
}

fn scale_card_description(description: &str, level: i64) -> String {
    let safe_level = level.clamp(1, 5);
    let mut source = description.trim();
    if source.starts_with('[') {
        if let Some(close) = source.find(']') {
            source = source[close + 1..].trim_start();
        }
    }

    let mut output = String::with_capacity(source.len());
    let mut remainder = source;
    while let Some(open) = remainder.find('{') {
        output.push_str(&remainder[..open]);
        let after_open = &remainder[open + 1..];
        let Some(close) = after_open.find('}') else {
            output.push_str(&remainder[open..]);
            remainder = "";
            break;
        };
        let expression = after_open[..close]
            .strip_prefix("scale=")
            .or_else(|| after_open[..close].strip_prefix("SCALE="))
            .unwrap_or(&after_open[..close]);
        let replacement = expression.split_once('|').and_then(|(base, step)| {
            let base = base.replace(',', "").parse::<f64>().ok()?;
            let step = step.replace(',', "").parse::<f64>().ok()?;
            Some(format_scaled_number(base + step * (safe_level - 1) as f64))
        });
        if let Some(replacement) = replacement {
            output.push_str(&replacement);
        } else {
            output.push_str(&remainder[open..open + close + 2]);
        }
        remainder = &after_open[close + 1..];
    }
    output.push_str(remainder);
    output.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn fallback_queue_name(queue_id: i64) -> &'static str {
    match queue_id {
        424 | 428 | 486 => "Siege",
        437 => "Payload",
        451 => "Survival",
        452 => "Onslaught",
        469 => "Team Deathmatch",
        474 => "Battlegrounds Solo",
        475 => "Battlegrounds Duo",
        476 => "Battlegrounds Quad",
        _ => "Unknown mode",
    }
}

fn clean_queue_mode(label: &str) -> String {
    let trimmed = label.trim();
    for prefix in ["Ranked ", "Casual "] {
        if let Some(mode) = trimmed.strip_prefix(prefix) {
            return mode.trim().to_string();
        }
    }
    trimmed.to_string()
}

fn match_party_numbers(players: &[serde_json::Value]) -> HashMap<String, i64> {
    let mut counts = HashMap::<i64, usize>::new();
    for player in players {
        let id = num(player.get("party_id"));
        if id > 0 {
            *counts.entry(id).or_default() += 1;
        }
    }
    let mut ids: Vec<i64> = counts
        .into_iter()
        .filter_map(|(id, count)| (count > 1).then_some(id))
        .collect();
    ids.sort_unstable();
    let derived: HashMap<i64, i64> = ids
        .into_iter()
        .enumerate()
        .map(|(index, id)| (id, index as i64 + 1))
        .collect();
    players
        .iter()
        .filter_map(|player| {
            let player_id = str_of(player.get("player_id"));
            let raw_party = num(player.get("party_id"));
            let party = if raw_party > 0 {
                derived.get(&raw_party).copied().unwrap_or(0)
            } else {
                let stored = num(player.get("party")).max(num(player.get("party_number")));
                if stored > 0 {
                    stored
                } else {
                    0
                }
            };
            (party > 0 && !player_id.is_empty()).then_some((player_id, party))
        })
        .collect()
}

fn duration(seconds: i64) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn tier_name(tier: i64) -> String {
    const NAMES: [&str; 28] = [
        "Unranked",
        "Bronze V",
        "Bronze IV",
        "Bronze III",
        "Bronze II",
        "Bronze I",
        "Silver V",
        "Silver IV",
        "Silver III",
        "Silver II",
        "Silver I",
        "Gold V",
        "Gold IV",
        "Gold III",
        "Gold II",
        "Gold I",
        "Platinum V",
        "Platinum IV",
        "Platinum III",
        "Platinum II",
        "Platinum I",
        "Diamond V",
        "Diamond IV",
        "Diamond III",
        "Diamond II",
        "Diamond I",
        "Master",
        "Grandmaster",
    ];
    let idx = tier.clamp(0, 27) as usize;
    NAMES[idx].to_string()
}

/// Match the web scoreboard's display rule for the synthetic Grandmaster tier.
/// refs: none
fn player_display_tier(player: &serde_json::Value) -> i64 {
    let value = num_f64(Some(
        player.get("kbm_tier").unwrap_or(
            player.get("tier").unwrap_or(
                player
                    .get("league_tier")
                    .unwrap_or(&serde_json::Value::Null),
            ),
        ),
    )) as i64;
    let value = value.clamp(0, 27);
    let rank = num_f64(
        player.get("kbm_rank").or(player
            .get("profile_snapshot")
            .and_then(|p| p.get("kbm_rank"))),
    );
    if value == 26 && rank.is_finite() && rank >= 1.0 && rank <= 100.0 {
        27
    } else {
        value
    }
}

fn damage(player: &serde_json::Value) -> i64 {
    let physical = num(player.get("damage_done_physical"));
    if physical != 0 {
        physical
    } else {
        num(player.get("damage_done_in_hand"))
    }
}

fn metrics(player: &serde_json::Value) -> [i64; 6] {
    [
        num(player.get("gold_earned")),
        num(player.get("objective_assists")),
        damage(player),
        num(player.get("damage_taken")),
        num(player.get("damage_mitigated")),
        num(player.get("healing")),
    ]
}

impl TemplateEngine {
    /// Load a template engine from a config.
    ///
    /// I/O: `&TemplateConfig` -> `Result<TemplateEngine, Box<dyn Error + Send + Sync>>`
/// refs: none
    pub fn load(config: &TemplateConfig) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let match_template = fs::read_to_string(&config.match_template_path).map_err(|e| {
            format!(
                "Failed to load match template {}: {}",
                config.match_template_path, e
            )
        })?;
        let loadout_template = fs::read_to_string(&config.loadout_template_path).map_err(|e| {
            format!(
                "Failed to load loadout template {}: {}",
                config.loadout_template_path, e
            )
        })?;
        let canonical_match_css = match config.canonical_match_css_path.as_deref() {
            Some(path) => {
                let stylesheet = fs::read_to_string(path).map_err(|error| {
                    format!("Failed to load canonical match CSS {path}: {error}")
                })?;
                Some(Arc::new(
                    Self::extract_browser_scoreboard_css(&stylesheet).ok_or_else(|| {
                        format!("Canonical match CSS {path} has no #browser-scoreboard block")
                    })?,
                ))
            }
            None => None,
        };
        let cheater_pattern_url: String =
            if fs::exists(&config.cheater_pattern_path).unwrap_or(false) {
                let svg = fs::read_to_string(&config.cheater_pattern_path).unwrap_or_default();
                format!("data:image/svg+xml,{}", url_encode(&svg))
            } else {
                String::new()
            };
        Ok(Self {
            match_template: Arc::new(match_template),
            canonical_match_css,
            loadout_template: Arc::new(loadout_template),
            cheater_pattern_url,
            assets: AssetCatalog::new(&config.asset_root_path),
        })
    }

    /// Extract the CSS block from a template document.
    ///
    /// I/O: `&str` (template) -> `String`
/// refs: none
    pub fn extract_css(template: &str) -> String {
        if let Some(idx) = template.find("<style") {
            let rest = &template[idx..];
            if let (Some(open_end), Some(close)) = (rest.find('>'), rest.find("</style>")) {
                if close > open_end {
                    return rest[open_end + 1..close]
                        .lines()
                        .filter(|line| {
                            !(line.contains("@import") && line.contains("fonts.googleapis.com"))
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                }
            }
        }
        String::new()
    }

    fn extract_browser_scoreboard_css(stylesheet: &str) -> Option<String> {
        stylesheet
            .find("#browser-scoreboard {")
            .map(|start| stylesheet[start..].trim().to_string())
    }

    fn asset_url(&self, path: Option<PathBuf>) -> String {
        const TRANSPARENT: &str =
            "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg'/%3E";
        let Some(path) = path else {
            return TRANSPARENT.to_string();
        };
        let cache = ASSET_DATA_URL_CACHE.get_or_init(|| RwLock::new(HashMap::new()));
        if let Ok(guard) = cache.read() {
            if let Some(url) = guard.get(&path) {
                return url.clone();
            }
        }
        let Ok(bytes) = fs::read(&path) else {
            return TRANSPARENT.to_string();
        };
        let mime = asset_mime(&path, &bytes);
        let url = format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        );
        if let Ok(mut guard) = cache.write() {
            guard.insert(path, url.clone());
        }
        url
    }

    /// Build a complete, data-bound scoreboard document with a real
    /// `#scoreboard` element. The static design prototype is not injected
    /// wholesale — we reuse its CSS and build the hero / columns / team rows /
    /// summary from the record JSON.
    ///
    /// I/O: `&Value` (data) -> `String`
/// refs: none
    pub fn match_document(&self, data: &serde_json::Value) -> String {
        let css = match self.canonical_match_css.as_deref() {
            Some(css) => Cow::Borrowed(css.as_str()),
            None => Cow::Owned(Self::extract_css(self.match_template.as_ref())),
        };
        let cheater_css = if self.cheater_pattern_url.is_empty() {
            String::new()
        } else {
            format!(
                "#browser-scoreboard{{--cheater-pattern:url(\"{}\")}}",
                self.cheater_pattern_url
            )
        };
        let map_name = str_of(data.get("match").and_then(|m| m.get("map")));
        let map_url = self.asset_url(self.assets.map_image(&map_name));
        let map_css = format!(
            "#scoreboard::before{{background-image:url('{}')!important}}",
            map_url
        );
        let board = self.scoreboard_markup(data);
        format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"/><style>{css}{map_css}{cheater}\
             html,body{{width:1280px;height:720px;margin:0;overflow:hidden}}body{{padding:0;background:transparent}}#browser-scoreboard{{width:1280px;height:720px}}#browser-scoreboard .scoreboard{{transform:none}}\
             #browser-scoreboard .scoreboard-canvas{{width:1280px;height:720px}}#browser-scoreboard .viewport{{width:1280px;max-width:none}}\
             .prototype-note,.color-lab{{display:none}}.talent-empty{{display:grid;place-items:center;color:#8f9bad;font-size:18px;font-weight:700}}</style>\
             </head><body><section id=\"browser-scoreboard\" data-theme=\"dark\"><main class=\"viewport\"><div class=\"scoreboard-canvas\">\
             <section class=\"scoreboard\" id=\"scoreboard\" aria-label=\"Paladins match scoreboard\">{board}</section>\
             </div></main></section></body></html>",
            cheater = cheater_css,
            map_css = map_css,
        )
    }

    /// Build scoreboard section markup (hero + columns + both teams), mirroring
    /// the TS `document()` inner structure.
/// refs: none
    fn scoreboard_markup(&self, data: &serde_json::Value) -> String {
        let match_obj = data.get("match");
        let players = data
            .get("players")
            .and_then(|p| p.as_array())
            .cloned()
            .unwrap_or_default();
        let queue_id = num(match_obj.and_then(|m| m.get("queue_id")));
        let bans = data
            .get("bans")
            .and_then(|b| b.as_array())
            .cloned()
            .unwrap_or_default();
        let facts = data
            .get("facts")
            .and_then(|f| f.as_array())
            .cloned()
            .unwrap_or_default();

        let mut sorted_bans = bans.clone();
        sorted_bans.sort_by_key(|b| num(b.get("ban_slot")));
        let split = (sorted_bans.len() + 1) / 2;

        let party_numbers = match_party_numbers(&players);
        let hero = self.hero_markup(match_obj, &players, queue_id, &sorted_bans, split);
        let columns = "<div class=\"columns grid-row\"><div>Party</div><div></div><div>Level</div><div>Player</div><div>Elo</div><div>Talent</div><div>Credits</div><div>K / D / A</div><div>OB. Time</div><div>Damage</div><div>Taken</div><div>Shielding</div><div>Healing</div></div>";
        let team_one_rows = self.team_rows(&players, &facts, 1, &party_numbers);
        let team_one_summary = self.team_summary(match_obj, &players, 1);
        let team_two_rows = self.team_rows(&players, &facts, 2, &party_numbers);
        let team_two_summary = self.team_summary(match_obj, &players, 2);

        format!(
            "{hero}{columns}<div class=\"players\" id=\"team-one\">{team_one_rows}</div>\
             {team_one_summary}<div class=\"players\" id=\"team-two\">{team_two_rows}</div>{team_two_summary}"
        )
    }

    fn hero_markup(
        &self,
        match_obj: Option<&serde_json::Value>,
        players: &[serde_json::Value],
        queue_id: i64,
        sorted_bans: &[serde_json::Value],
        split: usize,
    ) -> String {
        let map_name = match_obj
            .map(|m| {
                let s = str_of(m.get("map"));
                // Strip lead-in queue tokens & version suffixes like the TS.
                let no_prefix = regex_strip_prefix(&s);
                no_prefix
            })
            .unwrap_or_default();
        let region = str_of(match_obj.and_then(|m| m.get("region")));
        let ranked = bool_of(match_obj.and_then(|m| m.get("is_ranked"))) || queue_id == 486;
        let custom = bool_of(match_obj.and_then(|m| m.get("is_custom")))
            || str_of(match_obj.and_then(|m| m.get("stats_scope"))) == "custom"
            || str_of(match_obj.and_then(|m| m.get("participant_model"))) == "custom";
        let category = if ranked {
            "Ranked"
        } else if custom {
            "Custom"
        } else {
            "Casual"
        };
        let queue_name = str_of(match_obj.and_then(|m| m.get("queue_name")));
        let mode = if queue_name.trim().is_empty() {
            fallback_queue_name(queue_id).to_string()
        } else {
            clean_queue_mode(&queue_name)
        };
        let map_class = if map_name.len() > 19 {
            "map-name long"
        } else {
            "map-name"
        };

        let broken = bool_of(match_obj.and_then(|m| m.get("broken")));
        let recovered = bool_of(match_obj.and_then(|m| m.get("recovered")));
        let private = bool_of(match_obj.and_then(|m| m.get("private")));
        let limited = bool_of(match_obj.and_then(|m| m.get("limited")));
        let mut status = format!(
            "<span class=\"status-tag {}\">{}</span>",
            if ranked {
                "ranked"
            } else if custom {
                "custom"
            } else {
                "casual"
            },
            category
        );
        if limited {
            status.push_str("<span class=\"status-tag limited\">Limited</span>");
        }
        if broken && !recovered && !limited {
            status.push_str("<span class=\"status-tag broken\">Broken</span>");
        }
        if recovered {
            status.push_str("<span class=\"status-tag recovered\">Recovered</span>");
        }
        if private {
            status.push_str("<span class=\"status-tag private\">Private</span>");
        }

        let score = |value: Option<&serde_json::Value>| match value {
            Some(value) if !value.is_null() => num(Some(value)).to_string(),
            _ => "?".to_string(),
        };
        let score1 = score(match_obj.and_then(|m| m.get("team1_score")));
        let score2 = score(match_obj.and_then(|m| m.get("team2_score")));

        let ban_set = |entries: &[serde_json::Value]| -> String {
            entries
                .iter()
                .take(4)
                .map(|b| {
                    let cn = str_of(b.get("champion_name"));
                    format!(
                        "<span class=\"ban-pick\"><img src=\"{}\" alt=\"{}\"/></span>",
                        self.asset_url(self.assets.champion_icon(&cn)),
                        escape_html(&cn)
                    )
                })
                .collect()
        };
        let ban_markup = if ranked {
            format!(
                "<div class=\"score-bans left\"><span class=\"ban-label\">Bans</span><div class=\"ban-picks\">{}</div></div>",
                ban_set(&sorted_bans[..split.min(sorted_bans.len())])
            )
        } else {
            String::new()
        };
        let right_ban_markup = if ranked && sorted_bans.len() > split {
            format!(
                "<div class=\"score-bans right\"><span class=\"ban-label\">Bans</span><div class=\"ban-picks\">{}</div></div>",
                ban_set(&sorted_bans[split..])
            )
        } else {
            String::new()
        };

        let avg_tier = Self::average_tier(players);
        let tier_name = tier_name(avg_tier);
        let tier_icon = self.asset_url(self.assets.rank_icon(avg_tier as u32));
        let tier_markup = format!(
            "<div class=\"tier-meta\"{}><img src=\"{}\" alt=\"{}\"/><div><div class=\"meta-value\">{}</div><div class=\"meta-label\">Avg tier</div></div></div>",
            if ranked { "" } else { " aria-hidden=\"true\"" },
            tier_icon,
            if ranked { escape_html(&tier_name) } else { String::new() },
            escape_html(&tier_name)
        );
        let brand_icon = self.asset_url(self.assets.icon("paladinscat", None));

        let entry = str_of(match_obj.and_then(|m| m.get("entry_datetime")));
        let timestamp = if entry.is_empty() {
            "—".to_string()
        } else {
            utc_timestamp_str(&entry)
        };
        let dur = duration(num(match_obj.and_then(|m| m.get("duration_seconds"))));
        let match_id = str_of(match_obj.and_then(|m| m.get("match_id")));

        format!(
            "<header class=\"hero{}\"><div class=\"match-identity\"><div class=\"brand-line\">\
             <span class=\"brand-name\"><img src=\"{brand_icon}\" alt=\"\"/> PaladinsCat</span>\
             <div class=\"status-tags\">{status}</div></div>\
             <div class=\"map-line\"><div class=\"{map_class}\" title=\"{m_esc}\">{map_name}</div></div>\
             <div class=\"match-context\"><span>{region}</span><span>{mode}</span></div></div>\
             <div class=\"score{}\">{ban_markup}<span class=\"score-number team-one-score\">{score1}</span>\
             <span class=\"score-separator\">/</span><span class=\"score-number team-two-score\">{score2}</span>{right_ban_markup}</div>\
             <div class=\"match-meta{}\">{tier_markup}<time class=\"timestamp-meta\" datetime=\"{dt}\">{timestamp}</time>\
             <div class=\"duration-meta\"><div class=\"meta-value\">{dur}</div><div class=\"meta-label\">Duration</div></div>\
             <div class=\"match-id-meta\"><div class=\"meta-value\">{match_id}</div><div class=\"meta-label\">Match ID</div></div>\
             </div></header>",
            if ranked { "" } else { " casual" },
            if ranked { "" } else { " casual" },
            if ranked { "" } else { " casual-meta" },
            brand_icon = brand_icon,
            status = status,
            map_class = map_class,
            m_esc = escape_html(&map_name),
            map_name = escape_html(&map_name),
            region = escape_html(&region),
            mode = escape_html(&mode),
            ban_markup = ban_markup,
            score1 = score1,
            score2 = score2,
            right_ban_markup = right_ban_markup,
            tier_markup = tier_markup,
            dt = escape_html(&entry),
            timestamp = timestamp,
            dur = dur,
            match_id = escape_html(&match_id),
        )
    }

    fn average_tier(players: &[serde_json::Value]) -> i64 {
        if players.is_empty() {
            return 0;
        }
        let sum: i64 = players
            .iter()
            .map(|player| {
                num(player
                    .get("kbm_tier")
                    .or_else(|| player.get("tier"))
                    .or_else(|| player.get("league_tier")))
                .clamp(0, 27)
            })
            .sum();
        ((sum as f64) / players.len() as f64).floor() as i64
    }

    fn team_rows(
        &self,
        players: &[serde_json::Value],
        facts: &[serde_json::Value],
        team: i64,
        party_numbers: &HashMap<String, i64>,
    ) -> String {
        let team_players: Vec<&serde_json::Value> = players
            .iter()
            .filter(|p| num(p.get("task_force")) == team)
            .take(5)
            .collect();
        let metrics_all: Vec<[i64; 6]> = team_players.iter().map(|p| metrics(p)).collect();
        let mut row_html = String::new();
        for (player, values) in team_players.iter().zip(metrics_all.iter()) {
            let pid = str_of(player.get("player_id"));
            let fact = facts.iter().find(|f| str_of(f.get("player_id")) == pid);
            let talent = fact
                .and_then(|f| f.get("talents"))
                .and_then(|t| t.as_array())
                .and_then(|a| a.first());
            let talent_name = str_of(talent.and_then(|t| t.get("talent_name")));
            let talent_id = num(talent.and_then(|t| t.get("talent_id"))).max(0) as u32;
            let champion_name = str_of(player.get("champion_name"));

            let peak = |key: usize, require_value: bool| -> String {
                let max = metrics_all.iter().map(|m| m[key]).max().unwrap_or(0).max(0);
                if values[key] == max && (!require_value || values[key] > 0) {
                    " peak".to_string()
                } else {
                    String::new()
                }
            };

            let level = {
                let fml = num(player.get("final_match_level"));
                if fml != 0 {
                    fml
                } else {
                    num(player.get("account_level"))
                }
            };
            let cheater = bool_of(player.get("cheater"))
                || bool_of(
                    player
                        .get("profile_snapshot")
                        .and_then(|p| p.get("cheater")),
                );
            let sus_count = {
                let sc = num(player.get("sus_count"));
                if sc != 0 {
                    sc
                } else {
                    num(player
                        .get("profile_snapshot")
                        .and_then(|p| p.get("sus_count")))
                }
            };
            let suspicious = !cheater && sus_count >= PLAYER_TAG_MINIMUM_COUNT;
            let verified = bool_of(player.get("verified"))
                || bool_of(
                    player
                        .get("profile_snapshot")
                        .and_then(|p| p.get("verified")),
                );

            let verification_badge = if verified {
                format!(
                    "<img class=\"verified-player-icon\" src=\"{}\" alt=\"Verified PaladinsCat player\"/>",
                    self.asset_url(self.assets.icon("Verified_Player_Support_Icon", Some("avif")))
                )
            } else {
                String::new()
            };
            let mut moderation_tags = Vec::new();
            if cheater {
                moderation_tags.push(("cheater", "CHEATER"));
            } else {
                if bool_of(player.get("dropper"))
                    && num(player.get("dropper_vote_count")) >= PLAYER_TAG_MINIMUM_COUNT
                {
                    moderation_tags.push(("dropper", "DROP"));
                }
                if suspicious {
                    moderation_tags.push(("suspicious", "SUS"));
                }
                let community_afk = bool_of(player.get("afk_wintrade"))
                    && num(player.get("afk_wintrade_vote_count")) >= PLAYER_TAG_MINIMUM_COUNT;
                let automatic_afk =
                    num(player.get("automatic_afk_count")) >= PLAYER_TAG_MINIMUM_COUNT;
                if community_afk || automatic_afk {
                    moderation_tags.push((
                        match (automatic_afk, community_afk) {
                            (true, true) => "afk automatic-community",
                            (true, false) => "afk automatic-only",
                            (false, true) => "afk community-only",
                            (false, false) => unreachable!(),
                        },
                        "AFK",
                    ));
                }
                for (field, class, label) in [
                    ("wall_shooter_count", "wall-shooter", "WALL"),
                    ("master_feeding_count", "master-feeding", "FEED"),
                    ("tank_diff_count", "performance-diff tank-diff", "TANK"),
                    ("support_diff_count", "performance-diff support-diff", "SUP"),
                    ("dps_diff_count", "performance-diff dps-diff", "DPS"),
                    ("flank_diff_count", "performance-diff flank-diff", "FLANK"),
                    ("noob_count", "performance-diff noob", "NOOB"),
                    ("hypercarry_count", "performance-diff hypercarry", "CARRY"),
                ] {
                    if num(player.get(field)) >= PLAYER_TAG_MINIMUM_COUNT {
                        moderation_tags.push((class, label));
                    }
                }
                if bool_of(player.get("boosted"))
                    && num(player.get("boosted_match_count")) >= PLAYER_TAG_MINIMUM_COUNT
                {
                    moderation_tags.push(("boosted", "BOOST"));
                }
                if bool_of(player.get("alt_account"))
                    && num(player.get("alt_account_vote_count")) >= PLAYER_TAG_MINIMUM_COUNT
                {
                    moderation_tags.push(("alt", "ALT"));
                }
            }
            let moderation_tag = moderation_tags
                .into_iter()
                .map(|(class, label)| {
                    format!("<span class=\"player-status-tag {class}\">{label}</span>")
                })
                .collect::<String>();
            let party_number = party_numbers.get(&pid).copied().unwrap_or(0);
            let party_badge = if party_number > 0 {
                format!("<span class=\"party-badge\" title=\"Party {party_number}\">{party_number}</span>")
            } else {
                String::new()
            };
            let talent_markup = if !talent_name.is_empty() {
                format!(
                    "<img class=\"talent-icon\" src=\"{}\" alt=\"{}\"/>",
                    self.asset_url(self.assets.talent_icon(
                        Some(talent_id),
                        &champion_name,
                        &talent_name
                    )),
                    escape_html(&talent_name)
                )
            } else {
                "<span class=\"talent-icon talent-empty\" aria-label=\"Talent unavailable\">—</span>".to_string()
            };
            let kda = format!(
                "{} / {} / {}",
                num(player.get("kills")),
                num(player.get("deaths")),
                num(player.get("assists"))
            );
            let player_tier = player_display_tier(player);
            let champion_icon = self.asset_url(self.assets.champion_icon(&champion_name));
            let rank_icon = self.asset_url(self.assets.rank_icon(player_tier as u32));
            let credits_icon = self.asset_url(self.assets.icon("Currency_Credits", None));
            let elo = num(player.get("queue_elo"));
            let peak_cells = format!(
                "<div class=\"metric credits{}\"><img src=\"{}\" alt=\"\"/>{}</div><div class=\"metric kda\">{}</div>\
                 <div class=\"metric obj{}\">{}</div><div class=\"metric damage{}\">{}</div>\
                 <div class=\"metric taken{}\">{}</div><div class=\"metric shield{}\">{}</div>\
                 <div class=\"metric heal{}\">{}</div>",
                peak(0, false),
                credits_icon,
                number(values[0]),
                kda,
                peak(1, true),
                number(values[1]),
                peak(2, true),
                number(values[2]),
                peak(3, true),
                number(values[3]),
                peak(4, true),
                number(values[4]),
                peak(5, true),
                number(values[5]),
            );
            row_html.push_str(&format!(
                "<div class=\"player-row grid-row{}\"><div class=\"champion-wrap\">\
                 <img class=\"champion-icon\" src=\"{champion_icon}\" alt=\"{champion_name}\"/>{party_badge}</div>\
                 <div class=\"rank\"><img src=\"{rank_icon}\" alt=\"{rank_name}\"/></div>\
                 <div class=\"level\">{level}</div>\
                 <div class=\"player\"><div class=\"player-name\"><span class=\"player-name-text\">{name}</span>{vb}{mt}</div>\
                 <div class=\"player-sub\">PID {pid}</div></div>\
                 <div class=\"player-elo\">{elo}</div>{talent_markup}\
                 {peak_cells}</div>",
                if cheater { " cheater-row" } else { "" },
                champion_icon = champion_icon,
                champion_name = escape_html(&champion_name),
                party_badge = party_badge,
                rank_icon = rank_icon,
                rank_name = escape_html(&tier_name(player_tier)),
                level = number(level),
                name = escape_html(&str_of(player.get("player_name"))),
                vb = verification_badge,
                mt = moderation_tag,
                pid = escape_html(&pid),
                elo = if elo == 0 { "—".to_string() } else { number(elo) },
                talent_markup = talent_markup,
                peak_cells = peak_cells,
            ));
        }
        row_html
    }

    fn team_summary(
        &self,
        match_obj: Option<&serde_json::Value>,
        players: &[serde_json::Value],
        team: i64,
    ) -> String {
        let team_players: Vec<&serde_json::Value> = players
            .iter()
            .filter(|p| num(p.get("task_force")) == team)
            .take(5)
            .collect();
        let divisor = team_players.len().max(1);
        let mut sum = [0i64; 6];
        let mut level = 0i64;
        let mut elo = 0i64;
        let mut kda = [0i64; 3];
        for p in &team_players {
            let m = metrics(p);
            for i in 0..6 {
                sum[i] += m[i];
            }
            let fml = num(p.get("final_match_level"));
            level += if fml != 0 {
                fml
            } else {
                num(p.get("account_level"))
            };
            elo += num(p.get("queue_elo"));
            kda[0] += num(p.get("kills"));
            kda[1] += num(p.get("deaths"));
            kda[2] += num(p.get("assists"));
        }
        let level_avg = ((level as f64) / divisor as f64).round() as i64;
        let elo_avg = ((elo as f64) / divisor as f64).round() as i64;
        let kda_str = format!("{} / {} / {}", kda[0], kda[1], kda[2]);
        let winning_team = num(match_obj.and_then(|m| m.get("winning_task_force")));
        let won = winning_team == team;
        let classes = if team == 1 { "team-one" } else { "team-two" };
        let id = if team == 1 { "one" } else { "two" };
        let credits_icon = self.asset_url(self.assets.icon("Currency_Credits", None));
        format!(
            "<div class=\"team-bar {classes} grid-row\" id=\"team-{id}-summary\"><div class=\"team-heading\">\
             <div class=\"team-name\">Team {team} <span class=\"result\">{result}</span></div></div>\
             <div class=\"team-total level-total average-total\"><span class=\"team-average-label\">AVG</span>{level_avg}</div>\
             <div class=\"team-total elo-total average-total\"><span class=\"team-average-label\">AVG</span>{elo_avg}</div>\
             <div class=\"team-total credits-total\"><img src=\"{credits_icon}\" alt=\"\"/>{credits}</div>\
             <div class=\"team-total kda-total\">{kda_str}</div><div class=\"team-total objective-total\">{objective}</div>\
             <div class=\"team-total damage-total\">{damage}</div><div class=\"team-total taken-total\">{taken}</div>\
             <div class=\"team-total shield-total\">{shielding}</div><div class=\"team-total healing-total\">{healing}</div></div>",
            classes = classes,
            id = id,
            team = team,
            result = if won { "Win" } else { "Defeat" },
            level_avg = number(level_avg),
            elo_avg = number(elo_avg),
            credits_icon = credits_icon,
            credits = compact(sum[0]),
            kda_str = kda_str,
            objective = compact(sum[1]),
            damage = compact(sum[2]),
            taken = compact(sum[3]),
            shielding = compact(sum[4]),
            healing = compact(sum[5]),
        )
    }

    /// Build the complete, data-bound loadout-card document.
    ///
    /// I/O: `&Value` (data) -> `String`
/// refs: none
    pub fn loadout_document(&self, data: &serde_json::Value) -> String {
        let player = data.get("player");
        let loadout = data.get("loadout");
        let player_name = escape_html(&str_of(player.and_then(|v| v.get("name"))));
        let champion_name = str_of(loadout.and_then(|v| v.get("champion_name")));
        let loadout_name = str_of(loadout.and_then(|v| v.get("loadout_name")));
        let loadout_name = if loadout_name.is_empty() {
            "Unnamed Loadout"
        } else {
            &loadout_name
        };
        let champion_banner = self.asset_url(self.assets.champion_banner(&champion_name));
        let champion_icon = self.asset_url(self.assets.champion_icon(&champion_name));
        let background = if champion_banner.contains("image/svg+xml") {
            champion_icon
        } else {
            champion_banner
        };
        let brand_icon = self.asset_url(self.assets.icon("paladinscat", None));
        let ids = loadout
            .and_then(|v| v.get("card_ids"))
            .and_then(|v| v.as_array());
        let levels = loadout
            .and_then(|v| v.get("card_levels"))
            .and_then(|v| v.as_array());
        let mut cards = String::new();
        for (index, card_id_value) in ids.into_iter().flatten().take(5).enumerate() {
            let card_id = num(Some(card_id_value)).max(0) as u32;
            let level = levels
                .and_then(|values| values.get(index))
                .map(|v| num(Some(v)))
                .unwrap_or(1)
                .clamp(1, 5);
            let card = self.assets.loadout_card(card_id);
            let name = card
                .as_ref()
                .map(|card| card.name.clone())
                .unwrap_or_else(|| "Unknown Card".to_string());
            let fallback_name = format!("Card {card_id}");
            let name = if name == "Unknown Card" {
                fallback_name
            } else {
                name
            };
            let description = card
                .as_ref()
                .map(|card| {
                    if !card.description.is_empty() {
                        card.description.as_str()
                    } else if !card.short_description.is_empty() {
                        card.short_description.as_str()
                    } else {
                        "Card details unavailable."
                    }
                })
                .unwrap_or("Card details unavailable.");
            let description = scale_card_description(description, level);
            let artwork = self.asset_url(card.and_then(|card| card.icon_path));
            let frame = self.assets.loadout_frame(level as u32);
            let rarity = frame
                .as_ref()
                .map(|frame| frame.rarity.clone())
                .unwrap_or_default();
            let frame_url = self.asset_url(frame.map(|frame| frame.icon_path));
            let title_class = match name.chars().count() {
                22.. => "extra-long-card-name",
                20.. => "long-card-name",
                _ => "",
            };
            cards.push_str(&format!(
                "<article class=\"loadout-card level-{level}\" aria-label=\"{name}, level {level} {rarity}\"><img class=\"card-art\" src=\"{artwork}\" alt=\"\"/><img class=\"card-frame\" src=\"{frame_url}\" alt=\"\"/><h2 class=\"{title_class}\">{name}</h2><p class=\"card-description\">{description}</p><span class=\"level-badge\">{level}</span></article>",
                name = escape_html(&name), rarity = escape_html(&rarity), description = escape_html(&description)
            ));
        }

        let css = Self::extract_css(self.match_template.as_ref());
        format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"><style>{css}{loadout_css}</style></head><body data-theme=\"dark\"><main id=\"loadout\" style=\"--loadout-background:url('{background}')\"><header class=\"loadout-header\"><div class=\"loadout-identity\"><div class=\"brand-line\"><span class=\"brand-name\"><img src=\"{brand_icon}\" alt=\"\">PaladinsCat</span><div class=\"status-tags\"><span class=\"status-tag loadout-status\">Loadout</span></div></div><h1>{player_name}</h1><div class=\"match-context loadout-context\"><span>{champion}</span><span class=\"deck\">{deck}</span></div></div></header><section class=\"cards\">{cards}</section></main></body></html>",
            loadout_css = LOADOUT_DOCUMENT_CSS,
            champion = escape_html(&champion_name),
            deck = escape_html(loadout_name),
        )
    }

    /// URL of the cheater-pattern asset.
    ///
    /// I/O: () -> `&str`
/// refs: none
    pub fn cheater_pattern_url(&self) -> &str {
        &self.cheater_pattern_url
    }
}

const LOADOUT_DOCUMENT_CSS: &str = r#"
*{box-sizing:border-box}html,body{margin:0;width:1280px;height:720px;min-height:720px;overflow:hidden;padding:0;background:transparent;color:var(--text);font-family:Inter,ui-sans-serif,system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif}
#loadout{position:relative;width:1280px;height:720px;overflow:hidden;border:1px solid rgba(111,130,153,.35);border-radius:20px;background:var(--bg)}
#loadout::before{content:"";position:absolute;inset:0;background-image:var(--loadout-background);background-size:cover;background-position:center;filter:saturate(1.15);opacity:.7;pointer-events:none}
#loadout>*{z-index:1}.loadout-header{position:relative;height:238px;padding:28px 38px;display:flex;align-items:flex-start;background:rgba(5,9,15,.58);border-bottom:1px solid rgba(72,211,190,.22)}
.loadout-header::before{content:"";position:absolute;inset:0;pointer-events:none;-webkit-backdrop-filter:blur(7px);backdrop-filter:blur(7px);-webkit-mask-image:linear-gradient(90deg,#000,transparent 20%,transparent 80%,#000);mask-image:linear-gradient(90deg,#000,transparent 20%,transparent 80%,#000)}
.loadout-header>*{position:relative;z-index:1}.loadout-identity{min-width:0}.brand-line{display:flex;align-items:center;gap:12px;justify-content:flex-start}.brand-name{display:inline-flex;align-items:center;gap:11px;font-size:25px;line-height:1;font-weight:800;letter-spacing:-.02em}.brand-name img{width:32px;height:32px;border-radius:0;object-fit:contain}.loadout-status{height:23px;padding:0 9px;color:#bff7ee;border-color:rgba(55,214,192,.34);background:rgba(15,118,110,.25);font-size:10px;font-weight:760}
h1{margin:10px 0 0;padding-bottom:4px;max-width:760px;overflow:hidden;color:var(--text);font-size:54px;line-height:1.08;font-weight:720;letter-spacing:-.01em;text-overflow:ellipsis;white-space:nowrap;text-shadow:0 3px 5px rgba(0,0,0,.45)}
.loadout-context{display:flex;align-items:center;gap:10px;margin-top:8px;color:#d3dce7;font-size:15px;line-height:1.1;font-weight:740;letter-spacing:.14em}.loadout-context span{font-weight:740}.loadout-context .deck{color:#d3dce7}.loadout-context span+span::before{margin-right:9px}
.cards{position:relative;z-index:2;display:grid;grid-template-columns:repeat(5,minmax(0,1fr));gap:4px;padding:0 16px 18px;align-items:start;background:color-mix(in srgb,var(--bg) 80%,transparent);border-bottom:1px solid var(--line)}
.cards::before{content:"";position:absolute;inset:0;pointer-events:none;-webkit-backdrop-filter:blur(3px);backdrop-filter:blur(3px)}.cards>*{position:relative;z-index:1}.loadout-card{position:relative;width:100%;aspect-ratio:316/480;filter:drop-shadow(0 3px 5px rgba(0,0,0,.45))}
.card-art{position:absolute;z-index:1;left:6.5%;top:8.7%;width:87%;height:44%;object-fit:cover;background:#071014}.card-frame{position:absolute;z-index:2;inset:0;width:100%;height:100%;object-fit:fill;pointer-events:none}
.loadout-card h2{position:absolute;z-index:3;left:9%;top:51.2%;width:82%;height:6.8%;margin:0;padding:0 5px;transform:translateY(-1px);display:flex;align-items:center;justify-content:center;color:#fff;font-size:18px;line-height:1;font-weight:800;letter-spacing:-.02em;text-align:center;text-shadow:0 2px 2px #111;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.loadout-card h2.long-card-name{padding-inline:2px;font-size:14px;letter-spacing:-.025em}.loadout-card h2.extra-long-card-name{padding-inline:1px;font-size:13px;letter-spacing:-.035em}
.card-description{position:absolute;z-index:3;left:9.5%;top:59.5%;width:81%;height:29%;margin:0;padding:4px 9px 0;display:flex;align-items:flex-start;justify-content:center;color:#303943;font-size:14px;line-height:1.25;font-weight:700;text-align:center;overflow:hidden}.level-badge{position:absolute;z-index:3;left:13.2%;top:92.7%;width:20%;aspect-ratio:1;transform:translate(-47%,-44%);display:flex;align-items:center;justify-content:center;padding:0;color:#f7fbff;font-size:27px;line-height:1;font-weight:680;font-variant-numeric:tabular-nums;text-align:center;text-shadow:0 2px 3px #10151d}
"#;

/// Strip leading queue tokens (Ranked/Live/WIP + numbers) from a map name,
/// mirroring the TS normalization.
/// refs: none
fn regex_strip_prefix(s: &str) -> String {
    let mut result = s.trim().to_string();
    loop {
        let mut stripped = false;
        for prefix in ["Ranked ", "Live ", "WIP "] {
            if result
                .get(..prefix.len())
                .is_some_and(|value| value.eq_ignore_ascii_case(prefix))
            {
                result = result[prefix.len()..].trim_start().to_string();
                stripped = true;
                break;
            }
        }
        if !stripped {
            break;
        }
    }
    result
}

fn utc_timestamp_str(s: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|date| date.format("%b %-d, %Y · %-I:%M %p UTC").to_string())
        .unwrap_or_else(|_| "—".to_string())
}

/// Escape HTML special characters.
///
/// I/O: `&str` -> `String`
/// refs: none
pub fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn url_encode(s: &str) -> String {
    let mut r = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '.' | '_' | '~' => r.push(c),
            ' ' => r.push_str("%20"),
            '\n' => r.push_str("%0A"),
            _ => {
                for b in c.to_string().as_bytes() {
                    r.push_str(&format!("%{:02X}", *b));
                }
            }
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_record() -> serde_json::Value {
        let mut players = Vec::new();
        for team in 1..=2 {
            for i in 1..=5 {
                players.push(serde_json::json!({
                    "player_id": format!("p{team}{i}"),
                    "player_name": format!("Player{team}_{i}"),
                    "champion_name": if i % 2 == 0 { "Kinessa" } else { "Fernando" },
                    "kills": i * 2,
                    "deaths": i,
                    "assists": i + team,
                    "damage_done_physical": i * 5000,
                    "damage_taken": i * 3000,
                    "damage_mitigated": i * 2000,
                    "healing": i * 1500,
                    "gold_earned": i * 1200,
                    "objective_assists": i * 3,
                    "final_match_level": 5 + i,
                    "account_level": 100 + i,
                    "queue_elo": 2200 + team * 100 + i,
                    "task_force": team,
                    "league_tier": 11 + i,
                    "win_status": if team == 1 { "Win" } else { "Loss" },
                    "verides": i as i64,
                    "sus_count": if team == 2 && i == 5 { 3 } else { 0 },
                    "cheater": false,
                    "party_number": if i == 1 { 1 } else { 2 },
                }));
            }
        }
        let bans = vec![
            serde_json::json!({"ban_slot": 1, "champion_id": 100, "champion_name": "Androxus"}),
            serde_json::json!({"ban_slot": 2, "champion_id": 101, "champion_name": "Jenos"}),
            serde_json::json!({"ban_slot": 3, "champion_id": 102, "champion_name": "Tiberius"}),
            serde_json::json!({"ban_slot": 4, "champion_id": 103, "champion_name": "Makoa"}),
            serde_json::json!({"ban_slot": 5, "champion_id": 104, "champion_name": "Yagorath"}),
            serde_json::json!({"ban_slot": 6, "champion_id": 105, "champion_name": "Vatu"}),
        ];
        let facts = vec![
            serde_json::json!({
                "player_id": "p11",
                "talents": [{"talent_id": 1, "talent_name": "Aegis", "champion_name": "Fernando"}],
            }),
            serde_json::json!({
                "player_id": "p21",
                "talents": [{"talent_id": 2, "talent_name": "Eagle Eye", "champion_name": "Kinessa"}],
            }),
        ];
        serde_json::json!({
            "match": {
                "match_id": 1281311346,
                "duration_seconds": 812,
                "region": "NA",
                "map": "Ranked Frozen Guard",
                "queue_id": 486,
                "winning_task_force": 1,
                "broken": false,
                "recovered": false,
                "private": false,
                "team1_score": 4,
                "team2_score": 2,
                "entry_datetime": "2026-08-08T12:00:00Z",
            },
            "players": players,
            "bans": bans,
            "facts": facts,
        })
    }

    #[test]
    fn match_document_builds_data_bound_scoreboard() {
        let record = fake_record();
        let engine = TemplateEngine {
            match_template: Arc::new(String::new()),
            canonical_match_css: None,
            loadout_template: Arc::new(String::new()),
            cheater_pattern_url: String::new(),
            assets: AssetCatalog::new("missing-test-assets"),
        };
        let doc = engine.match_document(&record);

        // Must carry the exact id the renderer's screenshot_element targets.
        assert!(
            doc.contains("id=\"scoreboard\""),
            "missing #scoreboard element"
        );
        assert!(doc.contains("class=\"scoreboard\" id=\"scoreboard\""));

        // Real, data-bound player rows present (team one & team two sections).
        assert!(doc.contains("id=\"team-one\""));
        assert!(doc.contains("id=\"team-two\""));
        assert!(doc.contains("Player1_1"));
        assert!(doc.contains("Player2_5"));

        // Ranked queue -> bans rendered + Ranked status tag.
        assert!(doc.contains("Ranked"));
        assert!(doc.contains("Androxus"));
        assert!(doc.contains("Yagorath"));

        // Match identity values bound.
        assert!(doc.contains("1281311346"));
        assert!(doc.contains("13:32")); // 812s -> 13:32

        // The emitted row structure must match the prototype CSS selectors.
        assert!(doc.contains(
            "class=\"player\"><div class=\"player-name\"><span class=\"player-name-text\""
        ));
        assert!(doc.contains("class=\"metric credits"));
        assert!(doc.contains("class=\"metric damage"));
        assert!(doc.contains("class=\"team-total credits-total\""));
        assert!(!doc.contains("level-cell"));
        assert!(!doc.contains("http://localhost:3000"));

        // The old static-prototype empty state must NOT be present.
        assert!(!doc.contains("scoreboard-canvas-wrapper-only"));
        assert!(!doc.contains("<div class=\"scoreboard\">"));
    }

    #[test]
    fn match_document_escapes_html_in_player_names() {
        let record = fake_record();
        let mut record = record;
        record["players"][0]["player_name"] =
            serde_json::Value::String("<script>alert(1)</script>".into());
        let engine = TemplateEngine {
            match_template: Arc::new(String::new()),
            canonical_match_css: None,
            loadout_template: Arc::new(String::new()),
            cheater_pattern_url: String::new(),
            assets: AssetCatalog::new("missing-test-assets"),
        };
        let doc = engine.match_document(&record);
        assert!(doc.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(!doc.contains("<script>alert(1)</script>"));
    }

    #[test]
    fn match_document_uses_the_inclusive_five_count_tag_boundary() {
        let mut record = fake_record();
        record["players"][0]["sus_count"] = serde_json::json!(4);
        record["players"][1]["sus_count"] = serde_json::json!(5);
        record["players"][1]["dropper"] = serde_json::json!(true);
        record["players"][1]["dropper_vote_count"] = serde_json::json!(5);
        record["players"][1]["wall_shooter_count"] = serde_json::json!(5);
        record["players"][1]["afk_wintrade"] = serde_json::json!(true);
        record["players"][1]["afk_wintrade_vote_count"] = serde_json::json!(5);
        record["players"][1]["automatic_afk_count"] = serde_json::json!(5);
        record["players"][1]["tank_diff_count"] = serde_json::json!(5);
        record["players"][1]["boosted"] = serde_json::json!(true);
        record["players"][1]["boosted_match_count"] = serde_json::json!(5);
        let engine = TemplateEngine {
            match_template: Arc::new(String::new()),
            canonical_match_css: None,
            loadout_template: Arc::new(String::new()),
            cheater_pattern_url: String::new(),
            assets: AssetCatalog::new("missing-test-assets"),
        };
        let doc = engine.match_document(&record);

        assert_eq!(doc.matches("player-status-tag suspicious").count(), 1);
        assert!(doc.contains("player-status-tag dropper\">DROP"));
        assert!(doc.contains("player-status-tag afk automatic-community\">AFK"));
        assert!(doc.contains("player-status-tag wall-shooter\">WALL"));
        assert!(doc.contains("player-status-tag performance-diff tank-diff\">TANK"));
        assert!(doc.contains("player-status-tag boosted\">BOOST"));
    }

    #[test]
    fn extract_css_removes_external_font_import_and_style_tag() {
        let css = TemplateEngine::extract_css(
            "<style>\n@import url('https://fonts.googleapis.com/css2?family=Inter');\n.player-row{display:grid}\n</style>",
        );
        assert_eq!(css.trim(), ".player-row{display:grid}");
    }

    #[test]
    fn extracts_the_frontend_owned_scoreboard_styles() {
        let css = TemplateEngine::extract_browser_scoreboard_css(
            "body { color: red; }\n#browser-scoreboard { --bg: #161618; }\n#browser-scoreboard .scoreboard { display: grid; }",
        )
        .unwrap();
        assert!(css.starts_with("#browser-scoreboard {"));
        assert!(!css.contains("body { color: red; }"));
    }

    #[test]
    fn limited_unknown_queue_matches_web_presentation() {
        let mut record = fake_record();
        record["match"]["map"] = serde_json::json!("WIP Waterway (Siege)");
        record["match"]["queue_id"] = serde_json::json!(10225);
        record["match"]["queue_name"] = serde_json::json!("Unclassified Queue 10225");
        record["match"]["is_ranked"] = serde_json::json!(false);
        record["match"]["is_custom"] = serde_json::json!(false);
        record["match"]["stats_scope"] = serde_json::json!("other");
        record["match"]["participant_model"] = serde_json::json!("unknown");
        record["match"]["limited"] = serde_json::json!(true);
        record["match"]["broken"] = serde_json::json!(true);
        let engine = TemplateEngine {
            match_template: Arc::new(String::new()),
            canonical_match_css: None,
            loadout_template: Arc::new(String::new()),
            cheater_pattern_url: String::new(),
            assets: AssetCatalog::new("missing-test-assets"),
        };
        let doc = engine.match_document(&record);

        assert!(doc.contains("status-tag casual\">Casual"));
        assert!(doc.contains("status-tag limited\">Limited"));
        assert!(!doc.contains("status-tag broken\">Broken"));
        assert!(doc.contains("<span>Unclassified Queue 10225</span>"));
    }

    #[test]
    fn loadout_document_builds_data_bound_capture_target() {
        let engine = TemplateEngine {
            match_template: Arc::new("<style>:root{--bg:#080d13;--text:#fff}</style>".into()),
            canonical_match_css: None,
            loadout_template: Arc::new(String::new()),
            cheater_pattern_url: String::new(),
            assets: AssetCatalog::new("missing-test-assets"),
        };
        let record = serde_json::json!({
            "player": {"id": "16706730", "name": "Nabi<Cook>TV"},
            "loadout": {
                "id": "7968",
                "champion_name": "Androxus",
                "loadout_name": "New Loadout",
                "card_ids": [11928, 13316, 13293, 13290, 13322],
                "card_levels": [5, 5, 2, 2, 1]
            }
        });
        let doc = engine.loadout_document(&record);
        assert!(doc.contains("id=\"loadout\""));
        assert!(doc.contains("Nabi&lt;Cook&gt;TV"));
        assert!(doc.contains("Androxus"));
        assert!(doc.contains("New Loadout"));
        assert_eq!(doc.matches("class=\"loadout-card level-").count(), 5);
        assert!(doc.contains("background-position:center"));
        assert!(!doc.contains("background-position:center 24%"));
        assert!(!doc.contains("__renderData"));
        assert!(!doc.contains("http://localhost"));
    }

    #[test]
    fn card_description_scaling_matches_typescript() {
        assert_eq!(
            scale_card_description("[Ability] Increase speed by {scale=10|10}%.", 5),
            "Increase speed by 50%."
        );
        assert_eq!(
            scale_card_description("Gain {1,000|250.5} Health.", 3),
            "Gain 1,501 Health."
        );
    }

    #[test]
    fn asset_mime_uses_file_signature_before_misleading_extension() {
        assert_eq!(
            asset_mime(
                std::path::Path::new("card.png"),
                b"RIFF\x10\x00\x00\x00WEBPVP8 "
            ),
            "image/webp"
        );
        assert_eq!(
            asset_mime(std::path::Path::new("card.avif"), b"\x89PNG\r\n\x1a\nrest"),
            "image/png"
        );
    }

    #[test]
    fn party_markers_and_zero_scores_match_typescript_renderer() {
        let mut record = fake_record();
        record["match"]["team1_score"] = serde_json::json!(0);
        for player in record["players"].as_array_mut().unwrap() {
            player["party"] = serde_json::json!(0);
            player["party_number"] = serde_json::json!(0);
        }
        record["players"][0]["party_id"] = serde_json::json!(9001);
        record["players"][1]["party_id"] = serde_json::json!(9001);
        record["players"][2]["party_id"] = serde_json::json!(7777);
        let engine = TemplateEngine {
            match_template: Arc::new(String::new()),
            canonical_match_css: None,
            loadout_template: Arc::new(String::new()),
            cheater_pattern_url: String::new(),
            assets: AssetCatalog::new("missing-test-assets"),
        };
        let doc = engine.match_document(&record);
        assert!(doc.contains("team-one-score\">0</span>"));
        assert_eq!(doc.matches("title=\"Party 1\"").count(), 2);
        assert!(!doc.contains("Party 7777"));
        assert!(doc.contains("<span>NA</span><span>Siege</span>"));
    }
}
