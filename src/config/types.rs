// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Configuration structure definitions
//!
//! Defines the schema for OxiDNS Next configuration files (YAML format).

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use serde_yaml_ng::Value;
use thiserror::Error;

use crate::infra::network::proxy::validate_socks5_syntax;
use crate::infra::system::parse_simple_duration;

/// Configuration validation errors
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Plugin tag cannot be empty")]
    EmptyPluginTag,

    #[error("Invalid log level: {0}")]
    InvalidLogLevel(String),

    #[error("log.{field} cannot be empty")]
    EmptyLogFilePath { field: &'static str },

    #[error("log.file and log.query_file must use different paths")]
    DuplicateLogFilePath,

    #[error("Plugin type cannot be empty")]
    EmptyPluginType,

    #[error("runtime.worker_threads must be greater than 0")]
    InvalidRuntimeWorkerThreads,

    #[error("api.http.listen cannot be empty")]
    EmptyApiHttpListen,

    #[error("api.http.auth.basic.username cannot be empty")]
    EmptyApiBasicAuthUsername,

    #[error("api.http.auth.basic.password cannot be empty")]
    EmptyApiBasicAuthPassword,

    #[error("Invalid api.http.auth configuration: {0}")]
    InvalidApiAuth(String),

    #[error("Invalid api.http.cors configuration: {0}")]
    InvalidApiCors(String),

    #[error("api.http.ssl.cert and api.http.ssl.key must be configured together")]
    IncompleteApiTlsConfig,

    #[error("api.http.ssl.require_client_cert requires api.http.ssl.client_ca")]
    MissingApiTlsClientCa,

    #[error("api.http.webui.root cannot be empty")]
    EmptyApiWebUiRoot,

    #[error("api.http.webui.index cannot be empty")]
    EmptyApiWebUiIndex,

    #[error("Invalid network outbound config: {0}")]
    InvalidNetworkOutbound(String),

    #[error(
        "Duplicate plugin tag '{tag}' found at plugins[{first_index}] and plugins[{duplicate_index}]"
    )]
    DuplicatePluginTag {
        tag: String,
        first_index: usize,
        duplicate_index: usize,
    },
}

/// Main server configuration
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Additional configuration files whose plugins should be loaded first.
    #[serde(default)]
    pub include: Vec<String>,

    /// Tokio runtime configuration.
    #[serde(default)]
    pub runtime: RuntimeConfig,

    /// Optional management API configuration.
    #[serde(default)]
    pub api: ApiConfig,

    /// Logging configuration (level, file output)
    #[serde(default)]
    pub log: LogConfig,

    /// Shared network policy configuration.
    #[serde(default)]
    pub network: NetworkConfig,

    /// List of plugins to load and their configurations
    #[serde(default)]
    pub plugins: Vec<PluginConfig>,
}

impl Config {
    /// Validate configuration
    ///
    /// Validates the configuration structure (log level, plugin tags/types).
    /// Plugin-specific validation (e.g., listen addresses, upstreams) is
    /// delegated to each PluginFactory during plugin initialization.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if matches!(self.runtime.worker_threads, Some(0)) {
            return Err(ConfigError::InvalidRuntimeWorkerThreads);
        }

        // Validate log level
        match self.log.level.to_lowercase().as_str() {
            "off" | "trace" | "debug" | "info" | "warn" | "error" => {}
            _ => return Err(ConfigError::InvalidLogLevel(self.log.level.clone())),
        }
        for (field, path) in [
            ("file", self.log.file.as_deref()),
            ("query_file", self.log.query_file.as_deref()),
        ] {
            if path.is_some_and(|value| value.trim().is_empty()) {
                return Err(ConfigError::EmptyLogFilePath { field });
            }
        }
        if self
            .log
            .file
            .as_deref()
            .zip(self.log.query_file.as_deref())
            .is_some_and(|(system, query)| log_paths_equivalent(system, query))
        {
            return Err(ConfigError::DuplicateLogFilePath);
        }

        if let Some(http) = &self.api.http {
            let resolved = http.resolve();
            if resolved.listen.trim().is_empty() {
                return Err(ConfigError::EmptyApiHttpListen);
            }

            if let Some(ssl) = &resolved.ssl {
                let cert_present = ssl.cert.is_some();
                let key_present = ssl.key.is_some();
                if cert_present != key_present {
                    return Err(ConfigError::IncompleteApiTlsConfig);
                }
                if ssl.require_client_cert.unwrap_or(false) && ssl.client_ca.is_none() {
                    return Err(ConfigError::MissingApiTlsClientCa);
                }
            }

            if let Some(auth) = &resolved.auth {
                auth.validate()?;
            }

            if let Some(cors) = &resolved.cors {
                cors.validate()?;
            }

            if let Some(webui) = &resolved.webui {
                if webui.root.trim().is_empty() {
                    return Err(ConfigError::EmptyApiWebUiRoot);
                }
                if matches!(webui.index.as_deref(), Some(index) if index.trim().is_empty()) {
                    return Err(ConfigError::EmptyApiWebUiIndex);
                }
            }
        }

        self.network.validate()?;

        // Validate plugins - basic structure checks
        let mut seen_tags = HashMap::new();
        for (idx, plugin) in self.plugins.iter().enumerate() {
            // Check for empty tag
            if plugin.tag.is_empty() {
                return Err(ConfigError::EmptyPluginTag);
            }
            if let Some(prev_idx) = seen_tags.insert(plugin.tag.as_str(), idx) {
                return Err(ConfigError::DuplicatePluginTag {
                    tag: plugin.tag.clone(),
                    first_index: prev_idx,
                    duplicate_index: idx,
                });
            }

            // Check for empty type
            if plugin.plugin_type.is_empty() {
                return Err(ConfigError::EmptyPluginType);
            }
        }

        Ok(())
    }
}

fn log_paths_equivalent(left: &str, right: &str) -> bool {
    let base = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let normalize = |raw: &str| {
        let path = Path::new(raw.trim());
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            base.join(path)
        };
        normalize_path_lexically(&absolute)
    };
    let left = normalize(left);
    let right = normalize(right);

    #[cfg(windows)]
    {
        left.to_string_lossy().to_lowercase() == right.to_string_lossy().to_lowercase()
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn normalize_path_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                // The caller makes relative inputs absolute first, so a parent
                // component at the filesystem root cannot escape that root.
                normalized.pop();
            }
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::Normal(part) => normalized.push(part),
        }
    }
    normalized
}

/// Shared network configuration.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct NetworkConfig {
    /// Named outbound connection profiles shared by HTTP clients and upstreams.
    #[serde(default)]
    pub outbound: NetworkOutboundConfig,
}

impl NetworkConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        self.outbound.validate()
    }
}

