// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared MikroTik RouterOS management-plane infrastructure.

use std::net::IpAddr;
use std::time::Duration;

use ahash::AHashMap;

use crate::proto::Message;

pub(crate) mod batching;
pub(crate) mod completion;
pub(crate) mod ip_prefix;
pub(crate) mod lease;
pub(crate) mod lifecycle;
pub(crate) mod mailbox;
pub(crate) mod reconcile;
pub(crate) mod throttle;
pub(crate) mod transport;

/// Total budget for cancelling background work and cleaning owned RouterOS
/// objects during plugin shutdown.
pub(crate) const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);

/// One address observed in a DNS answer section.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) struct ObservedAddr {
    pub(crate) addr: IpAddr,
    pub(crate) ttl_secs: u32,
}

/// Validate response identity and collect all enabled A/AAAA answer records.
///
/// The RouterOS observers intentionally do not reconstruct CNAME chains. Each
/// address is independently useful, and duplicates retain the largest TTL.
pub(crate) fn collect_observed_addrs(
    request: &Message,
    response: &Message,
    mut family_enabled: impl FnMut(IpAddr) -> bool,
) -> Vec<ObservedAddr> {
    let Some(request_question) = request.first_question() else {
        return Vec::new();
    };
    if response
        .first_question()
        .is_some_and(|response_question| response_question != request_question)
    {
        return Vec::new();
    }

    let mut dedup = AHashMap::<IpAddr, u32>::new();
    for answer in response.answers() {
        let Some(addr) = answer.ip_addr() else {
            continue;
        };
        if !family_enabled(addr) {
            continue;
        }
        dedup
            .entry(addr)
            .and_modify(|ttl| *ttl = (*ttl).max(answer.ttl()))
            .or_insert(answer.ttl());
    }

    dedup
        .into_iter()
        .map(|(addr, ttl_secs)| ObservedAddr { addr, ttl_secs })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    use super::*;
    use crate::proto::rdata::{A, AAAA};
    use crate::proto::{DNSClass, Name, Question, RData, Record, RecordType};

    #[test]
    fn collector_validates_question_and_keeps_largest_duplicate_ttl() {
        let mut request = Message::new();
        request.add_question(Question::new(
            Name::from_ascii("example.com.").expect("name"),
            RecordType::A,
            DNSClass::IN,
        ));
        let mut response = Message::new();
        response.add_answer(Record::from_rdata(
            Name::from_ascii("alias.example.").expect("name"),
            30,
            RData::A(A(Ipv4Addr::new(192, 0, 2, 1))),
        ));
        response.add_answer(Record::from_rdata(
            Name::from_ascii("other.example.").expect("name"),
            120,
            RData::A(A(Ipv4Addr::new(192, 0, 2, 1))),
        ));
        response.add_answer(Record::from_rdata(
            Name::from_ascii("other.example.").expect("name"),
            60,
            RData::AAAA(AAAA(Ipv6Addr::LOCALHOST)),
        ));

        let observed = collect_observed_addrs(&request, &response, |_| true);
        assert_eq!(observed.len(), 2);
        assert!(observed.contains(&ObservedAddr {
            addr: IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
            ttl_secs: 120,
        }));
        assert!(observed.contains(&ObservedAddr {
            addr: IpAddr::V6(Ipv6Addr::LOCALHOST),
            ttl_secs: 60,
        }));

        response.add_question(Question::new(
            Name::from_ascii("other.example.").expect("name"),
            RecordType::A,
            DNSClass::IN,
        ));
        assert!(collect_observed_addrs(&request, &response, |_| true).is_empty());
    }
}
