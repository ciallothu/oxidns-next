// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Literal-DNS resolver used by network clients that must avoid system DNS.
//!
//! This is a shared implementation detail. Outbound profiles expose it as a
//! `nameservers` resolver for OxiDNS-owned HTTP clients, while upstream keeps
//! its separate `bootstrap` configuration for DNS upstream server names.

use std::collections::HashMap;
use std::fmt::Debug;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use rand::random;
use tokio::net::UdpSocket;
use tokio::sync::{Notify, RwLock};
use tracing::{debug, error, info, warn};

use crate::infra::clock::AppClock;
use crate::infra::error::{DnsError, Result};
use crate::infra::network::deadline::{DeadlineOutcome, QueryDeadline};
use crate::infra::network::dial::{
    DialTarget, SocketOptions, TcpDialOptions, UdpDialOptions, connect_tcp, connect_udp,
};
use crate::infra::network::transport::tcp_transport::{TcpTransportReader, TcpTransportWriter};
use crate::infra::network::transport::udp_transport::UdpTransport;
use crate::proto::{DNSClass, Message, MessageType, Name, Opcode, Question, Record, RecordType};

const UDP_RECV_BUFFER_SIZE: usize = 8_196;
const DEFAULT_BOOTSTRAP_ENTRY_LIMIT: usize = 4_096;

const STATE_NONE: u8 = 0;
const STATE_QUERYING: u8 = 1;
const STATE_CACHED: u8 = 2;
const STATE_FAILED: u8 = 3;

#[derive(Clone, Debug)]
struct CacheData {
    ip: IpAddr,
    expires_at: u64,
}

/// Shared resolver backed by one or more literal-IP DNS servers.
#[derive(Debug)]
pub(crate) struct BootstrapResolver {
    clients: Vec<Arc<dyn BootstrapQueryClient>>,
    ip_version: Option<u8>,
    entries: Mutex<HashMap<String, Arc<BootstrapEntry>>>,
    entry_limit: usize,
}

impl BootstrapResolver {
    pub(crate) fn new(servers: Vec<String>, ip_version: Option<u8>) -> Result<Self> {
        if servers.is_empty() {
            return Err(DnsError::config(
                "bootstrap resolver requires at least one server",
            ));
        }
        let clients = servers
            .into_iter()
            .map(|server| BootstrapClient::new(&server).map(|client| Arc::new(client) as _))
            .collect::<Result<Vec<Arc<dyn BootstrapQueryClient>>>>()?;
        Ok(Self::from_clients(clients, ip_version))
    }

    fn from_clients(clients: Vec<Arc<dyn BootstrapQueryClient>>, ip_version: Option<u8>) -> Self {
        Self::from_clients_with_entry_limit(clients, ip_version, DEFAULT_BOOTSTRAP_ENTRY_LIMIT)
    }

    fn from_clients_with_entry_limit(
        clients: Vec<Arc<dyn BootstrapQueryClient>>,
        ip_version: Option<u8>,
        entry_limit: usize,
    ) -> Self {
        Self {
            clients,
            ip_version,
            entries: Mutex::new(HashMap::new()),
            entry_limit,
        }
    }

    #[inline]
    pub(crate) async fn resolve(&self, host: &str, deadline: QueryDeadline) -> Result<IpAddr> {
        let domain = bootstrap_domain(host);
        let entry = self.entry_for(domain)?;
        entry.get_with_deadline(deadline).await
    }

    fn entry_for(&self, domain: String) -> Result<Arc<BootstrapEntry>> {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(entry) = entries.get(&domain) {
            entry.touch(AppClock::elapsed_millis());
            return Ok(entry.clone());
        }
        if self.entry_limit > 0 && entries.len() >= self.entry_limit {
            prune_entries(&mut entries, self.entry_limit, AppClock::elapsed_millis());
        }
        let entry = Arc::new(BootstrapEntry::new(
            domain.clone(),
            self.ip_version,
            self.clients.clone(),
        )?);
        entry.touch(AppClock::elapsed_millis());
        entries.insert(domain, entry.clone());
        Ok(entry)
    }
}

