// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Optional short-lived Redis cache for query-recorder HTTP API responses.

use std::sync::Arc;
#[cfg(feature = "storage-redis")]
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
#[cfg(feature = "storage-redis")]
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
#[cfg(any(feature = "storage-redis", test))]
use sha2::{Digest, Sha256};
#[cfg(feature = "storage-redis")]
use tracing::debug;

#[cfg(feature = "storage-redis")]
use crate::infra::clock::AppClock;
use crate::infra::error::{DnsError, Result};

#[cfg(any(feature = "storage-redis", test))]
const CACHE_KEY_SCHEMA_VERSION: &str = "v1";
#[cfg(feature = "storage-redis")]
const CACHE_PAYLOAD_VERSION: u8 = 1;
#[cfg(feature = "storage-redis")]
const DEFAULT_RECORDS_TTL_MS: u64 = 2_000;
#[cfg(feature = "storage-redis")]
const DEFAULT_STATS_TTL_MS: u64 = 5_000;
#[cfg(feature = "storage-redis")]
const DEFAULT_COMMAND_TIMEOUT_MS: u64 = 100;
#[cfg(feature = "storage-redis")]
const DEFAULT_MAX_VALUE_BYTES: usize = 1_048_576;
#[cfg(feature = "storage-redis")]
const CIRCUIT_FAILURE_THRESHOLD: usize = 3;
#[cfg(feature = "storage-redis")]
const CIRCUIT_RETRY_AFTER_MS: u64 = 5_000;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct QueryRecorderApiCacheConfig {
    #[serde(default)]
    pub(super) enabled: bool,
    pub(super) records_ttl_ms: Option<u64>,
    pub(super) stats_ttl_ms: Option<u64>,
    pub(super) command_timeout_ms: Option<u64>,
    pub(super) max_value_bytes: Option<usize>,
}

