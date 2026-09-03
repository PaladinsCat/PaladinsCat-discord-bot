//! Slash command registration — replaces command-registration.ts
//! refs: none

use std::collections::HashSet;
use twilight_http::Client;
use twilight_model::application::command::{
    Command, CommandOption, CommandOptionChoice, CommandOptionChoiceValue, CommandOptionType,
    CommandType,
};
use twilight_model::id::marker::{ApplicationMarker, GuildMarker};
use twilight_model::id::Id;

/// Result of a registration run.
/// refs: none
#[derive(Debug, Clone)]
/// Define RegistrationResult.
///
/// Contract: accepts the arguments shown in the signature and returns the documented result; side effects follow the implementation.
///
/// refs: none
pub struct RegistrationResult {
    pub scope: String,
    pub registered: usize,
    pub cleared_guild_scopes: usize,
    pub failed_guild_scopes: usize,
}

const RANKED_LOBBY_CHOICES: &[(&str, &str)] = &[
    ("Global ranked lobbies", "global"),
    ("Bronze–Gold lobbies", "bronze-gold"),
    ("Platinum+ lobbies", "platinum"),
    ("Diamond+ lobbies", "diamond"),
];

fn command(name: &str, description: &str, options: Vec<CommandOption>) -> Command {
    Command {
        id: None,
        application_id: None,
        guild_id: None,
        kind: CommandType::ChatInput,
        name: name.to_string(),
        description: description.to_string(),
        options,
        // Match the legacy SlashCommandBuilder output: leave context defaults
        // to Discord instead of accidentally excluding Bot DMs.
        contexts: None,
        integration_types: None,
        default_member_permissions: None,
        #[allow(deprecated)]
        dm_permission: None,
        name_localizations: None,
        description_localizations: None,
        nsfw: Some(false),
        version: Id::new(1),
    }
}

fn user_command(name: &str) -> Command {
    let mut command = command(name, "", vec![]);
    command.kind = CommandType::User;
    command
}

fn string_option(name: &str, description: &str, required: bool) -> CommandOption {
    CommandOption {
        name: name.to_string(),
        description: description.to_string(),
        required: Some(required),
        kind: CommandOptionType::String,
        options: None,
        choices: None,
        autocomplete: None,
        channel_types: None,
        min_value: None,
        max_value: None,
        min_length: None,
        max_length: None,
        name_localizations: None,
        description_localizations: None,
    }
}

fn string_option_with_choices(
    name: &str,
    description: &str,
    required: bool,
    choices: Vec<CommandOptionChoice>,
) -> CommandOption {
    CommandOption {
        name: name.to_string(),
        description: description.to_string(),
        required: Some(required),
        kind: CommandOptionType::String,
        options: None,
        choices: Some(choices),
        autocomplete: None,
        channel_types: None,
        min_value: None,
        max_value: None,
        min_length: None,
        max_length: None,
        name_localizations: None,
        description_localizations: None,
    }
}

fn string_option_autocomplete(name: &str, description: &str, required: bool) -> CommandOption {
    CommandOption {
        name: name.to_string(),
        description: description.to_string(),
        required: Some(required),
        kind: CommandOptionType::String,
        options: None,
        choices: None,
        autocomplete: Some(true),
        channel_types: None,
        min_value: None,
        max_value: None,
        min_length: None,
        max_length: None,
        name_localizations: None,
        description_localizations: None,
    }
}

fn boolean_option(name: &str, description: &str) -> CommandOption {
    let mut option = string_option(name, description, false);
    option.kind = CommandOptionType::Boolean;
    option
}

fn integer_option(name: &str, description: &str) -> CommandOption {
    let mut option = string_option(name, description, false);
    option.kind = CommandOptionType::Integer;
    option
}

fn choices(values: &[(&str, &str)]) -> Vec<CommandOptionChoice> {
    values
        .iter()
        .map(|(name, value)| CommandOptionChoice {
            name: (*name).to_string(),
            name_localizations: None,
            value: CommandOptionChoiceValue::String((*value).to_string()),
        })
        .collect()
}

fn option_slot(required: bool, include_all: bool) -> CommandOption {
    let mut values = vec![
        ("Primary", "primary"),
        ("Alternate 1", "alt1"),
        ("Alternate 2", "alt2"),
    ];
    if include_all {
        values.push(("All saved players", "all"));
    }
    string_option_with_choices("slot", "Saved player slot", required, choices(&values))
}

fn lobby_choices() -> Vec<CommandOptionChoice> {
    RANKED_LOBBY_CHOICES
        .iter()
        .map(|(name, value)| CommandOptionChoice {
            name: name.to_string(),
            name_localizations: None,
            value: CommandOptionChoiceValue::String(value.to_string()),
        })
        .collect()
}

fn option_champion() -> CommandOption {
    string_option_autocomplete("champion", "Champion name", true)
}

fn option_lobby() -> CommandOption {
    string_option_with_choices(
        "lobby",
        "Ranked lobby tier; choose Global for all ranks",
        true,
        lobby_choices(),
    )
}

