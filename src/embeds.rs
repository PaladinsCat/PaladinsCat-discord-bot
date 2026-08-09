//! Discord embed message builders — mirrors TS message-builders.ts 1-to-1.

use chrono::Datelike;
use percent_encoding::{percent_encode, NON_ALPHANUMERIC};
use twilight_model::channel::message::embed::{Embed, EmbedField};
use twilight_model::util::Timestamp;
use twilight_util::builder::embed::{
    EmbedBuilder, EmbedFieldBuilder, EmbedFooterBuilder, ImageSource,
};

use serde_json::Value;

/// PaladinsCat accent green
const ACCENT: u32 = 0x2dd4a3;
/// Pending lobby amber
const PENDING: u32 = 0xf0b232;
/// Not in match gray
const NOT_IN_MATCH: u32 = 0x77808d;

fn url_encode(value: &str) -> String {
    percent_encode(value.as_bytes(), NON_ALPHANUMERIC).to_string()
}

/// Queue labels — matches TS QUEUE_LABELS
pub const QUEUE_LABELS: &[(i32, &str)] = &[
    (1, "Casual Queue"),
    (2, "KBM"),
    (4, "1v1"),
    (8, "Team Queue"),
    (16, "Open"),
    (32, "Doomspire"),
    (424, "Casual Siege"),
    (428, "Ranked Siege (Controller)"),
    (437, "Casual Payload"),
    (451, "PvE Survival"),
    (452, "Casual Onslaught"),
    (469, "Casual Team Deathmatch"),
    (474, "Casual Battlegrounds Solo"),
    (475, "Casual Battlegrounds Duo"),
    (476, "Casual Battlegrounds Quad"),
    (486, "Ranked Siege"),
];

