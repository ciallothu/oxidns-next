// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! PostgreSQL and MySQL persistence for `query_recorder`.
//!
//! The remote stores intentionally keep JSON payloads as text and normalized
//! filter columns as ordinary scalar columns. This preserves the SQLite API
//! contract while keeping the query plans portable and predictable.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Serialize;
use sqlx::mysql::{MySqlPool, MySqlPoolOptions, MySqlRow};
use sqlx::postgres::{PgPool, PgPoolOptions, PgRow};
use sqlx::{Executor, Row};

use super::model::{
    DistributionQuery, DistributionResponse, DistributionRow, LatencyHistogramBucket, LatencyQuery,
    LatencySlowRow, LatencySummary, ListCursor, ListQuery, PendingRecord, PluginStatsKind,
    PluginStatsRow, PluginsStatsQuery, QueryRecordFilter, QueryRecordStatus, RecordDetail,
    RecordRow, RecordSummaryRow, ResolvedDatabaseConfig, StepJson, TableNames, TimeseriesPoint,
    TimeseriesQuery, TimeseriesResponse, TopBucketRow, TopBucketsResponse, TopQuery,
};
use super::store::{
    bucket_floor, bucket_share, latency_histogram, latency_percentiles, percentile_value,
};
use crate::infra::error::{DnsError, Result};

const CLEANUP_BATCH_SIZE: i64 = 1_000;
const STATS_SAMPLE_LIMIT: i64 = 10_000;

#[derive(Debug, Clone)]
pub(super) enum RemotePool {
    Postgres(PgPool),
    Mysql(MySqlPool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoteDialect {
    Postgres,
    Mysql,
}

impl RemoteDialect {
    fn sample_cte(self) -> &'static str {
        match self {
            Self::Postgres => "AS MATERIALIZED",
            Self::Mysql => "AS",
        }
    }

    fn signed_count(self, expression: &str) -> String {
        match self {
            Self::Postgres => format!("CAST({expression} AS BIGINT)"),
            Self::Mysql => format!("CAST({expression} AS SIGNED)"),
        }
    }

    fn float_average(self, expression: &str) -> String {
        match self {
            Self::Postgres => format!("CAST(AVG({expression}) AS DOUBLE PRECISION)"),
            Self::Mysql => format!("CAST(AVG({expression}) AS DOUBLE)"),
        }
    }
}

impl RemotePool {
    pub(super) async fn connect(config: &ResolvedDatabaseConfig) -> Result<Self> {
        match config {
            ResolvedDatabaseConfig::Postgres {
                url,
                max_connections,
                connect_timeout_ms,
                acquire_timeout_ms,
                ..
            } => {
                let connect = PgPoolOptions::new()
                    .max_connections(*max_connections)
                    .acquire_timeout(Duration::from_millis(*acquire_timeout_ms))
                    .connect(url);
                let pool =
                    tokio::time::timeout(Duration::from_millis(*connect_timeout_ms), connect)
                        .await
                        .map_err(|_| {
                            DnsError::runtime("query_recorder PostgreSQL connect timed out")
                        })??;
                Ok(Self::Postgres(pool))
            }
            ResolvedDatabaseConfig::Mysql {
                url,
                max_connections,
                connect_timeout_ms,
                acquire_timeout_ms,
                ..
            } => {
                let connect = MySqlPoolOptions::new()
                    .max_connections(*max_connections)
                    .acquire_timeout(Duration::from_millis(*acquire_timeout_ms))
                    .connect(url);
                let pool =
                    tokio::time::timeout(Duration::from_millis(*connect_timeout_ms), connect)
                        .await
                        .map_err(|_| {
                            DnsError::runtime("query_recorder MySQL connect timed out")
                        })??;
                Ok(Self::Mysql(pool))
            }
            ResolvedDatabaseConfig::Sqlite { .. } => Err(DnsError::runtime(
                "query_recorder remote pool requested for SQLite",
            )),
        }
    }

    fn dialect(&self) -> RemoteDialect {
        match self {
            Self::Postgres(_) => RemoteDialect::Postgres,
            Self::Mysql(_) => RemoteDialect::Mysql,
        }
    }

    pub(super) async fn create_schema(&self, tables: &TableNames) -> Result<()> {
        match self {
            Self::Postgres(pool) => create_postgres_schema(pool, tables).await,
            Self::Mysql(pool) => create_mysql_schema(pool, tables).await,
        }
    }

    pub(super) async fn insert_batch(
        &self,
        tables: &TableNames,
        pending: Vec<PendingRecord>,
    ) -> Result<Vec<RecordDetail>> {
        match self {
            Self::Postgres(pool) => insert_postgres_batch(pool, tables, pending).await,
            Self::Mysql(pool) => insert_mysql_batch(pool, tables, pending).await,
        }
    }

    pub(super) async fn cleanup(&self, tables: &TableNames, cutoff_ms: i64) -> Result<()> {
        match self {
            Self::Postgres(pool) => cleanup_postgres(pool, tables, cutoff_ms).await,
            Self::Mysql(pool) => cleanup_mysql(pool, tables, cutoff_ms).await,
        }
    }

    pub(super) async fn clear_history(&self, tables: &TableNames) -> Result<usize> {
        let sql = format!("DELETE FROM {}", tables.records);
        let affected = match self {
            Self::Postgres(pool) => sqlx::query(&sql).execute(pool).await?.rows_affected(),
            Self::Mysql(pool) => sqlx::query(&sql).execute(pool).await?.rows_affected(),
        };
        usize::try_from(affected)
            .map_err(|_| DnsError::runtime("query_recorder cleared record count overflow"))
    }

    pub(super) async fn close(&self) {
        match self {
            Self::Postgres(pool) => pool.close().await,
            Self::Mysql(pool) => pool.close().await,
        }
    }
}

async fn create_postgres_schema(pool: &PgPool, tables: &TableNames) -> Result<()> {
    let statements = [
        format!(
            "CREATE TABLE IF NOT EXISTS {records} (
                id BIGINT GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY,
                created_at_ms BIGINT NOT NULL,
                elapsed_ms BIGINT NOT NULL,
                request_id INTEGER NOT NULL,
                client_ip VARCHAR(45) NOT NULL,
                questions_json TEXT NOT NULL,
                req_rd INTEGER NOT NULL,
                req_cd INTEGER NOT NULL,
                req_ad INTEGER NOT NULL,
                req_opcode VARCHAR(32) NOT NULL,
                req_edns_json TEXT NULL,
                error TEXT NULL,
                has_response INTEGER NOT NULL,
                rcode VARCHAR(64) NULL,
                resp_aa INTEGER NULL,
                resp_tc INTEGER NULL,
                resp_ra INTEGER NULL,
                resp_ad INTEGER NULL,
                resp_cd INTEGER NULL,
                answer_count BIGINT NOT NULL,
                authority_count BIGINT NOT NULL,
                additional_count BIGINT NOT NULL,
                answer_preview_json TEXT NOT NULL DEFAULT '[]',
                answers_json TEXT NOT NULL,
                authorities_json TEXT NOT NULL,
                additionals_json TEXT NOT NULL,
                signature_json TEXT NOT NULL,
                resp_edns_json TEXT NULL
            )",
            records = tables.records
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS {steps} (
                record_id BIGINT NOT NULL REFERENCES {records}(id) ON DELETE CASCADE,
                event_index BIGINT NOT NULL,
                sequence_tag VARCHAR(255) NOT NULL,
                node_index BIGINT NULL,
                kind VARCHAR(32) NOT NULL,
                tag VARCHAR(255) NULL,
                outcome VARCHAR(64) NOT NULL,
                PRIMARY KEY (record_id, event_index)
            )",
            steps = tables.steps,
            records = tables.records
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS {questions} (
                record_id BIGINT NOT NULL REFERENCES {records}(id) ON DELETE CASCADE,
                question_index BIGINT NOT NULL,
                name_lc VARCHAR(255) NOT NULL,
                qtype VARCHAR(32) NOT NULL,
                qclass VARCHAR(32) NOT NULL,
                PRIMARY KEY (record_id, question_index)
            )",
            questions = tables.questions,
            records = tables.records
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS {} (meta_key VARCHAR(255) PRIMARY KEY, meta_value TEXT NOT NULL)",
            tables.meta
        ),
        format!(
            "CREATE INDEX IF NOT EXISTS {records}_created_at_id_idx ON {records}(created_at_ms DESC, id DESC)",
            records = tables.records
        ),
        format!(
            "CREATE INDEX IF NOT EXISTS {records}_client_ip_idx ON {records}(client_ip)",
            records = tables.records
        ),
        format!(
            "CREATE INDEX IF NOT EXISTS {records}_rcode_idx ON {records}(rcode)",
            records = tables.records
        ),
        format!(
            "CREATE INDEX IF NOT EXISTS {questions}_name_idx ON {questions}(name_lc, record_id)",
            questions = tables.questions
        ),
        format!(
            "CREATE INDEX IF NOT EXISTS {questions}_qtype_idx ON {questions}(qtype, record_id)",
            questions = tables.questions
        ),
        format!(
            "CREATE INDEX IF NOT EXISTS {steps}_matcher_lookup_idx ON {steps}(kind, tag, outcome, record_id)",
            steps = tables.steps
        ),
        format!(
            "CREATE INDEX IF NOT EXISTS {steps}_record_kind_idx ON {steps}(record_id, kind)",
            steps = tables.steps
        ),
    ];
    for statement in statements {
        pool.execute(statement.as_str()).await?;
    }
    Ok(())
}