/// Global outbound profile registry.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct NetworkOutboundConfig {
    /// Optional default profile used when a caller does not name one.
    pub default: Option<String>,

    /// Named outbound profiles.
    #[serde(default)]
    pub profiles: HashMap<String, OutboundProfileConfig>,
}

impl NetworkOutboundConfig {
    pub(crate) fn validate(&self) -> Result<(), ConfigError> {
        if let Some(default) = self.default.as_deref() {
            if default.trim().is_empty() {
                return Err(ConfigError::InvalidNetworkOutbound(
                    "default profile name cannot be empty".to_string(),
                ));
            }
            if default != default.trim() {
                return Err(ConfigError::InvalidNetworkOutbound(format!(
                    "default profile '{}' cannot contain leading or trailing whitespace",
                    default
                )));
            }
            if !self.profiles.contains_key(default) {
                return Err(ConfigError::InvalidNetworkOutbound(format!(
                    "default profile '{}' is not defined",
                    default
                )));
            }
        }

        for (name, profile) in &self.profiles {
            if name.trim().is_empty() {
                return Err(ConfigError::InvalidNetworkOutbound(
                    "profile name cannot be empty".to_string(),
                ));
            }
            if name != name.trim() {
                return Err(ConfigError::InvalidNetworkOutbound(format!(
                    "profile name '{}' cannot contain leading or trailing whitespace",
                    name
                )));
            }
            profile.validate(name)?;
        }
        Ok(())
    }
}

/// One named outbound connection profile.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutboundProfileConfig {
    pub resolver: Option<OutboundResolverConfig>,
    pub proxy: Option<OutboundProxyConfig>,
}

impl OutboundProfileConfig {
    fn validate(&self, profile_name: &str) -> Result<(), ConfigError> {
        if let Some(resolver) = &self.resolver {
            resolver.validate(profile_name)?;
        }
        if let Some(proxy) = &self.proxy {
            proxy.validate(profile_name)?;
        }
        if self.resolver_uses_profile_proxy()
            && !matches!(self.proxy, Some(OutboundProxyConfig::Socks5 { .. }))
        {
            return Err(ConfigError::InvalidNetworkOutbound(format!(
                "profile '{}' resolver.proxy profile requires a socks5 proxy",
                profile_name
            )));
        }
        Ok(())
    }

    fn resolver_uses_profile_proxy(&self) -> bool {
        matches!(
            self.resolver,
            Some(OutboundResolverConfig::Nameservers(
                OutboundResolverDetailedConfig {
                    proxy: Some(OutboundResolverProxyConfig::Profile),
                    ..
                }
            ))
        )
    }
}

/// Resolver policy for an outbound profile.
///
/// This resolver is used by OxiDNS Next-owned outbound clients and opt-in
/// upstreams. It is intentionally separate from legacy upstream `bootstrap`,
/// whose field remains available on each upstream for local override
/// compatibility.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum OutboundResolverConfig {
    Mode(String),
    Nameservers(OutboundResolverDetailedConfig),
}

impl OutboundResolverConfig {
    fn validate(&self, profile_name: &str) -> Result<(), ConfigError> {
        match self {
            Self::Mode(mode) if mode.trim().eq_ignore_ascii_case("system") => Ok(()),
            Self::Mode(mode) => Err(ConfigError::InvalidNetworkOutbound(format!(
                "profile '{}' has invalid resolver mode '{}'",
                profile_name, mode
            ))),
            Self::Nameservers(config) => config.validate(profile_name),
        }
    }
}

/// Detailed resolver policy for an outbound profile.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutboundResolverDetailedConfig {
    pub nameservers: Vec<OutboundNameserverConfig>,
    pub ip_version: Option<u8>,
    pub timeout: Option<String>,
    pub proxy: Option<OutboundResolverProxyConfig>,
}

impl OutboundResolverDetailedConfig {
    fn validate(&self, profile_name: &str) -> Result<(), ConfigError> {
        if self.nameservers.is_empty() {
            return Err(ConfigError::InvalidNetworkOutbound(format!(
                "profile '{}' resolver.nameservers requires at least one server",
                profile_name
            )));
        }
        if !matches!(self.ip_version, None | Some(4) | Some(6)) {
            return Err(ConfigError::InvalidNetworkOutbound(format!(
                "profile '{}' resolver.ip_version must be 4 or 6",
                profile_name
            )));
        }
        if let Some(timeout) = &self.timeout {
            parse_simple_duration(timeout).map_err(|err| {
                ConfigError::InvalidNetworkOutbound(format!(
                    "profile '{}' resolver.timeout is invalid: {}",
                    profile_name, err
                ))
            })?;
        }

        let resolver_uses_profile_proxy = matches!(
            self.proxy
                .as_ref()
                .unwrap_or(&OutboundResolverProxyConfig::None),
            OutboundResolverProxyConfig::Profile
        );
        for nameserver in &self.nameservers {
            nameserver.validate(profile_name, resolver_uses_profile_proxy)?;
        }
        Ok(())
    }
}

/// One outbound resolver nameserver endpoint.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutboundNameserverConfig {
    pub addr: String,
    pub dial_addr: Option<IpAddr>,
}

impl OutboundNameserverConfig {
    fn validate(
        &self,
        profile_name: &str,
        resolver_uses_profile_proxy: bool,
    ) -> Result<(), ConfigError> {
        if self.addr.trim().is_empty() {
            return Err(ConfigError::InvalidNetworkOutbound(format!(
                "profile '{}' resolver.nameservers addr cannot be empty",
                profile_name
            )));
        }

        let parsed = parse_nameserver_addr(self.addr.as_str()).ok_or_else(|| {
            ConfigError::InvalidNetworkOutbound(format!(
                "profile '{}' resolver.nameservers has invalid addr '{}'",
                profile_name, self.addr
            ))
        })?;
        if let Some(hint) = parsed.rebuild_hint() {
            return Err(ConfigError::InvalidNetworkOutbound(format!(
                "profile '{}' resolver.nameservers addr '{}': {}",
                profile_name, self.addr, hint
            )));
        }

        if parsed.host.parse::<IpAddr>().is_err() && self.dial_addr.is_none() {
            return Err(ConfigError::InvalidNetworkOutbound(format!(
                "profile '{}' resolver.nameservers domain addr '{}' requires dial_addr",
                profile_name, self.addr
            )));
        }

        if resolver_uses_profile_proxy && parsed.proxy_unsupported {
            return Err(ConfigError::InvalidNetworkOutbound(format!(
                "profile '{}' resolver proxy cannot be used with {} nameserver '{}'",
                profile_name, parsed.scheme, self.addr
            )));
        }

        Ok(())
    }
}

struct ParsedNameserverAddr {
    scheme: String,
    host: String,
    proxy_unsupported: bool,
}

