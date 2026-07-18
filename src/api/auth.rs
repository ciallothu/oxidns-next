// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Session-based authentication for the management API.

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm as ArgonAlgorithm, Argon2, Params as ArgonParams, Version};
use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use bytes::Bytes;
use cookie::{Cookie, SameSite};
use http::header::{COOKIE, HOST, LOCATION, ORIGIN, SET_COOKIE};
use http::{HeaderMap, HeaderValue, Method, Request, StatusCode};
use openidconnect::core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata};
use openidconnect::{
    AccessTokenHash, AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl, Nonce,
    OAuth2TokenResponse, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope, TokenResponse,
};
use rand::RngExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::sync::{Mutex, OnceCell, Semaphore};
use totp_rs::{Algorithm as TotpAlgorithm, Secret, TOTP};
use tracing::{info, warn};
use uuid::Uuid;
use webauthn_rs::prelude::{
    Passkey, PasskeyAuthentication, PasskeyRegistration, PublicKeyCredential,
    RegisterPublicKeyCredential, Webauthn, WebauthnBuilder,
};

use crate::api::auth_store::{AuthStore, OidcIdentityRecord, PasskeyRecord, UserRecord};
use crate::api::{ApiHandler, ApiRegister, ApiResponse, json_error, json_ok, simple_response};
use crate::config::types::{ApiAuthConfig, ApiCookieSameSite, ApiOidcConfig};
use crate::infra::error::{DnsError, Result};

const SESSION_COOKIE: &str = "oxidns_next_session";
const OIDC_FLOW_COOKIE_PREFIX: &str = "oxidns_next_oidc_flow_";
const DEFAULT_BOOTSTRAP_TOKEN_ENV: &str = "OXIDNS_NEXT_BOOTSTRAP_TOKEN";
const MAX_JSON_BODY: usize = 64 * 1024;
const MAX_AUTH_QUERY: usize = 16 * 1024;
const LOGIN_CHALLENGE_TTL: Duration = Duration::from_secs(300);
const SETUP_CHALLENGE_TTL: Duration = Duration::from_secs(600);
const OIDC_CHALLENGE_TTL: Duration = Duration::from_secs(600);
const PASSKEY_CHALLENGE_TTL: Duration = Duration::from_secs(300);
const PASSWORD_HASH_CONCURRENCY: usize = 2;

type OidcClient = CoreClient<
    openidconnect::EndpointSet,
    openidconnect::EndpointNotSet,
    openidconnect::EndpointNotSet,
    openidconnect::EndpointNotSet,
    openidconnect::EndpointMaybeSet,
    openidconnect::EndpointMaybeSet,
>;

/// The actual socket peer. Forwarded headers are deliberately not trusted for
/// bootstrap authorization or rate-limit identity.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PeerAddr(pub(crate) SocketAddr);

#[derive(Clone)]
pub(crate) struct AuthPrincipal {
    pub(crate) session_id: String,
    pub(crate) user_id: String,
    pub(crate) username: String,
    pub(crate) auth_method: String,
    pub(crate) csrf_token: String,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) struct AuthMethods {
    pub(crate) password: bool,
    pub(crate) totp: bool,
    pub(crate) oidc: bool,
    pub(crate) passkey: bool,
}

#[derive(Debug)]
struct LoginChallenge {
    user_id: String,
    login_limit_key: String,
    peer_limit_key: String,
    expires_at: Instant,
    attempts: u8,
}

struct TotpSetupChallenge {
    user_id: String,
    secret: String,
    expires_at: Instant,
}

struct OidcChallenge {
    nonce: Nonce,
    pkce_verifier: PkceCodeVerifier,
    browser_token_hash: Vec<u8>,
    return_to: String,
    expires_at: Instant,
}

struct PasskeyRegistrationChallenge {
    user_id: String,
    state: PasskeyRegistration,
    expires_at: Instant,
}

struct PasskeyLoginChallenge {
    user_id: String,
    state: PasskeyAuthentication,
    passkeys: Vec<Passkey>,
    expires_at: Instant,
}

#[derive(Default, Debug)]
struct AttemptLimiter {
    failures: HashMap<String, VecDeque<Instant>>,
}

impl AttemptLimiter {
    fn ensure_allowed(&mut self, key: &str, maximum: usize, window: Duration) -> ApiResult<()> {
        self.cleanup_key(key, window);
        if self
            .failures
            .get(key)
            .is_some_and(|items| items.len() >= maximum)
        {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "too many authentication attempts; try again later",
            ));
        }
        Ok(())
    }

    fn record_failure(&mut self, key: String) {
        if !self.failures.contains_key(&key) && self.failures.len() >= 4096 {
            // Never clear every limiter bucket under attacker-controlled key
            // churn. Evict only the least recently used bucket so stable peer
            // limits continue protecting username-spray paths.
            let oldest = self
                .failures
                .iter()
                .min_by_key(|(_, values)| values.back().copied())
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                self.failures.remove(&oldest);
            }
        }
        self.failures
            .entry(key)
            .or_default()
            .push_back(Instant::now());
    }

    fn clear(&mut self, key: &str) {
        self.failures.remove(key);
    }

    fn cleanup_key(&mut self, key: &str, window: Duration) {
        let cutoff = Instant::now()
            .checked_sub(window)
            .unwrap_or_else(Instant::now);
        let remove = self.failures.get_mut(key).is_some_and(|values| {
            while values.front().is_some_and(|item| *item < cutoff) {
                values.pop_front();
            }
            values.is_empty()
        });
        if remove {
            self.failures.remove(key);
        }
    }
}

struct RuntimeConfig {
    legacy_credentials: Option<(String, String)>,
    bootstrap_token: Option<String>,
    session_ttl: Duration,
    secure_cookie: bool,
    cookie_same_site: ApiCookieSameSite,
    public_origin: Option<String>,
    oidc: Option<ApiOidcConfig>,
    oidc_return_origins: Vec<String>,
}

pub(crate) struct AuthService {
    store: Arc<StdMutex<AuthStore>>,
    config: RuntimeConfig,
    dummy_password_hash: StdMutex<Option<String>>,
    password_hash_slots: Arc<Semaphore>,
    limiter: Mutex<AttemptLimiter>,
    login_challenges: Mutex<HashMap<String, LoginChallenge>>,
    totp_setups: Mutex<HashMap<String, TotpSetupChallenge>>,
    oidc_challenges: Mutex<HashMap<String, OidcChallenge>>,
    oidc_client: OnceCell<OidcClient>,
    passkey_registrations: Mutex<HashMap<String, PasskeyRegistrationChallenge>>,
    passkey_logins: Mutex<HashMap<String, PasskeyLoginChallenge>>,
    webauthn: Option<Webauthn>,
}

impl std::fmt::Debug for AuthService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuthService")
            .field("oidc_enabled", &self.config.oidc.is_some())
            .field("passkey_enabled", &self.webauthn.is_some())
            .finish_non_exhaustive()
    }
}

impl AuthService {
    #[cfg(test)]
    pub(crate) fn new(config: &ApiAuthConfig) -> Result<Arc<Self>> {
        Self::new_with_tls(config, false, &[])
    }

    pub(crate) fn new_with_tls(
        config: &ApiAuthConfig,
        tls_enabled: bool,
        oidc_return_origins: &[String],
    ) -> Result<Arc<Self>> {
        let store = AuthStore::open(config.database_path())?;
        let (
            legacy_credentials,
            bootstrap_token,
            bootstrap_token_env,
            cookie_secure,
            cookie_same_site,
            public_url,
            oidc,
            passkey,
        ) = match config {
            ApiAuthConfig::Basic { username, password } => (
                Some((username.clone(), password.clone())),
                None,
                None,
                None,
                ApiCookieSameSite::Lax,
                None,
                None,
                None,
            ),
            ApiAuthConfig::Accounts {
                bootstrap_token,
                bootstrap_token_env,
                cookie_secure,
                cookie_same_site,
                public_url,
                oidc,
                passkey,
                ..
            } => (
                None,
                bootstrap_token.clone(),
                bootstrap_token_env.clone(),
                *cookie_secure,
                *cookie_same_site,
                public_url.clone(),
                oidc.clone().filter(|config| config.enabled),
                passkey.clone().filter(|config| config.enabled),
            ),
        };
        let bootstrap_token = if matches!(config, ApiAuthConfig::Accounts { .. }) {
            resolve_bootstrap_secret(bootstrap_token, bootstrap_token_env.as_deref())?
        } else {
            None
        };
        let oidc = oidc
            .map(|mut config| -> Result<ApiOidcConfig> {
                config.client_secret = resolve_secret(
                    config.client_secret.take(),
                    config.client_secret_env.as_deref(),
                    "api.http.auth.oidc.client_secret_env",
                )?;
                config.client_secret_env = None;
                Ok(config)
            })
            .transpose()?;
        let secure_cookie = cookie_secure.unwrap_or_else(|| {
            tls_enabled
                || public_url
                    .as_deref()
                    .and_then(|value| url::Url::parse(value).ok())
                    .is_some_and(|url| url.scheme() == "https")
                || oidc
                    .as_ref()
                    .and_then(|config| url::Url::parse(&config.redirect_url).ok())
                    .is_some_and(|url| url.scheme() == "https")
        });
        let public_origin = public_url
            .as_deref()
            .and_then(|value| url::Url::parse(value).ok())
            .map(|url| url.origin().ascii_serialization());
        if matches!(cookie_same_site, ApiCookieSameSite::None) && !secure_cookie {
            return Err(DnsError::config(
                "api.http.auth.cookie_same_site=none requires TLS, an HTTPS public_url, or cookie_secure=true",
            ));
        }
        let webauthn = passkey
            .as_ref()
            .map(|config| build_webauthn(config, public_url.as_deref()))
            .transpose()?;
        Ok(Arc::new(Self {
            store: Arc::new(StdMutex::new(store)),
            config: RuntimeConfig {
                legacy_credentials,
                bootstrap_token,
                session_ttl: Duration::from_secs(config.session_ttl_seconds()),
                secure_cookie,
                cookie_same_site,
                public_origin,
                oidc,
                oidc_return_origins: oidc_return_origins
                    .iter()
                    .filter(|origin| origin.as_str() != "*")
                    .cloned()
                    .collect(),
            },
            dummy_password_hash: StdMutex::new(None),
            password_hash_slots: Arc::new(Semaphore::new(PASSWORD_HASH_CONCURRENCY)),
            limiter: Mutex::new(AttemptLimiter::default()),
            login_challenges: Mutex::new(HashMap::new()),
            totp_setups: Mutex::new(HashMap::new()),
            oidc_challenges: Mutex::new(HashMap::new()),
            oidc_client: OnceCell::new(),
            passkey_registrations: Mutex::new(HashMap::new()),
            passkey_logins: Mutex::new(HashMap::new()),
            webauthn,
        }))
    }

    /// Returns whether a browser origin exactly matches the configured public
    /// API origin. This is the trusted origin when HTTPS terminates at a
    /// reverse proxy; forwarded headers are deliberately not consulted.
    pub(crate) fn allows_public_origin(&self, origin: &str) -> bool {
        self.config.public_origin.as_deref() == Some(origin)
    }