fn prune_entries(
    entries: &mut HashMap<String, Arc<BootstrapEntry>>,
    entry_limit: usize,
    now_ms: u64,
) {
    entries.retain(|_, entry| !entry.can_prune_expired(now_ms));

    while entries.len() >= entry_limit {
        let Some(key) = entries
            .iter()
            .filter(|(_, entry)| !entry.is_querying())
            .min_by_key(|(_, entry)| entry.last_used_at())
            .map(|(domain, _)| domain.clone())
        else {
            break;
        };
        entries.remove(&key);
    }
}

#[async_trait]
trait BootstrapQueryClient: Debug + Send + Sync {
    async fn query(&self, request: Message, deadline: QueryDeadline) -> Result<Message>;

    fn label(&self) -> &str;
}

#[derive(Debug)]
struct BootstrapClient {
    label: String,
    socket_addr: SocketAddr,
}

impl BootstrapClient {
    fn new(server: &str) -> Result<Self> {
        let server = server.trim();
        let socket_addr = SocketAddr::from_str(server).map_err(|err| {
            if server.contains("://") {
                DnsError::plugin(format!("invalid bootstrap upstream '{}': {}", server, err))
            } else {
                DnsError::plugin(format!(
                    "bootstrap upstream '{}' must use a literal IP address",
                    server
                ))
            }
        })?;
        Ok(Self {
            label: server.to_string(),
            socket_addr,
        })
    }

    async fn query_udp(&self, request: Message, deadline: QueryDeadline) -> Result<Message> {
        let socket = connect_udp(UdpDialOptions::new(
            DialTarget::from_socket_addr(self.socket_addr),
            SocketOptions::default(),
        ))?;
        let socket = UdpSocket::from_std(socket)?;
        let transport = UdpTransport::new(socket);
        let query_id = request.id();

        match deadline
            .run(transport.write_message_with_id(&request, query_id))
            .await
        {
            DeadlineOutcome::Completed(result) => result?,
            DeadlineOutcome::Expired => return Err(deadline.timeout_error()),
        }

        let mut buf = [0u8; UDP_RECV_BUFFER_SIZE];
        let response = match deadline.run(transport.read_message(&mut buf)).await {
            DeadlineOutcome::Completed(result) => result?,
            DeadlineOutcome::Expired => return Err(deadline.timeout_error()),
        };
        validate_response_id(&response, query_id)?;
        Ok(response)
    }

    async fn query_tcp(&self, request: Message, deadline: QueryDeadline) -> Result<Message> {
        let stream = match deadline
            .run(connect_tcp(TcpDialOptions::new(
                DialTarget::from_socket_addr(self.socket_addr),
            )))
            .await
        {
            DeadlineOutcome::Completed(result) => result?,
            DeadlineOutcome::Expired => return Err(deadline.timeout_error()),
        };
        let (reader, writer) = stream.into_split();
        let mut reader = TcpTransportReader::new(reader);
        let mut writer = TcpTransportWriter::new(writer);
        let query_id = request.id();

        match deadline
            .run(writer.write_message_with_id(&request, query_id))
            .await
        {
            DeadlineOutcome::Completed(result) => result?,
            DeadlineOutcome::Expired => return Err(deadline.timeout_error()),
        }

        let response = match deadline.run(reader.read_message()).await {
            DeadlineOutcome::Completed(result) => result?,
            DeadlineOutcome::Expired => return Err(deadline.timeout_error()),
        };
        validate_response_id(&response, query_id)?;
        Ok(response)
    }
}

#[async_trait]
impl BootstrapQueryClient for BootstrapClient {
    async fn query(&self, request: Message, deadline: QueryDeadline) -> Result<Message> {
        let response = self.query_udp(request.clone(), deadline).await?;
        if response.truncated() {
            debug!(
                server = %self.label,
                "Bootstrap UDP response truncated, falling back to TCP"
            );
            return self.query_tcp(request, deadline).await;
        }
        Ok(response)
    }

    fn label(&self) -> &str {
        self.label.as_str()
    }
}

