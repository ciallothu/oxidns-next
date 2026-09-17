// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared high-performance rule matchers used by providers and matchers.

pub use domain::DomainRuleMatcher;
#[allow(unused_imports)]
pub(crate) use domain::{DomainRuleKind, split_domain_rule_expression};
pub use ip::IpPrefixMatcher;
#[allow(unused_imports)]
pub(crate) use ip::IpRuleFamily;

mod domain;
mod ip;