    /// Complete expensive startup-only work away from the async runtime.
    pub(crate) async fn initialize(&self) -> Result<()> {
        let dummy_hash = self
            .password_cpu(|| hash_password("dummy-password-never-accepted"))
            .await?;
        *self
            .dummy_password_hash
            .lock()
            .map_err(|_| DnsError::runtime("authentication hash lock poisoned"))? =
            Some(dummy_hash);

        let Some((username, password)) = self.config.legacy_credentials.clone() else {
            return Ok(());
        };
        if self.user_count().await? != 0 {
            warn!(
                "Deprecated api.http.auth type=basic credentials were ignored because the accounts database is already initialized"
            );
            return Ok(());
        }
        let password_hash = self.password_cpu(move || hash_password(&password)).await?;
        let user_id = Uuid::new_v4().to_string();
        let webauthn_user_id = Uuid::new_v4().to_string();
        let now = unix_seconds();
        self.run_store(move |store| {
            if store.user_count()? == 0 {
                store.create_user(&user_id, &webauthn_user_id, &username, &password_hash, now)?;
            }
            Ok(())
        })
        .await?;
        warn!(
            "Deprecated Basic authentication was migrated once to the persistent accounts database; remove username and password from configuration"
        );
        Ok(())
    }

    pub(crate) fn methods(&self) -> AuthMethods {
        AuthMethods {
            password: true,
            totp: true,
            oidc: self.config.oidc.is_some(),
            passkey: self.webauthn.is_some(),
        }
    }

    pub(crate) async fn setup_required(&self) -> Result<bool> {
        Ok(self.user_count().await? == 0)
    }

    pub(crate) async fn authenticate(&self, headers: &HeaderMap) -> Result<Option<AuthPrincipal>> {
        let Some(token) = session_cookie(headers) else {
            return Ok(None);
        };
        let token_hash = token_hash(&token);
        let now = unix_seconds();
        let session = self
            .run_store(move |store| store.session_by_token_hash(&token_hash, now))
            .await?;
        Ok(session.map(|record| AuthPrincipal {
            session_id: record.id,
            user_id: record.user.id,
            username: record.user.username,
            auth_method: record.auth_method,
            csrf_token: record.csrf_token,
        }))
    }

    pub(crate) fn has_session_cookie(&self, headers: &HeaderMap) -> bool {
        session_cookie(headers).is_some()
    }

    pub(crate) fn verify_csrf(&self, headers: &HeaderMap, principal: &AuthPrincipal) -> bool {
        let Some(provided) = headers
            .get("x-csrf-token")
            .and_then(|value| value.to_str().ok())
        else {
            return false;
        };
        provided
            .as_bytes()
            .ct_eq(principal.csrf_token.as_bytes())
            .into()
    }

    async fn user_count(&self) -> Result<u64> {
        self.run_store(|store| store.user_count()).await
    }

