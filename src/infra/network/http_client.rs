// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared HTTP client helpers.
//!
//! The client is intentionally dependency-light and shared by HTTP side-effect
//! executors, rule downloads, and release upgrades. It centralizes TLS policy,
//! SOCKS5 dialing, redirects, response draining, and atomic file downloads.

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::task::{Context, Poll};
use std::{fmt, io};

use bytes::Bytes;
use futures::future::BoxFuture;
use http::header::{CONTENT_LENGTH, HeaderName, HeaderValue, LOCATION};
use http::{HeaderMap, Method, Request, StatusCode, Uri};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper_rustls::HttpsConnectorBuilder;
use hyper_util::client::legacy::Client as HyperClient;
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::fs;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tower_service::Service;
use url::Url;

use crate::infra::error::{DnsError, Result};
use crate::infra::network::dial::{DialTarget, SocketOptions};
use crate::infra::network::outbound::{self, OutboundPolicy};
use crate::infra::network::proxy::{Socks5Opt, connect_tcp, parse_optional_socks5};
use crate::infra::network::tls_config::{insecure_client_config, secure_client_config};

pub const DEFAULT_MAX_REDIRECTS: usize = 5;

type InnerClient = HyperClient<hyper_rustls::HttpsConnector<HttpConnector>, Full<Bytes>>;

#[derive(Clone, Debug, Default)]
pub struct HttpClientOptions {
    pub insecure_skip_verify: bool,
    outbound: OutboundPolicy,
}

impl HttpClientOptions {
    pub fn new(insecure_skip_verify: bool, socks5: Option<Socks5Opt>) -> Self {
        Self {
            insecure_skip_verify,
            outbound: OutboundPolicy::system(socks5),
        }
    }

    pub fn from_outbound<F>(
        insecure_skip_verify: bool,
        outbound_ref: Option<&str>,
        legacy_socks5: Option<&str>,
        invalid_socks5: F,
    ) -> Result<Self>
    where
        F: FnOnce(&str) -> DnsError,
    {
        let legacy_socks5 = parse_optional_socks5(legacy_socks5, invalid_socks5)?;
        let outbound = outbound::global().resolve_policy(outbound_ref, legacy_socks5)?;
        Ok(Self {
            insecure_skip_verify,
            outbound,
        })
    }
}

#[derive(Clone, Debug)]
pub struct HttpRequestOptions {
    pub url: String,
    pub headers: Vec<(HeaderName, HeaderValue)>,
    pub body: Bytes,
    pub max_redirects: usize,
}

impl HttpRequestOptions {
    pub fn from_url(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            headers: Vec::new(),
            body: Bytes::new(),
            max_redirects: DEFAULT_MAX_REDIRECTS,
        }
    }

    pub fn with_headers(mut self, headers: Vec<(HeaderName, HeaderValue)>) -> Self {
        self.headers = headers;
        self
    }

    pub fn with_body(mut self, body: Bytes) -> Self {
        self.body = body;
        self
    }

    pub fn with_max_redirects(mut self, max_redirects: usize) -> Self {
        self.max_redirects = max_redirects;
        self
    }
}

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
}

#[derive(Clone)]
pub struct HttpClient {
    client: InnerClient,
}

impl fmt::Debug for HttpClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("HttpClient").finish_non_exhaustive()
    }
}

impl HttpClient {
    pub fn new(options: HttpClientOptions) -> Self {
        let tls_config = if options.insecure_skip_verify {
            insecure_client_config()
        } else {
            secure_client_config()
        };

        let connector = HttpsConnectorBuilder::new()
            .with_tls_config(tls_config)
            .https_or_http()
            .enable_http1()
            .enable_http2()
            .wrap_connector(HttpConnector {
                outbound: options.outbound,
            });

        Self {
            client: HyperClient::builder(TokioExecutor::new()).build(connector),
        }
    }

    pub async fn get_request(&self, options: HttpRequestOptions) -> Result<HttpResponse> {
        let options = HttpRequestOptions {
            body: Bytes::new(),
            ..options
        };
        self.request(Method::GET, options).await
    }