#[derive(Debug, Clone)]
pub(super) struct ResolvedApiCacheConfig {
    #[cfg(feature = "storage-redis")]
    records_ttl_ms: u64,
    #[cfg(feature = "storage-redis")]
    stats_ttl_ms: u64,
    #[cfg(feature = "storage-redis")]
    command_timeout: Duration,
    #[cfg(feature = "storage-redis")]
    max_value_bytes: usize,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum CacheLifetime {
    Records,
    Stats,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CacheGeneration(u64);

#[derive(Debug)]
pub(super) enum CacheLookup<T> {
    Hit(T),
    Miss(CacheGeneration),
    Bypass,
}

impl ResolvedApiCacheConfig {
    #[cfg(feature = "storage-redis")]
    fn ttl_ms(&self, lifetime: CacheLifetime) -> u64 {
        match lifetime {
            CacheLifetime::Records => self.records_ttl_ms,
            CacheLifetime::Stats => self.stats_ttl_ms,
        }
    }
}

pub(super) fn resolve_config(
    config: Option<QueryRecorderApiCacheConfig>,
) -> Result<Option<ResolvedApiCacheConfig>> {
    let Some(config) = config else {
        return Ok(None);
    };
    if !config.enabled {
        return Ok(None);
    }

    #[cfg(not(feature = "storage-redis"))]
    return Err(DnsError::plugin(
        "query_recorder api_cache.enabled requires the 'storage-redis' feature",
    ));

    #[cfg(feature = "storage-redis")]
    {
        let records_ttl_ms = checked_nonzero(
            "records_ttl_ms",
            config.records_ttl_ms.unwrap_or(DEFAULT_RECORDS_TTL_MS),
        )?;
        let stats_ttl_ms = checked_nonzero(
            "stats_ttl_ms",
            config.stats_ttl_ms.unwrap_or(DEFAULT_STATS_TTL_MS),
        )?;
        let command_timeout_ms = checked_nonzero(
            "command_timeout_ms",
            config
                .command_timeout_ms
                .unwrap_or(DEFAULT_COMMAND_TIMEOUT_MS),
        )?;
        let max_value_bytes = config.max_value_bytes.unwrap_or(DEFAULT_MAX_VALUE_BYTES);
        if max_value_bytes == 0 {
            return Err(DnsError::plugin(
                "query_recorder api_cache.max_value_bytes must be greater than 0",
            ));
        }

        Ok(Some(ResolvedApiCacheConfig {
            records_ttl_ms,
            stats_ttl_ms,
            command_timeout: Duration::from_millis(command_timeout_ms),
            max_value_bytes,
        }))
    }
}

#[cfg(feature = "storage-redis")]
fn checked_nonzero(field: &str, value: u64) -> Result<u64> {
    if value == 0 {
        return Err(DnsError::plugin(format!(
            "query_recorder api_cache.{field} must be greater than 0"
        )));
    }
    Ok(value)
}

#[cfg(feature = "storage-redis")]
#[derive(Debug, Serialize, Deserialize)]
struct CacheEnvelope<T> {
    version: u8,
    value: T,
}

#[cfg(feature = "storage-redis")]
#[derive(Debug)]
struct CircuitBreaker {
    consecutive_failures: AtomicUsize,
    open_until_ms: AtomicU64,
}

#[cfg(feature = "storage-redis")]
impl CircuitBreaker {
    fn new() -> Self {
        Self {
            consecutive_failures: AtomicUsize::new(0),
            open_until_ms: AtomicU64::new(0),
        }
    }

    fn allow_request(&self, now_ms: u64) -> bool {
        loop {
            let state = self.open_until_ms.load(Ordering::Acquire);
            if state == 0 {
                return true;
            }
            if state > now_ms {
                return false;
            }
            // Claim a bounded half-open probe lease. If the permitted future
            // is cancelled or dropped before recording an outcome, another
            // request may probe again after this lease expires.
            let probe_lease_until = now_ms.saturating_add(CIRCUIT_RETRY_AFTER_MS);
            if self
                .open_until_ms
                .compare_exchange(
                    state,
                    probe_lease_until,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                return true;
            }
        }
    }

    fn record_success(&self) {
        self.consecutive_failures.store(0, Ordering::Release);
        self.open_until_ms.store(0, Ordering::Release);
    }

    fn record_failure(&self, now_ms: u64) {
        let failures = self
            .consecutive_failures
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1);
        if failures >= CIRCUIT_FAILURE_THRESHOLD {
            self.open_until_ms.store(
                now_ms.saturating_add(CIRCUIT_RETRY_AFTER_MS),
                Ordering::Release,
            );
        }
    }
}

#[derive(Debug)]
pub(super) struct ApiCache {
    #[cfg(feature = "storage-redis")]
    service: Arc<crate::infra::cache::redis::RedisService>,
    #[cfg(feature = "storage-redis")]
    base_key: String,
    #[cfg(feature = "storage-redis")]
    generation_key: String,
    #[cfg(feature = "storage-redis")]
    generation: AtomicU64,
    #[cfg(feature = "storage-redis")]
    circuit: CircuitBreaker,
    #[cfg(feature = "storage-redis")]
    config: ResolvedApiCacheConfig,
}

impl ApiCache {
    pub(super) fn new(
        tag: &str,
        database_identity_digest: &str,
        config: ResolvedApiCacheConfig,
    ) -> Result<Arc<Self>> {
        #[cfg(not(feature = "storage-redis"))]
        {
            let _ = (tag, database_identity_digest, config);
            return Err(DnsError::plugin(
                "query_recorder api_cache.enabled requires the 'storage-redis' feature",
            ));
        }

        #[cfg(feature = "storage-redis")]
        {
            let service = crate::infra::cache::redis::global().ok_or_else(|| {
                DnsError::plugin(
                    "query_recorder api_cache.enabled requires a configured storage.redis connection",
                )
            })?;
            let tag_digest = sha256_hex(tag.as_bytes());
            let base_key = service.key(&format!(
                "query-recorder-api:{CACHE_KEY_SCHEMA_VERSION}:{tag_digest}:{database_identity_digest}"
            ));
            Ok(Arc::new(Self {
                service,
                generation_key: format!("{base_key}:generation"),
                base_key,
                generation: AtomicU64::new(0),
                circuit: CircuitBreaker::new(),
                config,
            }))
        }
    }