    async fn run_store<T, F>(&self, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut AuthStore) -> Result<T> + Send + 'static,
    {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = store
                .lock()
                .map_err(|_| DnsError::runtime("authentication database lock poisoned"))?;
            operation(&mut store)
        })
        .await
        .map_err(|error| {
            DnsError::runtime(format!("authentication database task failed: {error}"))
        })?
    }

    async fn password_cpu<T, F>(&self, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T> + Send + 'static,
    {
        let _permit = self
            .password_hash_slots
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| DnsError::runtime("authentication password worker closed"))?;
        spawn_cpu(operation).await
    }

    async fn bootstrap(
        &self,
        username: String,
        password: String,
        token: Option<String>,
        peer: Option<SocketAddr>,
        direct_loopback_request: bool,
    ) -> ApiResult<CreatedSession> {
        validate_username(&username)?;
        validate_password(&password)?;
        let limit_key = format!("bootstrap:{}", peer_key(peer));
        self.ensure_rate_limit(&limit_key, 3, Duration::from_secs(600))
            .await?;
        if !direct_loopback_request {
            let Some(expected) = self.config.bootstrap_token.as_deref() else {
                self.record_failure(limit_key).await;
                return Err(ApiError::new(
                    StatusCode::FORBIDDEN,
                    "bootstrap_not_allowed",
                    "non-local bootstrap requires api.http.auth.bootstrap_token, bootstrap_token_env, or OXIDNS_NEXT_BOOTSTRAP_TOKEN",
                ));
            };
            let provided = token.as_deref().unwrap_or_default();
            if !constant_time_equal(provided, expected) {
                self.record_failure(limit_key).await;
                return Err(ApiError::new(
                    StatusCode::FORBIDDEN,
                    "invalid_bootstrap_token",
                    "invalid bootstrap token",
                ));
            }
        }
        if !self.setup_required().await.map_err(ApiError::internal)? {
            self.record_failure(limit_key).await;
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "already_initialized",
                "the administrator account has already been initialized",
            ));
        }
        let password_hash = self
            .password_cpu(move || hash_password(&password))
            .await
            .map_err(ApiError::internal)?;
        let user_id = Uuid::new_v4().to_string();
        let webauthn_user_id = Uuid::new_v4().to_string();
        let create_user_id = user_id.clone();
        let now = unix_seconds();
        self.run_store(move |store| {
            if store.user_count()? != 0 {
                return Err(DnsError::runtime(
                    "administrator account was initialized concurrently",
                ));
            }
            store.create_user(
                &create_user_id,
                &webauthn_user_id,
                &username,
                &password_hash,
                now,
            )?;
            Ok(())
        })
        .await
        .map_err(|error| {
            ApiError::new(
                StatusCode::CONFLICT,
                "already_initialized",
                error.to_string(),
            )
        })?;
        self.clear_rate_limit(&limit_key).await;
        info!(user_id, "Management administrator account initialized");
        self.create_session(&user_id, "bootstrap")
            .await
            .map_err(ApiError::internal)
    }

    async fn login(
        &self,
        username: String,
        password: String,
        peer: Option<SocketAddr>,
    ) -> ApiResult<LoginOutcome> {
        validate_username(&username).map_err(|_| invalid_credentials())?;
        let peer_limit_key = format!("login-peer:{}", peer_key(peer));
        let limit_key = format!("login:{}:{}", peer_key(peer), username.to_lowercase());
        self.ensure_rate_limit(&peer_limit_key, 20, Duration::from_secs(300))
            .await?;
        self.ensure_rate_limit(&limit_key, 5, Duration::from_secs(300))
            .await?;
        let lookup_name = username.clone();
        let user = self
            .run_store(move |store| store.user_by_username(&lookup_name))
            .await
            .map_err(ApiError::internal)?;
        let hash = user
            .as_ref()
            .map(|user| user.password_hash.clone())
            .or_else(|| self.dummy_password_hash.lock().ok()?.clone())
            .ok_or_else(|| ApiError::internal("authentication service is not initialized"))?;
        let password_permit = self
            .password_hash_slots
            .clone()
            .acquire_owned()
            .await
            .map_err(ApiError::internal)?;
        // Requests that queued behind another Argon2 operation must observe
        // failures recorded while they waited before allocating hash memory.
        self.ensure_rate_limit(&peer_limit_key, 20, Duration::from_secs(300))
            .await?;
        self.ensure_rate_limit(&limit_key, 5, Duration::from_secs(300))
            .await?;
        let password_valid = spawn_cpu(move || verify_password(&hash, &password))
            .await
            .map_err(ApiError::internal)?;
        drop(password_permit);
        let Some(user) = user.filter(|_| password_valid) else {
            self.record_failure(limit_key).await;
            self.record_failure(peer_limit_key).await;
            return Err(invalid_credentials());
        };
        if user.totp_enabled {
            let challenge_id = random_token(24);
            let mut challenges = self.login_challenges.lock().await;
            retain_unexpired_login_challenges(&mut challenges);
            challenges.retain(|_, challenge| challenge.user_id != user.id);
            if challenges.len() >= 1024 {
                challenges.clear();
            }
            challenges.insert(
                challenge_id.clone(),
                LoginChallenge {
                    user_id: user.id,
                    login_limit_key: limit_key,
                    peer_limit_key,
                    expires_at: Instant::now() + LOGIN_CHALLENGE_TTL,
                    attempts: 0,
                },
            );
            Ok(LoginOutcome::TotpRequired { challenge_id })
        } else {
            self.clear_rate_limit(&limit_key).await;
            self.clear_rate_limit(&peer_limit_key).await;
            Ok(LoginOutcome::Authenticated(
                self.create_session(&user.id, "password")
                    .await
                    .map_err(ApiError::internal)?,
            ))
        }
    }

    async fn finish_totp_login(
        &self,
        challenge_id: String,
        code: String,
        peer: Option<SocketAddr>,
    ) -> ApiResult<CreatedSession> {
        validate_flow_id(&challenge_id)?;
        let (user_id, login_limit_key, login_peer_limit_key) = {
            let mut challenges = self.login_challenges.lock().await;
            retain_unexpired_login_challenges(&mut challenges);
            let challenge = challenges.get_mut(&challenge_id).ok_or_else(|| {
                ApiError::new(
                    StatusCode::UNAUTHORIZED,
                    "invalid_challenge",
                    "TOTP login challenge is invalid or expired",
                )
            })?;
            challenge.attempts = challenge.attempts.saturating_add(1);
            (
                challenge.user_id.clone(),
                challenge.login_limit_key.clone(),
                challenge.peer_limit_key.clone(),
            )
        };
        let limit_key = format!("totp:{}:{user_id}", peer_key(peer));
        let user_limit_key = format!("totp-user:{user_id}");
        self.ensure_rate_limit(&limit_key, 5, Duration::from_secs(300))
            .await?;
        self.ensure_rate_limit(&user_limit_key, 10, Duration::from_secs(300))
            .await?;
        let lookup_id = user_id.clone();
        let user = self
            .run_store(move |store| store.user_by_id(&lookup_id))
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(invalid_credentials)?;
        let valid = self.verify_second_factor(&user, &code).await?;
        if !valid {
            self.record_failure(limit_key).await;
            self.record_failure(user_limit_key).await;
            let mut challenges = self.login_challenges.lock().await;
            if challenges
                .get(&challenge_id)
                .is_some_and(|challenge| challenge.attempts >= 5)
            {
                challenges.remove(&challenge_id);
            }
            return Err(invalid_credentials());
        }
        if self
            .login_challenges
            .lock()
            .await
            .remove(&challenge_id)
            .is_none()
        {
            return Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "invalid_challenge",
                "TOTP login challenge was already used",
            ));
        }
        self.clear_rate_limit(&limit_key).await;
        self.clear_rate_limit(&user_limit_key).await;
        self.clear_rate_limit(&login_limit_key).await;
        self.clear_rate_limit(&login_peer_limit_key).await;
        self.create_session(&user_id, "password_totp")
            .await
            .map_err(ApiError::internal)
    }

    async fn verify_second_factor(&self, user: &UserRecord, code: &str) -> ApiResult<bool> {
        let code = code.trim();
        if let Some(secret) = user.totp_secret.as_deref() {
            let totp = build_totp(secret, &user.username)?;
            if let Some(counter) = matched_totp_counter(&totp, code, unix_seconds()) {
                let user_id = user.id.clone();
                return self
                    .run_store(move |store| store.consume_totp_counter(&user_id, counter))
                    .await
                    .map_err(ApiError::internal);
            }
        }
        let recovery_hash = recovery_code_hash(code);
        let user_id = user.id.clone();
        self.run_store(move |store| {
            store.consume_recovery_code(&user_id, &recovery_hash, unix_seconds())
        })
        .await
        .map_err(ApiError::internal)
    }

    async fn create_session(&self, user_id: &str, auth_method: &str) -> Result<CreatedSession> {
        let token = random_token(32);
        let csrf_token = random_token(32);
        let id = Uuid::new_v4().to_string();
        let token_hash = token_hash(&token);
        let now = unix_seconds();
        let ttl_seconds = self.config.session_ttl.as_secs().min(i64::MAX as u64) as i64;
        let expires_at = now.saturating_add(ttl_seconds);
        let insert_id = id.clone();
        let insert_user = user_id.to_string();
        let insert_csrf = csrf_token.clone();
        let insert_method = auth_method.to_string();
        self.run_store(move |store| {
            store.purge_expired_sessions(now)?;
            store.create_session(
                &insert_id,
                &token_hash,
                &insert_user,
                &insert_csrf,
                &insert_method,
                now,
                expires_at,
            )
        })
        .await?;
        let user_id = user_id.to_string();
        let user = self
            .run_store(move |store| store.user_by_id(&user_id))
            .await?
            .ok_or_else(|| DnsError::runtime("session user disappeared"))?;
        Ok(CreatedSession {
            token,
            principal: AuthPrincipal {
                session_id: id,
                user_id: user.id,
                username: user.username,
                auth_method: auth_method.to_string(),
                csrf_token,
            },
        })
    }

    async fn logout(&self, principal: &AuthPrincipal) -> ApiResult<()> {
        let session_id = principal.session_id.clone();
        self.run_store(move |store| store.revoke_session(&session_id, unix_seconds()))
            .await
            .map_err(ApiError::internal)
    }

    async fn change_password(
        &self,
        principal: &AuthPrincipal,
        current_password: String,
        new_password: String,
        peer: Option<SocketAddr>,
    ) -> ApiResult<()> {
        validate_password(&new_password)?;
        let limit_key = format!("password:{}:{}", peer_key(peer), principal.user_id);
        self.ensure_rate_limit(&limit_key, 5, Duration::from_secs(300))
            .await?;
        let user_id = principal.user_id.clone();
        let user = self
            .run_store(move |store| store.user_by_id(&user_id))
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(invalid_credentials)?;
        let hash = user.password_hash;
        let valid = self
            .password_cpu(move || verify_password(&hash, &current_password))
            .await
            .map_err(ApiError::internal)?;
        if !valid {
            self.record_failure(limit_key).await;
            return Err(step_up_failed());
        }
        let new_hash = self
            .password_cpu(move || hash_password(&new_password))
            .await
            .map_err(ApiError::internal)?;
        let update_user = principal.user_id.clone();
        let current_session = principal.session_id.clone();
        self.run_store(move |store| {
            let now = unix_seconds();
            store.set_password_hash(&update_user, &new_hash, now)?;
            store.revoke_other_sessions(&update_user, &current_session, now)
        })
        .await
        .map_err(ApiError::internal)?;
        self.clear_rate_limit(&limit_key).await;
        Ok(())
    }

    async fn begin_totp(&self, principal: &AuthPrincipal) -> ApiResult<TotpSetupResponse> {
        let user_id = principal.user_id.clone();
        let already_enabled = self
            .run_store(move |store| {
                Ok(store
                    .user_by_id(&user_id)?
                    .is_some_and(|user| user.totp_enabled))
            })
            .await
            .map_err(ApiError::internal)?;
        if already_enabled {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "totp_already_enabled",
                "disable the current TOTP factor before enrolling a replacement",
            ));
        }
        let secret = Secret::generate_secret().to_encoded().to_string();
        let totp = build_totp(&secret, &principal.username)?;
        let response = TotpSetupResponse {
            ok: true,
            secret: secret.clone(),
            otpauth_uri: totp.get_url(),
            expires_in: SETUP_CHALLENGE_TTL.as_secs(),
        };
        let mut setups = self.totp_setups.lock().await;
        setups.retain(|_, setup| setup.expires_at > Instant::now());
        setups.insert(
            principal.user_id.clone(),
            TotpSetupChallenge {
                user_id: principal.user_id.clone(),
                secret,
                expires_at: Instant::now() + SETUP_CHALLENGE_TTL,
            },
        );
        Ok(response)
    }

    async fn confirm_totp(
        &self,
        principal: &AuthPrincipal,
        code: String,
    ) -> ApiResult<Vec<String>> {
        let secret = {
            let setups = self.totp_setups.lock().await;
            let setup = setups.get(&principal.user_id).ok_or_else(|| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "invalid_challenge",
                    "TOTP setup challenge is missing or expired",
                )
            })?;
            if setup.expires_at <= Instant::now() || setup.user_id != principal.user_id {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "invalid_challenge",
                    "TOTP setup challenge is missing or expired",
                ));
            }
            setup.secret.clone()
        };
        let totp = build_totp(&secret, &principal.username)?;
        let now = unix_seconds();
        let Some(accepted_counter) = matched_totp_counter(&totp, code.trim(), now) else {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "invalid_totp",
                "invalid TOTP code",
            ));
        };
        let consumed = self
            .totp_setups
            .lock()
            .await
            .remove(&principal.user_id)
            .is_some_and(|setup| setup.secret == secret && setup.expires_at > Instant::now());
        if !consumed {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "invalid_challenge",
                "TOTP setup challenge was already used or expired",
            ));
        }
        let recovery_codes = generate_recovery_codes(10);
        let stored_codes = recovery_codes
            .iter()
            .map(|code| (Uuid::new_v4().to_string(), recovery_code_hash(code)))
            .collect::<Vec<_>>();
        let user_id = principal.user_id.clone();
        self.run_store(move |store| {
            store.set_totp(&user_id, &secret, accepted_counter, &stored_codes, now)
        })
        .await
        .map_err(ApiError::internal)?;
        Ok(recovery_codes)
    }

    async fn disable_totp(
        &self,
        principal: &AuthPrincipal,
        password: String,
        code: String,
        peer: Option<SocketAddr>,
    ) -> ApiResult<()> {
        let limit_key = format!("totp-disable:{}:{}", peer_key(peer), principal.user_id);
        self.ensure_rate_limit(&limit_key, 5, Duration::from_secs(300))
            .await?;
        let user_id = principal.user_id.clone();
        let user = self
            .run_store(move |store| store.user_by_id(&user_id))
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(invalid_credentials)?;
        let hash = user.password_hash.clone();
        let password_valid = self
            .password_cpu(move || verify_password(&hash, &password))
            .await
            .map_err(ApiError::internal)?;
        if !password_valid || !self.verify_second_factor(&user, &code).await? {
            self.record_failure(limit_key).await;
            return Err(step_up_failed());
        }
        let user_id = principal.user_id.clone();
        self.run_store(move |store| store.clear_totp(&user_id, unix_seconds()))
            .await
            .map_err(ApiError::internal)?;
        self.clear_rate_limit(&limit_key).await;
        Ok(())
    }

    async fn security_summary(&self, principal: &AuthPrincipal) -> ApiResult<SecurityResponse> {
        let user_id = principal.user_id.clone();
        let (user, passkeys, identities) = self
            .run_store(move |store| {
                let user = store
                    .user_by_id(&user_id)?
                    .ok_or_else(|| DnsError::runtime("authenticated user disappeared"))?;
                let passkeys = store.passkeys_for_user(&user_id)?;
                let identities = store.oidc_identities_for_user(&user_id)?;
                Ok((user, passkeys, identities))
            })
            .await
            .map_err(ApiError::internal)?;
        Ok(SecurityResponse {
            ok: true,
            totp_enabled: user.totp_enabled,
            passkeys: passkeys.iter().map(PasskeySummary::from).collect(),
            oidc_identities: identities.iter().map(OidcIdentitySummary::from).collect(),
        })
    }

    async fn begin_passkey_registration(
        &self,
        principal: &AuthPrincipal,
    ) -> ApiResult<PasskeyFlowResponse> {
        let webauthn = self.webauthn.as_ref().ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                "passkey_disabled",
                "passkey authentication is not enabled",
            )
        })?;
        let user_id = principal.user_id.clone();
        let (user, records) = self
            .run_store(move |store| {
                let user = store
                    .user_by_id(&user_id)?
                    .ok_or_else(|| DnsError::runtime("authenticated user disappeared"))?;
                let records = store.passkeys_for_user(&user_id)?;
                Ok((user, records))
            })
            .await
            .map_err(ApiError::internal)?;
        let existing = records
            .iter()
            .map(|record| serde_json::from_str::<Passkey>(&record.credential_json))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(ApiError::internal)?;
        let excluded = (!existing.is_empty()).then(|| {
            existing
                .iter()
                .map(|passkey| passkey.cred_id().clone())
                .collect()
        });
        let webauthn_user_id = Uuid::parse_str(&user.webauthn_user_id)
            .map_err(|error| ApiError::internal(format!("invalid WebAuthn user id: {error}")))?;
        let (options, state) = webauthn
            .start_passkey_registration(webauthn_user_id, &user.username, &user.username, excluded)
            .map_err(ApiError::internal)?;
        let flow_id = random_token(24);
        let mut flows = self.passkey_registrations.lock().await;
        flows.retain(|_, flow| {
            flow.expires_at > Instant::now() && flow.user_id != principal.user_id
        });
        if flows.len() >= 1024 {
            flows.clear();
        }
        flows.insert(
            flow_id.clone(),
            PasskeyRegistrationChallenge {
                user_id: principal.user_id.clone(),
                state,
                expires_at: Instant::now() + PASSKEY_CHALLENGE_TTL,
            },
        );
        Ok(PasskeyFlowResponse {
            ok: true,
            flow_id,
            options: serde_json::to_value(options).map_err(ApiError::internal)?,
        })
    }

    async fn finish_passkey_registration(
        &self,
        principal: &AuthPrincipal,
        flow_id: String,
        credential: serde_json::Value,
        name: Option<String>,
    ) -> ApiResult<PasskeySummary> {
        validate_flow_id(&flow_id)?;
        let webauthn = self.webauthn.as_ref().ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                "passkey_disabled",
                "passkey authentication is not enabled",
            )
        })?;
        let flow = self
            .passkey_registrations
            .lock()
            .await
            .remove(&flow_id)
            .filter(|flow| flow.expires_at > Instant::now() && flow.user_id == principal.user_id)
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "invalid_challenge",
                    "passkey registration challenge is invalid or expired",
                )
            })?;
        let credential = serde_json::from_value::<RegisterPublicKeyCredential>(credential)
            .map_err(|error| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "invalid_passkey",
                    format!("invalid passkey credential: {error}"),
                )
            })?;
        let passkey = webauthn
            .finish_passkey_registration(&credential, &flow.state)
            .map_err(|_| {
                ApiError::new(
                    StatusCode::UNAUTHORIZED,
                    "invalid_passkey",
                    "passkey registration verification failed",
                )
            })?;
        let name = validate_passkey_name(name.as_deref().unwrap_or("Passkey"))?;
        let id = Uuid::new_v4().to_string();
        let credential_id = passkey.cred_id().as_ref().to_vec();
        let credential_json = serde_json::to_string(&passkey).map_err(ApiError::internal)?;
        let user_id = principal.user_id.clone();
        let insert_id = id.clone();
        let insert_name = name.clone();
        let now = unix_seconds();
        self.run_store(move |store| {
            store.insert_passkey(
                &insert_id,
                &user_id,
                &insert_name,
                &credential_id,
                &credential_json,
                now,
            )
        })
        .await
        .map_err(ApiError::internal)?;
        Ok(PasskeySummary {
            id,
            name,
            created_at_ms: now.saturating_mul(1000),
            last_used_at_ms: None,
        })
    }

    async fn begin_passkey_login(
        &self,
        username: String,
        peer: Option<SocketAddr>,
    ) -> ApiResult<PasskeyFlowResponse> {
        validate_username(&username).map_err(|_| invalid_credentials())?;
        let webauthn = self.webauthn.as_ref().ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                "passkey_disabled",
                "passkey authentication is not enabled",
            )
        })?;
        let peer_limit_key = format!("passkey-peer:{}", peer_key(peer));
        let limit_key = format!("passkey:{}:{}", peer_key(peer), username.to_lowercase());
        self.ensure_rate_limit(&peer_limit_key, 30, Duration::from_secs(300))
            .await?;
        self.ensure_rate_limit(&limit_key, 10, Duration::from_secs(300))
            .await?;
        self.record_failure(limit_key.clone()).await;
        self.record_failure(peer_limit_key).await;
        let lookup_name = username;
        let result = self
            .run_store(move |store| {
                let Some(user) = store.user_by_username(&lookup_name)? else {
                    return Ok(None);
                };
                let records = store.passkeys_for_user(&user.id)?;
                Ok(Some((user, records)))
            })
            .await
            .map_err(ApiError::internal)?;
        let Some((user, records)) = result.filter(|(_, records)| !records.is_empty()) else {
            return Err(invalid_credentials());
        };
        let passkeys = records
            .iter()
            .map(|record| serde_json::from_str::<Passkey>(&record.credential_json))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(ApiError::internal)?;
        let (options, state) = webauthn
            .start_passkey_authentication(&passkeys)
            .map_err(ApiError::internal)?;
        let flow_id = random_token(24);
        let user_id = user.id;
        let mut flows = self.passkey_logins.lock().await;
        flows.retain(|_, flow| flow.expires_at > Instant::now() && flow.user_id != user_id);
        if flows.len() >= 1024 {
            flows.clear();
        }
        flows.insert(
            flow_id.clone(),
            PasskeyLoginChallenge {
                user_id,
                state,
                passkeys,
                expires_at: Instant::now() + PASSKEY_CHALLENGE_TTL,
            },
        );
        Ok(PasskeyFlowResponse {
            ok: true,
            flow_id,
            options: serde_json::to_value(options).map_err(ApiError::internal)?,
        })
    }

    async fn finish_passkey_login(
        &self,
        flow_id: String,
        credential: serde_json::Value,
        peer: Option<SocketAddr>,
    ) -> ApiResult<CreatedSession> {
        validate_flow_id(&flow_id)?;
        let webauthn = self.webauthn.as_ref().ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                "passkey_disabled",
                "passkey authentication is not enabled",
            )
        })?;
        let limit_key = format!("passkey-finish:{}", peer_key(peer));
        self.ensure_rate_limit(&limit_key, 20, Duration::from_secs(300))
            .await?;
        self.record_failure(limit_key).await;
        let mut flow = self
            .passkey_logins
            .lock()
            .await
            .remove(&flow_id)
            .filter(|flow| flow.expires_at > Instant::now())
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "invalid_challenge",
                    "passkey login challenge is invalid or expired",
                )
            })?;
        let credential =
            serde_json::from_value::<PublicKeyCredential>(credential).map_err(|error| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "invalid_passkey",
                    format!("invalid passkey credential: {error}"),
                )
            })?;
        let auth_result = webauthn
            .finish_passkey_authentication(&credential, &flow.state)
            .map_err(|_| {
                ApiError::new(
                    StatusCode::UNAUTHORIZED,
                    "invalid_passkey",
                    "passkey authentication failed",
                )
            })?;
        let credential_id = auth_result.cred_id().as_ref().to_vec();
        let passkey = flow
            .passkeys
            .iter_mut()
            .find(|passkey| passkey.cred_id().as_ref() == credential_id.as_slice())
            .ok_or_else(invalid_credentials)?;
        let expected_credential_json =
            serde_json::to_string(&*passkey).map_err(ApiError::internal)?;
        passkey.update_credential(&auth_result);
        let credential_json = serde_json::to_string(passkey).map_err(ApiError::internal)?;
        let user_id = flow.user_id.clone();
        let update_user_id = user_id.clone();
        let still_registered = self
            .run_store(move |store| {
                store.update_passkey_credential(
                    &update_user_id,
                    &credential_id,
                    &expected_credential_json,
                    &credential_json,
                    unix_seconds(),
                )
            })
            .await
            .map_err(ApiError::internal)?;
        if !still_registered {
            return Err(invalid_credentials());
        }
        self.create_session(&user_id, "passkey")
            .await
            .map_err(ApiError::internal)
    }

    async fn rename_passkey(
        &self,
        principal: &AuthPrincipal,
        id: String,
        name: String,
    ) -> ApiResult<PasskeySummary> {
        let name = validate_passkey_name(&name)?;
        let user_id = principal.user_id.clone();
        let lookup_id = id.clone();
        let lookup_name = name.clone();
        let record = self
            .run_store(move |store| {
                if !store.rename_passkey(&user_id, &lookup_id, &lookup_name)? {
                    return Ok(None);
                }
                Ok(store
                    .passkeys_for_user(&user_id)?
                    .into_iter()
                    .find(|record| record.id == lookup_id))
            })
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(|| {
                ApiError::new(StatusCode::NOT_FOUND, "not_found", "passkey not found")
            })?;
        Ok(PasskeySummary::from(&record))
    }

    async fn delete_passkey(&self, principal: &AuthPrincipal, id: String) -> ApiResult<()> {
        let user_id = principal.user_id.clone();
        let deleted = self
            .run_store(move |store| store.delete_passkey(&user_id, &id))
            .await
            .map_err(ApiError::internal)?;
        if !deleted {
            return Err(ApiError::new(
                StatusCode::NOT_FOUND,
                "not_found",
                "passkey not found",
            ));
        }
        Ok(())
    }

    async fn begin_oidc(
        &self,
        return_to: Option<String>,
        peer: Option<SocketAddr>,
    ) -> ApiResult<(String, String, String)> {
        let config = self.config.oidc.as_ref().ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                "oidc_disabled",
                "OIDC authentication is not enabled",
            )
        })?;
        let limit_key = format!("oidc-start:{}", peer_key(peer));
        self.ensure_rate_limit(&limit_key, 20, Duration::from_secs(300))
            .await?;
        self.record_failure(limit_key).await;
        let client = self.oidc_client(config).await?;
        let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
        let mut authorization = client.authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        );
        // `openidconnect::Client::authorize_url` adds the mandatory `openid`
        // scope itself. Avoid sending it twice because some providers reject
        // duplicated scope values even though they are semantically harmless.
        for scope in config
            .scopes
            .iter()
            .filter(|scope| scope.as_str() != "openid")
        {
            authorization = authorization.add_scope(Scope::new(scope.clone()));
        }
        let (url, state, nonce) = authorization.set_pkce_challenge(pkce_challenge).url();
        let return_to = resolve_oidc_return_to(
            return_to,
            &config.success_redirect,
            &self.config.oidc_return_origins,
        );
        let state_value = state.secret().to_string();
        let browser_token = random_token(24);
        let mut challenges = self.oidc_challenges.lock().await;
        challenges.retain(|_, challenge| challenge.expires_at > Instant::now());
        if challenges.len() >= 1024 {
            challenges.clear();
        }
        challenges.insert(
            state_value,
            OidcChallenge {
                nonce,
                pkce_verifier,
                browser_token_hash: token_hash(&browser_token),
                return_to,
                expires_at: Instant::now() + OIDC_CHALLENGE_TTL,
            },
        );
        Ok((url.to_string(), state.secret().to_string(), browser_token))
    }

    async fn finish_oidc(
        &self,
        code: String,
        state: String,
        browser_token: String,
    ) -> ApiResult<(CreatedSession, String)> {
        validate_flow_id(&state)?;
        if code.is_empty() || code.len() > 8192 {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "invalid_oidc_callback",
                "OIDC authorization code is invalid",
            ));
        }
        let config = self.config.oidc.as_ref().ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                "oidc_disabled",
                "OIDC authentication is not enabled",
            )
        })?;
        let challenge = self
            .oidc_challenges
            .lock()
            .await
            .remove(&state)
            .filter(|challenge| challenge.expires_at > Instant::now())
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "invalid_oidc_state",
                    "OIDC state is invalid or expired",
                )
            })?;
        if !bool::from(
            challenge
                .browser_token_hash
                .as_slice()
                .ct_eq(token_hash(&browser_token).as_slice()),
        ) {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "invalid_oidc_state",
                "OIDC browser flow is invalid or expired",
            ));
        }
        let client = self.oidc_client(config).await?;
        let http_client = oidc_http_client()?;
        let token_response = client
            .exchange_code(AuthorizationCode::new(code))
            .map_err(ApiError::internal)?
            .set_pkce_verifier(challenge.pkce_verifier)
            .request_async(&http_client)
            .await
            .map_err(ApiError::internal)?;
        let id_token = token_response.id_token().ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "invalid_oidc_token",
                "OIDC provider did not return an ID token",
            )
        })?;
        let verifier = client.id_token_verifier();
        let claims = id_token.claims(&verifier, &challenge.nonce).map_err(|_| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "invalid_oidc_token",
                "OIDC ID token verification failed",
            )
        })?;
        if let Some(expected_hash) = claims.access_token_hash() {
            let actual_hash = AccessTokenHash::from_token(
                token_response.access_token(),
                id_token.signing_alg().map_err(ApiError::internal)?,
                id_token
                    .signing_key(&verifier)
                    .map_err(ApiError::internal)?,
            )
            .map_err(ApiError::internal)?;
            if actual_hash != *expected_hash {
                return Err(ApiError::new(
                    StatusCode::UNAUTHORIZED,
                    "invalid_oidc_token",
                    "OIDC access token hash verification failed",
                ));
            }
        }
        let issuer = claims.issuer().as_str().to_string();
        let subject = claims.subject().as_str().to_string();
        if config.username_claim == "email" && claims.email_verified() != Some(true) {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "oidc_user_not_allowed",
                "OIDC email claim is not verified",
            ));
        }
        let claim_value = oidc_claim_value(claims, &config.username_claim).ok_or_else(|| {
            ApiError::new(
                StatusCode::FORBIDDEN,
                "oidc_user_not_allowed",
                "OIDC token does not contain the configured username claim",
            )
        })?;
        let mapping = config
            .allowed_users
            .iter()
            .find(|mapping| mapping.claim == claim_value)
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::FORBIDDEN,
                    "oidc_user_not_allowed",
                    "OIDC identity is not in the administrator allowlist",
                )
            })?;
        let mapped_username = mapping.username.clone();
        let display_name = claims
            .name()
            .and_then(|name| name.get(None))
            .map(|name| name.as_str().to_string())
            .or_else(|| Some(claim_value.clone()));
        let lookup_issuer = issuer.clone();
        let lookup_subject = subject.clone();
        let user = self
            .run_store(move |store| {
                if let Some(user) = store.user_for_oidc_identity(&lookup_issuer, &lookup_subject)? {
                    if !user.username.eq_ignore_ascii_case(&mapped_username) {
                        return Err(DnsError::runtime(
                            "OIDC identity is linked to a different local account",
                        ));
                    }
                    return Ok(user);
                }
                let user = store
                    .user_by_username(&mapped_username)?
                    .ok_or_else(|| DnsError::runtime("mapped OIDC local account does not exist"))?;
                store.link_oidc_identity(
                    &Uuid::new_v4().to_string(),
                    &user.id,
                    &lookup_issuer,
                    &lookup_subject,
                    display_name.as_deref(),
                    unix_seconds(),
                )?;
                Ok(user)
            })
            .await
            .map_err(|error| {
                ApiError::new(
                    StatusCode::FORBIDDEN,
                    "oidc_user_not_allowed",
                    error.to_string(),
                )
            })?;
        let session = self
            .create_session(&user.id, "oidc")
            .await
            .map_err(ApiError::internal)?;
        Ok((session, challenge.return_to))
    }

    async fn oidc_client(&self, config: &ApiOidcConfig) -> ApiResult<&OidcClient> {
        self.oidc_client
            .get_or_try_init(|| discover_oidc_client(config))
            .await
    }

    async fn ensure_rate_limit(
        &self,
        key: &str,
        maximum: usize,
        window: Duration,
    ) -> ApiResult<()> {
        self.limiter
            .lock()
            .await
            .ensure_allowed(key, maximum, window)
    }

    async fn record_failure(&self, key: String) {
        self.limiter.lock().await.record_failure(key);
    }

    async fn clear_rate_limit(&self, key: &str) {
        self.limiter.lock().await.clear(key);
    }
}

