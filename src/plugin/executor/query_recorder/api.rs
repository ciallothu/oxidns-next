// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use http::{Request, StatusCode};
use hyper::body::Frame;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use super::api_cache::{ApiCache, CacheGeneration, CacheLifetime, CacheLookup};
use super::backend::RecorderBackend;
use super::model::{
    DistributionQuery, DistributionResponse, LatencyQuery, LatencySummary, ListCursor, ListQuery,
    PluginStatsKind, PluginStatsRow, PluginsStatsQuery, QueryRecordFilter, QueryRecordStatus,
    RecordDetail, RecordSummaryRow, TimeseriesBucket, TimeseriesQuery, TimeseriesResponse,
    TopBucketsResponse, TopQuery,
};
use super::store::{
    load_latency_summary_on_connection, load_plugin_stats_on_connection,
    load_qtype_distribution_on_connection, load_rcode_distribution_on_connection,
    load_record_detail_on_connection, load_timeseries_on_connection,
    load_top_clients_on_connection, load_top_qnames_on_connection, query_records_on_connection,
};
use crate::api::query::{
    optional_text, optional_upper_text, parse_u64_param, parse_usize_param, visit_query_params,
};
use crate::api::{ApiHandler, json_error, json_ok, simple_response, streaming_response};
use crate::infra::error::{DnsError, Result};
use crate::register_plugin_api;