async fn create_mysql_schema(pool: &MySqlPool, tables: &TableNames) -> Result<()> {
    let statements = [
        format!(
            "CREATE TABLE IF NOT EXISTS {records} (
                id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
                created_at_ms BIGINT NOT NULL,
                elapsed_ms BIGINT NOT NULL,
                request_id INT NOT NULL,
                client_ip VARCHAR(45) NOT NULL,
                questions_json LONGTEXT NOT NULL,
                req_rd INT NOT NULL,
                req_cd INT NOT NULL,
                req_ad INT NOT NULL,
                req_opcode VARCHAR(32) NOT NULL,
                req_edns_json LONGTEXT NULL,
                error TEXT NULL,
                has_response INT NOT NULL,
                rcode VARCHAR(64) NULL,
                resp_aa INT NULL,
                resp_tc INT NULL,
                resp_ra INT NULL,
                resp_ad INT NULL,
                resp_cd INT NULL,
                answer_count BIGINT NOT NULL,
                authority_count BIGINT NOT NULL,
                additional_count BIGINT NOT NULL,
                answer_preview_json LONGTEXT NOT NULL,
                answers_json LONGTEXT NOT NULL,
                authorities_json LONGTEXT NOT NULL,
                additionals_json LONGTEXT NOT NULL,
                signature_json LONGTEXT NOT NULL,
                resp_edns_json LONGTEXT NULL,
                INDEX created_at_id_idx (created_at_ms DESC, id DESC),
                INDEX client_ip_idx (client_ip),
                INDEX rcode_idx (rcode)
            ) ENGINE=InnoDB",
            records = tables.records
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS {steps} (
                record_id BIGINT NOT NULL,
                event_index BIGINT NOT NULL,
                sequence_tag VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
                node_index BIGINT NULL,
                kind VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
                tag VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NULL,
                outcome VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
                PRIMARY KEY (record_id, event_index),
                INDEX matcher_lookup_idx (kind, tag, outcome, record_id),
                INDEX record_kind_idx (record_id, kind),
                CONSTRAINT {steps}_record_fk FOREIGN KEY (record_id)
                    REFERENCES {records}(id) ON DELETE CASCADE
            ) ENGINE=InnoDB",
            steps = tables.steps,
            records = tables.records
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS {questions} (
                record_id BIGINT NOT NULL,
                question_index BIGINT NOT NULL,
                name_lc VARCHAR(255) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
                qtype VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
                qclass VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
                PRIMARY KEY (record_id, question_index),
                INDEX name_idx (name_lc, record_id),
                INDEX qtype_idx (qtype, record_id),
                CONSTRAINT {questions}_record_fk FOREIGN KEY (record_id)
                    REFERENCES {records}(id) ON DELETE CASCADE
            ) ENGINE=InnoDB",
            questions = tables.questions,
            records = tables.records
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS {} (meta_key VARCHAR(255) PRIMARY KEY, meta_value TEXT NOT NULL) ENGINE=InnoDB",
            tables.meta
        ),
    ];
    for statement in statements {
        pool.execute(statement.as_str()).await?;
    }
    Ok(())
}

async fn insert_postgres_batch(
    pool: &PgPool,
    tables: &TableNames,
    pending: Vec<PendingRecord>,
) -> Result<Vec<RecordDetail>> {
    let record_sql = format!(
        "INSERT INTO {} (
            created_at_ms, elapsed_ms, request_id, client_ip, questions_json,
            req_rd, req_cd, req_ad, req_opcode, req_edns_json, error,
            has_response, rcode, resp_aa, resp_tc, resp_ra, resp_ad, resp_cd,
            answer_count, authority_count, additional_count, answer_preview_json,
            answers_json, authorities_json, additionals_json, signature_json, resp_edns_json
        ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13,
            $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27
        ) RETURNING id",
        tables.records
    );
    let question_sql = format!(
        "INSERT INTO {} (record_id, question_index, name_lc, qtype, qclass)
         VALUES ($1, $2, $3, $4, $5)",
        tables.questions
    );
    let step_sql = format!(
        "INSERT INTO {} (record_id, event_index, sequence_tag, node_index, kind, tag, outcome)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        tables.steps
    );

    let mut tx = pool.begin().await?;
    let mut committed = Vec::with_capacity(pending.len());
    for pending_record in pending {
        let (record, steps) = pending_record.take_to_record();
        let json = SerializedRecord::new(&record)?;
        let row = sqlx::query(&record_sql)
            .bind(record.created_at_ms)
            .bind(i64_from_u64(record.elapsed_ms, "elapsed_ms")?)
            .bind(i32::from(record.request_id))
            .bind(record.client_ip.as_str())
            .bind(json.questions.as_str())
            .bind(bool_i32(record.req_rd))
            .bind(bool_i32(record.req_cd))
            .bind(bool_i32(record.req_ad))
            .bind(record.req_opcode.as_str())
            .bind(json.req_edns.as_deref())
            .bind(record.error.as_deref())
            .bind(bool_i32(record.has_response))
            .bind(record.rcode.as_deref())
            .bind(record.resp_aa.map(bool_i32))
            .bind(record.resp_tc.map(bool_i32))
            .bind(record.resp_ra.map(bool_i32))
            .bind(record.resp_ad.map(bool_i32))
            .bind(record.resp_cd.map(bool_i32))
            .bind(i64::from(record.answer_count))
            .bind(i64::from(record.authority_count))
            .bind(i64::from(record.additional_count))
            .bind(json.answer_preview.as_str())
            .bind(json.answers.as_str())
            .bind(json.authorities.as_str())
            .bind(json.additionals.as_str())
            .bind(json.signature.as_str())
            .bind(json.resp_edns.as_deref())
            .fetch_one(&mut *tx)
            .await?;
        let record_id: i64 = row.try_get(0)?;

        for (index, question) in record.questions_json.iter().enumerate() {
            sqlx::query(&question_sql)
                .bind(record_id)
                .bind(i64_from_usize(index, "question_index")?)
                .bind(question.name.to_ascii_lowercase())
                .bind(question.qtype.to_ascii_uppercase())
                .bind(question.qclass.as_str())
                .execute(&mut *tx)
                .await?;
        }
        for step in &steps {
            sqlx::query(&step_sql)
                .bind(record_id)
                .bind(i64_from_usize(step.event_index, "event_index")?)
                .bind(step.sequence_tag.as_str())
                .bind(
                    step.node_index
                        .map(|value| i64_from_usize(value, "node_index"))
                        .transpose()?,
                )
                .bind(step.kind.as_str())
                .bind(step.tag.as_deref())
                .bind(step.outcome.as_str())
                .execute(&mut *tx)
                .await?;
        }
        committed.push(RecordDetail {
            record: RecordRow {
                id: record_id,
                ..record
            },
            steps,
        });
    }
    tx.commit().await?;
    Ok(committed)
}

