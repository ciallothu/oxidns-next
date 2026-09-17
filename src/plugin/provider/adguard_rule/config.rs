// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! AdGuard provider configuration only.

use serde::Deserialize;
use serde_yaml_ng::Value;

use crate::infra::error::{DnsError, Result as DnsResult};

#[derive(Debug, Clone, Deserialize, Default)]
pub(super) struct AdGuardRuleConfig {
    #[serde(default)]
    pub(super) rules: Vec<String>,
    #[serde(default)]
    pub(super) files: Vec<String>,
}

pub(super) fn parse_config(args: Option<Value>) -> DnsResult<AdGuardRuleConfig> {
    let Some(args) = args else {
        return Ok(AdGuardRuleConfig::default());
    };

    serde_yaml_ng::from_value(args)
        .map_err(|error| DnsError::plugin(format!("failed to parse adguard_rule config: {error}")))
}
