// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Aggregate completion for one observation expanded into multiple target keys.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::oneshot;

use crate::infra::error::{DnsError, Result};

#[derive(Debug)]
pub(crate) struct BatchCompletion {
    remaining: AtomicUsize,
    first_error: Mutex<Option<String>>,
    sender: Mutex<Option<oneshot::Sender<Result<()>>>>,
}

impl BatchCompletion {
    pub(crate) fn new(items: usize, sender: oneshot::Sender<Result<()>>) -> Arc<Self> {
        assert!(items > 0, "completion item count must be positive");
        Arc::new(Self {
            remaining: AtomicUsize::new(items),
            first_error: Mutex::new(None),
            sender: Mutex::new(Some(sender)),
        })
    }

    pub(crate) fn finish(&self, result: &Result<()>) {
        if let Err(error) = result {
            let mut first = self
                .first_error
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if first.is_none() {
                *first = Some(error.to_string());
            }
        }
        if self.remaining.fetch_sub(1, Ordering::AcqRel) != 1 {
            return;
        }
        let result = self
            .first_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .map_or(Ok(()), |message| Err(DnsError::plugin(message)));
        if let Some(sender) = self
            .sender
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            let _ = sender.send(result);
        }
    }
}