async fn insert_mysql_batch(
    pool: &MySqlPool,
    tables: &TableNames,
    pending: Vec<PendingRecord>,
) -> Result<Vec<RecordDetail>> {
    let record_sql = format!(
        "INSERT INTO {} (
            created_at_ms, elapsed_ms, request_id, client_ip, questions_json,
            req_rd, req_cd, req_ad, req_opcode, req_edns_json, error,
            has_response, rcode, resp_aa, resp_tc, resp_ra, resp_ad, resp_cd,
            answer_count, authority_count, additional_count, answer_preview_json,
            answers_json, authorities_json, additionals_json, signature_json, resp_edns_json
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        tables.records
    );
    let question_sql = format!(
        "INSERT INTO {} (record_id, question_index, name_lc, qtype, qclass)
         VALUES (?, ?, ?, ?, ?)",
        tables.questions
    );
    let step_sql = format!(
        "INSERT INTO {} (record_id, event_index, sequence_tag, node_index, kind, tag, outcome)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
        tables.steps
    );

    let mut tx = pool.begin().await?;
    let mut committed = Vec::with_capacity(pending.len());
    for pending_record in pending {
        let (record, steps) = pending_record.take_to_record();
        let json = SerializedRecord::new(&record)?;
        let result = sqlx::query(&record_sql)
            .bind(record.created_at_ms)
            .bind(i64_from_u64(record.elapsed_ms, "elapsed_ms")?)
            .bind(i32::from(record.request_id))
            .bind(record.client_ip.as_str())
            .bind(json.questions.as_str())
            .bind(bool_i32(record.req_rd))
            .bind(bool_i32(record.req_cd))
            .bind(bool_i32(record.req_ad))
            .bind(record.req_opcode.as_str())
            .bind(json.req_edns.as_deref())
            .bind(record.error.as_deref())
            .bind(bool_i32(record.has_response))
            .bind(record.rcode.as_deref())
            .bind(record.resp_aa.map(bool_i32))
            .bind(record.resp_tc.map(bool_i32))
            .bind(record.resp_ra.map(bool_i32))
            .bind(record.resp_ad.map(bool_i32))
            .bind(record.resp_cd.map(bool_i32))
            .bind(i64::from(record.answer_count))
            .bind(i64::from(record.authority_count))
            .bind(i64::from(record.additional_count))
            .bind(json.answer_preview.as_str())
            .bind(json.answers.as_str())
            .bind(json.authorities.as_str())
            .bind(json.additionals.as_str())
            .bind(json.signature.as_str())
            .bind(json.resp_edns.as_deref())
            .execute(&mut *tx)
            .await?;
        let record_id = i64::try_from(result.last_insert_id())
            .map_err(|_| DnsError::runtime("query_recorder MySQL record id overflow"))?;

        for (index, question) in record.questions_json.iter().enumerate() {
            sqlx::query(&question_sql)
                .bind(record_id)
                .bind(i64_from_usize(index, "question_index")?)
                .bind(question.name.to_ascii_lowercase())
                .bind(question.qtype.to_ascii_uppercase())
                .bind(question.qclass.as_str())
                .execute(&mut *tx)
                .await?;
        }
        for step in &steps {
            sqlx::query(&step_sql)
                .bind(record_id)
                .bind(i64_from_usize(step.event_index, "event_index")?)
                .bind(step.sequence_tag.as_str())
                .bind(
                    step.node_index
                        .map(|value| i64_from_usize(value, "node_index"))
                        .transpose()?,
                )
                .bind(step.kind.as_str())
                .bind(step.tag.as_deref())
                .bind(step.outcome.as_str())
                .execute(&mut *tx)
                .await?;
        }
        committed.push(RecordDetail {
            record: RecordRow {
                id: record_id,
                ..record
            },
            steps,
        });
    }
    tx.commit().await?;
    Ok(committed)
}

struct SerializedRecord {
    questions: String,
    req_edns: Option<String>,
    answer_preview: String,
    answers: String,
    authorities: String,
    additionals: String,
    signature: String,
    resp_edns: Option<String>,
}

impl SerializedRecord {
    fn new(record: &RecordRow) -> Result<Self> {
        Ok(Self {
            questions: serde_json::to_string(&record.questions_json)?,
            req_edns: serialize_optional_json(&record.req_edns_json)?,
            answer_preview: serde_json::to_string(&record.answer_preview)?,
            answers: serde_json::to_string(&record.answers_json)?,
            authorities: serde_json::to_string(&record.authorities_json)?,
            additionals: serde_json::to_string(&record.additionals_json)?,
            signature: serde_json::to_string(&record.signature_json)?,
            resp_edns: serialize_optional_json(&record.resp_edns_json)?,
        })
    }
}

fn serialize_optional_json<T: Serialize>(value: &Option<T>) -> Result<Option<String>> {
    value
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(Into::into)
}

fn bool_i32(value: bool) -> i32 {
    if value { 1 } else { 0 }
}

fn i64_from_u64(value: u64, field: &str) -> Result<i64> {
    i64::try_from(value).map_err(|_| DnsError::runtime(format!("query_recorder {field} overflow")))
}

fn i64_from_usize(value: usize, field: &str) -> Result<i64> {
    i64::try_from(value).map_err(|_| DnsError::runtime(format!("query_recorder {field} overflow")))
}

async fn cleanup_postgres(pool: &PgPool, tables: &TableNames, cutoff_ms: i64) -> Result<()> {
    let sql = format!(
        "DELETE FROM {records}
         WHERE id IN (
            SELECT id FROM {records}
            WHERE created_at_ms < $1
            ORDER BY created_at_ms ASC, id ASC
            LIMIT $2
         )",
        records = tables.records
    );
    loop {
        let affected = sqlx::query(&sql)
            .bind(cutoff_ms)
            .bind(CLEANUP_BATCH_SIZE)
            .execute(pool)
            .await?
            .rows_affected();
        if affected == 0 {
            break;
        }
    }
    Ok(())
}