/// Tier names — matches TS TIER_NAMES
pub const TIER_NAMES: &[&str] = &[
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

/// Escape Discord markdown special characters — mirrors TS cleanDiscordText
pub fn clean_discord_text(value: &Value, fallback: &str) -> String {
    let text = value.as_str().unwrap_or(fallback).trim().to_string();
    if text.is_empty() {
        return fallback.to_string();
    }
    // Escape Discord markdown characters
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if "\\`*_{}[]()#+-.!|>~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Numeric metric extraction — mirrors TS numericMetric
pub fn numeric_metric(value: &Value) -> Option<f64> {
    if value.is_null() || value.is_boolean() {
        return None;
    }
    if let Some(n) = value.as_f64() {
        if n.is_finite() {
            return Some(n);
        }
    }
    if let Some(s) = value.as_str() {
        if let Ok(n) = s.parse::<f64>() {
            if n.is_finite() {
                return Some(n);
            }
        }
    }
    None
}

/// Format number with locale grouping — mirrors TS toLocaleString
pub fn format_number(value: f64) -> String {
    let i = value as i64;
    if (i as f64) == value {
        format_grouped(i)
    } else {
        format!("{}", value)
    }
}

/// Insert thousands separators (commas) — mirrors en-US toLocaleString grouping.
fn format_grouped(n: i64) -> String {
    let neg = n < 0;
    let digits = n.abs().to_string();
    let bytes = digits.as_bytes();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (idx, &b) in bytes.iter().enumerate() {
        if idx > 0 && (bytes.len() - idx) % 3 == 0 {
            out.push(',');
        }
        out.push(b as char);
    }
    if neg {
        format!("-{}", out)
    } else {
        out
    }
}

/// Format number with decimals — mirrors TS formattedNumber
pub fn format_number_dec(value: Option<f64>, decimals: usize) -> String {
    let Some(value) = value else { return "—".to_string() };
    // `toLocaleString` groups the integer part even when a fixed fractional
    // precision is requested (including precision zero).
    let formatted = format!("{:.prec$}", value, prec = decimals);
    let (sign, digits) = formatted.strip_prefix('-').map_or(("", formatted.as_str()), |v| ("-", v));
    let (integer, fraction) = digits.split_once('.').unwrap_or((digits, ""));
    let grouped = format_grouped(integer.parse::<i64>().unwrap_or(0));
    if decimals == 0 { format!("{}{}", sign, grouped) } else { format!("{}{}.{}", sign, grouped, fraction) }
}

/// Duration label — mirrors TS durationLabel
pub fn duration_label(value: &Value) -> String {
    let seconds = numeric_metric(value).unwrap_or(0.0).max(0.0).round() as i64;
    let mins = seconds / 60;
    let secs = (seconds % 60) as u32;
    format!("{}m {:02}s", mins, secs)
}

/// Get queue label — mirrors TS QUEUE_LABELS lookup
pub fn queue_label(queue_id: i32) -> String {
    for &(id, label) in QUEUE_LABELS {
        if id == queue_id {
            return label.to_string();
        }
    }
    if queue_id > 0 {
        format!("Queue #{}", queue_id)
    } else {
        "Unknown queue".to_string()
    }
}

fn json_id(value: Option<&Value>) -> Option<String> {
    value.and_then(|value| match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    })
}

/// Strip leading "Live ", "Ranked ", "WIP " prefixes (repeated) — mirrors TS
/// `String(map).replace(/^(?:(?:live|ranked|wip)\s+)+/i, '')`.
fn strip_map_prefix(name: &str) -> String {
    let mut s = name;
    loop {
        let t = s.trim_start();
        let lower = t.to_ascii_lowercase();
        let matched = ["live ", "ranked ", "wip "]
            .iter()
            .find(|p| lower.starts_with(**p));
        match matched {
            Some(p) => s = &t[p.len()..],
            None => break,
        }
    }
    s.to_string()
}

/// Get tier name — mirrors TS TIER_NAMES lookup
pub fn tier_name(tier: i32) -> String {
    if tier >= 0 && (tier as usize) < TIER_NAMES.len() {
        TIER_NAMES[tier as usize].to_string()
    } else {
        "Unranked".to_string()
    }
}

/// Build a simple embed with title, description, and optional URL — mirrors TS simpleEmbed
pub fn simple_embed(title: &str, description: &str, url: Option<&str>) -> Embed {
    let mut builder = EmbedBuilder::new()
        .color(ACCENT)
        .title(title)
        .description(description);
    if let Some(u) = url {
        builder = builder.url(u);
    }
    builder.build()
}

/// Build embed with footer — mirrors TS embedPayload
pub fn embed_with_footer(title: &str, description: &str, footer: &str, color: u32) -> Embed {
    EmbedBuilder::new()
        .color(color)
        .title(title)
        .description(description)
        .footer(EmbedFooterBuilder::new(footer))
        .build()
}

/// Build history payload — mirrors TS buildHistoryPayload
pub fn build_history_payload(player_name: &str, history: &[Value], web_url: &str) -> Embed {
    let mut lines = Vec::new();
    for row in history.iter().take(10) {
        if let Some(obj) = row.as_object() {
            let w = if obj.get("win_status").and_then(|v| v.as_str()) == Some("Winner") {
                "✅"
            } else {
                "❌"
            };
            let map = obj
                .get("map")
                .or_else(|| obj.get("champion_name"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let dur = obj
                .get("duration_seconds")
                .and_then(|v| numeric_metric(v))
                .map(|s| format!("{}m", (s / 60.0).round() as i64))
                .unwrap_or_default();
            let region = obj.get("region").and_then(|v| v.as_str()).unwrap_or("");
            let champ = obj
                .get("champion_name")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown");
            let kda = format!(
                "{}/{}/{}",
                obj.get("kills")
                    .and_then(|v| numeric_metric(v))
                    .unwrap_or(0.0) as i64,
                obj.get("deaths")
                    .and_then(|v| numeric_metric(v))
                    .unwrap_or(0.0) as i64,
                obj.get("assists")
                    .and_then(|v| numeric_metric(v))
                    .unwrap_or(0.0) as i64,
            );
            let id = json_id(obj.get("match_id")).unwrap_or_default();
            let parts: Vec<&str> = vec![w, map, &dur, region, champ, &kda]
                .iter()
                .filter(|s| !s.is_empty())
                .map(|s| *s)
                .collect();
            let joined = parts.join(" · ");
            lines.push(format!("{} · [{}]({}/matches/{})", joined, id, web_url, id));
        }
    }
    let description = if lines.is_empty() {
        "No recent matches found.".to_string()
    } else {
        lines.join("\n")
    };
    let title = format!("{} · Recent matches", player_name);
    simple_embed(&title, &description, None)
}

/// Build current payload — mirrors TS buildCurrentPayload
pub fn build_current_payload(result: &Value, web_url: &str) -> Embed {
    let match_data = result.get("match").unwrap_or(&Value::Null);
    let empty_players: Vec<Value> = Vec::new();
    let players = result
        .get("players")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty_players);
    let player_id = json_id(result.get("player_id"))
        .or_else(|| json_id(match_data.get("source_player_id")))
        .unwrap_or_default();

    // Pending state
    if result.get("pending") == Some(&Value::Bool(true)) {
        embed_with_footer(
            "Live lobby loading",
            "The player is in a match, but the lobby snapshot is still being assembled. Try `/current` again shortly.",
            "PaladinsCat refreshes pending live lobbies automatically.",
            PENDING,
        )
    }
    // Not in match
    else if match_data.get("match_id").is_none() {
        embed_with_footer(
            "Not in a live match",
            "No active Paladins match was found for this player.",
            "Live status is cached briefly to protect the Paladins API.",
            NOT_IN_MATCH,
        )
    }
    // Active match
    else {
        let match_id = match_data
            .get("match_id")
            .and_then(|v| json_id(Some(v)))
            .unwrap_or_default();
        let queue_id = match_data
            .get("queue_id")
            .and_then(|v| numeric_metric(v))
            .unwrap_or(0.0) as i32;
        let queue = queue_label(queue_id);
        let map = clean_discord_text(
            &match_data.get("map").unwrap_or(&Value::Null),
            "Unknown map",
        );
        // Strip leading "Live ", "Ranked ", "WIP " prefixes (repeated) — mirrors TS regex
        let map = strip_map_prefix(&map);
        let region = clean_discord_text(
            &match_data.get("region").unwrap_or(&Value::Null),
            "Unknown region",
        );
        let title = format!("{} · Live match", map);
        let description = format!("**{}** · {}\nMatch ID `{}`", queue, region, match_id);

        // Build team fields
        let mut team1_players = Vec::new();
        let mut team2_players = Vec::new();
        for player in players {
            if let Some(obj) = player.as_object() {
                let tf = obj
                    .get("task_force")
                    .and_then(|v| numeric_metric(v))
                    .unwrap_or(0.0) as i32;
                let line = current_player_line(player, &player_id, web_url);
                if tf == 1 {
                    team1_players.push(line);
                } else {
                    team2_players.push(line);
                }
            }
        }
        let team1 = if team1_players.is_empty() {
            "Lobby details unavailable.".to_string()
        } else {
            team1_players.join("\n")
        };
        let team2 = if team2_players.is_empty() {
            "Lobby details unavailable.".to_string()
        } else {
            team2_players.join("\n")
        };

        // Team win chance estimate
        let estimate = estimate_live_team_win_chance(players);
        let team1_name = if let Some(est) = &estimate {
            format!("Team 1 · {}% win chance", est.team_one)
        } else {
            "Team 1".to_string()
        };
        let team2_name = if let Some(est) = &estimate {
            format!("Team 2 · {}% win chance", est.team_two)
        } else {
            "Team 2".to_string()
        };

        let footer = if estimate.is_some() {
            "Estimate blends queue ELO with global win rate · ▸ marks the requested player · Live lobby snapshot".to_string()
        } else {
            "▸ marks the requested player · Live lobby snapshot".to_string()
        };

        let mut builder = EmbedBuilder::new()
            .color(ACCENT)
            .title(&title)
            .description(&description)
            .field(EmbedFieldBuilder::new(team1_name, team1).inline())
            .field(EmbedFieldBuilder::new(team2_name, team2).inline())
            .footer(EmbedFooterBuilder::new(&footer));

        // Timestamp
        if let Some(detected_at) = match_data.get("detected_at").and_then(|v| v.as_str()) {
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(detected_at) {
                if let Ok(ts) = Timestamp::from_secs(dt.timestamp()) {
                    builder = builder.timestamp(ts);
                }
            }
        }

        builder.build()
    }
}

/// Build player line for current match — mirrors TS currentPlayerLine
fn current_player_line(player: &Value, source_player_id: &str, web_url: &str) -> String {
    let player_id = player
        .get("player_id")
        .and_then(|v| json_id(Some(v)))
        .unwrap_or_default();
    let player_name = clean_discord_text(
        player.get("player_name").unwrap_or(&Value::Null),
        "Private Account",
    );
    let champion = clean_discord_text(
        player.get("champion_name").unwrap_or(&Value::Null),
        "Unknown champion",
    );
    let tier_number = player
        .get("kbm_tier")
        .or_else(|| player.get("live_tier"))
        .or_else(|| player.get("tier"))
        .and_then(|v| numeric_metric(v))
        .unwrap_or(0.0) as i32;
    let tier = tier_name(tier_number);

    let name = if !player_id.is_empty()
        && player_id.parse::<i64>().is_ok()
        && player_id.parse::<i64>().unwrap() > 0
    {
        format!("[{}]({}/players/{})", player_name, web_url, player_id)
    } else {
        player_name.clone()
    };

    let marker = if player_id == source_player_id {
        "▸ "
    } else {
        ""
    };

    let global_wr = player
        .get("profile_win_rate")
        .and_then(|v| numeric_metric(v));
    let queue_elo = player.get("queue_elo").and_then(|v| numeric_metric(v));

    let mut details = Vec::new();
    if tier != "Unranked" {
        details.push(tier);
    }
    if let Some(wr) = global_wr {
        details.push(format!("Global {:.1}% WR", wr));
    }
    if let Some(elo) = queue_elo {
        details.push(format!("{} ELO", format_number(elo.round())));
    }

    let detail_str = if details.is_empty() {
        String::new()
    } else {
        format!(" · {}", details.join(" · "))
    };

    format!("{}**{}** · {}{}", marker, champion, name, detail_str)
}

/// Estimated win chance for each live-match team — mirrors TS estimateLiveTeamWinChance.
struct TeamWinEstimate {
    team_one: i32,
    team_two: i32,
}

/// Per-team average ELO and win rate used to blend a win-chance estimate.
struct TeamMetrics {
    average_elo: Option<f64>,
    average_wr: Option<f64>,
}

/// Estimate live team win chance — mirrors TS estimateLiveTeamWinChance
fn estimate_live_team_win_chance(players: &[Value]) -> Option<TeamWinEstimate> {
    let team_metrics = |task_force: i32| -> Option<TeamMetrics> {
        let team: Vec<_> = players
            .iter()
            .filter(|p| {
                p.get("task_force")
                    .and_then(|v| numeric_metric(v))
                    .unwrap_or(0.0) as i32
                    == task_force
            })
            .collect();

        let elos: Vec<f64> = team
            .iter()
            .filter_map(|p| {
                p.get("queue_elo")
                    .and_then(|v| numeric_metric(v))
                    .filter(|&v| v > 0.0 && v <= 3500.0)
            })
            .collect();

        let wrs: Vec<f64> = team
            .iter()
            .filter_map(|p| {
                p.get("profile_win_rate")
                    .and_then(|v| numeric_metric(v))
                    .filter(|&v| v >= 0.0 && v <= 100.0)
            })
            .collect();

        let min_coverage = std::cmp::min(3, team.len()) as usize;

        let avg_elo = if elos.len() >= min_coverage {
            Some(elos.iter().sum::<f64>() / elos.len() as f64)
        } else {
            None
        };

        let avg_wr = if wrs.len() >= min_coverage {
            Some(wrs.iter().sum::<f64>() / wrs.len() as f64)
        } else {
            None
        };

        avg_elo.map(|average_elo| TeamMetrics {
            average_elo: Some(average_elo),
            average_wr: avg_wr,
        })
    };

    let team1 = team_metrics(1)?;
    let team2 = team_metrics(2)?;

    let elo_prob = 1.0 / (1.0 + 10.0_f64.powf((team2.average_elo? - team1.average_elo?) / 400.0));
    let wr_prob = match (team1.average_wr, team2.average_wr) {
        (Some(w1), Some(w2)) if w1 + w2 > 0.0 => w1 / (w1 + w2),
        _ => 0.5,
    };
    let blended = (elo_prob * 0.85 + wr_prob * 0.15).max(0.15).min(0.85);
    let team_one = (blended * 100.0).round() as i32;
    Some(TeamWinEstimate {
        team_one,
        team_two: 100 - team_one,
    })
}

/// Build loadouts payload — mirrors TS buildLoadoutsPayload
pub fn build_loadouts_payload(
    player_name: &str,
    loadouts: &[Value],
    web_url: &str,
    player_id: Option<&str>,
) -> Embed {
    let lines: Vec<String> = loadouts
        .iter()
        .take(15)
        .map(|row| {
            let champ = row
                .get("champion_name")
                .and_then(|v| v.as_str())
                .unwrap_or("Champion");
            let name = row
                .get("loadout_name")
                .and_then(|v| v.as_str())
                .unwrap_or("Unnamed");
            format!("• **{}** · {}", champ, name)
        })
        .collect();

    let description = if lines.is_empty() {
        "No saved loadouts found.".to_string()
    } else {
        lines.join("\n")
    };

    let title = format!("{} · Loadouts", player_name);
    let url = player_id.map(|pid| format!("{}/players/{}/loadouts", web_url, pid));
    simple_embed(&title, &description, url.as_deref())
}

/// Build champion payload — mirrors TS buildChampionPayload
pub fn build_champion_payload(result: &Value, web_url: &str, lobby_label: &str) -> Embed {
    let champion = result.get("champion").unwrap_or(&Value::Null);
    let stats = result.get("stats").unwrap_or(&Value::Null);
    let performance = result.get("championPerformance").unwrap_or(&Value::Null);
    let talent_stats = result.get("talentStats").unwrap_or(&Value::Null);

    // Get identity metric
    let identity_metric = ["dpm", "wpm", "apm", "gpm", "hpm", "mpm", "kda"]
        .iter()
        .map(|&key| performance.get(key).unwrap_or(&Value::Null))
        .find(|m| m.get("championName").is_some() || m.get("className").is_some())
        .unwrap_or(&Value::Null);

    let name = champion
        .get("name")
        .and_then(|v| v.as_str())
        .or_else(|| identity_metric.get("championName").and_then(|v| v.as_str()))
        .unwrap_or("Unknown");
    let class_name = champion
        .get("roles")
        .and_then(|v| v.as_str())
        .or_else(|| identity_metric.get("className").and_then(|v| v.as_str()))
        .unwrap_or("Unknown");

    // Tier
    let avg_tier = stats.get("avg_league_tier").and_then(|v| numeric_metric(v));
    let tier_value = if let Some(t) = avg_tier.filter(|t| *t > 0.0) {
        let rounded = t.round() as i32;
        let clamped = rounded.max(0).min((TIER_NAMES.len() - 1) as i32);
        let tier_name = TIER_NAMES[clamped as usize];
        format!("**{}**\n{:.1} average", tier_name, t)
    } else {
        "—".to_string()
    };

    // Win rate and record
    let win_rate = stats.get("win_rate").and_then(|v| numeric_metric(v));
    let wins = stats
        .get("wins")
        .and_then(|v| numeric_metric(v))
        .unwrap_or(0.0);
    let losses = stats
        .get("losses")
        .and_then(|v| numeric_metric(v))
        .unwrap_or(0.0);
    let total = stats
        .get("total_plays")
        .or_else(|| stats.get("total_matches"))
        .and_then(|v| numeric_metric(v))
        .unwrap_or(0.0);
    let record_value = format!(
        "{}\n{} W · {} L\n{} total plays",
        win_rate
            .map(|w| format!("**{:.1}%** win rate", w))
            .unwrap_or("**—** win rate".to_string()),
        format_number(wins.round()),
        format_number(losses.round()),
        format_number(total.round()),
    );

    // Metric fields
    let metrics = vec![
        ("DPM", "dpm", 0),
        ("WPM", "wpm", 0),
        ("APM", "apm", 0),
        ("CPM", "gpm", 0),
        ("HPM", "hpm", 0),
        ("SPM", "mpm", 0),
        ("KDA", "kda", 1),
    ];
    let mut fields = Vec::new();

    for (label, key, decimals) in &metrics {
        let metric = performance.get(key).unwrap_or(&Value::Null);
        let avg = metric.get("avgValue").and_then(|v| numeric_metric(v));
        let p10 = metric.get("p10").and_then(|v| numeric_metric(v));
        let p90 = metric.get("p90").and_then(|v| numeric_metric(v));
        let value = format!(
            "**{}**\nP10–P90 {}–{}",
            format_number_dec(avg, *decimals as usize),
            format_number_dec(p10, *decimals as usize),
            format_number_dec(p90, *decimals as usize),
        );
        fields.push(EmbedField {
            name: label.to_string(),
            value,
            inline: true,
        });
    }

    // Talents
    let talent_coverage = talent_stats
        .get("talentCoveredMatches")
        .and_then(|v| numeric_metric(v))
        .unwrap_or(0.0)
        .max(0.0) as i64;
    let empty_talents: Vec<Value> = Vec::new();
    let talents = talent_stats
        .get("talents")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty_talents);
    let mut sorted_talents: Vec<_> = talents.iter().collect();
    sorted_talents.sort_by(|a, b| {
        let pa = a
            .get("totalPlays")
            .and_then(|v| numeric_metric(v))
            .unwrap_or(0.0);
        let pb = b
            .get("totalPlays")
            .and_then(|v| numeric_metric(v))
            .unwrap_or(0.0);
        pb.partial_cmp(&pa).unwrap_or(std::cmp::Ordering::Equal)
    });
    let top3 = sorted_talents.iter().take(3);
    let talent_lines: Vec<String> = top3
        .map(|talent| {
            let plays = talent
                .get("totalPlays")
                .and_then(|v| numeric_metric(v))
                .unwrap_or(0.0);
            let pick_rate = if talent_coverage > 0 {
                format!("{:.1}%", 100.0 * plays / talent_coverage as f64)
            } else {
                "—".to_string()
            };
            let wr = talent.get("winRate").and_then(|v| numeric_metric(v));
            let name =
                clean_discord_text(talent.get("talentName").unwrap_or(&Value::Null), "Unknown");
            format!(
                "**{}** · {:.1}% WR · {} pick · {} plays",
                name,
                wr.map(|value| format!("{:.1}", value))
                    .unwrap_or_else(|| "—".into()),
                pick_rate,
                format_number(plays.round()),
            )
        })
        .collect();
    let talent_value = if talent_lines.is_empty() {
        "No ranked talent data in this lobby range.".to_string()
    } else {
        talent_lines.join("\n")
    };

    // Build fields
    let class_field = EmbedField {
        name: "Class".to_string(),
        value: class_name.to_string(),
        inline: true,
    };
    let tier_field = EmbedField {
        name: "Average lobby tier".to_string(),
        value: tier_value,
        inline: true,
    };
    let record_field = EmbedField {
        name: "Ranked record".to_string(),
        value: record_value,
        inline: true,
    };

    let url = format!("{}/champions/{}", web_url, url_encode(&name.to_lowercase()));
    let description = format!(
        "**{}** · Served from the PaladinsCat champion database.",
        lobby_label
    );

    let mut builder = EmbedBuilder::new()
        .color(ACCENT)
        .title(&format!("{} · Ranked performance", name))
        .url(&url)
        .description(&description)
        .field(class_field)
        .field(tier_field)
        .field(record_field)
        .footer(EmbedFooterBuilder::new(
            "Lobby filters use the ranked match database; global is the default.",
        ));
    for field in fields {
        builder = builder.field(field);
    }
    builder = builder.field(EmbedFieldBuilder::new("Most played talents", talent_value));
    builder.build()
}

/// Build maps payload — mirrors TS buildMapsPayload
pub fn build_maps_payload(rows: &[Value], web_url: &str) -> Embed {
    let mut lines = Vec::new();
    for row in rows {
        let name = row.get("map").and_then(|v| v.as_str()).unwrap_or("Unknown");
        let safe_name = clean_discord_text(
            &Value::String(name.strip_prefix("Ranked ").unwrap_or(name).to_string()),
            "Unknown",
        );
        let matches = row
            .get("total_matches")
            .and_then(|v| numeric_metric(v))
            .unwrap_or(0.0)
            .max(0.0)
            .round() as i64;
        let share = row.get("distribution_rate").and_then(|v| numeric_metric(v));
        let share_str = share
            .map(|value| format!("{:.1}%", value))
            .unwrap_or_else(|| "—".into());
        let duration = duration_label(row.get("avg_duration_seconds").unwrap_or(&Value::Null));

        lines.push(format!(
            "**[{}]({}/game/maps/{})** · {} matches · {} of pool · {} avg",
            safe_name,
            web_url,
            url_encode(name),
            format_number(matches as f64),
            share_str,
            duration,
        ));
    }

    let description = if lines.is_empty() {
        "No ranked map statistics are available.".to_string()
    } else {
        // Truncate to 4000 chars
        let mut buf = String::new();
        let mut length = 0;
        for line in &lines {
            let next = length + line.len() + if buf.is_empty() { 0 } else { 1 };
            if next > 4000 {
                break;
            }
            if !buf.is_empty() {
                buf.push('\n');
            }
            buf.push_str(line);
            length = next;
        }
        buf
    };

    let title = "Ranked map statistics";
    let url = format!("{}/game/maps", web_url);
    EmbedBuilder::new()
        .color(ACCENT)
        .title(title)
        .url(&url)
        .description(&description)
        .footer(EmbedFooterBuilder::new(
            "PaladinsCat ranked match database · Ordered by matches played",
        ))
        .build()
}

/// Build composition payload — mirrors TS buildCompositionPayload
pub fn build_composition_payload(rows: &[Value], web_url: &str) -> Embed {
    let fields: Vec<EmbedField> = rows
        .iter()
        .take(5)
        .enumerate()
        .map(|(i, row)| {
            let frontline = row
                .get("frontline")
                .and_then(|v| numeric_metric(v))
                .unwrap_or(0.0)
                .round() as i32;
            let damage = row
                .get("damage")
                .and_then(|v| numeric_metric(v))
                .unwrap_or(0.0)
                .round() as i32;
            let flank = row
                .get("flank")
                .and_then(|v| numeric_metric(v))
                .unwrap_or(0.0)
                .round() as i32;
            let support = row
                .get("support")
                .and_then(|v| numeric_metric(v))
                .unwrap_or(0.0)
                .round() as i32;
            let count = row
                .get("count")
                .and_then(|v| numeric_metric(v))
                .unwrap_or(0.0)
                .max(0.0)
                .round() as i64;
            let wr = row.get("winrate").and_then(|v| numeric_metric(v));
            let roles = format!(
                "{} Frontline · {} Damage · {} Flank · {} Support",
                frontline, damage, flank, support
            );
            let name = format!("{}. {}", i + 1, roles);
            let value = format!(
                "{} matches · {} win rate",
                format_number(count as f64),
                wr.map(|value| format!("{:.1}%", value))
                    .unwrap_or_else(|| "—".into())
            );
            EmbedField {
                name,
                value,
                inline: false,
            }
        })
        .collect();

    let url = format!("{}/game/compositions", web_url);
    let description = if rows.is_empty() {
        "No ranked composition statistics are available.".to_string()
    } else {
        "Most-played global ranked role lineups.".to_string()
    };

    let mut builder = EmbedBuilder::new()
        .color(ACCENT)
        .title("Top ranked team compositions")
        .url(&url)
        .description(&description)
        .footer(EmbedFooterBuilder::new(
            "Top five by matches played · PaladinsCat ranked match database",
        ));
    for field in fields {
        builder = builder.field(field);
    }
    builder.build()
}

/// Build items payload — mirrors TS buildItemsPayload
pub fn build_items_payload(rows: &[Value], web_url: &str, lobby_label: &str) -> Embed {
    let mut lines = Vec::new();
    for (i, row) in rows.iter().enumerate().take(20) {
        let id = row.get("item_id").and_then(|v| v.as_str()).unwrap_or("");
        let name = clean_discord_text(row.get("item_name").unwrap_or(&Value::Null), "Unknown item");
        let uses = row
            .get("total_uses")
            .or_else(|| row.get("total_usage"))
            .and_then(|v| numeric_metric(v))
            .unwrap_or(0.0)
            .max(0.0)
            .round() as i64;
        let pick_rate = row.get("pick_rate").and_then(|v| numeric_metric(v));
        let win_rate = row.get("win_rate").and_then(|v| numeric_metric(v));
        let linked_name = if !id.is_empty() {
            format!("[{}]({}/game/items/{})", name, web_url, url_encode(id))
        } else {
            name
        };
        lines.push(format!(
            "**{}. {}** · {} pick · {} WR · {} uses",
            i + 1,
            linked_name,
            pick_rate
                .map(|value| format!("{:.1}%", value))
                .unwrap_or_else(|| "—".into()),
            win_rate
                .map(|value| format!("{:.1}%", value))
                .unwrap_or_else(|| "—".into()),
            format_number(uses as f64),
        ));
    }

    let description = if lines.is_empty() {
        format!(
            "**{}**\nNo ranked item statistics are available.",
            lobby_label
        )
    } else {
        let mut buf = String::new();
        let mut length = 0;
        for line in &lines {
            let next = length + line.len() + if buf.is_empty() { 0 } else { 1 };
            if next > 4000 {
                break;
            }
            if !buf.is_empty() {
                buf.push('\n');
            }
            buf.push_str(line);
            length = next;
        }
        format!("**{}**\n{}", lobby_label, buf)
    };

    let url = format!("{}/game/items", web_url);
    EmbedBuilder::new()
        .color(ACCENT)
        .title("Ranked item statistics")
        .url(&url)
        .description(&description)
        .footer(EmbedFooterBuilder::new(
            "Top twenty by usage · Global ranked lobbies are the default",
        ))
        .build()
}

// ——— Player profile (mirrors TS player-profile-message.ts) ———

/// Strip HTML tags, collapse whitespace, escape markdown, truncate — mirrors TS compact.
fn compact(value: &Value, max_length: usize) -> Option<String> {
    let raw = value.as_str().unwrap_or("");
    let mut in_tag = false;
    let mut plain = String::with_capacity(raw.len());
    for c in raw.chars() {
        if in_tag {
            if c == '>' {
                in_tag = false;
            }
        } else if c == '<' {
            in_tag = true;
        } else {
            plain.push(c);
        }
    }
    let collapsed: String = plain.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    let escaped = clean_discord_text(&Value::String(collapsed), "");
    Some(escaped.chars().take(max_length).collect())
}

/// Left-pad a label to 14 columns — mirrors TS statLine.
fn stat_line(label: &str, value: String) -> String {
    format!("{:<14}: {}", label, value)
}

/// Wrap lines in a code block — mirrors TS codeBlock.
fn code_block(lines: &[String]) -> String {
    format!("```\n{}\n```", lines.join("\n"))
}

/// Win rate percentage from wins/losses — mirrors TS formatPercent.
fn format_percent(wins: f64, losses: f64) -> Option<String> {
    let games = wins + losses;
    if games > 0.0 {
        Some(format!("{:.1}%", (wins / games) * 100.0))
    } else {
        None
    }
}

/// Global KDA from globalStats — mirrors TS globalKda.
fn global_kda(stats: &Value) -> Option<String> {
    if !stats.is_object() {
        return None;
    }
    let kills = numeric_metric(stats.get("kills").unwrap_or(&Value::Null))?;
    let deaths = numeric_metric(stats.get("deaths").unwrap_or(&Value::Null))?;
    let assists = numeric_metric(stats.get("assists").unwrap_or(&Value::Null))?;
    let games = numeric_metric(stats.get("wins").unwrap_or(&Value::Null)).unwrap_or(0.0)
        + numeric_metric(stats.get("losses").unwrap_or(&Value::Null)).unwrap_or(0.0);
    if kills + deaths + assists == 0.0 && games == 0.0 {
        return None;
    }
    Some(format!("{:.2}", (kills + assists / 2.0) / deaths.max(1.0)))
}

/// Tier name with Master/Grandmaster leaderboard handling — mirrors TS tierName.
fn tier_name_profile(tier: &Value, rank: &Value) -> String {
    let value = numeric_metric(tier).unwrap_or(0.0) as i64;
    let leaderboard_rank = numeric_metric(rank).unwrap_or(0.0) as i64;
    if value == 26 && leaderboard_rank > 0 && leaderboard_rank <= 100 {
        return format!("Grandmaster #{}", leaderboard_rank);
    }
    if value == 26 {
        return if leaderboard_rank > 100 {
            format!("Master #{}", leaderboard_rank - 100)
        } else {
            "Master".to_string()
        };
    }
    const NAMES: &[&str] = &[
        "",
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
    ];
    if value >= 1 && (value as usize) < NAMES.len() {
        NAMES[value as usize].to_string()
    } else {
        "Unranked".to_string()
    }
}

/// Build a ranked code-block field, or None when there is no data — mirrors TS rankedField.
fn ranked_field(
    label: &str,
    tier: &Value,
    rank: &Value,
    points: &Value,
    wins: &Value,
    losses: &Value,
    leaves: &Value,
) -> Option<EmbedField> {
    let value = numeric_metric(tier).unwrap_or(0.0) as i64;
    let games = numeric_metric(wins).unwrap_or(0.0) + numeric_metric(losses).unwrap_or(0.0);
    let points_n = numeric_metric(points).unwrap_or(0.0);
    if value <= 0 && games <= 0.0 && points_n <= 0.0 {
        return None;
    }
    let win_n = numeric_metric(wins).unwrap_or(0.0);
    let loss_n = numeric_metric(losses).unwrap_or(0.0);
    let mut lines = vec![
        stat_line("Rank", tier_name_profile(tier, rank)),
        stat_line("TP", format_number(points_n)),
    ];
    if let Some(wr) = format_percent(win_n, loss_n) {
        lines.push(stat_line(
            "Win rate",
            format!(
                "{} ({}-{})",
                wr,
                format_number(win_n),
                format_number(loss_n)
            ),
        ));
    }
    if let Some(l) = numeric_metric(leaves) {
        if l > 0.0 {
            lines.push(stat_line("Times deserted", format_number(l)));
        }
    }
    Some(EmbedField {
        name: label.to_string(),
        value: code_block(&lines),
        inline: false,
    })
}

/// Ranked performance code-block field — mirrors TS performanceField.
fn performance_field(player: &Value) -> Option<EmbedField> {
    let metrics: Vec<(String, f64)> = [
        ("DPM", "avg_dpm"),
        ("HPM", "avg_hpm"),
        ("MPM", "avg_mpm"),
        ("EGPM", "avg_egpm"),
    ]
    .iter()
    .filter_map(|(label, key)| {
        let v = numeric_metric(player.get(*key).unwrap_or(&Value::Null))?;
        if v > 0.0 {
            Some((label.to_string(), v))
        } else {
            None
        }
    })
    .collect();
    if metrics.is_empty() {
        return None;
    }
    let lines: Vec<String> = metrics
        .iter()
        .map(|(label, v)| stat_line(label, format_number(v.round())))
        .collect();
    Some(EmbedField {
        name: "Ranked performance".to_string(),
        value: code_block(&lines),
        inline: false,
    })
}

/// Playtime label — mirrors TS formatPlaytime.
fn format_playtime(hours: &Value, minutes: &Value) -> Option<String> {
    let total_hours = match numeric_metric(hours) {
        Some(h) => h,
        None => numeric_metric(minutes).unwrap_or(0.0) / 60.0,
    };
    if !total_hours.is_finite() || total_hours <= 0.0 {
        return None;
    }
    let rounded_hours = total_hours.floor() as i64;
    let days = rounded_hours / 24;
    if days > 0 {
        Some(format!(
            "{}d {}h ({} hours)",
            days,
            rounded_hours % 24,
            format_number(rounded_hours as f64)
        ))
    } else {
        Some(format!("{} hours", rounded_hours))
    }
}

/// Date label in UTC — mirrors TS formatDate (en-US locale, non-zero-padded day).
fn format_date(value: &Value) -> Option<String> {
    let s = value.as_str()?;
    let dt = chrono::DateTime::parse_from_rfc3339(s).ok()?;
    // en-US locale: "Aug 7, 2026" (non-zero-padded day)
    Some(format!(
        "{} {}, {}",
        month_short(dt.month0()),
        dt.day(),
        dt.year()
    ))
}

fn month_short(month: u32) -> &'static str {
    match month {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        12 => "Dec",
        _ => "Jan",
    }
}