#[derive(Serialize)]
struct PasskeyFlowResponse {
    ok: bool,
    flow_id: String,
    options: serde_json::Value,
}

fn validate_passkey_name(name: &str) -> ApiResult<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_passkey_name",
            "passkey name must contain 1 to 64 non-control characters",
        ));
    }
    Ok(name.to_string())
}

fn validate_flow_id(value: &str) -> ApiResult<()> {
    if !(16..=128).contains(&value.len())
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_challenge",
            "authentication challenge identifier is invalid",
        ));
    }
    Ok(())
}

fn is_safe_return_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.starts_with("//")
        && !path.contains('\\')
        && !path.chars().any(char::is_control)
}

fn resolve_oidc_return_to(
    requested: Option<String>,
    fallback: &str,
    allowed_origins: &[String],
) -> String {
    let Some(requested) = requested else {
        return fallback.to_string();
    };
    if is_safe_return_path(&requested) {
        return requested;
    }
    let Some(url) = url::Url::parse(&requested).ok().filter(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
    }) else {
        return fallback.to_string();
    };
    let origin = url.origin().ascii_serialization();
    if allowed_origins.iter().any(|allowed| allowed == &origin) {
        url.to_string()
    } else {
        fallback.to_string()
    }
}

fn oidc_http_client() -> ApiResult<openidconnect::reqwest::Client> {
    openidconnect::reqwest::ClientBuilder::new()
        .redirect(openidconnect::reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(ApiError::internal)
}

async fn discover_oidc_client(config: &ApiOidcConfig) -> ApiResult<OidcClient> {
    let issuer = IssuerUrl::new(config.issuer_url.clone()).map_err(ApiError::internal)?;
    let http_client = oidc_http_client()?;
    let metadata = CoreProviderMetadata::discover_async(issuer, &http_client)
        .await
        .map_err(ApiError::internal)?;
    let client_secret = config.client_secret.clone().map(ClientSecret::new);
    let redirect_url = RedirectUrl::new(config.redirect_url.clone()).map_err(ApiError::internal)?;
    Ok(CoreClient::from_provider_metadata(
        metadata,
        ClientId::new(config.client_id.clone()),
        client_secret,
    )
    .set_redirect_uri(redirect_url))
}

fn oidc_claim_value(
    claims: &openidconnect::core::CoreIdTokenClaims,
    claim_name: &str,
) -> Option<String> {
    match claim_name {
        "preferred_username" => claims
            .preferred_username()
            .map(|value| value.as_str().to_string()),
        "email" => claims.email().map(|value| value.as_str().to_string()),
        "name" => claims
            .name()
            .and_then(|value| value.get(None))
            .map(|value| value.as_str().to_string()),
        "sub" => Some(claims.subject().as_str().to_string()),
        _ => None,
    }
}

struct CreatedSession {
    token: String,
    principal: AuthPrincipal,
}

enum LoginOutcome {
    Authenticated(CreatedSession),
    TotpRequired { challenge_id: String },
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

type ApiResult<T> = std::result::Result<T, ApiError>;

impl ApiError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    fn internal(error: impl std::fmt::Display) -> Self {
        warn!(error = %error, "Management authentication request failed");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "authentication service failed",
        )
    }

    fn response(self) -> ApiResponse {
        json_error(self.status, self.code, self.message)
    }
}