const DEFAULT_LIST_LIMIT: usize = 100;
const MAX_LIST_LIMIT: usize = 500;
const DEFAULT_TOP_LIMIT: usize = 20;
const DEFAULT_SLOW_LIMIT: usize = 20;
const DEFAULT_TIMESERIES_BUCKETS: usize = 60;
const MAX_TIMESERIES_BUCKETS: usize = 720;
const SSE_HEARTBEAT_SECS: u64 = 15;
const CACHE_RECORDS_LIST: &str = "records-list";
const CACHE_RECORD_DETAIL: &str = "record-detail";
const CACHE_STATS_PLUGINS: &str = "stats-plugins";
const CACHE_STATS_TOP_CLIENTS: &str = "stats-top-clients";
const CACHE_STATS_TOP_QNAMES: &str = "stats-top-qnames";
const CACHE_STATS_QTYPE: &str = "stats-qtype";
const CACHE_STATS_RCODE: &str = "stats-rcode";
const CACHE_STATS_LATENCY: &str = "stats-latency";
const CACHE_STATS_TIMESERIES: &str = "stats-timeseries";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RecordListResponse {
    ok: bool,
    next_cursor: Option<String>,
    records: Vec<RecordSummaryRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RecordDetailResponse {
    ok: bool,
    record: RecordDetail,
}

#[derive(Debug, Clone, Serialize)]
struct RecordsClearResponse {
    ok: bool,
    cleared_records: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PluginStatsResponse {
    ok: bool,
    query_total: u64,
    stats: Vec<PluginStatsRow>,
}

#[derive(Debug)]
struct RecordsListHandler {
    backend: Arc<RecorderBackend>,
    api_cache: Option<Arc<ApiCache>>,
}

#[derive(Debug)]
struct RecordDetailHandler {
    backend: Arc<RecorderBackend>,
    path_prefix: String,
    api_cache: Option<Arc<ApiCache>>,
}

#[derive(Debug)]
struct RecordsClearHandler {
    backend: Arc<RecorderBackend>,
    api_cache: Option<Arc<ApiCache>>,
}

#[derive(Debug)]
struct StatsPluginsHandler {
    backend: Arc<RecorderBackend>,
    api_cache: Option<Arc<ApiCache>>,
}

#[derive(Debug)]
struct StreamHandler {
    backend: Arc<RecorderBackend>,
}

#[derive(Debug)]
struct TopClientsHandler {
    backend: Arc<RecorderBackend>,
    api_cache: Option<Arc<ApiCache>>,
}

#[derive(Debug)]
struct TopQnamesHandler {
    backend: Arc<RecorderBackend>,
    api_cache: Option<Arc<ApiCache>>,
}

#[derive(Debug)]
struct QtypeDistributionHandler {
    backend: Arc<RecorderBackend>,
    api_cache: Option<Arc<ApiCache>>,
}

#[derive(Debug)]
struct RcodeDistributionHandler {
    backend: Arc<RecorderBackend>,
    api_cache: Option<Arc<ApiCache>>,
}

#[derive(Debug)]
struct LatencyHandler {
    backend: Arc<RecorderBackend>,
    api_cache: Option<Arc<ApiCache>>,
}

#[derive(Debug)]
struct TimeseriesHandler {
    backend: Arc<RecorderBackend>,
    api_cache: Option<Arc<ApiCache>>,
}

#[derive(Debug)]
enum ReaderQueryError {
    Busy,
    TimedOut,
    Failed(DnsError),
}

async fn run_reader_query<T, F, Fut>(
    backend: Arc<RecorderBackend>,
    op: F,
) -> std::result::Result<T, ReaderQueryError>
where
    T: Send + 'static,
    F: FnOnce(Arc<RecorderBackend>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = std::result::Result<T, DnsError>> + Send,
{
    let permit = tokio::time::timeout(
        backend.acquire_timeout,
        backend.reader_semaphore.clone().acquire_owned(),
    )
    .await
    .map_err(|_| ReaderQueryError::Busy)?
    .map_err(|_| ReaderQueryError::Busy)?;
    let result = tokio::time::timeout(backend.query_timeout, op(backend))
        .await
        .map_err(|_| ReaderQueryError::TimedOut)?
        .map_err(ReaderQueryError::Failed);
    drop(permit);
    result
}

fn reader_error_response(
    error: ReaderQueryError,
    failed_code: &'static str,
) -> crate::api::ApiResponse {
    match error {
        ReaderQueryError::Busy => json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "query_recorder_reader_busy",
            "query recorder reader capacity is busy; retry later",
        ),
        ReaderQueryError::TimedOut => json_error(
            StatusCode::GATEWAY_TIMEOUT,
            "query_recorder_query_timed_out",
            "query recorder database query timed out",
        ),
        ReaderQueryError::Failed(error) => json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            failed_code,
            error.to_string(),
        ),
    }
}

async fn lookup_cache<T>(
    cache: &Option<Arc<ApiCache>>,
    namespace: &str,
    identity: &str,
) -> CacheLookup<T>
where
    T: serde::de::DeserializeOwned + Send,
{
    match cache {
        Some(cache) => cache.lookup(namespace, identity).await,
        None => CacheLookup::Bypass,
    }
}

async fn store_cache<T>(
    cache: &Option<Arc<ApiCache>>,
    namespace: &str,
    identity: &str,
    lifetime: CacheLifetime,
    generation: Option<CacheGeneration>,
    value: &T,
) where
    T: Serialize + Sync,
{
    if let (Some(cache), Some(generation)) = (cache, generation) {
        cache
            .store(namespace, identity, lifetime, generation, value)
            .await;
    }
}

#[async_trait]
impl ApiHandler for RecordsListHandler {
    async fn handle(&self, request: Request<Bytes>) -> crate::api::ApiResponse {
        let cache_identity = request.uri().query().unwrap_or("");
        let cache_generation = match lookup_cache::<RecordListResponse>(
            &self.api_cache,
            CACHE_RECORDS_LIST,
            cache_identity,
        )
        .await
        {
            CacheLookup::Hit(response) => return json_ok(StatusCode::OK, &response),
            CacheLookup::Miss(generation) => Some(generation),
            CacheLookup::Bypass => None,
        };
        let query = match parse_list_query(request.uri().query()) {
            Ok(query) => query,
            Err(err) => return json_error(StatusCode::BAD_REQUEST, "invalid_query", err),
        };

        let backend = self.backend.clone();
        match run_reader_query(backend, move |backend| async move {
            if let Some(remote) = backend.remote_pool().cloned() {
                remote.query_records(&backend.tables, query).await
            } else {
                backend
                    .run_sqlite_reader(move |backend, conn| {
                        query_records_on_connection(backend, conn, query)
                    })
                    .await
            }
        })
        .await
        {
            Ok((records, next_cursor)) => {
                let response = RecordListResponse {
                    ok: true,
                    next_cursor,
                    records,
                };
                store_cache(
                    &self.api_cache,
                    CACHE_RECORDS_LIST,
                    cache_identity,
                    CacheLifetime::Records,
                    cache_generation,
                    &response,
                )
                .await;
                json_ok(StatusCode::OK, &response)
            }
            Err(err) => reader_error_response(err, "query_recorder_records_failed"),
        }
    }
}

#[async_trait]
impl ApiHandler for RecordDetailHandler {
    async fn handle(&self, request: Request<Bytes>) -> crate::api::ApiResponse {
        let Some(raw_id) = request.uri().path().strip_prefix(self.path_prefix.as_str()) else {
            return simple_response(StatusCode::NOT_FOUND, Bytes::from("404 Not Found"));
        };
        if raw_id.is_empty() || raw_id.contains('/') {
            return json_error(
                StatusCode::BAD_REQUEST,
                "invalid_record_id",
                "invalid record id",
            );
        }
        let record_id = match raw_id.parse::<i64>() {
            Ok(record_id) if record_id > 0 => record_id,
            _ => {
                return json_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_record_id",
                    "record id must be a positive integer",
                );
            }
        };
        let cache_generation = match lookup_cache::<RecordDetailResponse>(
            &self.api_cache,
            CACHE_RECORD_DETAIL,
            raw_id,
        )
        .await
        {
            CacheLookup::Hit(response) => return json_ok(StatusCode::OK, &response),
            CacheLookup::Miss(generation) => Some(generation),
            CacheLookup::Bypass => None,
        };

        let backend = self.backend.clone();
        match run_reader_query(backend, move |backend| async move {
            if let Some(remote) = backend.remote_pool().cloned() {
                remote.load_record_detail(&backend.tables, record_id).await
            } else {
                backend
                    .run_sqlite_reader(move |backend, conn| {
                        load_record_detail_on_connection(backend, conn, record_id)
                    })
                    .await
            }
        })
        .await
        {
            Ok(Some(record)) => {
                let response = RecordDetailResponse { ok: true, record };
                store_cache(
                    &self.api_cache,
                    CACHE_RECORD_DETAIL,
                    raw_id,
                    CacheLifetime::Records,
                    cache_generation,
                    &response,
                )
                .await;
                json_ok(StatusCode::OK, &response)
            }
            Ok(None) => json_error(
                StatusCode::NOT_FOUND,
                "record_not_found",
                format!("record {} does not exist", record_id),
            ),
            Err(err) => reader_error_response(err, "query_recorder_record_failed"),
        }
    }
}

