//! Authenticate the bot service to Keycloak using signed client assertions.
//!
//! Validate realm/token endpoint boundaries and read an external RSA key.
//! Refresh short-lived bearer tokens under a shared lock; tokens are never logged here.
//! refs: doc: documents/02-technical/security/service-identity.md
use aws_lc_rs::signature::{KeyPair, RsaKeyPair, RsaPublicKeyComponents};
use futures_util::StreamExt;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use reqwest::Client;
use serde::Serialize;
use std::{
    fs::File,
    io::Read,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Clone)]
/// Carry the HTTPS Keycloak realm issuer, matching public/internal token endpoint, client ID, and
/// external RSA private-key filename.
/// Environment loading validates endpoint boundaries but does not read the key or request tokens.
/// refs: doc: documents/02-technical/security/service-identity.md
pub struct ServiceAuthConfig {
    pub issuer: String,
    pub token_url: String,
    pub client_id: String,
    pub private_key_file: String,
    pub ca_file: Option<String>,
}

impl ServiceAuthConfig {
    /// Load and validate service-auth configuration from environment variables.
    ///
    /// Require four PALADINSCAT_SERVICE_OIDC_* values, trim them, and validate an HTTPS realm plus
    /// its exact public or private HTTPS token endpoint. Missing values or invalid URLs return
    /// errors; no key read or HTTP.
    ///
    /// I/O: () -> `Result<ServiceAuthConfig, Box<dyn std::error::Error + Send + Sync>>`
    /// refs: doc: documents/02-technical/security/service-identity.md
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let config = Self {
            issuer: required_env("PALADINSCAT_SERVICE_OIDC_ISSUER")?,
            token_url: required_env("PALADINSCAT_SERVICE_OIDC_TOKEN_URL")?,
            client_id: required_env("PALADINSCAT_SERVICE_OIDC_CLIENT_ID")?,
            private_key_file: required_env("PALADINSCAT_SERVICE_OIDC_PRIVATE_KEY_FILE")?,
            ca_file: std::env::var("PALADINSCAT_OIDC_CA_FILE")
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty()),
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
    let internal_match = token.scheme() == "https"
        && token.host_str() == Some("keycloak")
        && token.port() == Some(8443)
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
/// Share a configured HTTP client, RSA signing key, and bearer-token cache across clones.
/// A Tokio mutex serializes token refresh; private-key material is loaded once and never serialized
/// by this provider.
/// refs: doc: documents/02-technical/security/service-identity.md
pub struct ServiceTokenProvider {
    client: Client,
    config: Arc<ServiceAuthConfig>,
    signing_key: Arc<EncodingKey>,
    cache: Arc<Mutex<Option<CachedToken>>>,
}

impl ServiceTokenProvider {
    /// Build a service-token provider from a config.
    ///
    /// Validate endpoints and read one external RSA PEM file (at most 32 KiB; at least 3072 bits),
    /// then create a ten-second HTTP client without redirects. URL/file/UTF-8/PEM/key-size/client
    /// errors propagate; no token is requested.
    ///
    /// I/O: `ServiceAuthConfig` -> `Result<ServiceTokenProvider, Box<dyn std::error::Error + Send + Sync>>`
    /// refs: doc: documents/02-technical/security/service-identity.md
    pub fn new(
        config: ServiceAuthConfig,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        validate_endpoints(&config)?;
        // The key is read once from the runtime-mounted file and is never logged or serialized.
        let key = std::fs::read(&config.private_key_file)?;
        let signing_key = parse_signing_key(&key)?;
        let client = token_client_builder(&config)?.build()?;
        Ok(Self {
            client,
            config: Arc::new(config),
            signing_key: Arc::new(signing_key),
            cache: Arc::new(Mutex::new(None)),
        })
    }

    fn assertion(&self, now: u64) -> jsonwebtoken::errors::Result<String> {
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
        encode(&header, &claims, &self.signing_key)
    }