fn invalid_credentials() -> ApiError {
    ApiError::new(
        StatusCode::UNAUTHORIZED,
        "invalid_credentials",
        "invalid username or authentication credential",
    )
}

fn step_up_failed() -> ApiError {
    ApiError::new(
        StatusCode::FORBIDDEN,
        "step_up_failed",
        "current password or second-factor code is invalid",
    )
}

async fn spawn_cpu<T, F>(operation: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| DnsError::runtime(format!("authentication CPU task failed: {error}")))?
}

fn argon2() -> Result<Argon2<'static>> {
    let params = ArgonParams::new(19_456, 2, 1, None)
        .map_err(|error| DnsError::runtime(format!("invalid Argon2 parameters: {error}")))?;
    Ok(Argon2::new(
        ArgonAlgorithm::Argon2id,
        Version::V0x13,
        params,
    ))
}

fn hash_password(password: &str) -> Result<String> {
    let mut salt_bytes = [0u8; 16];
    rand::rng().fill(&mut salt_bytes);
    let salt = SaltString::encode_b64(&salt_bytes)
        .map_err(|error| DnsError::runtime(format!("failed to encode password salt: {error}")))?;
    argon2()?
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| DnsError::runtime(format!("failed to hash password: {error}")))
}

fn verify_password(encoded_hash: &str, password: &str) -> Result<bool> {
    let parsed = PasswordHash::new(encoded_hash).map_err(|error| {
        DnsError::runtime(format!("invalid password hash in database: {error}"))
    })?;
    Ok(argon2()?
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

fn validate_username(username: &str) -> ApiResult<()> {
    if username.trim() != username
        || !(1..=64).contains(&username.chars().count())
        || username.chars().any(char::is_control)
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_username",
            "username must contain 1 to 64 non-control characters without surrounding whitespace",
        ));
    }
    Ok(())
}

fn validate_password(password: &str) -> ApiResult<()> {
    if !(12..=1024).contains(&password.chars().count()) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "weak_password",
            "password must contain between 12 and 1024 characters",
        ));
    }
    Ok(())
}

fn build_totp(secret: &str, username: &str) -> ApiResult<TOTP> {
    let bytes = Secret::Encoded(secret.to_string())
        .to_bytes()
        .map_err(ApiError::internal)?;
    // `totp-rs` reserves ':' as the issuer/account separator in otpauth URLs.
    // Usernames may legitimately contain it, so keep authentication identity
    // unchanged and sanitize only the human-readable authenticator label.
    let account_name = username.replace(':', "_");
    TOTP::new(
        TotpAlgorithm::SHA1,
        6,
        1,
        30,
        bytes,
        Some("OxiDNS Next".to_string()),
        account_name,
    )
    .map_err(ApiError::internal)
}

fn matched_totp_counter(totp: &TOTP, token: &str, unix_time: i64) -> Option<i64> {
    let unix_time = u64::try_from(unix_time).ok()?;
    if totp.step == 0 {
        return None;
    }
    let current_counter = unix_time / totp.step;
    let skew = u64::from(totp.skew);
    let first_counter = current_counter.saturating_sub(skew);
    let last_counter = current_counter.saturating_add(skew);

    (first_counter..=last_counter)
        .find(|counter| {
            let step_time = counter.saturating_mul(totp.step);
            totp.generate(step_time)
                .as_bytes()
                .ct_eq(token.as_bytes())
                .into()
        })
        .and_then(|counter| i64::try_from(counter).ok())
}

fn build_webauthn(
    config: &crate::config::types::ApiPasskeyConfig,
    public_url: Option<&str>,
) -> Result<Webauthn> {
    let origin_strings = if config.origins.is_empty() {
        vec![
            public_url
                .ok_or_else(|| DnsError::config("passkey origin is missing"))?
                .to_string(),
        ]
    } else {
        config.origins.clone()
    };
    let origins = origin_strings
        .iter()
        .map(|origin| {
            url::Url::parse(origin)
                .map_err(|error| DnsError::config(format!("invalid passkey origin: {error}")))
        })
        .collect::<Result<Vec<_>>>()?;
    let rp_id = config
        .rp_id
        .clone()
        .or_else(|| {
            public_url
                .and_then(|value| url::Url::parse(value).ok())
                .and_then(|url| url.host_str().map(str::to_string))
        })
        .ok_or_else(|| DnsError::config("passkey rp_id is missing"))?;
    let mut builder = WebauthnBuilder::new(&rp_id, &origins[0])
        .map_err(|error| DnsError::config(format!("invalid passkey configuration: {error}")))?
        .rp_name("OxiDNS Next");
    for origin in origins.iter().skip(1) {
        builder = builder.append_allowed_origin(origin);
    }
    builder
        .build()
        .map_err(|error| DnsError::config(format!("invalid passkey configuration: {error}")))
}

fn resolve_secret(
    inline: Option<String>,
    environment: Option<&str>,
    field: &str,
) -> Result<Option<String>> {
    resolve_secret_with(inline, environment, field, |name| std::env::var(name))
}

fn resolve_secret_with<F>(
    inline: Option<String>,
    environment: Option<&str>,
    field: &str,
    lookup: F,
) -> Result<Option<String>>
where
    F: Fn(&str) -> std::result::Result<String, std::env::VarError>,
{
    if let Some(value) = inline {
        return Ok(Some(value));
    }
    let Some(variable) = environment else {
        return Ok(None);
    };
    let value = lookup(variable).map_err(|error| {
        DnsError::config(format!(
            "{field} references unavailable environment variable: {error}"
        ))
    })?;
    if value.is_empty() {
        return Err(DnsError::config(format!(
            "{field} references an empty environment variable"
        )));
    }
    Ok(Some(value))
}

