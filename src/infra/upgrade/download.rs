// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Upgrade archive download and checksum verification.

use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::Path;

use sha2::{Digest, Sha256};
use tokio::time::timeout;

use super::release::{
    build_asset_http_client, fetch_release, github_request_headers, select_asset,
    sha256_from_asset_digest,
};
use super::{UpgradeConfig, UpgradeDownload};
use crate::infra::error::{DnsError, Result};
use crate::infra::network::http_client::{DownloadProgress, HttpRequestOptions};

pub(crate) async fn download<F>(config: &UpgradeConfig, progress: F) -> Result<UpgradeDownload>
where
    F: FnMut(DownloadProgress),
{
    let release = fetch_release(config).await?;
    let asset = select_asset(config, &release)?;
    let expected = sha256_from_asset_digest(asset)?;
    let client = build_asset_http_client(config)?;
    fs::create_dir_all(&config.cache_dir).map_err(|err| {
        DnsError::runtime(format!(
            "failed to create upgrade cache directory '{}': {}",
            config.cache_dir.display(),
            err
        ))
    })?;

    let archive_path = config.cache_dir.join(&asset.name);
    timeout(
        config.timeout,
        client.download_with_progress(
            HttpRequestOptions::from_url(asset.browser_download_url.as_str())
                .with_headers(github_request_headers(config.github_token.as_deref())),
            &archive_path,
            progress,
        ),
    )
    .await
    .map_err(|_| DnsError::runtime("upgrade archive download timed out"))??;

    verify_sha256(&archive_path, &expected)?;
    Ok(UpgradeDownload {
        version: release.version_string(),
        asset_name: asset.name.clone(),
        archive_path,
        sha256: expected,
    })
}

fn verify_sha256(path: &Path, expected: &str) -> Result<()> {
    let actual = sha256_file(path)?;
    if actual != expected.to_ascii_lowercase() {
        return Err(DnsError::runtime(format!(
            "SHA256 mismatch for '{}': expected {}, got {}",
            path.display(),
            expected,
            actual
        )));
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String> {
    let file = File::open(path).map_err(|err| {
        DnsError::runtime(format!("failed to open '{}': {}", path.display(), err))
    })?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];
    loop {
        let read = reader.read(&mut buffer).map_err(|err| {
            DnsError::runtime(format!("failed to read '{}': {}", path.display(), err))
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}