#[async_trait]
impl ApiHandler for RecordsClearHandler {
    async fn handle(&self, _request: Request<Bytes>) -> crate::api::ApiResponse {
        let backend = self.backend.clone();
        match tokio::task::spawn_blocking(move || backend.clear_history()).await {
            Ok(Ok(result)) => {
                if let Some(api_cache) = &self.api_cache {
                    api_cache.invalidate().await;
                }
                json_ok(
                    StatusCode::OK,
                    &RecordsClearResponse {
                        ok: true,
                        cleared_records: result.cleared_records,
                    },
                )
            }
            Ok(Err(err)) => json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "query_recorder_clear_failed",
                err,
            ),
            Err(err) => json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "query_recorder_clear_failed",
                format!("blocking task failed: {err}"),
            ),
        }
    }
}

#[async_trait]
impl ApiHandler for StatsPluginsHandler {
    async fn handle(&self, request: Request<Bytes>) -> crate::api::ApiResponse {
        let cache_identity = request.uri().query().unwrap_or("");
        let cache_generation = match lookup_cache::<PluginStatsResponse>(
            &self.api_cache,
            CACHE_STATS_PLUGINS,
            cache_identity,
        )
        .await
        {
            CacheLookup::Hit(response) => return json_ok(StatusCode::OK, &response),
            CacheLookup::Miss(generation) => Some(generation),
            CacheLookup::Bypass => None,
        };
        let query = match parse_plugins_stats_query(request.uri().query()) {
            Ok(query) => query,
            Err(err) => return json_error(StatusCode::BAD_REQUEST, "invalid_query", err),
        };
        let backend = self.backend.clone();
        match run_reader_query(backend, move |backend| async move {
            if let Some(remote) = backend.remote_pool().cloned() {
                remote.load_plugin_stats(&backend.tables, query).await
            } else {
                backend
                    .run_sqlite_reader(move |backend, conn| {
                        load_plugin_stats_on_connection(backend, conn, query)
                    })
                    .await
            }
        })
        .await
        {
            Ok((query_total, stats)) => {
                let response = PluginStatsResponse {
                    ok: true,
                    query_total,
                    stats,
                };
                store_cache(
                    &self.api_cache,
                    CACHE_STATS_PLUGINS,
                    cache_identity,
                    CacheLifetime::Stats,
                    cache_generation,
                    &response,
                )
                .await;
                json_ok(StatusCode::OK, &response)
            }
            Err(err) => reader_error_response(err, "query_recorder_stats_failed"),
        }
    }
}

#[async_trait]
impl ApiHandler for StreamHandler {
    async fn handle(&self, request: Request<Bytes>) -> crate::api::ApiResponse {
        let tail_count = match parse_tail_param(request.uri().query(), self.backend.memory_tail) {
            Ok(tail_count) => tail_count,
            Err(err) => return json_error(StatusCode::BAD_REQUEST, "invalid_query", err),
        };

        let initial = {
            let guard = match self.backend.tail.lock() {
                Ok(guard) => guard,
                Err(_) => {
                    return json_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "query_recorder_stream_failed",
                        "tail buffer lock poisoned",
                    );
                }
            };
            let skip = guard.len().saturating_sub(tail_count);
            guard.iter().skip(skip).cloned().collect::<Vec<_>>()
        };

