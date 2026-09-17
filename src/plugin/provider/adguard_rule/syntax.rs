// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

use std::borrow::Cow;
use std::net::IpAddr;
use std::str::FromStr;

use regex::RegexBuilder;

use super::model::{DnsTypeConstraint, PatternMatcher};
use crate::core::rule_matcher::DomainRuleKind;
use crate::proto::RecordType;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum SkipReason {
    Hosts,
    Cosmetic,
    Path,
    UnsupportedModifier,
    UnknownModifier,
}

impl SkipReason {
    pub(super) const ALL: [Self; 5] = [
        Self::Hosts,
        Self::Cosmetic,
        Self::Path,
        Self::UnsupportedModifier,
        Self::UnknownModifier,
    ];

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Hosts => "hosts_style",
            Self::Cosmetic => "non_dns_cosmetic",
            Self::Path => "non_dns_url_or_path",
            Self::UnsupportedModifier => "unsupported_modifier",
            Self::UnknownModifier => "unknown_modifier",
        }
    }

    pub(super) const fn index(self) -> usize {
        match self {
            Self::Hosts => 0,
            Self::Cosmetic => 1,
            Self::Path => 2,
            Self::UnsupportedModifier => 3,
            Self::UnknownModifier => 4,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct RuleMeta<'a> {
    pub(super) pattern: &'a str,
    pub(super) is_exception: bool,
    pub(super) important: bool,
    pub(super) badfilter: bool,
    pub(super) dnstype: Option<&'a str>,
    pub(super) denyallow: Option<&'a str>,
}

impl RuleMeta<'_> {
    #[inline]
    pub(super) fn is_conditional(&self) -> bool {
        self.dnstype.is_some() || self.denyallow.is_some()
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum ParsedLine<'a> {
    Ignored,
    Skipped(SkipReason),
    Rule(RuleMeta<'a>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModifierKind {
    Important,
    Badfilter,
    Denyallow,
    Dnstype,
    Unsupported,
    Unknown,
}

impl ModifierKind {
    fn is_supported(self) -> bool {
        matches!(
            self,
            Self::Important | Self::Badfilter | Self::Denyallow | Self::Dnstype
        )
    }
}

#[derive(Debug, Clone, Copy)]
struct ModifierToken<'a> {
    kind: ModifierKind,
    value: Option<&'a str>,
}

/// Parse one physical line without performing I/O or logging.
///
/// `leading_comment` is a lexical annotation supplied by the text source. It
/// is interpreted only after cosmetic syntax, because cosmetic rules can also
/// begin with `#` markers.
pub(super) fn parse_line<'a>(
    raw: &'a str,
    leading_comment: Option<&str>,
) -> Result<ParsedLine<'a>, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(ParsedLine::Ignored);
    }

    if is_non_dns_rule(raw) {
        return Ok(ParsedLine::Skipped(SkipReason::Cosmetic));
    }
    if leading_comment.is_some() {
        return Ok(ParsedLine::Ignored);
    }
    if is_hosts_style_rule(raw) {
        return Ok(ParsedLine::Skipped(SkipReason::Hosts));
    }

    let (body, is_exception) = raw
        .strip_prefix("@@")
        .map(|body| (body.trim(), true))
        .unwrap_or((raw, false));
    let Some((pattern, modifiers)) = split_pattern_and_modifiers(body)? else {
        return Ok(ParsedLine::Skipped(SkipReason::Path));
    };
    let pattern = pattern.trim();
    if pattern.is_empty() {
        if modifiers.is_some_and(has_supported_modifier) {
            return Err("empty rule pattern".to_string());
        }
        if modifiers.is_some() {
            return Ok(ParsedLine::Skipped(SkipReason::UnsupportedModifier));
        }
        return Err("empty rule pattern".to_string());
    }
    if !pattern.starts_with('/')
        && pattern
            .chars()
            .any(|ch| matches!(ch, '/' | '?' | ':' | '#') || ch.is_ascii_whitespace())
    {
        return Ok(ParsedLine::Skipped(SkipReason::Path));
    }

    let mut meta = RuleMeta {
        pattern,
        is_exception,
        important: false,
        badfilter: false,
        dnstype: None,
        denyallow: None,
    };

    if let Some(modifiers) = modifiers {
        for modifier in modifiers
            .split(',')
            .map(str::trim)
            .filter(|modifier| !modifier.is_empty())
        {
            let token = parse_modifier_token(modifier);
            match token.kind {
                ModifierKind::Important => meta.important = true,
                ModifierKind::Badfilter => meta.badfilter = true,
                ModifierKind::Denyallow => {
                    meta.denyallow = Some(
                        token
                            .value
                            .ok_or_else(|| "denyallow modifier requires a value".to_string())?,
                    );
                }
                ModifierKind::Dnstype => {
                    meta.dnstype = Some(
                        token
                            .value
                            .ok_or_else(|| "dnstype modifier requires a value".to_string())?,
                    );
                }
                ModifierKind::Unsupported => {
                    return Ok(ParsedLine::Skipped(SkipReason::UnsupportedModifier));
                }
                ModifierKind::Unknown => {
                    return Ok(ParsedLine::Skipped(SkipReason::UnknownModifier));
                }
            }
        }
    }

    Ok(ParsedLine::Rule(meta))
}

