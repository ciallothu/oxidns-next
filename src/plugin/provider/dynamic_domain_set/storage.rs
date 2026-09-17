// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use fs2::FileExt;

use super::rules::{DynamicDomainRuleKind, canonicalize_rule};
use crate::infra::error::{DnsError, Result as DnsResult};
use crate::infra::io::{LineClassifier, TextSource};

pub(super) fn read_rule_file(path: &Path) -> DnsResult<Vec<Arc<str>>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let files = vec![path.display().to_string()];
    let mut rules = Vec::new();
    let mut seen = HashSet::new();
    TextSource::new("dynamic_domain_set.rules", &[], &files)
        .scan(&LineClassifier::new(&["#"]), |line| -> DnsResult<()> {
            if line.annotations().blank || line.annotations().leading_comment.is_some() {
                return Ok(());
            }
            // Existing text files follow `domain_set` semantics: bare domains
            // mean suffix-domain rules. Auto-learned exact rules are written
            // with an explicit `full:` prefix.
            let rule = canonicalize_rule(
                line.trimmed(),
                DynamicDomainRuleKind::Domain,
                line.location(),
            )?;
            let rule: Arc<str> = Arc::from(rule);
            if seen.insert(rule.clone()) {
                rules.push(rule);
            }
            Ok(())
        })
        .map_err(|error| {
            DnsError::plugin(format!("failed to load dynamic domain rules: {error}"))
        })?;
    Ok(rules)
}

pub(super) fn append_rule_file<T: AsRef<str>>(path: &Path, rules: &[T]) -> DnsResult<()> {
    if rules.is_empty() {
        return Ok(());
    }
    // Append is used only for newly staged rules. Full rewrites are reserved
    // for delete/clear so the common learn path avoids rewriting large files.
    with_rule_file_lock(path, || {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(path)?;
        let file_len = file.metadata()?.len();
        if file_len > 0 {
            let mut last = [0_u8; 1];
            file.seek(SeekFrom::Start(file_len - 1))?;
            file.read_exact(&mut last)?;
            if last[0] != b'\n' {
                // External edits may leave the file without a trailing newline.
                // Separate the first appended rule from the previous line so a
                // later reload sees the same canonical rules the hot snapshot
                // already contains.
                writeln!(file)?;
            }
        }
        for rule in rules {
            writeln!(file, "{}", rule.as_ref())?;
        }
        file.sync_all()?;
        Ok(())
    })
}

pub(super) fn rewrite_rule_file<T: AsRef<str>>(path: &Path, rules: &[T]) -> DnsResult<()> {
    with_rule_file_lock(path, || {
        let tmp_path = temp_path_for(path);
        {
            let mut file = File::create(&tmp_path)?;
            for rule in rules {
                writeln!(file, "{}", rule.as_ref())?;
            }
            file.sync_all()?;
        }
        // Rename keeps readers from observing a partially rewritten file on
        // platforms where same-directory rename is atomic.
        fs::rename(&tmp_path, path)?;
        Ok(())
    })
}

fn with_rule_file_lock<F>(path: &Path, op: F) -> DnsResult<()>
where
    F: FnOnce() -> std::io::Result<()>,
{
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    // A side-car lock file avoids locking the rule file being atomically
    // renamed. It is advisory, but it reduces corruption risk when two OxiDNS
    // Next processes accidentally manage the same path.
    let lock_path = lock_path_for(path);
    let lock_file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    lock_file.lock_exclusive()?;
    let result = op();
    let unlock_result = lock_file.unlock();
    result?;
    unlock_result?;
    Ok(())
}

fn lock_path_for(path: &Path) -> PathBuf {
    let mut lock_name = path.as_os_str().to_os_string();
    lock_name.push(".lock");
    PathBuf::from(lock_name)
}

fn temp_path_for(path: &Path) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let mut tmp_name = path.as_os_str().to_os_string();
    tmp_name.push(format!(".tmp.{}.{}", std::process::id(), now));
    PathBuf::from(tmp_name)
}