        let pending = initial
            .into_iter()
            .map(|record| sse_record_frame(&record))
            .collect::<VecDeque<_>>();
        let receiver = self.backend.broadcaster.subscribe();
        let heartbeat = tokio::time::interval(Duration::from_secs(SSE_HEARTBEAT_SECS));
        let stream = futures::stream::unfold(
            SseState {
                pending,
                receiver,
                heartbeat,
            },
            |mut state| async move {
                if let Some(bytes) = state.pending.pop_front() {
                    return Some((Ok(Frame::data(bytes)), state));
                }

                loop {
                    tokio::select! {
                        recv = state.receiver.recv() => {
                            match recv {
                                Ok(record) => return Some((Ok(Frame::data(sse_record_frame(&record))), state)),
                                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                                Err(broadcast::error::RecvError::Closed) => return None,
                            }
                        }
                        _ = state.heartbeat.tick() => {
                            return Some((Ok(Frame::data(Bytes::from_static(b": heartbeat\n\n"))), state));
                        }
                    }
                }
            },
        );

        let mut response = streaming_response(StatusCode::OK, stream);
        response.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("text/event-stream; charset=utf-8"),
        );
        response.headers_mut().insert(
            http::header::CACHE_CONTROL,
            http::HeaderValue::from_static("no-cache"),
        );
        response.headers_mut().insert(
            http::header::CONNECTION,
            http::HeaderValue::from_static("keep-alive"),
        );
        response
    }
}

#[derive(Debug)]
struct SseState {
    pending: VecDeque<Bytes>,
    receiver: broadcast::Receiver<RecordDetail>,
    heartbeat: tokio::time::Interval,
}

#[async_trait]
impl ApiHandler for TopClientsHandler {
    async fn handle(&self, request: Request<Bytes>) -> crate::api::ApiResponse {
        let cache_identity = request.uri().query().unwrap_or("");
        let cache_generation = match lookup_cache::<TopBucketsResponse>(
            &self.api_cache,
            CACHE_STATS_TOP_CLIENTS,
            cache_identity,
        )
        .await
        {
            CacheLookup::Hit(response) => return json_ok(StatusCode::OK, &response),
            CacheLookup::Miss(generation) => Some(generation),
            CacheLookup::Bypass => None,
        };
        let query = match parse_top_query(request.uri().query()) {
            Ok(query) => query,
            Err(err) => return json_error(StatusCode::BAD_REQUEST, "invalid_query", err),
        };
        let backend = self.backend.clone();
        match run_reader_query(backend, move |backend| async move {
            if let Some(remote) = backend.remote_pool().cloned() {
                remote.load_top_clients(&backend.tables, query).await
            } else {
                backend
                    .run_sqlite_reader(move |backend, conn| {
                        load_top_clients_on_connection(backend, conn, query)
                    })
                    .await
            }
        })
        .await
        {
            Ok(response) => {
                store_cache(
                    &self.api_cache,
                    CACHE_STATS_TOP_CLIENTS,
                    cache_identity,
                    CacheLifetime::Stats,
                    cache_generation,
                    &response,
                )
                .await;
                json_ok(StatusCode::OK, &response)
            }
            Err(err) => reader_error_response(err, "query_recorder_top_clients_failed"),
        }
    }
}

#[async_trait]
impl ApiHandler for TopQnamesHandler {
    async fn handle(&self, request: Request<Bytes>) -> crate::api::ApiResponse {
        let cache_identity = request.uri().query().unwrap_or("");
        let cache_generation = match lookup_cache::<TopBucketsResponse>(
            &self.api_cache,
            CACHE_STATS_TOP_QNAMES,
            cache_identity,
        )
        .await
        {
            CacheLookup::Hit(response) => return json_ok(StatusCode::OK, &response),
            CacheLookup::Miss(generation) => Some(generation),
            CacheLookup::Bypass => None,
        };
        let query = match parse_top_query(request.uri().query()) {
            Ok(query) => query,
            Err(err) => return json_error(StatusCode::BAD_REQUEST, "invalid_query", err),
        };
        let backend = self.backend.clone();
        match run_reader_query(backend, move |backend| async move {
            if let Some(remote) = backend.remote_pool().cloned() {
                remote.load_top_qnames(&backend.tables, query).await
            } else {
                backend
                    .run_sqlite_reader(move |backend, conn| {
                        load_top_qnames_on_connection(backend, conn, query)
                    })
                    .await
            }
        })
        .await
        {
            Ok(response) => {
                store_cache(
                    &self.api_cache,
                    CACHE_STATS_TOP_QNAMES,
                    cache_identity,
                    CacheLifetime::Stats,
                    cache_generation,
                    &response,
                )
                .await;
                json_ok(StatusCode::OK, &response)
            }
            Err(err) => reader_error_response(err, "query_recorder_top_qnames_failed"),
        }
    }
}

