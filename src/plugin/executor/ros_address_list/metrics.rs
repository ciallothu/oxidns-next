// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! RouterOS address-list plugin metrics.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::infra::observability::metrics::{MetricLabel, MetricSample, MetricSink, MetricSource};

#[derive(Debug)]
pub(super) struct RosMetrics {
    pub(super) tag: String,
    pub(super) observe_total: AtomicU64,
    pub(super) dropped_total: AtomicU64,
    pub(super) sync_error_total: AtomicU64,
    pub(super) sync_timeout_total: AtomicU64,
    pub(super) write_success_total: AtomicU64,
    pub(super) write_error_total: AtomicU64,
    pub(super) last_write_success_timestamp_seconds: AtomicU64,
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

impl RosMetrics {
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

impl MetricSource for RosMetrics {
    fn tag(&self) -> &str {
        &self.tag
    }

    fn plugin_type(&self) -> &'static str {
        "ros_address_list"
    }

    fn collect(&self, sink: &mut dyn MetricSink) {
        let labels = [MetricLabel::new("plugin_tag", self.tag.as_str())];
        sink.emit(MetricSample::counter(
            "ros_address_list_observe_total",
            "Total address observations submitted to the RouterOS address-list manager.",
            &labels,
            self.observe_total.load(Ordering::Relaxed),
        ));
        sink.emit(MetricSample::counter(
            "ros_address_list_dropped_total",
            "Total observations dropped in async mode (queue full or channel closed).",
            &labels,
            self.dropped_total.load(Ordering::Relaxed),
        ));
        sink.emit(MetricSample::counter(
            "ros_address_list_sync_error_total",
            "Total sync-mode observations that failed at the RouterOS manager.",
            &labels,
            self.sync_error_total.load(Ordering::Relaxed),
        ));
        sink.emit(MetricSample::counter(
            "ros_address_list_sync_timeout_total",
            "Total sync-mode observations that timed out waiting for manager completion.",
            &labels,
            self.sync_timeout_total.load(Ordering::Relaxed),
        ));
        for (name, help, value) in [
            (
                "ros_address_list_pending_observations",
                "Current coalesced address-list observations waiting for processing.",
                self.pending_observations.load(Ordering::Relaxed),
            ),
            (
                "ros_address_list_managed_entries",
                "Current address-list entries retained by the manager.",
                self.managed_entries.load(Ordering::Relaxed),
            ),
            (
                "ros_address_list_last_write_success_timestamp_seconds",
                "Unix timestamp of the last successful address-list upsert.",
                self.last_write_success_timestamp_seconds
                    .load(Ordering::Relaxed),
            ),
            (
                "ros_address_list_last_reconcile_success_timestamp_seconds",
                "Unix timestamp of the last successful address-list reconcile.",
                self.last_reconcile_success_timestamp_seconds
                    .load(Ordering::Relaxed),
            ),
            (
                "ros_address_list_degraded",
                "Whether the RouterOS transport is currently degraded.",
                self.degraded.load(Ordering::Relaxed),
            ),
        ] {
            sink.emit(MetricSample::gauge(name, help, &labels, value));
        }
        for (name, help, value) in [
            (
                "ros_address_list_write_success_total",
                "Total successful RouterOS address-list upserts.",
                self.write_success_total.load(Ordering::Relaxed),
            ),
            (
                "ros_address_list_write_error_total",
                "Total failed RouterOS address-list upserts.",
                self.write_error_total.load(Ordering::Relaxed),
            ),
            (
                "ros_address_list_coalesced_total",
                "Total address-list observations merged into an existing mailbox key.",
                self.coalesced_total.load(Ordering::Relaxed),
            ),
            (
                "ros_address_list_reconnect_total",
                "Total successful RouterOS transport reconnections.",
                self.reconnect_total.load(Ordering::Relaxed),
            ),
            (
                "ros_address_list_connect_attempt_total",
                "Total RouterOS transport connection attempts.",
                self.connect_attempt_total.load(Ordering::Relaxed),
            ),
            (
                "ros_address_list_backoff_total",
                "Total RouterOS transport backoff schedules.",
                self.backoff_total.load(Ordering::Relaxed),
            ),
            (
                "ros_address_list_reconcile_error_total",
                "Total failed address-list reconcile attempts.",
                self.reconcile_error_total.load(Ordering::Relaxed),
            ),
            (
                "ros_address_list_cleanup_error_total",
                "Total address-list entries that failed shutdown cleanup.",
                self.cleanup_error_total.load(Ordering::Relaxed),
            ),
        ] {
            sink.emit(MetricSample::counter(name, help, &labels, value));
        }
    }
}
