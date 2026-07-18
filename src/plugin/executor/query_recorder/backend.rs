// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Sender as ReplySender, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rusqlite::Connection;
use tokio::sync::{Semaphore, broadcast, mpsc, oneshot};
use tracing::{error, warn};

use super::model::{
    PendingRecord, RecordDetail, ResolvedDatabaseConfig, ResolvedRecorderConfig, TableNames,
};
use super::remote::RemotePool;
use super::store::{
    create_schema, open_reader_database, open_writer_database, portable_table_names,
    run_writer_thread, table_names,
};
use crate::infra::error::{DnsError, Result};

#[derive(Debug)]
pub(super) enum StorageBackend {
    Sqlite { path: PathBuf },
    Remote(RemotePool),
}

#[derive(Debug)]
enum WriterQueue {
    Blocking(SyncSender<WriterCommand>),
    Async(mpsc::Sender<WriterCommand>),
}

impl WriterQueue {
    fn try_send(&self, command: WriterCommand) -> std::result::Result<(), String> {
        match self {
            Self::Blocking(sender) => sender.try_send(command).map_err(|err| err.to_string()),
            Self::Async(sender) => sender.try_send(command).map_err(|err| err.to_string()),
        }
    }

    fn blocking_send(&self, command: WriterCommand) -> std::result::Result<(), String> {
        match self {
            Self::Blocking(sender) => sender.send(command).map_err(|err| err.to_string()),
            Self::Async(sender) => sender.blocking_send(command).map_err(|err| err.to_string()),
        }
    }
}

#[derive(Debug)]
enum WriterHandle {
    Blocking(JoinHandle<()>),
    Async(tokio::task::JoinHandle<()>),
}

#[derive(Debug)]
pub(super) struct RecorderBackend {
    pub(super) tag: String,
    pub(super) storage: StorageBackend,
    pub(super) tables: TableNames,
    queue_tx: WriterQueue,
    pub(super) stop_requested: Arc<AtomicBool>,
    writer_handle: Mutex<Option<WriterHandle>>,
    pub(super) tail: Arc<Mutex<VecDeque<RecordDetail>>>,
    pub(super) memory_tail: usize,
    pub(super) broadcaster: broadcast::Sender<RecordDetail>,
    pub(super) dropped_total: Arc<AtomicU64>,
    pub(super) reader_semaphore: Arc<Semaphore>,
    pub(super) acquire_timeout: Duration,
    pub(super) query_timeout: Duration,
}

#[derive(Debug, Clone)]
pub(super) struct ClearHistoryResult {
    pub(super) cleared_records: usize,
}

pub(super) type ClearHistoryReply = std::result::Result<ClearHistoryResult, String>;
#[cfg(test)]
pub(super) type FlushReply = std::result::Result<(), String>;

#[derive(Debug)]
pub(super) enum WriterCommand {
    Insert(Box<PendingRecord>),
    Cleanup {
        cutoff_ms: i64,
    },
    ClearHistory {
        reply_tx: ReplySender<ClearHistoryReply>,
    },
    #[cfg(test)]
    Flush {
        reply_tx: ReplySender<FlushReply>,
    },
}

#[derive(Debug)]
pub(super) struct WriterThreadContext {
    pub(super) tables: TableNames,
    pub(super) stop_requested: Arc<AtomicBool>,
    pub(super) tail: Arc<Mutex<VecDeque<RecordDetail>>>,
    pub(super) memory_tail: usize,
    pub(super) broadcaster: broadcast::Sender<RecordDetail>,
    pub(super) batch_size: usize,
    pub(super) flush_interval: Duration,
}

#[derive(Debug)]
struct RemoteWriterContext {
    tables: TableNames,
    stop_requested: Arc<AtomicBool>,
    tail: Arc<Mutex<VecDeque<RecordDetail>>>,
    memory_tail: usize,
    broadcaster: broadcast::Sender<RecordDetail>,
    dropped_total: Arc<AtomicU64>,
    batch_size: usize,
    flush_interval: Duration,
    query_timeout: Duration,
}

