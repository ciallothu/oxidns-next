// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Redis-backed L2 storage for the DNS response cache.
//!
//! L1 remains authoritative for the latency-sensitive local lookup. Redis is
//! consulted only after an L1 miss, and every Redis operation is fail-open,
//! bounded by a strict timeout, a non-waiting concurrency gate, and a circuit
//! breaker. Writes are best-effort through a bounded background queue.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio::sync::{Mutex as AsyncMutex, Semaphore, mpsc};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};
use wincode::{SchemaRead, SchemaWrite};

use super::key::CacheKey;
use super::{
    CacheItem, CacheMap, CacheMetrics, DEFAULT_REDIS_COMMAND_TIMEOUT_MS,
    DEFAULT_REDIS_FAILURE_THRESHOLD, DEFAULT_REDIS_MAX_INFLIGHT, DEFAULT_REDIS_RETRY_AFTER_MS,
    DEFAULT_REDIS_WRITE_QUEUE_SIZE, RedisCacheConfig,
};
use crate::infra::cache::redis::RedisService;
use crate::infra::clock::AppClock;
use crate::infra::error::{DnsError, Result};
use crate::proto::Message;

const REDIS_ENTRY_FORMAT_VERSION: u8 = 1;
const REDIS_KEY_SCHEMA_VERSION: &str = "v1";
const GENERATION_REFRESH_INTERVAL_MS: u64 = 1_000;

#[derive(Debug, Clone)]
pub(super) struct RedisCachePolicy {
    pub(super) ecs_in_key: bool,
    pub(super) cache_negative: bool,
    pub(super) max_negative_ttl: u32,
    pub(super) negative_ttl_without_soa: u32,
    pub(super) max_positive_ttl: Option<u32>,
    pub(super) min_positive_ttl: Option<u32>,
    pub(super) lazy_cache_ttl: Option<u32>,
}

#[derive(Debug)]
pub(super) struct RedisHydratedEntry {
    generation: RedisGenerationToken,
    response: Message,
    ttl: u32,
    cache_age_ms: u64,
    remaining_fresh_ms: u64,
    remaining_total_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RedisGenerationToken(u64);

#[derive(Debug, Default)]
struct GenerationState {
    value: AtomicU64,
    fence: RwLock<()>,
}

impl GenerationState {
    fn current(&self) -> RedisGenerationToken {
        RedisGenerationToken(self.value.load(Ordering::Acquire))
    }

    fn is_current(&self, token: RedisGenerationToken) -> bool {
        self.current() == token
    }

    fn bump(&self) -> RedisGenerationToken {
        let _guard = self
            .fence
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let next = self.value.load(Ordering::Relaxed).saturating_add(1);
        self.value.store(next, Ordering::Release);
        RedisGenerationToken(next)
    }

    fn advance_to(&self, remote: u64) -> RedisGenerationToken {
        let _guard = self
            .fence
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let current = self.value.load(Ordering::Relaxed);
        let next = current.max(remote);
        self.value.store(next, Ordering::Release);
        RedisGenerationToken(next)
    }

