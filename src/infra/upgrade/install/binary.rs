// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Atomic binary replacement for supported platforms.

use std::fs;
use std::path::Path;

use crate::infra::error::{DnsError, Result};

/// Windows binary replacement using the rename trick.
///
/// Windows prevents overwriting a running executable but allows renaming it.
/// This function stages the new binary first, renames the running exe to the
/// backup path, then moves the staged binary to the original path.
#[cfg(windows)]
pub(crate) fn replace_binary_windows(
    source: &Path,
    target: &Path,
    backup_path: &Path,
) -> Result<()> {
    if let Some(parent) = backup_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let staging = target.with_extension("upgrade-new.exe");
    fs::copy(source, &staging).map_err(|e| {
        DnsError::runtime(format!(
            "failed to stage new binary '{}': {e}",
            staging.display()
        ))
    })?;
    // Rename the running exe to backup (allowed by Windows even while running).
    if let Err(e) = fs::rename(target, backup_path) {
        let _ = fs::remove_file(&staging);
        return Err(DnsError::runtime(format!(
            "failed to move running binary to backup '{}': {e}",
            backup_path.display()
        )));
    }
    // Move staged binary to the original path.
    if let Err(e) = fs::rename(&staging, target) {
        let _ = fs::rename(backup_path, target); // attempt rollback
        let _ = fs::remove_file(&staging);
        return Err(DnsError::runtime(format!(
            "failed to place new binary at '{}': {e}",
            target.display()
        )));
    }
    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn replace_binary(source: &Path, target: &Path) -> Result<()> {
    let tmp = target.with_extension("oxidns-next-upgrade-new");
    fs::copy(source, &tmp).map_err(|err| {
        DnsError::runtime(format!(
            "failed to stage upgraded binary '{}': {}",
            tmp.display(),
            err
        ))
    })?;
    let permissions = fs::metadata(source)?.permissions();
    fs::set_permissions(&tmp, permissions)?;
    fs::rename(&tmp, target).map_err(|err| {
        let _ = fs::remove_file(&tmp);
        DnsError::runtime(format!(
            "failed to replace binary '{}': {}",
            target.display(),
            err
        ))
    })
}
