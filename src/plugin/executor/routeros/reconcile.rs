// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Single-flight background snapshot coordination for RouterOS managers.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;
use tokio::task::{JoinError, JoinHandle};
use tokio::time::Instant;

use crate::infra::error::Result;

const MAX_RECONCILE_RETRY_SECS: u64 = 60;

#[derive(Debug, Default)]
pub(crate) struct ReconcileRetry {
    failures: u32,
    retry_at: Option<Instant>,
}

impl ReconcileRetry {
    pub(crate) fn schedule(&mut self, transport_delay: Option<Duration>) {
        self.failures = self.failures.saturating_add(1);
        let exponent = self.failures.saturating_sub(1).min(6);
        let local = Duration::from_secs(
            1u64.checked_shl(exponent)
                .unwrap_or(MAX_RECONCILE_RETRY_SECS)
                .min(MAX_RECONCILE_RETRY_SECS),
        );
        let delay = transport_delay.map_or(local, |transport| transport.max(local));
        self.retry_at = Some(Instant::now() + delay);
    }

    pub(crate) fn reset(&mut self) {
        self.failures = 0;
        self.retry_at = None;
    }

    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.retry_at
    }

    pub(crate) fn mark_due(&mut self) {
        self.retry_at = None;
    }
}

#[derive(Debug)]
pub(crate) struct VersionedSnapshot<T> {
    #[allow(dead_code)]
    pub(crate) generation: u64,
    pub(crate) value: T,
}

struct CompletionNotify(Arc<Notify>);

impl Drop for CompletionNotify {
    fn drop(&mut self) {
        // `wait()` registers its waiter before checking `JoinHandle` state, so
        // no stored permit is required when completion wins that race.
        self.0.notify_waiters();
    }
}

#[derive(Debug)]
pub(crate) struct BackgroundReconcile<T> {
    handle: Option<JoinHandle<Result<VersionedSnapshot<T>>>>,
    completed: Arc<Notify>,
}

impl<T: Send + 'static> BackgroundReconcile<T> {
    pub(crate) fn new() -> Self {
        Self {
            handle: None,
            completed: Arc::new(Notify::new()),
        }
    }

    pub(crate) fn is_running(&self) -> bool {
        self.handle.is_some()
    }

    pub(crate) fn start<F>(&mut self, generation: u64, future: F) -> bool
    where
        F: Future<Output = Result<T>> + Send + 'static,
    {
        if self.handle.is_some() {
            return false;
        }
        let completed = self.completed.clone();
        self.handle = Some(tokio::spawn(async move {
            let _completion = CompletionNotify(completed);
            future
                .await
                .map(|value| VersionedSnapshot { generation, value })
        }));
        true
    }

    #[cfg(any(
        test,
        feature = "plugin-ros-address-list",
        feature = "plugin-ros-route"
    ))]
    pub(crate) fn is_finished(&self) -> bool {
        self.handle.as_ref().is_some_and(JoinHandle::is_finished)
    }

    #[cfg(any(feature = "plugin-ros-address-list", feature = "plugin-ros-route"))]
    pub(crate) async fn wait(&self) {
        if self.handle.is_none() {
            std::future::pending::<()>().await;
        }
        let notified = self.completed.notified();
        tokio::pin!(notified);
        // Register first, then inspect the level-triggered JoinHandle state.
        // Completion before registration is observed by `is_finished`, while
        // completion after registration wakes this waiter.
        notified.as_mut().enable();
        if self.is_finished() {
            return;
        }
        notified.await;
    }

    #[cfg(any(test, feature = "plugin-ros-route"))]
    pub(crate) async fn take_finished(
        &mut self,
    ) -> Option<std::result::Result<Result<VersionedSnapshot<T>>, JoinError>> {
        if !self.is_finished() {
            return None;
        }
        Some(self.handle.take()?.await)
    }

    #[cfg(feature = "plugin-ros-address-list")]
    pub(crate) async fn take(
        &mut self,
    ) -> Option<std::result::Result<Result<VersionedSnapshot<T>>, JoinError>> {
        Some(self.handle.take()?.await)
    }

    pub(crate) async fn cancel(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
            let _ = handle.await;
        }
    }
}

impl<T: Send + 'static> Default for BackgroundReconcile<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn wait_observes_completion_that_precedes_waiter_registration() {
        let mut reconcile = BackgroundReconcile::new();
        assert!(reconcile.start(7, async { Ok::<_, crate::infra::error::DnsError>("done") }));
        while !reconcile.is_finished() {
            tokio::task::yield_now().await;
        }

        tokio::time::timeout(Duration::from_millis(100), reconcile.wait())
            .await
            .expect("finished reconcile must not lose its completion signal");
        let result = reconcile
            .take_finished()
            .await
            .expect("finished reconcile result")
            .expect("reconcile task")
            .expect("reconcile result");
        assert_eq!(result.generation, 7);
        assert_eq!(result.value, "done");
    }

    #[tokio::test]
    async fn wait_wakes_when_reconcile_finishes_after_registration() {
        let mut reconcile = BackgroundReconcile::new();
        let (release, wait_release) = tokio::sync::oneshot::channel();
        assert!(reconcile.start(8, async move {
            wait_release.await.expect("release reconcile");
            Ok::<_, crate::infra::error::DnsError>("done")
        }));

        let wait = reconcile.wait();
        tokio::pin!(wait);
        assert!(
            tokio::time::timeout(Duration::from_millis(10), wait.as_mut())
                .await
                .is_err()
        );
        release.send(()).expect("release reconcile");
        tokio::time::timeout(Duration::from_millis(100), wait)
            .await
            .expect("registered waiter must be notified");
    }
}
