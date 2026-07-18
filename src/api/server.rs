// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! HTTP listener and per-connection request dispatch for the management API.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use ahash::AHashMap;
use bytes::Bytes;
use http::{Method, Request, StatusCode};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as AutoBuilder;
use tokio::net::TcpStream;
use tokio::sync::{oneshot, watch};
use tokio_rustls::TlsAcceptor;
use tracing::{debug, info, warn};

use crate::api::auth::{
    AuthPrincipal, AuthService, PeerAddr, is_public_json_route, is_public_route,
};
use crate::api::cors::add_cors_headers;
use crate::api::health::HealthState;
use crate::api::request::{read_hyper_request, rewrite_request_path, strip_api_prefix};
use crate::api::route::{PrefixRoute, RouteKey, lookup_handler};
#[cfg(feature = "webui")]
use crate::api::static_files::StaticFileServer;
use crate::api::{ApiHandler, ApiResponse, json_error, simple_response};
use crate::config::types::{ApiCorsConfig, ResolvedApiHttpConfig};
use crate::infra::error::{DnsError, Result};
use crate::infra::network::listen;
use crate::infra::network::tls_config::load_server_tls_config;

pub(super) struct ApiServerContext {
    pub(super) listen: SocketAddr,
    pub(super) routes: AHashMap<RouteKey, Arc<dyn ApiHandler>>,
    pub(super) prefix_routes: Vec<PrefixRoute>,
    pub(super) tls_acceptor: Option<Arc<TlsAcceptor>>,
    pub(super) auth: Option<Arc<AuthService>>,
    pub(super) cors: Option<ApiCorsConfig>,
    #[cfg(feature = "webui")]
    pub(super) webui: Option<Arc<StaticFileServer>>,
    pub(super) health: Arc<HealthState>,
}

pub(super) fn build_tls_acceptor(
    config: &ResolvedApiHttpConfig,
) -> Result<Option<Arc<TlsAcceptor>>> {
    let Some(ssl) = &config.ssl else {
        return Ok(None);
    };
    let server_config = load_server_tls_config(
        ssl.cert.as_deref(),
        ssl.key.as_deref(),
        ssl.client_ca.as_deref(),
        ssl.require_client_cert.unwrap_or(false),
    )?;
    Ok(server_config.map(|mut cfg| {
        cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        Arc::new(TlsAcceptor::from(Arc::new(cfg)))
    }))
}

#[hotpath::measure]
pub(super) async fn run_api_server(
    context: ApiServerContext,
    shutdown_rx: &mut watch::Receiver<bool>,
    startup_tx: oneshot::Sender<std::result::Result<(), String>>,
) {
    let context = Arc::new(context);
    let listener = match listen::build_tcp_listener(context.listen, 512, |_| {}) {
        Ok(listener) => listener,
        Err(err) => {
            let _ = startup_tx.send(Err(format!(
                "failed to bind API listener on {}: {}",
                context.listen, err
            )));
            return;
        }
    };
    context.health.mark_api_listening();
    let _ = startup_tx.send(Ok(()));
    #[cfg(feature = "webui")]
    let webui_enabled = context.webui.is_some();
    #[cfg(not(feature = "webui"))]
    let webui_enabled = false;
    info!(
        listen = %context.listen,
        tls = %context.tls_acceptor.is_some(),
        auth = %context.auth.is_some(),
        cors = %context.cors.is_some(),
        webui = %webui_enabled,
        routes = context.routes.len(),
        prefix_routes = context.prefix_routes.len(),
        "Management API listening"
    );

    loop {
        tokio::select! {
            changed = shutdown_rx.changed() => {
                if changed.is_ok() && *shutdown_rx.borrow() {
                    break;
                }
            }
            accepted = listener.accept() => {
                let (stream, remote_addr) = match accepted {
                    Ok(item) => item,
                    Err(err) => {
                        warn!(error = %err, "API accept failed");
                        continue;
                    }
                };
                let context = context.clone();
                tokio::spawn(async move {
                    if let Err(err) = handle_connection(stream, remote_addr, context).await {
                        let msg = err.to_string();
                        if msg.contains("connection closed")
                            || msg.contains("broken pipe")
                            || msg.contains("reset by peer")
                            || msg.contains("Connection reset")
                        {
                            debug!(remote = %remote_addr, error = %err, "API connection closed");
                        } else {
                            warn!(remote = %remote_addr, error = %err, "API connection failed");
                        }
                    }
                });
            }
        }
    }
}

