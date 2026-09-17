// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! V2Ray protobuf models, parsing, and selector support.

mod model;
mod parser;
mod selector;

pub(crate) use model::{Cidr, Domain, DomainType, GeoIp, GeoIpList, GeoSite, GeoSiteList};
pub(crate) use parser::{
    DatFileSession, ParsedDat, cidr_to_rule, detect_dat_kind, geoip_code, geosite_code,
    geosite_domain_expression, geosite_domain_expression_original_with_attrs, parse_geoip_dat,
    parse_geosite_dat,
};
pub(crate) use selector::{
    GeoSiteSelector, geosite_domain_matches_selectors, matched_geosite_selectors,
    normalized_selectors, parse_geosite_selectors, unique_nonempty_selectors,
};
