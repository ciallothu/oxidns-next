// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Bounded keyed mailboxes for RouterOS manager workers.
//!
//! A mailbox consumes capacity per distinct key. Enqueuing an already queued
//! key merges the newer value in place, so repeated DNS observations cannot
//! crowd out the most recent state for other domains.

use std::collections::VecDeque;
use std::fmt::{Debug, Formatter};
use std::hash::Hash;
use std::sync::{Arc, Mutex};

use ahash::AHashMap;
use tokio::sync::Notify;

/// Value semantics used when a newer item arrives for an already queued key.
pub(crate) trait Coalesce: Sized {
    fn coalesce(&mut self, newer: Self);
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum PushOutcome {
    Inserted,
    Coalesced,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum TryPushError<V> {
    Full(V),
    Closed(V),
}

struct MailboxState<K, V> {
    values: AHashMap<K, V>,
    order: VecDeque<K>,
    closed: bool,
}

struct MailboxInner<K, V> {
    capacity: usize,
    state: Mutex<MailboxState<K, V>>,
    items_ready: Notify,
}

pub(crate) struct KeyedMailbox<K, V> {
    inner: Arc<MailboxInner<K, V>>,
}

impl<K, V> Clone for KeyedMailbox<K, V> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<K, V> Debug for KeyedMailbox<K, V>
where
    K: Eq + Hash,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f.debug_struct("KeyedMailbox")
            .field("capacity", &self.inner.capacity)
            .field("len", &state.values.len())
            .field("closed", &state.closed)
            .finish()
    }
}

impl<K, V> KeyedMailbox<K, V>
where
    K: Clone + Eq + Hash,
    V: Coalesce,
{
    pub(crate) fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "mailbox capacity must be positive");
        Self {
            inner: Arc::new(MailboxInner {
                capacity,
                state: Mutex::new(MailboxState {
                    values: AHashMap::with_capacity(capacity.min(1_024)),
                    order: VecDeque::with_capacity(capacity.min(1_024)),
                    closed: false,
                }),
                items_ready: Notify::new(),
            }),
        }
    }

    pub(crate) fn try_push(
        &self,
        key: K,
        value: V,
    ) -> std::result::Result<PushOutcome, TryPushError<V>> {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.closed {
            return Err(TryPushError::Closed(value));
        }
        if let Some(queued) = state.values.get_mut(&key) {
            queued.coalesce(value);
            return Ok(PushOutcome::Coalesced);
        }
        if state.values.len() >= self.inner.capacity {
            return Err(TryPushError::Full(value));
        }
        state.order.push_back(key.clone());
        state.values.insert(key, value);
        drop(state);
        self.inner.items_ready.notify_one();
        Ok(PushOutcome::Inserted)
    }

    // Used by ros_address_list batching; it is intentionally unused in a
    // minimal build that enables only ros_route.
    #[allow(dead_code)]
    pub(crate) fn try_recv(&self) -> Option<(K, V)> {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let key = state.order.pop_front()?;
        let value = state
            .values
            .remove(&key)
            .expect("mailbox order and values must remain consistent");
        drop(state);
        Some((key, value))
    }

    /// Remove one queued key without disturbing the order of other work.
    #[cfg(any(feature = "plugin-ros-address-list", feature = "plugin-ros-route"))]
    pub(crate) fn take(&self, key: &K) -> Option<V> {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let value = state.values.remove(key)?;
        state.order.retain(|queued| queued != key);
        drop(state);
        Some(value)
    }

    pub(crate) async fn recv(&self) -> Option<(K, V)> {
        loop {
            let notified = self.inner.items_ready.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let mut state = self
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if let Some(key) = state.order.pop_front() {
                    let value = state
                        .values
                        .remove(&key)
                        .expect("mailbox order and values must remain consistent");
                    drop(state);
                    return Some((key, value));
                }
                if state.closed {
                    return None;
                }
            }
            notified.await;
        }
    }

    pub(crate) fn close(&self) {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.closed = true;
        state.values.clear();
        state.order.clear();
        drop(state);
        self.inner.items_ready.notify_waiters();
        self.inner.items_ready.notify_one();
    }

    pub(crate) fn len(&self) -> usize {
        self.inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values
            .len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Eq, PartialEq)]
    struct Latest(u32);

    impl Coalesce for Latest {
        fn coalesce(&mut self, newer: Self) {
            *self = newer;
        }
    }

    #[tokio::test]
    async fn same_key_replaces_before_capacity_check() {
        let mailbox = KeyedMailbox::new(1);
        assert_eq!(mailbox.try_push("a", Latest(1)), Ok(PushOutcome::Inserted));
        assert_eq!(mailbox.try_push("a", Latest(2)), Ok(PushOutcome::Coalesced));
        assert!(matches!(
            mailbox.try_push("b", Latest(3)),
            Err(TryPushError::Full(Latest(3)))
        ));

        assert_eq!(mailbox.recv().await, Some(("a", Latest(2))));
    }

    #[tokio::test]
    async fn close_wakes_receiver_and_rejects_producer() {
        let mailbox = KeyedMailbox::<&str, Latest>::new(1);
        mailbox.try_push("queued", Latest(0)).expect("queued item");
        mailbox.close();

        assert_eq!(mailbox.len(), 0);
        assert_eq!(mailbox.recv().await, None);
        assert!(matches!(
            mailbox.try_push("a", Latest(1)),
            Err(TryPushError::Closed(Latest(1)))
        ));
    }

    #[test]
    #[cfg(feature = "plugin-ros-address-list")]
    fn take_removes_only_the_selected_key() {
        let mailbox = KeyedMailbox::new(2);
        mailbox.try_push("a", Latest(1)).expect("a");
        mailbox.try_push("b", Latest(2)).expect("b");

        assert_eq!(mailbox.take(&"a"), Some(Latest(1)));
        assert_eq!(mailbox.try_recv(), Some(("b", Latest(2))));
        assert_eq!(mailbox.try_recv(), None);
    }
}