async fn handle_connection(
    stream: TcpStream,
    remote_addr: SocketAddr,
    context: Arc<ApiServerContext>,
) -> Result<()> {
    match &context.tls_acceptor {
        Some(acceptor) => {
            let stream = acceptor
                .accept(stream)
                .await
                .map_err(|err| DnsError::runtime(format!("API TLS handshake failed: {err}")))?;
            handle_hyper_stream(stream, remote_addr, context).await
        }
        None => handle_hyper_stream(stream, remote_addr, context).await,
    }
}

async fn handle_hyper_stream<S>(
    stream: S,
    remote_addr: SocketAddr,
    context: Arc<ApiServerContext>,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Sync + Unpin + 'static,
{
    let service = service_fn(move |request: Request<Incoming>| {
        let context = context.clone();
        async move { handle_hyper_request(request, remote_addr, context).await }
    });

    let io = TokioIo::new(stream);
    AutoBuilder::new(TokioExecutor::new())
        .serve_connection_with_upgrades(io, service)
        .await
        .map_err(|err| DnsError::runtime(format!("API hyper connection failed: {err}")))
}

#[hotpath::measure]
async fn handle_hyper_request(
    request: Request<Incoming>,
    remote_addr: SocketAddr,
    context: Arc<ApiServerContext>,
) -> std::result::Result<ApiResponse, Infallible> {
    let request = match read_hyper_request(request).await {
        Ok(request) => request,
        Err(status) => return Ok(simple_response(status, Bytes::new())),
    };

    debug!(
        remote = %remote_addr,
        method = %request.method(),
        path = %request.uri().path(),
        body_len = request.body().len(),
        "API request received"
    );

    let request_headers = request.headers().clone();
    let Some(api_path) = strip_api_prefix(request.uri().path()) else {
        #[cfg(feature = "webui")]
        {
            return Ok(match &context.webui {
                Some(webui) => webui.handle(request).await,
                None => simple_response(StatusCode::NOT_FOUND, Bytes::from("404 Not Found")),
            });
        }
        #[cfg(not(feature = "webui"))]
        {
            return Ok(simple_response(
                StatusCode::NOT_FOUND,
                Bytes::from("404 Not Found"),
            ));
        }
    };
    let mut request = match rewrite_request_path(request, &api_path) {
        Ok(request) => request,
        Err(()) => return Ok(simple_response(StatusCode::BAD_REQUEST, Bytes::new())),
    };
    request.extensions_mut().insert(PeerAddr(remote_addr));

    // Handle CORS preflight requests before authentication so browsers can
    // discover whether credentials are allowed.
    if request.method() == Method::OPTIONS {
        if let Some(ref cors_cfg) = context.cors {
            let mut response = simple_response(StatusCode::NO_CONTENT, Bytes::new());
            add_cors_headers(response.headers_mut(), Some(&request_headers), cors_cfg);
            return Ok(response);
        }
        return Ok(simple_response(
            StatusCode::METHOD_NOT_ALLOWED,
            Bytes::new(),
        ));
    }

    let public_route = is_public_route(request.method(), api_path.as_str());
    if is_public_json_route(request.method(), api_path.as_str()) {
        if !has_json_content_type(request.headers()) {
            return Ok(with_cors(
                json_error(
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    "json_content_type_required",
                    "Content-Type: application/json is required",
                ),
                &request_headers,
                context.cors.as_ref(),
            ));
        }
        if !public_auth_origin_allowed(&request, &context) {
            return Ok(with_cors(
                json_error(
                    StatusCode::FORBIDDEN,
                    "origin_not_allowed",
                    "request Origin is not allowed",
                ),
                &request_headers,
                context.cors.as_ref(),
            ));
        }
    }
    if let Some(auth) = &context.auth {
        let had_session_cookie = auth.has_session_cookie(request.headers());
        let principal = match auth.authenticate(request.headers()).await {
            Ok(principal) => principal,
            Err(err) => {
                warn!(error = %err, "Authentication session lookup failed");
                return Ok(with_cors(
                    json_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "internal_error",
                        "authentication service failed",
                    ),
                    &request_headers,
                    context.cors.as_ref(),
                ));
            }
        };
        if let Some(principal) = principal {
            request.extensions_mut().insert(principal);
        } else if !public_route {
            let (code, message) = if had_session_cookie {
                ("session_expired", "authentication session expired")
            } else {
                ("unauthorized", "authentication required")
            };
            return Ok(with_cors(
                json_error(StatusCode::UNAUTHORIZED, code, message),
                &request_headers,
                context.cors.as_ref(),
            ));
        }

        if !public_route && is_mutating(request.method()) {
            let csrf_valid = request
                .extensions()
                .get::<AuthPrincipal>()
                .is_some_and(|principal| auth.verify_csrf(request.headers(), principal));
            if !csrf_valid {
                return Ok(with_cors(
                    json_error(
                        StatusCode::FORBIDDEN,
                        "invalid_csrf_token",
                        "valid X-CSRF-Token header required",
                    ),
                    &request_headers,
                    context.cors.as_ref(),
                ));
            }
        }
    }

    let response = if let Some(handler) = lookup_handler(
        request.method(),
        api_path.as_str(),
        &context.routes,
        &context.prefix_routes,
    ) {
        with_cors(
            handler.handle(request).await,
            &request_headers,
            context.cors.as_ref(),
        )
    } else {
        with_cors(
            simple_response(StatusCode::NOT_FOUND, Bytes::from("404 Not Found")),
            &request_headers,
            context.cors.as_ref(),
        )
    };

    Ok(response)
}

