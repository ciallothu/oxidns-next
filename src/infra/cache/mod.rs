// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared infrastructure cache primitives.

#[cfg(feature = "storage-redis")]
pub(crate) mod redis;
pub mod ttl;
