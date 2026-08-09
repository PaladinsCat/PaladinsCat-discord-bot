//! Slash command registration — replaces command-registration.ts

use std::collections::HashSet;
use twilight_http::Client;
use twilight_model::application::command::{
    Command, CommandOption, CommandOptionChoice, CommandOptionChoiceValue, CommandOptionType,
    CommandType,
};
use twilight_model::id::marker::{ApplicationMarker, GuildMarker};
use twilight_model::id::Id;

/// Result of a registration run.
#[derive(Debug, Clone)]
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
    string_option("id", "Match ID", true)
}

pub fn all_command_definitions() -> Vec<Command> {
    vec![
        command("help", "List PaladinsCat bot commands", vec![]),
        command(
            "save",
            "Save your default Paladins player",
            vec![string_option("player", "Player name or ID", true)],
        ),
        command(
            "profile",
            "Show a Paladins player profile",
            vec![option_player()],
        ),
        command(
            "player",
            "Show a Paladins player profile",
            vec![option_player()],
        ),
        command(
            "match",
            "Render a match result image",
            vec![option_match_id()],
        ),
        command(
            "history",
            "Show recent matches for a player",
            vec![option_player()],
        ),
        command(
            "current",
            "Check a player's current live match",
            vec![option_player()],
        ),
        command(
            "loadout",
            "Render one of a player's saved champion loadouts",
            vec![option_champion(), option_player()],
        ),
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
    ]
}

#[allow(dead_code)] // Kept for manual registration scenarios
pub async fn register_global_commands(
    http: &Client,
    application_id: Id<ApplicationMarker>,
) -> Result<RegistrationResult, twilight_http::Error> {
    let commands = all_command_definitions();
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

pub async fn register_guild_commands(
    http: &Client,
    application_id: Id<ApplicationMarker>,
    guild_id: Id<GuildMarker>,
) -> Result<RegistrationResult, twilight_http::Error> {
    let commands = all_command_definitions();
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

pub async fn register_commands(
    http: &Client,
    application_id: Id<ApplicationMarker>,
    development_guild_id: Option<Id<GuildMarker>>,
    connected_guild_ids: &[Id<GuildMarker>],
) -> Result<RegistrationResult, twilight_http::Error> {
    if let Some(guild_id) = development_guild_id {
        register_guild_commands(http, application_id, guild_id).await
    } else {
        register_and_clear_guilds(http, application_id, connected_guild_ids).await
    }
}

async fn register_and_clear_guilds(
    http: &Client,
    application_id: Id<ApplicationMarker>,
    guild_ids: &[Id<GuildMarker>],
) -> Result<RegistrationResult, twilight_http::Error> {
    let commands = all_command_definitions();
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
        assert_eq!(all_command_definitions().len(), 12);
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
}
