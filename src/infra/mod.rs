// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Infrastructure services used across OxiDNS Next runtime surfaces.

pub mod build_info;
pub mod cache;
pub mod clock;
pub mod control;
pub mod env;
pub mod error;
pub mod network;
pub mod observability;
pub mod service;
pub mod system;
pub mod task;
#[cfg(feature = "plugin-upgrade")]
pub mod upgrade;

/// OxiDNS Next version shared by CLI and management APIs.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
