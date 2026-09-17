// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! RouterOS route plugin metrics.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::infra::observability::metrics::{MetricLabel, MetricSample, MetricSink, MetricSource};

#[derive(Debug)]
pub(super) struct RosRouteMetrics {
    pub(super) tag: String,
    pub(super) observe_total: AtomicU64,
    pub(super) dropped_total: AtomicU64,
    pub(super) sync_error_total: AtomicU64,
    pub(super) sync_timeout_total: AtomicU64,
    pub(super) write_success_total: AtomicU64,
    pub(super) write_error_total: AtomicU64,
    pub(super) last_write_success_timestamp_seconds: AtomicU64,
    pub(super) delete_deferred_total: AtomicU64,
    pub(super) connection_check_error_total: AtomicU64,
    pub(super) pending_observations: AtomicU64,
    pub(super) managed_entries: AtomicU64,
    pub(super) coalesced_total: AtomicU64,
    pub(super) reconnect_total: AtomicU64,
    pub(super) connect_attempt_total: AtomicU64,
    pub(super) backoff_total: AtomicU64,
    pub(super) reconcile_error_total: AtomicU64,
    pub(super) last_reconcile_success_timestamp_seconds: AtomicU64,
    pub(super) degraded: AtomicU64,
    pub(super) cleanup_error_total: AtomicU64,
}

impl RosRouteMetrics {
    pub(super) fn new(tag: String) -> Self {
        Self {
            tag,
            observe_total: AtomicU64::new(0),
            dropped_total: AtomicU64::new(0),
            sync_error_total: AtomicU64::new(0),
            sync_timeout_total: AtomicU64::new(0),
            write_success_total: AtomicU64::new(0),
            write_error_total: AtomicU64::new(0),
            last_write_success_timestamp_seconds: AtomicU64::new(0),
            delete_deferred_total: AtomicU64::new(0),
            connection_check_error_total: AtomicU64::new(0),
            pending_observations: AtomicU64::new(0),
            managed_entries: AtomicU64::new(0),
            coalesced_total: AtomicU64::new(0),
            reconnect_total: AtomicU64::new(0),
            connect_attempt_total: AtomicU64::new(0),
            backoff_total: AtomicU64::new(0),
            reconcile_error_total: AtomicU64::new(0),
            last_reconcile_success_timestamp_seconds: AtomicU64::new(0),
            degraded: AtomicU64::new(0),
            cleanup_error_total: AtomicU64::new(0),
        }
    }
}

impl MetricSource for RosRouteMetrics {
    fn tag(&self) -> &str {
        &self.tag
    }

    fn plugin_type(&self) -> &'static str {
        "ros_route"
    }

    fn collect(&self, sink: &mut dyn MetricSink) {
        let labels = [MetricLabel::new("plugin_tag", self.tag.as_str())];
        sink.emit(MetricSample::counter(
            "ros_route_observe_total",
            "Total address observations submitted to the RouterOS route manager.",
            &labels,
            self.observe_total.load(Ordering::Relaxed),
        ));
        sink.emit(MetricSample::counter(
            "ros_route_dropped_total",
            "Total route observations dropped because the manager queue was unavailable.",
            &labels,
            self.dropped_total.load(Ordering::Relaxed),
        ));
        sink.emit(MetricSample::counter(
            "ros_route_sync_error_total",
            "Total synchronous route observations that failed without changing DNS output.",
            &labels,
            self.sync_error_total.load(Ordering::Relaxed),
        ));
        sink.emit(MetricSample::counter(
            "ros_route_sync_timeout_total",
            "Total synchronous route observations that timed out without changing DNS output.",
            &labels,
            self.sync_timeout_total.load(Ordering::Relaxed),
        ));
        sink.emit(MetricSample::counter(
            "ros_route_delete_deferred_total",
            "Total route deletions deferred because a matching RouterOS connection exists.",
            &labels,
            self.delete_deferred_total.load(Ordering::Relaxed),
        ));
        sink.emit(MetricSample::counter(
            "ros_route_connection_check_error_total",
            "Total RouterOS connection-tracking queries that failed during route deletion.",
            &labels,
            self.connection_check_error_total.load(Ordering::Relaxed),
        ));
        for (name, help, value) in [
            (
                "ros_route_pending_observations",
                "Current coalesced route observations waiting for processing.",
                self.pending_observations.load(Ordering::Relaxed),
            ),
            (
                "ros_route_managed_entries",
                "Current route entries retained by the manager.",
                self.managed_entries.load(Ordering::Relaxed),
            ),
            (
                "ros_route_last_write_success_timestamp_seconds",
                "Unix timestamp of the last successful route upsert.",
                self.last_write_success_timestamp_seconds
                    .load(Ordering::Relaxed),
            ),
            (
                "ros_route_last_reconcile_success_timestamp_seconds",
                "Unix timestamp of the last successful route reconcile.",
                self.last_reconcile_success_timestamp_seconds
                    .load(Ordering::Relaxed),
            ),
            (
                "ros_route_degraded",
                "Whether the RouterOS transport is currently degraded.",
                self.degraded.load(Ordering::Relaxed),
            ),
        ] {
            sink.emit(MetricSample::gauge(name, help, &labels, value));
        }
        for (name, help, value) in [
            (
                "ros_route_write_success_total",
                "Total successful RouterOS route upserts.",
                self.write_success_total.load(Ordering::Relaxed),
            ),
            (
                "ros_route_write_error_total",
                "Total failed RouterOS route upserts.",
                self.write_error_total.load(Ordering::Relaxed),
            ),
            (
                "ros_route_coalesced_total",
                "Total route observations merged into an existing mailbox key.",
                self.coalesced_total.load(Ordering::Relaxed),
            ),
            (
                "ros_route_reconnect_total",
                "Total successful RouterOS transport reconnections.",
                self.reconnect_total.load(Ordering::Relaxed),
            ),
            (
                "ros_route_connect_attempt_total",
                "Total RouterOS transport connection attempts.",
                self.connect_attempt_total.load(Ordering::Relaxed),
            ),
            (
                "ros_route_backoff_total",
                "Total RouterOS transport backoff schedules.",
                self.backoff_total.load(Ordering::Relaxed),
            ),
            (
                "ros_route_reconcile_error_total",
                "Total failed route reconcile attempts.",
                self.reconcile_error_total.load(Ordering::Relaxed),
            ),
            (
                "ros_route_cleanup_error_total",
                "Total route entries that failed shutdown cleanup.",
                self.cleanup_error_total.load(Ordering::Relaxed),
            ),
        ] {
            sink.emit(MetricSample::counter(name, help, &labels, value));
        }
    }
}