// Keep the extension manifest single-sourced from the legacy bot.  Avatar IDs
// with animated assets must use the canonical GitHub URL for Discord to retain GIF animation.
const AVATAR_MANIFEST: &str = include_str!("../../discord-bot/src/paladins-avatar-assets.ts");
const AVATAR_ASSET_BASE: &str =
    "https://raw.githubusercontent.com/EthanHicks1/PaladinsArtAssets/master/avatars";

fn canonical_avatar_asset_url(avatar_id: &Value) -> Option<String> {
    let id = json_id(Some(avatar_id))?
        .parse::<i64>()
        .ok()
        .filter(|id| *id >= 0)?;
    let marker = format!("{id}: '");
    let start = AVATAR_MANIFEST.find(&marker)? + marker.len();
    let extension = AVATAR_MANIFEST[start..].split('\'').next()?;
    matches!(extension, "png" | "gif").then(|| format!("{AVATAR_ASSET_BASE}/{id}.{extension}"))
}

/// Resolve the player avatar URL — mirrors TS playerAvatarUrl.
fn player_avatar_url(value: &Value, avatar_id: &Value, web_url: &str) -> String {
    if let Some(url) = canonical_avatar_asset_url(avatar_id) {
        return url;
    }
    let raw = value.as_str().unwrap_or("").trim();
    if raw.starts_with("http://") || raw.starts_with("https://") {
        return raw.to_string();
    }
    format!(
        "{}/images/icons/Avatar_Default_Icon.png",
        web_url.trim_end_matches('/')
    )
}