fn has_supported_modifier(modifiers: &str) -> bool {
    modifiers
        .split(',')
        .map(str::trim)
        .filter(|modifier| !modifier.is_empty())
        .any(|modifier| parse_modifier_token(modifier).kind.is_supported())
}

fn parse_modifier_token(raw: &str) -> ModifierToken<'_> {
    let (name, value) = raw
        .split_once('=')
        .map(|(name, value)| (name.trim(), Some(value.trim())))
        .unwrap_or((raw, None));
    let kind = if name.eq_ignore_ascii_case("important") {
        ModifierKind::Important
    } else if name.eq_ignore_ascii_case("badfilter") {
        ModifierKind::Badfilter
    } else if name.eq_ignore_ascii_case("denyallow") {
        ModifierKind::Denyallow
    } else if name.eq_ignore_ascii_case("dnstype") {
        ModifierKind::Dnstype
    } else if name.eq_ignore_ascii_case("dnsrewrite")
        || name.eq_ignore_ascii_case("client")
        || name.eq_ignore_ascii_case("ctag")
    {
        ModifierKind::Unsupported
    } else {
        ModifierKind::Unknown
    };
    ModifierToken { kind, value }
}

/// Split a DNS pattern from its modifiers. A leading slash denotes a regular
/// expression only when a closing delimiter is present at the end or directly
/// before `$modifiers`; other leading-slash forms are URL/path rules.
fn split_pattern_and_modifiers(raw: &str) -> Result<Option<(&str, Option<&str>)>, String> {
    if raw.starts_with('/') {
        if raw.ends_with('/') && raw.len() > 1 {
            return Ok(Some((raw, None)));
        }
        if let Some(delimiter) = raw.rfind("/$") {
            let modifiers = &raw[delimiter + 2..];
            return Ok(Some((&raw[..=delimiter], Some(modifiers))));
        }
        return Ok(None);
    }

    Ok(Some(match raw.split_once('$') {
        Some((pattern, modifiers)) => (pattern, Some(modifiers)),
        None => (raw, None),
    }))
}

#[derive(Debug)]
pub(super) struct RuleDetails {
    pub(super) dnstype: Option<DnsTypeConstraint>,
    pub(super) denyallow: Vec<String>,
}

pub(super) fn parse_rule_details(meta: &RuleMeta<'_>) -> Result<RuleDetails, String> {
    Ok(RuleDetails {
        dnstype: meta.dnstype.map(parse_dnstype).transpose()?,
        denyallow: meta
            .denyallow
            .map(parse_denyallow)
            .transpose()?
            .unwrap_or_default(),
    })
}

