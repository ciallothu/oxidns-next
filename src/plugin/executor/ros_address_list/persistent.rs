// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Persistent RouterOS address-list rule loading and normalization.

use std::fmt::Display;
use std::net::IpAddr;

use ahash::AHashSet;

use super::config::PersistentArgs;
use super::model::{AddressListFamily, AddressListKey};
use crate::infra::error::{DnsError, Result};
use crate::infra::io::{LineClassifier, TextSource};

#[derive(Debug, Default)]
pub(super) struct ParsedPersistentItems {
    /// Final desired set after merging inline and file sources.
    pub(super) all_items: AHashSet<AddressListKey>,
    /// Count of items skipped because that family is not configured.
    pub(super) ignored_by_family: usize,
}

/// Parse `persistent` config into normalized address-list keys.
///
/// The parser performs all expensive normalization and validation at startup:
/// plain IPs become host prefixes, CIDRs are masked to network form, and each
/// item is bound to the correct IPv4/IPv6 address-list name.
pub(super) fn parse_persistent_items(
    persistent: Option<PersistentArgs>,
    address_list4: Option<&str>,
    address_list6: Option<&str>,
) -> Result<ParsedPersistentItems> {
    let mut parsed = ParsedPersistentItems::default();
    let Some(persistent) = persistent else {
        return Ok(parsed);
    };

    let ips = persistent.ips.unwrap_or_default();
    let files = parse_persistent_files(persistent.files)?;
    let (all_items, ignored_by_family) =
        load_persistent_items(&ips, &files, address_list4, address_list6)?;
    parsed.all_items = all_items;
    parsed.ignored_by_family = ignored_by_family;
    Ok(parsed)
}

pub(super) fn parse_persistent_files(files: Option<Vec<String>>) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let Some(files) = files else {
        return Ok(out);
    };
    for (index, file_raw) in files.into_iter().enumerate() {
        let file = file_raw.trim();
        if file.is_empty() {
            return Err(DnsError::plugin(format!(
                "ros_address_list persistent.files[{index}] cannot be empty"
            )));
        }
        out.push(file.to_string());
    }
    Ok(out)
}

/// Parse one file body into normalized persistent items.
///
/// Files use the same item grammar as inline YAML. Empty lines and `#` comments
/// are ignored. Family-mismatched entries are skipped rather than failing
/// startup so shared source files can contain both IPv4 and IPv6 items.
#[cfg(test)]
pub(super) fn load_persistent_items_from_content(
    source_prefix: &str,
    content: &str,
    address_list4: Option<&str>,
    address_list6: Option<&str>,
) -> Result<(AHashSet<AddressListKey>, usize)> {
    let mut out = AHashSet::new();
    let mut ignored_by_family = 0usize;

    for (line_no, line) in content.lines().enumerate() {
        let token = line.split('#').next().unwrap_or_default().trim();
        if token.is_empty() {
            continue;
        }

        let source = format!("{source_prefix} line {}", line_no + 1);
        match parse_persistent_item(token, source.as_str(), address_list4, address_list6)? {
            Some(key) => {
                out.insert(key);
            }
            None => {
                ignored_by_family = ignored_by_family.saturating_add(1);
            }
        }
    }

    Ok((out, ignored_by_family))
}

fn load_persistent_items(
    inline: &[String],
    files: &[String],
    address_list4: Option<&str>,
    address_list6: Option<&str>,
) -> Result<(AHashSet<AddressListKey>, usize)> {
    let mut out = AHashSet::new();
    let mut ignored_by_family = 0usize;

    TextSource::new("persistent.ips", inline, files)
        .scan(&LineClassifier::new(&["#"]), |line| -> Result<()> {
            if line.annotations().blank || line.annotations().leading_comment.is_some() {
                return Ok(());
            }
            let raw = line.raw();
            let token = raw.split('#').next().unwrap_or_default().trim();
            if token.is_empty() {
                return Ok(());
            }
            match parse_persistent_item(token, line.location(), address_list4, address_list6)? {
                Some(key) => {
                    out.insert(key);
                }
                None => ignored_by_family = ignored_by_family.saturating_add(1),
            }
            Ok(())
        })
        .map_err(|error| {
            DnsError::plugin(format!(
                "failed to load persistent address-list rules: {error}"
            ))
        })?;

    Ok((out, ignored_by_family))
}

/// Parse one human-facing persistent item and bind it to the correct list.
///
/// Return `Ok(None)` when the item is valid but its IP family has no configured
/// target list, allowing callers to ignore mixed-family source files cleanly.
fn parse_persistent_item(
    raw: &str,
    source: impl Display,
    address_list4: Option<&str>,
    address_list6: Option<&str>,
) -> Result<Option<AddressListKey>> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(DnsError::plugin(format!(
            "ros_address_list {source} is empty"
        )));
    }

    let (ip, prefix) = if let Some((ip_raw, prefix_raw)) = value.split_once('/') {
        let ip = ip_raw.trim().parse::<IpAddr>().map_err(|e| {
            DnsError::plugin(format!(
                "ros_address_list {source} has invalid ip '{ip_raw}': {e}"
            ))
        })?;
        let prefix = prefix_raw.trim().parse::<u8>().map_err(|e| {
            DnsError::plugin(format!(
                "ros_address_list {source} has invalid prefix '{prefix_raw}': {e}"
            ))
        })?;
        (ip, prefix)
    } else {
        let ip = value.parse::<IpAddr>().map_err(|e| {
            DnsError::plugin(format!(
                "ros_address_list {source} has invalid ip '{value}': {e}"
            ))
        })?;
        let family = AddressListFamily::from_ip(ip);
        (ip, family.host_prefix())
    };

    let family = AddressListFamily::from_ip(ip);
    let list = match family {
        AddressListFamily::Ipv4 => address_list4,
        AddressListFamily::Ipv6 => address_list6,
    };
    let Some(list) = list else {
        return Ok(None);
    };

    AddressListKey::new_with_prefix(ip, prefix, list.to_string())
        .ok_or_else(|| {
            DnsError::plugin(format!(
                "ros_address_list {source} has invalid prefix /{prefix} for {ip}"
            ))
        })
        .map(Some)
}
