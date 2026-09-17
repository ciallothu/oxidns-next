// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! `client_ip_from_ecs` executor plugin.
//!
//! Replaces the request-local client IP with the address carried by an EDNS
//! Client Subnet (ECS) option, such as one added by dnsmasq `--add-subnet`.
//! `args` is an optional array of trusted original client IPs or CIDR prefixes;
//! missing or empty args trust only IPv4 and IPv6 loopback. ECS is used only
//! when the transport peer matches this allow-list and the ECS source prefix
//! identifies a complete host (`/32` for IPv4 or `/128` for IPv6). Place the
//! plugin before client-IP matchers and recorders. Rejected non-host prefixes
//! emit a warning; otherwise it performs no allocation, locking, or I/O on the
//! request path.

use std::net::SocketAddr;

use async_trait::async_trait;

use crate::config::types::PluginConfig;
use crate::core::context::DnsContext;
use crate::core::rule_matcher::IpPrefixMatcher;
use crate::infra::error::{DnsError, Result};
use crate::infra::network::ip::normalize_ipv4_mapped_ip;
use crate::plugin::executor::{ExecStep, Executor};
use crate::plugin::{Plugin, PluginFactory, UninitializedPlugin};
use crate::plugin_factory;
use crate::proto::{EdnsCode, EdnsOption};

const DEFAULT_TRUSTED_SOURCES: [&str; 2] = ["127.0.0.1", "::1"];

#[derive(Debug)]
struct ClientIpFromEcs {
    tag: String,
    trusted_sources: IpPrefixMatcher,
}

#[async_trait]
impl Plugin for ClientIpFromEcs {
    fn tag(&self) -> &str {
        &self.tag
    }

    async fn init(&mut self, _context: &crate::plugin::PluginInitContext<'_>) -> Result<()> {
        Ok(())
    }

    async fn destroy(&self) -> Result<()> {
        Ok(())
    }
}

#[async_trait]
impl Executor for ClientIpFromEcs {
    #[hotpath::measure]
    async fn execute(&self, context: &mut DnsContext) -> Result<ExecStep> {
        let original_peer = context.peer_addr();
        if !self
            .trusted_sources
            .contains_ip(normalize_ipv4_mapped_ip(original_peer.ip()))
        {
            return Ok(ExecStep::Next);
        }

        let ecs_addr = context
            .request()
            .edns()
            .as_ref()
            .and_then(|edns| edns.option(EdnsCode::Subnet))
            .and_then(|option| match option {
                EdnsOption::Subnet(subnet) => {
                    let addr = subnet.addr();
                    let source_prefix = subnet.source_prefix();
                    let expected_prefix = if addr.is_ipv4() { 32 } else { 128 };
                    if source_prefix == expected_prefix {
                        Some(normalize_ipv4_mapped_ip(addr))
                    } else {
                        tracing::debug!(
                            plugin = %self.tag,
                            ecs_addr = %addr,
                            source_prefix,
                            expected_prefix,
                            "ignored ECS client address with non-host source prefix"
                        );
                        None
                    }
                }
                _ => None,
            });

        if let Some(ip) = ecs_addr {
            context.set_peer_addr(SocketAddr::new(ip, original_peer.port()));
        }

        Ok(ExecStep::Next)
    }
}

#[derive(Debug, Clone)]
#[plugin_factory("client_ip_from_ecs")]
pub struct ClientIpFromEcsFactory;

impl PluginFactory for ClientIpFromEcsFactory {
    fn create(
        &self,
        plugin_config: &PluginConfig,
        _init_context: &crate::plugin::PluginInitContext<'_>,
    ) -> Result<UninitializedPlugin> {
        let rules = parse_trusted_sources(plugin_config.args.clone())?;
        Ok(UninitializedPlugin::Executor(Box::new(ClientIpFromEcs {
            tag: plugin_config.tag.clone(),
            trusted_sources: build_trusted_sources(rules)?,
        })))
    }

    fn quick_setup(&self, tag: &str, param: Option<String>) -> Result<UninitializedPlugin> {
        let rules = param
            .unwrap_or_default()
            .split([',', ' ', '\t'])
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect();
        Ok(UninitializedPlugin::Executor(Box::new(ClientIpFromEcs {
            tag: tag.to_string(),
            trusted_sources: build_trusted_sources(rules)?,
        })))
    }
}

fn parse_trusted_sources(args: Option<serde_yaml_ng::Value>) -> Result<Vec<String>> {
    let Some(args) = args else {
        return Ok(default_trusted_sources());
    };

    serde_yaml_ng::from_value(args).map_err(|error| {
        DnsError::plugin(format!(
            "failed to parse client_ip_from_ecs args as an IP/CIDR array: {}",
            error
        ))
    })
}

fn build_trusted_sources(rules: Vec<String>) -> Result<IpPrefixMatcher> {
    let rules = if rules.iter().all(|rule| rule.trim().is_empty()) {
        default_trusted_sources()
    } else {
        rules
    };
    let mut matcher = IpPrefixMatcher::default();
    for raw_rule in rules {
        let rule = raw_rule.trim();
        if rule.is_empty() {
            continue;
        }
        matcher.add_rule(rule).map_err(|error| {
            DnsError::plugin(format!(
                "invalid client_ip_from_ecs trusted source '{}': {}",
                rule, error
            ))
        })?;
    }

    matcher.finalize_compact();
    Ok(matcher)
}

