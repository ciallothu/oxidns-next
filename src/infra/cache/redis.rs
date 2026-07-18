// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Optional process-wide Redis connection infrastructure.
//!
//! Consumers own their latency budgets, concurrency limits, circuit breakers,
//! and cache semantics. This layer only validates the shared connection
//! configuration, maintains a lazy reconnecting connection, and exposes the
//! small set of binary-safe commands needed by cache implementations.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use ::redis::aio::{ConnectionManager, ConnectionManagerConfig};

use crate::config::types::RedisStorageConfig;
use crate::infra::error::{DnsError, Result};

const REDIS_PIPELINE_BUFFER_SIZE: usize = 256;
const REDIS_CONNECTION_CONCURRENCY_LIMIT: usize = 256;
const ADVANCE_GENERATION_SCRIPT: &str = r#"
local current = tonumber(redis.call('GET', KEYS[1])) or 0
local minimum = tonumber(ARGV[1]) or 0
if current < minimum then
    redis.call('SET', KEYS[1], ARGV[1])
    return minimum
end
return redis.call('INCR', KEYS[1])
"#;

pub(crate) struct RedisService {
    manager: ConnectionManager,
    key_prefix: String,
}

impl std::fmt::Debug for RedisService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RedisService")
            .field("connection", &"<redacted>")
            .field("key_prefix", &self.key_prefix)
            .finish()
    }
}

impl RedisService {
    fn from_config(config: &RedisStorageConfig) -> Result<Self> {
        let url = config.url.trim();
        if url.is_empty() {
            return Err(DnsError::config("storage.redis.url cannot be empty"));
        }
        let key_prefix = config.key_prefix.trim().trim_end_matches(':');
        if key_prefix.is_empty() {
            return Err(DnsError::config("storage.redis.key_prefix cannot be empty"));
        }
        if config.connect_timeout_ms == 0 {
            return Err(DnsError::config(
                "storage.redis.connect_timeout_ms must be greater than 0",
            ));
        }

        crate::infra::network::tls_config::install_default_provider();

        // Never attach the dependency error here: malformed URL errors may
        // contain the original URL, including a password.
        let client = ::redis::Client::open(url)
            .map_err(|_| DnsError::config("storage.redis.url is invalid"))?;
        let timeout = Duration::from_millis(config.connect_timeout_ms);
        let manager_config = ConnectionManagerConfig::new()
            .set_connection_timeout(Some(timeout))
            .set_response_timeout(Some(timeout))
            .set_pipeline_buffer_size(REDIS_PIPELINE_BUFFER_SIZE)
            .set_concurrency_limit(REDIS_CONNECTION_CONCURRENCY_LIMIT);
        let manager = ConnectionManager::new_lazy_with_config(client, manager_config)
            .map_err(|_| DnsError::config("failed to initialize shared Redis client"))?;

        Ok(Self {
            manager,
            key_prefix: key_prefix.to_string(),
        })
    }

    pub(crate) fn key(&self, suffix: &str) -> String {
        format!("{}:{}", self.key_prefix, suffix.trim_start_matches(':'))
    }

    pub(crate) async fn get_with_ttl_ms(
        &self,
        key: &str,
    ) -> ::redis::RedisResult<Option<(Vec<u8>, u64)>> {
        let mut connection = self.manager.clone();
        let (value, ttl_ms): (Option<Vec<u8>>, i64) = ::redis::pipe()
            .atomic()
            .cmd("GET")
            .arg(key)
            .cmd("PTTL")
            .arg(key)
            .query_async(&mut connection)
            .await?;
        match (value, ttl_ms) {
            (Some(value), ttl_ms) if ttl_ms > 0 => Ok(Some((value, ttl_ms as u64))),
            _ => Ok(None),
        }
    }

    pub(crate) async fn get_u64(&self, key: &str) -> ::redis::RedisResult<Option<u64>> {
        let mut connection = self.manager.clone();
        ::redis::cmd("GET")
            .arg(key)
            .query_async(&mut connection)
            .await
    }

    pub(crate) async fn set_px(
        &self,
        key: &str,
        value: &[u8],
        ttl_ms: u64,
    ) -> ::redis::RedisResult<()> {
        let mut connection = self.manager.clone();
        ::redis::cmd("SET")
            .arg(key)
            .arg(value)
            .arg("PX")
            .arg(ttl_ms)
            .query_async(&mut connection)
            .await
    }

    pub(crate) async fn unlink(&self, key: &str) -> ::redis::RedisResult<()> {
        let mut connection = self.manager.clone();
        let _: u64 = ::redis::cmd("UNLINK")
            .arg(key)
            .query_async(&mut connection)
            .await?;
        Ok(())
    }

    pub(crate) async fn increment(&self, key: &str) -> ::redis::RedisResult<u64> {
        let mut connection = self.manager.clone();
        ::redis::cmd("INCR")
            .arg(key)
            .query_async(&mut connection)
            .await
    }

    /// Atomically publish a generation that is at least `minimum` while still
    /// advancing once when another replica has already reached that value.
    pub(crate) async fn advance_generation(
        &self,
        key: &str,
        minimum: u64,
    ) -> ::redis::RedisResult<u64> {
        let mut connection = self.manager.clone();
        ::redis::cmd("EVAL")
            .arg(ADVANCE_GENERATION_SCRIPT)
            .arg(1)
            .arg(key)
            .arg(minimum)
            .query_async(&mut connection)
            .await
    }
}

fn global_slot() -> &'static Mutex<Option<Arc<RedisService>>> {
    static GLOBAL: OnceLock<Mutex<Option<Arc<RedisService>>>> = OnceLock::new();
    GLOBAL.get_or_init(|| Mutex::new(None))
}

pub(crate) fn install_global(config: Option<&RedisStorageConfig>) -> Result<()> {
    let service = config
        .map(RedisService::from_config)
        .transpose()?
        .map(Arc::new);
    *global_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = service;
    Ok(())
}

pub(crate) fn restore_global(service: Option<Arc<RedisService>>) {
    *global_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = service;
}

pub(crate) fn clear_global() {
    restore_global(None);
}

pub(crate) fn global() -> Option<Arc<RedisService>> {
    global_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}
