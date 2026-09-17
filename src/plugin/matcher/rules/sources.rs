// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::core::rule_matcher::{
    DomainRuleMatcher, IpPrefixMatcher, IpRuleFamily, split_domain_rule_expression,
};
use crate::infra::error::{DnsError, Result as DnsResult};
use crate::infra::io::{LineClassifier, TextSource};

#[cfg(test)]
pub(crate) fn parse_ip_prefix_matcher(
    field: &str,
    raw_rules: &[String],
) -> DnsResult<IpPrefixMatcher> {
    let mut matcher = IpPrefixMatcher::default();
    for raw in raw_rules {
        let value = raw.trim();
        if value.is_empty() {
            continue;
        }
        matcher.add_rule(value).map_err(|error| {
            DnsError::plugin(format!("invalid {} rule '{}': {}", field, value, error))
        })?;
    }
    matcher.finalize_compact();
    Ok(matcher)
}

pub(crate) fn parse_domain_rules_and_set_tags(
    raw_rules: Vec<String>,
    field: &str,
) -> DnsResult<(DomainRuleMatcher, Vec<String>)> {
    let (inline_rules, set_tags, files) = split_rule_sources(raw_rules);

    let mut domain_rules = DomainRuleMatcher::default();
    TextSource::new(field, &inline_rules, &files)
        .scan(&LineClassifier::new(&["#"]), |line| {
            if line.annotations().blank || line.annotations().leading_comment.is_some() {
                return Ok(());
            }
            let (kind, value) = split_domain_rule_expression(line.trimmed());
            domain_rules
                .add_rule(kind, value, "")
                .map_err(|error| format!("invalid {field} domain rule: {error}"))
        })
        .map_err(|error| DnsError::plugin(error.to_string()))?;
    domain_rules.finalize().map_err(DnsError::plugin)?;
    Ok((domain_rules, set_tags))
}

pub(crate) fn validate_non_empty_domain_rules_or_set_tags(
    field: &str,
    domain_rules: &DomainRuleMatcher,
    set_tags: &[String],
    set_name: &str,
) -> DnsResult<()> {
    if !domain_rules.has_rules() && set_tags.is_empty() {
        return Err(DnsError::plugin(format!(
            "{} matcher requires at least one domain rule or {} tag",
            field, set_name
        )));
    }
    Ok(())
}

pub(crate) fn parse_ip_rules_and_set_tags(
    raw_rules: Vec<String>,
    field: &str,
) -> DnsResult<(IpPrefixMatcher, Vec<String>)> {
    let (inline_rules, set_tags, files) = split_rule_sources(raw_rules);
    let (v4, v6) = count_ip_capacities(&inline_rules, &files, field)?;
    let mut matcher = IpPrefixMatcher::default();
    matcher.reserve_rules(v4, v6);
    scan_ip_rules(field, &inline_rules, &files, |rule| {
        matcher
            .add_rule(rule)
            .map_err(|error| format!("invalid {field} IP rule '{rule}': {error}"))
    })?;
    matcher.finalize_compact();
    Ok((matcher, set_tags))
}

pub(crate) fn validate_non_empty_ip_rules_or_set_tags(
    field: &str,
    ip_rules: &IpPrefixMatcher,
    set_tags: &[String],
    set_name: &str,
) -> DnsResult<()> {
    if !ip_rules.has_v4_rules() && !ip_rules.has_v6_rules() && set_tags.is_empty() {
        return Err(DnsError::plugin(format!(
            "{} matcher requires at least one IP rule or {} tag",
            field, set_name
        )));
    }
    Ok(())
}

pub(crate) fn split_rule_sources(
    raw_rules: Vec<String>,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut inline_rules = Vec::new();
    let mut set_tags = Vec::new();
    let mut files = Vec::new();

    for raw in raw_rules {
        let token = raw.trim();
        if token.is_empty() {
            continue;
        }
        if let Some(tag) = token.strip_prefix('$') {
            if !tag.trim().is_empty() {
                set_tags.push(tag.trim().to_string());
            }
        } else if let Some(path) = token.strip_prefix('&') {
            if !path.trim().is_empty() {
                files.push(path.trim().to_string());
            }
        } else {
            inline_rules.push(token.to_string());
        }
    }
    (inline_rules, set_tags, files)
}

/// Extract provider references without opening files or compiling rules.
pub(crate) fn provider_tags_from_rules(raw_rules: &[String]) -> Vec<String> {
    raw_rules
        .iter()
        .filter_map(|raw| raw.trim().strip_prefix('$'))
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(str::to_owned)
        .collect()
}

fn count_ip_capacities(
    inline: &[String],
    files: &[String],
    field: &str,
) -> DnsResult<(usize, usize)> {
    let mut v4 = 0usize;
    let mut v6 = 0usize;
    scan_ip_rules(field, inline, files, |rule| {
        match IpPrefixMatcher::classify_rule(rule)
            .map_err(|error| format!("invalid {field} IP rule '{rule}': {error}"))?
        {
            IpRuleFamily::V4 => v4 += 1,
            IpRuleFamily::V6 => v6 += 1,
        }
        Ok(())
    })?;
    Ok((v4, v6))
}

fn scan_ip_rules<F>(
    field: &str,
    inline: &[String],
    files: &[String],
    mut visitor: F,
) -> DnsResult<()>
where
    F: FnMut(&str) -> Result<(), String>,
{
    TextSource::new(field, inline, files)
        .scan(&LineClassifier::new(&["#"]), |line| {
            if line.annotations().blank || line.annotations().leading_comment.is_some() {
                return Ok(());
            }
            visitor(line.trimmed())
        })
        .map_err(|error| DnsError::plugin(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_sources_are_classified() {
        let (inline, tags, files) = split_rule_sources(vec![
            "a.com".to_string(),
            "$set_a".to_string(),
            "&/tmp/rules.txt".to_string(),
            "  ".to_string(),
        ]);
        assert_eq!(inline, vec!["a.com"]);
        assert_eq!(tags, vec!["set_a"]);
        assert_eq!(files, vec!["/tmp/rules.txt"]);
    }

    #[test]
    fn dependency_tags_do_not_touch_file_sources() {
        let rules = vec![
            "&/definitely/missing/rules.txt".to_string(),
            "$set_a".to_string(),
        ];
        assert_eq!(provider_tags_from_rules(&rules), vec!["set_a"]);
    }
}