fn option_player() -> CommandOption {
    string_option("player", "Player name or ID", false)
}

fn option_match_id() -> CommandOption {
    string_option(
        "id",
        "Match ID (defaults to your saved player's latest match)",
        false,
    )
}

/// Build the full command set (social commands included when enabled).
///
/// I/O: `bool` (social enabled) -> `Vec<Command>`
/// refs: none
pub fn all_command_definitions(social_commands_enabled: bool) -> Vec<Command> {
    let mut commands = vec![
        command("help", "List PaladinsCat bot commands", vec![]),
        command(
            "save",
            "Save a Paladins player",
            vec![
                string_option("player", "Player name or ID", true),
                option_slot(false, false),
            ],
        ),
        command(
            "forget",
            "Delete a saved Paladins player link",
            vec![option_slot(false, true)],
        ),
        command(
            "privacy",
            "Show what the bot stores and how to delete it",
            vec![],
        ),
        command(
            "profile",
            "Show a Paladins player profile",
            vec![option_player(), option_slot(false, false)],
        ),
        command(
            "player",
            "Show a Paladins player profile",
            vec![option_player(), option_slot(false, false)],
        ),
        command(
            "match",
            "Render a match result image",
            vec![option_match_id()],
        ),
        command(
            "history",
            "Show recent matches for a player",
            vec![
                option_player(),
                option_slot(false, false),
                string_option_with_choices(
                    "queue",
                    "Queue filter",
                    false,
                    choices(&[
                        ("Ranked", "486"),
                        ("Siege", "424"),
                        ("Onslaught", "452"),
                        ("Team Deathmatch", "469"),
                    ]),
                ),
                string_option_autocomplete("champion", "Champion filter", false),
                string_option_with_choices(
                    "result",
                    "Match result filter",
                    false,
                    choices(&[("Wins", "Winner"), ("Losses", "Loser")]),
                ),
                integer_option("page", "History page (10 matches per page)"),
            ],
        ),
        command(
            "current",
            "Check a player's current live match",
            vec![
                option_player(),
                option_slot(false, false),
                boolean_option("details", "Show champion-specific Elo, win rate, and KDA"),
            ],
        ),
        command(
            "loadout",
            "Render one of a player's saved champion loadouts",
            vec![
                option_champion(),
                option_player(),
                option_slot(false, false),
            ],
        ),
        command(
            "champions",
            "Show a player's champion statistics",
            vec![
                option_player(),
                option_slot(false, false),
                string_option_with_choices(
                    "sort",
                    "Sort champion statistics",
                    false,
                    choices(&[
                        ("Matches", "matches"),
                        ("Win rate", "winrate"),
                        ("KDA", "kda"),
                    ]),
                ),
                string_option_with_choices(
                    "role",
                    "Champion role",
                    false,
                    choices(&[
                        ("Frontline", "Frontline"),
                        ("Damage", "Damage"),
                        ("Flank", "Flank"),
                        ("Support", "Support"),
                    ]),
                ),
            ],
        ),
        command(
            "leaderboard",
            "Show PaladinsCat leaderboards",
            vec![
                string_option_with_choices(
                    "category",
                    "Leaderboard type",
                    true,
                    choices(&[
                        ("Performance", "performance"),
                        ("Class Elo", "class"),
                        ("Champion Elo", "champion"),
                    ]),
                ),
                string_option_with_choices(
                    "metric",
                    "Performance metric",
                    false,
                    choices(&[
                        ("Damage per minute", "dpm"),
                        ("Healing per minute", "hpm"),
                        ("Credits per minute", "gpm"),
                        ("Mitigation per minute", "mpm"),
                    ]),
                ),
                string_option_with_choices(
                    "role",
                    "Role",
                    false,
                    choices(&[
                        ("Frontline", "Frontline"),
                        ("Damage", "Damage"),
                        ("Flank", "Flank"),
                        ("Support", "Support"),
                    ]),
                ),
                string_option_autocomplete("champion", "Champion for champion Elo", false),
            ],
        ),
        command("activity", "Show observed Paladins player activity", vec![]),
        command("status", "Show Paladins service status", vec![]),
        command(
            "champion",
            "Show champion ranked statistics",
            vec![option_champion(), option_lobby()],
        ),
        command("maps", "Show statistics for every ranked map", vec![]),
        command(
            "composition",
            "Show the five most-played ranked team compositions",
            vec![],
        ),
        command(
            "items",
            "Show global ranked item statistics",
            vec![option_lobby()],
        ),
        user_command("Paladins Profile"),
        user_command("Paladins History"),
        user_command("Paladins Current"),
    ];
    if social_commands_enabled {
        commands.push(command(
            "random",
            "Pick a random champion, map, or balanced role team",
            vec![
                string_option_with_choices(
                    "kind",
                    "What to generate",
                    true,
                    choices(&[
                        ("Champion", "champion"),
                        ("Map", "map"),
                        ("Role team", "team"),
                    ]),
                ),
                string_option_with_choices(
                    "role",
                    "Champion role filter",
                    false,
                    choices(&[
                        ("Frontline", "Frontline"),
                        ("Damage", "Damage"),
                        ("Flank", "Flank"),
                        ("Support", "Support"),
                    ]),
                ),
            ],
        ));
        commands.push(command(
            "teams",
            "Split your current voice channel into two random teams",
            vec![],
        ));
    }
    commands
}

