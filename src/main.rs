// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! OxiDNS Next binary entry point.
//!
//! The binary is intentionally thin: it parses CLI arguments and delegates to
//! either foreground runtime startup or operating-system service management.

use oxidns_next::cli;
use oxidns_next::infra::error::Result;

fn main() -> Result<()> {
    #[cfg(windows)]
    if oxidns_next::infra::service::try_dispatch_windows_service()? {
        return Ok(());
    }

    cli::run()
}