impl ParsedNameserverAddr {
    fn rebuild_hint(&self) -> Option<&'static str> {
        match self.scheme.as_str() {
            "tls" | "tls+pipeline" if !cfg!(feature = "resolver-dot") => Some(
                "nameserver DoT is not compiled into this build; rebuild with --features resolver-dot",
            ),
            "https" | "doh" if !cfg!(feature = "resolver-doh") => Some(
                "nameserver DoH is not compiled into this build; rebuild with --features resolver-doh",
            ),
            "h3" if !cfg!(feature = "resolver-doh3") => Some(
                "nameserver DoH3 is not compiled into this build; rebuild with --features resolver-doh3",
            ),
            "quic" | "doq" if !cfg!(feature = "resolver-doq") => Some(
                "nameserver DoQ is not compiled into this build; rebuild with --features resolver-doq",
            ),
            _ => None,
        }
    }
}

fn parse_nameserver_addr(addr: &str) -> Option<ParsedNameserverAddr> {
    let raw = addr.trim();
    let normalized;
    let candidate = if raw.contains("//") {
        raw
    } else {
        normalized = format!("udp://{raw}");
        normalized.as_str()
    };
    let url = url::Url::parse(candidate).ok()?;
    let host = match url.host()? {
        url::Host::Domain(domain) => domain.to_string(),
        url::Host::Ipv4(ip) => ip.to_string(),
        url::Host::Ipv6(ip) => ip.to_string(),
    };
    let scheme = url.scheme().to_ascii_lowercase();
    if !matches!(
        scheme.as_str(),
        "udp"
            | "tcp"
            | "tcp+pipeline"
            | "tls"
            | "tls+pipeline"
            | "https"
            | "doh"
            | "h3"
            | "quic"
            | "doq"
    ) {
        return None;
    }
    let proxy_unsupported = matches!(scheme.as_str(), "udp" | "doq" | "quic" | "h3");
    Some(ParsedNameserverAddr {
        scheme,
        host,
        proxy_unsupported,
    })
}

/// Resolver proxy policy for outbound profile nameservers.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OutboundResolverProxyConfig {
    #[default]
    None,
    Profile,
}

/// One or more legacy upstream bootstrap DNS servers.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum BootstrapServerConfig {
    One(String),
    Many(Vec<String>),
}

impl BootstrapServerConfig {
    pub fn servers(&self) -> Vec<&str> {
        match self {
            Self::One(server) => vec![server.as_str()],
            Self::Many(servers) => servers.iter().map(String::as_str).collect(),
        }
    }
}

/// Proxy policy for an outbound profile.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum OutboundProxyConfig {
    Mode(String),
    Socks5 { socks5: String },
}

impl OutboundProxyConfig {
    fn validate(&self, profile_name: &str) -> Result<(), ConfigError> {
        match self {
            Self::Mode(mode)
                if mode.trim().eq_ignore_ascii_case("none")
                    || mode.trim().eq_ignore_ascii_case("direct") =>
            {
                Ok(())
            }
            Self::Mode(mode) => Err(ConfigError::InvalidNetworkOutbound(format!(
                "profile '{}' has invalid proxy mode '{}'",
                profile_name, mode
            ))),
            Self::Socks5 { socks5 } if socks5.trim().is_empty() => {
                Err(ConfigError::InvalidNetworkOutbound(format!(
                    "profile '{}' socks5 proxy cannot be empty",
                    profile_name
                )))
            }
            Self::Socks5 { socks5 } if !validate_socks5_syntax(socks5) => {
                Err(ConfigError::InvalidNetworkOutbound(format!(
                    "profile '{}' has invalid socks5 proxy '{}'",
                    profile_name, socks5
                )))
            }
            Self::Socks5 { .. } => Ok(()),
        }
    }
}

/// Management API configuration.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ApiConfig {
    /// Optional HTTP management API configuration.
    pub http: Option<ApiHttpConfig>,
}

/// `api.http` supports shorthand string and detailed object forms.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ApiHttpConfig {
    Listen(String),
    Detailed(Box<ApiHttpDetailedConfig>),
}

impl ApiHttpConfig {
    /// Resolve user-facing config variants into one canonical structure.
    pub fn resolve(&self) -> ResolvedApiHttpConfig {
        match self {
            Self::Listen(listen) => ResolvedApiHttpConfig {
                listen: listen.clone(),
                ssl: None,
                auth: None,
                cors: None,
                webui: None,
            },
            Self::Detailed(config) => ResolvedApiHttpConfig {
                listen: config.listen.clone(),
                ssl: config.ssl.clone(),
                auth: config.auth.clone(),
                cors: config.cors.clone(),
                webui: config.webui.clone(),
            },
        }
    }
}

/// CORS settings for the management API.
///
/// When present, cross-origin requests matching the configured origins are
/// accepted. This is needed when the WebUI is served from a different host
/// or port than the API server.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ApiCorsConfig {
    /// List of allowed `Origin` values (e.g. `http://localhost:3000`).
    ///
    /// Each entry is matched exactly against the incoming `Origin` header.
    /// Use `"*"` to allow any origin (credentials will not be sent in that
    /// case).
    #[serde(default)]
    pub allowed_origins: Vec<String>,
    /// Runtime-only flag used by the management API when CORS is inferred from
    /// a wildcard listen address such as `0.0.0.0` or `[::]`.
    #[serde(default, skip)]
    pub allow_any_origin: bool,
    /// Runtime-only host allowlist inferred from the API listen address.
    ///
    /// These entries match the host part of the browser `Origin` header and do
    /// not constrain the WebUI port.
    #[serde(default, skip)]
    pub allowed_origin_hosts: Vec<String>,
}

impl ApiCorsConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        for origin in &self.allowed_origins {
            if origin == "*" {
                continue;
            }
            let url = url::Url::parse(origin).map_err(|error| {
                ConfigError::InvalidApiCors(format!("invalid allowed origin {origin:?}: {error}"))
            })?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.origin().ascii_serialization() != *origin
            {
                return Err(ConfigError::InvalidApiCors(format!(
                    "allowed origin {origin:?} must be an exact HTTP(S) origin without credentials, a path, query, or fragment"
                )));
            }
        }
        Ok(())
    }
}

/// Expanded HTTP API configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct ApiHttpDetailedConfig {
    pub listen: String,
    pub ssl: Option<ApiTlsConfig>,
    pub auth: Option<ApiAuthConfig>,
    pub cors: Option<ApiCorsConfig>,
    pub webui: Option<ApiWebUiConfig>,
}

/// Static WebUI files served by the management API listener.
#[derive(Debug, Clone, Deserialize)]
pub struct ApiWebUiConfig {
    pub root: String,
    pub index: Option<String>,
}

/// TLS settings for the management API.
#[derive(Debug, Clone, Deserialize)]
pub struct ApiTlsConfig {
    pub cert: Option<String>,
    pub key: Option<String>,
    pub client_ca: Option<String>,
    pub require_client_cert: Option<bool>,
}