#[allow(dead_code)] // Kept for manual registration scenarios
/// Register the global command set with Discord.
///
/// I/O: `&Client`, `Id<ApplicationMarker>` -> `Result<RegistrationResult, twilight_http::Error>`
/// refs: none
pub async fn register_global_commands(
    http: &Client,
    application_id: Id<ApplicationMarker>,
) -> Result<RegistrationResult, twilight_http::Error> {
    let commands = all_command_definitions(false);
    http.interaction(application_id)
        .set_global_commands(&commands)
        .await?;
    Ok(RegistrationResult {
        scope: "global".to_string(),
        registered: commands.len(),
        cleared_guild_scopes: 0,
        failed_guild_scopes: 0,
    })
}

/// Register the command set for a specific guild.
///
/// I/O: `&Client`, `Id<ApplicationMarker>`, `Id<GuildMarker>`, `bool` (social enabled) -> `Result<RegistrationResult, twilight_http::Error>`
/// refs: none
pub async fn register_guild_commands(
    http: &Client,
    application_id: Id<ApplicationMarker>,
    guild_id: Id<GuildMarker>,
    social_commands_enabled: bool,
) -> Result<RegistrationResult, twilight_http::Error> {
    let commands = all_command_definitions(social_commands_enabled);
    http.interaction(application_id)
        .set_guild_commands(guild_id, &commands)
        .await?;
    Ok(RegistrationResult {
        scope: "guild".to_string(),
        registered: commands.len(),
        cleared_guild_scopes: 0,
        failed_guild_scopes: 0,
    })
}

/// Remove all registered commands from a guild.
///
/// I/O: `&Client`, `Id<ApplicationMarker>`, `Id<GuildMarker>` -> `Result<(), twilight_http::Error>`
/// refs: none
pub async fn clear_guild_commands(
    http: &Client,
    application_id: Id<ApplicationMarker>,
    guild_id: Id<GuildMarker>,
) -> Result<(), twilight_http::Error> {
    http.interaction(application_id)
        .set_guild_commands(guild_id, &[])
        .await?;
    Ok(())
}

/// Register commands globally, or for a development guild when one is set.
///
/// I/O: `&Client`, `Id<ApplicationMarker>`, `Option<Id<GuildMarker>>` (dev guild), `&[Id<GuildMarker]]` (connected), `bool` (social enabled) -> `Result<RegistrationResult, twilight_http::Error>`
/// refs: none
pub async fn register_commands(
    http: &Client,
    application_id: Id<ApplicationMarker>,
    development_guild_id: Option<Id<GuildMarker>>,
    connected_guild_ids: &[Id<GuildMarker>],
    social_commands_enabled: bool,
) -> Result<RegistrationResult, twilight_http::Error> {
    if let Some(guild_id) = development_guild_id {
        register_guild_commands(http, application_id, guild_id, social_commands_enabled).await
    } else {
        register_and_clear_guilds(
            http,
            application_id,
            connected_guild_ids,
            social_commands_enabled,
        )
        .await
    }
}

async fn register_and_clear_guilds(
    http: &Client,
    application_id: Id<ApplicationMarker>,
    guild_ids: &[Id<GuildMarker>],
    social_commands_enabled: bool,
) -> Result<RegistrationResult, twilight_http::Error> {
    let commands = all_command_definitions(social_commands_enabled);
    http.interaction(application_id)
        .set_global_commands(&commands)
        .await?;

    let mut cleared = 0;
    let mut failed = 0;
    let mut seen = HashSet::new();
    for guild_id in guild_ids.iter().filter(|id| seen.insert(id.get())) {
        match clear_guild_commands(http, application_id, *guild_id).await {
            Ok(()) => cleared += 1,
            Err(_) => failed += 1,
        }
    }

    Ok(RegistrationResult {
        scope: "global".to_string(),
        registered: commands.len(),
        cleared_guild_scopes: cleared,
        failed_guild_scopes: failed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_count() {
        assert_eq!(all_command_definitions(false).len(), 21);
        assert_eq!(all_command_definitions(true).len(), 23);
    }

    #[test]
    fn test_champion_has_autocomplete() {
        let champ = option_champion();
        assert!(champ.autocomplete.unwrap_or(false));
    }

    #[test]
    fn test_lobby_has_choices() {
        let lobby = option_lobby();
        assert_eq!(lobby.choices.as_ref().unwrap().len(), 4);
    }

    #[test]
    fn match_id_is_optional_for_saved_player_fallback() {
        assert_eq!(option_match_id().required, Some(false));
    }
}