    pub(super) async fn lookup<T>(&self, namespace: &str, identity: &str) -> CacheLookup<T>
    where
        T: DeserializeOwned + Send,
    {
        #[cfg(not(feature = "storage-redis"))]
        {
            let _ = (namespace, identity);
            CacheLookup::Bypass
        }

        #[cfg(feature = "storage-redis")]
        {
            if !self.circuit.allow_request(AppClock::elapsed_millis()) {
                return CacheLookup::Bypass;
            }
            let operation = async {
                let generation = self.refresh_generation().await?;
                let key = self.entry_key(generation, namespace, identity);
                let value = self.service.get_with_ttl_ms(&key).await?;
                Ok::<_, ::redis::RedisError>((value, generation))
            };
            let (encoded, generation) = match tokio::time::timeout(
                self.config.command_timeout,
                operation,
            )
            .await
            {
                Ok(Ok((Some((encoded, _remaining_ttl_ms)), generation))) => {
                    self.circuit.record_success();
                    (encoded, CacheGeneration(generation))
                }
                Ok(Ok((None, generation))) => {
                    self.circuit.record_success();
                    return CacheLookup::Miss(CacheGeneration(generation));
                }
                Ok(Err(err)) => {
                    self.circuit.record_failure(AppClock::elapsed_millis());
                    debug!(error = %err, "query_recorder Redis API cache lookup failed; using database");
                    return CacheLookup::Bypass;
                }
                Err(_) => {
                    self.circuit.record_failure(AppClock::elapsed_millis());
                    debug!("query_recorder Redis API cache lookup timed out; using database");
                    return CacheLookup::Bypass;
                }
            };
            if encoded.len() > self.config.max_value_bytes {
                debug!(
                    encoded_bytes = encoded.len(),
                    max_value_bytes = self.config.max_value_bytes,
                    "query_recorder Redis API cache value is too large; using database"
                );
                return CacheLookup::Bypass;
            }
            let result = match serde_json::from_slice::<CacheEnvelope<T>>(&encoded) {
                Ok(envelope) if envelope.version == CACHE_PAYLOAD_VERSION => {
                    CacheLookup::Hit(envelope.value)
                }
                Ok(_) => {
                    debug!(
                        "query_recorder Redis API cache payload version mismatch; using database"
                    );
                    CacheLookup::Miss(generation)
                }
                Err(err) => {
                    debug!(error = %err, "query_recorder Redis API cache payload is invalid; using database");
                    CacheLookup::Miss(generation)
                }
            };
            if self.generation.load(Ordering::Acquire) != generation.0 {
                CacheLookup::Miss(CacheGeneration(self.generation.load(Ordering::Acquire)))
            } else {
                result
            }
        }
    }