#[derive(Debug)]
struct BootstrapEntry {
    clients: Vec<Arc<dyn BootstrapQueryClient>>,
    state: AtomicU8,
    cache: RwLock<Option<CacheData>>,
    cache_expires_at: AtomicU64,
    last_used_at: AtomicU64,
    query_done: Notify,
    message: Message,
    query_name: Name,
    domain: String,
}

impl BootstrapEntry {
    fn new(
        domain: String,
        ip_version: Option<u8>,
        clients: Vec<Arc<dyn BootstrapQueryClient>>,
    ) -> Result<Self> {
        let parsed_name = Name::from_str(&domain).map_err(|err| {
            DnsError::plugin(format!(
                "invalid bootstrap target domain '{}': {}",
                domain, err
            ))
        })?;

        let mut message = Message::new();
        message.set_message_type(MessageType::Query);
        message.set_opcode(Opcode::Query);
        message.set_recursion_desired(true);
        message.add_question(Question::new(
            parsed_name.clone(),
            match ip_version {
                Some(6) => RecordType::AAAA,
                _ => RecordType::A,
            },
            DNSClass::IN,
        ));

        Ok(Self {
            clients,
            state: AtomicU8::new(STATE_NONE),
            cache: RwLock::new(None),
            cache_expires_at: AtomicU64::new(0),
            last_used_at: AtomicU64::new(0),
            query_done: Notify::new(),
            message,
            query_name: parsed_name,
            domain,
        })
    }