/// Build player profile payload — mirrors TS buildPlayerProfileMessage.
pub fn build_player_profile(result: &Value, web_url: &str) -> Embed {
    let player = result.get("player").unwrap_or(result);
    let player_id = json_id(player.get("id")).unwrap_or_default();
    let player_name = compact(player.get("name").unwrap_or(&Value::Null), 256)
        .unwrap_or_else(|| "Unknown player".to_string());
    let title = compact(player.get("title").unwrap_or(&Value::Null), 220);
    let heading = match title {
        Some(t) => format!("{} ({})", player_name, t)
            .chars()
            .take(256)
            .collect(),
        None => player_name.clone(),
    };

    let mut fields: Vec<EmbedField> = Vec::new();

    // General
    let wins = numeric_metric(player.get("wins").unwrap_or(&Value::Null)).unwrap_or(0.0);
    let losses = numeric_metric(player.get("losses").unwrap_or(&Value::Null)).unwrap_or(0.0);
    let total_matches = (wins + losses) as i64;
    let record = format_percent(wins, losses);
    let kda = global_kda(result.get("globalStats").unwrap_or(&Value::Null));
    let mut general = vec![
        stat_line("Account ID", player_id.clone()),
        stat_line(
            "Account level",
            format_number(
                numeric_metric(player.get("level").unwrap_or(&Value::Null)).unwrap_or(0.0),
            ),
        ),
        stat_line(
            "Total XP",
            format_number(
                numeric_metric(player.get("total_xp").unwrap_or(&Value::Null)).unwrap_or(0.0),
            ),
        ),
        stat_line("Total matches", format_number(total_matches as f64)),
        stat_line(
            "Casual deserted",
            format_number(
                numeric_metric(player.get("leaves").unwrap_or(&Value::Null)).unwrap_or(0.0),
            ),
        ),
        stat_line(
            "Win rate",
            match record {
                Some(r) => format!("{} ({}-{})", r, format_number(wins), format_number(losses)),
                None => "—".to_string(),
            },
        ),
    ];
    if let Some(k) = kda {
        general.push(stat_line("Global KDA", k));
    }
    fields.push(EmbedField {
        name: "General".to_string(),
        value: code_block(&general),
        inline: false,
    });

    // Ranked KBM / Controller
    if let Some(f) = ranked_field(
        "Ranked KBM",
        player.get("kbm_tier").unwrap_or(&Value::Null),
        player.get("kbm_rank").unwrap_or(&Value::Null),
        player.get("kbm_points").unwrap_or(&Value::Null),
        player.get("kbm_wins").unwrap_or(&Value::Null),
        player.get("kbm_losses").unwrap_or(&Value::Null),
        player.get("kbm_leaves").unwrap_or(&Value::Null),
    ) {
        fields.push(f);
    }
    if let Some(f) = ranked_field(
        "Ranked Controller",
        player.get("controller_tier").unwrap_or(&Value::Null),
        player.get("controller_rank").unwrap_or(&Value::Null),
        player.get("controller_points").unwrap_or(&Value::Null),
        player.get("controller_wins").unwrap_or(&Value::Null),
        player.get("controller_losses").unwrap_or(&Value::Null),
        player.get("controller_leaves").unwrap_or(&Value::Null),
    ) {
        fields.push(f);
    }

    // Other
    let mut other = vec![
        stat_line(
            "Platform",
            compact(player.get("platform").unwrap_or(&Value::Null), 40)
                .unwrap_or_else(|| "Unknown".to_string()),
        ),
        stat_line(
            "Region",
            compact(player.get("region").unwrap_or(&Value::Null), 40)
                .unwrap_or_else(|| "Unknown".to_string()),
        ),
    ];
    if let Some(pt) = format_playtime(
        player.get("hours_played").unwrap_or(&Value::Null),
        player.get("minutes_played").unwrap_or(&Value::Null),
    ) {
        other.push(stat_line("Playtime", pt));
    }
    if let Some(m) = numeric_metric(player.get("mastery_level").unwrap_or(&Value::Null)) {
        if m > 0.0 {
            other.push(stat_line("Mastery level", format_number(m)));
        }
    }
    if let Some(a) = numeric_metric(player.get("total_achievements").unwrap_or(&Value::Null)) {
        if a > 0.0 {
            other.push(stat_line("Achievements", format_number(a)));
        }
    }
    if let Some(d) = format_date(player.get("created_datetime").unwrap_or(&Value::Null)) {
        other.push(stat_line("Account created", d));
    }
    if let Some(d) = format_date(player.get("last_login_datetime").unwrap_or(&Value::Null)) {
        other.push(stat_line("Last login", d));
    }
    if let Some(lf) = compact(player.get("loading_frame").unwrap_or(&Value::Null), 80) {
        other.push(stat_line("Loading frame", lf));
    }
    fields.push(EmbedField {
        name: "Other".to_string(),
        value: code_block(&other),
        inline: false,
    });

    // Performance
    if let Some(pf) = performance_field(player) {
        fields.push(pf);
    }

    let url = format!("{}/players/{}", web_url, player_id);
    let mut builder = EmbedBuilder::new()
        .color(ACCENT)
        .title(&heading)
        .url(&url)
        .footer(EmbedFooterBuilder::new("PaladinsCat"));
    for field in fields {
        builder = builder.field(field);
    }

    let avatar = player_avatar_url(
        player.get("avatar_url").unwrap_or(&Value::Null),
        player.get("avatar_id").unwrap_or(&Value::Null),
        web_url,
    );
    if let Ok(src) = ImageSource::url(&avatar) {
        builder = builder.thumbnail(src);
    }

    let refreshed = result
        .get("profileRefresh")
        .and_then(|v| v.get("refreshed_at"))
        .or_else(|| player.get("last_updated"))
        .and_then(|v| v.as_str());
    if let Some(ts_str) = refreshed {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts_str) {
            if let Ok(ts) = Timestamp::from_secs(dt.timestamp()) {
                builder = builder.timestamp(ts);
            }
        }
    }

    builder.build()
}