    pub async fn post_request(&self, options: HttpRequestOptions) -> Result<HttpResponse> {
        self.request(Method::POST, options).await
    }

    pub async fn request(
        &self,
        method: Method,
        options: HttpRequestOptions,
    ) -> Result<HttpResponse> {
        let response = self.request_following_redirects(method, options).await?;
        let status = response.status();
        let headers = response.headers().clone();
        let body = response
            .into_body()
            .collect()
            .await
            .map_err(|err| DnsError::plugin(format!("failed to read response body: {err}")))?
            .to_bytes();
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }

    pub async fn download(&self, options: HttpRequestOptions, path: &Path) -> Result<()> {
        self.download_with_progress(options, path, |_| {}).await
    }

    pub async fn download_with_progress<F>(
        &self,
        options: HttpRequestOptions,
        path: &Path,
        progress: F,
    ) -> Result<()>
    where
        F: FnMut(DownloadProgress),
    {
        let options = HttpRequestOptions {
            body: Bytes::new(),
            ..options
        };
        let response = self
            .request_following_redirects(Method::GET, options)
            .await?;
        let total = content_length(response.headers());
        write_target_file(path, response.into_body(), total, progress).await
    }

    async fn request_following_redirects(
        &self,
        method: Method,
        mut options: HttpRequestOptions,
    ) -> Result<hyper::Response<Incoming>> {
        let label = request_label(&method, options.url.as_str());

        for redirect_count in 0..=options.max_redirects {
            let request = build_hyper_request(&method, &options)?;
            let response =
                self.client.request(request).await.map_err(|err| {
                    DnsError::plugin(format!("request failed for '{label}': {err}"))
                })?;

            let status = response.status();
            if status.is_success() {
                return Ok(response);
            }

            if status.is_redirection() {
                if redirect_count == options.max_redirects {
                    drain_response_body(response.into_body()).await?;
                    return Err(DnsError::plugin(format!(
                        "request failed for '{}': too many redirects",
                        request_label(&method, options.url.as_str())
                    )));
                }

                let location = response
                    .headers()
                    .get(LOCATION)
                    .ok_or_else(|| {
                        DnsError::plugin(format!(
                            "request failed for '{}': redirect {} missing Location header",
                            request_label(&method, options.url.as_str()),
                            format_status(status)
                        ))
                    })?
                    .to_str()
                    .map_err(|err| {
                        DnsError::plugin(format!(
                            "request failed for '{}': invalid redirect Location header: {}",
                            request_label(&method, options.url.as_str()),
                            err
                        ))
                    })?
                    .to_string();
                drain_response_body(response.into_body()).await?;
                options.url = resolve_redirect_url(options.url.as_str(), location.as_str())?;
                continue;
            }

            drain_response_body(response.into_body()).await?;
            return Err(DnsError::plugin(format!(
                "request failed for '{}': unexpected status {}",
                request_label(&method, options.url.as_str()),
                format_status(status)
            )));
        }

        Err(DnsError::plugin(format!(
            "request failed for '{}': too many redirects",
            request_label(&method, options.url.as_str())
        )))
    }
}

fn build_hyper_request(
    method: &Method,
    options: &HttpRequestOptions,
) -> Result<Request<Full<Bytes>>> {
    let uri = options.url.parse::<Uri>().map_err(|err| {
        DnsError::plugin(format!(
            "failed to build http request for '{}': invalid uri: {}",
            request_label(method, options.url.as_str()),
            err
        ))
    })?;
    let mut builder = Request::builder().method(method.clone()).uri(uri);
    let headers = builder.headers_mut().ok_or_else(|| {
        DnsError::plugin(format!(
            "failed to build http request for '{}': headers unavailable",
            request_label(method, options.url.as_str())
        ))
    })?;
    for (name, value) in &options.headers {
        headers.append(name.clone(), value.clone());
    }
    builder
        .body(Full::new(options.body.clone()))
        .map_err(|err| {
            DnsError::plugin(format!(
                "failed to build http request for '{}': {}",
                request_label(method, options.url.as_str()),
                err
            ))
        })
}

#[derive(Debug, Clone)]
struct HttpConnector {
    outbound: OutboundPolicy,
}