pub(super) fn compile_pattern(raw: &str) -> Result<PatternMatcher, String> {
    if raw.starts_with('/') {
        return compile_regex_pattern(raw);
    }

    let normalized = normalize_domain(raw);
    if normalized.is_empty() {
        return Err("empty domain pattern".to_string());
    }

    if let Some(domain) = normalized
        .strip_prefix("||")
        .and_then(|value| value.strip_suffix('^'))
        && is_simple_hostname(domain)
    {
        return Ok(PatternMatcher::Domain(domain.to_string().into_boxed_str()));
    }

    if !normalized.contains('*') && !normalized.contains('^') && !normalized.contains('|') {
        return Ok(PatternMatcher::Exact(normalized.into_boxed_str()));
    }

    if let Some(prefix) = normalized.strip_prefix('|')
        && !prefix.contains('*')
        && !prefix.contains('^')
        && !prefix.contains('|')
    {
        return Ok(PatternMatcher::Prefix(prefix.to_string().into_boxed_str()));
    }

    if let Some(suffix) = normalized.strip_suffix('|')
        && !suffix.contains('*')
        && !suffix.contains('^')
        && !suffix.contains('|')
    {
        return Ok(PatternMatcher::Suffix(suffix.to_string().into_boxed_str()));
    }

    let regex = translate_pattern_to_regex(&normalized)?;
    let regex = RegexBuilder::new(&regex)
        .case_insensitive(false)
        .build()
        .map_err(|e| format!("failed to build adguard mask '{}': {}", raw, e))?;
    Ok(PatternMatcher::Regex(regex))
}

/// Compile a non-conditional rule directly into a caller-provided sink. The
/// normalized or translated value is borrowed for the duration of the call;
/// only the final matcher representation remains allocated afterward.
pub(super) fn with_fast_rule<T>(
    raw: &str,
    sink: impl FnOnce(DomainRuleKind, &str) -> Result<T, String>,
) -> Result<T, String> {
    if raw.starts_with('/') {
        let body = regex_body(raw)?;
        return sink(DomainRuleKind::Regexp, body);
    }

    let normalized = normalize_domain_cow(raw);
    if normalized.is_empty() {
        return Err("empty domain pattern".to_string());
    }

    if let Some(domain) = normalized
        .strip_prefix("||")
        .and_then(|value| value.strip_suffix('^'))
        && is_simple_hostname(domain)
    {
        return sink(DomainRuleKind::Domain, domain);
    }
    if !normalized.contains('*') && !normalized.contains('^') && !normalized.contains('|') {
        return sink(DomainRuleKind::Full, normalized.as_ref());
    }

    let regex = translate_pattern_to_regex(&normalized)?;
    sink(DomainRuleKind::Regexp, &regex)
}

pub(super) fn fast_rule_kind(raw: &str) -> Result<DomainRuleKind, String> {
    with_fast_rule(raw, |kind, _| Ok(kind))
}

/// Validate the matcher representation without retaining a compiled matcher.
pub(super) fn validate_pattern(raw: &str) -> Result<(), String> {
    with_fast_rule(raw, |kind, value| {
        if kind == DomainRuleKind::Regexp {
            RegexBuilder::new(value)
                .case_insensitive(true)
                .build()
                .map_err(|error| format!("invalid regex '{raw}': {error}"))?;
        }
        Ok(())
    })
}

pub(super) fn parse_denyallow(raw: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for domain in raw
        .split('|')
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let normalized = normalize_domain(domain);
        if !is_simple_hostname(&normalized) {
            return Err(format!("invalid denyallow domain '{}'", domain));
        }
        out.push(normalized);
    }
    Ok(out)
}