#[async_trait]
impl ApiHandler for QtypeDistributionHandler {
    async fn handle(&self, request: Request<Bytes>) -> crate::api::ApiResponse {
        let cache_identity = request.uri().query().unwrap_or("");
        let cache_generation = match lookup_cache::<DistributionResponse>(
            &self.api_cache,
            CACHE_STATS_QTYPE,
            cache_identity,
        )
        .await
        {
            CacheLookup::Hit(response) => return json_ok(StatusCode::OK, &response),
            CacheLookup::Miss(generation) => Some(generation),
            CacheLookup::Bypass => None,
        };
        let query = match parse_distribution_query(request.uri().query()) {
            Ok(query) => query,
            Err(err) => return json_error(StatusCode::BAD_REQUEST, "invalid_query", err),
        };
        let backend = self.backend.clone();
        match run_reader_query(backend, move |backend| async move {
            if let Some(remote) = backend.remote_pool().cloned() {
                remote.load_qtype_distribution(&backend.tables, query).await
            } else {
                backend
                    .run_sqlite_reader(move |backend, conn| {
                        load_qtype_distribution_on_connection(backend, conn, query)
                    })
                    .await
            }
        })
        .await
        {
            Ok(response) => {
                store_cache(
                    &self.api_cache,
                    CACHE_STATS_QTYPE,
                    cache_identity,
                    CacheLifetime::Stats,
                    cache_generation,
                    &response,
                )
                .await;
                json_ok(StatusCode::OK, &response)
            }
            Err(err) => reader_error_response(err, "query_recorder_qtype_failed"),
        }
    }
}

#[async_trait]
impl ApiHandler for RcodeDistributionHandler {
    async fn handle(&self, request: Request<Bytes>) -> crate::api::ApiResponse {
        let cache_identity = request.uri().query().unwrap_or("");
        let cache_generation = match lookup_cache::<DistributionResponse>(
            &self.api_cache,
            CACHE_STATS_RCODE,
            cache_identity,
        )
        .await
        {
            CacheLookup::Hit(response) => return json_ok(StatusCode::OK, &response),
            CacheLookup::Miss(generation) => Some(generation),
            CacheLookup::Bypass => None,
        };
        let query = match parse_distribution_query(request.uri().query()) {
            Ok(query) => query,
            Err(err) => return json_error(StatusCode::BAD_REQUEST, "invalid_query", err),
        };
        let backend = self.backend.clone();
        match run_reader_query(backend, move |backend| async move {
            if let Some(remote) = backend.remote_pool().cloned() {
                remote.load_rcode_distribution(&backend.tables, query).await
            } else {
                backend
                    .run_sqlite_reader(move |backend, conn| {
                        load_rcode_distribution_on_connection(backend, conn, query)
                    })
                    .await
            }
        })
        .await
        {
            Ok(response) => {
                store_cache(
                    &self.api_cache,
                    CACHE_STATS_RCODE,
                    cache_identity,
                    CacheLifetime::Stats,
                    cache_generation,
                    &response,
                )
                .await;
                json_ok(StatusCode::OK, &response)
            }
            Err(err) => reader_error_response(err, "query_recorder_rcode_failed"),
        }
    }
}

#[async_trait]
impl ApiHandler for LatencyHandler {
    async fn handle(&self, request: Request<Bytes>) -> crate::api::ApiResponse {
        let cache_identity = request.uri().query().unwrap_or("");
        let cache_generation = match lookup_cache::<LatencySummary>(
            &self.api_cache,
            CACHE_STATS_LATENCY,
            cache_identity,
        )
        .await
        {
            CacheLookup::Hit(response) => return json_ok(StatusCode::OK, &response),
            CacheLookup::Miss(generation) => Some(generation),
            CacheLookup::Bypass => None,
        };
        let query = match parse_latency_query(request.uri().query()) {
            Ok(query) => query,
            Err(err) => return json_error(StatusCode::BAD_REQUEST, "invalid_query", err),
        };
        let backend = self.backend.clone();
        match run_reader_query(backend, move |backend| async move {
            if let Some(remote) = backend.remote_pool().cloned() {
                remote.load_latency_summary(&backend.tables, query).await
            } else {
                backend
                    .run_sqlite_reader(move |backend, conn| {
                        load_latency_summary_on_connection(backend, conn, query)
                    })
                    .await
            }
        })
        .await
        {
            Ok(response) => {
                store_cache(
                    &self.api_cache,
                    CACHE_STATS_LATENCY,
                    cache_identity,
                    CacheLifetime::Stats,
                    cache_generation,
                    &response,
                )
                .await;
                json_ok(StatusCode::OK, &response)
            }
            Err(err) => reader_error_response(err, "query_recorder_latency_failed"),
        }
    }
}