fn default_trusted_sources() -> Vec<String> {
    DEFAULT_TRUSTED_SOURCES
        .iter()
        .map(|source| (*source).to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

    use super::*;
    use crate::plugin::test_utils::test_context;
    use crate::proto::{ClientSubnet, EdnsOption};

    fn add_ecs(context: &mut DnsContext, addr: IpAddr, prefix: u8) {
        context
            .request_mut()
            .ensure_edns_mut()
            .insert(EdnsOption::Subnet(ClientSubnet::new(addr, prefix, 0)));
    }

    fn plugin(rules: &[&str]) -> ClientIpFromEcs {
        ClientIpFromEcs {
            tag: "client_ip_from_ecs".to_string(),
            trusted_sources: build_trusted_sources(
                rules.iter().map(|rule| (*rule).to_string()).collect(),
            )
            .unwrap(),
        }
    }

    #[tokio::test]
    async fn replaces_ip_and_preserves_port() {
        let mut context = test_context();
        context.set_peer_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 5353)));
        add_ecs(&mut context, IpAddr::V4(Ipv4Addr::new(192, 0, 2, 123)), 32);

        assert_eq!(
            plugin(&["127.0.0.1"]).execute(&mut context).await.unwrap(),
            ExecStep::Next
        );
        assert_eq!(
            context.peer_addr(),
            SocketAddr::from((Ipv4Addr::new(192, 0, 2, 123), 5353))
        );
    }

    #[tokio::test]
    async fn supports_ipv6_ecs() {
        let mut context = test_context();
        let ecs_ip = Ipv6Addr::new(0x2001, 0xDB8, 1, 2, 0, 0, 0, 0);
        add_ecs(&mut context, IpAddr::V6(ecs_ip), 128);

        plugin(&["127.0.0.1"]).execute(&mut context).await.unwrap();
        assert_eq!(context.peer_addr().ip(), IpAddr::V6(ecs_ip));
    }

    #[tokio::test]
    async fn leaves_client_ip_unchanged_without_usable_ecs() {
        let mut without_ecs = test_context();
        let original = without_ecs.peer_addr();
        plugin(&["127.0.0.1"])
            .execute(&mut without_ecs)
            .await
            .unwrap();
        assert_eq!(without_ecs.peer_addr(), original);

        let mut zero_prefix = test_context();
        let original = zero_prefix.peer_addr();
        add_ecs(&mut zero_prefix, IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0);
        plugin(&["127.0.0.1"])
            .execute(&mut zero_prefix)
            .await
            .unwrap();
        assert_eq!(zero_prefix.peer_addr(), original);
    }

    #[tokio::test]
    async fn ignores_ecs_with_non_host_prefix() {
        for (addr, prefix) in [
            (IpAddr::V4(Ipv4Addr::new(192, 0, 2, 0)), 24),
            (
                IpAddr::V6(Ipv6Addr::new(0x2001, 0xDB8, 1, 2, 0, 0, 0, 0)),
                64,
            ),
        ] {
            let mut context = test_context();
            let original = context.peer_addr();
            add_ecs(&mut context, addr, prefix);

            plugin(&["127.0.0.1"]).execute(&mut context).await.unwrap();

            assert_eq!(context.peer_addr(), original);
        }
    }

    #[tokio::test]
    async fn ignores_ecs_from_untrusted_source() {
        let mut context = test_context();
        context.set_peer_addr(SocketAddr::from((Ipv4Addr::new(198, 51, 100, 9), 5353)));
        add_ecs(&mut context, IpAddr::V4(Ipv4Addr::new(192, 0, 2, 123)), 32);

        plugin(&["10.0.0.0/24"])
            .execute(&mut context)
            .await
            .unwrap();

        assert_eq!(
            context.peer_addr(),
            SocketAddr::from((Ipv4Addr::new(198, 51, 100, 9), 5353))
        );
    }

    #[tokio::test]
    async fn accepts_ecs_from_trusted_cidr() {
        let mut context = test_context();
        context.set_peer_addr(SocketAddr::from((Ipv4Addr::new(10, 0, 0, 42), 5353)));
        add_ecs(&mut context, IpAddr::V4(Ipv4Addr::new(192, 0, 2, 123)), 32);

        plugin(&["10.0.0.0/24"])
            .execute(&mut context)
            .await
            .unwrap();

        assert_eq!(
            context.peer_addr(),
            SocketAddr::from((Ipv4Addr::new(192, 0, 2, 123), 5353))
        );
    }

    #[test]
    fn validates_trusted_source_args() {
        let defaults = build_trusted_sources(Vec::new()).unwrap();
        assert!(defaults.contains_ip(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert!(defaults.contains_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert!(build_trusted_sources(vec!["lan".to_string()]).is_err());
        assert!(build_trusted_sources(vec!["10.0.0.0/24".to_string()]).is_ok());
    }

    #[test]
    fn missing_args_use_loopback_defaults() {
        assert_eq!(
            parse_trusted_sources(None).unwrap(),
            vec!["127.0.0.1".to_string(), "::1".to_string()]
        );
    }
}
