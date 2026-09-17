// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Upgrade progress reporting shared by CLI and plugin callers.

use std::io::Write;

use tracing::info;

use super::UpgradeContext;
use crate::infra::network::http_client::DownloadProgress;

pub(crate) struct UpgradeDownloadProgressReporter {
    context: UpgradeContext,
    state: std::sync::Arc<std::sync::Mutex<UpgradeDownloadProgressState>>,
}

#[derive(Debug, Default)]
struct UpgradeDownloadProgressState {
    last_percent_bucket: Option<u64>,
    last_unknown_bucket: u64,
}

impl UpgradeDownloadProgressReporter {
    pub(crate) fn new(context: UpgradeContext) -> Self {
        Self {
            context,
            state: Default::default(),
        }
    }

    pub(crate) fn report(&self, progress: DownloadProgress) {
        match self.context {
            UpgradeContext::Cli => self.report_cli(progress),
            UpgradeContext::Plugin => self.report_plugin(progress),
        }
    }

    fn report_cli(&self, progress: DownloadProgress) {
        match progress.total {
            Some(total) if total > 0 => {
                let percent = progress.downloaded.saturating_mul(100) / total;
                print!(
                    "\rDownload progress: {}% ({}/{})",
                    percent,
                    format_bytes(progress.downloaded),
                    format_bytes(total)
                );
                let _ = std::io::stdout().flush();
                if progress.downloaded >= total {
                    println!();
                }
            }
            _ => {
                print!("\rDownload progress: {}", format_bytes(progress.downloaded));
                let _ = std::io::stdout().flush();
            }
        }
    }

    fn report_plugin(&self, progress: DownloadProgress) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };

        match progress.total {
            Some(total) if total > 0 => {
                let percent = progress.downloaded.saturating_mul(100) / total;
                let bucket = (percent / 10) * 10;
                let should_log = state.last_percent_bucket != Some(bucket)
                    || progress.downloaded >= total && state.last_percent_bucket != Some(100);
                if should_log {
                    state.last_percent_bucket = Some(bucket);
                    info!(
                        downloaded = progress.downloaded,
                        total, percent, "upgrade archive download progress"
                    );
                }
            }
            _ => {
                let bucket = progress.downloaded / (1024 * 1024);
                if bucket > state.last_unknown_bucket {
                    state.last_unknown_bucket = bucket;
                    info!(
                        downloaded = progress.downloaded,
                        "upgrade archive download progress"
                    );
                }
            }
        }
    }
}

fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;

    let bytes_f = bytes as f64;
    if bytes_f >= GIB {
        format!("{:.1} GiB", bytes_f / GIB)
    } else if bytes_f >= MIB {
        format!("{:.1} MiB", bytes_f / MIB)
    } else if bytes_f >= KIB {
        format!("{:.1} KiB", bytes_f / KIB)
    } else {
        format!("{bytes} B")
    }
}