#[async_trait]
impl ApiHandler for TimeseriesHandler {
    async fn handle(&self, request: Request<Bytes>) -> crate::api::ApiResponse {
        let cache_identity = request.uri().query().unwrap_or("");
        let cache_generation = match lookup_cache::<TimeseriesResponse>(
            &self.api_cache,
            CACHE_STATS_TIMESERIES,
            cache_identity,
        )
        .await
        {
            CacheLookup::Hit(response) => return json_ok(StatusCode::OK, &response),
            CacheLookup::Miss(generation) => Some(generation),
            CacheLookup::Bypass => None,
        };
        let query = match parse_timeseries_query(request.uri().query()) {
            Ok(query) => query,
            Err(err) => return json_error(StatusCode::BAD_REQUEST, "invalid_query", err),
        };
        let backend = self.backend.clone();
        match run_reader_query(backend, move |backend| async move {
            if let Some(remote) = backend.remote_pool().cloned() {
                remote.load_timeseries(&backend.tables, query).await
            } else {
                backend
                    .run_sqlite_reader(move |backend, conn| {
                        load_timeseries_on_connection(backend, conn, query)
                    })
                    .await
            }
        })
        .await
        {
            Ok(response) => {
                store_cache(
                    &self.api_cache,
                    CACHE_STATS_TIMESERIES,
                    cache_identity,
                    CacheLifetime::Stats,
                    cache_generation,
                    &response,
                )
                .await;
                json_ok(StatusCode::OK, &response)
            }
            Err(err) => reader_error_response(err, "query_recorder_timeseries_failed"),
        }
    }
}

pub(super) fn parse_list_query(query: Option<&str>) -> std::result::Result<ListQuery, String> {
    let mut cursor = None;
    let mut limit = DEFAULT_LIST_LIMIT;
    let mut since_ms = None;
    let mut until_ms = None;
    let mut filter = QueryRecordFilter::default();

    visit_query_params(query, |key, value| {
        match key {
            "cursor" => cursor = Some(parse_cursor(value)?),
            "limit" => limit = parse_limit(value)?,
            "since_ms" => since_ms = Some(parse_u64_query("since_ms", value)?),
            "until_ms" => until_ms = Some(parse_u64_query("until_ms", value)?),
            "search" => filter.search = optional_text(value),
            "qname" => filter.qname = optional_text(value),
            "qtype" => filter.qtype = optional_upper_text(value),
            "client_ip" => filter.client_ip = optional_text(value),
            "rcode" => filter.rcode = optional_upper_text(value),
            "status" => {
                if let Some(value) = optional_text(value) {
                    filter.status = QueryRecordStatus::parse(value.as_str())?;
                }
            }
            "matcher_tag" => filter.matcher_tag = optional_text(value),
            _ => {}
        }
        Ok(())
    })?;

    Ok(ListQuery {
        cursor,
        limit,
        since_ms,
        until_ms,
        filter,
    })
}

pub(super) fn parse_plugins_stats_query(
    query: Option<&str>,
) -> std::result::Result<PluginsStatsQuery, String> {
    let mut since_ms = None;
    let mut until_ms = None;
    let mut kind = PluginStatsKind::All;
    let mut filter = QueryRecordFilter::default();
    visit_query_params(query, |key, value| {
        match key {
            "since_ms" => since_ms = Some(parse_u64_query("since_ms", value)?),
            "until_ms" => until_ms = Some(parse_u64_query("until_ms", value)?),
            "kind" => kind = PluginStatsKind::parse(value)?,
            "search" => filter.search = optional_text(value),
            "qname" => filter.qname = optional_text(value),
            "qtype" => filter.qtype = optional_upper_text(value),
            "client_ip" => filter.client_ip = optional_text(value),
            "rcode" => filter.rcode = optional_upper_text(value),
            "status" => {
                if let Some(value) = optional_text(value) {
                    filter.status = QueryRecordStatus::parse(value.as_str())?;
                }
            }
            "matcher_tag" => filter.matcher_tag = optional_text(value),
            _ => {}
        }
        Ok(())
    })?;
    Ok(PluginsStatsQuery {
        since_ms,
        until_ms,
        kind,
        filter,
    })
}

