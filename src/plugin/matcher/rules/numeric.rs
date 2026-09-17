// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

use ahash::AHashSet;

use crate::infra::error::{DnsError, Result as DnsResult};

pub(crate) fn parse_u16_rules(
    field: &str,
    raw_rules: &[String],
    named_parser: fn(&str) -> Option<u16>,
) -> DnsResult<AHashSet<u16>> {
    let mut parsed = AHashSet::with_capacity(raw_rules.len());
    for raw in raw_rules {
        let value = raw.trim();
        if value.is_empty() {
            continue;
        }
        let number = if let Some(number) = parse_u16_rule_token(field, value)? {
            number
        } else {
            named_parser(value).ok_or_else(|| {
                DnsError::plugin(format!(
                    "invalid {} value '{}': unsupported token",
                    field, value
                ))
            })?
        };
        parsed.insert(number);
    }
    Ok(parsed)
}

fn parse_u16_rule_token(field: &str, raw: &str) -> DnsResult<Option<u16>> {
    if let Ok(number) = raw.parse::<u16>() {
        return Ok(Some(number));
    }
    if raw.parse::<u64>().is_ok() || raw.parse::<i64>().is_ok() {
        return Err(DnsError::plugin(format!(
            "invalid {} value '{}': numeric value must be between 0 and 65535",
            field, raw
        )));
    }
    if raw.parse::<f64>().is_ok() {
        return Err(DnsError::plugin(format!(
            "invalid {} value '{}': numeric value must be an integer between 0 and 65535",
            field, raw
        )));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_test_token(raw: &str) -> Option<u16> {
        raw.eq_ignore_ascii_case("a").then_some(1)
    }

    #[test]
    fn rejects_invalid_numeric_strings() {
        for (raw, expected) in [
            ("70000", "between 0 and 65535"),
            ("-1", "between 0 and 65535"),
            ("1.0", "must be an integer"),
        ] {
            let err = parse_u16_rules("qtype", &[raw.to_string()], parse_test_token)
                .expect_err("invalid numeric string should be rejected");
            assert!(err.to_string().contains(expected));
        }
    }
}