    fn with_current<R>(
        &self,
        token: RedisGenerationToken,
        action: impl FnOnce() -> R,
    ) -> Option<R> {
        let _guard = self
            .fence
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.value.load(Ordering::Acquire) != token.0 {
            return None;
        }
        Some(action())
    }
}

#[derive(Debug, SchemaRead, SchemaWrite)]
struct RedisWireEntry {
    format_version: u8,
    total_lifetime_ms: u64,
    fresh_lifetime_ms: u64,
    ttl: u32,
    response_wire: Vec<u8>,
}

#[derive(Debug)]
enum RedisWriteCommand {
    Set {
        generation: RedisGenerationToken,
        key: CacheKey,
        response: Message,
        ttl: u32,
        fresh_lifetime_ms: u64,
        total_lifetime_ms: u64,
    },
    Delete {
        generation: RedisGenerationToken,
        key: CacheKey,
    },
}

impl RedisWriteCommand {
    fn generation(&self) -> RedisGenerationToken {
        match self {
            Self::Set { generation, .. } | Self::Delete { generation, .. } => *generation,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RedisWriteOutcome {
    Written,
    StaleGeneration,
}

#[derive(Debug)]
struct CircuitBreaker {
    failure_threshold: usize,
    retry_after_ms: u64,
    half_open_lease_ms: u64,
    consecutive_failures: AtomicUsize,
    open_until_ms: AtomicU64,
}

impl CircuitBreaker {
    fn new(failure_threshold: usize, retry_after_ms: u64, command_timeout: Duration) -> Self {
        let command_timeout_ms = u64::try_from(command_timeout.as_millis()).unwrap_or(u64::MAX);
        Self {
            failure_threshold,
            retry_after_ms,
            half_open_lease_ms: retry_after_ms.max(command_timeout_ms.saturating_mul(2)),
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
            let lease_until_ms = now_ms.saturating_add(self.half_open_lease_ms);
            if self
                .open_until_ms
                .compare_exchange(state, lease_until_ms, Ordering::AcqRel, Ordering::Acquire)
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
        if failures >= self.failure_threshold {
            let open_until = now_ms.saturating_add(self.retry_after_ms);
            let previous = self.open_until_ms.swap(open_until, Ordering::AcqRel);
            if previous == 0 || previous <= now_ms {
                warn!(
                    retry_after_ms = self.retry_after_ms,
                    "Redis DNS cache circuit opened; falling back to L1 and upstream"
                );
            }
        }
    }
}

#[derive(Debug)]
struct RedisSharedState {
    service: Arc<RedisService>,
    base_key: String,
    generation_key: String,
    generation: GenerationState,
    generation_initialized: AtomicBool,
    last_generation_refresh_ms: AtomicU64,
    generation_refresh_lock: AsyncMutex<()>,
    command_timeout: Duration,
    circuit: CircuitBreaker,
    metrics: Arc<CacheMetrics>,
}

impl RedisSharedState {
    async fn refresh_generation_if_due(&self) -> ::redis::RedisResult<()> {
        let now_ms = AppClock::elapsed_millis();
        let initialized = self.generation_initialized.load(Ordering::Acquire);
        let last_refresh = self.last_generation_refresh_ms.load(Ordering::Acquire);
        if initialized && now_ms.saturating_sub(last_refresh) < GENERATION_REFRESH_INTERVAL_MS {
            return Ok(());
        }

        let _guard = self.generation_refresh_lock.lock().await;
        let now_ms = AppClock::elapsed_millis();
        let initialized = self.generation_initialized.load(Ordering::Acquire);
        let last_refresh = self.last_generation_refresh_ms.load(Ordering::Acquire);
        if initialized && now_ms.saturating_sub(last_refresh) < GENERATION_REFRESH_INTERVAL_MS {
            return Ok(());
        }

        let remote_generation = self
            .service
            .get_u64(&self.generation_key)
            .await?
            .unwrap_or(0);
        self.generation.advance_to(remote_generation);
        self.generation_initialized.store(true, Ordering::Release);
        self.last_generation_refresh_ms
            .store(now_ms, Ordering::Release);
        Ok(())
    }

    fn entry_key(&self, key: &CacheKey, generation: RedisGenerationToken) -> String {
        format!(
            "{}:{}:{}",
            self.base_key,
            generation.0,
            cache_key_digest(key)
        )
    }
}

#[derive(Debug)]
pub(super) struct RedisDnsCache {
    shared: Arc<RedisSharedState>,
    inflight: Arc<Semaphore>,
    write_tx: mpsc::Sender<RedisWriteCommand>,
    cancellation: CancellationToken,
    writer_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl RedisDnsCache {
    pub(super) fn new(
        tag: &str,
        config: &RedisCacheConfig,
        policy: RedisCachePolicy,
        metrics: Arc<CacheMetrics>,
    ) -> Result<Self> {
        let service = crate::infra::cache::redis::global().ok_or_else(|| {
            DnsError::plugin("cache redis.enabled requires a configured storage.redis connection")
        })?;
        let command_timeout = Duration::from_millis(
            config
                .command_timeout_ms
                .unwrap_or(DEFAULT_REDIS_COMMAND_TIMEOUT_MS),
        );
        let max_inflight = config.max_inflight.unwrap_or(DEFAULT_REDIS_MAX_INFLIGHT);
        let write_queue_size = config
            .write_queue_size
            .unwrap_or(DEFAULT_REDIS_WRITE_QUEUE_SIZE);
        let failure_threshold = config
            .failure_threshold
            .unwrap_or(DEFAULT_REDIS_FAILURE_THRESHOLD);
        let retry_after_ms = config
            .retry_after_ms
            .unwrap_or(DEFAULT_REDIS_RETRY_AFTER_MS);

        let tag_digest = short_digest(tag.as_bytes());
        let policy_digest = policy_digest(&policy);
        let base_key = service.key(&format!(
            "dns:{}:{}:{}",
            REDIS_KEY_SCHEMA_VERSION, tag_digest, policy_digest
        ));
        let shared = Arc::new(RedisSharedState {
            generation_key: format!("{}:generation", base_key),
            service,
            base_key,
            generation: GenerationState::default(),
            generation_initialized: AtomicBool::new(false),
            last_generation_refresh_ms: AtomicU64::new(0),
            generation_refresh_lock: AsyncMutex::new(()),
            command_timeout,
            circuit: CircuitBreaker::new(failure_threshold, retry_after_ms, command_timeout),
            metrics,
        });

        let (write_tx, write_rx) = mpsc::channel(write_queue_size);
        let cancellation = CancellationToken::new();
        let writer_handle =
            tokio::spawn(run_writer(shared.clone(), cancellation.clone(), write_rx));

        Ok(Self {
            shared,
            inflight: Arc::new(Semaphore::new(max_inflight)),
            write_tx,
            cancellation,
            writer_handle: Mutex::new(Some(writer_handle)),
        })
    }

    pub(super) async fn lookup(&self, key: &CacheKey) -> Option<RedisHydratedEntry> {
        let now_ms = AppClock::elapsed_millis();
        if !self.shared.circuit.allow_request(now_ms) {
            self.shared.metrics.record_l2_lookup_bypass();
            return None;
        }
        let Ok(_permit) = self.inflight.clone().try_acquire_owned() else {
            self.shared.metrics.record_l2_lookup_bypass();
            return None;
        };

        let operation = async {
            self.shared.refresh_generation_if_due().await?;
            let generation = self.shared.generation.current();
            let redis_key = self.shared.entry_key(key, generation);
            self.shared
                .service
                .get_with_ttl_ms(&redis_key)
                .await
                .map(|value| (generation, value))
        };
        let result = tokio::time::timeout(self.shared.command_timeout, operation).await;
        let (generation, value) = match result {
            Ok(Ok(value)) => {
                self.shared.circuit.record_success();
                value
            }
            Ok(Err(err)) => {
                self.shared
                    .circuit
                    .record_failure(AppClock::elapsed_millis());
                self.shared.metrics.record_l2_lookup_error();
                debug!(error = %err, "Redis DNS cache lookup failed; using upstream");
                return None;
            }
            Err(_) => {
                self.shared
                    .circuit
                    .record_failure(AppClock::elapsed_millis());
                self.shared.metrics.record_l2_lookup_error();
                debug!("Redis DNS cache lookup timed out; using upstream");
                return None;
            }
        };

        let Some((encoded, remaining_total_ms)) = value else {
            self.shared.metrics.record_l2_lookup_miss();
            return None;
        };
        let entry = match decode_entry(&encoded, remaining_total_ms, generation) {
            Ok(entry) => entry,
            Err(err) => {
                self.shared.metrics.record_l2_lookup_error();
                debug!(error = %err, "Discarding invalid Redis DNS cache entry");
                self.enqueue_delete_for_generation(key.clone(), generation);
                return None;
            }
        };
        if !self.shared.generation.is_current(generation) {
            self.shared.metrics.record_l2_lookup_bypass();
            return None;
        }
        self.shared.metrics.record_l2_lookup_hit();
        Some(entry)
    }

    pub(super) fn populate_l1(
        &self,
        cache_map: &CacheMap,
        key: CacheKey,
        entry: RedisHydratedEntry,
    ) -> bool {
        let populated = populate_l1_if_current(&self.shared.generation, cache_map, key, entry);
        if !populated {
            self.shared.metrics.record_l2_lookup_bypass();
        }
        populated
    }

    pub(super) fn enqueue_set(
        &self,
        key: CacheKey,
        response: Message,
        ttl: u32,
        fresh_lifetime_ms: u64,
        total_lifetime_ms: u64,
    ) {
        if total_lifetime_ms == 0 {
            self.shared.metrics.record_l2_write_dropped();
            return;
        }
        let generation = self.shared.generation.current();
        if self
            .write_tx
            .try_send(RedisWriteCommand::Set {
                generation,
                key,
                response,
                ttl,
                fresh_lifetime_ms,
                total_lifetime_ms,
            })
            .is_err()
        {
            self.shared.metrics.record_l2_write_dropped();
        }
    }

    pub(super) fn enqueue_delete(&self, key: CacheKey) {
        self.enqueue_delete_for_generation(key, self.shared.generation.current());
    }

    fn enqueue_delete_for_generation(&self, key: CacheKey, generation: RedisGenerationToken) {
        if self
            .write_tx
            .try_send(RedisWriteCommand::Delete { generation, key })
            .is_err()
        {
            self.shared.metrics.record_l2_write_dropped();
        }
    }

    pub(super) fn invalidate_local(&self) -> RedisGenerationToken {
        let generation = self.shared.generation.bump();
        self.shared
            .generation_initialized
            .store(true, Ordering::Release);
        self.shared
            .last_generation_refresh_ms
            .store(AppClock::elapsed_millis(), Ordering::Release);
        generation
    }

    pub(super) async fn propagate_invalidation(&self, generation: RedisGenerationToken) {
        // Explicit management invalidation must bypass the circuit's admission
        // gate. Otherwise a flush performed during the retry window would only
        // clear this replica's L1 and could never be replayed after Redis
        // recovers. The command remains strictly bounded by command_timeout,
        // and its outcome still closes or extends the circuit as appropriate.
        let result = tokio::time::timeout(
            self.shared.command_timeout,
            self.shared
                .service
                .advance_generation(&self.shared.generation_key, generation.0),
        )
        .await;
        match result {
            Ok(Ok(generation)) => {
                self.shared.generation.advance_to(generation);
                self.shared.circuit.record_success();
            }
            Ok(Err(err)) => {
                self.shared
                    .circuit
                    .record_failure(AppClock::elapsed_millis());
                debug!(error = %err, "Redis DNS cache generation invalidation failed");
            }
            Err(_) => {
                self.shared
                    .circuit
                    .record_failure(AppClock::elapsed_millis());
                debug!("Redis DNS cache generation invalidation timed out");
            }
        }
    }

    pub(super) async fn shutdown(&self) {
        self.cancellation.cancel();
        let handle = self
            .writer_handle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(handle) = handle {
            let _ = handle.await;
        }
    }
}

impl Drop for RedisDnsCache {
    fn drop(&mut self) {
        self.cancellation.cancel();
        if let Some(handle) = self
            .writer_handle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            handle.abort();
        }
    }
}

fn populate_l1_if_current(
    generation_state: &GenerationState,
    cache_map: &CacheMap,
    key: CacheKey,
    entry: RedisHydratedEntry,
) -> bool {
    let RedisHydratedEntry {
        generation,
        response,
        ttl,
        cache_age_ms,
        remaining_fresh_ms,
        remaining_total_ms,
    } = entry;
    generation_state
        .with_current(generation, || {
            let now_ms = AppClock::elapsed_millis();
            let cache_time_ms = now_ms.saturating_sub(cache_age_ms);
            let fresh_until_ms = now_ms.saturating_add(remaining_fresh_ms);
            let expire_at_ms = now_ms.saturating_add(remaining_total_ms);
            cache_map.insert_or_update_with_meta(
                key,
                Arc::new(CacheItem::new(response, ttl, fresh_until_ms)),
                cache_time_ms,
                expire_at_ms,
                now_ms,
            );
        })
        .is_some()
}

async fn run_writer(
    shared: Arc<RedisSharedState>,
    cancellation: CancellationToken,
    mut write_rx: mpsc::Receiver<RedisWriteCommand>,
) {
    loop {
        let command = tokio::select! {
            _ = cancellation.cancelled() => break,
            command = write_rx.recv() => match command {
                Some(command) => command,
                None => break,
            },
        };

        let command_generation = command.generation();
        if !shared.generation.is_current(command_generation) {
            shared.metrics.record_l2_write_dropped();
            continue;
        }
        if !shared.circuit.allow_request(AppClock::elapsed_millis()) {
            shared.metrics.record_l2_write_dropped();
            continue;
        }
        let operation = async {
            shared.refresh_generation_if_due().await?;
            if !shared.generation.is_current(command_generation) {
                return Ok::<RedisWriteOutcome, ::redis::RedisError>(
                    RedisWriteOutcome::StaleGeneration,
                );
            }
            let result = match command {
                RedisWriteCommand::Set {
                    generation,
                    key,
                    response,
                    ttl,
                    fresh_lifetime_ms,
                    total_lifetime_ms,
                } => {
                    let response_wire = response.to_bytes().map_err(redis_type_error)?;
                    let encoded = wincode::serialize(&RedisWireEntry {
                        format_version: REDIS_ENTRY_FORMAT_VERSION,
                        total_lifetime_ms,
                        fresh_lifetime_ms,
                        ttl,
                        response_wire,
                    })
                    .map_err(redis_type_error)?;
                    let redis_key = shared.entry_key(&key, generation);
                    shared
                        .service
                        .set_px(&redis_key, &encoded, total_lifetime_ms)
                        .await
                }
                RedisWriteCommand::Delete { generation, key } => {
                    let redis_key = shared.entry_key(&key, generation);
                    shared.service.unlink(&redis_key).await
                }
            };
            result?;
            Ok(RedisWriteOutcome::Written)
        };

        match tokio::time::timeout(shared.command_timeout, operation).await {
            Ok(Ok(RedisWriteOutcome::Written)) => {
                shared.circuit.record_success();
                shared.metrics.record_l2_write_success();
            }
            Ok(Ok(RedisWriteOutcome::StaleGeneration)) => {
                shared.circuit.record_success();
                shared.metrics.record_l2_write_dropped();
            }
            Ok(Err(err)) => {
                shared.circuit.record_failure(AppClock::elapsed_millis());
                shared.metrics.record_l2_write_error();
                debug!(error = %err, "Redis DNS cache background write failed");
            }
            Err(_) => {
                shared.circuit.record_failure(AppClock::elapsed_millis());
                shared.metrics.record_l2_write_error();
                debug!("Redis DNS cache background write timed out");
            }
        }
    }
}

fn decode_entry(
    encoded: &[u8],
    redis_ttl_ms: u64,
    generation: RedisGenerationToken,
) -> std::result::Result<RedisHydratedEntry, String> {
    let entry: RedisWireEntry =
        wincode::deserialize(encoded).map_err(|err| format!("invalid cache payload: {err}"))?;
    if entry.format_version != REDIS_ENTRY_FORMAT_VERSION {
        return Err(format!(
            "unsupported cache payload version {}",
            entry.format_version
        ));
    }
    if entry.total_lifetime_ms == 0
        || entry.fresh_lifetime_ms == 0
        || entry.fresh_lifetime_ms > entry.total_lifetime_ms
        || entry.ttl == 0
    {
        return Err("invalid cache lifetime metadata".to_string());
    }

    let remaining_total_ms = redis_ttl_ms.min(entry.total_lifetime_ms);
    if remaining_total_ms == 0 {
        return Err("expired cache payload".to_string());
    }
    let cache_age_ms = entry.total_lifetime_ms.saturating_sub(remaining_total_ms);
    let remaining_fresh_ms = entry.fresh_lifetime_ms.saturating_sub(cache_age_ms);
    let response = Message::from_bytes(&entry.response_wire)
        .map_err(|err| format!("invalid cached DNS response: {err}"))?;
    if response.truncated() {
        return Err("truncated DNS response is not cacheable".to_string());
    }

    Ok(RedisHydratedEntry {
        generation,
        response,
        ttl: entry.ttl,
        cache_age_ms,
        remaining_fresh_ms,
        remaining_total_ms,
    })
}

fn redis_type_error(error: impl std::fmt::Display) -> ::redis::RedisError {
    ::redis::RedisError::from((
        ::redis::ErrorKind::UnexpectedReturnType,
        "failed to encode Redis DNS cache entry",
        error.to_string(),
    ))
}

fn short_digest(value: &[u8]) -> String {
    let digest = Sha256::digest(value);
    hex::encode(&digest[..16])
}

fn policy_digest(policy: &RedisCachePolicy) -> String {
    let mut digest = Sha256::new();
    digest.update([u8::from(policy.ecs_in_key)]);
    digest.update([u8::from(policy.cache_negative)]);
    digest.update(policy.max_negative_ttl.to_be_bytes());
    digest.update(policy.negative_ttl_without_soa.to_be_bytes());
    update_optional_u32(&mut digest, policy.max_positive_ttl);
    update_optional_u32(&mut digest, policy.min_positive_ttl);
    update_optional_u32(&mut digest, policy.lazy_cache_ttl);
    let digest = digest.finalize();
    hex::encode(&digest[..16])
}

fn update_optional_u32(digest: &mut Sha256, value: Option<u32>) {
    match value {
        Some(value) => {
            digest.update([1]);
            digest.update(value.to_be_bytes());
        }
        None => digest.update([0]),
    }
}

fn cache_key_digest(key: &CacheKey) -> String {
    let mut digest = Sha256::new();
    digest.update(key.domain.as_bytes());
    digest.update([0]);
    digest.update(u16::from(key.record_type).to_be_bytes());
    digest.update(u16::from(key.dns_class).to_be_bytes());
    digest.update([u8::from(key.do_bit), u8::from(key.cd_bit)]);
    if let Some(ecs) = &key.ecs_scope {
        digest.update([1]);
        digest.update(ecs.family.to_be_bytes());
        digest.update([ecs.source_prefix, ecs.scope_prefix, ecs.network_len]);
        digest.update(&ecs.network[..usize::from(ecs.network_len.min(16))]);
    } else {
        digest.update([0]);
    }
    hex::encode(digest.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::RedisStorageConfig;
    use crate::plugin::executor::cache::key::EcsScopeDigest;
    use crate::proto::{DNSClass, Message, Name, Question, Rcode, RecordType};

    fn test_key(domain: &str) -> CacheKey {
        CacheKey {
            domain: domain.to_string(),
            record_type: RecordType::A,
            dns_class: DNSClass::IN,
            do_bit: false,
            cd_bit: false,
            ecs_scope: None,
        }
    }

    fn hydrated_entry(generation: RedisGenerationToken) -> RedisHydratedEntry {
        RedisHydratedEntry {
            generation,
            response: Message::new(),
            ttl: 60,
            cache_age_ms: 0,
            remaining_fresh_ms: 60_000,
            remaining_total_ms: 60_000,
        }
    }

    #[test]
    fn redis_key_digest_does_not_expose_qname_and_changes_with_flags() {
        let plain = test_key("private.example");
        let mut dnssec = plain.clone();
        dnssec.do_bit = true;

        let plain_digest = cache_key_digest(&plain);
        let dnssec_digest = cache_key_digest(&dnssec);

        assert!(!plain_digest.contains("private.example"));
        assert_ne!(plain_digest, dnssec_digest);
    }

    #[test]
    fn redis_key_digest_includes_ecs_scope() {
        let plain = test_key("example.com");
        let mut ecs = plain.clone();
        ecs.ecs_scope = Some(EcsScopeDigest {
            family: 1,
            source_prefix: 24,
            scope_prefix: 0,
            network_len: 3,
            network: [0; 16],
        });

        assert_ne!(cache_key_digest(&plain), cache_key_digest(&ecs));
    }

    #[test]
    fn wire_entry_rejects_lifetime_inversion() {
        let encoded = wincode::serialize(&RedisWireEntry {
            format_version: REDIS_ENTRY_FORMAT_VERSION,
            total_lifetime_ms: 1_000,
            fresh_lifetime_ms: 2_000,
            ttl: 2,
            response_wire: Vec::new(),
        })
        .expect("encode invalid fixture");

        assert!(decode_entry(&encoded, 500, RedisGenerationToken(0)).is_err());
    }

    #[test]
    fn stale_queued_write_is_rejected_after_generation_bump() {
        let generation = GenerationState::default();
        let token = generation.current();
        let commands = [
            RedisWriteCommand::Set {
                generation: token,
                key: test_key("queued-set.example"),
                response: Message::new(),
                ttl: 60,
                fresh_lifetime_ms: 60_000,
                total_lifetime_ms: 60_000,
            },
            RedisWriteCommand::Delete {
                generation: token,
                key: test_key("queued-delete.example"),
            },
        ];

        assert!(
            commands
                .iter()
                .all(|command| generation.is_current(command.generation()))
        );
        generation.bump();
        assert!(
            commands
                .iter()
                .all(|command| !generation.is_current(command.generation()))
        );
    }

    #[test]
    fn abandoned_half_open_probe_recovers_after_lease_expiry() {
        let circuit = CircuitBreaker::new(1, 100, Duration::from_millis(20));
        assert_eq!(circuit.half_open_lease_ms, 100);
        circuit.record_failure(1_000);

        assert!(!circuit.allow_request(1_099));
        assert!(circuit.allow_request(1_100));
        assert!(!circuit.allow_request(1_199));

        // The first probe intentionally records neither success nor failure,
        // modeling a dropped/cancelled future. Its lease must not wedge the
        // circuit forever.
        assert!(circuit.allow_request(1_200));

        let timeout_dominated = CircuitBreaker::new(1, 10, Duration::from_millis(20));
        assert_eq!(timeout_dominated.half_open_lease_ms, 40);
    }

    #[test]
    fn stale_lookup_token_cannot_populate_l1() {
        AppClock::start();
        let generation = GenerationState::default();
        let stale_token = generation.current();
        generation.bump();
        let cache_map = CacheMap::with_capacity(4);

        assert!(!populate_l1_if_current(
            &generation,
            &cache_map,
            test_key("stale-lookup.example"),
            hydrated_entry(stale_token),
        ));
        assert!(cache_map.is_empty());
    }

    #[test]
    fn invalidation_order_clears_old_population_and_accepts_only_new_token() {
        AppClock::start();
        let generation = GenerationState::default();
        let old_token = generation.current();
        let cache_map = CacheMap::with_capacity(4);

        assert!(populate_l1_if_current(
            &generation,
            &cache_map,
            test_key("before-invalidation.example"),
            hydrated_entry(old_token),
        ));

        // Management API ordering: bump synchronously, then clear/replace L1.
        let new_token = generation.bump();
        cache_map.clear();

        assert!(!populate_l1_if_current(
            &generation,
            &cache_map,
            test_key("late-old-lookup.example"),
            hydrated_entry(old_token),
        ));
        assert!(cache_map.is_empty());
        assert!(populate_l1_if_current(
            &generation,
            &cache_map,
            test_key("new-generation.example"),
            hydrated_entry(new_token),
        ));
        assert_eq!(cache_map.len(), 1);
    }

    #[tokio::test]
    async fn test_redis_dns_cache_integration() {
        let Ok(url) = std::env::var("OXIDNS_NEXT_TEST_REDIS_URL") else {
            eprintln!(
                "skipping Redis integration test: OXIDNS_NEXT_TEST_REDIS_URL is not configured"
            );
            return;
        };

        AppClock::start();
        let test_namespace = format!("integration:{}", AppClock::instance_id());
        let storage = RedisStorageConfig {
            url,
            key_prefix: test_namespace,
            connect_timeout_ms: 2_000,
        };
        crate::infra::cache::redis::install_global(Some(&storage))
            .expect("shared Redis service should initialize");
        let service =
            crate::infra::cache::redis::global().expect("shared Redis service should be installed");

        // Exercise the process-wide binary-safe primitives first.
        let raw_key = service.key("raw");
        service
            .set_px(&raw_key, b"value", 5_000)
            .await
            .expect("SET PX should succeed");
        let (raw_value, raw_ttl_ms) = service
            .get_with_ttl_ms(&raw_key)
            .await
            .expect("GET/PTTL should succeed")
            .expect("raw key should exist");
        assert_eq!(raw_value, b"value");
        assert!((1..=5_000).contains(&raw_ttl_ms));

        let counter_key = service.key("counter");
        assert_eq!(
            service
                .increment(&counter_key)
                .await
                .expect("first INCR should succeed"),
            1
        );
        assert_eq!(service.get_u64(&counter_key).await.unwrap(), Some(1));
        service
            .unlink(&raw_key)
            .await
            .expect("UNLINK should succeed");
        assert!(service.get_with_ttl_ms(&raw_key).await.unwrap().is_none());

        // Then exercise the DNS L2 write/read path and generation invalidation.
        let metrics = Arc::new(CacheMetrics::new("redis_integration".to_string()));
        let config = RedisCacheConfig {
            enabled: Some(true),
            command_timeout_ms: Some(2_000),
            max_inflight: Some(8),
            write_queue_size: Some(32),
            failure_threshold: Some(5),
            retry_after_ms: Some(1_000),
        };
        let cache = RedisDnsCache::new(
            "redis_integration",
            &config,
            RedisCachePolicy {
                ecs_in_key: false,
                cache_negative: true,
                max_negative_ttl: 300,
                negative_ttl_without_soa: 60,
                max_positive_ttl: None,
                min_positive_ttl: None,
                lazy_cache_ttl: None,
            },
            metrics,
        )
        .expect("Redis DNS cache should initialize");

        let key = test_key("redis-integration.example");
        let mut request = Message::new();
        request.add_question(Question::new(
            Name::from_ascii("redis-integration.example.").unwrap(),
            RecordType::A,
            DNSClass::IN,
        ));
        let response = request.response(Rcode::NoError);
        cache.enqueue_set(key.clone(), response, 5, 5_000, 5_000);

        let first_entry = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(entry) = cache.lookup(&key).await {
                    break entry;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("background Redis DNS cache write should become readable");
        assert_eq!(first_entry.response.rcode(), Rcode::NoError);
        assert!(first_entry.remaining_total_ms > 0);

        let generation_zero_entry_key = cache
            .shared
            .entry_key(&key, cache.shared.generation.current());
        let circuit_opened_at = AppClock::elapsed_millis();
        for _ in 0..cache.shared.circuit.failure_threshold {
            cache.shared.circuit.record_failure(circuit_opened_at);
        }
        assert!(
            cache.shared.circuit.open_until_ms.load(Ordering::Acquire) > circuit_opened_at,
            "integration fixture should have an open circuit before explicit invalidation"
        );
        let invalidation = cache.invalidate_local();
        cache.propagate_invalidation(invalidation).await;
        assert_eq!(
            cache.shared.circuit.open_until_ms.load(Ordering::Acquire),
            0,
            "successful explicit invalidation should close the circuit"
        );
        assert!(
            service
                .get_u64(&cache.shared.generation_key)
                .await
                .expect("generation GET should succeed")
                .is_some_and(|remote| remote >= invalidation.0)
        );
        assert!(
            cache.lookup(&key).await.is_none(),
            "generation invalidation must hide the previous DNS cache entry"
        );

        cache.enqueue_set(
            key.clone(),
            request.response(Rcode::NoError),
            5,
            5_000,
            5_000,
        );
        tokio::time::timeout(Duration::from_secs(5), async {
            while cache.lookup(&key).await.is_none() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("post-invalidation generation should accept new entries");
        let current_entry_key = cache
            .shared
            .entry_key(&key, cache.shared.generation.current());

        cache.enqueue_delete(key.clone());
        tokio::time::timeout(Duration::from_secs(5), async {
            while cache.lookup(&key).await.is_some() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("background UNLINK should remove the current DNS cache entry");

        cache.shutdown().await;
        for key in [
            raw_key,
            counter_key,
            generation_zero_entry_key,
            current_entry_key,
            cache.shared.generation_key.clone(),
        ] {
            service
                .unlink(&key)
                .await
                .expect("test key cleanup should succeed");
        }
        crate::infra::cache::redis::clear_global();
    }
}