/// Authentication settings for the management API.
#[derive(Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ApiAuthConfig {
    /// Deprecated compatibility form. On first startup these credentials are
    /// imported into the accounts database and are ignored thereafter.
    Basic { username: String, password: String },
    /// Persistent management accounts with session-based authentication.
    Accounts {
        #[serde(default = "default_auth_database")]
        database: String,
        /// One-time token that permits non-loopback bootstrap requests.
        bootstrap_token: Option<String>,
        /// Environment variable containing the one-time bootstrap token.
        bootstrap_token_env: Option<String>,
        #[serde(default = "default_auth_session_ttl_seconds")]
        session_ttl_seconds: u64,
        /// Override the automatic Secure-cookie policy. Production HTTPS
        /// deployments should normally leave this unset.
        cookie_secure: Option<bool>,
        #[serde(default)]
        cookie_same_site: ApiCookieSameSite,
        /// Browser-visible base URL, used to derive WebAuthn settings.
        public_url: Option<String>,
        oidc: Option<Box<ApiOidcConfig>>,
        passkey: Option<ApiPasskeyConfig>,
    },
}

impl std::fmt::Debug for ApiAuthConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Basic { username, .. } => formatter
                .debug_struct("Basic")
                .field("username", username)
                .field("password", &"<redacted>")
                .finish(),
            Self::Accounts {
                database,
                bootstrap_token,
                bootstrap_token_env,
                session_ttl_seconds,
                cookie_secure,
                cookie_same_site,
                public_url,
                oidc,
                passkey,
            } => formatter
                .debug_struct("Accounts")
                .field("database", database)
                .field("bootstrap_token_configured", &bootstrap_token.is_some())
                .field("bootstrap_token_env", bootstrap_token_env)
                .field("session_ttl_seconds", session_ttl_seconds)
                .field("cookie_secure", cookie_secure)
                .field("cookie_same_site", cookie_same_site)
                .field("public_url", public_url)
                .field("oidc", oidc)
                .field("passkey", passkey)
                .finish(),
        }
    }
}

impl ApiAuthConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        match self {
            Self::Basic { username, password } => {
                if username.trim().is_empty() {
                    return Err(ConfigError::EmptyApiBasicAuthUsername);
                }
                if password.trim().is_empty() {
                    return Err(ConfigError::EmptyApiBasicAuthPassword);
                }
            }
            Self::Accounts {
                database,
                bootstrap_token,
                bootstrap_token_env,
                session_ttl_seconds,
                cookie_secure,
                cookie_same_site,
                public_url,
                oidc,
                passkey,
                ..
            } => {
                if database.trim().is_empty() {
                    return Err(ConfigError::InvalidApiAuth(
                        "accounts.database cannot be empty".to_string(),
                    ));
                }
                if bootstrap_token
                    .as_ref()
                    .is_some_and(|value| value.trim().is_empty())
                {
                    return Err(ConfigError::InvalidApiAuth(
                        "accounts.bootstrap_token cannot be empty".to_string(),
                    ));
                }
                if bootstrap_token_env
                    .as_ref()
                    .is_some_and(|value| value.trim().is_empty())
                {
                    return Err(ConfigError::InvalidApiAuth(
                        "accounts.bootstrap_token_env cannot be empty".to_string(),
                    ));
                }
                if bootstrap_token.is_some() && bootstrap_token_env.is_some() {
                    return Err(ConfigError::InvalidApiAuth(
                        "configure only one of accounts.bootstrap_token and bootstrap_token_env"
                            .to_string(),
                    ));
                }
                if !(300..=604_800).contains(session_ttl_seconds) {
                    return Err(ConfigError::InvalidApiAuth(
                        "accounts.session_ttl_seconds must be between 300 and 604800".to_string(),
                    ));
                }
                if matches!(cookie_same_site, ApiCookieSameSite::None)
                    && *cookie_secure == Some(false)
                {
                    return Err(ConfigError::InvalidApiAuth(
                        "accounts.cookie_same_site=none cannot be combined with cookie_secure=false"
                            .to_string(),
                    ));
                }
                if let Some(url) = public_url {
                    validate_browser_url(url, "accounts.public_url")?;
                }
                if let Some(oidc) = oidc {
                    oidc.validate()?;
                }
                if let Some(passkey) = passkey {
                    passkey.validate(public_url.as_deref())?;
                }
            }
        }
        Ok(())
    }

    #[cfg(feature = "api")]
    pub(crate) fn database_path(&self) -> &str {
        match self {
            Self::Basic { .. } => "./data/oxidns-next-auth.db",
            Self::Accounts { database, .. } => database,
        }
    }

    #[cfg(feature = "api")]
    pub(crate) fn session_ttl_seconds(&self) -> u64 {
        match self {
            Self::Basic { .. } => default_auth_session_ttl_seconds(),
            Self::Accounts {
                session_ttl_seconds,
                ..
            } => *session_ttl_seconds,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiCookieSameSite {
    Strict,
    #[default]
    Lax,
    None,
}

#[derive(Clone, Deserialize)]
pub struct ApiOidcConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub issuer_url: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    /// Environment variable containing the OIDC client secret.
    pub client_secret_env: Option<String>,
    pub redirect_url: String,
    #[serde(default = "default_oidc_scopes")]
    pub scopes: Vec<String>,
    #[serde(default = "default_oidc_username_claim")]
    pub username_claim: String,
    /// Explicit claim-to-local-account allowlist. OIDC never creates an
    /// administrator implicitly.
    #[serde(default)]
    pub allowed_users: Vec<ApiOidcUserMapping>,
    #[serde(default = "default_oidc_success_redirect")]
    pub success_redirect: String,
}

impl std::fmt::Debug for ApiOidcConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ApiOidcConfig")
            .field("enabled", &self.enabled)
            .field("issuer_url", &self.issuer_url)
            .field("client_id", &self.client_id)
            .field("client_secret_configured", &self.client_secret.is_some())
            .field("client_secret_env", &self.client_secret_env)
            .field("redirect_url", &self.redirect_url)
            .field("scopes", &self.scopes)
            .field("username_claim", &self.username_claim)
            .field("allowed_users", &self.allowed_users)
            .field("success_redirect", &self.success_redirect)
            .finish()
    }
}