impl RecorderBackend {
    pub(super) async fn run(tag: String, config: ResolvedRecorderConfig) -> Result<Arc<Self>> {
        let tables = match &config.database {
            ResolvedDatabaseConfig::Sqlite { .. } => table_names(&tag),
            ResolvedDatabaseConfig::Postgres { .. } | ResolvedDatabaseConfig::Mysql { .. } => {
                portable_table_names(&tag)
            }
        };
        let tail = Arc::new(Mutex::new(VecDeque::with_capacity(
            config.memory_tail.max(1),
        )));
        let (broadcaster, _) = broadcast::channel(config.memory_tail.max(16));
        let dropped_total = Arc::new(AtomicU64::new(0));
        let stop_requested = Arc::new(AtomicBool::new(false));
        let reader_semaphore = Arc::new(Semaphore::new(config.reader_concurrency));
        let acquire_timeout = Duration::from_millis(config.database.acquire_timeout_ms());
        let query_timeout = Duration::from_millis(config.database.query_timeout_ms());

        let (queue_tx, writer_handle, storage) = match &config.database {
            ResolvedDatabaseConfig::Sqlite { path, .. } => {
                let (sender, writer_handle) = start_sqlite_writer(
                    &tag,
                    path,
                    &tables,
                    &config,
                    stop_requested.clone(),
                    tail.clone(),
                    broadcaster.clone(),
                )?;
                (
                    WriterQueue::Blocking(sender),
                    WriterHandle::Blocking(writer_handle),
                    StorageBackend::Sqlite { path: path.clone() },
                )
            }
            database @ (ResolvedDatabaseConfig::Postgres { .. }
            | ResolvedDatabaseConfig::Mysql { .. }) => {
                let remote = RemotePool::connect(database).await?;
                tokio::time::timeout(query_timeout, remote.create_schema(&tables))
                    .await
                    .map_err(|_| {
                        DnsError::runtime("query_recorder database schema setup timed out")
                    })??;
                let (sender, receiver) = mpsc::channel(config.queue_size);
                let handle = tokio::spawn(run_remote_writer(
                    RemoteWriterContext {
                        tables: tables.clone(),
                        stop_requested: stop_requested.clone(),
                        tail: tail.clone(),
                        memory_tail: config.memory_tail.max(1),
                        broadcaster: broadcaster.clone(),
                        dropped_total: dropped_total.clone(),
                        batch_size: config.batch_size,
                        flush_interval: Duration::from_millis(config.flush_interval_ms),
                        query_timeout,
                    },
                    receiver,
                    remote.clone(),
                ));
                (
                    WriterQueue::Async(sender),
                    WriterHandle::Async(handle),
                    StorageBackend::Remote(remote),
                )
            }
        };

        Ok(Arc::new(Self {
            tag,
            storage,
            tables,
            queue_tx,
            stop_requested,
            writer_handle: Mutex::new(Some(writer_handle)),
            tail,
            memory_tail: config.memory_tail.max(1),
            broadcaster,
            dropped_total,
            reader_semaphore,
            acquire_timeout,
            query_timeout,
        }))
    }

    pub(super) fn sqlite_path(&self) -> Result<&Path> {
        match &self.storage {
            StorageBackend::Sqlite { path } => Ok(path),
            StorageBackend::Remote(_) => Err(DnsError::runtime(
                "query_recorder SQLite operation used with a remote database",
            )),
        }
    }

    pub(super) fn remote_pool(&self) -> Option<&RemotePool> {
        match &self.storage {
            StorageBackend::Sqlite { .. } => None,
            StorageBackend::Remote(remote) => Some(remote),
        }
    }

    pub(super) fn enqueue(&self, pending: PendingRecord) {
        if let Err(err) = self
            .queue_tx
            .try_send(WriterCommand::Insert(Box::new(pending)))
        {
            self.dropped_total.fetch_add(1, Ordering::Relaxed);
            warn!("query_recorder dropped record: {}", err);
        }
    }

    pub(super) fn cleanup(&self, cutoff_ms: i64) {
        if let Err(err) = self.queue_tx.try_send(WriterCommand::Cleanup { cutoff_ms }) {
            warn!("query_recorder cleanup skipped: {}", err);
        }
    }

    pub(super) fn clear_history(&self) -> ClearHistoryReply {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        self.queue_tx
            .blocking_send(WriterCommand::ClearHistory { reply_tx })
            .map_err(|err| format!("query_recorder clear enqueue failed: {err}"))?;
        reply_rx
            .recv()
            .map_err(|err| format!("query_recorder clear reply failed: {err}"))?
    }

    #[cfg(test)]
    pub(super) fn flush_for_test(&self) -> FlushReply {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        self.queue_tx
            .blocking_send(WriterCommand::Flush { reply_tx })
            .map_err(|err| format!("query_recorder flush enqueue failed: {err}"))?;
        reply_rx
            .recv()
            .map_err(|err| format!("query_recorder flush reply failed: {err}"))?
    }