fn resolve_bootstrap_secret(
    inline: Option<String>,
    environment: Option<&str>,
) -> Result<Option<String>> {
    resolve_bootstrap_secret_with(inline, environment, |name| std::env::var(name))
}

fn resolve_bootstrap_secret_with<F>(
    inline: Option<String>,
    environment: Option<&str>,
    lookup: F,
) -> Result<Option<String>>
where
    F: Fn(&str) -> std::result::Result<String, std::env::VarError>,
{
    if inline.is_some() || environment.is_some() {
        return resolve_secret_with(
            inline,
            environment,
            "api.http.auth.bootstrap_token_env",
            lookup,
        );
    }
    match lookup(DEFAULT_BOOTSTRAP_TOKEN_ENV) {
        Ok(value) if value.is_empty() => Err(DnsError::config(format!(
            "{DEFAULT_BOOTSTRAP_TOKEN_ENV} is set but empty"
        ))),
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(DnsError::config(format!(
            "unable to read {DEFAULT_BOOTSTRAP_TOKEN_ENV}: {error}"
        ))),
    }
}

fn random_token(byte_count: usize) -> String {
    let mut bytes = vec![0u8; byte_count];
    rand::rng().fill(bytes.as_mut_slice());
    URL_SAFE_NO_PAD.encode(bytes)
}

fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

fn recovery_code_hash(code: &str) -> Vec<u8> {
    Sha256::digest(code.trim().to_ascii_uppercase().as_bytes()).to_vec()
}

fn generate_recovery_codes(count: usize) -> Vec<String> {
    (0..count)
        .map(|_| {
            let raw = random_token(9).to_ascii_uppercase();
            raw.as_bytes()
                .chunks(4)
                .map(|chunk| String::from_utf8_lossy(chunk))
                .collect::<Vec<_>>()
                .join("-")
        })
        .collect()
}

fn unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
}

fn peer_key(peer: Option<SocketAddr>) -> String {
    peer.map(|peer| peer.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn constant_time_equal(provided: &str, expected: &str) -> bool {
    provided.as_bytes().ct_eq(expected.as_bytes()).into()
}

fn session_cookie(headers: &HeaderMap) -> Option<String> {
    named_cookie(headers, SESSION_COOKIE)
}

fn named_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    for header in headers.get_all(COOKIE).iter() {
        let Ok(header) = header.to_str() else {
            continue;
        };
        for part in header.split(';') {
            let Ok(cookie) = Cookie::parse(part.trim().to_string()) else {
                continue;
            };
            if cookie.name() == name {
                return Some(cookie.value().to_string());
            }
        }
    }
    None
}

fn retain_unexpired_login_challenges(challenges: &mut HashMap<String, LoginChallenge>) {
    challenges.retain(|_, challenge| challenge.expires_at > Instant::now());
}

fn set_cookie_header(service: &AuthService, session: &CreatedSession) -> HeaderValue {
    let cookie = Cookie::build((SESSION_COOKIE, session.token.clone()))
        .path("/")
        .http_only(true)
        .secure(service.config.secure_cookie)
        .same_site(cookie_same_site(service.config.cookie_same_site))
        .max_age(cookie::time::Duration::seconds(
            service.config.session_ttl.as_secs().min(i64::MAX as u64) as i64,
        ))
        .build();
    HeaderValue::from_str(&cookie.to_string()).expect("generated session cookie must be valid")
}

fn clear_cookie_header(service: &AuthService) -> HeaderValue {
    let cookie = Cookie::build((SESSION_COOKIE, ""))
        .path("/")
        .http_only(true)
        .secure(service.config.secure_cookie)
        .same_site(cookie_same_site(service.config.cookie_same_site))
        .max_age(cookie::time::Duration::seconds(0))
        .build();
    HeaderValue::from_str(&cookie.to_string()).expect("generated clear cookie must be valid")
}

fn oidc_flow_cookie_name(state: &str) -> String {
    format!("{OIDC_FLOW_COOKIE_PREFIX}{state}")
}

fn oidc_flow_cookie_path(service: &AuthService) -> String {
    service
        .config
        .oidc
        .as_ref()
        .and_then(|config| url::Url::parse(&config.redirect_url).ok())
        .map(|url| url.path().to_string())
        .filter(|path| !path.is_empty())
        .unwrap_or_else(|| "/api/auth/oidc/callback".to_string())
}

fn oidc_flow_cookie_header(
    service: &AuthService,
    state: &str,
    token: &str,
    max_age: Duration,
) -> HeaderValue {
    let cookie = Cookie::build((oidc_flow_cookie_name(state), token.to_string()))
        .path(oidc_flow_cookie_path(service))
        .http_only(true)
        .secure(service.config.secure_cookie)
        // The identity provider returns through a top-level cross-site GET.
        .same_site(SameSite::Lax)
        .max_age(cookie::time::Duration::seconds(
            max_age.as_secs().min(i64::MAX as u64) as i64,
        ))
        .build();
    HeaderValue::from_str(&cookie.to_string()).expect("generated OIDC flow cookie must be valid")
}

fn cookie_same_site(value: ApiCookieSameSite) -> SameSite {
    match value {
        ApiCookieSameSite::Strict => SameSite::Strict,
        ApiCookieSameSite::Lax => SameSite::Lax,
        ApiCookieSameSite::None => SameSite::None,
    }
}

#[derive(Serialize)]
struct UserResponse<'a> {
    id: &'a str,
    username: &'a str,
}

#[derive(Serialize)]
struct SessionResponse<'a> {
    ok: bool,
    authenticated: bool,
    setup_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<UserResponse<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_method: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    csrf_token: Option<&'a str>,
    methods: AuthMethods,
}

#[derive(Serialize)]
struct TotpSetupResponse {
    ok: bool,
    secret: String,
    otpauth_uri: String,
    expires_in: u64,
}

#[derive(Serialize)]
struct SecurityResponse {
    ok: bool,
    totp_enabled: bool,
    passkeys: Vec<PasskeySummary>,
    oidc_identities: Vec<OidcIdentitySummary>,
}

#[derive(Clone, Serialize)]
struct PasskeySummary {
    id: String,
    name: String,
    created_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_used_at_ms: Option<i64>,
}

impl From<&PasskeyRecord> for PasskeySummary {
    fn from(record: &PasskeyRecord) -> Self {
        Self {
            id: record.id.clone(),
            name: record.name.clone(),
            created_at_ms: record.created_at.saturating_mul(1000),
            last_used_at_ms: record.last_used_at.map(|value| value.saturating_mul(1000)),
        }
    }
}

#[derive(Serialize)]
struct OidcIdentitySummary {
    issuer: String,
    subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
}

impl From<&OidcIdentityRecord> for OidcIdentitySummary {
    fn from(record: &OidcIdentityRecord) -> Self {
        Self {
            issuer: record.issuer.clone(),
            subject: record.subject.clone(),
            display_name: record.display_name.clone(),
        }
    }
}

fn authenticated_session_response<'a>(
    service: &AuthService,
    principal: &'a AuthPrincipal,
) -> SessionResponse<'a> {
    SessionResponse {
        ok: true,
        authenticated: true,
        setup_required: false,
        user: Some(UserResponse {
            id: &principal.user_id,
            username: &principal.username,
        }),
        auth_method: Some(&principal.auth_method),
        csrf_token: Some(&principal.csrf_token),
        methods: service.methods(),
    }
}

fn session_api_response(service: &AuthService, session: &CreatedSession) -> ApiResponse {
    let mut response = json_ok(
        StatusCode::OK,
        &authenticated_session_response(service, &session.principal),
    );
    response
        .headers_mut()
        .insert(SET_COOKIE, set_cookie_header(service, session));
    response
}

#[derive(Clone, Copy, Debug)]
enum AuthAction {
    Methods,
    Session,
    Bootstrap,
    Login,
    LoginTotp,
    Logout,
    Password,
    Security,
    TotpBegin,
    TotpConfirm,
    TotpDisable,
    PasskeyRegisterBegin,
    PasskeyRegisterFinish,
    PasskeyLoginBegin,
    PasskeyLoginFinish,
    PasskeyItem,
    OidcStart,
    OidcCallback,
}

#[derive(Debug)]
struct AuthHandler {
    service: Option<Arc<AuthService>>,
    action: AuthAction,
}

impl AuthHandler {
    fn new(service: Option<Arc<AuthService>>, action: AuthAction) -> Arc<Self> {
        Arc::new(Self { service, action })
    }

    fn service(&self) -> ApiResult<&AuthService> {
        self.service.as_deref().ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                "auth_disabled",
                "management authentication is disabled",
            )
        })
    }
}

#[async_trait]
impl ApiHandler for AuthHandler {
    async fn handle(&self, request: Request<Bytes>) -> ApiResponse {
        let response = match self.handle_inner(request).await {
            Ok(response) => response,
            Err(error) => error.response(),
        };
        with_auth_response_headers(response)
    }
}

