//! Bot configuration — replaces config.ts
//!
//! Environment-based configuration loading with sensible defaults.

#[derive(Debug, Clone)]
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
}

impl Config {
    /// Load config from environment variables with defaults.
    pub fn load() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Config {
            bot_mode: std::env::var("DISCORD_BOT_MODE").unwrap_or_else(|_| "render".into()),
            discord_token: std::env::var("DISCORD_TOKEN")?,
            api_base_url: std::env::var("API_BASE_URL")
                .unwrap_or_else(|_| "http://localhost:3001/api".into()),
            cache_bytes: parse_env("CACHE_BYTES", 32 * 1024 * 1024),
            cache_ttl_secs: parse_env("CACHE_TTL_SECS", 600),
            health_port: parse_env("HEALTH_PORT", 3020),
            development_guild_id: std::env::var("DEVELOPMENT_GUILD_ID").ok(),
            web_url: std::env::var("PALADINSCAT_WEB_URL")
                .unwrap_or_else(|_| "https://paladinscat.com".into()),
            chrome_path: std::env::var("CHROME_PATH").ok().unwrap_or_default(),
        })
    }
}

fn parse_env<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