async fn cleanup_mysql(pool: &MySqlPool, tables: &TableNames, cutoff_ms: i64) -> Result<()> {
    let sql = format!(
        "DELETE FROM {records}
         WHERE id IN (
            SELECT doomed.id FROM (
                SELECT id FROM {records}
                WHERE created_at_ms < ?
                ORDER BY created_at_ms ASC, id ASC
                LIMIT ?
            ) AS doomed
         )",
        records = tables.records
    );
    loop {
        let affected = sqlx::query(&sql)
            .bind(cutoff_ms)
            .bind(CLEANUP_BATCH_SIZE)
            .execute(pool)
            .await?
            .rows_affected();
        if affected == 0 {
            break;
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
enum BindValue {
    I64(i64),
    Text(String),
}

#[derive(Debug)]
struct SqlStatement {
    sql: String,
    params: Vec<BindValue>,
}

#[derive(Debug)]
struct SqlBuilder {
    dialect: RemoteDialect,
    params: Vec<BindValue>,
}

impl SqlBuilder {
    fn new(dialect: RemoteDialect) -> Self {
        Self {
            dialect,
            params: Vec::new(),
        }
    }

    fn bind(&mut self, value: BindValue) -> String {
        self.params.push(value);
        match self.dialect {
            RemoteDialect::Postgres => format!("${}", self.params.len()),
            RemoteDialect::Mysql => "?".to_string(),
        }
    }

    fn finish(self, sql: String) -> SqlStatement {
        SqlStatement {
            sql,
            params: self.params,
        }
    }
}

async fn fetch_all_postgres(pool: &PgPool, statement: &SqlStatement) -> Result<Vec<PgRow>> {
    let mut query = sqlx::query(&statement.sql);
    for value in &statement.params {
        query = match value {
            BindValue::I64(value) => query.bind(*value),
            BindValue::Text(value) => query.bind(value.as_str()),
        };
    }
    Ok(query.fetch_all(pool).await?)
}

async fn fetch_all_mysql(pool: &MySqlPool, statement: &SqlStatement) -> Result<Vec<MySqlRow>> {
    let mut query = sqlx::query(&statement.sql);
    for value in &statement.params {
        query = match value {
            BindValue::I64(value) => query.bind(*value),
            BindValue::Text(value) => query.bind(value.as_str()),
        };
    }
    Ok(query.fetch_all(pool).await?)
}

impl RemotePool {
    pub(super) async fn query_records(
        &self,
        tables: &TableNames,
        query: ListQuery,
    ) -> Result<(Vec<RecordSummaryRow>, Option<String>)> {
        let mut builder = SqlBuilder::new(self.dialect());
        let mut clauses = record_filter_clauses(
            "r",
            tables,
            query.since_ms,
            query.until_ms,
            &query.filter,
            &mut builder,
        )?;
        if let Some(cursor) = query.cursor {
            let created_a = builder.bind(BindValue::I64(cursor.created_at_ms));
            let created_b = builder.bind(BindValue::I64(cursor.created_at_ms));
            let id = builder.bind(BindValue::I64(cursor.id));
            clauses.push(format!(
                "(r.created_at_ms < {created_a} OR (r.created_at_ms = {created_b} AND r.id < {id}))"
            ));
        }
        let limit = builder.bind(BindValue::I64(i64_from_usize(
            query.limit.saturating_add(1),
            "list limit",
        )?));
        let sql = format!(
            "SELECT {}
             FROM {} r
             WHERE {}
             ORDER BY r.created_at_ms DESC, r.id DESC
             LIMIT {}",
            summary_columns("r"),
            tables.records,
            join_clauses(&clauses),
            limit
        );
        let statement = builder.finish(sql);
        let mut records = match self {
            Self::Postgres(pool) => fetch_all_postgres(pool, &statement)
                .await?
                .iter()
                .map(decode_summary_row)
                .collect::<Result<Vec<_>>>()?,
            Self::Mysql(pool) => fetch_all_mysql(pool, &statement)
                .await?
                .iter()
                .map(decode_summary_row)
                .collect::<Result<Vec<_>>>()?,
        };
        let has_more = records.len() > query.limit;
        if has_more {
            records.truncate(query.limit);
        }
        let next_cursor = if has_more {
            records.last().map(|record| {
                encode_cursor(ListCursor {
                    created_at_ms: record.created_at_ms,
                    id: record.id,
                })
            })
        } else {
            None
        };
        Ok((records, next_cursor))
    }

    pub(super) async fn load_record_detail(
        &self,
        tables: &TableNames,
        record_id: i64,
    ) -> Result<Option<RecordDetail>> {
        let placeholder = match self.dialect() {
            RemoteDialect::Postgres => "$1",
            RemoteDialect::Mysql => "?",
        };
        let record_sql = format!(
            "SELECT {} FROM {} WHERE id = {}",
            record_columns(""),
            tables.records,
            placeholder
        );
        let record = match self {
            Self::Postgres(pool) => {
                let row = sqlx::query(&record_sql)
                    .bind(record_id)
                    .fetch_optional(pool)
                    .await?;
                row.as_ref().map(decode_record_row).transpose()?
            }
            Self::Mysql(pool) => {
                let row = sqlx::query(&record_sql)
                    .bind(record_id)
                    .fetch_optional(pool)
                    .await?;
                row.as_ref().map(decode_record_row).transpose()?
            }
        };
        let Some(record) = record else {
            return Ok(None);
        };

        let steps_sql = format!(
            "SELECT event_index, sequence_tag, node_index, kind, tag, outcome
             FROM {} WHERE record_id = {} ORDER BY event_index ASC",
            tables.steps, placeholder
        );
        let steps = match self {
            Self::Postgres(pool) => sqlx::query(&steps_sql)
                .bind(record_id)
                .fetch_all(pool)
                .await?
                .iter()
                .map(decode_step_row)
                .collect::<Result<Vec<_>>>()?,
            Self::Mysql(pool) => sqlx::query(&steps_sql)
                .bind(record_id)
                .fetch_all(pool)
                .await?
                .iter()
                .map(decode_step_row)
                .collect::<Result<Vec<_>>>()?,
        };
        Ok(Some(RecordDetail { record, steps }))
    }

    pub(super) async fn load_plugin_stats(
        &self,
        tables: &TableNames,
        query: PluginsStatsQuery,
    ) -> Result<(u64, Vec<PluginStatsRow>)> {
        let dialect = self.dialect();
        let mut builder = SqlBuilder::new(dialect);
        let clauses = record_filter_clauses(
            "r",
            tables,
            query.since_ms,
            query.until_ms,
            &query.filter,
            &mut builder,
        )?;
        let sample_limit = builder.bind(BindValue::I64(STATS_SAMPLE_LIMIT));
        let kind_filter = if query.kind == PluginStatsKind::All {
            String::new()
        } else {
            let kind = builder.bind(BindValue::Text(plugin_stats_kind(query.kind).to_string()));
            format!("AND s.kind = {kind}")
        };
        let count = dialect.signed_count("COUNT(*)");
        let checked = dialect.signed_count(
            "SUM(CASE WHEN s.kind = 'matcher' AND s.outcome IN ('matched', 'not_matched') THEN 1 ELSE 0 END)",
        );
        let matched = dialect.signed_count(
            "SUM(CASE WHEN s.kind = 'matcher' AND s.outcome = 'matched' THEN 1 ELSE 0 END)",
        );
        let executed = dialect.signed_count(
            "SUM(CASE WHEN s.kind = 'executor' AND s.outcome = 'entered' THEN 1 WHEN s.kind = 'builtin' THEN 1 ELSE 0 END)",
        );
        let query_hits = dialect.signed_count("COUNT(DISTINCT s.record_id)");
        let sql = format!(
            "WITH sample_records {sample_cte} (
                SELECT r.id FROM {records} r
                WHERE {where_sql}
                ORDER BY r.created_at_ms DESC, r.id DESC
                LIMIT {sample_limit}
             ), totals AS (
                SELECT {count} AS total_records FROM sample_records
             ), step_agg AS (
                SELECT s.kind, s.tag,
                       {checked} AS checked,
                       {matched} AS matched,
                       {executed} AS executed,
                       {query_hits} AS query_hits
                FROM sample_records sr
                JOIN {steps} s ON s.record_id = sr.id
                WHERE 1 = 1 {kind_filter}
                GROUP BY s.kind, s.tag
             )
             SELECT totals.total_records, sa.kind, sa.tag, sa.checked, sa.matched,
                    sa.executed, sa.query_hits
             FROM totals LEFT JOIN step_agg sa ON 1 = 1
             ORDER BY sa.kind ASC, sa.query_hits DESC, sa.tag ASC",
            sample_cte = dialect.sample_cte(),
            records = tables.records,
            where_sql = join_clauses(&clauses),
            steps = tables.steps,
        );
        let statement = builder.finish(sql);
        match self {
            Self::Postgres(pool) => {
                decode_plugin_stats(&fetch_all_postgres(pool, &statement).await?)
            }
            Self::Mysql(pool) => decode_plugin_stats(&fetch_all_mysql(pool, &statement).await?),
        }
    }
}

fn decode_plugin_stats<R: DecodeRemoteRow>(rows: &[R]) -> Result<(u64, Vec<PluginStatsRow>)> {
    let mut total_records = 0;
    let mut stats = Vec::new();
    for row in rows {
        total_records = non_negative_u64(row.i64_at(0)?, "plugin stats total")?;
        let Some(kind) = row.optional_string_at(1)? else {
            continue;
        };
        let query_total = non_negative_u64(row.i64_at(6)?, "plugin stats query total")?;
        stats.push(PluginStatsRow {
            kind,
            tag: row.optional_string_at(2)?,
            checked: non_negative_u64(row.i64_at(3)?, "plugin stats checked")?,
            matched: non_negative_u64(row.i64_at(4)?, "plugin stats matched")?,
            executed: non_negative_u64(row.i64_at(5)?, "plugin stats executed")?,
            query_total,
            query_share: if total_records == 0 {
                0.0
            } else {
                query_total as f64 / total_records as f64
            },
        });
    }
    Ok((total_records, stats))
}

fn plugin_stats_kind(kind: PluginStatsKind) -> &'static str {
    match kind {
        PluginStatsKind::Matcher => "matcher",
        PluginStatsKind::Executor => "executor",
        PluginStatsKind::Builtin => "builtin",
        PluginStatsKind::All => "all",
    }
}

impl RemotePool {
    pub(super) async fn load_top_clients(
        &self,
        tables: &TableNames,
        query: TopQuery,
    ) -> Result<TopBucketsResponse> {
        let dialect = self.dialect();
        let mut builder = SqlBuilder::new(dialect);
        let clauses = record_filter_clauses(
            "r",
            tables,
            query.since_ms,
            query.until_ms,
            &query.filter,
            &mut builder,
        )?;
        let sample_limit = builder.bind(BindValue::I64(STATS_SAMPLE_LIMIT));
        let row_limit = builder.bind(BindValue::I64(i64_from_usize(query.limit, "top limit")?));
        let sample_count = dialect.signed_count("COUNT(*)");
        let bucket_count = dialect.signed_count("COUNT(sample_records.client_ip)");
        let sql = format!(
            "WITH sample_records {sample_cte} (
                SELECT r.id, r.client_ip FROM {records} r
                WHERE {where_sql}
                ORDER BY r.created_at_ms DESC, r.id DESC
                LIMIT {sample_limit}
             ), totals AS (
                SELECT {sample_count} AS sample_size FROM sample_records
             )
             SELECT totals.sample_size, sample_records.client_ip, {bucket_count} AS bucket_count
             FROM totals LEFT JOIN sample_records ON 1 = 1
             GROUP BY totals.sample_size, sample_records.client_ip
             ORDER BY bucket_count DESC, sample_records.client_ip ASC
             LIMIT {row_limit}",
            sample_cte = dialect.sample_cte(),
            records = tables.records,
            where_sql = join_clauses(&clauses),
        );
        self.fetch_top_buckets(builder.finish(sql)).await
    }

    pub(super) async fn load_top_qnames(
        &self,
        tables: &TableNames,
        query: TopQuery,
    ) -> Result<TopBucketsResponse> {
        let dialect = self.dialect();
        let mut builder = SqlBuilder::new(dialect);
        let clauses = record_filter_clauses(
            "r",
            tables,
            query.since_ms,
            query.until_ms,
            &query.filter,
            &mut builder,
        )?;
        let sample_limit = builder.bind(BindValue::I64(STATS_SAMPLE_LIMIT));
        let row_limit = builder.bind(BindValue::I64(i64_from_usize(query.limit, "top limit")?));
        let sample_count = dialect.signed_count("COUNT(*)");
        let bucket_count = dialect.signed_count("COUNT(q.name_lc)");
        let sql = format!(
            "WITH sample_records {sample_cte} (
                SELECT r.id FROM {records} r
                WHERE {where_sql}
                ORDER BY r.created_at_ms DESC, r.id DESC
                LIMIT {sample_limit}
             ), totals AS (
                SELECT {sample_count} AS sample_size FROM sample_records
             )
             SELECT totals.sample_size, q.name_lc, {bucket_count} AS bucket_count
             FROM totals
             LEFT JOIN sample_records ON 1 = 1
             LEFT JOIN {questions} q ON q.record_id = sample_records.id
             GROUP BY totals.sample_size, q.name_lc
             ORDER BY bucket_count DESC, q.name_lc ASC
             LIMIT {row_limit}",
            sample_cte = dialect.sample_cte(),
            records = tables.records,
            where_sql = join_clauses(&clauses),
            questions = tables.questions,
        );
        self.fetch_top_buckets(builder.finish(sql)).await
    }

    async fn fetch_top_buckets(&self, statement: SqlStatement) -> Result<TopBucketsResponse> {
        match self {
            Self::Postgres(pool) => {
                decode_top_buckets(&fetch_all_postgres(pool, &statement).await?)
            }
            Self::Mysql(pool) => decode_top_buckets(&fetch_all_mysql(pool, &statement).await?),
        }
    }

    pub(super) async fn load_qtype_distribution(
        &self,
        tables: &TableNames,
        query: DistributionQuery,
    ) -> Result<DistributionResponse> {
        let dialect = self.dialect();
        let mut builder = SqlBuilder::new(dialect);
        let clauses = record_filter_clauses(
            "r",
            tables,
            query.since_ms,
            query.until_ms,
            &query.filter,
            &mut builder,
        )?;
        let sample_limit = builder.bind(BindValue::I64(STATS_SAMPLE_LIMIT));
        let sample_count = dialect.signed_count("COUNT(*)");
        let bucket_count = dialect.signed_count("COUNT(q.qtype)");
        let sql = format!(
            "WITH sample_records {sample_cte} (
                SELECT r.id FROM {records} r
                WHERE {where_sql}
                ORDER BY r.created_at_ms DESC, r.id DESC
                LIMIT {sample_limit}
             ), totals AS (
                SELECT {sample_count} AS sample_size FROM sample_records
             )
             SELECT totals.sample_size, q.qtype, {bucket_count} AS bucket_count
             FROM totals
             LEFT JOIN sample_records ON 1 = 1
             LEFT JOIN {questions} q ON q.record_id = sample_records.id
             GROUP BY totals.sample_size, q.qtype
             ORDER BY bucket_count DESC, q.qtype ASC",
            sample_cte = dialect.sample_cte(),
            records = tables.records,
            where_sql = join_clauses(&clauses),
            questions = tables.questions,
        );
        self.fetch_distribution(builder.finish(sql)).await
    }

    pub(super) async fn load_rcode_distribution(
        &self,
        tables: &TableNames,
        query: DistributionQuery,
    ) -> Result<DistributionResponse> {
        let dialect = self.dialect();
        let mut builder = SqlBuilder::new(dialect);
        let clauses = record_filter_clauses(
            "r",
            tables,
            query.since_ms,
            query.until_ms,
            &query.filter,
            &mut builder,
        )?;
        let sample_limit = builder.bind(BindValue::I64(STATS_SAMPLE_LIMIT));
        let sample_count = dialect.signed_count("COUNT(*)");
        let bucket_count = dialect.signed_count("COUNT(sample_records.id)");
        let sql = format!(
            "WITH sample_records {sample_cte} (
                SELECT r.id, r.rcode, r.error, r.has_response FROM {records} r
                WHERE {where_sql}
                ORDER BY r.created_at_ms DESC, r.id DESC
                LIMIT {sample_limit}
             ), totals AS (
                SELECT {sample_count} AS sample_size FROM sample_records
             )
             SELECT totals.sample_size,
                    CASE
                        WHEN sample_records.id IS NULL THEN NULL
                        WHEN sample_records.rcode IS NOT NULL THEN sample_records.rcode
                        WHEN sample_records.error IS NOT NULL THEN '_ERROR'
                        WHEN sample_records.has_response = 0 THEN '_NO_RESPONSE'
                        ELSE '_UNKNOWN'
                    END AS bucket,
                    {bucket_count} AS bucket_count
             FROM totals LEFT JOIN sample_records ON 1 = 1
             GROUP BY totals.sample_size, bucket
             ORDER BY bucket_count DESC, bucket ASC",
            sample_cte = dialect.sample_cte(),
            records = tables.records,
            where_sql = join_clauses(&clauses),
        );
        self.fetch_distribution(builder.finish(sql)).await
    }

    async fn fetch_distribution(&self, statement: SqlStatement) -> Result<DistributionResponse> {
        match self {
            Self::Postgres(pool) => {
                decode_distribution(&fetch_all_postgres(pool, &statement).await?)
            }
            Self::Mysql(pool) => decode_distribution(&fetch_all_mysql(pool, &statement).await?),
        }
    }
}

