// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared bounded batching for RouterOS management-plane operations.

use std::collections::VecDeque;
use std::future::Future;

/// Poll futures in ordered `join_all` batches without allowing more than
/// `limit` operations to be in flight at once.
pub(crate) async fn join_all_bounded<F>(
    futures: impl IntoIterator<Item = F>,
    limit: usize,
) -> Vec<F::Output>
where
    F: Future,
{
    assert!(limit > 0, "batch limit must be positive");
    let mut pending = futures.into_iter().collect::<VecDeque<_>>();
    let mut output = Vec::with_capacity(pending.len());
    while !pending.is_empty() {
        let batch = pending.drain(..pending.len().min(limit));
        output.extend(futures::future::join_all(batch).await);
    }
    output
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[tokio::test]
    async fn bounded_join_never_exceeds_limit_and_preserves_order() {
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let futures = (0..33).map(|value| {
            let active = active.clone();
            let maximum = maximum.clone();
            async move {
                let current = active.fetch_add(1, Ordering::AcqRel) + 1;
                maximum.fetch_max(current, Ordering::AcqRel);
                tokio::task::yield_now().await;
                active.fetch_sub(1, Ordering::AcqRel);
                value
            }
        });

        let output = join_all_bounded(futures, 16).await;

        assert_eq!(output, (0..33).collect::<Vec<_>>());
        assert_eq!(maximum.load(Ordering::Acquire), 16);
    }
}
