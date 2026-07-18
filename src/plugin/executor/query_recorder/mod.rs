// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! `query_recorder` executor plugin.
//!
//! Records structured request/response snapshots plus execution-path events
//! into recorder-scoped SQLite, PostgreSQL, or MySQL tables.
//!
//! Design constraints:
//! - pure executor observer, no server-path finalization hook;
//! - request snapshot is captured at recorder entry, response snapshot after
//!   `next`;
//! - each recorder owns its own bounded writer queue, tail buffer, and SSE
//!   broadcaster;
//! - persistence uses one `records` table and one `steps` table per recorder
//!   schema version.

mod api;
mod api_cache;
mod backend;
mod capture;
mod model;
mod remote;
mod store;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use jiff::Timestamp;
use serde_yaml_ng::Value as YamlValue;

use self::backend::RecorderBackend;
use self::model::{
    PendingRecord, QueryRecorderConfig, RecorderDatabaseConfig, ResolvedDatabaseConfig,
    ResolvedRecorderConfig,
};
use crate::config::types::PluginConfig;
use crate::core::context::DnsContext;
use crate::infra::clock::AppClock;
use crate::infra::error::{DnsError, Result};
use crate::infra::task as task_center;
use crate::plugin::executor::{ExecStep, Executor, ExecutorNext};
use crate::plugin::{Plugin, PluginFactory, UninitializedPlugin};
use crate::{continue_next, plugin_factory};

const DEFAULT_QUEUE_SIZE: usize = 8_192;
const DEFAULT_BATCH_SIZE: usize = 256;
const DEFAULT_FLUSH_INTERVAL_MS: u64 = 200;
const DEFAULT_MEMORY_TAIL: usize = 1_024;
const DEFAULT_RETENTION_DAYS: u64 = 7;
const DEFAULT_CLEANUP_INTERVAL_HOURS: u64 = 1;
const DEFAULT_READER_CONCURRENCY: usize = 2;
const DEFAULT_DATABASE_MAX_CONNECTIONS: u32 = 8;
const DEFAULT_CONNECT_TIMEOUT_MS: u64 = 5_000;
const DEFAULT_ACQUIRE_TIMEOUT_MS: u64 = 3_000;
const DEFAULT_QUERY_TIMEOUT_MS: u64 = 20_000;
const MAX_DATABASE_CONNECTIONS: u32 = 256;
const MAX_DATABASE_TIMEOUT_MS: u64 = 300_000;
const ONE_DAY_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Debug)]
struct QueryRecorder {
    tag: String,
    config: ResolvedRecorderConfig,
    backend: Option<Arc<RecorderBackend>>,
    cleanup_task_id: Option<u64>,
}

#[async_trait]
impl Plugin for QueryRecorder {
    fn tag(&self) -> &str {
        &self.tag
    }

    async fn init(&mut self, _context: &crate::plugin::PluginInitContext<'_>) -> Result<()> {
        let backend = RecorderBackend::run(self.tag.clone(), self.config.clone()).await?;
        let api_cache = self
            .config
            .api_cache
            .clone()
            .map(|config| {
                api_cache::ApiCache::new(
                    &self.tag,
                    &self.config.database.cache_identity_digest(),
                    config,
                )
            })
            .transpose()?;
        api::register(&backend, api_cache)?;

        let recorder_backend = backend.clone();
        let retention_ms = self.config.retention_days.saturating_mul(ONE_DAY_MS) as i64;
        self.cleanup_task_id = Some(task_center::spawn_fixed(
            format!("query_recorder:{}:cleanup", self.tag),
            Duration::from_secs(self.config.cleanup_interval_hours * 60 * 60),
            move || {
                let recorder_backend = recorder_backend.clone();
                async move {
                    recorder_backend.cleanup(Timestamp::now().as_millisecond() - retention_ms);
                }
            },
        ));
        self.backend.replace(backend);
        Ok(())
    }

    async fn destroy(&self) -> Result<()> {
        if let Some(task_id) = self.cleanup_task_id {
            task_center::stop_task(task_id).await;
        }
        if let Some(backend) = &self.backend {
            backend.shutdown().await?;
        }
        Ok(())
    }
}

#[async_trait]
impl Executor for QueryRecorder {
    fn with_next(&self) -> bool {
        true
    }

    async fn execute(&self, context: &mut DnsContext) -> Result<ExecStep> {
        self.execute_with_next(context, None).await
    }