impl AuthHandler {
    async fn handle_inner(&self, request: Request<Bytes>) -> ApiResult<ApiResponse> {
        match self.action {
            AuthAction::Methods => {
                let (enabled, setup_required, methods) = match &self.service {
                    Some(service) => (
                        true,
                        service.setup_required().await.map_err(ApiError::internal)?,
                        service.methods(),
                    ),
                    None => (
                        false,
                        false,
                        AuthMethods {
                            password: false,
                            totp: false,
                            oidc: false,
                            passkey: false,
                        },
                    ),
                };
                Ok(json_ok(
                    StatusCode::OK,
                    &serde_json::json!({
                        "ok": true,
                        "enabled": enabled,
                        "setup_required": setup_required,
                        "methods": methods,
                    }),
                ))
            }
            AuthAction::Session => match &self.service {
                Some(service) => {
                    let principal = request.extensions().get::<AuthPrincipal>();
                    let setup_required =
                        service.setup_required().await.map_err(ApiError::internal)?;
                    let mut response = json_ok(
                        StatusCode::OK,
                        &SessionResponse {
                            ok: true,
                            authenticated: principal.is_some(),
                            setup_required,
                            user: principal.map(|principal| UserResponse {
                                id: &principal.user_id,
                                username: &principal.username,
                            }),
                            auth_method: principal.map(|principal| principal.auth_method.as_str()),
                            csrf_token: principal.map(|principal| principal.csrf_token.as_str()),
                            methods: service.methods(),
                        },
                    );
                    if principal.is_none() && service.has_session_cookie(request.headers()) {
                        response
                            .headers_mut()
                            .insert(SET_COOKIE, clear_cookie_header(service));
                    }
                    Ok(response)
                }
                None => Ok(json_ok(
                    StatusCode::OK,
                    &serde_json::json!({
                        "ok": true,
                        "authenticated": true,
                        "setup_required": false,
                        "user": {"id": "auth-disabled", "username": "anonymous"},
                        "auth_method": "disabled",
                        "csrf_token": "",
                        "methods": {"password": false, "totp": false, "oidc": false, "passkey": false},
                    }),
                )),
            },
            AuthAction::Bootstrap => {
                let service = self.service()?;
                let body: BootstrapRequest = parse_json_body(&request)?;
                let direct_loopback_request = is_direct_loopback_bootstrap_request(&request);
                let session = service
                    .bootstrap(
                        body.username,
                        body.password,
                        body.token,
                        request_peer(&request),
                        direct_loopback_request,
                    )
                    .await?;
                Ok(session_api_response(service, &session))
            }
            AuthAction::Login => {
                let service = self.service()?;
                let body: LoginRequest = parse_json_body(&request)?;
                match service
                    .login(body.username, body.password, request_peer(&request))
                    .await?
                {
                    LoginOutcome::Authenticated(session) => {
                        Ok(session_api_response(service, &session))
                    }
                    LoginOutcome::TotpRequired { challenge_id } => Ok(json_ok(
                        StatusCode::ACCEPTED,
                        &serde_json::json!({
                            "ok": true,
                            "requires_totp": true,
                            "challenge_id": challenge_id,
                            "expires_in": LOGIN_CHALLENGE_TTL.as_secs(),
                        }),
                    )),
                }
            }
            AuthAction::LoginTotp => {
                let service = self.service()?;
                let body: TotpLoginRequest = parse_json_body(&request)?;
                let session = service
                    .finish_totp_login(body.challenge_id, body.code, request_peer(&request))
                    .await?;
                Ok(session_api_response(service, &session))
            }
            AuthAction::Logout => {
                let service = self.service()?;
                let principal = request_principal(&request)?;
                service.logout(principal).await?;
                let mut response = json_ok(StatusCode::OK, &serde_json::json!({"ok": true}));
                response
                    .headers_mut()
                    .insert(SET_COOKIE, clear_cookie_header(service));
                Ok(response)
            }
            AuthAction::Password => {
                let service = self.service()?;
                let principal = request_principal(&request)?;
                let body: PasswordRequest = parse_json_body(&request)?;
                service
                    .change_password(
                        principal,
                        body.current_password,
                        body.new_password,
                        request_peer(&request),
                    )
                    .await?;
                Ok(json_ok(StatusCode::OK, &serde_json::json!({"ok": true})))
            }
            AuthAction::Security => {
                let service = self.service()?;
                let summary = service
                    .security_summary(request_principal(&request)?)
                    .await?;
                Ok(json_ok(StatusCode::OK, &summary))
            }
            AuthAction::TotpBegin => {
                let service = self.service()?;
                let response = service.begin_totp(request_principal(&request)?).await?;
                Ok(json_ok(StatusCode::OK, &response))
            }
            AuthAction::TotpConfirm => {
                let service = self.service()?;
                let body: CodeRequest = parse_json_body(&request)?;
                let recovery_codes = service
                    .confirm_totp(request_principal(&request)?, body.code)
                    .await?;
                Ok(json_ok(
                    StatusCode::OK,
                    &serde_json::json!({"ok": true, "recovery_codes": recovery_codes}),
                ))
            }
            AuthAction::TotpDisable => {
                let service = self.service()?;
                let principal = request_principal(&request)?;
                let body: DisableTotpRequest = parse_json_body(&request)?;
                service
                    .disable_totp(principal, body.password, body.code, request_peer(&request))
                    .await?;
                Ok(json_ok(StatusCode::OK, &serde_json::json!({"ok": true})))
            }
            AuthAction::PasskeyRegisterBegin => {
                let service = self.service()?;
                let flow = service
                    .begin_passkey_registration(request_principal(&request)?)
                    .await?;
                Ok(json_ok(StatusCode::OK, &flow))
            }
            AuthAction::PasskeyRegisterFinish => {
                let service = self.service()?;
                let body: PasskeyRegistrationRequest = parse_json_body(&request)?;
                let passkey = service
                    .finish_passkey_registration(
                        request_principal(&request)?,
                        body.flow_id,
                        body.credential,
                        body.name,
                    )
                    .await?;
                Ok(json_ok(
                    StatusCode::OK,
                    &serde_json::json!({"ok": true, "passkey": passkey}),
                ))
            }
            AuthAction::PasskeyLoginBegin => {
                let service = self.service()?;
                let body: PasskeyLoginBeginRequest = parse_json_body(&request)?;
                let flow = service
                    .begin_passkey_login(body.username, request_peer(&request))
                    .await?;
                Ok(json_ok(StatusCode::OK, &flow))
            }
            AuthAction::PasskeyLoginFinish => {
                let service = self.service()?;
                let body: PasskeyLoginFinishRequest = parse_json_body(&request)?;
                let session = service
                    .finish_passkey_login(body.flow_id, body.credential, request_peer(&request))
                    .await?;
                Ok(session_api_response(service, &session))
            }
            AuthAction::PasskeyItem => {
                let service = self.service()?;
                let principal = request_principal(&request)?;
                let id = request
                    .uri()
                    .path()
                    .strip_prefix("/auth/passkeys/")
                    .filter(|id| !id.is_empty() && !id.contains('/'))
                    .ok_or_else(|| {
                        ApiError::new(StatusCode::BAD_REQUEST, "invalid_id", "invalid passkey id")
                    })?
                    .to_string();
                if request.method() == Method::PATCH {
                    let body: PasskeyRenameRequest = parse_json_body(&request)?;
                    let passkey = service.rename_passkey(principal, id, body.name).await?;
                    Ok(json_ok(
                        StatusCode::OK,
                        &serde_json::json!({"ok": true, "passkey": passkey}),
                    ))
                } else if request.method() == Method::DELETE {
                    service.delete_passkey(principal, id).await?;
                    Ok(json_ok(StatusCode::OK, &serde_json::json!({"ok": true})))
                } else {
                    Err(ApiError::new(
                        StatusCode::METHOD_NOT_ALLOWED,
                        "method_not_allowed",
                        "method not allowed",
                    ))
                }
            }
            AuthAction::OidcStart => {
                let service = self.service()?;
                let body: OidcStartRequest = parse_json_body(&request)?;
                let (url, state, browser_token) = service
                    .begin_oidc(body.return_to, request_peer(&request))
                    .await?;
                let mut response =
                    json_ok(StatusCode::OK, &serde_json::json!({"ok": true, "url": url}));
                response.headers_mut().insert(
                    SET_COOKIE,
                    oidc_flow_cookie_header(service, &state, &browser_token, OIDC_CHALLENGE_TTL),
                );
                Ok(response)
            }
            AuthAction::OidcCallback => {
                let service = self.service()?;
                let query = query_parameters(&request)?;
                if let Some(provider_error) = query.get("error") {
                    return Err(ApiError::new(
                        StatusCode::UNAUTHORIZED,
                        "oidc_provider_error",
                        format!("OIDC provider rejected login: {provider_error}"),
                    ));
                }
                let code = query.get("code").cloned().ok_or_else(|| {
                    ApiError::new(
                        StatusCode::BAD_REQUEST,
                        "invalid_oidc_callback",
                        "OIDC callback is missing code",
                    )
                })?;
                let state = query.get("state").cloned().ok_or_else(|| {
                    ApiError::new(
                        StatusCode::BAD_REQUEST,
                        "invalid_oidc_callback",
                        "OIDC callback is missing state",
                    )
                })?;
                let browser_token =
                    named_cookie(request.headers(), oidc_flow_cookie_name(&state).as_str())
                        .unwrap_or_default();
                let (session, return_to) = service
                    .finish_oidc(code, state.clone(), browser_token)
                    .await?;
                let mut response = simple_response(StatusCode::SEE_OTHER, Bytes::new());
                response.headers_mut().insert(
                    LOCATION,
                    HeaderValue::from_str(&return_to).map_err(ApiError::internal)?,
                );
                response
                    .headers_mut()
                    .insert(SET_COOKIE, set_cookie_header(service, &session));
                response.headers_mut().append(
                    SET_COOKIE,
                    oidc_flow_cookie_header(service, &state, "", Duration::ZERO),
                );
                Ok(response)
            }
        }
    }
}

pub(crate) fn register_builtin_routes(
    register: &ApiRegister,
    service: Option<Arc<AuthService>>,
) -> Result<()> {
    let routes = [
        (Method::GET, "/auth/methods", AuthAction::Methods),
        (Method::GET, "/auth/session", AuthAction::Session),
        (Method::POST, "/auth/bootstrap", AuthAction::Bootstrap),
        (Method::POST, "/auth/login", AuthAction::Login),
        (Method::POST, "/auth/login/totp", AuthAction::LoginTotp),
        (Method::POST, "/auth/logout", AuthAction::Logout),
        (Method::PUT, "/auth/password", AuthAction::Password),
        (Method::GET, "/auth/security", AuthAction::Security),
        (Method::POST, "/auth/totp/begin", AuthAction::TotpBegin),
        (Method::POST, "/auth/totp/confirm", AuthAction::TotpConfirm),
        (Method::DELETE, "/auth/totp", AuthAction::TotpDisable),
        (
            Method::POST,
            "/auth/passkeys/register/begin",
            AuthAction::PasskeyRegisterBegin,
        ),
        (
            Method::POST,
            "/auth/passkeys/register/finish",
            AuthAction::PasskeyRegisterFinish,
        ),
        (
            Method::POST,
            "/auth/passkeys/login/begin",
            AuthAction::PasskeyLoginBegin,
        ),
        (
            Method::POST,
            "/auth/passkeys/login/finish",
            AuthAction::PasskeyLoginFinish,
        ),
        (Method::POST, "/auth/oidc/start", AuthAction::OidcStart),
        (Method::GET, "/auth/oidc/callback", AuthAction::OidcCallback),
    ];
    for (method, path, action) in routes {
        register.register_route(method, path, AuthHandler::new(service.clone(), action))?;
    }
    register.register_prefix_route(
        Method::PATCH,
        "/auth/passkeys/",
        AuthHandler::new(service.clone(), AuthAction::PasskeyItem),
    )?;
    register.register_prefix_route(
        Method::DELETE,
        "/auth/passkeys/",
        AuthHandler::new(service, AuthAction::PasskeyItem),
    )?;
    Ok(())
}

pub(crate) fn is_public_route(method: &Method, path: &str) -> bool {
    matches!(
        (method.as_str(), path),
        ("GET", "/auth/methods")
            | ("GET", "/auth/session")
            | ("POST", "/auth/bootstrap")
            | ("POST", "/auth/login")
            | ("POST", "/auth/login/totp")
            | ("POST", "/auth/passkeys/login/begin")
            | ("POST", "/auth/passkeys/login/finish")
            | ("POST", "/auth/oidc/start")
            | ("GET", "/auth/oidc/callback")
    )
}

pub(crate) fn is_public_json_route(method: &Method, path: &str) -> bool {
    method == Method::POST
        && matches!(
            path,
            "/auth/bootstrap"
                | "/auth/login"
                | "/auth/login/totp"
                | "/auth/passkeys/login/begin"
                | "/auth/passkeys/login/finish"
                | "/auth/oidc/start"
        )
}

