// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

use std::sync::Arc;

use crate::infra::error::{DnsError, Result as DnsResult};
use crate::plugin::PluginInitContext;
use crate::plugin::dependency::DependencySpec;
use crate::plugin::provider::Provider;

pub(crate) fn provider_dependency_specs(
    field_prefix: &str,
    tags: Vec<String>,
) -> Vec<DependencySpec> {
    tags.into_iter()
        .enumerate()
        .map(|(idx, tag)| DependencySpec::provider(format!("{field_prefix}[{idx}]"), tag))
        .collect()
}

pub(crate) fn resolve_provider_tags(
    context: &PluginInitContext<'_>,
    tags: &[String],
    matcher_name: &str,
) -> DnsResult<Vec<Arc<dyn Provider>>> {
    let mut providers = Vec::with_capacity(tags.len());
    for (idx, tag) in tags.iter().enumerate() {
        let field = format!("{}.provider_tags[{}]", matcher_name, idx);
        providers.push(context.provider(&field, tag)?);
    }
    Ok(providers)
}

pub(crate) fn ensure_ip_capable_providers(
    providers: &[Arc<dyn Provider>],
    matcher_name: &str,
    matcher_tag: &str,
    tags: &[String],
) -> DnsResult<()> {
    for (idx, provider) in providers.iter().enumerate() {
        if !provider.supports_ip_matching() {
            let tag = tags.get(idx).map(String::as_str).unwrap_or("<unknown>");
            return Err(DnsError::plugin(format!(
                "{} matcher '{}' requires provider '{}' to support IP matching",
                matcher_name, matcher_tag, tag
            )));
        }
    }
    Ok(())
}

pub(crate) fn ensure_domain_capable_providers(
    providers: &[Arc<dyn Provider>],
    matcher_name: &str,
    matcher_tag: &str,
    tags: &[String],
) -> DnsResult<()> {
    for (idx, provider) in providers.iter().enumerate() {
        if !provider.supports_domain_matching() {
            let tag = tags.get(idx).map(String::as_str).unwrap_or("<unknown>");
            return Err(DnsError::plugin(format!(
                "{} matcher '{}' requires provider '{}' to support domain matching",
                matcher_name, matcher_tag, tag
            )));
        }
    }
    Ok(())
}