impl ApiOidcConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        if !self.enabled {
            return Ok(());
        }
        if self.issuer_url.trim().is_empty()
            || self.client_id.trim().is_empty()
            || self.redirect_url.trim().is_empty()
            || self.username_claim.trim().is_empty()
        {
            return Err(ConfigError::InvalidApiAuth(
                "enabled OIDC requires issuer_url, client_id, redirect_url and username_claim"
                    .to_string(),
            ));
        }
        if self
            .client_secret_env
            .as_ref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(ConfigError::InvalidApiAuth(
                "oidc.client_secret_env cannot be empty".to_string(),
            ));
        }
        if self.client_secret.is_some() && self.client_secret_env.is_some() {
            return Err(ConfigError::InvalidApiAuth(
                "configure only one of oidc.client_secret and client_secret_env".to_string(),
            ));
        }
        if !matches!(
            self.username_claim.as_str(),
            "preferred_username" | "email" | "name" | "sub"
        ) {
            return Err(ConfigError::InvalidApiAuth(
                "oidc.username_claim must be preferred_username, email, name, or sub".to_string(),
            ));
        }
        validate_secure_browser_url(&self.issuer_url, "oidc.issuer_url")?;
        validate_secure_browser_url(&self.redirect_url, "oidc.redirect_url")?;
        if self.scopes.iter().all(|scope| scope != "openid") {
            return Err(ConfigError::InvalidApiAuth(
                "oidc.scopes must include openid".to_string(),
            ));
        }
        if self.allowed_users.is_empty() {
            return Err(ConfigError::InvalidApiAuth(
                "enabled OIDC requires at least one allowed_users mapping".to_string(),
            ));
        }
        if self
            .allowed_users
            .iter()
            .any(|mapping| mapping.claim.trim().is_empty() || mapping.username.trim().is_empty())
        {
            return Err(ConfigError::InvalidApiAuth(
                "oidc.allowed_users entries require non-empty claim and username".to_string(),
            ));
        }
        if !self.success_redirect.starts_with('/')
            || self.success_redirect.starts_with("//")
            || self.success_redirect.contains('\\')
            || self.success_redirect.chars().any(char::is_control)
        {
            return Err(ConfigError::InvalidApiAuth(
                "oidc.success_redirect must be an absolute local path".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiOidcUserMapping {
    pub claim: String,
    pub username: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiPasskeyConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub rp_id: Option<String>,
    #[serde(default)]
    pub origins: Vec<String>,
}

impl ApiPasskeyConfig {
    fn validate(&self, public_url: Option<&str>) -> Result<(), ConfigError> {
        if !self.enabled {
            return Ok(());
        }
        if self
            .rp_id
            .as_ref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(ConfigError::InvalidApiAuth(
                "passkey.rp_id cannot be empty".to_string(),
            ));
        }
        if self.rp_id.is_none() && public_url.is_none() {
            return Err(ConfigError::InvalidApiAuth(
                "enabled passkey auth requires rp_id or accounts.public_url".to_string(),
            ));
        }
        if self.rp_id.is_none()
            && public_url
                .and_then(|value| url::Url::parse(value).ok())
                .is_none_or(|url| url.domain().is_none())
        {
            return Err(ConfigError::InvalidApiAuth(
                "accounts.public_url must use a DNS hostname when deriving passkey.rp_id"
                    .to_string(),
            ));
        }
        if self.origins.is_empty() {
            let origin = public_url.ok_or_else(|| {
                ConfigError::InvalidApiAuth(
                    "enabled passkey auth requires origins or accounts.public_url".to_string(),
                )
            })?;
            validate_webauthn_origin(origin, "accounts.public_url")?;
        } else {
            for origin in &self.origins {
                validate_webauthn_origin(origin, "passkey.origins")?;
            }
        }
        Ok(())
    }
}

fn default_auth_database() -> String {
    "./data/oxidns-next-auth.db".to_string()
}

fn default_auth_session_ttl_seconds() -> u64 {
    43_200
}

fn default_true() -> bool {
    true
}

fn default_oidc_scopes() -> Vec<String> {
    vec![
        "openid".to_string(),
        "profile".to_string(),
        "email".to_string(),
    ]
}

fn default_oidc_username_claim() -> String {
    "preferred_username".to_string()
}

fn default_oidc_success_redirect() -> String {
    "/".to_string()
}

fn validate_browser_url(value: &str, field: &str) -> Result<(), ConfigError> {
    let url = url::Url::parse(value)
        .map_err(|err| ConfigError::InvalidApiAuth(format!("{field} is not a valid URL: {err}")))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(ConfigError::InvalidApiAuth(format!(
            "{field} must be an absolute HTTP(S) URL"
        )));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(ConfigError::InvalidApiAuth(format!(
            "{field} must not contain credentials"
        )));
    }
    Ok(())
}

fn validate_secure_browser_url(value: &str, field: &str) -> Result<(), ConfigError> {
    validate_browser_url(value, field)?;
    let url = url::Url::parse(value).expect("browser URL was validated above");
    let loopback = url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .trim_matches(['[', ']'])
                .parse::<IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if url.scheme() != "https" && !loopback {
        return Err(ConfigError::InvalidApiAuth(format!(
            "{field} must use HTTPS except on loopback"
        )));
    }
    Ok(())
}

fn validate_webauthn_origin(value: &str, field: &str) -> Result<(), ConfigError> {
    validate_secure_browser_url(value, field)?;
    let url = url::Url::parse(value).expect("secure browser URL was validated above");
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return Err(ConfigError::InvalidApiAuth(format!(
            "{field} must be an origin without a path, query, or fragment when used for passkeys"
        )));
    }
    if url.domain().is_none() {
        return Err(ConfigError::InvalidApiAuth(format!(
            "{field} must use a DNS hostname for passkeys; use localhost instead of a loopback IP"
        )));
    }
    Ok(())
}

/// Canonical HTTP API configuration used at runtime.
#[derive(Debug, Clone)]
pub struct ResolvedApiHttpConfig {
    pub listen: String,
    pub ssl: Option<ApiTlsConfig>,
    pub auth: Option<ApiAuthConfig>,
    pub cors: Option<ApiCorsConfig>,
    pub webui: Option<ApiWebUiConfig>,
}

/// Tokio runtime configuration.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RuntimeConfig {
    /// Number of Tokio worker threads for the multi-thread runtime.
    ///
    /// When omitted, OxiDNS Next uses the system's available CPU parallelism.
    pub worker_threads: Option<usize>,
}

impl RuntimeConfig {
    /// Resolve the effective Tokio worker-thread count.
    pub fn effective_worker_threads(&self) -> usize {
        self.worker_threads.unwrap_or_else(default_worker_threads)
    }
}

fn default_worker_threads() -> usize {
    std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1)
}

/// Logging configuration
#[derive(Debug, Clone, Deserialize)]
pub struct LogConfig {
    /// Log level: off, trace, debug, info, warn, error
    #[serde(default = "default_level")]
    pub level: String,

    /// Optional file path for log output (in addition to console)
    pub file: Option<String>,

    /// Optional file for DNS query diagnostic events. Query events are never
    /// written to the console, ordinary log file, or management log ring.
    pub query_file: Option<String>,

