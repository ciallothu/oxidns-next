// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared RouterOS task lifecycle helpers.

use tokio::task::JoinHandle;

/// Request cancellation immediately and retain a detached reaper so callers
/// can honor a hard shutdown deadline without leaking the join handle.
pub(crate) fn abort_and_reap(handle: JoinHandle<()>) {
    handle.abort();
    tokio::spawn(async move {
        let _ = handle.await;
    });
}