fn with_auth_response_headers(mut response: ApiResponse) -> ApiResponse {
    response.headers_mut().insert(
        http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    response
        .headers_mut()
        .insert(http::header::PRAGMA, HeaderValue::from_static("no-cache"));
    response.headers_mut().insert(
        http::header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

fn request_peer(request: &Request<Bytes>) -> Option<SocketAddr> {
    request.extensions().get::<PeerAddr>().map(|peer| peer.0)
}

fn is_direct_loopback_bootstrap_request(request: &Request<Bytes>) -> bool {
    if !request_peer(request).is_some_and(|peer| peer.ip().is_loopback()) {
        return false;
    }
    if [
        "forwarded",
        "x-forwarded-for",
        "x-forwarded-host",
        "x-forwarded-proto",
        "x-real-ip",
    ]
    .iter()
    .any(|name| request.headers().contains_key(*name))
    {
        return false;
    }
    if request
        .headers()
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| !matches!(value, "same-origin" | "none"))
    {
        return false;
    }

    let host_headers = request.headers().get_all(HOST);
    let mut hosts = host_headers.iter();
    let Some(host) = hosts.next().and_then(|value| value.to_str().ok()) else {
        return false;
    };
    let Ok(authority) = host.parse::<http::uri::Authority>() else {
        return false;
    };
    if hosts.next().is_some() || !is_loopback_host(authority.host()) {
        return false;
    }

    let origin_headers = request.headers().get_all(ORIGIN);
    let mut origins = origin_headers.iter();
    let Some(origin) = origins.next() else {
        return true;
    };
    if origins.next().is_some() {
        return false;
    }
    origin
        .to_str()
        .ok()
        .and_then(|value| url::Url::parse(value).ok())
        .is_some_and(|origin| {
            matches!(origin.scheme(), "http" | "https")
                && origin.host_str().is_some_and(is_loopback_host)
                && origin.host_str().is_some_and(|host| {
                    host.trim_matches(['[', ']'])
                        .eq_ignore_ascii_case(authority.host().trim_matches(['[', ']']))
                })
                && origin.port() == authority.port_u16()
        })
}

fn is_loopback_host(host: &str) -> bool {
    let host = host.trim_matches(['[', ']']).trim_end_matches('.');
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

fn request_principal(request: &Request<Bytes>) -> ApiResult<&AuthPrincipal> {
    request.extensions().get::<AuthPrincipal>().ok_or_else(|| {
        ApiError::new(
            StatusCode::UNAUTHORIZED,
            "authentication_required",
            "authentication required",
        )
    })
}

fn parse_json_body<T>(request: &Request<Bytes>) -> ApiResult<T>
where
    T: for<'de> Deserialize<'de>,
{
    if request.body().len() > MAX_JSON_BODY {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "payload_too_large",
            "authentication request body is too large",
        ));
    }
    serde_json::from_slice(request.body()).map_err(|error| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_json",
            format!("invalid JSON request: {error}"),
        )
    })
}

fn query_parameters(request: &Request<Bytes>) -> ApiResult<HashMap<String, String>> {
    let Some(query) = request.uri().query() else {
        return Ok(HashMap::new());
    };
    if query.len() > MAX_AUTH_QUERY {
        return Err(ApiError::new(
            StatusCode::URI_TOO_LONG,
            "query_too_large",
            "authentication query string is too large",
        ));
    }
    let mut values = HashMap::new();
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        if values
            .insert(key.into_owned(), value.into_owned())
            .is_some()
        {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "invalid_query",
                "authentication query parameters must not be repeated",
            ));
        }
    }
    Ok(values)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BootstrapRequest {
    username: String,
    password: String,
    token: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoginRequest {
    username: String,
    password: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OidcStartRequest {
    return_to: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TotpLoginRequest {
    challenge_id: String,
    code: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PasswordRequest {
    current_password: String,
    new_password: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CodeRequest {
    code: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DisableTotpRequest {
    password: String,
    code: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PasskeyRegistrationRequest {
    flow_id: String,
    credential: serde_json::Value,
    name: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PasskeyLoginBeginRequest {
    username: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PasskeyLoginFinishRequest {
    flow_id: String,
    credential: serde_json::Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PasskeyRenameRequest {
    name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(bootstrap_token: Option<&str>) -> ApiAuthConfig {
        ApiAuthConfig::Accounts {
            database: ":memory:".to_string(),
            bootstrap_token: bootstrap_token.map(str::to_string),
            bootstrap_token_env: None,
            session_ttl_seconds: 3600,
            cookie_secure: Some(false),
            cookie_same_site: ApiCookieSameSite::Lax,
            public_url: None,
            oidc: None,
            passkey: None,
        }
    }

    #[test]
    fn argon2id_password_hashes_are_salted_and_verifiable() {
        let first = hash_password("correct horse battery staple").expect("first hash");
        let second = hash_password("correct horse battery staple").expect("second hash");
        assert_ne!(first, second);
        assert!(first.starts_with("$argon2id$"));
        assert!(verify_password(&first, "correct horse battery staple").unwrap());
        assert!(!verify_password(&first, "incorrect password").unwrap());
    }

    #[test]
    fn totp_matching_returns_the_exact_counter_within_the_skew_window() {
        let secret = "JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP";
        let totp = build_totp(secret, "admin").expect("build TOTP");
        let unix_time = 1_700_000_000i64;
        let previous_counter = (unix_time as u64 / totp.step).saturating_sub(1);
        let token = totp.generate(previous_counter * totp.step);

        assert_eq!(
            matched_totp_counter(&totp, &token, unix_time),
            Some(previous_counter as i64)
        );
        assert_eq!(matched_totp_counter(&totp, "not-a-token", unix_time), None);
    }

    #[test]
    fn authentication_rate_limit_windows_do_not_clean_other_keys() {
        let mut limiter = AttemptLimiter::default();
        limiter.failures.insert(
            "long-window".to_string(),
            VecDeque::from([Instant::now() - Duration::from_secs(400)]),
        );

        limiter
            .ensure_allowed("short-window", 1, Duration::from_secs(300))
            .expect("unrelated key is allowed");
        let error = limiter
            .ensure_allowed("long-window", 1, Duration::from_secs(600))
            .expect_err("the long-window failure must still be retained");
        assert_eq!(error.status, StatusCode::TOO_MANY_REQUESTS);
    }

    #[test]
    fn conventional_bootstrap_environment_is_used_only_without_explicit_config() {
        let fallback = resolve_bootstrap_secret_with(None, None, |name| {
            assert_eq!(name, DEFAULT_BOOTSTRAP_TOKEN_ENV);
            Ok("docker-bootstrap-secret".to_string())
        })
        .expect("fallback secret");
        assert_eq!(fallback.as_deref(), Some("docker-bootstrap-secret"));

        let missing =
            resolve_bootstrap_secret_with(None, None, |_| Err(std::env::VarError::NotPresent))
                .expect("missing fallback is allowed for loopback bootstrap");
        assert!(missing.is_none());

        let explicit =
            resolve_bootstrap_secret_with(Some("configured-secret".to_string()), None, |_| {
                panic!("inline configuration must not read the environment")
            })
            .expect("inline secret");
        assert_eq!(explicit.as_deref(), Some("configured-secret"));

        let named = resolve_bootstrap_secret_with(None, Some("CUSTOM_BOOTSTRAP_TOKEN"), |name| {
            assert_eq!(name, "CUSTOM_BOOTSTRAP_TOKEN");
            Ok("custom-secret".to_string())
        })
        .expect("explicit environment secret");
        assert_eq!(named.as_deref(), Some("custom-secret"));
    }

    #[tokio::test]
    async fn loopback_bootstrap_creates_an_opaque_cookie_session() {
        let service = AuthService::new(&test_config(None)).expect("auth service");
        service.initialize().await.expect("initialize auth");
        let created = service
            .bootstrap(
                "admin".to_string(),
                "correct horse battery staple".to_string(),
                None,
                Some("127.0.0.1:12345".parse().unwrap()),
                true,
            )
            .await
            .expect("bootstrap");
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_str(&format!("{SESSION_COOKIE}={}", created.token)).unwrap(),
        );
        let principal = service
            .authenticate(&headers)
            .await
            .expect("session lookup")
            .expect("authenticated principal");
        assert_eq!(principal.username, "admin");
        headers.insert(
            "x-csrf-token",
            HeaderValue::from_str(&principal.csrf_token).unwrap(),
        );
        assert!(service.verify_csrf(&headers, &principal));
    }

    #[tokio::test]
    async fn remote_bootstrap_requires_the_configured_one_time_token() {
        let service =
            AuthService::new(&test_config(Some("expected-secret"))).expect("auth service");
        service.initialize().await.expect("initialize auth");
        let result = service
            .bootstrap(
                "admin".to_string(),
                "correct horse battery staple".to_string(),
                Some("wrong-secret".to_string()),
                Some("192.0.2.10:12345".parse().unwrap()),
                false,
            )
            .await;
        let error = match result {
            Ok(_) => panic!("remote bootstrap must reject the wrong token"),
            Err(error) => error,
        };
        assert_eq!(error.status, StatusCode::FORBIDDEN);
        assert!(service.setup_required().await.unwrap());
    }

    #[test]
    fn configured_public_url_is_an_exact_trusted_origin() {
        let mut config = test_config(None);
        let ApiAuthConfig::Accounts { public_url, .. } = &mut config else {
            unreachable!("test config must use accounts auth");
        };
        *public_url = Some("https://dns.example.test/console".to_string());
        let service = AuthService::new(&config).expect("auth service");

        assert!(service.allows_public_origin("https://dns.example.test"));
        assert!(!service.allows_public_origin("http://dns.example.test"));
        assert!(!service.allows_public_origin("https://dns.example.test:444"));
    }

    #[test]
    fn loopback_bootstrap_rejects_public_or_proxied_request_metadata() {
        fn request(host: &str, origin: Option<&str>) -> Request<Bytes> {
            let mut request = Request::builder()
                .uri("/auth/bootstrap")
                .header(HOST, host)
                .body(Bytes::new())
                .expect("request");
            if let Some(origin) = origin {
                request.headers_mut().insert(
                    ORIGIN,
                    HeaderValue::from_str(origin).expect("origin header"),
                );
            }
            request
                .extensions_mut()
                .insert(PeerAddr("127.0.0.1:12345".parse().expect("loopback peer")));
            request
        }

        assert!(is_direct_loopback_bootstrap_request(&request(
            "127.0.0.1:9199",
            None,
        )));
        assert!(is_direct_loopback_bootstrap_request(&request(
            "localhost:9199",
            Some("http://localhost:9199"),
        )));
        assert!(!is_direct_loopback_bootstrap_request(&request(
            "dns.example.test",
            None,
        )));
        assert!(!is_direct_loopback_bootstrap_request(&request(
            "127.0.0.1:9199",
            Some("https://dns.example.test"),
        )));
        assert!(!is_direct_loopback_bootstrap_request(&request(
            "localhost:9199",
            Some("http://localhost:3000"),
        )));

        let mut cross_site = request("localhost:9199", Some("http://localhost:9199"));
        cross_site
            .headers_mut()
            .insert("sec-fetch-site", HeaderValue::from_static("cross-site"));
        assert!(!is_direct_loopback_bootstrap_request(&cross_site));

        let mut proxied = request("127.0.0.1:9199", None);
        proxied
            .headers_mut()
            .insert("x-forwarded-for", HeaderValue::from_static("198.51.100.25"));
        assert!(!is_direct_loopback_bootstrap_request(&proxied));
    }
}
