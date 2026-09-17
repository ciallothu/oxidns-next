// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Global plugin runtime lifecycle and application-control integration.

#[cfg(debug_assertions)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use tokio::sync::Mutex as AsyncMutex;

use super::{
    PluginRegistry, PluginRuntime, lock_mutex, read_rwlock, try_global_catalog, write_rwlock,
};
use crate::config::types::Config;
#[cfg(feature = "storage-redis")]
use crate::infra::cache::redis as redis_cache;
use crate::infra::control::{AppController, ControlRequestError};
use crate::infra::error::{DnsError, Result};
use crate::infra::network::outbound;

#[cfg(debug_assertions)]
#[derive(Debug)]
pub(super) struct TestRuntimeGuard {
    pub(super) _guard: tokio::sync::OwnedMutexGuard<()>,
}

#[cfg(debug_assertions)]
fn test_runtime_lock() -> Arc<AsyncMutex<()>> {
    static TEST_RUNTIME_LOCK: OnceLock<Arc<AsyncMutex<()>>> = OnceLock::new();
    TEST_RUNTIME_LOCK
        .get_or_init(|| Arc::new(AsyncMutex::new(())))
        .clone()
}

#[cfg(debug_assertions)]
static SERIALIZE_RUNTIME_FOR_TESTS: AtomicBool = AtomicBool::new(false);

#[cfg(debug_assertions)]
pub fn enable_runtime_test_serialization() {
    SERIALIZE_RUNTIME_FOR_TESTS.store(true, Ordering::Relaxed);
}

#[derive(Debug)]
pub struct PluginRuntimeManager {
    pub(super) current: RwLock<Option<Arc<PluginRuntime>>>,
    controller: Mutex<Option<Arc<AppController>>>,
    lifecycle: Arc<AsyncMutex<()>>,
}

impl PluginRuntimeManager {
    pub(super) fn new() -> Self {
        Self {
            current: RwLock::new(None),
            controller: Mutex::new(None),
            lifecycle: Arc::new(AsyncMutex::new(())),
        }
    }

    pub fn current_runtime(&self) -> Option<Arc<PluginRuntime>> {
        read_rwlock(&self.current).clone()
    }

    pub async fn init_runtime(self: &Arc<Self>, config: Config) -> Result<Arc<PluginRuntime>> {
        // Test serialization must not wait while holding the lifecycle lock.
        // A concurrently running test may need `destroy_runtime` to acquire
        // that lifecycle lock before it can drop the previous test guard.
        #[cfg(debug_assertions)]
        let test_guard = if SERIALIZE_RUNTIME_FOR_TESTS.load(Ordering::Relaxed) {
            Some(test_runtime_lock().lock_owned().await)
        } else {
            None
        };
        let lifecycle_guard = self.lifecycle.clone().lock_owned().await;
        let manager = self.clone();
        tokio::spawn(async move {
            let _lifecycle_guard = lifecycle_guard;
            let mut candidate = PluginRegistry::new();
            #[cfg(debug_assertions)]
            candidate.set_test_runtime_guard(test_guard);
            candidate.load_catalog(try_global_catalog()?);
            let candidate = Arc::new(candidate);

            let previous = write_rwlock(&manager.current).take();
            if let Some(previous) = previous {
                previous.destroy().await;
            }
            outbound::clear_global();
            #[cfg(feature = "storage-redis")]
            redis_cache::clear_global();
            outbound::install_global(&config.network.outbound)?;
            #[cfg(feature = "storage-redis")]
            if let Err(err) = redis_cache::install_global(config.storage.redis.as_ref()) {
                outbound::clear_global();
                return Err(err);
            }
            #[cfg(not(feature = "storage-redis"))]
            if config.storage.redis.is_some() {
                outbound::clear_global();
                return Err(DnsError::config(
                    "storage.redis requires the storage-redis feature",
                ));
            }

            let matcher_runtime_controls_enabled =
                cfg!(feature = "api") && config.api.http.is_some();
            if let Err(err) = candidate
                .clone()
                .init_plugins_with_runtime_controls(
                    config.plugins,
                    matcher_runtime_controls_enabled,
                )
                .await
            {
                candidate.destroy().await;
                outbound::clear_global();
                #[cfg(feature = "storage-redis")]
                redis_cache::clear_global();
                return Err(err);
            }

            write_rwlock(&manager.current).replace(candidate.clone());
            Ok(candidate)
        })
        .await
        .map_err(|error| {
            DnsError::runtime(format!("runtime initialization task failed: {error}"))
        })?
    }

