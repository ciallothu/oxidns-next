// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Crash-safe WebUI installation and backup handling.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::infra::error::{DnsError, Result};

pub(crate) fn find_extracted_webui(unpack_dir: &Path) -> Option<PathBuf> {
    let candidate = unpack_dir.join("webui");
    candidate.is_dir().then_some(candidate)
}

/// Recursively copies a directory tree using std only.
pub(crate) fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Moves a directory, falling back to a recursive copy when the source and
/// destination live on different filesystems.
pub(super) fn move_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::CrossesDevices => {
            copy_dir_all(from, to)?;
            fs::remove_dir_all(from)
        }
        Err(err) => Err(err),
    }
}

pub(super) fn resolve_webui_install_target(target: &Path) -> PathBuf {
    if fs::symlink_metadata(target)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
        && let Ok(resolved) = fs::canonicalize(target)
    {
        return resolved;
    }
    target.to_path_buf()
}

/// Installs the unpacked `webui/` tree into `target`, keeping the served
/// directory crash-safe.
///
/// The new tree is fully staged into a sibling of `target` first, so `target`
/// keeps serving the old UI untouched until the final swap. The final swap is a
/// same-filesystem rename (staging is a sibling), so it is atomic and cannot
/// leave a half-written served directory. The only window where `target` is
/// absent is between renaming the old tree to the backup and renaming the new
/// tree in: two single-parent renames, during which the old tree is fully
/// recoverable at the backup path.
///
/// Returns `(installed_path, backup_path)`; `backup_path` is `None` on a fresh
/// install where `target` did not previously exist.
pub(crate) fn replace_webui(
    unpacked_webui: &Path,
    target: &Path,
    backup_dir: &Path,
    version: &str,
) -> Result<(PathBuf, Option<PathBuf>)> {
    let target = resolve_webui_install_target(target);
    let target = target.as_path();
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|err| {
        DnsError::runtime(format!(
            "failed to create WebUI parent directory '{}': {}",
            parent.display(),
            err
        ))
    })?;

    let staging = target.with_extension("webui-upgrade-new");
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(|err| {
            DnsError::runtime(format!(
                "failed to clear stale WebUI staging '{}': {}",
                staging.display(),
                err
            ))
        })?;
    }
    move_dir(unpacked_webui, &staging).map_err(|err| {
        DnsError::runtime(format!(
            "failed to stage WebUI into '{}': {}",
            staging.display(),
            err
        ))
    })?;

    let backup_path = if target.exists() {
        fs::create_dir_all(backup_dir).map_err(|err| {
            DnsError::runtime(format!(
                "failed to create WebUI backup directory '{}': {}",
                backup_dir.display(),
                err
            ))
        })?;
        let path = backup_dir.join(format!(
            "webui-{}-{}",
            version,
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        ));
        if let Err(err) = move_dir(target, &path) {
            let _ = fs::remove_dir_all(&staging);
            return Err(DnsError::runtime(format!(
                "failed to back up existing WebUI '{}': {}",
                target.display(),
                err
            )));
        }
        Some(path)
    } else {
        None
    };

    if let Err(err) = fs::rename(&staging, target) {
        if let Some(ref backup) = backup_path {
            let _ = move_dir(backup, target);
        }
        let _ = fs::remove_dir_all(&staging);
        return Err(DnsError::runtime(format!(
            "failed to install WebUI into '{}': {}",
            target.display(),
            err
        )));
    }

    Ok((target.to_path_buf(), backup_path))
}