/// Build loadout selection payload — mirrors TS buildLoadoutSelectionPayload
/// Returns embed for displaying loadout choices to user.
pub fn build_loadout_selection_payload(
    player_name: &str,
    champion_name: &str,
    count: usize,
    web_url: &str,
    player_id: &str,
    refreshed: bool,
) -> Embed {
    let description = format!(
        "Choose one of **{}** saved loadout{} below to generate its image.",
        count,
        if count == 1 { "" } else { "s" }
    );
    let footer_text = if refreshed {
        "Saved loadouts refreshed from Paladins before this result."
    } else {
        "Served from the PaladinsCat database."
    };

    let url = format!("{}/players/{}/loadouts", web_url, player_id);
    EmbedBuilder::new()
        .color(ACCENT)
        .title(&format!("{} · {}", player_name, champion_name))
        .url(&url)
        .description(&description)
        .footer(EmbedFooterBuilder::new(footer_text))
        .build()
}

/// Build no-loadouts payload — mirrors TS buildNoLoadoutsPayload
pub fn build_no_loadouts_payload(
    player_name: &str,
    champion_name: &str,
    refresh_error: Option<&str>,
) -> Embed {
    let suffix = if let Some(err) = refresh_error {
        if !err.to_lowercase().contains("cooldown") {
            "\nThe refresh did not complete, so this result may be stale."
        } else {
            ""
        }
    } else {
        ""
    };
    let description = format!("No saved loadouts found for this champion.{}", suffix);
    simple_embed(
        &format!("{} · {}", player_name, champion_name),
        &description,
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn clean_discord_text_escapes_markdown() {
        let v = json!("a*b_c`d");
        assert_eq!(clean_discord_text(&v, ""), "a\\*b\\_c\\`d");
    }

    #[test]
    fn format_number_groups_thousands() {
        assert_eq!(format_number(1234567.0), "1,234,567");
        assert_eq!(format_number(42.0), "42");
    }

    #[test]
    fn history_payload_formats_lines() {
        let rows = vec![json!({
            "win_status": "Winner",
            "map": "Ranked Siege",
            "duration_seconds": 1500,
            "region": "EU",
            "champion_name": "Ying",
            "kills": 10, "deaths": 3, "assists": 5,
            "match_id": "12345",
        })];
        let embed = build_history_payload("Ylva", &rows, "https://paladinscat.com");
        assert_eq!(embed.title.as_deref(), Some("Ylva · Recent matches"));
        let desc = embed.description.unwrap();
        assert!(desc.contains("✅"));
        assert!(desc.contains("10/3/5"));
        assert!(desc.contains("[12345](https://paladinscat.com/matches/12345)"));
    }

    #[test]
    fn player_profile_has_general_field_and_footer() {
        let result = json!({
            "player": {
                "id": "42",
                "name": "Ylva",
                "level": 120,
                "total_xp": 500000,
                "wins": 100,
                "losses": 50,
                "leaves": 2,
                "platform": "PC",
                "region": "EU",
                "kbm_tier": 26,
                "kbm_rank": 5,
                "kbm_points": 3500,
                "kbm_wins": 60,
                "kbm_losses": 30,
                "kbm_leaves": 0,
            },
            "globalStats": { "kills": 1000, "deaths": 500, "assists": 800, "wins": 100, "losses": 50 },
        });
        let embed = build_player_profile(&result, "https://paladinscat.com");
        assert_eq!(embed.title.as_deref(), Some("Ylva"));
        assert_eq!(
            embed.url.as_deref(),
            Some("https://paladinscat.com/players/42")
        );
        assert_eq!(
            embed.footer.as_ref().map(|f| f.text.as_str()),
            Some("PaladinsCat")
        );
        let names: Vec<&str> = embed.fields.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"General"));
        assert!(names.contains(&"Ranked KBM"));
        assert!(names.contains(&"Other"));
        let general = embed.fields.iter().find(|f| f.name == "General").unwrap();
        assert!(general.value.contains("Account ID"));
        let kbm = embed
            .fields
            .iter()
            .find(|f| f.name == "Ranked KBM")
            .unwrap();
        assert!(kbm.value.contains("Grandmaster #5"));
    }

    #[test]
    fn current_payload_renders_teams_and_win_chance() {
        let result = json!({
            "match": { "match_id": "9", "queue_id": 486, "map": "Ranked Siege", "region": "EU", "detected_at": "2024-01-01T00:00:00Z" },
            "player_id": "1",
            "players": [
                { "player_id": "1", "player_name": "A", "champion_name": "Ying", "task_force": 1, "kbm_tier": 11, "queue_elo": 2000, "profile_win_rate": 55.0 },
                { "player_id": "2", "player_name": "B", "champion_name": "Khan", "task_force": 1, "kbm_tier": 11, "queue_elo": 2000, "profile_win_rate": 55.0 },
                { "player_id": "3", "player_name": "C", "champion_name": "Viktor", "task_force": 1, "kbm_tier": 11, "queue_elo": 2000, "profile_win_rate": 55.0 },
                { "player_id": "4", "player_name": "D", "champion_name": "Maeve", "task_force": 2, "kbm_tier": 11, "queue_elo": 2000, "profile_win_rate": 55.0 },
                { "player_id": "5", "player_name": "E", "champion_name": "Fernando", "task_force": 2, "kbm_tier": 11, "queue_elo": 2000, "profile_win_rate": 55.0 },
                { "player_id": "6", "player_name": "F", "champion_name": "Cassie", "task_force": 2, "kbm_tier": 11, "queue_elo": 2000, "profile_win_rate": 55.0 },
            ],
        });
        let embed = build_current_payload(&result, "https://paladinscat.com");
        assert_eq!(embed.title.as_deref(), Some("Siege · Live match"));
        assert!(embed.fields[0].name.contains("win chance"));
        assert!(embed.fields[0].value.contains("▸ **Ying**"));
        assert!(embed.fields[0]
            .value
            .contains("[A](https://paladinscat.com/players/1)"));
        assert!(embed.timestamp.is_some());
    }

    #[test]
    fn history_and_live_payloads_accept_numeric_ids() {
        let history = vec![json!({"match_id": 123, "win_status":"Winner", "champion_name":"Ying"})];
        assert!(
            build_history_payload("A", &history, "https://paladinscat.com")
                .description
                .unwrap()
                .contains("/matches/123")
        );
        let live = json!({"match":{"match_id":9,"queue_id":486},"player_id":1,"players":[{"player_id":1,"player_name":"A","champion_name":"Ying","task_force":1}]});
        assert!(
            build_current_payload(&live, "https://paladinscat.com").fields[0]
                .value
                .contains("▸ **Ying**")
        );
    }

    #[test]
    fn canonical_avatar_assets_preserve_manifest_extensions() {
        assert_eq!(canonical_avatar_asset_url(&json!(9918)).as_deref(), Some("https://raw.githubusercontent.com/EthanHicks1/PaladinsArtAssets/master/avatars/9918.png"));
        assert_eq!(canonical_avatar_asset_url(&json!(23226)).as_deref(), Some("https://raw.githubusercontent.com/EthanHicks1/PaladinsArtAssets/master/avatars/23226.gif"));
        assert!(canonical_avatar_asset_url(&json!(999999)).is_none());
    }
}
