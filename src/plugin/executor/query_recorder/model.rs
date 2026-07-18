// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;

use oxidns_next_proto::Message;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::api_cache::{QueryRecorderApiCacheConfig, ResolvedApiCacheConfig};
use crate::core::context::ExecutionPath;

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct QueryRecorderConfig {
    /// Legacy SQLite shorthand. Mutually exclusive with `database`.
    pub(super) path: Option<String>,
    pub(super) database: Option<RecorderDatabaseConfig>,
    pub(super) api_cache: Option<QueryRecorderApiCacheConfig>,
    pub(super) queue_size: Option<usize>,
    pub(super) batch_size: Option<usize>,
    pub(super) flush_interval_ms: Option<u64>,
    pub(super) memory_tail: Option<usize>,
    pub(super) retention_days: Option<u64>,
    pub(super) cleanup_interval_hours: Option<u64>,
    pub(super) reader_concurrency: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum RecorderDatabaseConfig {
    Sqlite {
        path: String,
        acquire_timeout_ms: Option<u64>,
        query_timeout_ms: Option<u64>,
    },
    #[serde(alias = "postgresql")]
    Postgres {
        url: String,
        max_connections: Option<u32>,
        connect_timeout_ms: Option<u64>,
        acquire_timeout_ms: Option<u64>,
        query_timeout_ms: Option<u64>,
    },
    Mysql {
        url: String,
        max_connections: Option<u32>,
        connect_timeout_ms: Option<u64>,
        acquire_timeout_ms: Option<u64>,
        query_timeout_ms: Option<u64>,
    },
}

#[derive(Clone)]
pub(super) enum ResolvedDatabaseConfig {
    Sqlite {
        path: PathBuf,
        acquire_timeout_ms: u64,
        query_timeout_ms: u64,
    },
    Postgres {
        url: String,
        max_connections: u32,
        connect_timeout_ms: u64,
        acquire_timeout_ms: u64,
        query_timeout_ms: u64,
    },
    Mysql {
        url: String,
        max_connections: u32,
        connect_timeout_ms: u64,
        acquire_timeout_ms: u64,
        query_timeout_ms: u64,
    },
}

impl fmt::Debug for ResolvedDatabaseConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite {
                path,
                acquire_timeout_ms,
                query_timeout_ms,
            } => formatter
                .debug_struct("Sqlite")
                .field("path", path)
                .field("acquire_timeout_ms", acquire_timeout_ms)
                .field("query_timeout_ms", query_timeout_ms)
                .finish(),
            Self::Postgres {
                max_connections,
                connect_timeout_ms,
                acquire_timeout_ms,
                query_timeout_ms,
                ..
            } => formatter
                .debug_struct("Postgres")
                .field("url", &"<redacted>")
                .field("max_connections", max_connections)
                .field("connect_timeout_ms", connect_timeout_ms)
                .field("acquire_timeout_ms", acquire_timeout_ms)
                .field("query_timeout_ms", query_timeout_ms)
                .finish(),
            Self::Mysql {
                max_connections,
                connect_timeout_ms,
                acquire_timeout_ms,
                query_timeout_ms,
                ..
            } => formatter
                .debug_struct("Mysql")
                .field("url", &"<redacted>")
                .field("max_connections", max_connections)
                .field("connect_timeout_ms", connect_timeout_ms)
                .field("acquire_timeout_ms", acquire_timeout_ms)
                .field("query_timeout_ms", query_timeout_ms)
                .finish(),
        }
    }
}

