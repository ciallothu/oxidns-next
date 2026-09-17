// SPDX-FileCopyrightText: 2026 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! `response` executor plugin.
//!
//! Builds an explicit DNS response from pre-parsed zone-style records. The
//! configured answer, authority, and additional sections replace any response
//! already present in the [`DnsContext`]. Record templates are parsed during
//! plugin construction; request-time work is limited to cloning records and
//! resolving the optional `{qname}` / `{qclass}` owner and class placeholders.
//!
//! Configuration accepts a base DNS `rcode`, response flags, and three arrays
//! of one-record zone snippets. `{qname}` is valid only as an RR owner and
//! `{qclass}` only as an RR class. Both refer to the first request question.
//! `short_circuit` defaults to `true`, making the plugin a terminal policy
//! stage unless explicitly configured otherwise.

use std::str::FromStr;

use async_trait::async_trait;
use serde::Deserialize;
use serde_yaml_ng::Value;
use zoneparser::{ParseOptions, parse_str as parse_zone_str};

use crate::config::types::PluginConfig;
use crate::core::context::DnsContext;
use crate::infra::error::{DnsError, Result};
use crate::plugin::executor::{ExecStep, Executor};
use crate::plugin::{Plugin, PluginFactory, UninitializedPlugin};
use crate::plugin_factory;
use crate::proto::{DNSClass, Name, Question, Rcode, Record, RecordType};

const QNAME_PLACEHOLDER: &str = "{qname}";
const QCLASS_PLACEHOLDER: &str = "{qclass}";
const QNAME_SENTINEL: &str = "response-placeholder.oxidns.invalid.";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseConfig {
    /// DNS response code, expressed as a base numeric code or mnemonic.
    #[serde(default)]
    rcode: Option<Value>,
    /// Records placed in the Answer section.
    #[serde(default)]
    answers: Vec<String>,
    /// Records placed in the Authority section.
    #[serde(default)]
    authorities: Vec<String>,
    /// Records placed in the Additional section.
    #[serde(default)]
    additionals: Vec<String>,
    /// Set the authoritative-answer header bit.
    #[serde(default)]
    authoritative: bool,
    /// Set the authenticated-data header bit.
    #[serde(default)]
    authentic_data: bool,
    /// Stop the executor chain after setting the response.
    #[serde(default = "default_short_circuit")]
    short_circuit: bool,
}

impl Default for ResponseConfig {
    fn default() -> Self {
        Self {
            rcode: None,
            answers: Vec::new(),
            authorities: Vec::new(),
            additionals: Vec::new(),
            authoritative: false,
            authentic_data: false,
            short_circuit: default_short_circuit(),
        }
    }
}

const fn default_short_circuit() -> bool {
    true
}

#[derive(Debug, Clone)]
struct RecordTemplate {
    record: Record,
    dynamic_name: bool,
    dynamic_class: bool,
}

#[derive(Debug)]
struct TemplateToken {
    raw: String,
    quoted: bool,
}

impl RecordTemplate {
    fn instantiate(&self, question: Option<&Question>) -> Result<Record> {
        if !self.dynamic_name && !self.dynamic_class {
            return Ok(self.record.clone());
        }

        let question = question.ok_or_else(|| {
            DnsError::plugin(
                "response template uses {qname} or {qclass}, but request has no question",
            )
        })?;
        let name = if self.dynamic_name {
            question.name().clone()
        } else {
            self.record.name().clone()
        };
        let class = if self.dynamic_class {
            question.qclass()
        } else {
            self.record.class()
        };

        Ok(Record::from_arc_rdata_with_class(
            name,
            self.record.ttl(),
            class,
            self.record.data_arc(),
        ))
    }
}

#[derive(Debug)]
struct ResponseExecutor {
    tag: String,
    rcode: Rcode,
    answers: Vec<RecordTemplate>,
    authorities: Vec<RecordTemplate>,
    additionals: Vec<RecordTemplate>,
    authoritative: bool,
    authentic_data: bool,
    short_circuit: bool,
}