pub(super) fn parse_dnstype(raw: &str) -> Result<DnsTypeConstraint, String> {
    let mut include = Vec::new();
    let mut exclude = Vec::new();

    for token in raw
        .split('|')
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let (negated, rr_type_raw) = token
            .strip_prefix('~')
            .map(|rest| (true, rest))
            .unwrap_or((false, token));
        let rr_type = RecordType::from_str(&rr_type_raw.to_ascii_uppercase())
            .map_err(|_| format!("invalid dnstype value '{}'", rr_type_raw))?;
        if negated {
            exclude.push(rr_type);
        } else {
            include.push(rr_type);
        }
    }

    if !include.is_empty() {
        include.sort_unstable_by_key(|item| u16::from(*item));
        include.dedup();
        return Ok(DnsTypeConstraint::Allow(include));
    }

    exclude.sort_unstable_by_key(|item| u16::from(*item));
    exclude.dedup();
    Ok(DnsTypeConstraint::Deny(exclude))
}

pub(super) fn canonical_pattern_key(raw: &str) -> String {
    if raw.starts_with('/') {
        raw.to_string()
    } else {
        normalize_domain(raw)
    }
}

pub(super) fn normalize_domain(raw: &str) -> String {
    normalize_domain_cow(raw).into_owned()
}

fn normalize_domain_cow(raw: &str) -> Cow<'_, str> {
    let trimmed = raw.trim().trim_end_matches('.');
    if trimmed.bytes().any(|byte| byte.is_ascii_uppercase()) {
        Cow::Owned(trimmed.to_ascii_lowercase())
    } else {
        Cow::Borrowed(trimmed)
    }
}

fn compile_regex_pattern(raw: &str) -> Result<PatternMatcher, String> {
    let body = regex_body(raw)?;
    let regex = RegexBuilder::new(body)
        .case_insensitive(true)
        .build()
        .map_err(|e| format!("invalid regex '{}': {}", raw, e))?;
    Ok(PatternMatcher::Regex(regex))
}

fn regex_body(raw: &str) -> Result<&str, String> {
    let body = raw
        .strip_prefix('/')
        .and_then(|value| value.strip_suffix('/'))
        .ok_or_else(|| "unterminated regex rule".to_string())?;
    if body.trim().is_empty() {
        return Err("empty regex rule".to_string());
    }
    Ok(body)
}

fn translate_pattern_to_regex(raw: &str) -> Result<String, String> {
    let mut rest = raw;
    let mut prefix = String::new();
    if let Some(stripped) = rest.strip_prefix("||") {
        prefix.push_str(r"(^|.+\.)");
        rest = stripped;
    } else if let Some(stripped) = rest.strip_prefix('|') {
        prefix.push('^');
        rest = stripped;
    }

    let mut suffix = String::new();
    if let Some(stripped) = rest.strip_suffix('|') {
        suffix.push('$');
        rest = stripped;
    }

    let mut out = prefix;
    for ch in rest.chars() {
        match ch {
            '*' => out.push_str(".*"),
            '^' => out.push('$'),
            '.' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '\\' => {
                out.push('\\');
                out.push(ch);
            }
            '|' => return Err(format!("unsupported interior '|' in pattern '{}'", raw)),
            other => out.push(other),
        }
    }
    out.push_str(&suffix);
    Ok(out)
}

fn is_hosts_style_rule(raw: &str) -> bool {
    let mut parts = raw.split_whitespace();
    let Some(first) = parts.next() else {
        return false;
    };
    let Some(_second) = parts.next() else {
        return false;
    };
    first.parse::<IpAddr>().is_ok()
}

fn is_non_dns_rule(raw: &str) -> bool {
    raw.contains("##")
        || raw.contains("#@#")
        || raw.contains("#$#")
        || raw.contains("#@$#")
        || raw.contains("#%#")
        || raw.contains("#@%#")
        || raw.contains("#?#")
        || raw.contains("#@?#")
}

fn is_simple_hostname(raw: &str) -> bool {
    !raw.is_empty()
        && raw
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '_')
}