impl Service<Uri> for HttpConnector {
    type Error = DnsError;
    type Future = BoxFuture<'static, std::result::Result<Self::Response, Self::Error>>;
    type Response = TokioIo<TcpStream>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<std::result::Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, dst: Uri) -> Self::Future {
        let outbound = self.outbound.clone();
        Box::pin(async move {
            let host = dst.host().ok_or_else(|| {
                DnsError::plugin(format!("http request uri '{}' is missing host", dst))
            })?;
            let port = dst
                .port_u16()
                .or_else(|| match dst.scheme_str() {
                    Some("http") => Some(80),
                    Some("https") => Some(443),
                    _ => None,
                })
                .ok_or_else(|| {
                    DnsError::plugin(format!(
                        "http request uri '{}' uses unsupported or missing scheme",
                        dst
                    ))
                })?;
            let mut remote_ip = parse_uri_host_ip_literal(host);
            let socks5 = outbound.proxy();
            if remote_ip.is_none() && (outbound.has_custom_resolver() || socks5.is_none()) {
                remote_ip = Some(outbound.resolve_host(host, port).await?);
            }
            let target = DialTarget::new(remote_ip, host.to_string(), port);
            let stream = connect_tcp(target, SocketOptions::default(), socks5).await?;
            Ok(TokioIo::new(stream))
        })
    }
}

pub async fn drain_response_body(mut body: Incoming) -> Result<()> {
    while let Some(frame) = body.frame().await {
        frame.map_err(|err| {
            DnsError::plugin(format!("failed to read http response body: {}", err))
        })?;
    }
    Ok(())
}

async fn write_target_file<F>(
    path: &Path,
    mut body: Incoming,
    total: Option<u64>,
    mut progress: F,
) -> Result<()>
where
    F: FnMut(DownloadProgress),
{
    let dir = path.parent().ok_or_else(|| {
        DnsError::plugin(format!(
            "target path '{}' has no parent directory",
            path.display()
        ))
    })?;
    fs::create_dir_all(dir).await.map_err(|err| {
        DnsError::plugin(format!(
            "failed to create target directory '{}': {}",
            dir.display(),
            err
        ))
    })?;

    let tmp_path = temp_path_for(path);
    let mut temp_file = TemporaryDownloadFile::create(tmp_path.clone())
        .await
        .map_err(|err| {
            DnsError::plugin(format!(
                "failed to create temp file '{}': {}",
                tmp_path.display(),
                err
            ))
        })?;
    let mut downloaded = 0u64;
    progress(DownloadProgress { downloaded, total });
    while let Some(frame_result) = body.frame().await {
        let frame = match frame_result {
            Ok(frame) => frame,
            Err(err) => {
                return Err(DnsError::plugin(format!(
                    "failed to read response body: {err}"
                )));
            }
        };
        if let Ok(data) = frame.into_data() {
            if let Err(err) = temp_file.file_mut().write_all(&data).await {
                return Err(DnsError::plugin(format!(
                    "failed to write temp file '{}': {}",
                    tmp_path.display(),
                    err
                )));
            }
            downloaded = downloaded.saturating_add(data.len() as u64);
            progress(DownloadProgress { downloaded, total });
        }
    }
    temp_file.file_mut().sync_all().await.map_err(|err| {
        DnsError::plugin(format!(
            "failed to sync temp file '{}': {}",
            tmp_path.display(),
            err
        ))
    })?;
    temp_file.close();

    if let Err(err) = fs::rename(&tmp_path, path).await {
        let rename_fallback = matches!(
            err.kind(),
            std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::PermissionDenied
        );
        if !rename_fallback {
            return Err(DnsError::plugin(format!(
                "failed to replace target file '{}': {}",
                path.display(),
                err
            )));
        }

        if fs::try_exists(path).await.unwrap_or(false)
            && let Err(err) = fs::remove_file(path).await
        {
            return Err(DnsError::plugin(format!(
                "failed to remove existing target file '{}': {}",
                path.display(),
                err
            )));
        }
        fs::rename(&tmp_path, path).await.map_err(|err| {
            DnsError::plugin(format!(
                "failed to replace target file '{}' after fallback: {}",
                path.display(),
                err
            ))
        })?;
    }
    temp_file.persisted();

    Ok(())
}

