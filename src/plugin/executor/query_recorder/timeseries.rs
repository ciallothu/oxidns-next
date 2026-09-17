//! Calendar-aligned, exact time-series aggregation shared by every SQL backend.

use std::collections::BTreeMap;

use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;

use super::model::{TimeseriesBucket, TimeseriesPoint, TimeseriesQuery, TimeseriesResponse};
use crate::infra::error::{DnsError, Result};

impl TimeseriesBucket {
    pub(super) fn key(self, millis: i64) -> Result<i64> {
        if self != Self::Month {
            return Ok(millis.div_euclid(self.millis()));
        }
        let date = Timestamp::from_millisecond(millis)
            .map_err(|e| DnsError::runtime(e.to_string()))?
            .to_zoned(TimeZone::UTC);
        Ok(i64::from(date.year()) * 12 + i64::from(date.month()) - 1)
    }

    pub(super) fn start(self, key: i64) -> Result<i64> {
        if self != Self::Month {
            return key
                .checked_mul(self.millis())
                .ok_or_else(|| DnsError::runtime("bucket overflow"));
        }
        let year =
            i16::try_from(key.div_euclid(12)).map_err(|e| DnsError::runtime(e.to_string()))?;
        let month = (key.rem_euclid(12) + 1) as i8;
        Date::new(year, month, 1)
            .and_then(|date| date.at(0, 0, 0, 0).to_zoned(TimeZone::UTC))
            .map(|date| date.timestamp().as_millisecond())
            .map_err(|e| DnsError::runtime(e.to_string()))
    }
}

pub(super) struct Window {
    pub(super) since: u64,
    pub(super) until: u64,
    first: i64,
    last: i64,
    bucket: TimeseriesBucket,
}

impl Window {
    pub(super) fn new(query: &TimeseriesQuery) -> Result<Self> {
        let until = query
            .until_ms
            .unwrap_or_else(|| Timestamp::now().as_millisecond().max(0) as u64);
        let end = i64::try_from(until).map_err(|e| DnsError::runtime(e.to_string()))?;
        if query.since_ms.is_some_and(|since| since > until) {
            return Err(DnsError::runtime("since_ms must not exceed until_ms"));
        }
        let last = query.bucket.key(end)?;
        let earliest = last - query.max_buckets.clamp(1, 720) as i64 + 1;
        let since = query
            .since_ms
            .unwrap_or(0)
            .max(query.bucket.start(earliest)?.max(0) as u64);
        let first = query.bucket.key(since as i64)?;
        Ok(Self {
            since,
            until,
            first,
            last,
            bucket: query.bucket,
        })
    }

    pub(super) fn finish(self, rows: Vec<TimeseriesPoint>) -> Result<TimeseriesResponse> {
        let mut by_time: BTreeMap<_, _> =
            rows.into_iter().map(|row| (row.bucket_ms, row)).collect();
        let mut points = Vec::with_capacity((self.last - self.first + 1) as usize);
        for key in self.first..=self.last {
            let bucket_ms = self.bucket.start(key)?;
            points.push(by_time.remove(&bucket_ms).unwrap_or(TimeseriesPoint {
                bucket_ms,
                total: 0,
                error_count: 0,
                no_response_count: 0,
                avg_ms: 0.0,
                p95_ms: 0,
            }));
        }
        Ok(TimeseriesResponse {
            ok: true,
            sample_size: points.iter().map(|point| point.total).sum(),
            bucket_ms: self.bucket.millis(),
            since_ms: self.since,
            until_ms: self.until,
            points,
        })
    }
}

