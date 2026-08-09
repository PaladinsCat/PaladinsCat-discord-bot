//! HTML template data binding — builds data-bound scoreboard/loadout documents.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine as _;

use super::asset_catalog::AssetCatalog;

#[derive(Debug, Clone)]
pub struct TemplateConfig {
    pub match_template_path: String,
    pub loadout_template_path: String,
    pub cheater_pattern_path: String,
    pub asset_root_path: String,
}

impl TemplateConfig {
    pub fn dev_defaults() -> Self {
        Self {
            match_template_path: "dev/prototypes/match-result-scoreboard.html".into(),
            loadout_template_path: "dev/prototypes/loadout-card-layout.html".into(),
            cheater_pattern_path: "dev/prototypes/cheater-police-line.svg".into(),
            asset_root_path: "src/frontend/public/images".into(),
        }
    }
}

#[derive(Clone)]
pub struct TemplateEngine {
    match_template: Arc<String>,
    loadout_template: Arc<String>,
    cheater_pattern_url: String,
    assets: AssetCatalog,
}

// ---------------------------------------------------------------------------
// Scoreboard field helpers — mirror the TS match-renderer.ts formatting.
// ---------------------------------------------------------------------------

fn num(value: Option<&serde_json::Value>) -> i64 {
    value.and_then(|v| v.as_i64()).unwrap_or(0)
}