    pub async fn destroy_runtime(self: &Arc<Self>) {
        let lifecycle_guard = self.lifecycle.clone().lock_owned().await;
        let manager = self.clone();
        let task = tokio::spawn(async move {
            let _lifecycle_guard = lifecycle_guard;
            let previous = write_rwlock(&manager.current).take();
            if let Some(previous) = previous {
                previous.destroy().await;
            }
            outbound::clear_global();
            #[cfg(feature = "storage-redis")]
            redis_cache::clear_global();
        });
        if let Err(error) = task.await {
            tracing::error!(%error, "runtime destruction task failed");
        }
    }

    /// The manager is the single authoritative owner of the application
    /// controller. Runtimes never carry their own copy, so swapping a runtime
    /// can neither lose nor race the controller.
    pub fn set_controller(&self, controller: Arc<AppController>) {
        *lock_mutex(&self.controller) = Some(controller);
    }

    pub fn clear_controller(&self) {
        *lock_mutex(&self.controller) = None;
    }

    fn controller(&self) -> Option<Arc<AppController>> {
        lock_mutex(&self.controller).clone()
    }

    pub fn request_app_reload(&self) -> Result<()> {
        let controller = self.controller().ok_or_else(|| {
            DnsError::plugin("reload executor requires application control context")
        })?;
        controller.request_reload().map_err(|err| match err {
            ControlRequestError::ReloadBusy => {
                DnsError::plugin("reload is already pending or in progress")
            }
            ControlRequestError::CommandChannelClosed => {
                DnsError::plugin("application reload command channel is closed")
            }
        })
    }

    pub fn request_app_restart(&self) -> Result<()> {
        let controller = self
            .controller()
            .ok_or_else(|| DnsError::plugin("restart requires application control context"))?;
        controller
            .request_restart()
            .map_err(|err| DnsError::plugin(err.to_string()))
    }

    pub async fn reload_provider(&self, tag: &str) -> Result<()> {
        let runtime = self
            .current_runtime()
            .ok_or_else(|| DnsError::plugin("provider reload requires an initialized runtime"))?;
        runtime.reload_provider(tag).await
    }

    pub fn plugin_count(&self) -> usize {
        self.current_runtime()
            .map_or(0, |runtime| runtime.plugin_count())
    }

    pub fn server_plugin_count(&self) -> usize {
        self.current_runtime()
            .map_or(0, |runtime| runtime.server_plugin_count())
    }

    #[cfg(test)]
    async fn set_current_runtime_for_test(&self, runtime: Arc<PluginRuntime>) {
        let _guard = self.lifecycle.lock().await;
        let previous = write_rwlock(&self.current).take();
        if let Some(previous) = previous {
            previous.destroy().await;
        }
        write_rwlock(&self.current).replace(runtime);
    }
}

static GLOBAL_MANAGER: OnceLock<Arc<PluginRuntimeManager>> = OnceLock::new();

pub fn global_manager() -> Arc<PluginRuntimeManager> {
    GLOBAL_MANAGER
        .get_or_init(|| Arc::new(PluginRuntimeManager::new()))
        .clone()
}

pub fn current_runtime() -> Option<Arc<PluginRuntime>> {
    global_manager().current_runtime()
}

pub async fn init(config: Config) -> Result<Arc<PluginRuntime>> {
    global_manager().init_runtime(config).await
}

pub async fn destroy_runtime() {
    global_manager().destroy_runtime().await;
}

pub fn set_app_controller(controller: Arc<AppController>) {
    global_manager().set_controller(controller);
}

pub fn clear_app_controller() {
    global_manager().clear_controller();
}

pub fn request_app_reload() -> Result<()> {
    global_manager().request_app_reload()
}

pub fn request_app_restart() -> Result<()> {
    global_manager().request_app_restart()
}

pub async fn reload_provider(tag: &str) -> Result<()> {
    global_manager().reload_provider(tag).await
}

pub fn plugin_count() -> usize {
    global_manager().plugin_count()
}

pub fn server_plugin_count() -> usize {
    global_manager().server_plugin_count()
}

#[cfg(test)]
pub async fn reset_runtime_for_test() {
    destroy_runtime().await;
    clear_app_controller();
}

#[cfg(test)]
pub async fn set_current_runtime_for_test(runtime: Arc<PluginRuntime>) {
    global_manager().set_current_runtime_for_test(runtime).await;
}
