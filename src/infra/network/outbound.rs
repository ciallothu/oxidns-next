// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared outbound connection profiles.
//!
//! Outbound profiles describe how process-owned clients connect to external
//! services: which resolver to use and whether a proxy is involved. Callers
//! such as the shared HTTP client consume the resolved runtime policy instead
//! of parsing SOCKS5 or bootstrap DNS settings on their own.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use crate::config::types::{
    NetworkOutboundConfig, OutboundProfileConfig, OutboundProxyConfig, OutboundResolverConfig,
};
use crate::infra::error::{DnsError, Result};
use crate::infra::network::deadline::QueryDeadline;
use crate::infra::network::proxy::{Socks5Opt, parse_socks5_opt};
use crate::infra::network::resolver::BootstrapResolver;

const DEFAULT_BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub(crate) struct OutboundPolicy {
    resolver: ResolverPolicy,
    proxy: ProxyPolicy,
}

impl OutboundPolicy {
    pub(crate) fn system(proxy: Option<Socks5Opt>) -> Self {
        Self {
            resolver: ResolverPolicy::System,
            proxy: ProxyPolicy::from_socks5(proxy),
        }
    }

    pub(crate) fn proxy(&self) -> Option<Socks5Opt> {
        match &self.proxy {
            ProxyPolicy::Direct => None,
            ProxyPolicy::Socks5(socks5) => Some(socks5.clone()),
        }
    }

    pub(crate) async fn resolve_host(&self, host: &str, port: u16) -> Result<IpAddr> {
        self.resolver.resolve_host(host, port).await
    }

    pub(crate) fn resolves_before_proxy(&self) -> bool {
        self.resolver.resolves_before_proxy()
    }

    fn with_proxy(mut self, proxy: ProxyPolicy) -> Self {
        self.proxy = proxy;
        self
    }
}

impl Default for OutboundPolicy {
    fn default() -> Self {
        Self::system(None)
    }
}

#[derive(Debug, Clone)]
enum ResolverPolicy {
    System,
    Nameservers(Arc<BootstrapResolver>),
}

impl ResolverPolicy {
    fn resolves_before_proxy(&self) -> bool {
        matches!(self, Self::Nameservers(_))
    }

    async fn resolve_host(&self, host: &str, port: u16) -> Result<IpAddr> {
        match self {
            Self::System => resolve_system(host, port).await,
            Self::Nameservers(resolver) => {
                resolver
                    .resolve(host, QueryDeadline::new(DEFAULT_BOOTSTRAP_TIMEOUT))
                    .await
            }
        }
    }
}

#[derive(Debug, Clone)]
enum ProxyPolicy {
    Direct,
    Socks5(Socks5Opt),
}

impl ProxyPolicy {
    fn from_socks5(socks5: Option<Socks5Opt>) -> Self {
        socks5.map_or(Self::Direct, Self::Socks5)
    }
}

#[derive(Debug, Default)]
pub(crate) struct OutboundRuntime {
    default: Option<String>,
    profiles: HashMap<String, OutboundPolicy>,
}

impl OutboundRuntime {
    pub(crate) fn from_config(config: &NetworkOutboundConfig) -> Result<Self> {
        let mut profiles = HashMap::new();
        for (name, profile) in &config.profiles {
            profiles.insert(name.clone(), policy_from_profile(name, profile)?);
        }
        if let Some(default) = config.default.as_deref()
            && !profiles.contains_key(default)
        {
            return Err(DnsError::config(format!(
                "network.outbound.default references unknown profile '{}'",
                default
            )));
        }
        Ok(Self {
            default: config.default.clone(),
            profiles,
        })
    }

    pub(crate) fn resolve_policy(
        &self,
        outbound_ref: Option<&str>,
        legacy_socks5: Option<Socks5Opt>,
    ) -> Result<OutboundPolicy> {
        let mut policy = match outbound_ref
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .or(self.default.as_deref())
        {
            Some(name) => self.profiles.get(name).cloned().ok_or_else(|| {
                DnsError::config(format!("unknown network outbound profile '{}'", name))
            })?,
            None => OutboundPolicy::system(None),
        };

        if legacy_socks5.is_some() {
            policy = policy.with_proxy(ProxyPolicy::from_socks5(legacy_socks5));
        }

        Ok(policy)
    }
}

