// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Public library surface for OxiDNS Next.
//!
//! This crate exposes the runtime building blocks used by the CLI binary,
//! integration tests, and embedding scenarios. The architecture is organized
//! around the main request path:
//!
//! `server -> DnsContext -> matcher / executor / provider pipeline -> upstream
//! or side effects -> response`
//!
//! Top-level modules:
//!
//! - [`api`]: management and health HTTP endpoints.
//! - [`app`]: foreground runtime bootstrap and process orchestration.
//! - [`build_info`]: compiled feature and runtime plugin capability reporting.
//! - [`cli`]: command-line definitions and option parsing.
//! - [`config`]: YAML configuration schema, loading, and validation.
//! - [`core`]: DNS request execution context and policy matching primitives.
//! - [`infra`]: process, network, service, observability, and shared runtime
//!   infrastructure.
//! - [`plugin`]: plugin registry plus server, executor, matcher, and provider
//!   extension points.
//! - [`proto`]: owned DNS protocol model and wire codec.

// `async_trait` marks its generated boxed futures as `must_use`. Newer nightly
// Clippy versions otherwise report those generated attributes as redundant on
// every async trait method, with the diagnostic pointing at the macro call.
#![allow(clippy::double_must_use)]

#[cfg(feature = "api")]
pub mod api;
#[path = "api/macros.rs"]
mod api_macros;
pub mod app;
pub mod build_info;
pub mod cli;
pub mod config;
pub mod core;
pub mod infra;
pub mod plugin;

pub use oxidns_next_macros::plugin_factory;
pub use oxidns_next_proto as proto;