impl ResolvedDatabaseConfig {
    /// Returns a one-way identity used to isolate disposable API cache keys.
    ///
    /// Connection URLs can contain credentials, so callers must never expose
    /// the unhashed identity in Redis keys, logs, or API responses.
    pub(super) fn cache_identity_digest(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"query-recorder-database:v1\0");
        match self {
            Self::Sqlite { path, .. } => {
                hasher.update(b"sqlite\0");
                hasher.update(path.as_os_str().to_string_lossy().as_bytes());
            }
            Self::Postgres { url, .. } => {
                hasher.update(b"postgres\0");
                hasher.update(redacted_database_url_identity(url).as_bytes());
            }
            Self::Mysql { url, .. } => {
                hasher.update(b"mysql\0");
                hasher.update(redacted_database_url_identity(url).as_bytes());
            }
        }
        hex::encode(hasher.finalize())
    }

    pub(super) fn acquire_timeout_ms(&self) -> u64 {
        match self {
            Self::Sqlite {
                acquire_timeout_ms, ..
            }
            | Self::Postgres {
                acquire_timeout_ms, ..
            }
            | Self::Mysql {
                acquire_timeout_ms, ..
            } => *acquire_timeout_ms,
        }
    }

    pub(super) fn query_timeout_ms(&self) -> u64 {
        match self {
            Self::Sqlite {
                query_timeout_ms, ..
            }
            | Self::Postgres {
                query_timeout_ms, ..
            }
            | Self::Mysql {
                query_timeout_ms, ..
            } => *query_timeout_ms,
        }
    }
}

fn redacted_database_url_identity(raw: &str) -> String {
    let mut parsed = url::Url::parse(raw)
        .expect("query_recorder database URL was validated before identity construction");
    let _ = parsed.set_password(None);
    parsed.set_fragment(None);

    let query = parsed
        .query_pairs()
        .map(|(key, value)| {
            let value = if is_sensitive_database_query_key(&key) {
                "<redacted>".to_string()
            } else {
                value.into_owned()
            };
            (key.into_owned(), value)
        })
        .collect::<Vec<_>>();
    if query.is_empty() {
        parsed.set_query(None);
    } else {
        let mut serializer = url::form_urlencoded::Serializer::new(String::new());
        for (key, value) in query {
            serializer.append_pair(&key, &value);
        }
        parsed.set_query(Some(&serializer.finish()));
    }
    parsed.to_string()
}

fn is_sensitive_database_query_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("password") || key.contains("secret") || key.contains("token")
}

#[derive(Debug, Clone)]
pub(super) struct ResolvedRecorderConfig {
    pub(super) database: ResolvedDatabaseConfig,
    pub(super) api_cache: Option<ResolvedApiCacheConfig>,
    pub(super) queue_size: usize,
    pub(super) batch_size: usize,
    pub(super) flush_interval_ms: u64,
    pub(super) memory_tail: usize,
    pub(super) retention_days: u64,
    pub(super) cleanup_interval_hours: u64,
    pub(super) reader_concurrency: usize,
}

