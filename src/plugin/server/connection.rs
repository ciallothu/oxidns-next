// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared connection lifecycle support for server plugins.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tracing::debug;

pub(crate) struct ConnectionGuard {
    active_connections: Arc<AtomicU64>,
    src: SocketAddr,
    protocol: &'static str,
}

impl ConnectionGuard {
    pub(crate) fn new(
        active_connections: Arc<AtomicU64>,
        src: SocketAddr,
        protocol: &'static str,
    ) -> Self {
        Self {
            active_connections,
            src,
            protocol,
        }
    }
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        let active = self
            .active_connections
            .fetch_sub(1, Ordering::Relaxed)
            .saturating_sub(1);
        debug!(
            "{} connection from {} closed (active: {})",
            self.protocol, self.src, active
        );
        if active > 0 && active.is_multiple_of(10) {
            debug!("Active connections: {}", active);
        }
    }
}
