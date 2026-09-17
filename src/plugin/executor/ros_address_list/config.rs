// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! RouterOS address-list plugin configuration.

use std::time::Duration;

use ahash::AHashSet;
use serde::Deserialize;
use serde_yaml_ng::Value;
use tracing::warn;

use super::api::{
    DEFAULT_CONNECT_TIMEOUT_SECS, DEFAULT_RECEIVE_TIMEOUT_SECS, DEFAULT_SEND_TIMEOUT_SECS,
    MikrotikApiTimeouts,
};
use super::model::AddressListKey;
use super::persistent::parse_persistent_items;
use crate::infra::error::{DnsError, Result};
use crate::infra::system::deserialize_duration_option;
use crate::plugin::executor::routeros::transport::{RouterOsConnectionConfig, RouterOsTlsArgs};

/// Default lower TTL clamp for dynamic address-list entries.
pub(super) const DEFAULT_MIN_TTL: u32 = 60;
/// Default upper TTL clamp for dynamic address-list entries.
pub(super) const DEFAULT_MAX_TTL: u32 = 3600;
/// Default execution mode keeps RouterOS writes off the DNS request path.
pub(super) const DEFAULT_ASYNC_MODE: bool = true;
/// Default shutdown behavior removes plugin-owned RouterOS entries.
pub(super) const DEFAULT_CLEANUP_ON_SHUTDOWN: bool = true;
/// Default comment prefix used to mark OxiDNS-owned RouterOS rows.
pub(super) const DEFAULT_COMMENT_PREFIX: &str = "fdns";
pub(super) const DEFAULT_WAIT_TIMEOUT: Duration = Duration::from_secs(8);
pub(super) const DEFAULT_QUEUE_CAPACITY: usize = 16_384;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct MikrotikConfigArgs {
    /// RouterOS API endpoint, usually `<host>:8728`.
    address: Option<String>,
    /// RouterOS login username.
    username: Option<String>,
    /// RouterOS login password.
    password: Option<String>,
    /// RouterOS API connection timeout in seconds.
    connect_timeout: Option<u64>,
    /// RouterOS API command send timeout in seconds.
    send_timeout: Option<u64>,
    /// RouterOS API response receive timeout in seconds.
    receive_timeout: Option<u64>,
    /// Optional RouterOS API-SSL configuration. Presence enables TLS.
    tls: Option<RouterOsTlsArgs>,
    /// Whether post stage waits RouterOS writes (`false`) or queues work
    /// (`true`).
    #[serde(rename = "async")]
    async_mode: Option<bool>,
    /// Maximum time synchronous mode waits for manager completion.
    #[serde(default, deserialize_with = "deserialize_duration_option")]
    wait_timeout: Option<Duration>,
    /// Maximum number of distinct keys in each manager queue stage.
    queue_capacity: Option<usize>,
    /// IPv4 address-list name for observed IPv4 answers.
    address_list4: Option<String>,
    /// IPv6 address-list name for observed IPv6 answers.
    address_list6: Option<String>,
    /// Prefix used in RouterOS comments to mark OxiDNS-managed entries.
    /// Defaults to `oxi` when omitted.
    comment_prefix: Option<String>,
    /// Always-present address-list items that should never expire.
    persistent: Option<PersistentArgs>,
    /// Minimum effective TTL clamp (seconds) for observed records.
    min_ttl: Option<u32>,
    /// Maximum effective TTL clamp (seconds) for observed records.
    max_ttl: Option<u32>,
    /// Optional fixed TTL override (seconds) for dynamic observed records.
    /// `0` means do not set RouterOS timeout.
    fixed_ttl: Option<u32>,
    /// Whether to clean managed address-list entries on shutdown.
    cleanup_on_shutdown: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct PersistentArgs {
    /// Inline always-present IPs/CIDRs. Plain IP is normalized to host entry.
    pub(super) ips: Option<Vec<String>>,
    /// File list that provides always-present IPs/CIDRs.
    pub(super) files: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub(super) struct MikrotikConfig {
    /// Connection settings consumed when the API transport is constructed.
    pub(super) connection: Option<RouterOsConnectionConfig>,
    /// Async mode switch for post stage writes.
    pub(super) async_mode: bool,
    pub(super) wait_timeout: Duration,
    pub(super) queue_capacity: usize,
    /// IPv4 address-list name managed by this plugin.
    pub(super) address_list4: Option<String>,
    /// IPv6 address-list name managed by this plugin.
    pub(super) address_list6: Option<String>,
    /// Full persistent desired set after merging inline and file sources.
    pub(super) persistent_items: AHashSet<AddressListKey>,
    /// Prefix used in RouterOS comments to mark plugin ownership.
    pub(super) comment_prefix: String,
    /// Minimum TTL clamp for dynamic entries.
    pub(super) min_ttl: u32,
    /// Maximum TTL clamp for dynamic entries.
    pub(super) max_ttl: u32,
    /// Optional fixed TTL override for dynamic entries.
    /// `0` means do not set RouterOS timeout.
    pub(super) fixed_ttl: Option<u32>,
    /// Whether shutdown should remove owned entries from RouterOS.
    pub(super) cleanup_on_shutdown: bool,
}

impl MikrotikConfigArgs {
    /// Validate user-facing config and normalize it into a runtime-ready form.
    ///
    /// This is also where persistent input sources are parsed into normalized
    /// `AddressListKey` values so the manager does not need to re-interpret
    /// human-facing YAML at runtime.
    fn into_config(self, emit_warnings: bool) -> Result<MikrotikConfig> {
        let address = required_non_empty(self.address, "address")?;
        let username = required_non_empty(self.username, "username")?;
        let password = required_non_empty(self.password, "password")?;
        let api_timeouts = MikrotikApiTimeouts::from_secs(
            timeout_secs(
                self.connect_timeout,
                "connect_timeout",
                DEFAULT_CONNECT_TIMEOUT_SECS,
            )?,
            timeout_secs(self.send_timeout, "send_timeout", DEFAULT_SEND_TIMEOUT_SECS)?,
            timeout_secs(
                self.receive_timeout,
                "receive_timeout",
                DEFAULT_RECEIVE_TIMEOUT_SECS,
            )?,
        );
        let connection = RouterOsConnectionConfig::new(
            address.clone(),
            username,
            password,
            api_timeouts,
            self.tls,
        )?;
        let address_list4 = optional_non_empty(self.address_list4);
        let address_list6 = optional_non_empty(self.address_list6);
        if address_list4.is_none() && address_list6.is_none() {
            return Err(DnsError::plugin(
                "ros_address_list requires at least one of address_list4 or address_list6",
            ));
        }

        let comment_prefix = optional_non_empty(self.comment_prefix)
            .unwrap_or_else(|| DEFAULT_COMMENT_PREFIX.to_string());
        validate_comment_token("comment_prefix", &comment_prefix)?;

        let min_ttl = self.min_ttl.unwrap_or(DEFAULT_MIN_TTL);
        let max_ttl = self.max_ttl.unwrap_or(DEFAULT_MAX_TTL);
        if min_ttl > max_ttl {
            return Err(DnsError::plugin(format!(
                "ros_address_list ttl range is invalid: min_ttl({min_ttl}) > max_ttl({max_ttl})"
            )));
        }
        let fixed_ttl = self.fixed_ttl;
        let wait_timeout = positive_duration(
            self.wait_timeout.unwrap_or(DEFAULT_WAIT_TIMEOUT),
            "wait_timeout",
        )?;
        let queue_capacity = positive_usize(
            self.queue_capacity.unwrap_or(DEFAULT_QUEUE_CAPACITY),
            "queue_capacity",
        )?;

        let parsed_persistent = parse_persistent_items(
            self.persistent,
            address_list4.as_deref(),
            address_list6.as_deref(),
        )?;
        if emit_warnings && parsed_persistent.ignored_by_family > 0 {
            warn!(
                ignored = parsed_persistent.ignored_by_family,
                "ros_address_list persistent ignored entries without corresponding address list family"
            );
        }

        Ok(MikrotikConfig {
            connection: Some(connection),
            async_mode: self.async_mode.unwrap_or(DEFAULT_ASYNC_MODE),
            wait_timeout,
            queue_capacity,
            address_list4,
            address_list6,
            persistent_items: parsed_persistent.all_items,
            comment_prefix,
            min_ttl,
            max_ttl,
            fixed_ttl,
            cleanup_on_shutdown: self
                .cleanup_on_shutdown
                .unwrap_or(DEFAULT_CLEANUP_ON_SHUTDOWN),
        })
    }
}

pub(super) fn parse_plugin_config(
    args: Option<Value>,
    emit_warnings: bool,
) -> Result<MikrotikConfig> {
    let Some(args) = args else {
        return Err(DnsError::plugin("ros_address_list plugin requires args"));
    };
    let raw = serde_yaml_ng::from_value::<MikrotikConfigArgs>(args)
        .map_err(|e| DnsError::plugin(format!("failed to parse ros_address_list config: {e}")))?;
    raw.into_config(emit_warnings)
}

fn required_non_empty(value: Option<String>, field: &str) -> Result<String> {
    let Some(value) = value else {
        return Err(DnsError::plugin(format!(
            "ros_address_list '{field}' is required"
        )));
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(DnsError::plugin(format!(
            "ros_address_list '{field}' cannot be empty"
        )));
    }
    Ok(trimmed.to_string())
}

fn optional_non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn timeout_secs(value: Option<u64>, field: &str, default_secs: u64) -> Result<u64> {
    match value {
        Some(0) => Err(DnsError::plugin(format!(
            "ros_address_list '{field}' must be greater than 0 seconds"
        ))),
        Some(value) => Ok(value),
        None => Ok(default_secs),
    }
}

fn positive_duration(value: Duration, field: &str) -> Result<Duration> {
    if value.is_zero() {
        return Err(DnsError::plugin(format!(
            "ros_address_list '{field}' must be greater than 0"
        )));
    }
    Ok(value)
}

fn positive_usize(value: usize, field: &str) -> Result<usize> {
    if value == 0 {
        return Err(DnsError::plugin(format!(
            "ros_address_list '{field}' must be greater than 0"
        )));
    }
    Ok(value)
}

#[inline]
fn contains_comment_delimiter(value: &str) -> bool {
    value.contains(';') || value.contains('=')
}

pub(super) fn validate_comment_token(field: &str, value: &str) -> Result<()> {
    if contains_comment_delimiter(value) {
        return Err(DnsError::plugin(format!(
            "ros_address_list '{field}' cannot contain ';' or '='"
        )));
    }
    Ok(())
}
