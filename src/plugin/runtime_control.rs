// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Domain-specific runtime controls attached to initialized plugins.

use std::sync::Arc;

#[cfg(feature = "api")]
use crate::plugin::matcher::MatcherRuntimeControl;
use crate::plugin::provider::ProviderRuntimeControl;

#[derive(Debug, Clone)]
#[cfg_attr(not(feature = "api"), allow(dead_code))]
pub(crate) enum PluginRuntimeControl {
    #[cfg(feature = "api")]
    Matcher(Arc<MatcherRuntimeControl>),
    Provider(Arc<ProviderRuntimeControl>),
}

impl PluginRuntimeControl {
    pub(crate) async fn drain(&self) {
        match self {
            #[cfg(feature = "api")]
            Self::Matcher(_) => {}
            Self::Provider(control) => control.drain().await,
        }
    }
}