    #[inline]
    async fn get_with_deadline(&self, deadline: QueryDeadline) -> Result<IpAddr> {
        self.touch(AppClock::elapsed_millis());
        let mut failed_count = 0;

        loop {
            if deadline.remaining().is_none() {
                return Err(deadline.timeout_error());
            }

            match self.state.load(Ordering::Acquire) {
                STATE_CACHED => {
                    let cache = self.cache.read().await;
                    if let Some(ref data) = *cache
                        && AppClock::elapsed_millis() < data.expires_at
                    {
                        return Ok(data.ip);
                    }
                    drop(cache);

                    debug!(
                        domain = %self.domain,
                        "Bootstrap cache expired, triggering refresh"
                    );
                    if self
                        .state
                        .compare_exchange(
                            STATE_CACHED,
                            STATE_NONE,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_ok()
                    {
                        continue;
                    }
                    self.wait_query_done(deadline).await?;
                }
                STATE_NONE => {
                    if self
                        .state
                        .compare_exchange(
                            STATE_NONE,
                            STATE_QUERYING,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_ok()
                    {
                        let mut query_guard = BootstrapQueryGuard::new(self);
                        self.query(deadline).await;
                        query_guard.disarm();
                        continue;
                    }
                    self.wait_query_done(deadline).await?;
                }
                STATE_QUERYING => self.wait_query_done(deadline).await?,
                STATE_FAILED => {
                    if failed_count > 3 {
                        return Err(DnsError::protocol(format!(
                            "Bootstrap DNS resolution failed for '{}' after {} attempts",
                            self.domain, failed_count
                        )));
                    }
                    failed_count += 1;
                    if self
                        .state
                        .compare_exchange(
                            STATE_FAILED,
                            STATE_NONE,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_ok()
                    {
                        continue;
                    }
                    self.wait_query_done(deadline).await?;
                }
                _ => unreachable!("invalid bootstrap state"),
            }
        }
    }

    async fn wait_query_done(&self, deadline: QueryDeadline) -> Result<()> {
        let notified = self.query_done.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if self.state.load(Ordering::Acquire) != STATE_QUERYING {
            return Ok(());
        }
        match deadline.run(notified.as_mut()).await {
            DeadlineOutcome::Completed(()) => Ok(()),
            DeadlineOutcome::Expired => Err(deadline.timeout_error()),
        }
    }

    async fn query(&self, deadline: QueryDeadline) {
        let mut message = self.message.clone();
        message.set_id(random());
        let mut last_error = None;

        for client in &self.clients {
            match client.query(message.clone(), deadline).await {
                Ok(response) => {
                    if let Some((ip, ttl_seconds, record_type)) =
                        select_bootstrap_answer(response.answers(), &self.query_name)
                    {
                        info!(
                            domain = %self.domain,
                            server = %client.label(),
                            ip = %ip,
                            ttl_seconds,
                            record_type = ?record_type,
                            "Bootstrap DNS resolution successful"
                        );

                        let ttl = ttl_seconds as u64 * 1000;
                        let expires_at = AppClock::elapsed_millis().saturating_add(ttl);
                        *self.cache.write().await = Some(CacheData { ip, expires_at });
                        self.cache_expires_at.store(expires_at, Ordering::Release);
                        self.state.store(STATE_CACHED, Ordering::Release);
                        self.query_done.notify_waiters();
                        return;
                    }

                    warn!(
                        domain = %self.domain,
                        server = %client.label(),
                        answer_count = response.answers().len(),
                        "No A/AAAA records found in bootstrap DNS response"
                    );
                    last_error = Some(DnsError::protocol(format!(
                        "No A/AAAA records found in bootstrap DNS response for '{}'",
                        self.domain
                    )));
                }
                Err(err) => {
                    error!(
                        domain = %self.domain,
                        server = %client.label(),
                        error = %err,
                        "Bootstrap DNS query failed"
                    );
                    last_error = Some(err);
                }
            }
        }

        if let Some(err) = last_error {
            debug!(domain = %self.domain, error = %err, "Bootstrap query exhausted servers");
        }
        self.state.store(STATE_FAILED, Ordering::Release);
        self.query_done.notify_waiters();
    }

    fn touch(&self, now_ms: u64) {
        self.last_used_at.store(now_ms, Ordering::Relaxed);
    }

    fn last_used_at(&self) -> u64 {
        self.last_used_at.load(Ordering::Relaxed)
    }

    fn is_querying(&self) -> bool {
        self.state.load(Ordering::Acquire) == STATE_QUERYING
    }

    fn can_prune_expired(&self, now_ms: u64) -> bool {
        match self.state.load(Ordering::Acquire) {
            STATE_QUERYING => false,
            STATE_CACHED => {
                let expires_at = self.cache_expires_at.load(Ordering::Acquire);
                expires_at == 0 || now_ms >= expires_at
            }
            _ => true,
        }
    }
}

fn validate_response_id(response: &Message, query_id: u16) -> Result<()> {
    if response.id() == query_id {
        Ok(())
    } else {
        Err(DnsError::protocol(format!(
            "bootstrap DNS response ID mismatch: expected {}, got {}",
            query_id,
            response.id()
        )))
    }
}

fn select_bootstrap_answer(
    answers: &[Record],
    query_name: &Name,
) -> Option<(IpAddr, u32, RecordType)> {
    let mut accepted_names = HashMap::new();
    accepted_names.insert(query_name.clone(), u32::MAX);

    loop {
        let mut changed = false;
        for answer in answers {
            let Some(target) = answer.cname_target() else {
                continue;
            };
            let Some(owner_ttl) = accepted_names.get(answer.name()).copied() else {
                continue;
            };
            let ttl = owner_ttl.min(answer.ttl());
            match accepted_names.get(target).copied() {
                Some(existing_ttl) if existing_ttl <= ttl => {}
                _ => {
                    accepted_names.insert(target.clone(), ttl);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }

    for answer in answers {
        if !matches!(answer.rr_type(), RecordType::A | RecordType::AAAA) {
            continue;
        }
        let Some(owner_ttl) = accepted_names.get(answer.name()).copied() else {
            continue;
        };
        let Some(ip) = answer.ip_addr() else {
            continue;
        };
        return Some((ip, owner_ttl.min(answer.ttl()), answer.rr_type()));
    }

    None
}

fn bootstrap_domain(host: &str) -> String {
    if host.ends_with('.') {
        host.to_string()
    } else {
        format!("{host}.")
    }
}

struct BootstrapQueryGuard<'a> {
    entry: &'a BootstrapEntry,
    armed: bool,
}

impl<'a> BootstrapQueryGuard<'a> {
    fn new(entry: &'a BootstrapEntry) -> Self {
        Self { entry, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for BootstrapQueryGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.entry.state.store(STATE_FAILED, Ordering::Release);
            self.entry.query_done.notify_waiters();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::future::pending;
    use std::net::{Ipv4Addr, Ipv6Addr};
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use std::time::Duration;

    use tokio::sync::oneshot;

    use super::*;
    use crate::proto::RData;
    use crate::proto::rdata::{A, AAAA, CNAME};

    #[derive(Debug)]
    enum FakeOutcome {
        Response(Message),
        Error(&'static str),
    }

    #[derive(Debug)]
    struct FakeClient {
        label: String,
        outcomes: Mutex<VecDeque<FakeOutcome>>,
        count: AtomicUsize,
    }

    impl FakeClient {
        fn new(label: &str, outcomes: Vec<FakeOutcome>) -> Self {
            Self {
                label: label.to_string(),
                outcomes: Mutex::new(VecDeque::from(outcomes)),
                count: AtomicUsize::new(0),
            }
        }

        fn count(&self) -> usize {
            self.count.load(AtomicOrdering::Relaxed)
        }
    }

    #[async_trait]
    impl BootstrapQueryClient for FakeClient {
        async fn query(&self, _request: Message, _deadline: QueryDeadline) -> Result<Message> {
            self.count.fetch_add(1, AtomicOrdering::Relaxed);
            match self
                .outcomes
                .lock()
                .expect("outcomes lock should not be poisoned")
                .pop_front()
            {
                Some(FakeOutcome::Response(response)) => Ok(response),
                Some(FakeOutcome::Error(message)) => Err(DnsError::protocol(message)),
                None => Err(DnsError::protocol("no fake response configured")),
            }
        }

        fn label(&self) -> &str {
            self.label.as_str()
        }
    }

    #[derive(Debug)]
    struct SlowClient {
        label: String,
        response: Message,
        count: AtomicUsize,
    }

    #[async_trait]
    impl BootstrapQueryClient for SlowClient {
        async fn query(&self, _request: Message, _deadline: QueryDeadline) -> Result<Message> {
            self.count.fetch_add(1, AtomicOrdering::Relaxed);
            tokio::time::sleep(Duration::from_millis(30)).await;
            Ok(self.response.clone())
        }

        fn label(&self) -> &str {
            self.label.as_str()
        }
    }

    #[derive(Debug)]
    struct BlockingClient {
        started: Mutex<Option<oneshot::Sender<()>>>,
    }

    #[async_trait]
    impl BootstrapQueryClient for BlockingClient {
        async fn query(&self, _request: Message, _deadline: QueryDeadline) -> Result<Message> {
            if let Some(started) = self
                .started
                .lock()
                .expect("started lock should not be poisoned")
                .take()
            {
                let _ = started.send(());
            }
            pending::<Result<Message>>().await
        }

        fn label(&self) -> &str {
            "blocking"
        }
    }

    fn start_clock() {
        AppClock::start();
    }

    fn answer_response(name: &str, ttl: u32, ip: IpAddr) -> Message {
        let name = Name::from_ascii(name).expect("answer name should parse");
        let mut message = Message::new();
        let record = match ip {
            IpAddr::V4(ip) => Record::from_rdata(name, ttl, RData::A(A(ip))),
            IpAddr::V6(ip) => Record::from_rdata(name, ttl, RData::AAAA(AAAA(ip))),
        };
        message.add_answer(record);
        message
    }

    #[test]
    fn test_client_rejects_invalid_bootstrap_server() {
        let result = BootstrapClient::new("udp://127.0.0.1:notaport");

        assert!(result.is_err());
    }

    #[test]
    fn test_client_rejects_hostname_bootstrap_server() {
        let result = BootstrapClient::new("resolver.example.invalid:53");

        assert!(
            result
                .expect_err("hostname bootstrap server should be rejected")
                .to_string()
                .contains("must use a literal IP address")
        );
    }

    #[test]
    fn test_entry_builds_ipv4_query_by_default() {
        let client = Arc::new(FakeClient::new("fake", Vec::new()));
        let entry = BootstrapEntry::new("example.com.".to_string(), None, vec![client])
            .expect("entry should be created");

        let query = entry
            .message
            .first_question()
            .expect("question should be pre-built");

        assert_eq!(entry.domain, "example.com.");
        assert_eq!(query.qtype(), RecordType::A);
        assert_eq!(query.name().to_fqdn(), "example.com.");
    }

    #[test]
    fn test_entry_builds_ipv6_query_when_requested() {
        let client = Arc::new(FakeClient::new("fake", Vec::new()));
        let entry = BootstrapEntry::new("example.com.".to_string(), Some(6), vec![client])
            .expect("entry should be created");

        let query = entry
            .message
            .first_question()
            .expect("question should be pre-built");

        assert_eq!(query.qtype(), RecordType::AAAA);
    }

    #[test]
    fn test_entry_rejects_invalid_target_domain() {
        let client = Arc::new(FakeClient::new("fake", Vec::new()));
        let result = BootstrapEntry::new("example..com.".to_string(), None, vec![client]);

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_canceled_query_releases_querying_state() {
        start_clock();
        let (started_tx, started_rx) = oneshot::channel();
        let client = Arc::new(BlockingClient {
            started: Mutex::new(Some(started_tx)),
        });
        let entry = Arc::new(
            BootstrapEntry::new("example.com.".to_string(), None, vec![client])
                .expect("entry should be created"),
        );

        let task_entry = entry.clone();
        let handle = tokio::spawn(async move {
            task_entry
                .get_with_deadline(QueryDeadline::new(Duration::from_secs(5)))
                .await
        });

        started_rx.await.expect("bootstrap query should start");
        handle.abort();
        assert!(
            handle
                .await
                .expect_err("bootstrap task should be cancelled")
                .is_cancelled()
        );

        assert_eq!(entry.state.load(Ordering::Acquire), STATE_FAILED);
    }

    #[tokio::test]
    async fn test_wait_query_done_returns_when_state_already_changed() {
        start_clock();
        let client = Arc::new(FakeClient::new("fake", Vec::new()));
        let entry = BootstrapEntry::new("example.com.".to_string(), None, vec![client])
            .expect("entry should be created");
        entry.state.store(STATE_CACHED, Ordering::Release);

        entry
            .wait_query_done(QueryDeadline::new(Duration::from_millis(10)))
            .await
            .expect("changed state should not wait for a fresh notification");
    }

    #[tokio::test]
    async fn test_resolver_falls_back_to_next_bootstrap_server() {
        start_clock();
        let first = Arc::new(FakeClient::new("first", vec![FakeOutcome::Error("boom")]));
        let second = Arc::new(FakeClient::new(
            "second",
            vec![FakeOutcome::Response(answer_response(
                "example.com.",
                60,
                IpAddr::V4(Ipv4Addr::new(203, 0, 113, 53)),
            ))],
        ));
        let resolver = BootstrapResolver::from_clients(vec![first.clone(), second.clone()], None);

        let ip = resolver
            .resolve(
                "example.com",
                QueryDeadline::new(Duration::from_millis(200)),
            )
            .await
            .expect("second bootstrap server should resolve");

        assert_eq!(ip, IpAddr::V4(Ipv4Addr::new(203, 0, 113, 53)));
        assert_eq!(first.count(), 1);
        assert_eq!(second.count(), 1);
    }

    #[tokio::test]
    async fn test_concurrent_resolve_singleflights_same_domain() {
        start_clock();
        let client = Arc::new(SlowClient {
            label: "slow".to_string(),
            response: answer_response(
                "example.com.",
                60,
                IpAddr::V4(Ipv4Addr::new(203, 0, 113, 53)),
            ),
            count: AtomicUsize::new(0),
        });
        let resolver = Arc::new(BootstrapResolver::from_clients(vec![client.clone()], None));

        let mut handles = Vec::new();
        for _ in 0..5 {
            let resolver = resolver.clone();
            handles.push(tokio::spawn(async move {
                resolver
                    .resolve(
                        "example.com",
                        QueryDeadline::new(Duration::from_millis(500)),
                    )
                    .await
            }));
        }

        for handle in handles {
            let ip = handle
                .await
                .expect("task should complete")
                .expect("resolve should succeed");
            assert_eq!(ip, IpAddr::V4(Ipv4Addr::new(203, 0, 113, 53)));
        }
        assert_eq!(client.count.load(AtomicOrdering::Relaxed), 1);
    }

    #[tokio::test]
    async fn test_expired_cache_refreshes() {
        start_clock();
        let client = Arc::new(FakeClient::new(
            "fake",
            vec![
                FakeOutcome::Response(answer_response(
                    "example.com.",
                    60,
                    IpAddr::V4(Ipv4Addr::new(203, 0, 113, 1)),
                )),
                FakeOutcome::Response(answer_response(
                    "example.com.",
                    60,
                    IpAddr::V4(Ipv4Addr::new(203, 0, 113, 2)),
                )),
            ],
        ));
        let resolver = BootstrapResolver::from_clients(vec![client.clone()], None);

        let first = resolver
            .resolve(
                "example.com",
                QueryDeadline::new(Duration::from_millis(200)),
            )
            .await
            .expect("first resolve should succeed");
        let entry = resolver
            .entry_for("example.com.".to_string())
            .expect("entry should exist");
        *entry.cache.write().await = Some(CacheData {
            ip: first,
            expires_at: 0,
        });
        let second = resolver
            .resolve(
                "example.com",
                QueryDeadline::new(Duration::from_millis(200)),
            )
            .await
            .expect("second resolve should refresh");

        assert_eq!(first, IpAddr::V4(Ipv4Addr::new(203, 0, 113, 1)));
        assert_eq!(second, IpAddr::V4(Ipv4Addr::new(203, 0, 113, 2)));
        assert_eq!(client.count(), 2);
    }

    #[tokio::test]
    async fn test_resolver_prunes_expired_entries_before_insert() {
        start_clock();
        let client = Arc::new(FakeClient::new(
            "fake",
            vec![
                FakeOutcome::Response(answer_response(
                    "one.example.",
                    60,
                    IpAddr::V4(Ipv4Addr::new(203, 0, 113, 1)),
                )),
                FakeOutcome::Response(answer_response(
                    "two.example.",
                    60,
                    IpAddr::V4(Ipv4Addr::new(203, 0, 113, 2)),
                )),
                FakeOutcome::Response(answer_response(
                    "three.example.",
                    60,
                    IpAddr::V4(Ipv4Addr::new(203, 0, 113, 3)),
                )),
            ],
        ));
        let resolver =
            BootstrapResolver::from_clients_with_entry_limit(vec![client.clone()], None, 2);

        resolver
            .resolve(
                "one.example",
                QueryDeadline::new(Duration::from_millis(200)),
            )
            .await
            .expect("first domain should resolve");
        resolver
            .resolve(
                "two.example",
                QueryDeadline::new(Duration::from_millis(200)),
            )
            .await
            .expect("second domain should resolve");

        let expired = resolver
            .entries
            .lock()
            .expect("entries lock should not be poisoned")
            .get("one.example.")
            .expect("first entry should exist")
            .clone();
        expired.cache_expires_at.store(0, Ordering::Release);

        resolver
            .resolve(
                "three.example",
                QueryDeadline::new(Duration::from_millis(200)),
            )
            .await
            .expect("third domain should resolve");

        let entries = resolver
            .entries
            .lock()
            .expect("entries lock should not be poisoned");
        assert_eq!(entries.len(), 2);
        assert!(!entries.contains_key("one.example."));
        assert!(entries.contains_key("two.example."));
        assert!(entries.contains_key("three.example."));
    }

    #[tokio::test]
    async fn test_resolver_evicts_lru_non_querying_entry_at_limit() {
        start_clock();
        let client = Arc::new(FakeClient::new(
            "fake",
            vec![
                FakeOutcome::Response(answer_response(
                    "one.example.",
                    60,
                    IpAddr::V4(Ipv4Addr::new(203, 0, 113, 1)),
                )),
                FakeOutcome::Response(answer_response(
                    "two.example.",
                    60,
                    IpAddr::V4(Ipv4Addr::new(203, 0, 113, 2)),
                )),
                FakeOutcome::Response(answer_response(
                    "three.example.",
                    60,
                    IpAddr::V4(Ipv4Addr::new(203, 0, 113, 3)),
                )),
            ],
        ));
        let resolver =
            BootstrapResolver::from_clients_with_entry_limit(vec![client.clone()], None, 2);

        resolver
            .resolve(
                "one.example",
                QueryDeadline::new(Duration::from_millis(200)),
            )
            .await
            .expect("first domain should resolve");
        resolver
            .resolve(
                "two.example",
                QueryDeadline::new(Duration::from_millis(200)),
            )
            .await
            .expect("second domain should resolve");

        {
            let entries = resolver
                .entries
                .lock()
                .expect("entries lock should not be poisoned");
            entries
                .get("one.example.")
                .expect("first entry should exist")
                .last_used_at
                .store(20, Ordering::Relaxed);
            entries
                .get("two.example.")
                .expect("second entry should exist")
                .last_used_at
                .store(10, Ordering::Relaxed);
        }

        resolver
            .resolve(
                "three.example",
                QueryDeadline::new(Duration::from_millis(200)),
            )
            .await
            .expect("third domain should resolve");

        let entries = resolver
            .entries
            .lock()
            .expect("entries lock should not be poisoned");
        assert_eq!(entries.len(), 2);
        assert!(entries.contains_key("one.example."));
        assert!(!entries.contains_key("two.example."));
        assert!(entries.contains_key("three.example."));
    }

    #[test]
    fn test_resolver_keeps_querying_entries_when_at_limit() {
        start_clock();
        let client = Arc::new(FakeClient::new("fake", Vec::new()));
        let resolver = BootstrapResolver::from_clients_with_entry_limit(vec![client], None, 1);
        let querying = resolver
            .entry_for("one.example.".to_string())
            .expect("first entry should be created");
        querying.state.store(STATE_QUERYING, Ordering::Release);

        resolver
            .entry_for("two.example.".to_string())
            .expect("second entry should be created");

        let entries = resolver
            .entries
            .lock()
            .expect("entries lock should not be poisoned");
        assert_eq!(entries.len(), 2);
        assert!(entries.contains_key("one.example."));
        assert!(entries.contains_key("two.example."));
    }

    #[test]
    fn test_select_bootstrap_answer_follows_cname_and_rejects_unrelated_a() {
        let query_name = Name::from_ascii("example.com.").expect("name should parse");
        let alias_name = Name::from_ascii("alias.example.net.").expect("name should parse");
        let unrelated_name = Name::from_ascii("unrelated.example.").expect("name should parse");
        let unrelated = Record::from_rdata(
            unrelated_name,
            300,
            RData::A(A(Ipv4Addr::new(192, 0, 2, 10))),
        );
        let cname = Record::from_rdata(
            query_name.clone(),
            30,
            RData::CNAME(CNAME(alias_name.clone())),
        );
        let target =
            Record::from_rdata(alias_name, 300, RData::A(A(Ipv4Addr::new(203, 0, 113, 53))));

        let selected = select_bootstrap_answer(&[unrelated, cname, target], &query_name)
            .expect("CNAME target should be accepted");

        assert_eq!(selected.0, IpAddr::V4(Ipv4Addr::new(203, 0, 113, 53)));
        assert_eq!(selected.1, 30);
        assert_eq!(selected.2, RecordType::A);
    }

    #[test]
    fn test_select_bootstrap_answer_rejects_unrelated_a_records() {
        let query_name = Name::from_ascii("example.com.").expect("name should parse");
        let unrelated_name = Name::from_ascii("unrelated.example.").expect("name should parse");
        let unrelated = Record::from_rdata(
            unrelated_name,
            300,
            RData::A(A(Ipv4Addr::new(192, 0, 2, 10))),
        );

        assert!(select_bootstrap_answer(&[unrelated], &query_name).is_none());
    }

    #[test]
    fn test_answer_response_supports_ipv6() {
        let response = answer_response("example.com.", 60, IpAddr::V6(Ipv6Addr::LOCALHOST));

        assert_eq!(response.answers().len(), 1);
    }
}