    pub(super) async fn shutdown(&self) -> Result<()> {
        self.reader_semaphore.close();
        self.stop_requested.store(true, Ordering::Relaxed);
        let handle = self
            .writer_handle
            .lock()
            .map_err(|_| DnsError::runtime("query_recorder writer lock poisoned"))?
            .take();
        match handle {
            Some(WriterHandle::Blocking(handle)) => {
                let _ = tokio::task::spawn_blocking(move || handle.join())
                    .await
                    .map_err(|err| {
                        DnsError::runtime(format!("query_recorder join failed: {err}"))
                    })?;
            }
            Some(WriterHandle::Async(handle)) => {
                handle.await.map_err(|err| {
                    DnsError::runtime(format!("query_recorder writer task failed: {err}"))
                })?;
            }
            None => {}
        }
        if let Some(remote) = self.remote_pool() {
            remote.close().await;
        }
        Ok(())
    }

    pub(super) async fn run_sqlite_reader<T, F>(self: Arc<Self>, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&RecorderBackend, &Connection) -> Result<T> + Send + 'static,
    {
        let path = self.sqlite_path()?.to_path_buf();
        let backend = self.clone();
        let (interrupt_tx, interrupt_rx) = oneshot::channel();
        let task = tokio::task::spawn_blocking(move || {
            let conn = open_reader_database(&path)?;
            let interrupt = conn.get_interrupt_handle();
            if interrupt_tx.send(interrupt).is_err() {
                return Err(DnsError::runtime("query_recorder read cancelled"));
            }
            operation(&backend, &conn)
        });

        let interrupt = interrupt_rx.await.ok();
        let mut guard = InterruptOnDrop(interrupt);
        let result = task
            .await
            .map_err(|err| DnsError::runtime(format!("blocking task failed: {err}")))?;
        guard.0.take();
        result
    }
}

struct InterruptOnDrop(Option<rusqlite::InterruptHandle>);

impl Drop for InterruptOnDrop {
    fn drop(&mut self) {
        if let Some(interrupt) = self.0.take() {
            interrupt.interrupt();
        }
    }
}

fn start_sqlite_writer(
    tag: &str,
    path: &Path,
    tables: &TableNames,
    config: &ResolvedRecorderConfig,
    stop_requested: Arc<AtomicBool>,
    tail: Arc<Mutex<VecDeque<RecordDetail>>>,
    broadcaster: broadcast::Sender<RecordDetail>,
) -> Result<(SyncSender<WriterCommand>, JoinHandle<()>)> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|err| {
            DnsError::plugin(format!(
                "failed to create query_recorder directory '{}': {}",
                parent.display(),
                err
            ))
        })?;
    }

    let mut conn = open_writer_database(path).map_err(|err| {
        DnsError::plugin(format!(
            "failed to open query_recorder database '{}': {}",
            path.display(),
            err
        ))
    })?;
    restrict_database_permissions(path).map_err(|err| {
        DnsError::plugin(format!(
            "failed to restrict query_recorder database '{}': {}",
            path.display(),
            err
        ))
    })?;
    create_schema(&mut conn, tables)?;
    restrict_database_permissions(path).map_err(|err| {
        DnsError::plugin(format!(
            "failed to restrict query_recorder database sidecars '{}': {}",
            path.display(),
            err
        ))
    })?;
    if let Err(err) = conn.execute_batch("PRAGMA optimize;") {
        warn!("query_recorder PRAGMA optimize failed at startup: {}", err);
    }

    let (queue_tx, queue_rx) = sync_channel(config.queue_size);
    let memory_tail = config.memory_tail.max(1);
    let batch_size = config.batch_size;
    let flush_interval = Duration::from_millis(config.flush_interval_ms);
    let writer_handle = thread::Builder::new()
        .name(format!("query-recorder-{tag}"))
        .spawn({
            let tables = tables.clone();
            let stop_requested = stop_requested.clone();
            move || {
                if let Err(err) = run_writer_thread(
                    WriterThreadContext {
                        tables,
                        stop_requested,
                        tail,
                        memory_tail,
                        broadcaster,
                        batch_size,
                        flush_interval,
                    },
                    queue_rx,
                    conn,
                ) {
                    error!("query_recorder writer stopped: {}", err);
                }
            }
        })?;
    Ok((queue_tx, writer_handle))
}