    async fn execute_with_next(
        &self,
        context: &mut DnsContext,
        next: Option<ExecutorNext>,
    ) -> Result<ExecStep> {
        let Some(backend) = &self.backend else {
            return Err(DnsError::runtime(
                "query_recorder backend is not initialized",
            ));
        };

        let request = context.request.clone();
        context.enable_execution_path();
        let step_start_index = context.execution_path_len();
        let instant = AppClock::now();
        let timestamp = Timestamp::now();
        let result = continue_next!(next, context);
        let pending_record = PendingRecord::new(
            request,
            context.response.clone(),
            timestamp.as_millisecond(),
            instant.elapsed().as_millis() as u64,
            context.execution_path.clone(),
            step_start_index,
            context.peer_addr(),
            result.as_ref().err().map(ToString::to_string),
        );
        backend.enqueue(pending_record);
        result
    }
}

impl QueryRecorder {
    fn new(tag: String, config: ResolvedRecorderConfig) -> Self {
        Self {
            tag,
            config,
            backend: None,
            cleanup_task_id: None,
        }
    }
}

fn resolve_config(args: Option<YamlValue>) -> Result<ResolvedRecorderConfig> {
    let args = args.ok_or_else(|| DnsError::plugin("query_recorder requires structured args"))?;
    let parsed = serde_yaml_ng::from_value::<QueryRecorderConfig>(args)
        .map_err(|err| DnsError::plugin(format!("failed to parse query_recorder config: {err}")))?;

    let queue_size = parsed.queue_size.unwrap_or(DEFAULT_QUEUE_SIZE);
    let batch_size = parsed.batch_size.unwrap_or(DEFAULT_BATCH_SIZE);
    let flush_interval_ms = parsed
        .flush_interval_ms
        .unwrap_or(DEFAULT_FLUSH_INTERVAL_MS);
    let memory_tail = parsed.memory_tail.unwrap_or(DEFAULT_MEMORY_TAIL);
    let retention_days = parsed.retention_days.unwrap_or(DEFAULT_RETENTION_DAYS);
    let cleanup_interval_hours = parsed
        .cleanup_interval_hours
        .unwrap_or(DEFAULT_CLEANUP_INTERVAL_HOURS);
    let reader_concurrency = parsed
        .reader_concurrency
        .unwrap_or(DEFAULT_READER_CONCURRENCY);

    if queue_size == 0 {
        return Err(DnsError::plugin(
            "query_recorder queue_size must be greater than 0",
        ));
    }
    if batch_size == 0 {
        return Err(DnsError::plugin(
            "query_recorder batch_size must be greater than 0",
        ));
    }
    if flush_interval_ms == 0 {
        return Err(DnsError::plugin(
            "query_recorder flush_interval_ms must be greater than 0",
        ));
    }
    if memory_tail == 0 {
        return Err(DnsError::plugin(
            "query_recorder memory_tail must be greater than 0",
        ));
    }
    if retention_days == 0 {
        return Err(DnsError::plugin(
            "query_recorder retention_days must be at least 1",
        ));
    }
    if cleanup_interval_hours == 0 {
        return Err(DnsError::plugin(
            "query_recorder cleanup_interval_hours must be at least 1",
        ));
    }
    if reader_concurrency == 0 {
        return Err(DnsError::plugin(
            "query_recorder reader_concurrency must be greater than 0",
        ));
    }

    let database = resolve_database_config(parsed.path, parsed.database, reader_concurrency)?;
    let api_cache = api_cache::resolve_config(parsed.api_cache)?;

    Ok(ResolvedRecorderConfig {
        database,
        api_cache,
        queue_size,
        batch_size,
        flush_interval_ms,
        memory_tail,
        retention_days,
        cleanup_interval_hours,
        reader_concurrency,
    })
}