fn num_f64(value: Option<&serde_json::Value>) -> f64 {
    value.and_then(|v| v.as_f64()).unwrap_or(0.0)
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

fn compact(value: i64) -> String {
    if value.abs() >= 1000 {
        format!("{:.1}k", value as f64 / 1000.0)
    } else {
        value.to_string()
    }
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
        let cheater_pattern_url: String =
            if fs::exists(&config.cheater_pattern_path).unwrap_or(false) {
                let svg = fs::read_to_string(&config.cheater_pattern_path).unwrap_or_default();
                format!("data:image/svg+xml,{}", url_encode(&svg))
            } else {
                String::new()
            };
        Ok(Self {
            match_template: Arc::new(match_template),
            loadout_template: Arc::new(loadout_template),
            cheater_pattern_url,
            assets: AssetCatalog::new(&config.asset_root_path),
        })
    }

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

    fn asset_url(&self, path: Option<PathBuf>) -> String {
        const TRANSPARENT: &str =
            "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg'/%3E";
        let Some(path) = path else {
            return TRANSPARENT.to_string();
        };
        let Ok(bytes) = fs::read(&path) else {
            return TRANSPARENT.to_string();
        };
        let mime = match path
            .extension()
            .and_then(|ext| ext.to_str())
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
        };
        format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        )
    }

    /// Build a complete, data-bound scoreboard document with a real
    /// `#scoreboard` element. The static design prototype is not injected
    /// wholesale — we reuse its CSS and build the hero / columns / team rows /
    /// summary from the record JSON.
    pub fn match_document(&self, data: &serde_json::Value) -> String {
        let css = Self::extract_css(self.match_template.as_ref());
        let cheater_css = if self.cheater_pattern_url.is_empty() {
            String::new()
        } else {
            format!(
                "body{{--cheater-pattern:url(\"{}\")}}",
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
             body{{min-height:720px;padding:0;background:transparent}}.scoreboard{{transform:none}}\
             .scoreboard-canvas{{width:1280px;height:720px}}.viewport{{width:1280px;max-width:none}}\
             .prototype-note,.color-lab{{display:none}}.talent-empty{{display:grid;place-items:center;color:#8f9bad;font-size:18px;font-weight:700}}</style>\
             </head><body data-theme=\"dark\"><main class=\"viewport\"><div class=\"scoreboard-canvas\">\
             <section class=\"scoreboard\" id=\"scoreboard\" aria-label=\"Paladins match scoreboard\">{board}</section>\
             </div></main></body></html>",
            cheater = cheater_css,
            map_css = map_css,
        )
    }

    /// Build scoreboard section markup (hero + columns + both teams), mirroring
    /// the TS `document()` inner structure.
    fn scoreboard_markup(&self, data: &serde_json::Value) -> String {
        let match_obj = data.get("match");
        let players = data
            .get("players")
            .and_then(|p| p.as_array())
            .cloned()
            .unwrap_or_default();
        let queue_id = num(match_obj.and_then(|m| m.get("queue_id")));
        let ranked = queue_id == 486;
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

        let hero = self.hero_markup(match_obj, &players, ranked, &sorted_bans, split);
        let columns = "<div class=\"columns grid-row\"><div>Party</div><div></div><div>Level</div><div>Player</div><div>Elo</div><div>Talent</div><div>Credits</div><div>K / D / A</div><div>OB. Time</div><div>Damage</div><div>Taken</div><div>Shielding</div><div>Healing</div></div>";
        let team_one_rows = self.team_rows(&players, &facts, 1);
        let team_one_summary = self.team_summary(match_obj, &players, 1);
        let team_two_rows = self.team_rows(&players, &facts, 2);
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
        ranked: bool,
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
        let mode = if ranked { "Ranked" } else { "Casual" };
        let map_class = if map_name.len() > 19 {
            "map-name long"
        } else {
            "map-name"
        };

        let broken = bool_of(match_obj.and_then(|m| m.get("broken")));
        let recovered = bool_of(match_obj.and_then(|m| m.get("recovered")));
        let private = bool_of(match_obj.and_then(|m| m.get("private")));
        let mut status = format!(
            "<span class=\"status-tag {}\">{}</span>",
            if ranked { "ranked" } else { "casual" },
            mode
        );
        if broken && !recovered {
            status.push_str("<span class=\"status-tag broken\">Broken</span>");
        }
        if recovered {
            status.push_str("<span class=\"status-tag recovered\">Recovered</span>");
        }
        if private {
            status.push_str("<span class=\"status-tag private\">Private</span>");
        }

        let team1_score = num(match_obj.and_then(|m| m.get("team1_score")));
        let team2_score = num(match_obj.and_then(|m| m.get("team2_score")));
        let score1 = if team1_score == 0 {
            "?".to_string()
        } else {
            team1_score.to_string()
        };
        let score2 = if team2_score == 0 {
            "?".to_string()
        } else {
            team2_score.to_string()
        };

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
            "<div class=\"tier-meta\"><img src=\"{}\" alt=\"{}\"/><div><div class=\"meta-value\">{}</div><div class=\"meta-label\">Avg tier</div></div></div>",
            tier_icon, escape_html(&tier_name), escape_html(&tier_name)
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
            mode = mode,
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
        let sum: i64 = players.iter().map(player_display_tier).sum();
        ((sum as f64) / players.len() as f64).round() as i64
    }

    fn team_rows(
        &self,
        players: &[serde_json::Value],
        facts: &[serde_json::Value],
        team: i64,
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
            let suspicious = !cheater && sus_count > 0;
            let verified = bool_of(player.get("verified"))
                || bool_of(
                    player
                        .get("profile_snapshot")
                        .and_then(|p| p.get("verified")),
                );

            let verification_badge = if verified {
                format!(
                    "<img class=\"verified-player-icon\" src=\"{}\" alt=\"Verified PaladinsCat player\"/>",
                    self.asset_url(self.assets.icon("Verified_Player_Support_Icon", Some("png")))
                )
            } else {
                String::new()
            };
            let moderation_tag = if cheater {
                "<span class=\"player-status-tag cheater\">CHEATER</span>".to_string()
            } else if suspicious {
                "<span class=\"player-status-tag suspicious\">SUS</span>".to_string()
            } else {
                String::new()
            };
            let party_number = num(player.get("party_number"));
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
                values[0],
                kda,
                peak(1, true),
                values[1],
                peak(2, true),
                values[2],
                peak(3, true),
                values[3],
                peak(4, true),
                values[4],
                peak(5, true),
                values[5],
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
                level = level,
                name = escape_html(&str_of(player.get("player_name"))),
                vb = verification_badge,
                mt = moderation_tag,
                pid = escape_html(&pid),
                elo = if elo == 0 { "—".to_string() } else { elo.to_string() },
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
            level_avg = level_avg,
            elo_avg = elo_avg,
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

    pub fn loadout_document(&self, data: &serde_json::Value) -> String {
        let json_str = escape_html(&serde_json::to_string(data).unwrap_or_default());
        let tmpl = self.loadout_template.as_ref();
        if let Some(pos) = tmpl.find("</head>") {
            format!(
                "{}<script>var __renderData={};</script>{}",
                &tmpl[..pos],
                json_str,
                &tmpl[pos..]
            )
        } else {
            format!("<script>var __renderData={};</script>{}", json_str, tmpl)
        }
    }

    pub fn cheater_pattern_url(&self) -> &str {
        &self.cheater_pattern_url
    }
}

/// Strip leading queue tokens (Ranked/Live/WIP + numbers) from a map name,
/// mirroring the TS normalization.
fn regex_strip_prefix(s: &str) -> String {
    let mut result = s.to_string();
    for pat in ["Ranked ", "Live ", "WIP "] {
        if result.starts_with(pat) {
            result = result[pat.len()..].to_string();
            break;
        }
    }
    result
}

fn utc_timestamp_str(s: &str) -> String {
    s.replace('T', " · ").replace('Z', " UTC")
}

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
            loadout_template: Arc::new(String::new()),
            cheater_pattern_url: String::new(),
            assets: AssetCatalog::new("missing-test-assets"),
        };
        let doc = engine.match_document(&record);
        assert!(doc.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(!doc.contains("<script>alert(1)</script>"));
    }

    #[test]
    fn extract_css_removes_external_font_import_and_style_tag() {
        let css = TemplateEngine::extract_css(
            "<style>\n@import url('https://fonts.googleapis.com/css2?family=Inter');\n.player-row{display:grid}\n</style>",
        );
        assert_eq!(css.trim(), ".player-row{display:grid}");
    }
}