async fn run_remote_writer(
    context: RemoteWriterContext,
    mut receiver: mpsc::Receiver<WriterCommand>,
    remote: RemotePool,
) {
    let mut pending = Vec::with_capacity(context.batch_size);
    let mut interval = tokio::time::interval(context.flush_interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            command = receiver.recv() => {
                match command {
                    Some(WriterCommand::Insert(record)) => {
                        pending.push(*record);
                        if pending.len() >= context.batch_size {
                            let _ = flush_remote(&context, &remote, &mut pending).await;
                        }
                    }
                    Some(WriterCommand::Cleanup { cutoff_ms }) => {
                        let _ = flush_remote(&context, &remote, &mut pending).await;
                        match tokio::time::timeout(
                            context.query_timeout,
                            remote.cleanup(&context.tables, cutoff_ms),
                        )
                        .await
                        {
                            Ok(Ok(())) => {}
                            Ok(Err(err)) => warn!("query_recorder cleanup failed: {}", err),
                            Err(_) => warn!("query_recorder cleanup timed out"),
                        }
                    }
                    Some(WriterCommand::ClearHistory { reply_tx }) => {
                        let _ = flush_remote(&context, &remote, &mut pending).await;
                        let result = match tokio::time::timeout(
                            context.query_timeout,
                            remote.clear_history(&context.tables),
                        )
                        .await
                        {
                            Ok(Ok(cleared_records)) => {
                                if let Ok(mut tail) = context.tail.lock() {
                                    tail.clear();
                                }
                                Ok(ClearHistoryResult { cleared_records })
                            }
                            Ok(Err(err)) => Err(err.to_string()),
                            Err(_) => Err("query_recorder clear timed out".to_string()),
                        };
                        let _ = reply_tx.send(result);
                    }
                    #[cfg(test)]
                    Some(WriterCommand::Flush { reply_tx }) => {
                        let result = flush_remote(&context, &remote, &mut pending)
                            .await
                            .map_err(|err| err.to_string());
                        let _ = reply_tx.send(result);
                    }
                    None => {
                        let _ = flush_remote(&context, &remote, &mut pending).await;
                        break;
                    }
                }
            }
            _ = interval.tick() => {
                let _ = flush_remote(&context, &remote, &mut pending).await;
                if context.stop_requested.load(Ordering::Relaxed) {
                    break;
                }
            }
        }
    }
}

async fn flush_remote(
    context: &RemoteWriterContext,
    remote: &RemotePool,
    pending: &mut Vec<PendingRecord>,
) -> Result<()> {
    if pending.is_empty() {
        return Ok(());
    }
    let batch = std::mem::take(pending);
    let batch_len = batch.len();
    match tokio::time::timeout(
        context.query_timeout,
        remote.insert_batch(&context.tables, batch),
    )
    .await
    {
        Ok(Ok(committed)) => {
            let mut tail = context
                .tail
                .lock()
                .map_err(|_| DnsError::runtime("query_recorder tail buffer lock poisoned"))?;
            for detail in committed {
                if tail.len() >= context.memory_tail {
                    tail.pop_front();
                }
                tail.push_back(detail.clone());
                let _ = context.broadcaster.send(detail);
            }
            Ok(())
        }
        Ok(Err(err)) => {
            context
                .dropped_total
                .fetch_add(batch_len as u64, Ordering::Relaxed);
            warn!(
                "query_recorder dropped remote database batch ({} records): {}",
                batch_len, err
            );
            Err(err)
        }
        Err(_) => {
            context
                .dropped_total
                .fetch_add(batch_len as u64, Ordering::Relaxed);
            warn!(
                "query_recorder dropped remote database batch ({} records): query timed out",
                batch_len
            );
            Err(DnsError::runtime(
                "query_recorder remote database batch timed out",
            ))
        }
    }
}

#[cfg(unix)]
fn restrict_database_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let path_text = path.as_os_str().to_string_lossy();
    for candidate in [
        path.to_path_buf(),
        PathBuf::from(format!("{path_text}-wal")),
        PathBuf::from(format!("{path_text}-shm")),
    ] {
        if candidate.exists() {
            std::fs::set_permissions(candidate, std::fs::Permissions::from_mode(0o600))?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn restrict_database_permissions(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::restrict_database_permissions;

    #[test]
    fn permission_helper_restricts_query_database_sidecars() {
        let directory = tempfile::tempdir().expect("temporary query recorder directory");
        let path = directory.path().join("queries.db");
        let path_text = path.as_os_str().to_string_lossy();
        let candidates = [
            path.clone(),
            std::path::PathBuf::from(format!("{path_text}-wal")),
            std::path::PathBuf::from(format!("{path_text}-shm")),
        ];
        for candidate in &candidates {
            std::fs::write(candidate, []).expect("create database file");
            std::fs::set_permissions(candidate, std::fs::Permissions::from_mode(0o644))
                .expect("set deliberately broad permissions");
        }

        restrict_database_permissions(&path).expect("restrict query database files");

        for candidate in candidates {
            let mode = std::fs::metadata(candidate)
                .expect("database metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
    }
}