    pub(super) async fn store<T>(
        &self,
        namespace: &str,
        identity: &str,
        lifetime: CacheLifetime,
        generation: CacheGeneration,
        value: &T,
    ) where
        T: Serialize + Sync,
    {
        #[cfg(not(feature = "storage-redis"))]
        {
            let _ = (namespace, identity, lifetime, generation, value);
        }

        #[cfg(feature = "storage-redis")]
        {
            if self.generation.load(Ordering::Acquire) != generation.0 {
                return;
            }
            if !self.circuit.allow_request(AppClock::elapsed_millis()) {
                return;
            }
            let encoded = match serde_json::to_vec(&CacheEnvelope {
                version: CACHE_PAYLOAD_VERSION,
                value,
            }) {
                Ok(encoded) if encoded.len() <= self.config.max_value_bytes => encoded,
                Ok(encoded) => {
                    debug!(
                        encoded_bytes = encoded.len(),
                        max_value_bytes = self.config.max_value_bytes,
                        "query_recorder Redis API cache response is too large; skipping cache write"
                    );
                    return;
                }
                Err(err) => {
                    debug!(error = %err, "query_recorder Redis API cache serialization failed");
                    return;
                }
            };
            let operation = async {
                let current_generation = self.refresh_generation().await?;
                if current_generation != generation.0 {
                    return Ok(());
                }
                let key = self.entry_key(generation.0, namespace, identity);
                self.service
                    .set_px(&key, &encoded, self.config.ttl_ms(lifetime))
                    .await
            };
            match tokio::time::timeout(self.config.command_timeout, operation).await {
                Ok(Ok(())) => self.circuit.record_success(),
                Ok(Err(err)) => {
                    self.circuit.record_failure(AppClock::elapsed_millis());
                    debug!(error = %err, "query_recorder Redis API cache write failed");
                }
                Err(_) => {
                    self.circuit.record_failure(AppClock::elapsed_millis());
                    debug!("query_recorder Redis API cache write timed out");
                }
            }
        }
    }

    /// Hides all entries written under the previous generation immediately in
    /// this process, then best-effort propagates the generation bump to Redis.
    pub(super) async fn invalidate(&self) {
        #[cfg(feature = "storage-redis")]
        let minimum_generation = self
            .generation
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1);

        #[cfg(feature = "storage-redis")]
        {
            match tokio::time::timeout(
                self.config.command_timeout,
                self.service
                    .advance_generation(&self.generation_key, minimum_generation),
            )
            .await
            {
                Ok(Ok(generation)) => {
                    self.generation.fetch_max(generation, Ordering::AcqRel);
                    self.circuit.record_success();
                }
                Ok(Err(err)) => {
                    self.circuit.record_failure(AppClock::elapsed_millis());
                    debug!(error = %err, "query_recorder Redis API cache invalidation failed");
                }
                Err(_) => {
                    self.circuit.record_failure(AppClock::elapsed_millis());
                    debug!("query_recorder Redis API cache invalidation timed out");
                }
            }
        }
    }

    #[cfg(feature = "storage-redis")]
    async fn refresh_generation(&self) -> ::redis::RedisResult<u64> {
        let remote = self
            .service
            .get_u64(&self.generation_key)
            .await?
            .unwrap_or(0);
        let generation = self
            .generation
            .fetch_max(remote, Ordering::AcqRel)
            .max(remote);
        Ok(generation)
    }

    #[cfg(feature = "storage-redis")]
    fn entry_key(&self, generation: u64, namespace: &str, identity: &str) -> String {
        entry_key(&self.base_key, generation, namespace, identity)
    }
}

#[cfg(any(feature = "storage-redis", test))]
fn entry_key(base_key: &str, generation: u64, namespace: &str, identity: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(CACHE_KEY_SCHEMA_VERSION.as_bytes());
    hasher.update([0]);
    hasher.update(namespace.as_bytes());
    hasher.update([0]);
    hasher.update(identity.as_bytes());
    let request_digest = hex::encode(hasher.finalize());
    format!("{base_key}:{generation}:{request_digest}")
}