fn decode_top_buckets<R: DecodeRemoteRow>(rows: &[R]) -> Result<TopBucketsResponse> {
    let mut sample_size = 0;
    let mut buckets = Vec::new();
    for row in rows {
        sample_size = non_negative_u64(row.i64_at(0)?, "sample_size")?;
        let Some(key) = row.optional_string_at(1)? else {
            continue;
        };
        let count = non_negative_u64(row.i64_at(2)?, "bucket count")?;
        buckets.push(TopBucketRow {
            key,
            count,
            share: bucket_share(count, sample_size),
        });
    }
    Ok(TopBucketsResponse {
        ok: true,
        sample_size,
        rows: buckets,
    })
}

fn decode_distribution<R: DecodeRemoteRow>(rows: &[R]) -> Result<DistributionResponse> {
    let mut sample_size = 0;
    let mut distribution = Vec::new();
    for row in rows {
        sample_size = non_negative_u64(row.i64_at(0)?, "sample_size")?;
        let Some(key) = row.optional_string_at(1)? else {
            continue;
        };
        let count = non_negative_u64(row.i64_at(2)?, "distribution count")?;
        distribution.push(DistributionRow {
            key,
            count,
            share: bucket_share(count, sample_size),
        });
    }
    Ok(DistributionResponse {
        ok: true,
        sample_size,
        rows: distribution,
    })
}

