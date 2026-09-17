// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! GitHub release discovery and upgrade asset selection.

use http::header::{AUTHORIZATION, HeaderValue, USER_AGENT};
use semver::Version;
use serde::Deserialize;
use tokio::time::timeout;

use super::{UpgradeBundle, UpgradeConfig};
use crate::infra::error::{DnsError, Result};
use crate::infra::network::http_client::{HttpClient, HttpClientOptions, HttpRequestOptions};

const GITHUB_USER_AGENT: &str = "OxiDNS";

pub(super) async fn fetch_release(config: &UpgradeConfig) -> Result<GitHubRelease> {
    let url = if config.target.trim() == "latest" {
        format!(
            "https://api.github.com/repos/{}/releases/latest",
            config.repository
        )
    } else {
        format!(
            "https://api.github.com/repos/{}/releases/tags/{}",
            config.repository,
            config.target.trim()
        )
    };
    let client = build_asset_http_client(config)?;
    let response = timeout(
        config.timeout,
        client.get_request(
            HttpRequestOptions::from_url(url.as_str())
                .with_headers(github_request_headers(config.github_token.as_deref())),
        ),
    )
    .await
    .map_err(|_| DnsError::runtime("GitHub release request timed out"))??;
    let release = serde_json::from_slice::<GitHubRelease>(&response.body).map_err(|err| {
        DnsError::runtime(format!("failed to parse GitHub release response: {err}"))
    })?;
    if release.prerelease && !config.allow_prerelease {
        return Err(DnsError::runtime(format!(
            "release '{}' is a prerelease; pass allow_prerelease to use it",
            release.tag_name
        )));
    }
    Ok(release)
}

pub(super) fn github_request_headers(
    token: Option<&str>,
) -> Vec<(http::header::HeaderName, HeaderValue)> {
    let mut headers = vec![(USER_AGENT, HeaderValue::from_static(GITHUB_USER_AGENT))];
    if let Some(token) = token.map(str::trim).filter(|token| !token.is_empty())
        && let Ok(value) = HeaderValue::try_from(format!("Bearer {token}"))
    {
        headers.push((AUTHORIZATION, value));
    }
    headers
}

pub(super) fn build_asset_http_client(config: &UpgradeConfig) -> Result<HttpClient> {
    Ok(HttpClient::new(HttpClientOptions::from_outbound(
        config.insecure_skip_verify,
        config.outbound.as_deref(),
        config.socks5.as_deref(),
        |raw| DnsError::runtime(format!("invalid upgrade socks5 proxy '{raw}'")),
    )?))
}

pub(super) fn select_asset<'a>(
    config: &UpgradeConfig,
    release: &'a GitHubRelease,
) -> Result<&'a ReleaseAsset> {
    if config.asset.trim() != "auto" {
        return find_asset(release, config.asset.trim());
    }
    let expected = current_archive_name(config.bundle)?;
    find_asset(release, &expected)
}

pub(super) fn find_asset<'a>(release: &'a GitHubRelease, name: &str) -> Result<&'a ReleaseAsset> {
    release
        .assets
        .iter()
        .find(|asset| asset.name == name)
        .ok_or_else(|| {
            DnsError::runtime(format!(
                "release '{}' does not contain asset '{}'",
                release.tag_name, name
            ))
        })
}

pub(super) fn current_archive_name(bundle: UpgradeBundle) -> Result<String> {
    let selected = resolve_requested_bundle(bundle, crate::build_info::PRIMARY_BUNDLE)?;
    let target = current_release_target()?;
    let target = release_target_for_bundle(selected, target);
    let ext = if cfg!(windows) { "zip" } else { "tar.gz" };
    archive_name_for_bundle(selected, target.as_str(), ext)
}

pub(super) fn resolve_requested_bundle(
    requested: UpgradeBundle,
    primary_bundle: &str,
) -> Result<UpgradeBundle> {
    match requested {
        UpgradeBundle::Auto => match primary_bundle {
            "full" => Ok(UpgradeBundle::Full),
            "minimal" => Ok(UpgradeBundle::Minimal),
            "standard" => Ok(UpgradeBundle::Standard),
            "custom" => Err(DnsError::runtime(
                "current build bundle is custom; pass --bundle full|minimal|standard or --asset <NAME>",
            )),
            other => Err(DnsError::runtime(format!(
                "unsupported current build bundle '{other}'; pass --bundle full|minimal|standard or --asset <NAME>"
            ))),
        },
        bundle => Ok(bundle),
    }
}

