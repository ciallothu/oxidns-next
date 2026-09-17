// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Repeated RouterOS error-log throttling.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use ahash::AHashMap;

/// Permit one log per key in each interval.
#[derive(Debug)]
pub(crate) struct ErrorLogThrottle {
    interval: Duration,
    last_logged: Mutex<AHashMap<String, Instant>>,
}

impl ErrorLogThrottle {
    pub(crate) fn new(interval: Duration) -> Self {
        Self {
            interval,
            last_logged: Mutex::new(AHashMap::new()),
        }
    }

    pub(crate) fn should_log(&self, key: impl Into<String>) -> bool {
        let key = key.into();
        let now = Instant::now();
        let mut logged = self
            .last_logged
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if logged
            .get(&key)
            .is_some_and(|last| now.duration_since(*last) < self.interval)
        {
            return false;
        }
        logged.insert(key, now);
        true
    }
}

impl Default for ErrorLogThrottle {
    fn default() -> Self {
        Self::new(Duration::from_secs(60))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_key_is_throttled_independently() {
        let throttle = ErrorLogThrottle::new(Duration::from_secs(60));
        assert!(throttle.should_log("connect"));
        assert!(!throttle.should_log("connect"));
        assert!(throttle.should_log("reconcile"));
    }
}
