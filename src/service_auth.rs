//! Discord integration module: commands, transport, rendering, or support helpers.
//!
use futures_util::StreamExt;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use reqwest::Client;
use rsa::{
    pkcs1::DecodeRsaPrivateKey, pkcs8::DecodePrivateKey, traits::PublicKeyParts, RsaPrivateKey,
};
use serde::Serialize;
use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Clone)]
/// Define ServiceAuthConfig.
///
/// Contract: accepts the arguments shown in the signature and returns the documented result; side effects follow the implementation.
///
pub struct ServiceAuthConfig {
    pub issuer: String,
    pub token_url: String,
    pub client_id: String,
    pub private_key_file: String,
}

impl ServiceAuthConfig {
    /// Build a service-token provider from environment variables.
    ///
    /// I/O: () -> `Result<ServiceTokenProvider, Box<dyn Error + Send + Sync>>`
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let config = Self {
            issuer: required_env("PALADINSCAT_SERVICE_OIDC_ISSUER")?,
            token_url: required_env("PALADINSCAT_SERVICE_OIDC_TOKEN_URL")?,
            client_id: required_env("PALADINSCAT_SERVICE_OIDC_CLIENT_ID")?,
            private_key_file: required_env("PALADINSCAT_SERVICE_OIDC_PRIVATE_KEY_FILE")?,
        };
        validate_endpoints(&config)?;
        Ok(config)
    }
}

fn validate_endpoints(
    config: &ServiceAuthConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let issuer = reqwest::Url::parse(&config.issuer)?;
    let segments: Vec<_> = issuer
        .path_segments()
        .map(|s| s.collect())
        .unwrap_or_default();
    if issuer.scheme() != "https"
        || issuer.host_str().is_none()
        || issuer.query().is_some()
        || issuer.fragment().is_some()
        || issuer.username() != ""
        || issuer.password().is_some()
        || segments.len() != 2
        || segments[0] != "realms"
        || segments[1].is_empty()
    {
        return Err("service OIDC issuer must be an HTTPS Keycloak realm URL".into());
    }
    let token = reqwest::Url::parse(&config.token_url)?;
    let token_path = format!("{}/protocol/openid-connect/token", issuer.path());
    let public_match = token.scheme() == "https"
        && token.host_str() == issuer.host_str()
        && token.port_or_known_default() == issuer.port_or_known_default()
        && token.path() == token_path;
    let internal_match = token.scheme() == "http"
        && token.host_str() == Some("keycloak")
        && token.port() == Some(8080)
        && token.path() == token_path;
    if token.query().is_some()
        || token.fragment().is_some()
        || token.username() != ""
        || token.password().is_some()
        || !(public_match || internal_match)
    {
        return Err(
            "service OIDC token URL is not the exact configured realm token endpoint".into(),
        );
    }
    Ok(())
}

fn required_env(name: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_owned())
        .ok_or_else(|| format!("{name} is required").into())
}

#[derive(Debug, Serialize)]
struct ClientAssertion<'a> {
    iss: &'a str,
    sub: &'a str,
    aud: &'a str,
    iat: u64,
    exp: u64,
    jti: String,
}

#[derive(Debug, Clone)]
struct CachedToken {
    value: String,
    usable_until: SystemTime,
}

#[derive(Clone)]
/// Define ServiceTokenProvider.
///
/// Contract: accepts the arguments shown in the signature and returns the documented result; side effects follow the implementation.
///
pub struct ServiceTokenProvider {
    client: Client,
    config: Arc<ServiceAuthConfig>,
    signing_key: Arc<EncodingKey>,
    cache: Arc<Mutex<Option<CachedToken>>>,
}

impl ServiceTokenProvider {
    /// Build a service-token provider from a config.
    ///
    /// I/O: `ServiceAuthConfig` -> `Result<ServiceTokenProvider, Box<dyn Error + Send + Sync>>`
    pub fn new(
        config: ServiceAuthConfig,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        validate_endpoints(&config)?;
        // The key is read once from the runtime-mounted file and is never logged or serialized.
        let key = std::fs::read(&config.private_key_file)?;
        if key.len() > 32 * 1024 {
            return Err("service OIDC private key exceeds 32 KiB".into());
        }
        let key_text = std::str::from_utf8(&key)?;
        let rsa_key = RsaPrivateKey::from_pkcs8_pem(key_text)
            .or_else(|_| RsaPrivateKey::from_pkcs1_pem(key_text))?;
        if rsa_key.n().bits() < 3072 {
            return Err("service OIDC RSA private key must be at least 3072 bits".into());
        }
        let signing_key = EncodingKey::from_rsa_pem(&key)?;
        Ok(Self {
            client: Client::builder()
                .timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            config: Arc::new(config),
            signing_key: Arc::new(signing_key),
            cache: Arc::new(Mutex::new(None)),
        })
    }