/// Aggregate the entire selected range in SQL. The latency histogram preserves
/// exact nearest-rank P95 without loading one Rust value per DNS request.
pub(super) fn aggregation_sql(
    table: &str,
    filter: &str,
    bucket: &str,
    integer: &str,
    float: &str,
) -> String {
    format!(
        "WITH histogram AS (
            SELECT {bucket} AS bucket_key, r.elapsed_ms, COUNT(*) AS n,
                   SUM(CASE WHEN r.error IS NOT NULL THEN 1 ELSE 0 END) AS errors,
                   SUM(CASE WHEN r.error IS NULL AND r.has_response = 0 THEN 1 ELSE 0 END) AS missing
            FROM {table} r WHERE {filter}
            GROUP BY {bucket}, r.elapsed_ms
        ), ranked AS (
            SELECT *, SUM(n) OVER (PARTITION BY bucket_key) AS bucket_total,
                   SUM(n) OVER (PARTITION BY bucket_key ORDER BY elapsed_ms ROWS UNBOUNDED PRECEDING) AS cumulative
            FROM histogram
        )
        SELECT CAST(bucket_key AS {integer}), CAST(MAX(bucket_total) AS {integer}),
               CAST(SUM(errors) AS {integer}), CAST(SUM(missing) AS {integer}),
               CAST(SUM(1.0 * elapsed_ms * n) / MAX(bucket_total) AS {float}),
               CAST(MIN(CASE WHEN cumulative * 100 >= bucket_total * 95 THEN elapsed_ms END) AS {integer})
        FROM ranked GROUP BY bucket_key ORDER BY bucket_key"
    )
}

#[cfg(test)]
mod tests {
    use super::super::model::QueryRecordFilter;
    use super::*;

    #[test]
    fn sqlite_aggregates_all_records_and_exact_weighted_p95() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE records(created_at_ms INTEGER, elapsed_ms INTEGER, error TEXT, has_response INTEGER);
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i < 20000)
             INSERT INTO records SELECT CASE WHEN i <= 10000 THEN 1000 ELSE 86401000 END,
                 CASE WHEN i % 100 < 95 THEN 10 ELSE 1000 END, NULL, 1 FROM n;
             INSERT INTO records VALUES (1000, 1000, 'failed', 0), (172801000, 20, NULL, 0);"
        ).unwrap();
        let sql = aggregation_sql(
            "records",
            "r.created_at_ms >= 0",
            "r.created_at_ms / 86400000",
            "INTEGER",
            "REAL",
        );
        let rows = conn
            .prepare(&sql)
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, f64>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows.iter().map(|row| row.1).sum::<i64>(), 20002);
        assert_eq!(
            (rows[0].1, rows[0].2, rows[0].3, rows[0].5),
            (10001, 1, 0, 1000)
        );
        assert_eq!((rows[1].1, rows[1].5), (10000, 10));
        assert!((rows[1].4 - 59.5).abs() < 0.0001);
        assert_eq!((rows[2].1, rows[2].3), (1, 1));
    }

    #[test]
    fn month_buckets_follow_leap_year_and_year_boundaries() {
        let bucket = TimeseriesBucket::Month;
        let january = bucket
            .key(
                "2024-01-31T23:59:59Z"
                    .parse::<Timestamp>()
                    .unwrap()
                    .as_millisecond(),
            )
            .unwrap();
        assert_eq!(
            bucket.start(january + 1).unwrap(),
            "2024-02-01T00:00:00Z"
                .parse::<Timestamp>()
                .unwrap()
                .as_millisecond()
        );
        assert_eq!(
            bucket.start(january + 2).unwrap() - bucket.start(january + 1).unwrap(),
            29 * 86_400_000
        );
        assert_eq!(
            bucket.start(january + 12).unwrap(),
            "2025-01-01T00:00:00Z"
                .parse::<Timestamp>()
                .unwrap()
                .as_millisecond()
        );
    }

    #[test]
    fn empty_year_has_twelve_calendar_buckets() {
        let until = "2024-12-15T12:00:00Z"
            .parse::<Timestamp>()
            .unwrap()
            .as_millisecond() as u64;
        let response = Window::new(&TimeseriesQuery {
            since_ms: None,
            until_ms: Some(until),
            filter: QueryRecordFilter::default(),
            bucket: TimeseriesBucket::Month,
            max_buckets: 12,
        })
        .unwrap()
        .finish(vec![])
        .unwrap();
        assert_eq!(response.points.len(), 12);
        assert_eq!(response.sample_size, 0);
        assert!(
            response
                .points
                .windows(2)
                .all(|pair| pair[0].bucket_ms < pair[1].bucket_ms)
        );
    }
}