impl RemotePool {
    pub(super) async fn load_latency_summary(
        &self,
        tables: &TableNames,
        query: LatencyQuery,
    ) -> Result<LatencySummary> {
        let dialect = self.dialect();
        let mut builder = SqlBuilder::new(dialect);
        let clauses = record_filter_clauses(
            "r",
            tables,
            query.since_ms,
            query.until_ms,
            &query.filter,
            &mut builder,
        )?;
        let sample_limit = builder.bind(BindValue::I64(STATS_SAMPLE_LIMIT));
        let sql = format!(
            "SELECT r.elapsed_ms FROM {} r
             WHERE {}
             ORDER BY r.created_at_ms DESC, r.id DESC
             LIMIT {}",
            tables.records,
            join_clauses(&clauses),
            sample_limit
        );
        let statement = builder.finish(sql);
        let mut elapsed_values = match self {
            Self::Postgres(pool) => {
                decode_i64_column(&fetch_all_postgres(pool, &statement).await?, "elapsed_ms")?
            }
            Self::Mysql(pool) => {
                decode_i64_column(&fetch_all_mysql(pool, &statement).await?, "elapsed_ms")?
            }
        };
        let sample_size = elapsed_values.len() as u64;
        let (avg_ms, p50_ms, p95_ms, p99_ms, max_ms) = latency_percentiles(&mut elapsed_values);
        let histogram: Vec<LatencyHistogramBucket> = latency_histogram(&elapsed_values);

        let mut slow_builder = SqlBuilder::new(dialect);
        let slow_clauses = record_filter_clauses(
            "r",
            tables,
            query.since_ms,
            query.until_ms,
            &query.filter,
            &mut slow_builder,
        )?;
        let slow_sample_limit = slow_builder.bind(BindValue::I64(STATS_SAMPLE_LIMIT));
        let slow_limit = slow_builder.bind(BindValue::I64(i64_from_usize(
            query.slow_limit,
            "slow limit",
        )?));
        let count = dialect.signed_count("COUNT(*)");
        let average = dialect.float_average("sample_records.elapsed_ms");
        let maximum = dialect.signed_count("MAX(sample_records.elapsed_ms)");
        let slow_sql = format!(
            "WITH sample_records {sample_cte} (
                SELECT r.id, r.elapsed_ms FROM {records} r
                WHERE {where_sql}
                ORDER BY r.created_at_ms DESC, r.id DESC
                LIMIT {slow_sample_limit}
             )
             SELECT q.name_lc, {count} AS query_count, {average} AS avg_ms,
                    {maximum} AS max_ms
             FROM sample_records
             JOIN {questions} q ON q.record_id = sample_records.id
             GROUP BY q.name_lc
             ORDER BY avg_ms DESC, query_count DESC
             LIMIT {slow_limit}",
            sample_cte = dialect.sample_cte(),
            records = tables.records,
            where_sql = join_clauses(&slow_clauses),
            questions = tables.questions,
        );
        let slow_statement = slow_builder.finish(slow_sql);
        let slow_top = match self {
            Self::Postgres(pool) => {
                decode_slow_rows(&fetch_all_postgres(pool, &slow_statement).await?)?
            }
            Self::Mysql(pool) => decode_slow_rows(&fetch_all_mysql(pool, &slow_statement).await?)?,
        };

        Ok(LatencySummary {
            ok: true,
            sample_size,
            avg_ms,
            p50_ms,
            p95_ms,
            p99_ms,
            max_ms,
            histogram,
            slow_top,
        })
    }

    pub(super) async fn load_timeseries(
        &self,
        tables: &TableNames,
        query: TimeseriesQuery,
    ) -> Result<TimeseriesResponse> {
        let dialect = self.dialect();
        let mut builder = SqlBuilder::new(dialect);
        let clauses = record_filter_clauses(
            "r",
            tables,
            query.since_ms,
            query.until_ms,
            &query.filter,
            &mut builder,
        )?;
        let sample_limit = builder.bind(BindValue::I64(STATS_SAMPLE_LIMIT));
        let has_response = dialect.signed_count("r.has_response");
        let sql = format!(
            "SELECT r.created_at_ms, r.elapsed_ms, r.error,
                    {has_response} AS has_response
             FROM {} r
             WHERE {}
             ORDER BY r.created_at_ms DESC, r.id DESC
             LIMIT {}",
            tables.records,
            join_clauses(&clauses),
            sample_limit
        );
        let statement = builder.finish(sql);
        let rows = match self {
            Self::Postgres(pool) => {
                aggregate_timeseries(&fetch_all_postgres(pool, &statement).await?, &query)?
            }
            Self::Mysql(pool) => {
                aggregate_timeseries(&fetch_all_mysql(pool, &statement).await?, &query)?
            }
        };
        Ok(rows)
    }
}

fn decode_i64_column<R: DecodeRemoteRow>(rows: &[R], field: &str) -> Result<Vec<u64>> {
    rows.iter()
        .map(|row| non_negative_u64(row.i64_at(0)?, field))
        .collect()
}

fn decode_slow_rows<R: DecodeRemoteRow>(rows: &[R]) -> Result<Vec<LatencySlowRow>> {
    rows.iter()
        .map(|row| {
            Ok(LatencySlowRow {
                qname: row.string_at(0)?,
                count: non_negative_u64(row.i64_at(1)?, "slow query count")?,
                avg_ms: row.f64_at(2)?,
                max_ms: non_negative_u64(row.i64_at(3)?, "slow max_ms")?,
            })
        })
        .collect()
}