#[derive(Debug, Clone)]
pub(super) struct TableNames {
    pub(super) records: String,
    pub(super) steps: String,
    pub(super) questions: String,
    pub(super) meta: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct QuestionJson {
    pub(super) name: String,
    pub(super) qtype: String,
    pub(super) qclass: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(super) struct RecordJson {
    pub(super) name: String,
    pub(super) class: String,
    pub(super) ttl: u32,
    pub(super) rr_type: String,
    pub(super) payload_kind: String,
    pub(super) payload_text: String,
    pub(super) payload: Value,
}

/// Bounded response projection returned by the history-list endpoint.
///
/// Full DNS records remain available from the detail endpoint. Keeping only a
/// few short presentation values here prevents list requests from decoding the
/// potentially large `answers_json` column for every row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct AnswerPreviewJson {
    pub(super) name: String,
    pub(super) rr_type: String,
    pub(super) payload_text: String,
    pub(super) truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(super) struct EdnsOptionJson {
    pub(super) code: u16,
    pub(super) name: String,
    pub(super) payload_kind: String,
    pub(super) payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(super) struct EdnsJson {
    pub(super) udp_payload_size: u16,
    pub(super) ext_rcode: u8,
    pub(super) version: u8,
    pub(super) dnssec_ok: bool,
    pub(super) z: u16,
    pub(super) options: Vec<EdnsOptionJson>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct StepJson {
    pub(super) event_index: usize,
    pub(super) sequence_tag: String,
    pub(super) node_index: Option<usize>,
    pub(super) kind: String,
    pub(super) tag: Option<String>,
    pub(super) outcome: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(super) struct RecordRow {
    pub(super) id: i64,
    pub(super) created_at_ms: i64,
    pub(super) elapsed_ms: u64,
    pub(super) request_id: u16,
    pub(super) client_ip: String,
    pub(super) questions_json: Vec<QuestionJson>,
    pub(super) req_rd: bool,
    pub(super) req_cd: bool,
    pub(super) req_ad: bool,
    pub(super) req_opcode: String,
    pub(super) req_edns_json: Option<EdnsJson>,
    pub(super) error: Option<String>,
    pub(super) has_response: bool,
    pub(super) rcode: Option<String>,
    pub(super) resp_aa: Option<bool>,
    pub(super) resp_tc: Option<bool>,
    pub(super) resp_ra: Option<bool>,
    pub(super) resp_ad: Option<bool>,
    pub(super) resp_cd: Option<bool>,
    pub(super) answer_count: u32,
    pub(super) authority_count: u32,
    pub(super) additional_count: u32,
    pub(super) answer_preview: Vec<AnswerPreviewJson>,
    pub(super) answers_json: Vec<RecordJson>,
    pub(super) authorities_json: Vec<RecordJson>,
    pub(super) additionals_json: Vec<RecordJson>,
    pub(super) signature_json: Vec<RecordJson>,
    pub(super) resp_edns_json: Option<EdnsJson>,
}

/// Compact history-list projection. Full request/response sections are loaded
/// only by the record-detail endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(super) struct RecordSummaryRow {
    pub(super) id: i64,
    pub(super) created_at_ms: i64,
    pub(super) elapsed_ms: u64,
    pub(super) request_id: u16,
    pub(super) client_ip: String,
    pub(super) questions_json: Vec<QuestionJson>,
    pub(super) error: Option<String>,
    pub(super) has_response: bool,
    pub(super) rcode: Option<String>,
    pub(super) answer_count: u32,
    pub(super) authority_count: u32,
    pub(super) additional_count: u32,
    pub(super) answer_preview: Vec<AnswerPreviewJson>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(super) struct RecordDetail {
    #[serde(flatten)]
    pub(super) record: RecordRow,
    pub(super) steps: Vec<StepJson>,
}

#[derive(Debug, Clone)]
pub(super) struct PendingRecord {
    pub(super) request: Message,
    pub(super) response: Option<Message>,
    pub(super) created_at_ms: i64,
    pub(super) elapsed_ms: u64,
    pub(super) exec_path: ExecutionPath,
    pub(super) step_start_index: usize,
    pub(super) client_ip: SocketAddr,
    pub(super) error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct PluginStatsRow {
    pub(super) kind: String,
    pub(super) tag: Option<String>,
    pub(super) checked: u64,
    pub(super) matched: u64,
    pub(super) executed: u64,
    pub(super) query_total: u64,
    pub(super) query_share: f64,
}

#[derive(Debug, Clone)]
pub(super) struct ListQuery {
    pub(super) cursor: Option<ListCursor>,
    pub(super) limit: usize,
    pub(super) since_ms: Option<u64>,
    pub(super) until_ms: Option<u64>,
    pub(super) filter: QueryRecordFilter,
}

#[derive(Debug, Clone)]
pub(super) struct PluginsStatsQuery {
    pub(super) since_ms: Option<u64>,
    pub(super) until_ms: Option<u64>,
    pub(super) kind: PluginStatsKind,
    pub(super) filter: QueryRecordFilter,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct QueryRecordFilter {
    /// Unified operator search across question names and client IP addresses.
    pub(super) search: Option<String>,
    pub(super) qname: Option<String>,
    pub(super) qtype: Option<String>,
    pub(super) client_ip: Option<String>,
    pub(super) rcode: Option<String>,
    pub(super) status: QueryRecordStatus,
    pub(super) matcher_tag: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum QueryRecordStatus {
    #[default]
    All,
    Error,
    HasResponse,
    NoResponse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ListCursor {
    pub(super) created_at_ms: i64,
    pub(super) id: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PluginStatsKind {
    Matcher,
    Executor,
    Builtin,
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct TopBucketRow {
    pub(super) key: String,
    pub(super) count: u64,
    pub(super) share: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct TopBucketsResponse {
    pub(super) ok: bool,
    pub(super) sample_size: u64,
    pub(super) rows: Vec<TopBucketRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct DistributionRow {
    pub(super) key: String,
    pub(super) count: u64,
    pub(super) share: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct DistributionResponse {
    pub(super) ok: bool,
    pub(super) sample_size: u64,
    pub(super) rows: Vec<DistributionRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct LatencyHistogramBucket {
    pub(super) lt_ms: Option<u64>,
    pub(super) count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct LatencySlowRow {
    pub(super) qname: String,
    pub(super) count: u64,
    pub(super) avg_ms: f64,
    pub(super) max_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct LatencySummary {
    pub(super) ok: bool,
    pub(super) sample_size: u64,
    pub(super) avg_ms: f64,
    pub(super) p50_ms: u64,
    pub(super) p95_ms: u64,
    pub(super) p99_ms: u64,
    pub(super) max_ms: u64,
    pub(super) histogram: Vec<LatencyHistogramBucket>,
    pub(super) slow_top: Vec<LatencySlowRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct TimeseriesPoint {
    pub(super) bucket_ms: i64,
    pub(super) total: u64,
    pub(super) error_count: u64,
    pub(super) no_response_count: u64,
    pub(super) avg_ms: f64,
    pub(super) p95_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct TimeseriesResponse {
    pub(super) ok: bool,
    pub(super) sample_size: u64,
    pub(super) bucket_ms: i64,
    pub(super) points: Vec<TimeseriesPoint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TimeseriesBucket {
    Minute,
    Hour,
}

impl TimeseriesBucket {
    pub(super) fn millis(self) -> i64 {
        match self {
            Self::Minute => 60_000,
            Self::Hour => 3_600_000,
        }
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;

    fn postgres(url: &str) -> ResolvedDatabaseConfig {
        ResolvedDatabaseConfig::Postgres {
            url: url.to_string(),
            max_connections: 8,
            connect_timeout_ms: 5_000,
            acquire_timeout_ms: 3_000,
            query_timeout_ms: 20_000,
        }
    }

    #[test]
    fn cache_database_identity_excludes_credentials_but_keeps_dataset() {
        let first = postgres(
            "postgres://reader:first-password@db.example/one?sslpassword=first-secret&options=-csearch_path%3Dlogs",
        );
        let rotated = postgres(
            "postgres://reader:second-password@db.example/one?sslpassword=second-secret&options=-csearch_path%3Dlogs",
        );
        let other_database = postgres(
            "postgres://reader:first-password@db.example/two?sslpassword=first-secret&options=-csearch_path%3Dlogs",
        );

        assert_eq!(
            first.cache_identity_digest(),
            rotated.cache_identity_digest(),
            "credential rotation must not change or disclose the cache identity"
        );
        assert_ne!(
            first.cache_identity_digest(),
            other_database.cache_identity_digest(),
            "different datasets must use isolated API cache keys"
        );
    }
}

#[derive(Debug, Clone)]
pub(super) struct TopQuery {
    pub(super) since_ms: Option<u64>,
    pub(super) until_ms: Option<u64>,
    pub(super) filter: QueryRecordFilter,
    pub(super) limit: usize,
}

#[derive(Debug, Clone)]
pub(super) struct DistributionQuery {
    pub(super) since_ms: Option<u64>,
    pub(super) until_ms: Option<u64>,
    pub(super) filter: QueryRecordFilter,
}

#[derive(Debug, Clone)]
pub(super) struct LatencyQuery {
    pub(super) since_ms: Option<u64>,
    pub(super) until_ms: Option<u64>,
    pub(super) filter: QueryRecordFilter,
    pub(super) slow_limit: usize,
}

#[derive(Debug, Clone)]
pub(super) struct TimeseriesQuery {
    pub(super) since_ms: Option<u64>,
    pub(super) until_ms: Option<u64>,
    pub(super) filter: QueryRecordFilter,
    pub(super) bucket: TimeseriesBucket,
    pub(super) max_buckets: usize,
}