fn is_mutating(method: &Method) -> bool {
    method != Method::GET && method != Method::HEAD && method != Method::OPTIONS
}

fn has_json_content_type(headers: &http::HeaderMap) -> bool {
    let values = headers.get_all(http::header::CONTENT_TYPE);
    let mut values = values.iter();
    let Some(value) = values.next().and_then(|value| value.to_str().ok()) else {
        return false;
    };
    values.next().is_none()
        && value
            .split(';')
            .next()
            .is_some_and(|media_type| media_type.trim().eq_ignore_ascii_case("application/json"))
}

fn public_auth_origin_allowed(request: &http::Request<Bytes>, context: &ApiServerContext) -> bool {
    let origin_values = request.headers().get_all(http::header::ORIGIN);
    let mut origins = origin_values.iter();
    let Some(origin) = origins.next() else {
        return request
            .headers()
            .get("sec-fetch-site")
            .and_then(|value| value.to_str().ok())
            .is_none_or(|value| matches!(value, "same-origin" | "none"));
    };
    if origins.next().is_some() {
        return false;
    }
    let Some(origin_value) = origin.to_str().ok() else {
        return false;
    };
    let Some(origin) = canonical_origin(origin_value) else {
        return false;
    };
    if origin != origin_value {
        return false;
    }
    if context
        .auth
        .as_ref()
        .is_some_and(|auth| auth.allows_public_origin(&origin))
    {
        return true;
    }
    if request_origin(request, context.tls_acceptor.is_some()).as_deref() == Some(origin.as_str()) {
        return true;
    }
    context.cors.as_ref().is_some_and(|cors| {
        cors.allowed_origins
            .iter()
            .any(|allowed| allowed != "*" && allowed == &origin)
    })
}

fn request_origin(request: &http::Request<Bytes>, tls: bool) -> Option<String> {
    let host_values = request.headers().get_all(http::header::HOST);
    let mut hosts = host_values.iter();
    let host = hosts.next()?.to_str().ok()?;
    if hosts.next().is_some() {
        return None;
    }
    canonical_origin(&format!("{}://{host}", if tls { "https" } else { "http" }))
}

fn canonical_origin(value: &str) -> Option<String> {
    let url = url::Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    Some(url.origin().ascii_serialization())
}

fn with_cors(
    mut response: ApiResponse,
    request_headers: &http::HeaderMap,
    cors: Option<&ApiCorsConfig>,
) -> ApiResponse {
    if let Some(cors_cfg) = cors {
        add_cors_headers(response.headers_mut(), Some(request_headers), cors_cfg);
    }
    response
}