    async fn mint(&self) -> Result<CachedToken, Box<dyn std::error::Error + Send + Sync>> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let assertion = self.assertion(now)?;
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
    /// Hold the shared async cache mutex while minting an RS256 client assertion and POSTing
    /// client_credentials on a miss/expiry. Validate bounded Bearer responses and cache until
    /// thirty seconds before expiry; clock/signing/HTTP/response errors propagate.
    ///
    /// I/O: () -> `Result<String, Box<dyn std::error::Error + Send + Sync>>`
    /// refs: doc: documents/02-technical/security/service-identity.md
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

fn token_client_builder(
    config: &ServiceAuthConfig,
) -> Result<reqwest::ClientBuilder, Box<dyn std::error::Error + Send + Sync>> {
    validate_endpoints(config)?;
    let builder = Client::builder()
        .https_only(true)
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none());
    let endpoint = reqwest::Url::parse(&config.token_url)?;
    if endpoint.host_str() != Some("keycloak") {
        return Ok(builder);
    }
    if endpoint.port() != Some(8443) {
        return Err("private Keycloak token endpoint must use HTTPS port 8443".into());
    }
    let ca_file = config
        .ca_file
        .as_deref()
        .filter(|path| !path.trim().is_empty())
        .ok_or("private Keycloak TLS CA is required")?;
    let mut certificate = Vec::new();
    File::open(ca_file)?
        .take(65_537)
        .read_to_end(&mut certificate)?;
    if certificate.is_empty() || certificate.len() > 65_536 {
        return Err("private Keycloak TLS CA must contain at most 64 KiB".into());
    }
    let mut certificates = pem::parse_many(&certificate)?;
    if certificates.len() != 1 || certificates[0].tag() != "CERTIFICATE" {
        return Err("private Keycloak TLS CA must contain exactly one public certificate".into());
    }
    Ok(builder
        .no_proxy()
        .tls_built_in_root_certs(false)
        .add_root_certificate(reqwest::Certificate::from_der(
            certificates.remove(0).contents(),
        )?))
}

fn parse_signing_key(key: &[u8]) -> Result<EncodingKey, Box<dyn std::error::Error + Send + Sync>> {
    if key.is_empty() || key.len() > 32 * 1024 {
        return Err("service OIDC private key must contain at most 32 KiB".into());
    }
    let pem = pem::parse(key)?;
    // Retain PKCS#8 and PKCS#1 input support, using AWS-LC for private-key
    // validation and RS256 signing (RUSTSEC-2023-0071).
    let parsed = match pem.tag() {
        "PRIVATE KEY" => RsaKeyPair::from_pkcs8(pem.contents())?,
        "RSA PRIVATE KEY" => RsaKeyPair::from_der(pem.contents())?,
        _ => return Err("service OIDC private key must be RSA PKCS#8 or PKCS#1".into()),
    };
    let public = RsaPublicKeyComponents::<Vec<u8>>::from(parsed.public_key());
    let bits = public.n.len() * 8 - public.n[0].leading_zeros() as usize;
    if bits < 3072 {
        return Err("service OIDC RSA private key must be at least 3072 bits".into());
    }
    Ok(EncodingKey::from_rsa_pem(key)?)
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
            ca_file: None,
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
            "https://keycloak:8443/realms/paladinscat/protocol/openid-connect/token",
        ))
        .is_ok());
    }

    #[test]
    fn endpoint_validation_rejects_redirects_and_wrong_realms() {
        for (issuer, token_url) in [
            (
                "https://auth.example/realms/paladinscat",
                "http://keycloak:8080/realms/paladinscat/protocol/openid-connect/token",
            ),
            (
                "https://auth.example/realms/paladinscat",
                "https://keycloak:8080/realms/paladinscat/protocol/openid-connect/token",
            ),
            (
                "https://auth.example/realms/paladinscat",
                "https://keycloak/realms/paladinscat/protocol/openid-connect/token",
            ),
            (
                "https://auth.example/realms/paladinscat",
                "https://user:pass@keycloak:8443/realms/paladinscat/protocol/openid-connect/token",
            ),
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
        use aws_lc_rs::{
            encoding::{AsDer, Pkcs8V1Der},
            rsa::KeySize,
        };
        use jsonwebtoken::{decode, DecodingKey, Validation};

        let key_pair = RsaKeyPair::generate(KeySize::Rsa3072).unwrap();
        let der: Pkcs8V1Der<'static> = key_pair.as_der().unwrap();
        let pem = pem::encode(&pem::Pem::new("PRIVATE KEY", der.as_ref()));
        let signing_key = parse_signing_key(pem.as_bytes()).unwrap();
        let issuer = "https://auth.example/realms/paladinscat";
        let provider = ServiceTokenProvider {
            client: Client::new(),
            config: Arc::new(config(
                issuer,
                &format!("{issuer}/protocol/openid-connect/token"),
            )),
            signing_key: Arc::new(signing_key),
            cache: Arc::new(Mutex::new(None)),
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let assertion = provider.assertion(now).unwrap();
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[issuer]);
        validation.set_issuer(&[&provider.config.client_id]);
        validation.sub = Some(provider.config.client_id.clone());
        let public = DecodingKey::from_rsa_der(key_pair.public_key().as_ref());
        let decoded = decode::<serde_json::Value>(&assertion, &public, &validation).unwrap();
        assert_eq!(decoded.header.alg, Algorithm::RS256);
        assert_eq!(decoded.header.typ.as_deref(), Some("JWT"));
        assert_eq!(decoded.claims["iat"], now);
        assert_eq!(decoded.claims["exp"], now + 60);
        assert!(Uuid::parse_str(decoded.claims["jti"].as_str().unwrap()).is_ok());
        assert_ne!(assertion, provider.assertion(now).unwrap());
        validation.set_audience(&["wrong-audience"]);
        assert!(decode::<serde_json::Value>(&assertion, &public, &validation).is_err());

        let pkcs1 = pem::encode(&pem::Pem::new(
            "RSA PRIVATE KEY",
            provider.signing_key.as_bytes(),
        ));
        assert!(parse_signing_key(pkcs1.as_bytes()).is_ok());
        let weak = RsaKeyPair::generate(KeySize::Rsa2048).unwrap();
        let weak_der: Pkcs8V1Der<'static> = weak.as_der().unwrap();
        let weak_pem = pem::encode(&pem::Pem::new("PRIVATE KEY", weak_der.as_ref()));
        assert!(parse_signing_key(weak_pem.as_bytes()).is_err());
        assert!(parse_signing_key(b"not a key").is_err());
        assert!(parse_signing_key(&vec![0; 32 * 1024 + 1]).is_err());
    }

    fn tls_fixture(hostname: &str) -> (String, Arc<tokio_rustls::rustls::ServerConfig>) {
        use rcgen::{
            BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair,
            KeyUsagePurpose,
        };
        use tokio_rustls::rustls::{pki_types::PrivatePkcs8KeyDer, ServerConfig};
        let ca_key = KeyPair::generate().unwrap();
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.distinguished_name.push(
            DnType::CommonName,
            format!("Disposable CA {}", Uuid::new_v4()),
        );
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let ca = params.self_signed(&ca_key).unwrap();
        let key = KeyPair::generate().unwrap();
        let mut server_params = CertificateParams::new(vec![hostname.into()]).unwrap();
        server_params
            .distinguished_name
            .push(DnType::CommonName, hostname);
        server_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let certificate = server_params.signed_by(&key, &ca, &ca_key).unwrap();
        let server = ServerConfig::builder_with_provider(Arc::new(
            tokio_rustls::rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![certificate.der().clone()],
            PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
        )
        .unwrap();
        (ca.pem(), Arc::new(server))
    }

    async fn token_fixture_server(
        tls: Arc<tokio_rustls::rustls::ServerConfig>,
        response: String,
    ) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let operation = async {
                let (stream, _) = listener.accept().await.unwrap();
                if let Ok(mut stream) = tokio_rustls::TlsAcceptor::from(tls).accept(stream).await {
                    let mut request = [0; 8192];
                    if stream.read(&mut request).await.unwrap_or(0) > 0 {
                        let _ = stream.write_all(response.as_bytes()).await;
                        let _ = stream.shutdown().await;
                    }
                }
            };
            let _ = tokio::time::timeout(Duration::from_secs(3), operation).await;
        });
        (address, task)
    }

    #[tokio::test]
    async fn private_token_tls_rejects_interception_and_preserves_service_auth() {
        use aws_lc_rs::{
            encoding::{AsDer, Pkcs8V1Der},
            rsa::KeySize,
        };
        let directory = tempfile::tempdir().unwrap();
        let key_file = directory.path().join("key.pem");
        let key = RsaKeyPair::generate(KeySize::Rsa3072).unwrap();
        let der: Pkcs8V1Der<'static> = key.as_der().unwrap();
        std::fs::write(
            &key_file,
            pem::encode(&pem::Pem::new("PRIVATE KEY", der.as_ref())),
        )
        .unwrap();
        let ca_file = directory.path().join("ca.pem");
        let (trusted_ca, trusted_tls) = tls_fixture("keycloak");
        let (_, untrusted_tls) = tls_fixture("keycloak");
        let (wrong_name_ca, wrong_name_tls) = tls_fixture("other-keycloak");
        let body = r#"{"access_token":"fixture-bearer","token_type":"Bearer","expires_in":60}"#;
        let success = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let redirect_sink = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let redirect_response = format!("HTTP/1.1 302 Found\r\nLocation: https://keycloak:{}/stolen-token\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", redirect_sink.local_addr().unwrap().port());
        for (ca, tls, response, accepted) in [
            (&trusted_ca, trusted_tls.clone(), success.clone(), true),
            (&trusted_ca, untrusted_tls, success.clone(), false),
            (&wrong_name_ca, wrong_name_tls, success, false),
            (&trusted_ca, trusted_tls, redirect_response, false),
        ] {
            std::fs::write(&ca_file, ca).unwrap();
            let mut configuration = config(
                "https://auth.example/realms/paladinscat",
                "https://keycloak:8443/realms/paladinscat/protocol/openid-connect/token",
            );
            configuration.private_key_file = key_file.to_str().unwrap().into();
            configuration.ca_file = Some(ca_file.to_str().unwrap().into());
            let mut provider = ServiceTokenProvider::new(configuration.clone()).unwrap();
            let (address, task) = token_fixture_server(tls, response).await;
            provider.client = token_client_builder(&configuration)
                .unwrap()
                .resolve("keycloak", address)
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap();
            // Only the fixture changes the validated URL's port to its ephemeral loopback listener.
            let mut endpoint = reqwest::Url::parse(&configuration.token_url).unwrap();
            endpoint.set_port(Some(address.port())).unwrap();
            configuration.token_url = endpoint.to_string();
            provider.config = Arc::new(configuration);
            let result = provider.token().await;
            assert_eq!(
                result.is_ok(),
                accepted,
                "TLS fixture error: {:?}",
                result.as_ref().err()
            );
            if let Ok(token) = result {
                assert_eq!(token, "fixture-bearer");
                assert_eq!(provider.token().await.unwrap(), "fixture-bearer");
            } else {
                assert!(provider.cache.lock().await.is_none());
            }
            task.abort();
            let _ = task.await;
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(100), redirect_sink.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn private_ca_does_not_widen_public_token_endpoint_trust() {
        let directory = tempfile::tempdir().unwrap();
        let ca_file = directory.path().join("ca.pem");
        let (ca, tls) = tls_fixture("auth.example");
        std::fs::write(&ca_file, ca).unwrap();
        let mut configuration = config(
            "https://auth.example/realms/paladinscat",
            "https://auth.example/realms/paladinscat/protocol/openid-connect/token",
        );
        configuration.ca_file = Some(ca_file.to_str().unwrap().into());
        let (address, task) =
            token_fixture_server(tls, "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".into()).await;
        let client = token_client_builder(&configuration)
            .unwrap()
            .no_proxy()
            .resolve("auth.example", address)
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        assert!(client
            .get(format!(
                "https://auth.example:{}/realms/paladinscat/protocol/openid-connect/token",
                address.port()
            ))
            .send()
            .await
            .is_err());
        task.abort();
        let _ = task.await;
    }

    #[test]
    fn private_token_endpoint_requires_a_bounded_ca_and_public_default_needs_none() {
        let directory = tempfile::tempdir().unwrap();
        let ca_file = directory.path().join("ca.pem");
        let mut configuration = config(
            "https://auth.example/realms/paladinscat",
            "https://keycloak:8443/realms/paladinscat/protocol/openid-connect/token",
        );
        assert!(token_client_builder(&configuration).is_err());
        configuration.ca_file = Some(ca_file.to_str().unwrap().into());
        assert!(token_client_builder(&configuration).is_err());
        let certificate = rcgen::generate_simple_self_signed(vec!["keycloak".into()]).unwrap();
        let mixed_bundle = [
            certificate.cert.pem().into_bytes(),
            pem::encode(&pem::Pem::new("PRIVATE KEY", b"not public")).into_bytes(),
        ]
        .concat();
        for bytes in [
            Vec::new(),
            b"not a certificate".to_vec(),
            pem::encode(&pem::Pem::new("CERTIFICATE", b"not DER")).into_bytes(),
            pem::encode(&pem::Pem::new("PRIVATE KEY", b"not a CA")).into_bytes(),
            mixed_bundle,
            vec![b' '; 65_537],
        ] {
            std::fs::write(&ca_file, bytes).unwrap();
            assert!(token_client_builder(&configuration)
                .and_then(|builder| Ok(builder.build()?))
                .is_err());
        }
        assert!(token_client_builder(&config(
            "https://auth.example/realms/paladinscat",
            "https://auth.example/realms/paladinscat/protocol/openid-connect/token",
        ))
        .unwrap()
        .build()
        .is_ok());
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
