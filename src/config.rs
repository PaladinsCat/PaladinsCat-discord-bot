//! Bot configuration — replaces config.ts
//!
//! Environment-based configuration loading with sensible defaults.

#[derive(Debug, Clone)]
/// Define Config.
///
/// Contract: accepts the arguments shown in the signature and returns the documented result; side effects follow the implementation.
///
pub struct Config {
    pub bot_mode: String,
    pub discord_token: String,
    pub api_base_url: String,
    pub cache_bytes: usize,
    pub cache_ttl_secs: u64,
    pub health_port: u16,
    pub development_guild_id: Option<String>,
    pub web_url: String,
    pub chrome_path: String,
    pub social_commands_enabled: bool,
}

impl Config {
    /// Load config from environment variables with defaults.
    pub fn load() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Config {
            bot_mode: std::env::var("DISCORD_BOT_MODE").unwrap_or_else(|_| "render".into()),
            discord_token: required_secret("DISCORD_TOKEN")?,
            api_base_url: std::env::var("API_BASE_URL")
                .unwrap_or_else(|_| "http://localhost:3001/v1".into()),
            cache_bytes: parse_env("CACHE_BYTES", 32 * 1024 * 1024),
            cache_ttl_secs: parse_env("CACHE_TTL_SECS", 600),
            health_port: parse_env("HEALTH_PORT", 3020),
            development_guild_id: std::env::var("DEVELOPMENT_GUILD_ID").ok(),
            web_url: std::env::var("PALADINSCAT_WEB_URL")
                .unwrap_or_else(|_| "https://paladinscat.com".into()),
            chrome_path: std::env::var("CHROME_PATH").ok().unwrap_or_default(),
            social_commands_enabled: parse_env("ENABLE_SOCIAL_COMMANDS", false),
        })
    }
}

/// Implement optional_secret.
///
/// Contract: accepts the arguments shown in the signature and returns the documented result; side effects follow the implementation.
///
pub fn optional_secret(
    name: &str,
) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
    if let Ok(value) = std::env::var(name) {
        if !value.trim().is_empty() {
            return Ok(Some(value.trim().to_owned()));
        }
    }
    let file_name = format!("{name}_FILE");
    let Ok(path) = std::env::var(&file_name) else {
        return Ok(None);
    };
    let value = std::fs::read_to_string(path)?;
    let value = value.trim();
    Ok((!value.is_empty()).then(|| value.to_owned()))
}

fn required_secret(name: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    optional_secret(name)?.ok_or_else(|| format!("{name} or {name}_FILE is required").into())
}

fn parse_env<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