#[async_trait]
impl Plugin for ResponseExecutor {
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
impl Executor for ResponseExecutor {
    #[hotpath::measure]
    async fn execute(&self, context: &mut DnsContext) -> Result<ExecStep> {
        let question = context.request().first_question();
        let mut response = context.request().response(self.rcode);
        response.set_authoritative(self.authoritative);
        response.set_authentic_data(self.authentic_data);

        extend_section(response.answers_mut(), &self.answers, question)?;
        extend_section(response.authorities_mut(), &self.authorities, question)?;
        extend_section(response.additionals_mut(), &self.additionals, question)?;

        context.set_response(response);
        Ok(if self.short_circuit {
            ExecStep::Stop
        } else {
            ExecStep::Next
        })
    }
}

fn extend_section(
    target: &mut Vec<Record>,
    templates: &[RecordTemplate],
    question: Option<&Question>,
) -> Result<()> {
    target.reserve(templates.len());
    for template in templates {
        target.push(template.instantiate(question)?);
    }
    Ok(())
}

#[derive(Debug, Clone)]
#[plugin_factory("response")]
pub struct ResponseFactory;

impl PluginFactory for ResponseFactory {
    fn create(
        &self,
        plugin_config: &PluginConfig,
        _init_context: &crate::plugin::PluginInitContext<'_>,
    ) -> Result<UninitializedPlugin> {
        let config = parse_config(plugin_config.args.clone())?;
        let rcode = parse_rcode(config.rcode.as_ref())?;

        Ok(UninitializedPlugin::Executor(Box::new(ResponseExecutor {
            tag: plugin_config.tag.clone(),
            rcode,
            answers: parse_section("answers", &config.answers)?,
            authorities: parse_section("authorities", &config.authorities)?,
            additionals: parse_section("additionals", &config.additionals)?,
            authoritative: config.authoritative,
            authentic_data: config.authentic_data,
            short_circuit: config.short_circuit,
        })))
    }
}

fn parse_config(args: Option<Value>) -> Result<ResponseConfig> {
    let Some(args) = args else {
        return Ok(ResponseConfig::default());
    };

    serde_yaml_ng::from_value(args)
        .map_err(|err| DnsError::plugin(format!("failed to parse response config: {err}")))
}

fn parse_rcode(value: Option<&Value>) -> Result<Rcode> {
    let Some(value) = value else {
        return Ok(Rcode::NoError);
    };

    let token = match value {
        Value::String(value) => value.as_str(),
        Value::Number(value) => {
            let code = value.as_u64().ok_or_else(|| {
                DnsError::plugin("response rcode must be a decimal integer or mnemonic")
            })?;
            return parse_base_rcode(code.to_string().as_str());
        }
        _ => {
            return Err(DnsError::plugin(
                "response rcode must be a decimal integer or mnemonic",
            ));
        }
    };

    parse_base_rcode(token)
}

fn parse_base_rcode(raw: &str) -> Result<Rcode> {
    let rcode = Rcode::from_token(raw.trim())
        .ok_or_else(|| DnsError::plugin("response rcode must be a decimal integer or mnemonic"))?;
    if rcode.value() > 15 {
        return Err(DnsError::plugin(
            "response only supports base DNS rcodes 0..15",
        ));
    }
    Ok(rcode)
}

fn parse_section(section: &str, records: &[String]) -> Result<Vec<RecordTemplate>> {
    records
        .iter()
        .enumerate()
        .map(|(index, raw)| parse_record_template(section, index + 1, raw))
        .collect()
}

fn parse_record_template(section: &str, index: usize, raw: &str) -> Result<RecordTemplate> {
    let qname_count = raw.matches(QNAME_PLACEHOLDER).count();
    let qclass_count = raw.matches(QCLASS_PLACEHOLDER).count();
    if qname_count > 1 || qclass_count > 1 {
        return Err(DnsError::plugin(format!(
            "response {section} record #{index} may use each placeholder at most once"
        )));
    }

    let dynamic_name = qname_count == 1;
    let dynamic_class = qclass_count == 1;
    validate_placeholder_positions(section, index, raw, dynamic_name, dynamic_class)?;
    let parsed = raw
        .replace(QNAME_PLACEHOLDER, QNAME_SENTINEL)
        .replace(QCLASS_PLACEHOLDER, "CH");
    let records = parse_zone_str(parsed.as_str(), &ParseOptions::default()).map_err(|err| {
        DnsError::plugin(format!(
            "failed to parse response {section} record #{index}: {err}"
        ))
    })?;
    let [record] = records.as_slice() else {
        return Err(DnsError::plugin(format!(
            "response {section} record #{index} must contain exactly one resource record"
        )));
    };

    if dynamic_name
        && record.name()
            != &Name::from_ascii(QNAME_SENTINEL).expect("response qname sentinel should parse")
    {
        return Err(DnsError::plugin(format!(
            "response {section} record #{index}: {{qname}} is only valid as the record owner"
        )));
    }
    if dynamic_class && record.class() != DNSClass::CH {
        return Err(DnsError::plugin(format!(
            "response {section} record #{index}: {{qclass}} is only valid as the record class"
        )));
    }

    Ok(RecordTemplate {
        record: record.clone(),
        dynamic_name,
        dynamic_class,
    })
}

fn validate_placeholder_positions(
    section: &str,
    index: usize,
    raw: &str,
    dynamic_name: bool,
    dynamic_class: bool,
) -> Result<()> {
    if !dynamic_name && !dynamic_class {
        return Ok(());
    }

    let tokens = tokenize_record_template(raw).map_err(|message| {
        DnsError::plugin(format!(
            "response {section} record #{index}: invalid record template: {message}"
        ))
    })?;

    if dynamic_name
        && !matches!(
            tokens.first(),
            Some(TemplateToken { raw, quoted: false }) if raw == QNAME_PLACEHOLDER
        )
    {
        return Err(DnsError::plugin(format!(
            "response {section} record #{index}: {{qname}} is only valid as the record owner"
        )));
    }

    if dynamic_class {
        let record_type_index = tokens
            .iter()
            .enumerate()
            .skip(1)
            .find_map(|(index, token)| is_record_type_token(token).then_some(index));
        let has_class_placeholder = record_type_index.is_some_and(|record_type_index| {
            tokens[1..record_type_index]
                .iter()
                .any(|token| !token.quoted && token.raw == QCLASS_PLACEHOLDER)
        });
        if !has_class_placeholder {
            return Err(DnsError::plugin(format!(
                "response {section} record #{index}: {{qclass}} is only valid as the record class"
            )));
        }
    }

    Ok(())
}

fn is_record_type_token(token: &TemplateToken) -> bool {
    if token.quoted {
        return false;
    }

    let upper = token.raw.to_ascii_uppercase();
    upper
        .strip_prefix("TYPE")
        .is_some_and(|code| code.parse::<u16>().is_ok())
        || RecordType::from_str(upper.as_str()).is_ok()
}

fn tokenize_record_template(raw: &str) -> std::result::Result<Vec<TemplateToken>, &'static str> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut escaping = false;
    let mut just_closed_quote = false;

