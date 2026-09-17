// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Runtime mode shared by every configured matcher reference.

#[cfg(any(feature = "api", test))]
use std::sync::atomic::{AtomicU8, Ordering};

#[cfg(any(feature = "api", test))]
use serde::{Deserialize, Serialize};

/// Operational override applied to the matcher result before expression
/// negation.
#[cfg(any(feature = "api", test))]
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, Eq, PartialEq)]
#[repr(u8)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MatcherRuntimeMode {
    #[default]
    Normal = 0,
    AlwaysFalse = 1,
    AlwaysTrue = 2,
}

#[cfg(any(feature = "api", test))]
impl MatcherRuntimeMode {
    #[inline]
    fn from_raw(raw: u8) -> Self {
        match raw {
            value if value == Self::AlwaysFalse as u8 => Self::AlwaysFalse,
            value if value == Self::AlwaysTrue as u8 => Self::AlwaysTrue,
            _ => Self::Normal,
        }
    }
}

#[cfg(any(feature = "api", test))]
#[derive(Debug)]
pub(crate) struct MatcherRuntimeControl {
    mode: AtomicU8,
}

#[cfg(any(feature = "api", test))]
impl MatcherRuntimeControl {
    pub(crate) fn new() -> Self {
        Self {
            mode: AtomicU8::new(MatcherRuntimeMode::Normal as u8),
        }
    }

    #[inline]
    pub(crate) fn mode(&self) -> MatcherRuntimeMode {
        MatcherRuntimeMode::from_raw(self.mode.load(Ordering::Relaxed))
    }

    #[inline]
    pub(crate) fn set_mode(&self, mode: MatcherRuntimeMode) {
        self.mode.store(mode as u8, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddr};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;

    use super::*;
    use crate::core::context::DnsContext;
    use crate::plugin::Plugin;
    use crate::plugin::matcher::{Matcher, MatcherEvaluation, MatcherRef};
    use crate::proto::Message;

    #[derive(Debug)]
    struct CountingMatcher {
        calls: Arc<AtomicUsize>,
        result: bool,
    }

    #[async_trait]
    impl Plugin for CountingMatcher {
        fn tag(&self) -> &str {
            "counting"
        }
    }

    impl Matcher for CountingMatcher {
        fn is_match(&self, _context: &mut DnsContext) -> bool {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.result
        }
    }

    fn context() -> DnsContext {
        DnsContext::new(
            SocketAddr::from((Ipv4Addr::LOCALHOST, 5300)),
            Message::new(),
        )
    }

    #[test]
    fn runtime_modes_fix_the_base_result_before_reference_negation() {
        let calls = Arc::new(AtomicUsize::new(0));
        let matcher: Arc<dyn Matcher> = Arc::new(CountingMatcher {
            calls: calls.clone(),
            result: true,
        });
        let control = Arc::new(MatcherRuntimeControl::new());
        let normal = MatcherRef::with_runtime_control(matcher.clone(), false, control.clone());
        let reversed = MatcherRef::with_runtime_control(matcher, true, control.clone());
        let mut context = context();

        assert_eq!(normal.evaluate(&mut context), MatcherEvaluation::Matched);
        assert_eq!(
            reversed.evaluate(&mut context),
            MatcherEvaluation::NotMatched
        );
        assert_eq!(calls.load(Ordering::Relaxed), 2);

        control.set_mode(MatcherRuntimeMode::AlwaysFalse);
        assert_eq!(
            normal.evaluate(&mut context),
            MatcherEvaluation::AlwaysFalse { matched: false }
        );
        assert_eq!(
            reversed.evaluate(&mut context),
            MatcherEvaluation::AlwaysFalse { matched: true }
        );
        assert_eq!(calls.load(Ordering::Relaxed), 2);

        control.set_mode(MatcherRuntimeMode::AlwaysTrue);
        assert_eq!(
            normal.evaluate(&mut context),
            MatcherEvaluation::AlwaysTrue { matched: true }
        );
        assert_eq!(
            reversed.evaluate(&mut context),
            MatcherEvaluation::AlwaysTrue { matched: false }
        );
        assert_eq!(calls.load(Ordering::Relaxed), 2);

        control.set_mode(MatcherRuntimeMode::Normal);
        assert!(normal.is_match(&mut context));
        assert!(!reversed.is_match(&mut context));
        assert_eq!(calls.load(Ordering::Relaxed), 4);
    }
}