pub(super) fn parse_top_query(query: Option<&str>) -> std::result::Result<TopQuery, String> {
    let mut since_ms = None;
    let mut until_ms = None;
    let mut limit = DEFAULT_TOP_LIMIT;
    let mut filter = QueryRecordFilter::default();
    visit_query_params(query, |key, value| {
        match key {
            "since_ms" => since_ms = Some(parse_u64_query("since_ms", value)?),
            "until_ms" => until_ms = Some(parse_u64_query("until_ms", value)?),
            "limit" => limit = parse_top_limit(value)?,
            other => apply_filter_param(&mut filter, other, value)?,
        }
        Ok(())
    })?;
    Ok(TopQuery {
        since_ms,
        until_ms,
        filter,
        limit,
    })
}

pub(super) fn parse_distribution_query(
    query: Option<&str>,
) -> std::result::Result<DistributionQuery, String> {
    let mut since_ms = None;
    let mut until_ms = None;
    let mut filter = QueryRecordFilter::default();
    visit_query_params(query, |key, value| {
        match key {
            "since_ms" => since_ms = Some(parse_u64_query("since_ms", value)?),
            "until_ms" => until_ms = Some(parse_u64_query("until_ms", value)?),
            other => apply_filter_param(&mut filter, other, value)?,
        }
        Ok(())
    })?;
    Ok(DistributionQuery {
        since_ms,
        until_ms,
        filter,
    })
}

pub(super) fn parse_latency_query(
    query: Option<&str>,
) -> std::result::Result<LatencyQuery, String> {
    let mut since_ms = None;
    let mut until_ms = None;
    let mut slow_limit = DEFAULT_SLOW_LIMIT;
    let mut filter = QueryRecordFilter::default();
    visit_query_params(query, |key, value| {
        match key {
            "since_ms" => since_ms = Some(parse_u64_query("since_ms", value)?),
            "until_ms" => until_ms = Some(parse_u64_query("until_ms", value)?),
            "slow_limit" | "limit" => slow_limit = parse_top_limit(value)?,
            other => apply_filter_param(&mut filter, other, value)?,
        }
        Ok(())
    })?;
    Ok(LatencyQuery {
        since_ms,
        until_ms,
        filter,
        slow_limit,
    })
}

pub(super) fn parse_timeseries_query(
    query: Option<&str>,
) -> std::result::Result<TimeseriesQuery, String> {
    let mut since_ms = None;
    let mut until_ms = None;
    let mut bucket = TimeseriesBucket::Minute;
    let mut max_buckets = DEFAULT_TIMESERIES_BUCKETS;
    let mut filter = QueryRecordFilter::default();
    visit_query_params(query, |key, value| {
        match key {
            "since_ms" => since_ms = Some(parse_u64_query("since_ms", value)?),
            "until_ms" => until_ms = Some(parse_u64_query("until_ms", value)?),
            "bucket" => bucket = TimeseriesBucket::parse(value)?,
            "buckets" => max_buckets = parse_timeseries_buckets(value)?,
            other => apply_filter_param(&mut filter, other, value)?,
        }
        Ok(())
    })?;
    Ok(TimeseriesQuery {
        since_ms,
        until_ms,
        filter,
        bucket,
        max_buckets,
    })
}

fn apply_filter_param(
    filter: &mut QueryRecordFilter,
    key: &str,
    value: &str,
) -> std::result::Result<(), String> {
    match key {
        "search" => filter.search = optional_text(value),
        "qname" => filter.qname = optional_text(value),
        "qtype" => filter.qtype = optional_upper_text(value),
        "client_ip" => filter.client_ip = optional_text(value),
        "rcode" => filter.rcode = optional_upper_text(value),
        "status" => {
            if let Some(value) = optional_text(value) {
                filter.status = QueryRecordStatus::parse(value.as_str())?;
            }
        }
        "matcher_tag" => filter.matcher_tag = optional_text(value),
        _ => {}
    }
    Ok(())
}

fn parse_top_limit(raw: &str) -> std::result::Result<usize, String> {
    let parsed = parse_usize_param(raw, |err| format!("invalid limit query parameter: {err}"))?;
    if parsed == 0 {
        return Err("limit must be greater than 0".to_string());
    }
    let max_sql_limit = usize::try_from(i64::MAX).unwrap_or(usize::MAX);
    if parsed > max_sql_limit {
        return Err(format!(
            "limit must be less than or equal to {max_sql_limit}"
        ));
    }
    Ok(parsed)
}

fn parse_timeseries_buckets(raw: &str) -> std::result::Result<usize, String> {
    let parsed = parse_usize_param(raw, |err| format!("invalid buckets query parameter: {err}"))?;
    if parsed == 0 {
        return Err("buckets must be greater than 0".to_string());
    }
    Ok(parsed.min(MAX_TIMESERIES_BUCKETS))
}

impl TimeseriesBucket {
    fn parse(raw: &str) -> std::result::Result<Self, String> {
        match raw {
            "minute" => Ok(Self::Minute),
            "hour" => Ok(Self::Hour),
            _ => Err("bucket must be one of minute, hour".to_string()),
        }
    }
}

