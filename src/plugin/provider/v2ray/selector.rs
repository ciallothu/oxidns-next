// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashSet;

use super::model::{Domain, GeoSite, attribute};
use super::parser::geosite_code;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GeoSiteSelector {
    code: String,
    attr: Option<String>,
}

pub(crate) fn normalized_selectors(selectors: &[String]) -> Vec<String> {
    selectors
        .iter()
        .map(|selector| selector.trim())
        .filter(|selector| !selector.is_empty())
        .map(|selector| selector.to_ascii_lowercase())
        .collect()
}

pub(crate) fn unique_nonempty_selectors(selectors: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut unique = Vec::new();
    for selector in selectors {
        let trimmed = selector.trim();
        if !trimmed.is_empty() && seen.insert(trimmed.to_ascii_lowercase()) {
            unique.push(trimmed.to_string());
        }
    }
    unique
}

pub(crate) fn parse_geosite_selectors(
    raw_selectors: &[String],
) -> Result<Vec<GeoSiteSelector>, String> {
    let mut selectors = Vec::new();
    for raw in raw_selectors {
        let token = raw.trim();
        if token.is_empty() {
            continue;
        }
        let (code, attr) = match token.split_once('@') {
            Some((code, attr)) => (code.trim(), Some(attr.trim())),
            None => (token, None),
        };
        if code.is_empty() {
            return Err(format!("invalid empty geosite code selector '{}'", token));
        }
        if attr.is_some_and(str::is_empty) {
            return Err(format!(
                "invalid geosite selector '{}' with empty attribute name",
                token
            ));
        }
        selectors.push(GeoSiteSelector {
            code: code.to_ascii_lowercase(),
            attr: attr.map(|value| value.to_ascii_lowercase()),
        });
    }
    Ok(selectors)
}

pub(crate) fn matched_geosite_selectors<'a>(
    entry: &GeoSite,
    selectors: &'a [GeoSiteSelector],
) -> Vec<&'a GeoSiteSelector> {
    if selectors.is_empty() {
        return Vec::new();
    }
    let code = geosite_code(entry).to_ascii_lowercase();
    selectors
        .iter()
        .filter(|selector| selector.code == code)
        .collect()
}

pub(crate) fn geosite_domain_matches_selectors(
    domain: &Domain,
    selectors: &[&GeoSiteSelector],
) -> bool {
    if selectors.is_empty() {
        return true;
    }
    selectors.iter().any(|selector| match &selector.attr {
        None => true,
        Some(attr) => domain_has_attribute(domain, attr),
    })
}

fn domain_has_attribute(domain: &Domain, attr: &str) -> bool {
    domain.attribute.iter().any(|attribute| {
        if !attribute.key.eq_ignore_ascii_case(attr) {
            return false;
        }
        match &attribute.typed_value {
            None => true,
            Some(attribute::TypedValue::BoolValue(value)) => *value,
            Some(attribute::TypedValue::IntValue(value)) => *value != 0,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_attribute() {
        let err = parse_geosite_selectors(&["cn@".to_string()]).expect_err("selector should fail");
        assert!(err.contains("empty attribute"));
    }

    #[test]
    fn unique_selectors_keep_first_spelling() {
        let selectors =
            unique_nonempty_selectors(&[" CN ".to_string(), "cn".to_string(), "".to_string()]);
        assert_eq!(selectors, vec!["CN".to_string()]);
    }
}