pub(super) fn archive_name_for_bundle(
    bundle: UpgradeBundle,
    target: &str,
    ext: &str,
) -> Result<String> {
    match bundle {
        UpgradeBundle::Full => Ok(format!("oxidns-next-{target}.{ext}")),
        UpgradeBundle::Minimal | UpgradeBundle::Standard => {
            Ok(format!("oxidns-next-{}-{target}.{ext}", bundle.as_str()))
        }
        UpgradeBundle::Auto => Err(DnsError::runtime(
            "upgrade bundle auto must be resolved before archive naming",
        )),
    }
}

pub(super) fn release_target_for_bundle(bundle: UpgradeBundle, target: String) -> String {
    let target = match target.as_str() {
        "i686-unknown-linux-gnu" => "i686-unknown-linux-musl".to_string(),
        "arm-unknown-linux-gnueabihf" => "arm-unknown-linux-musleabihf".to_string(),
        "armv7-unknown-linux-gnueabihf" => "armv7-unknown-linux-musleabihf".to_string(),
        "x86_64-pc-windows-gnu" | "x86_64-pc-windows-gnullvm" => {
            "x86_64-pc-windows-msvc".to_string()
        }
        "i686-pc-windows-gnu" | "i686-pc-windows-gnullvm" => "i686-pc-windows-msvc".to_string(),
        "aarch64-pc-windows-gnullvm" => "aarch64-pc-windows-msvc".to_string(),
        _ => target,
    };

    if matches!(bundle, UpgradeBundle::Minimal | UpgradeBundle::Standard) {
        match target.as_str() {
            "x86_64-unknown-linux-gnu" => return "x86_64-unknown-linux-musl".to_string(),
            "aarch64-unknown-linux-gnu" => return "aarch64-unknown-linux-musl".to_string(),
            _ => {}
        }
    }
    target
}

pub(super) fn current_release_target() -> Result<String> {
    if let Some(target) = option_env!("OXIDNS_BUILD_TARGET").filter(|target| !target.is_empty()) {
        return Ok(target.to_string());
    }

    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        "x86" => "i686",
        "arm" => "arm",
        other => {
            return Err(DnsError::runtime(format!(
                "unsupported upgrade architecture '{other}'"
            )));
        }
    };
    let target = match std::env::consts::OS {
        "linux" => {
            if arch == "arm" {
                "arm-unknown-linux-musleabihf".to_string()
            } else {
                format!("{arch}-unknown-linux-musl")
            }
        }
        "macos" => format!("{arch}-apple-darwin"),
        "freebsd" => format!("{arch}-unknown-freebsd"),
        "windows" => format!("{arch}-pc-windows-msvc"),
        other => {
            return Err(DnsError::runtime(format!(
                "unsupported upgrade OS '{other}'"
            )));
        }
    };
    Ok(target)
}

pub(super) fn sha256_from_asset_digest(asset: &ReleaseAsset) -> Result<String> {
    let raw = asset.digest.as_deref().ok_or_else(|| {
        DnsError::runtime(format!(
            "release asset '{}' does not include a digest",
            asset.name
        ))
    })?;
    let Some(hash) = raw.strip_prefix("sha256:") else {
        return Err(DnsError::runtime(format!(
            "release asset '{}' uses unsupported digest '{}'",
            asset.name, raw
        )));
    };
    if hash.len() != 64 || hex::decode(hash).is_err() {
        return Err(DnsError::runtime(format!(
            "release asset '{}' has invalid SHA256 digest '{}'",
            asset.name, raw
        )));
    }
    Ok(hash.to_ascii_lowercase())
}

pub(super) fn is_newer_version(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Ok(candidate), Ok(current)) => candidate > current,
        _ => candidate != current,
    }
}

pub(super) fn parse_version(raw: &str) -> std::result::Result<Version, semver::Error> {
    Version::parse(raw.trim_start_matches('v'))
}

#[derive(Debug, Deserialize)]
pub(super) struct GitHubRelease {
    pub(super) tag_name: String,
    pub(super) prerelease: bool,
    pub(super) html_url: Option<String>,
    pub(super) assets: Vec<ReleaseAsset>,
}

impl GitHubRelease {
    pub(super) fn version_string(&self) -> String {
        self.tag_name.trim_start_matches('v').to_string()
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct ReleaseAsset {
    pub(super) name: String,
    pub(super) browser_download_url: String,
    pub(super) digest: Option<String>,
}