fn parse_tail_param(query: Option<&str>, max_tail: usize) -> std::result::Result<usize, String> {
    let mut tail = 0usize;
    visit_query_params(query, |key, value| {
        if key == "tail" {
            let requested =
                parse_usize_param(value, |err| format!("invalid tail query parameter: {err}"))?;
            tail = requested.min(max_tail);
        }
        Ok(())
    })?;
    Ok(tail)
}

fn parse_cursor(raw: &str) -> std::result::Result<ListCursor, String> {
    let (created_at_ms, id) = raw
        .split_once(':')
        .ok_or_else(|| "cursor must be formatted as <created_at_ms>:<id>".to_string())?;
    Ok(ListCursor {
        created_at_ms: created_at_ms
            .parse::<i64>()
            .map_err(|err| format!("invalid cursor created_at_ms: {err}"))?,
        id: id
            .parse::<i64>()
            .map_err(|err| format!("invalid cursor id: {err}"))?,
    })
}

fn parse_limit(raw: &str) -> std::result::Result<usize, String> {
    let limit = parse_usize_param(raw, |err| format!("invalid limit query parameter: {err}"))?;
    if limit == 0 {
        return Err("limit must be greater than 0".to_string());
    }
    Ok(limit.min(MAX_LIST_LIMIT))
}

fn parse_u64_query(field: &str, raw: &str) -> std::result::Result<u64, String> {
    parse_u64_param(raw, |err| format!("invalid {field} query parameter: {err}"))
}

impl PluginStatsKind {
    fn parse(raw: &str) -> std::result::Result<Self, String> {
        match raw {
            "matcher" => Ok(Self::Matcher),
            "executor" => Ok(Self::Executor),
            "builtin" => Ok(Self::Builtin),
            "all" => Ok(Self::All),
            _ => Err("kind must be one of matcher, executor, builtin, all".to_string()),
        }
    }
}

impl QueryRecordStatus {
    fn parse(raw: &str) -> std::result::Result<Self, String> {
        match raw {
            "all" => Ok(Self::All),
            "error" => Ok(Self::Error),
            "has_response" => Ok(Self::HasResponse),
            "no_response" => Ok(Self::NoResponse),
            _ => Err("status must be one of all, error, has_response, no_response".to_string()),
        }
    }
}

fn sse_record_frame(record: &RecordDetail) -> Bytes {
    match serde_json::to_vec(record) {
        Ok(data) => {
            let mut frame = Vec::with_capacity(data.len() + 32);
            frame.extend_from_slice(b"event: record\ndata: ");
            frame.extend_from_slice(&data);
            frame.extend_from_slice(b"\n\n");
            Bytes::from(frame)
        }
        Err(err) => Bytes::from(format!(
            "event: error\ndata: {{\"message\":\"failed to serialize stream record: {}\"}}\n\n",
            err
        )),
    }
}

pub(super) fn register(
    backend: &Arc<RecorderBackend>,
    api_cache: Option<Arc<ApiCache>>,
) -> Result<()> {
    register_plugin_api!(
        &backend.tag,
        |plugin_api|
        GET "/records" => RecordsListHandler {
            backend: backend.clone(),
            api_cache: api_cache.clone(),
        },
        DELETE "/records" => RecordsClearHandler {
            backend: backend.clone(),
            api_cache: api_cache.clone(),
        },
        GET_PREFIX "/records/" => RecordDetailHandler {
            backend: backend.clone(),
            path_prefix: plugin_api.path("/records/")?,
            api_cache: api_cache.clone(),
        },
        GET "/stats/plugins" => StatsPluginsHandler {
            backend: backend.clone(),
            api_cache: api_cache.clone(),
        },
        GET "/stats/top_clients" => TopClientsHandler {
            backend: backend.clone(),
            api_cache: api_cache.clone(),
        },
        GET "/stats/top_qnames" => TopQnamesHandler {
            backend: backend.clone(),
            api_cache: api_cache.clone(),
        },
        GET "/stats/qtype" => QtypeDistributionHandler {
            backend: backend.clone(),
            api_cache: api_cache.clone(),
        },
        GET "/stats/rcode" => RcodeDistributionHandler {
            backend: backend.clone(),
            api_cache: api_cache.clone(),
        },
        GET "/stats/latency" => LatencyHandler {
            backend: backend.clone(),
            api_cache: api_cache.clone(),
        },
        GET "/stats/timeseries" => TimeseriesHandler {
            backend: backend.clone(),
            api_cache: api_cache.clone(),
        },
        GET "/stream" => StreamHandler {
            backend: backend.clone(),
        },
    )?;

    Ok(())
}