fn policy_from_profile(name: &str, profile: &OutboundProfileConfig) -> Result<OutboundPolicy> {
    let resolver = match &profile.resolver {
        Some(OutboundResolverConfig::Mode(mode)) if mode.trim().eq_ignore_ascii_case("system") => {
            ResolverPolicy::System
        }
        Some(OutboundResolverConfig::Mode(mode)) => {
            return Err(DnsError::config(format!(
                "network.outbound profile '{}' has invalid resolver mode '{}'",
                name, mode
            )));
        }
        Some(OutboundResolverConfig::Nameservers {
            nameservers,
            ip_version,
        }) => {
            let servers = nameservers
                .servers()
                .into_iter()
                .map(str::trim)
                .filter(|server| !server.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            ResolverPolicy::Nameservers(Arc::new(BootstrapResolver::new(servers, *ip_version)?))
        }
        None => ResolverPolicy::System,
    };

    let proxy = match &profile.proxy {
        Some(OutboundProxyConfig::Mode(mode))
            if mode.trim().eq_ignore_ascii_case("none")
                || mode.trim().eq_ignore_ascii_case("direct") =>
        {
            ProxyPolicy::Direct
        }
        Some(OutboundProxyConfig::Mode(mode)) => {
            return Err(DnsError::config(format!(
                "network.outbound profile '{}' has invalid proxy mode '{}'",
                name, mode
            )));
        }
        Some(OutboundProxyConfig::Socks5 { socks5 }) => {
            ProxyPolicy::Socks5(parse_socks5_opt(socks5).ok_or_else(|| {
                DnsError::config(format!(
                    "network.outbound profile '{}' has invalid socks5 proxy '{}'",
                    name, socks5
                ))
            })?)
        }
        None => ProxyPolicy::Direct,
    };

    Ok(OutboundPolicy { resolver, proxy })
}

async fn resolve_system(host: &str, port: u16) -> Result<IpAddr> {
    let mut addrs = tokio::net::lookup_host((host, port)).await.map_err(|err| {
        DnsError::protocol(format!(
            "Async DNS resolution failed for '{}': {}",
            host, err
        ))
    })?;
    addrs.next().map(|addr| addr.ip()).ok_or_else(|| {
        DnsError::protocol(format!("Async DNS returned no addresses for '{}'", host))
    })
}

fn global_slot() -> &'static Mutex<Arc<OutboundRuntime>> {
    static GLOBAL: OnceLock<Mutex<Arc<OutboundRuntime>>> = OnceLock::new();
    GLOBAL.get_or_init(|| Mutex::new(Arc::new(OutboundRuntime::default())))
}

pub(crate) fn install_global(config: &NetworkOutboundConfig) -> Result<()> {
    let runtime = Arc::new(OutboundRuntime::from_config(config)?);
    *global_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = runtime;
    Ok(())
}

pub(crate) fn clear_global() {
    *global_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Arc::new(OutboundRuntime::default());
}

pub(crate) fn global() -> Arc<OutboundRuntime> {
    global_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::{NameserverConfig, NetworkOutboundConfig};

    #[test]
    fn test_resolve_policy_defaults_to_direct_system() {
        let runtime = OutboundRuntime::default();
        let policy = runtime
            .resolve_policy(None, None)
            .expect("default policy should resolve");
        assert!(policy.proxy().is_none());
    }

    #[test]
    fn test_resolve_policy_uses_named_profile() {
        let config = NetworkOutboundConfig {
            default: None,
            profiles: HashMap::from([(
                "oversea".to_string(),
                OutboundProfileConfig {
                    resolver: Some(OutboundResolverConfig::Nameservers {
                        nameservers: NameserverConfig::One("1.1.1.1:53".to_string()),
                        ip_version: Some(4),
                    }),
                    proxy: Some(OutboundProxyConfig::Socks5 {
                        socks5: "127.0.0.1:1080".to_string(),
                    }),
                },
            )]),
        };
        let runtime = OutboundRuntime::from_config(&config).expect("outbound config should parse");
        let policy = runtime
            .resolve_policy(Some("oversea"), None)
            .expect("profile should resolve");
        assert!(policy.proxy().is_some());
        assert!(policy.resolves_before_proxy());
    }

    #[test]
    fn test_system_resolver_does_not_resolve_before_proxy() {
        let policy = OutboundPolicy::system(Some(Socks5Opt {
            username: None,
            password: None,
            socket_addr: "127.0.0.1:1080".parse().expect("socket addr should parse"),
        }));

        assert!(!policy.resolves_before_proxy());
    }
}