/// Owns an incomplete download until it has been atomically persisted.
///
/// The synchronous cleanup in [`Drop`] is intentional: callers can wrap a
/// download in `tokio::time::timeout`, which cancels the future without giving
/// asynchronous cleanup code another chance to run.
struct TemporaryDownloadFile {
    path: Option<PathBuf>,
    file: Option<File>,
}

impl TemporaryDownloadFile {
    async fn create(path: PathBuf) -> io::Result<Self> {
        tokio::task::spawn_blocking(move || {
            let file = std::fs::File::create(&path)?;
            Ok(Self {
                path: Some(path),
                file: Some(File::from_std(file)),
            })
        })
        .await
        .map_err(|err| io::Error::other(format!("temp file creation task failed: {err}")))?
    }

    fn file_mut(&mut self) -> &mut File {
        self.file
            .as_mut()
            .expect("temporary download file must be open while writing")
    }

    fn close(&mut self) {
        self.file.take();
    }

    fn persisted(&mut self) {
        self.path.take();
    }
}

impl Drop for TemporaryDownloadFile {
    fn drop(&mut self) {
        self.close();
        let Some(path) = self.path.take() else {
            return;
        };
        if let Err(err) = std::fs::remove_file(&path)
            && err.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!(
                path = %path.display(),
                error = %err,
                "failed to remove incomplete download temp file"
            );
        }
    }
}

fn content_length(headers: &HeaderMap) -> Option<u64> {
    headers
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
}

fn temp_path_for(path: &Path) -> PathBuf {
    let mut tmp = path.to_path_buf();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("download");
    tmp.set_file_name(format!(".{file_name}.{nanos}.tmp"));
    tmp
}

fn request_label(method: &Method, url: &str) -> String {
    format!("{method} {url}")
}

fn parse_uri_host_ip_literal(host: &str) -> Option<IpAddr> {
    if let Some(inner) = host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
    {
        return inner.parse::<std::net::Ipv6Addr>().ok().map(IpAddr::V6);
    }
    host.parse::<std::net::Ipv4Addr>().ok().map(IpAddr::V4)
}

pub fn resolve_redirect_url(current_url: &str, location: &str) -> Result<String> {
    let base = Url::parse(current_url).map_err(|err| {
        DnsError::plugin(format!(
            "failed to parse redirect base url '{}': {}",
            current_url, err
        ))
    })?;
    base.join(location)
        .map(|url| url.to_string())
        .map_err(|err| {
            DnsError::plugin(format!(
                "failed to resolve redirect location '{}' against '{}': {}",
                location, current_url, err
            ))
        })
}

pub fn format_status(status: StatusCode) -> String {
    status
        .canonical_reason()
        .map(|reason| format!("{} {}", status.as_u16(), reason))
        .unwrap_or_else(|| status.as_u16().to_string())
}

#[cfg(test)]
mod tests {
    use std::future::pending;
    use std::net::{Ipv4Addr, SocketAddr};
    use std::time::Duration;

    use tempfile::TempDir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;
    use tokio::time::timeout;

    use super::*;

    #[test]
    fn test_resolve_redirect_url_supports_relative_location() {
        let resolved = resolve_redirect_url(
            "https://example.com/releases/latest/download/file.dat",
            "/assets/file.dat",
        )
        .expect("relative redirect should resolve");
        assert_eq!(resolved, "https://example.com/assets/file.dat");
    }

    #[test]
    fn test_request_options_builders_set_expected_fields() {
        let options = HttpRequestOptions::from_url("https://example.com")
            .with_headers(vec![(
                HeaderName::from_static("x-test"),
                HeaderValue::from_static("1"),
            )])
            .with_body(Bytes::from_static(b"body"))
            .with_max_redirects(2);
        assert_eq!(options.url, "https://example.com");
        assert_eq!(options.headers.len(), 1);
        assert_eq!(options.body, Bytes::from_static(b"body"));
        assert_eq!(options.max_redirects, 2);
    }