fn aggregate_timeseries<R: DecodeRemoteRow>(
    rows: &[R],
    query: &TimeseriesQuery,
) -> Result<TimeseriesResponse> {
    #[derive(Default)]
    struct Aggregator {
        total: u64,
        error_count: u64,
        no_response_count: u64,
        elapsed_sum: u64,
        elapsed_values: Vec<u64>,
    }

    let bucket_ms = query.bucket.millis();
    let mut buckets: BTreeMap<i64, Aggregator> = BTreeMap::new();
    let mut sample_size = 0u64;
    for row in rows {
        let created_at_ms = row.i64_at(0)?;
        let elapsed_ms = non_negative_u64(row.i64_at(1)?, "elapsed_ms")?;
        let error = row.optional_string_at(2)?;
        let has_response = row.i64_at(3)? != 0;
        let aggregator = buckets
            .entry(bucket_floor(created_at_ms, bucket_ms))
            .or_default();
        aggregator.total = aggregator.total.saturating_add(1);
        if error.is_some() {
            aggregator.error_count = aggregator.error_count.saturating_add(1);
        }
        if error.is_none() && !has_response {
            aggregator.no_response_count = aggregator.no_response_count.saturating_add(1);
        }
        aggregator.elapsed_sum = aggregator.elapsed_sum.saturating_add(elapsed_ms);
        aggregator.elapsed_values.push(elapsed_ms);
        sample_size = sample_size.saturating_add(1);
    }

    let mut points = Vec::with_capacity(buckets.len());
    for (bucket, mut aggregator) in buckets {
        let avg_ms = if aggregator.total == 0 {
            0.0
        } else {
            aggregator.elapsed_sum as f64 / aggregator.total as f64
        };
        points.push(TimeseriesPoint {
            bucket_ms: bucket,
            total: aggregator.total,
            error_count: aggregator.error_count,
            no_response_count: aggregator.no_response_count,
            avg_ms,
            p95_ms: percentile_value(&mut aggregator.elapsed_values, 0.95),
        });
    }
    if points.len() > query.max_buckets {
        points.drain(0..points.len() - query.max_buckets);
    }
    Ok(TimeseriesResponse {
        ok: true,
        sample_size,
        bucket_ms,
        points,
    })
}

fn record_filter_clauses(
    alias: &str,
    tables: &TableNames,
    since_ms: Option<u64>,
    until_ms: Option<u64>,
    filter: &QueryRecordFilter,
    builder: &mut SqlBuilder,
) -> Result<Vec<String>> {
    let mut clauses = Vec::new();
    if let Some(since_ms) = since_ms {
        let value = builder.bind(BindValue::I64(i64_from_u64(since_ms, "since_ms")?));
        clauses.push(format!("{alias}.created_at_ms >= {value}"));
    }
    if let Some(until_ms) = until_ms {
        let value = builder.bind(BindValue::I64(i64_from_u64(until_ms, "until_ms")?));
        clauses.push(format!("{alias}.created_at_ms <= {value}"));
    }
    if let Some(matcher_tag) = filter.matcher_tag.as_deref() {
        let value = builder.bind(BindValue::Text(matcher_tag.to_string()));
        clauses.push(format!(
            "{alias}.id IN (
                SELECT s.record_id FROM {steps} s
                WHERE s.kind = 'matcher' AND s.outcome = 'matched' AND s.tag = {value}
            )",
            steps = tables.steps
        ));
    }
    if let Some(search) = filter.search.as_deref() {
        let qname = builder.bind(BindValue::Text(like_pattern(&search.to_ascii_lowercase())));
        let client = builder.bind(BindValue::Text(like_pattern(search)));
        clauses.push(format!(
            "(EXISTS (
                SELECT 1 FROM {questions} q
                WHERE q.record_id = {alias}.id
                  AND q.name_lc LIKE {qname} ESCAPE '!'
             ) OR LOWER({alias}.client_ip) LIKE LOWER({client}) ESCAPE '!')",
            questions = tables.questions
        ));
    }
    if let Some(qname) = filter.qname.as_deref() {
        let value = builder.bind(BindValue::Text(like_pattern(&qname.to_ascii_lowercase())));
        clauses.push(format!(
            "EXISTS (
                SELECT 1 FROM {questions} q
                WHERE q.record_id = {alias}.id AND q.name_lc LIKE {value} ESCAPE '!'
            )",
            questions = tables.questions
        ));
    }
    if let Some(qtype) = filter.qtype.as_deref() {
        let value = builder.bind(BindValue::Text(qtype.to_ascii_uppercase()));
        clauses.push(format!(
            "{alias}.id IN (SELECT q.record_id FROM {questions} q WHERE q.qtype = {value})",
            questions = tables.questions
        ));
    }
    if let Some(client_ip) = filter.client_ip.as_deref() {
        let value = builder.bind(BindValue::Text(like_pattern(client_ip)));
        clauses.push(format!(
            "LOWER({alias}.client_ip) LIKE LOWER({value}) ESCAPE '!'"
        ));
    }
    if let Some(rcode) = filter.rcode.as_deref() {
        let value = builder.bind(BindValue::Text(rcode.to_string()));
        clauses.push(format!(
            "{alias}.rcode IS NOT NULL AND UPPER({alias}.rcode) = UPPER({value})"
        ));
    }
    match filter.status {
        QueryRecordStatus::All => {}
        QueryRecordStatus::Error => clauses.push(format!("{alias}.error IS NOT NULL")),
        QueryRecordStatus::HasResponse => clauses.push(format!("{alias}.has_response = 1")),
        QueryRecordStatus::NoResponse => clauses.push(format!(
            "{alias}.error IS NULL AND {alias}.has_response = 0"
        )),
    }
    Ok(clauses)
}

fn join_clauses(clauses: &[String]) -> String {
    if clauses.is_empty() {
        "1 = 1".to_string()
    } else {
        clauses.join(" AND ")
    }
}

fn like_pattern(raw: &str) -> String {
    let mut pattern = String::with_capacity(raw.len() + 2);
    pattern.push('%');
    for character in raw.chars() {
        if matches!(character, '!' | '%' | '_') {
            pattern.push('!');
        }
        pattern.push(character);
    }
    pattern.push('%');
    pattern
}

fn encode_cursor(cursor: ListCursor) -> String {
    format!("{}:{}", cursor.created_at_ms, cursor.id)
}

fn summary_columns(alias: &str) -> String {
    let prefix = if alias.is_empty() {
        String::new()
    } else {
        format!("{alias}.")
    };
    [
        "id",
        "created_at_ms",
        "elapsed_ms",
        "request_id",
        "client_ip",
        "questions_json",
        "error",
        "has_response",
        "rcode",
        "answer_count",
        "authority_count",
        "additional_count",
        "answer_preview_json",
    ]
    .iter()
    .map(|column| format!("{prefix}{column}"))
    .collect::<Vec<_>>()
    .join(", ")
}

fn record_columns(alias: &str) -> String {
    let prefix = if alias.is_empty() {
        String::new()
    } else {
        format!("{alias}.")
    };
    [
        "id",
        "created_at_ms",
        "elapsed_ms",
        "request_id",
        "client_ip",
        "questions_json",
        "req_rd",
        "req_cd",
        "req_ad",
        "req_opcode",
        "req_edns_json",
        "error",
        "has_response",
        "rcode",
        "resp_aa",
        "resp_tc",
        "resp_ra",
        "resp_ad",
        "resp_cd",
        "answer_count",
        "authority_count",
        "additional_count",
        "answer_preview_json",
        "answers_json",
        "authorities_json",
        "additionals_json",
        "signature_json",
        "resp_edns_json",
    ]
    .iter()
    .map(|column| format!("{prefix}{column}"))
    .collect::<Vec<_>>()
    .join(", ")
}