    #[serde(default)]
    pub rotation: LogRotation,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LogRotation {
    #[default]
    Never,
    Minutely {
        max_files: Option<usize>,
    },
    Hourly {
        max_files: Option<usize>,
    },
    Daily {
        max_files: Option<usize>,
    },
    Weekly {
        max_files: Option<usize>,
    },
}

impl LogRotation {
    #[inline]
    pub fn max_files(&self) -> Option<usize> {
        match self {
            LogRotation::Never => None,
            LogRotation::Minutely { max_files } => *max_files,
            LogRotation::Hourly { max_files } => *max_files,
            LogRotation::Daily { max_files } => *max_files,
            LogRotation::Weekly { max_files } => *max_files,
        }
    }

    #[inline]
    pub fn is_never(&self) -> bool {
        matches!(self, LogRotation::Never)
    }
}

impl Default for LogConfig {
    fn default() -> LogConfig {
        LogConfig {
            level: default_level(),
            file: None,
            query_file: None,
            rotation: LogRotation::Never,
        }
    }
}

/// Default log level
fn default_level() -> String {
    "info".to_string()
}

/// Plugin configuration entry
#[derive(Debug, Clone, Deserialize)]
pub struct PluginConfig {
    /// Unique identifier for this plugin instance
    pub tag: String,

    /// Plugin type (e.g., "udp_server", "forward")
    #[serde(rename = "type")]
    pub plugin_type: String,

    /// Plugin-specific arguments (parsed by plugin factory)
    pub args: Option<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct AuthConfigWrapper {
        auth: ApiAuthConfig,
    }

    fn plugin(tag: &str, plugin_type: &str) -> PluginConfig {
        PluginConfig {
            tag: tag.to_string(),
            plugin_type: plugin_type.to_string(),
            args: None,
        }
    }

    #[test]
    fn test_accounts_auth_deserializes_secure_defaults() {
        let wrapper: AuthConfigWrapper = serde_yaml_ng::from_str(
            r#"
auth:
  type: accounts
"#,
        )
        .expect("accounts auth should deserialize");

        match wrapper.auth {
            ApiAuthConfig::Accounts {
                database,
                bootstrap_token,
                bootstrap_token_env,
                session_ttl_seconds,
                cookie_secure,
                cookie_same_site,
                public_url,
                oidc,
                passkey,
            } => {
                assert_eq!(database, "./data/oxidns-next-auth.db");
                assert!(bootstrap_token.is_none());
                assert!(bootstrap_token_env.is_none());
                assert_eq!(session_ttl_seconds, 43_200);
                assert!(cookie_secure.is_none());
                assert!(matches!(cookie_same_site, ApiCookieSameSite::Lax));
                assert!(public_url.is_none());
                assert!(oidc.is_none());
                assert!(passkey.is_none());
            }
            ApiAuthConfig::Basic { .. } => panic!("expected accounts auth"),
        }
    }

    #[test]
    fn test_accounts_auth_rejects_inline_and_environment_bootstrap_secrets() {
        let wrapper: AuthConfigWrapper = serde_yaml_ng::from_str(
            r#"
auth:
  type: accounts
  bootstrap_token: inline-secret
  bootstrap_token_env: OXIDNS_NEXT_BOOTSTRAP_TOKEN
"#,
        )
        .expect("accounts auth should deserialize");

        let error = wrapper
            .auth
            .validate()
            .expect_err("two bootstrap secret sources should fail validation");
        assert!(matches!(error, ConfigError::InvalidApiAuth(_)));
    }

    #[test]
    fn test_accounts_auth_rejects_cross_site_insecure_cookie() {
        let wrapper: AuthConfigWrapper = serde_yaml_ng::from_str(
            r#"
auth:
  type: accounts
  cookie_secure: false
  cookie_same_site: none
"#,
        )
        .expect("accounts auth should deserialize");

        let error = wrapper
            .auth
            .validate()
            .expect_err("SameSite=None without Secure should fail validation");
        assert!(matches!(error, ConfigError::InvalidApiAuth(_)));
    }

    #[test]
    fn test_passkey_derived_origin_requires_https_away_from_loopback() {
        let wrapper: AuthConfigWrapper = serde_yaml_ng::from_str(
            r#"
auth:
  type: accounts
  public_url: http://dns.example.test
  passkey: {}
"#,
        )
        .expect("accounts auth should deserialize");

        let error = wrapper
            .auth
            .validate()
            .expect_err("a non-loopback passkey origin should require HTTPS");
        assert!(matches!(error, ConfigError::InvalidApiAuth(_)));
    }

    #[test]
    fn test_cors_rejects_non_exact_allowed_origin() {
        let cors = ApiCorsConfig {
            allowed_origins: vec!["https://dns.example.test/path".to_string()],
            ..Default::default()
        };
        let error = cors
            .validate()
            .expect_err("CORS entries must be exact origins");
        assert!(matches!(error, ConfigError::InvalidApiCors(_)));
    }

    #[test]
    fn test_validate_rejects_duplicate_plugin_tags() {
        let config = Config {
            include: Vec::new(),
            runtime: RuntimeConfig::default(),
            api: ApiConfig::default(),
            log: LogConfig::default(),
            network: NetworkConfig::default(),
            plugins: vec![plugin("dup", "debug_print"), plugin("dup", "ttl")],
        };

        let err = config
            .validate()
            .expect_err("should reject duplicate plugin tags");
        assert!(matches!(err, ConfigError::DuplicatePluginTag { .. }));
    }

    #[test]
    fn test_validate_rejects_empty_plugin_type() {
        let config = Config {
            include: Vec::new(),
            runtime: RuntimeConfig::default(),
            api: ApiConfig::default(),
            log: LogConfig::default(),
            network: NetworkConfig::default(),
            plugins: vec![plugin("test", "")],
        };

        let err = config
            .validate()
            .expect_err("should reject empty plugin type");
        assert!(matches!(err, ConfigError::EmptyPluginType));
    }