    #[test]
    fn test_build_hyper_request_uses_method_headers_and_body() {
        let options = HttpRequestOptions::from_url("https://example.com/api")
            .with_headers(vec![(
                HeaderName::from_static("x-test"),
                HeaderValue::from_static("1"),
            )])
            .with_body(Bytes::from_static(b"payload"));
        let request = build_hyper_request(&Method::PATCH, &options).unwrap();
        assert_eq!(request.method(), Method::PATCH);
        assert_eq!(request.uri(), "https://example.com/api");
        assert_eq!(request.headers()["x-test"], "1");
    }

    #[test]
    fn test_parse_uri_host_ip_literal_requires_bracketed_ipv6() {
        assert_eq!(
            parse_uri_host_ip_literal("[::1]"),
            Some(IpAddr::V6(std::net::Ipv6Addr::LOCALHOST))
        );
        assert_eq!(
            parse_uri_host_ip_literal("127.0.0.1"),
            Some(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST))
        );
        assert_eq!(parse_uri_host_ip_literal("::1"), None);
        assert_eq!(parse_uri_host_ip_literal("example.com"), None);
    }

    #[tokio::test]
    async fn test_download_timeout_removes_temp_file() {
        let (server_addr, server_task) = start_stalled_http_server().await;
        let dir = TempDir::new().expect("temp directory should be created");
        let target_path = dir.path().join("rules.dat");
        std::fs::write(&target_path, b"existing rules")
            .expect("existing target file should be written");
        let download_path = target_path.clone();
        let (progress_tx, progress_rx) = oneshot::channel();
        let mut progress_tx = Some(progress_tx);
        let client = HttpClient::new(HttpClientOptions::new(false, None));

        let task = tokio::spawn(async move {
            let result = timeout(
                Duration::from_millis(250),
                client.download_with_progress(
                    HttpRequestOptions::from_url(format!("http://{server_addr}/rules.dat")),
                    &download_path,
                    move |progress| {
                        if progress.downloaded > 0
                            && let Some(progress_tx) = progress_tx.take()
                        {
                            let _ = progress_tx.send(());
                        }
                    },
                ),
            )
            .await;
            assert!(result.is_err(), "stalled download should time out");
        });

        timeout(Duration::from_secs(1), progress_rx)
            .await
            .expect("partial download should arrive before test timeout")
            .expect("download task should report partial progress");
        assert_eq!(
            std::fs::read(&target_path).expect("existing target should remain readable"),
            b"existing rules"
        );
        let temp_files = download_temp_files(dir.path());
        assert_eq!(temp_files.len(), 1);

        timeout(Duration::from_secs(1), task)
            .await
            .expect("download timeout should complete within the test bound")
            .expect("download task should exit normally after timing out");
        assert!(download_temp_files(dir.path()).is_empty());
        assert_eq!(
            std::fs::read(&target_path).expect("existing target should remain after timeout"),
            b"existing rules"
        );

        server_task.abort();
        let server_error = server_task
            .await
            .expect_err("stalled server task should be cancelled by the test");
        assert!(server_error.is_cancelled());
    }

    fn download_temp_files(dir: &Path) -> Vec<std::fs::DirEntry> {
        std::fs::read_dir(dir)
            .expect("download directory should be readable")
            .collect::<std::result::Result<Vec<_>, _>>()
            .expect("download directory entries should be readable")
            .into_iter()
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect()
    }

    async fn start_stalled_http_server() -> (SocketAddr, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .expect("test HTTP listener should bind");
        let addr = listener
            .local_addr()
            .expect("test HTTP listener should have a local address");
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("test HTTP listener should accept a request");
            let mut request = Vec::new();
            let mut buffer = [0u8; 512];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let count = stream
                    .read(&mut buffer)
                    .await
                    .expect("test HTTP request should be readable");
                assert!(count > 0, "test HTTP client closed before sending headers");
                request.extend_from_slice(&buffer[..count]);
            }
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 1024\r\nConnection: close\r\n\r\npartial",
                )
                .await
                .expect("partial test HTTP response should be writable");
            pending::<()>().await;
        });
        (addr, task)
    }
}
