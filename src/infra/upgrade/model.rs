// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Upgrade configuration and public outcome types.

use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;

use crate::infra::error::{DnsError, Result};

const DEFAULT_REPOSITORY: &str = "ciallothu/oxidns-next";
const DEFAULT_TARGET: &str = "latest";
const DEFAULT_CACHE_DIR: &str = "./upgrade-cache";
const DEFAULT_BACKUP_DIR: &str = "./upgrade-backups";
const DEFAULT_WEBUI_DIR: &str = "./webui";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum UpgradeBundle {
    #[default]
    Auto,
    Full,
    Minimal,
    Standard,
}

impl UpgradeBundle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Full => "full",
            Self::Minimal => "minimal",
            Self::Standard => "standard",
        }
    }

    pub fn from_user_value(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "full" => Ok(Self::Full),
            "minimal" => Ok(Self::Minimal),
            "standard" => Ok(Self::Standard),
            other => Err(DnsError::runtime(format!(
                "invalid upgrade bundle '{other}', expected auto, full, minimal, or standard"
            ))),
        }
    }
}

#[derive(Debug, Clone)]
pub struct UpgradeConfig {
    pub target: String,
    pub repository: String,
    pub asset: String,
    pub bundle: UpgradeBundle,
    pub cache_dir: PathBuf,
    pub backup_dir: PathBuf,
    pub webui_dir: PathBuf,
    pub skip_webui: bool,
    pub no_restart: bool,
    pub allow_prerelease: bool,
    pub force: bool,
    pub cleanup_after_apply: bool,
    pub timeout: Duration,
    pub outbound: Option<String>,
    pub socks5: Option<String>,
    pub insecure_skip_verify: bool,
    pub github_token: Option<String>,
}

impl Default for UpgradeConfig {
    fn default() -> Self {
        Self {
            target: DEFAULT_TARGET.to_string(),
            repository: DEFAULT_REPOSITORY.to_string(),
            asset: "auto".to_string(),
            bundle: UpgradeBundle::Auto,
            cache_dir: PathBuf::from(DEFAULT_CACHE_DIR),
            backup_dir: PathBuf::from(DEFAULT_BACKUP_DIR),
            webui_dir: PathBuf::from(DEFAULT_WEBUI_DIR),
            skip_webui: false,
            no_restart: false,
            allow_prerelease: false,
            force: false,
            cleanup_after_apply: false,
            timeout: Duration::from_secs(30),
            outbound: None,
            socks5: None,
            insecure_skip_verify: false,
            github_token: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct UpgradeDownload {
    pub version: String,
    pub asset_name: String,
    pub archive_path: PathBuf,
    pub sha256: String,
}

#[derive(Debug, Clone)]
pub struct UpgradeCheck {
    pub current_version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub asset_name: String,
    pub release_url: String,
}

#[derive(Debug, Clone)]
pub struct ApplyOutcome {
    pub installed_version: String,
    pub asset_name: String,
    pub backup_path: PathBuf,
    pub binary_path: PathBuf,
    /// Whether the caller should restart the running service/process after the
    /// binary replacement.
    pub restart_required: bool,
    /// `Some` when the WebUI directory was installed; `None` when skipped or
    /// when the archive did not contain a `webui/` directory.
    pub webui_path: Option<PathBuf>,
    /// `Some` when an existing WebUI directory was backed up before the swap;
    /// `None` on a fresh install where there was nothing to back up.
    pub webui_backup_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpgradeContext {
    Cli,
    Plugin,
}

#[derive(Debug, Clone)]
pub enum ApplyDecision {
    Apply { check: UpgradeCheck },
    Skip { check: UpgradeCheck },
}

#[derive(Debug, Clone)]
pub enum ApplyRunOutcome {
    Applied {
        check: UpgradeCheck,
        outcome: ApplyOutcome,
    },
    Skipped {
        check: UpgradeCheck,
    },
}