    for ch in raw.chars() {
        if quoted {
            current.push(ch);
            if escaping {
                escaping = false;
            } else if ch == '\\' {
                escaping = true;
            } else if ch == '"' {
                current.pop();
                tokens.push(TemplateToken {
                    raw: std::mem::take(&mut current),
                    quoted: true,
                });
                quoted = false;
                just_closed_quote = true;
            }
            continue;
        }

        if escaping {
            current.push(ch);
            escaping = false;
            continue;
        }

        if just_closed_quote {
            if ch.is_whitespace() {
                just_closed_quote = false;
                continue;
            }
            return Err("quoted token must be followed by whitespace");
        }

        match ch {
            '\\' => {
                current.push(ch);
                escaping = true;
            }
            '"' if current.is_empty() => quoted = true,
            '"' => return Err("unexpected quote in unquoted token"),
            ';' | '#' => break,
            '(' | ')' => {
                if !current.is_empty() {
                    tokens.push(TemplateToken {
                        raw: std::mem::take(&mut current),
                        quoted: false,
                    });
                }
            }
            ch if ch.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(TemplateToken {
                        raw: std::mem::take(&mut current),
                        quoted: false,
                    });
                }
            }
            _ => current.push(ch),
        }
    }

    if quoted {
        return Err("unterminated quoted string");
    }
    if escaping {
        return Err("unterminated escape sequence");
    }
    if !current.is_empty() {
        tokens.push(TemplateToken {
            raw: current,
            quoted: false,
        });
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddr};

    use super::*;
    use crate::plugin::executor::Executor;
    use crate::proto::{Message, Question, RecordType};

    fn make_context(name: &str, class: DNSClass) -> DnsContext {
        let mut request = Message::new();
        request.set_id(0x1234);
        request.set_checking_disabled(true);
        request.add_question(Question::new(
            Name::from_ascii(name).expect("test name should parse"),
            RecordType::HTTPS,
            class,
        ));
        DnsContext::new(SocketAddr::from((Ipv4Addr::LOCALHOST, 5300)), request)
    }

    fn executor(config: ResponseConfig) -> ResponseExecutor {
        ResponseExecutor {
            tag: "response".to_string(),
            rcode: parse_rcode(config.rcode.as_ref()).expect("rcode should parse"),
            answers: parse_section("answers", &config.answers).expect("answers should parse"),
            authorities: parse_section("authorities", &config.authorities)
                .expect("authorities should parse"),
            additionals: parse_section("additionals", &config.additionals)
                .expect("additionals should parse"),
            authoritative: config.authoritative,
            authentic_data: config.authentic_data,
            short_circuit: config.short_circuit,
        }
    }

    #[test]
    fn config_defaults_to_noerror_and_short_circuit() {
        let config = parse_config(None).expect("default config should parse");
        assert!(config.rcode.is_none());
        assert!(config.short_circuit);
        assert_eq!(parse_rcode(config.rcode.as_ref()).unwrap(), Rcode::NoError);
    }

    #[test]
    fn rcode_accepts_numeric_and_mnemonic_base_codes() {
        for raw in ["NXDOMAIN", "nxdomain", "3", "15"] {
            let value = Value::String(raw.to_string());
            assert!(parse_rcode(Some(&value)).is_ok(), "{raw} should parse");
        }
        for raw in ["BADVERS", "16", "not-a-code"] {
            let value = Value::String(raw.to_string());
            assert!(parse_rcode(Some(&value)).is_err(), "{raw} should fail");
        }
    }

    #[test]
    fn templates_reject_placeholders_outside_owner_or_class() {
        assert!(parse_record_template("answers", 1, "example.com. 60 IN CNAME {qname}").is_err());
        assert!(
            parse_record_template("answers", 1, "example.com. 60 IN TXT \"{qclass}\"").is_err()
        );
        assert!(
            parse_record_template("answers", 1, "example.com. 60 CH TXT \"{qclass}\"").is_err()
        );
        let qname_in_rdata = format!("{QNAME_SENTINEL} 60 IN TXT \"{QNAME_PLACEHOLDER}\"");
        assert!(parse_record_template("answers", 1, &qname_in_rdata).is_err());
    }

    #[tokio::test]
    async fn execute_builds_nodata_soa_from_question_template() {
        let plugin = executor(ResponseConfig {
            authorities: vec![
                "{qname} 300 {qclass} SOA ns.example. hostmaster.example. 1 7200 1800 86400 300"
                    .to_string(),
            ],
            authentic_data: true,
            ..ResponseConfig::default()
        });
        let mut context = make_context("example.com.", DNSClass::CH);

        assert_eq!(plugin.execute(&mut context).await.unwrap(), ExecStep::Stop);
        let response = context.response().expect("response should be set");
        assert_eq!(response.id(), 0x1234);
        assert_eq!(response.rcode(), Rcode::NoError);
        assert!(response.answers().is_empty());
        assert_eq!(response.authorities().len(), 1);
        assert_eq!(
            response.authorities()[0].name(),
            &Name::from_ascii("example.com.").unwrap()
        );
        assert_eq!(response.authorities()[0].class(), DNSClass::CH);
        assert_eq!(response.authorities()[0].ttl(), 300);
        assert!(response.authentic_data());
        assert!(response.checking_disabled());
    }

    #[tokio::test]
    async fn execute_replaces_existing_response_and_can_continue() {
        let plugin = executor(ResponseConfig {
            answers: vec!["{qname} 60 {qclass} A 192.0.2.10".to_string()],
            authoritative: true,
            short_circuit: false,
            ..ResponseConfig::default()
        });
        let mut context = make_context("example.com.", DNSClass::IN);
        context.set_response(context.request().response(Rcode::ServFail));

        assert_eq!(plugin.execute(&mut context).await.unwrap(), ExecStep::Next);
        let response = context.response().expect("response should be replaced");
        assert_eq!(response.rcode(), Rcode::NoError);
        assert!(response.authoritative());
        assert_eq!(response.answers().len(), 1);
        assert!(response.authorities().is_empty());
    }
}