fn resolve_database_config(
    legacy_path: Option<String>,
    database: Option<RecorderDatabaseConfig>,
    reader_concurrency: usize,
) -> Result<ResolvedDatabaseConfig> {
    if legacy_path.is_some() && database.is_some() {
        return Err(DnsError::plugin(
            "query_recorder args.path and args.database are mutually exclusive",
        ));
    }

    let database = match (legacy_path, database) {
        (Some(path), None) => RecorderDatabaseConfig::Sqlite {
            path,
            acquire_timeout_ms: None,
            query_timeout_ms: None,
        },
        (None, Some(database)) => database,
        (None, None) => {
            return Err(DnsError::plugin(
                "query_recorder requires args.database or legacy args.path",
            ));
        }
        (Some(_), Some(_)) => unreachable!("mutual exclusion checked above"),
    };

    match database {
        RecorderDatabaseConfig::Sqlite {
            path,
            acquire_timeout_ms,
            query_timeout_ms,
        } => {
            let path = path.trim();
            if path.is_empty() {
                return Err(DnsError::plugin(
                    "query_recorder database.path cannot be empty",
                ));
            }
            Ok(ResolvedDatabaseConfig::Sqlite {
                path: PathBuf::from(path),
                acquire_timeout_ms: checked_timeout(
                    "acquire_timeout_ms",
                    acquire_timeout_ms.unwrap_or(DEFAULT_ACQUIRE_TIMEOUT_MS),
                )?,
                query_timeout_ms: checked_timeout(
                    "query_timeout_ms",
                    query_timeout_ms.unwrap_or(DEFAULT_QUERY_TIMEOUT_MS),
                )?,
            })
        }
        RecorderDatabaseConfig::Postgres {
            url,
            max_connections,
            connect_timeout_ms,
            acquire_timeout_ms,
            query_timeout_ms,
        } => resolve_remote_database(
            true,
            url,
            max_connections,
            connect_timeout_ms,
            acquire_timeout_ms,
            query_timeout_ms,
            reader_concurrency,
        ),
        RecorderDatabaseConfig::Mysql {
            url,
            max_connections,
            connect_timeout_ms,
            acquire_timeout_ms,
            query_timeout_ms,
        } => resolve_remote_database(
            false,
            url,
            max_connections,
            connect_timeout_ms,
            acquire_timeout_ms,
            query_timeout_ms,
            reader_concurrency,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_remote_database(
    postgres: bool,
    url: String,
    max_connections: Option<u32>,
    connect_timeout_ms: Option<u64>,
    acquire_timeout_ms: Option<u64>,
    query_timeout_ms: Option<u64>,
    reader_concurrency: usize,
) -> Result<ResolvedDatabaseConfig> {
    let trimmed_url = url.trim();
    if trimmed_url.is_empty() {
        return Err(DnsError::plugin(
            "query_recorder database.url cannot be empty",
        ));
    }
    let parsed_url = url::Url::parse(trimmed_url)
        .map_err(|_| DnsError::plugin("query_recorder database.url is invalid"))?;
    let scheme_valid = if postgres {
        matches!(parsed_url.scheme(), "postgres" | "postgresql")
    } else {
        parsed_url.scheme() == "mysql"
    };
    if !scheme_valid {
        let expected = if postgres {
            "postgres:// or postgresql://"
        } else {
            "mysql://"
        };
        return Err(DnsError::plugin(format!(
            "query_recorder database.url must use {expected}"
        )));
    }

    let max_connections = max_connections.unwrap_or(DEFAULT_DATABASE_MAX_CONNECTIONS);
    if !(2..=MAX_DATABASE_CONNECTIONS).contains(&max_connections) {
        return Err(DnsError::plugin(format!(
            "query_recorder database.max_connections must be between 2 and {MAX_DATABASE_CONNECTIONS}"
        )));
    }
    if reader_concurrency >= max_connections as usize {
        return Err(DnsError::plugin(
            "query_recorder reader_concurrency must be less than database.max_connections",
        ));
    }

    let connect_timeout_ms = checked_timeout(
        "connect_timeout_ms",
        connect_timeout_ms.unwrap_or(DEFAULT_CONNECT_TIMEOUT_MS),
    )?;
    let acquire_timeout_ms = checked_timeout(
        "acquire_timeout_ms",
        acquire_timeout_ms.unwrap_or(DEFAULT_ACQUIRE_TIMEOUT_MS),
    )?;
    let query_timeout_ms = checked_timeout(
        "query_timeout_ms",
        query_timeout_ms.unwrap_or(DEFAULT_QUERY_TIMEOUT_MS),
    )?;
    let url = trimmed_url.to_string();

    if postgres {
        Ok(ResolvedDatabaseConfig::Postgres {
            url,
            max_connections,
            connect_timeout_ms,
            acquire_timeout_ms,
            query_timeout_ms,
        })
    } else {
        Ok(ResolvedDatabaseConfig::Mysql {
            url,
            max_connections,
            connect_timeout_ms,
            acquire_timeout_ms,
            query_timeout_ms,
        })
    }
}

fn checked_timeout(field: &str, value: u64) -> Result<u64> {
    if !(1..=MAX_DATABASE_TIMEOUT_MS).contains(&value) {
        return Err(DnsError::plugin(format!(
            "query_recorder database.{field} must be between 1 and {MAX_DATABASE_TIMEOUT_MS}"
        )));
    }
    Ok(value)
}

#[derive(Debug, Clone)]
#[plugin_factory("query_recorder")]
pub struct QueryRecorderFactory;

impl PluginFactory for QueryRecorderFactory {
    fn create(
        &self,
        plugin_config: &PluginConfig,
        _init_context: &crate::plugin::PluginInitContext<'_>,
    ) -> Result<UninitializedPlugin> {
        let config = resolve_config(plugin_config.args.clone())?;
        Ok(UninitializedPlugin::Executor(Box::new(QueryRecorder::new(
            plugin_config.tag.clone(),
            config,
        ))))
    }
}
