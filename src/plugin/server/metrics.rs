// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared request metrics for server plugins.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::infra::clock::AppClock;
use crate::infra::observability::metrics::{MetricLabel, MetricSample, MetricSink, MetricSource};
use crate::plugin::server::RequestExit;

/// Shared per-server-plugin request metrics.
///
/// One instance is created per server plugin tag and shared by every request
/// handle owned by that plugin.
#[derive(Debug)]
pub(crate) struct ServerMetrics {
    tag: String,
    protocol: &'static str,
    request_total: AtomicU64,
    completed_total: AtomicU64,
    controlled_total: AtomicU64,
    failed_total: AtomicU64,
    inflight: AtomicU64,
    latency_count: AtomicU64,
    latency_sum_ms: AtomicU64,
}

impl ServerMetrics {
    pub(crate) fn new(tag: String, protocol: &'static str) -> Self {
        Self {
            tag,
            protocol,
            request_total: AtomicU64::new(0),
            completed_total: AtomicU64::new(0),
            controlled_total: AtomicU64::new(0),
            failed_total: AtomicU64::new(0),
            inflight: AtomicU64::new(0),
            latency_count: AtomicU64::new(0),
            latency_sum_ms: AtomicU64::new(0),
        }
    }

    #[inline]
    pub(super) fn on_request_start(&self) -> u64 {
        self.request_total.fetch_add(1, Ordering::Relaxed);
        self.inflight.fetch_add(1, Ordering::Relaxed);
        AppClock::elapsed_millis()
    }

    #[inline]
    pub(super) fn on_request_finish(&self, start_ms: u64, exit: RequestExit) {
        self.inflight.fetch_sub(1, Ordering::Relaxed);
        let counter = match exit {
            RequestExit::Completed => &self.completed_total,
            RequestExit::Controlled => &self.controlled_total,
            RequestExit::Failed => &self.failed_total,
        };
        counter.fetch_add(1, Ordering::Relaxed);
        let elapsed = AppClock::elapsed_millis().saturating_sub(start_ms);
        self.latency_count.fetch_add(1, Ordering::Relaxed);
        self.latency_sum_ms.fetch_add(elapsed, Ordering::Relaxed);
    }
}

impl MetricSource for ServerMetrics {
    fn tag(&self) -> &str {
        &self.tag
    }

    fn plugin_type(&self) -> &'static str {
        "server"
    }

    fn collect(&self, sink: &mut dyn MetricSink) {
        let labels = [
            MetricLabel::new("plugin_tag", self.tag.as_str()),
            MetricLabel::new("protocol", self.protocol),
        ];
        sink.emit(MetricSample::counter(
            "server_request_total",
            "Total inbound DNS requests handled by the server.",
            &labels,
            self.request_total.load(Ordering::Relaxed),
        ));
        sink.emit(MetricSample::counter(
            "server_completed_total",
            "Total requests that finished by running the executor chain to completion.",
            &labels,
            self.completed_total.load(Ordering::Relaxed),
        ));
        sink.emit(MetricSample::counter(
            "server_controlled_total",
            "Total requests stopped early by an executor (stop/return).",
            &labels,
            self.controlled_total.load(Ordering::Relaxed),
        ));
        sink.emit(MetricSample::counter(
            "server_failed_total",
            "Total requests that produced a SERVFAIL because the entry executor failed.",
            &labels,
            self.failed_total.load(Ordering::Relaxed),
        ));
        sink.emit(MetricSample::gauge(
            "server_inflight",
            "Current number of in-flight requests being handled by the server.",
            &labels,
            self.inflight.load(Ordering::Relaxed),
        ));
        sink.emit(MetricSample::counter(
            "server_latency_count",
            "Total requests included in server latency statistics.",
            &labels,
            self.latency_count.load(Ordering::Relaxed),
        ));
        sink.emit(MetricSample::counter(
            "server_latency_sum_ms",
            "Total server request handling latency in milliseconds.",
            &labels,
            self.latency_sum_ms.load(Ordering::Relaxed),
        ));
    }
}
