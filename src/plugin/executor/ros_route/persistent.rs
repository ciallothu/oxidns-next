// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Persistent RouterOS route loading and normalization.

use std::fmt::Display;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use ahash::AHashSet;

use super::config::PersistentArgs;
use crate::infra::error::{DnsError, Result};
use crate::infra::io::{LineClassifier, TextSource};

#[derive(Debug, Default)]
pub(super) struct ParsedPersistentRoutes {
    pub(super) all_ips: AHashSet<String>,
    pub(super) ignored_by_gateway: usize,
    pub(super) ignored_default_route: usize,
}

/// Parse always-present route list from inline args and optional files.
///
/// Accepted item formats:
/// - `1.1.1.1`
/// - `2001:db8::1`
/// - generic CIDR: `1.1.1.0/24`, `2001:db8::/64`
///
/// Entries whose IP family has no corresponding configured gateway are skipped.
pub(super) fn parse_persistent_ips(
    persistent: Option<PersistentArgs>,
    gateway4_enabled: bool,
    gateway6_enabled: bool,
) -> Result<ParsedPersistentRoutes> {
    let mut parsed = ParsedPersistentRoutes::default();
    let Some(route) = persistent else {
        return Ok(parsed);
    };

    let ips = route.ips.unwrap_or_default();
    let files = parse_persistent_files(route.files)?;
    let (all_ips, ignored_by_gateway, ignored_default_route) =
        load_persistent_ips(&ips, &files, gateway4_enabled, gateway6_enabled)?;
    parsed.all_ips = all_ips;
    parsed.ignored_by_gateway = ignored_by_gateway;
    parsed.ignored_default_route = ignored_default_route;

    Ok(parsed)
}

fn parse_persistent_files(files: Option<Vec<String>>) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let Some(files) = files else {
        return Ok(out);
    };
    for (index, file_raw) in files.into_iter().enumerate() {
        let file = file_raw.trim();
        if file.is_empty() {
            return Err(DnsError::plugin(format!(
                "ros_route persistent.files[{index}] cannot be empty"
            )));
        }
        out.push(file.to_string());
    }
    Ok(out)
}

fn load_persistent_ips(
    inline: &[String],
    files: &[String],
    gateway4_enabled: bool,
    gateway6_enabled: bool,
) -> Result<(AHashSet<String>, usize, usize)> {
    let mut out = AHashSet::new();
    let mut ignored_by_gateway = 0usize;
    let mut ignored_default_route = 0usize;

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
            let source = line.location();
            let cidr = parse_persistent_ip_item(token, source)?;
            if is_default_route_cidr(&cidr) {
                ignored_default_route = ignored_default_route.saturating_add(1);
                return Ok(());
            }
            if !is_persistent_ip_family_enabled(&cidr, gateway4_enabled, gateway6_enabled, source)?
            {
                ignored_by_gateway = ignored_by_gateway.saturating_add(1);
                return Ok(());
            }
            out.insert(cidr);
            Ok(())
        })
        .map_err(|error| DnsError::plugin(format!("failed to load persistent routes: {error}")))?;

    Ok((out, ignored_by_gateway, ignored_default_route))
}

/// Parse one persistent item and normalize into `ip/prefix`.
///
/// Rules:
/// - plain IPv4/IPv6 becomes `/32` or `/128`
/// - CIDR keeps its configured prefix and is normalized to network address
fn parse_persistent_ip_item(raw: &str, source: impl Display) -> Result<String> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(DnsError::plugin(format!("ros_route {source} is empty")));
    }

    if let Some((ip_raw, prefix_raw)) = value.split_once('/') {
        let ip = ip_raw.trim().parse::<IpAddr>().map_err(|e| {
            DnsError::plugin(format!("ros_route {source} has invalid ip '{ip_raw}': {e}"))
        })?;
        let prefix = prefix_raw.trim().parse::<u8>().map_err(|e| {
            DnsError::plugin(format!(
                "ros_route {source} has invalid prefix '{prefix_raw}': {e}"
            ))
        })?;
        let max_prefix = if ip.is_ipv4() { 32 } else { 128 };
        if prefix > max_prefix {
            return Err(DnsError::plugin(format!(
                "ros_route {source} has invalid prefix /{prefix} for {ip}, max /{max_prefix}"
            )));
        }
        let network_ip = normalize_network_ip(ip, prefix);
        return Ok(format!("{network_ip}/{prefix}"));
    }

    let ip = value.parse::<IpAddr>().map_err(|e| {
        DnsError::plugin(format!("ros_route {source} has invalid ip '{value}': {e}"))
    })?;
    let prefix = if ip.is_ipv4() { 32 } else { 128 };
    Ok(format!("{ip}/{prefix}"))
}

fn normalize_network_ip(ip: IpAddr, prefix: u8) -> IpAddr {
    match ip {
        IpAddr::V4(addr) => {
            let raw = u32::from(addr);
            let mask = if prefix == 0 {
                0
            } else {
                u32::MAX << (32 - prefix)
            };
            IpAddr::V4(Ipv4Addr::from(raw & mask))
        }
        IpAddr::V6(addr) => {
            let raw = u128::from(addr);
            let mask = if prefix == 0 {
                0
            } else {
                u128::MAX << (128 - prefix)
            };
            IpAddr::V6(Ipv6Addr::from(raw & mask))
        }
    }
}

#[inline]
fn is_default_route_cidr(cidr: &str) -> bool {
    cidr == "0.0.0.0/0" || cidr == "::/0"
}

/// Check whether this persistent route's family is enabled by gateway config.
///
/// Returns `Ok(false)` when family gateway is not configured so caller can skip
/// the item without failing plugin startup.
fn is_persistent_ip_family_enabled(
    cidr: &str,
    gateway4_enabled: bool,
    gateway6_enabled: bool,
    source: impl Display,
) -> Result<bool> {
    let (ip_raw, _) = cidr.split_once('/').ok_or_else(|| {
        DnsError::plugin(format!(
            "ros_route {source} has invalid normalized route '{cidr}'"
        ))
    })?;
    let ip = ip_raw.parse::<IpAddr>().map_err(|e| {
        DnsError::plugin(format!(
            "ros_route {source} has invalid normalized route '{cidr}': {e}"
        ))
    })?;

    match ip {
        IpAddr::V4(_) if !gateway4_enabled => Ok(false),
        IpAddr::V6(_) if !gateway6_enabled => Ok(false),
        _ => Ok(true),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;

    #[test]
    fn persistent_files_stream_into_final_set_with_counts() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            "# comment\n192.0.2.9\n192.0.2.1/24 # normalized\n0.0.0.0/0\n2001:db8::1"
        )
        .unwrap();
        let (rules, ignored_family, ignored_default) =
            load_persistent_ips(&[], &[file.path().display().to_string()], true, false).unwrap();
        assert!(rules.contains("192.0.2.9/32"));
        assert!(rules.contains("192.0.2.0/24"));
        assert_eq!(ignored_family, 1);
        assert_eq!(ignored_default, 1);
    }
}