#[cfg(any(feature = "storage-redis", test))]
fn sha256_hex(value: &[u8]) -> String {
    hex::encode(Sha256::digest(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_hides_tag_and_query_identity() {
        let base_key = format!(
            "test:query-recorder-api:{CACHE_KEY_SCHEMA_VERSION}:{}:{}",
            sha256_hex(b"private-recorder-tag"),
            sha256_hex(b"postgres://user:password@database/private")
        );
        let key = entry_key(&base_key, 7, "records", "search=private.example");
        assert!(!key.contains("private-recorder-tag"));
        assert!(!key.contains("password"));
        assert!(!key.contains("private.example"));
        assert_eq!(key.rsplit(':').next().map(str::len), Some(64));
    }

    #[test]
    fn disabled_cache_does_not_require_redis_feature() {
        let config = QueryRecorderApiCacheConfig {
            enabled: false,
            records_ttl_ms: None,
            stats_ttl_ms: None,
            command_timeout_ms: None,
            max_value_bytes: None,
        };
        assert!(resolve_config(Some(config)).expect("resolve").is_none());
    }

    #[cfg(feature = "storage-redis")]
    #[test]
    fn enabled_cache_uses_documented_defaults() {
        let config = QueryRecorderApiCacheConfig {
            enabled: true,
            records_ttl_ms: None,
            stats_ttl_ms: None,
            command_timeout_ms: None,
            max_value_bytes: None,
        };
        let resolved = resolve_config(Some(config))
            .expect("resolve")
            .expect("enabled cache");
        assert_eq!(resolved.records_ttl_ms, 2_000);
        assert_eq!(resolved.stats_ttl_ms, 5_000);
        assert_eq!(resolved.command_timeout, Duration::from_millis(100));
        assert_eq!(resolved.max_value_bytes, 1_048_576);
    }

    #[cfg(feature = "storage-redis")]
    #[test]
    fn circuit_breaker_opens_and_allows_one_recovery_probe() {
        let circuit = CircuitBreaker::new();
        let now_ms = 100;
        for _ in 0..CIRCUIT_FAILURE_THRESHOLD {
            assert!(circuit.allow_request(now_ms));
            circuit.record_failure(now_ms);
        }
        assert!(!circuit.allow_request(now_ms));
        assert!(circuit.allow_request(now_ms + CIRCUIT_RETRY_AFTER_MS));
        assert!(!circuit.allow_request(now_ms + CIRCUIT_RETRY_AFTER_MS));
        assert!(circuit.allow_request(now_ms + (2 * CIRCUIT_RETRY_AFTER_MS)));
        circuit.record_success();
        assert!(circuit.allow_request(now_ms + (2 * CIRCUIT_RETRY_AFTER_MS)));
    }

    #[cfg(feature = "storage-redis")]
    #[tokio::test]
    async fn test_redis_query_recorder_api_cache_integration() {
        use crate::config::types::RedisStorageConfig;
        use crate::infra::clock::AppClock;

        let Ok(url) = std::env::var("OXIDNS_NEXT_TEST_REDIS_URL") else {
            eprintln!(
                "skipping Redis integration test: OXIDNS_NEXT_TEST_REDIS_URL is not configured"
            );
            return;
        };

        AppClock::start();
        let storage = RedisStorageConfig {
            url,
            key_prefix: format!("integration:query-recorder-api:{}", AppClock::instance_id()),
            connect_timeout_ms: 2_000,
        };
        crate::infra::cache::redis::install_global(Some(&storage))
            .expect("shared Redis service should initialize");
        let service =
            crate::infra::cache::redis::global().expect("shared Redis service should be installed");
        let resolved = resolve_config(Some(QueryRecorderApiCacheConfig {
            enabled: true,
            records_ttl_ms: Some(5_000),
            stats_ttl_ms: Some(5_000),
            command_timeout_ms: Some(2_000),
            max_value_bytes: Some(64 * 1024),
        }))
        .expect("cache config should resolve")
        .expect("cache should be enabled");
        let database_a = sha256_hex(b"postgres://database-a");
        let database_b = sha256_hex(b"postgres://database-b");
        let cache = ApiCache::new("query-log", &database_a, resolved.clone())
            .expect("API cache should initialize");
        let identity = "limit=100&search=private.example";

        let first_generation = match cache.lookup::<Vec<String>>("records", identity).await {
            CacheLookup::Miss(generation) => generation,
            other => panic!("first lookup should miss, got {other:?}"),
        };
        let expected = vec!["cached-response".to_string()];
        cache
            .store(
                "records",
                identity,
                CacheLifetime::Records,
                first_generation,
                &expected,
            )
            .await;
        match cache.lookup::<Vec<String>>("records", identity).await {
            CacheLookup::Hit(value) => assert_eq!(value, expected),
            other => panic!("stored response should hit, got {other:?}"),
        }

        cache.invalidate().await;
        let current_generation = match cache.lookup::<Vec<String>>("records", identity).await {
            CacheLookup::Miss(generation) => generation,
            other => panic!("invalidation should hide old response, got {other:?}"),
        };
        assert_ne!(first_generation, current_generation);

        // A SQL query that began before clear must not repopulate the current
        // generation after clear completes.
        cache
            .store(
                "records",
                identity,
                CacheLifetime::Records,
                first_generation,
                &vec!["stale-response".to_string()],
            )
            .await;
        assert!(matches!(
            cache.lookup::<Vec<String>>("records", identity).await,
            CacheLookup::Miss(generation) if generation == current_generation
        ));

        cache
            .store(
                "records",
                identity,
                CacheLifetime::Records,
                current_generation,
                &expected,
            )
            .await;
        assert!(matches!(
            cache.lookup::<Vec<String>>("records", identity).await,
            CacheLookup::Hit(value) if value == expected
        ));

        // The same recorder tag backed by a different SQL database must not
        // be able to observe cached query-log data.
        let isolated = ApiCache::new("query-log", &database_b, resolved)
            .expect("isolated API cache should initialize");
        assert!(matches!(
            isolated.lookup::<Vec<String>>("records", identity).await,
            CacheLookup::Miss(_)
        ));

        for key in [
            cache.entry_key(first_generation.0, "records", identity),
            cache.entry_key(current_generation.0, "records", identity),
            cache.generation_key.clone(),
            isolated.generation_key.clone(),
        ] {
            service
                .unlink(&key)
                .await
                .expect("integration test key cleanup should succeed");
        }

        // A broken Redis endpoint is fail-open, and repeated failures open the
        // bounded circuit instead of charging every SQL request a timeout.
        let unavailable = RedisStorageConfig {
            url: "redis://127.0.0.1:1/15".to_string(),
            key_prefix: format!("integration:unavailable:{}", AppClock::instance_id()),
            connect_timeout_ms: 20,
        };
        crate::infra::cache::redis::install_global(Some(&unavailable))
            .expect("lazy unavailable Redis client should initialize");
        let unavailable_config = resolve_config(Some(QueryRecorderApiCacheConfig {
            enabled: true,
            records_ttl_ms: Some(5_000),
            stats_ttl_ms: Some(5_000),
            command_timeout_ms: Some(50),
            max_value_bytes: Some(64 * 1024),
        }))
        .expect("cache config should resolve")
        .expect("cache should be enabled");
        let unavailable_cache = ApiCache::new("query-log", &database_a, unavailable_config)
            .expect("fail-open API cache should initialize");
        for _ in 0..CIRCUIT_FAILURE_THRESHOLD {
            assert!(matches!(
                unavailable_cache
                    .lookup::<Vec<String>>("records", identity)
                    .await,
                CacheLookup::Bypass
            ));
        }
        assert_ne!(
            unavailable_cache
                .circuit
                .open_until_ms
                .load(Ordering::Acquire),
            0
        );

        crate::infra::cache::redis::restore_global(Some(service));
    }

    #[cfg(not(feature = "storage-redis"))]
    #[test]
    fn enabled_cache_requires_storage_redis_feature() {
        let config = QueryRecorderApiCacheConfig {
            enabled: true,
            records_ttl_ms: None,
            stats_ttl_ms: None,
            command_timeout_ms: None,
            max_value_bytes: None,
        };
        let err = resolve_config(Some(config)).expect_err("feature must be required");
        assert!(err.to_string().contains("storage-redis"));
    }
}