    async fn mint(&self) -> Result<CachedToken, Box<dyn std::error::Error + Send + Sync>> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let claims = ClientAssertion {
            iss: &self.config.client_id,
            sub: &self.config.client_id,
            aud: &self.config.issuer,
            iat: now,
            exp: now + 60,
            jti: Uuid::new_v4().to_string(),
        };
        let mut header = Header::new(Algorithm::RS256);
        header.typ = Some("JWT".to_owned());
        let assertion = encode(&header, &claims, &self.signing_key)?;
        let response = self
            .client
            .post(&self.config.token_url)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", self.config.client_id.as_str()),
                (
                    "client_assertion_type",
                    "urn:ietf:params:oauth:client-assertion-type:jwt-bearer",
                ),
                ("client_assertion", assertion.as_str()),
            ])
            .send()
            .await?;
        if !response.status().is_success() {
            return Err("OIDC service authentication failed".into());
        }
        if response
            .content_length()
            .is_some_and(|length| length > 64 * 1024)
        {
            return Err("OIDC service authentication response exceeds 64 KiB".into());
        }
        let mut body_bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if body_bytes.len() + chunk.len() > 64 * 1024 {
                return Err("OIDC service authentication response exceeds 64 KiB".into());
            }
            body_bytes.extend_from_slice(&chunk);
        }
        let body = serde_json::from_slice::<TokenResponse>(&body_bytes)?;
        let (value, expires_in) = validate_token_response(body)?;
        let usable_until = SystemTime::now() + Duration::from_secs(expires_in.saturating_sub(30));
        Ok(CachedToken {
            value,
            usable_until,
        })
    }

    /// Return a current bearer token, refreshing it when close to expiry.
    ///
    /// I/O: () -> `Result<String, Box<dyn Error + Send + Sync>>`
    pub async fn token(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let mut cache = self.cache.lock().await;
        if let Some(cached) = cache
            .as_ref()
            .filter(|token| token.usable_until > SystemTime::now())
        {
            return Ok(cached.value.clone());
        }
        let minted = self.mint().await?;
        let value = minted.value.clone();
        *cache = Some(minted);
        Ok(value)
    }
}

fn validate_token_response(
    body: TokenResponse,
) -> Result<(String, u64), Box<dyn std::error::Error + Send + Sync>> {
    if body.token_type.as_deref() != Some("Bearer") || !(31..=300).contains(&body.expires_in) {
        return Err("OIDC service authentication response is invalid".into());
    }
    let value = body
        .access_token
        .filter(|token| !token.is_empty() && token.len() <= 16 * 1024 && token.trim() == token)
        .ok_or("OIDC service authentication returned no access token")?;
    Ok((value, body.expires_in))
}

#[derive(Debug, serde::Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    token_type: Option<String>,
    #[serde(default)]
    expires_in: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(issuer: &str, token_url: &str) -> ServiceAuthConfig {
        ServiceAuthConfig {
            issuer: issuer.into(),
            token_url: token_url.into(),
            client_id: "paladinscat-discord-service".into(),
            private_key_file: "not-read-in-url-tests".into(),
        }
    }

    #[test]
    fn endpoint_validation_accepts_public_and_private_keycloak_urls() {
        assert!(validate_endpoints(&config(
            "https://auth.example/realms/paladinscat",
            "https://auth.example/realms/paladinscat/protocol/openid-connect/token",
        ))
        .is_ok());
        assert!(validate_endpoints(&config(
            "https://auth.example/realms/paladinscat",
            "http://keycloak:8080/realms/paladinscat/protocol/openid-connect/token",
        ))
        .is_ok());
    }

    #[test]
    fn endpoint_validation_rejects_redirects_and_wrong_realms() {
        for (issuer, token_url) in [
            (
                "http://auth.example/realms/paladinscat",
                "http://keycloak:8080/realms/paladinscat/protocol/openid-connect/token",
            ),
            (
                "https://auth.example/realms/paladinscat",
                "https://evil.example/realms/paladinscat/protocol/openid-connect/token",
            ),
            (
                "https://auth.example/realms/paladinscat",
                "http://keycloak:8080/realms/other/protocol/openid-connect/token",
            ),
            (
                "https://auth.example/realms/paladinscat",
                "https://auth.example/realms/paladinscat/protocol/openid-connect/token?redirect=1",
            ),
        ] {
            assert!(validate_endpoints(&config(issuer, token_url)).is_err());
        }
    }

    #[test]
    fn assertion_claims_are_short_lived_and_service_bound() {
        let claims = ClientAssertion {
            iss: "bot",
            sub: "bot",
            aud: "https://issuer",
            iat: 100,
            exp: 160,
            jti: "jti".into(),
        };
        assert_eq!(claims.iss, claims.sub);
        assert_eq!(claims.aud, "https://issuer");
        assert!(claims.exp - claims.iat <= 60);
        assert!(!claims.jti.is_empty());
    }

    #[test]
    fn token_response_requires_bearer_and_bounded_lifetime() {
        let valid = TokenResponse {
            access_token: Some("opaque".into()),
            token_type: Some("Bearer".into()),
            expires_in: 60,
        };
        assert_eq!(validate_token_response(valid).unwrap().1, 60);
        for (token_type, expires_in, access_token) in [
            (Some("bearer"), 60, Some("opaque")),
            (Some("Bearer"), 30, Some("opaque")),
            (Some("Bearer"), 301, Some("opaque")),
            (Some("Bearer"), 60, Some(" opaque")),
        ] {
            assert!(validate_token_response(TokenResponse {
                access_token: access_token.map(str::to_owned),
                token_type: token_type.map(str::to_owned),
                expires_in,
            })
            .is_err());
        }
    }
}