trait DecodeRemoteRow {
    fn decode_summary(&self) -> Result<RecordSummaryRow>;
    fn decode_record(&self) -> Result<RecordRow>;
    fn decode_step(&self) -> Result<StepJson>;
    fn i64_at(&self, index: usize) -> Result<i64>;
    fn f64_at(&self, index: usize) -> Result<f64>;
    fn string_at(&self, index: usize) -> Result<String>;
    fn optional_string_at(&self, index: usize) -> Result<Option<String>>;
}

macro_rules! impl_remote_row_decoder {
    ($row:ty, $string_at:path, $optional_string_at:path) => {
        impl DecodeRemoteRow for $row {
            fn decode_summary(&self) -> Result<RecordSummaryRow> {
                Ok(RecordSummaryRow {
                    id: self.try_get(0)?,
                    created_at_ms: self.try_get(1)?,
                    elapsed_ms: non_negative_u64(self.try_get(2)?, "elapsed_ms")?,
                    request_id: non_negative_u16(
                        i64::from(self.try_get::<i32, _>(3)?),
                        "request_id",
                    )?,
                    client_ip: self.string_at(4)?,
                    questions_json: parse_json(self.string_at(5)?)?,
                    error: self.optional_string_at(6)?,
                    has_response: self.try_get::<i32, _>(7)? != 0,
                    rcode: self.optional_string_at(8)?,
                    answer_count: non_negative_u32(self.try_get(9)?, "answer_count")?,
                    authority_count: non_negative_u32(self.try_get(10)?, "authority_count")?,
                    additional_count: non_negative_u32(self.try_get(11)?, "additional_count")?,
                    answer_preview: parse_json(self.string_at(12)?)?,
                })
            }

            fn decode_record(&self) -> Result<RecordRow> {
                Ok(RecordRow {
                    id: self.try_get(0)?,
                    created_at_ms: self.try_get(1)?,
                    elapsed_ms: non_negative_u64(self.try_get(2)?, "elapsed_ms")?,
                    request_id: non_negative_u16(
                        i64::from(self.try_get::<i32, _>(3)?),
                        "request_id",
                    )?,
                    client_ip: self.string_at(4)?,
                    questions_json: parse_json(self.string_at(5)?)?,
                    req_rd: self.try_get::<i32, _>(6)? != 0,
                    req_cd: self.try_get::<i32, _>(7)? != 0,
                    req_ad: self.try_get::<i32, _>(8)? != 0,
                    req_opcode: self.string_at(9)?,
                    req_edns_json: parse_optional_json(self.optional_string_at(10)?)?,
                    error: self.optional_string_at(11)?,
                    has_response: self.try_get::<i32, _>(12)? != 0,
                    rcode: self.optional_string_at(13)?,
                    resp_aa: self.try_get::<Option<i32>, _>(14)?.map(|value| value != 0),
                    resp_tc: self.try_get::<Option<i32>, _>(15)?.map(|value| value != 0),
                    resp_ra: self.try_get::<Option<i32>, _>(16)?.map(|value| value != 0),
                    resp_ad: self.try_get::<Option<i32>, _>(17)?.map(|value| value != 0),
                    resp_cd: self.try_get::<Option<i32>, _>(18)?.map(|value| value != 0),
                    answer_count: non_negative_u32(self.try_get(19)?, "answer_count")?,
                    authority_count: non_negative_u32(self.try_get(20)?, "authority_count")?,
                    additional_count: non_negative_u32(self.try_get(21)?, "additional_count")?,
                    answer_preview: parse_json(self.string_at(22)?)?,
                    answers_json: parse_json(self.string_at(23)?)?,
                    authorities_json: parse_json(self.string_at(24)?)?,
                    additionals_json: parse_json(self.string_at(25)?)?,
                    signature_json: parse_json(self.string_at(26)?)?,
                    resp_edns_json: parse_optional_json(self.optional_string_at(27)?)?,
                })
            }

            fn decode_step(&self) -> Result<StepJson> {
                Ok(StepJson {
                    event_index: non_negative_usize(self.try_get(0)?, "event_index")?,
                    sequence_tag: self.string_at(1)?,
                    node_index: self
                        .try_get::<Option<i64>, _>(2)?
                        .map(|value| non_negative_usize(value, "node_index"))
                        .transpose()?,
                    kind: self.string_at(3)?,
                    tag: self.optional_string_at(4)?,
                    outcome: self.string_at(5)?,
                })
            }

            fn i64_at(&self, index: usize) -> Result<i64> {
                Ok(self.try_get(index)?)
            }

            fn f64_at(&self, index: usize) -> Result<f64> {
                Ok(self.try_get(index)?)
            }

            fn string_at(&self, index: usize) -> Result<String> {
                $string_at(self, index)
            }

            fn optional_string_at(&self, index: usize) -> Result<Option<String>> {
                $optional_string_at(self, index)
            }
        }
    };
}

fn postgres_string_at(row: &PgRow, index: usize) -> Result<String> {
    Ok(row.try_get(index)?)
}

fn postgres_optional_string_at(row: &PgRow, index: usize) -> Result<Option<String>> {
    Ok(row.try_get(index)?)
}

fn mysql_string_at(row: &MySqlRow, index: usize) -> Result<String> {
    match row.try_get::<String, _>(index) {
        Ok(value) => Ok(value),
        Err(string_error) => {
            let bytes = row
                .try_get::<Vec<u8>, _>(index)
                .map_err(|_| string_error)?;
            String::from_utf8(bytes).map_err(|_| {
                DnsError::runtime(format!(
                    "query_recorder MySQL column {index} contains invalid UTF-8"
                ))
            })
        }
    }
}

fn mysql_optional_string_at(row: &MySqlRow, index: usize) -> Result<Option<String>> {
    match row.try_get::<Option<String>, _>(index) {
        Ok(value) => Ok(value),
        Err(string_error) => {
            let bytes = row
                .try_get::<Option<Vec<u8>>, _>(index)
                .map_err(|_| string_error)?;
            bytes
                .map(|value| {
                    String::from_utf8(value).map_err(|_| {
                        DnsError::runtime(format!(
                            "query_recorder MySQL column {index} contains invalid UTF-8"
                        ))
                    })
                })
                .transpose()
        }
    }
}

impl_remote_row_decoder!(PgRow, postgres_string_at, postgres_optional_string_at);
impl_remote_row_decoder!(MySqlRow, mysql_string_at, mysql_optional_string_at);

fn decode_summary_row<R: DecodeRemoteRow>(row: &R) -> Result<RecordSummaryRow> {
    row.decode_summary()
}

fn decode_record_row<R: DecodeRemoteRow>(row: &R) -> Result<RecordRow> {
    row.decode_record()
}

fn decode_step_row<R: DecodeRemoteRow>(row: &R) -> Result<StepJson> {
    row.decode_step()
}

fn parse_json<T: serde::de::DeserializeOwned>(raw: String) -> Result<T> {
    serde_json::from_str(&raw).map_err(Into::into)
}

fn parse_optional_json<T: serde::de::DeserializeOwned>(raw: Option<String>) -> Result<Option<T>> {
    raw.map(parse_json).transpose()
}

fn non_negative_u64(value: i64, field: &str) -> Result<u64> {
    u64::try_from(value).map_err(|_| DnsError::runtime(format!("query_recorder negative {field}")))
}

fn non_negative_u32(value: i64, field: &str) -> Result<u32> {
    u32::try_from(value).map_err(|_| DnsError::runtime(format!("query_recorder invalid {field}")))
}

fn non_negative_u16(value: i64, field: &str) -> Result<u16> {
    u16::try_from(value).map_err(|_| DnsError::runtime(format!("query_recorder invalid {field}")))
}

fn non_negative_usize(value: i64, field: &str) -> Result<usize> {
    usize::try_from(value).map_err(|_| DnsError::runtime(format!("query_recorder invalid {field}")))
}