    #[test]
    fn test_validate_accepts_basic_valid_config() {
        let config = Config {
            include: Vec::new(),
            runtime: RuntimeConfig::default(),
            api: ApiConfig::default(),
            log: LogConfig::default(),
            network: NetworkConfig::default(),
            plugins: vec![plugin("ok", "debug_print")],
        };

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_rejects_zero_runtime_worker_threads() {
        let config = Config {
            include: Vec::new(),
            runtime: RuntimeConfig {
                worker_threads: Some(0),
            },
            api: ApiConfig::default(),
            log: LogConfig::default(),
            network: NetworkConfig::default(),
            plugins: vec![plugin("ok", "debug_print")],
        };

        let err = config
            .validate()
            .expect_err("should reject zero runtime worker threads");
        assert!(matches!(err, ConfigError::InvalidRuntimeWorkerThreads));
    }

    #[test]
    fn test_runtime_worker_threads_default_to_available_parallelism() {
        let expected = std::thread::available_parallelism()
            .map(std::num::NonZeroUsize::get)
            .unwrap_or(1);

        assert_eq!(
            RuntimeConfig::default().effective_worker_threads(),
            expected
        );
    }

    #[test]
    fn test_validate_accepts_api_http_string_shorthand() {
        let config = Config {
            include: Vec::new(),
            runtime: RuntimeConfig::default(),
            api: ApiConfig {
                http: Some(ApiHttpConfig::Listen("0.0.0.0:8080".to_string())),
            },
            log: LogConfig::default(),
            network: NetworkConfig::default(),
            plugins: vec![plugin("ok", "debug_print")],
        };

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_rejects_api_mtls_without_client_ca() {
        let config = Config {
            include: Vec::new(),
            runtime: RuntimeConfig::default(),
            api: ApiConfig {
                http: Some(ApiHttpConfig::Detailed(Box::new(ApiHttpDetailedConfig {
                    listen: "127.0.0.1:9443".to_string(),
                    ssl: Some(ApiTlsConfig {
                        cert: Some("cert.pem".to_string()),
                        key: Some("key.pem".to_string()),
                        client_ca: None,
                        require_client_cert: Some(true),
                    }),
                    auth: None,
                    cors: None,
                    webui: None,
                }))),
            },
            log: LogConfig::default(),
            network: NetworkConfig::default(),
            plugins: vec![plugin("ok", "debug_print")],
        };

        let err = config
            .validate()
            .expect_err("should reject mtls config without client_ca");
        assert!(matches!(err, ConfigError::MissingApiTlsClientCa));
    }

    #[test]
    fn test_validate_accepts_network_outbound_profile() {
        let config: Config = serde_yaml_ng::from_str(
            r#"
network:
  outbound:
    default: remote
    profiles:
      remote:
        resolver:
          nameservers:
            - addr: 1.1.1.1:53
            - addr: 8.8.8.8:53
          ip_version: 4
        proxy:
          socks5: 127.0.0.1:1080
plugins:
  - tag: ok
    type: debug_print
"#,
        )
        .expect("config should deserialize");

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_rejects_padded_default_outbound_profile() {
        let config: Config = serde_yaml_ng::from_str(
            r#"
network:
  outbound:
    default: " remote "
    profiles:
      remote:
        resolver: system
plugins:
  - tag: ok
    type: debug_print
"#,
        )
        .expect("config should deserialize");

        let err = config
            .validate()
            .expect_err("padded outbound default profile should fail");
        assert!(matches!(err, ConfigError::InvalidNetworkOutbound(_)));
    }

    #[test]
    fn test_validate_rejects_padded_outbound_profile_name() {
        let config: Config = serde_yaml_ng::from_str(
            r#"
network:
  outbound:
    profiles:
      " remote ":
        resolver: system
plugins:
  - tag: ok
    type: debug_print
"#,
        )
        .expect("config should deserialize");

        let err = config
            .validate()
            .expect_err("padded outbound profile name should fail");
        assert!(matches!(err, ConfigError::InvalidNetworkOutbound(_)));
    }

    #[test]
    fn test_validate_rejects_profile_resolver_proxy_without_socks5() {
        let config: Config = serde_yaml_ng::from_str(
            r#"
network:
  outbound:
    profiles:
      remote:
        resolver:
          nameservers:
            - addr: tcp://1.1.1.1:53
          proxy: profile
plugins:
  - tag: ok
    type: debug_print
"#,
        )
        .expect("config should deserialize");

        let err = config
            .validate()
            .expect_err("profile resolver proxy without socks5 should fail");
        assert!(matches!(err, ConfigError::InvalidNetworkOutbound(_)));
    }

    #[test]
    fn test_validate_accepts_bracketed_ipv6_outbound_nameserver() {
        let config: Config = serde_yaml_ng::from_str(
            r#"
network:
  outbound:
    profiles:
      remote:
        resolver:
          nameservers:
            - addr: udp://[2001:4860:4860::8888]:53
plugins:
  - tag: ok
    type: debug_print
"#,
        )
        .expect("config should deserialize");

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_rejects_unbracketed_ipv6_outbound_nameserver() {
        let config: Config = serde_yaml_ng::from_str(
            r#"
network:
  outbound:
    profiles:
      remote:
        resolver:
          nameservers:
            - addr: udp://2001:4860:4860::8888:53
plugins:
  - tag: ok
    type: debug_print
"#,
        )
        .expect("config should deserialize");

        let err = config
            .validate()
            .expect_err("unbracketed IPv6 nameserver should fail");
        assert!(matches!(err, ConfigError::InvalidNetworkOutbound(_)));
    }

    #[test]
    fn test_validate_rejects_outbound_resolver_bootstrap() {
        let err = serde_yaml_ng::from_str::<Config>(
            r#"
network:
  outbound:
    profiles:
      remote:
        resolver:
          bootstrap:
            - 1.1.1.1:53
plugins:
  - tag: ok
    type: debug_print
"#,
        )
        .expect_err("outbound resolver.bootstrap should not deserialize");

        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn test_validate_rejects_domain_nameserver_without_dial_addr() {
        let config: Config = serde_yaml_ng::from_str(
            r#"
network:
  outbound:
    profiles:
      remote:
        resolver:
          nameservers:
            - addr: tls://dns.google:853
plugins:
  - tag: ok
    type: debug_print
"#,
        )
        .expect("config should deserialize");

        let err = config
            .validate()
            .expect_err("domain nameserver without dial_addr should fail");
        assert!(matches!(err, ConfigError::InvalidNetworkOutbound(_)));
    }

    #[test]
    fn test_validate_rejects_profile_proxy_with_doq_nameserver() {
        let config: Config = serde_yaml_ng::from_str(
            r#"
network:
  outbound:
    profiles:
      remote:
        resolver:
          nameservers:
            - addr: doq://94.140.14.14:853
          proxy: profile
        proxy:
          socks5: 127.0.0.1:1080
plugins:
  - tag: ok
    type: debug_print
"#,
        )
        .expect("config should deserialize");

        let err = config
            .validate()
            .expect_err("DoQ nameserver cannot use profile proxy");
        assert!(matches!(err, ConfigError::InvalidNetworkOutbound(_)));
    }

    #[test]
    fn test_validate_rejects_unsupported_outbound_nameserver_scheme() {
        let config: Config = serde_yaml_ng::from_str(
            r#"
network:
  outbound:
    profiles:
      remote:
        resolver:
          nameservers:
            - addr: ftp://1.1.1.1:53
plugins:
  - tag: ok
    type: debug_print
"#,
        )
        .expect("config should deserialize");

        let err = config
            .validate()
            .expect_err("unsupported nameserver scheme should fail");
        assert!(matches!(err, ConfigError::InvalidNetworkOutbound(_)));
    }

    #[cfg(not(feature = "resolver-dot"))]
    #[test]
    fn test_validate_rejects_feature_disabled_dot_nameserver() {
        let config: Config = serde_yaml_ng::from_str(
            r#"
network:
  outbound:
    profiles:
      remote:
        resolver:
          nameservers:
            - addr: tls://1.1.1.1:853
plugins:
  - tag: ok
    type: debug_print
"#,
        )
        .expect("config should deserialize");

        let err = config
            .validate()
            .expect_err("DoT nameserver should require resolver-dot");
        assert!(err.to_string().contains("resolver-dot"), "{err}");
    }

    #[cfg(not(feature = "resolver-doh"))]
    #[test]
    fn test_validate_rejects_feature_disabled_doh_nameserver() {
        let config: Config = serde_yaml_ng::from_str(
            r#"
network:
  outbound:
    profiles:
      remote:
        resolver:
          nameservers:
            - addr: https://1.1.1.1/dns-query
plugins:
  - tag: ok
    type: debug_print
"#,
        )
        .expect("config should deserialize");

        let err = config
            .validate()
            .expect_err("DoH nameserver should require resolver-doh");
        assert!(err.to_string().contains("resolver-doh"), "{err}");
    }

    #[cfg(not(feature = "resolver-doq"))]
    #[test]
    fn test_validate_rejects_feature_disabled_doq_nameserver() {
        let config: Config = serde_yaml_ng::from_str(
            r#"
network:
  outbound:
    profiles:
      remote:
        resolver:
          nameservers:
            - addr: doq://94.140.14.14:853
plugins:
  - tag: ok
    type: debug_print
"#,
        )
        .expect("config should deserialize");

        let err = config
            .validate()
            .expect_err("DoQ nameserver should require resolver-doq");
        assert!(err.to_string().contains("resolver-doq"), "{err}");
    }

    #[cfg(not(feature = "resolver-doh3"))]
    #[test]
    fn test_validate_rejects_feature_disabled_doh3_nameserver() {
        let config: Config = serde_yaml_ng::from_str(
            r#"
network:
  outbound:
    profiles:
      remote:
        resolver:
          nameservers:
            - addr: h3://1.1.1.1/dns-query
plugins:
  - tag: ok
    type: debug_print
"#,
        )
        .expect("config should deserialize");

        let err = config
            .validate()
            .expect_err("DoH3 nameserver should require resolver-doh3");
        assert!(err.to_string().contains("resolver-doh3"), "{err}");
    }

    #[test]
    fn test_validate_rejects_invalid_outbound_resolver_timeout() {
        let config: Config = serde_yaml_ng::from_str(
            r#"
network:
  outbound:
    profiles:
      remote:
        resolver:
          nameservers:
            - addr: 1.1.1.1:53
          timeout: nope
plugins:
  - tag: ok
    type: debug_print
"#,
        )
        .expect("config should deserialize");

        let err = config
            .validate()
            .expect_err("invalid resolver timeout should fail");
        assert!(matches!(err, ConfigError::InvalidNetworkOutbound(_)));
    }

    #[test]
    fn test_validate_accepts_hostname_socks5_outbound_proxy_syntax() {
        let config: Config = serde_yaml_ng::from_str(
            r#"
network:
  outbound:
    profiles:
      remote:
        proxy:
          socks5: user:pass@proxy.example.com:1080
plugins:
  - tag: ok
    type: debug_print
"#,
        )
        .expect("config should deserialize");

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_rejects_malformed_socks5_outbound_proxy() {
        let config: Config = serde_yaml_ng::from_str(
            r#"
network:
  outbound:
    profiles:
      remote:
        proxy:
          socks5: 127.0.0.1
plugins:
  - tag: ok
    type: debug_print
"#,
        )
        .expect("config should deserialize");

        let err = config
            .validate()
            .expect_err("malformed socks5 proxy should fail validation");
        assert!(matches!(err, ConfigError::InvalidNetworkOutbound(_)));
    }

    #[test]
    fn test_validate_rejects_unknown_default_outbound_profile() {
        let config: Config = serde_yaml_ng::from_str(
            r#"
network:
  outbound:
    default: missing
plugins:
  - tag: ok
    type: debug_print
"#,
        )
        .expect("config should deserialize");

        let err = config
            .validate()
            .expect_err("missing outbound default profile should fail");
        assert!(matches!(err, ConfigError::InvalidNetworkOutbound(_)));
    }

    #[test]
    fn test_log_rotation_deserializes_minutely() {
        #[derive(Debug, Deserialize)]
        struct Wrapper {
            rotation: LogRotation,
        }

        let config: Wrapper = serde_yaml_ng::from_str(
            r#"
rotation:
  type: minutely
  max_files: 7
"#,
        )
        .expect("parse minutely rotation");

        match config.rotation {
            LogRotation::Minutely { max_files } => assert_eq!(max_files, Some(7)),
            other => panic!("unexpected rotation: {other:?}"),
        }
    }

    #[test]
    fn test_log_rotation_deserializes_weekly() {
        #[derive(Debug, Deserialize)]
        struct Wrapper {
            rotation: LogRotation,
        }

        let config: Wrapper = serde_yaml_ng::from_str(
            r#"
rotation:
  type: weekly
  max_files: 4
"#,
        )
        .expect("parse weekly rotation");

        match config.rotation {
            LogRotation::Weekly { max_files } => assert_eq!(max_files, Some(4)),
            other => panic!("unexpected rotation: {other:?}"),
        }
    }

    #[test]
    fn test_query_diagnostics_allow_the_optional_file_sink_to_be_omitted() {
        let config: Config = serde_yaml_ng::from_str(
            r#"
plugins:
  - tag: query_debug
    type: debug_print
"#,
        )
        .expect("parse query diagnostic config");

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_log_sinks_cannot_share_a_path() {
        let config: Config = serde_yaml_ng::from_str(
            r#"
log:
  file: ./combined.log
  query_file: ./combined.log
"#,
        )
        .expect("parse log config");

        assert!(matches!(
            config.validate(),
            Err(ConfigError::DuplicateLogFilePath)
        ));
    }

    #[test]
    fn test_log_sink_path_comparison_is_absolute_and_lexical() {
        let config: Config = serde_yaml_ng::from_str(
            r#"
log:
  file: ./logs/../combined.log
  query_file: combined.log
"#,
        )
        .expect("parse log config");

        assert!(matches!(
            config.validate(),
            Err(ConfigError::DuplicateLogFilePath)
        ));
    }

    #[cfg(windows)]
    #[test]
    fn test_log_sink_path_comparison_is_case_insensitive_on_windows() {
        let config: Config = serde_yaml_ng::from_str(
            r#"
log:
  file: ./Logs/Combined.log
  query_file: ./logs/combined.LOG
"#,
        )
        .expect("parse log config");

        assert!(matches!(
            config.validate(),
            Err(ConfigError::DuplicateLogFilePath)
        ));
    }
}
