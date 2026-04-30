//! Shared retry and error-classification helpers for the agentic /search loop.
//!
//! Used by the router, SearXNG, reader, and judge callers to enforce the
//! "single retry on transient failures, no retry on semantic failures" rule
//! locked in the design doc. Keeps call sites free of per-module retry boilerplate.

use std::future::Future;
use std::time::Duration;

use tokio::time::sleep;

/// Run the given async operation. On the first failure, wait `delay`, then
/// try exactly once more. Returns the final `Result<T, E>` from whichever
/// attempt ran last. The operation may be called at most twice.
///
/// No exponential backoff: the semantics we want are "hiccup recovery", not
/// generic retry-with-backoff. If both attempts fail, propagate the error.
pub async fn retry_once<F, Fut, T, E>(delay: Duration, mut op: F) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    match op().await {
        Ok(v) => Ok(v),
        Err(_first) => {
            sleep(delay).await;
            op().await
        }
    }
}

/// Classify an error message as transient (worth a single retry).
///
/// Matches lowercase substrings typical of reqwest / hyper / io errors for
/// connection faults, timeouts, DNS failures, and broken pipes. Semantic
/// errors (`404`, `400`, `parse error`) are NOT transient.
///
/// Cross-platform: Unix and Windows surface the same TCP failure modes
/// through different wording. Windows IO errors carry the WSA error code
/// in `(os error N)` and prose like "actively refused" /
/// "host is unreachable" / "network is unreachable" rather than the BSD
/// strings. Both wordings are matched here so the classifier behaves
/// consistently no matter where Wren runs — otherwise a real user on
/// Windows would see `ReaderPartialFailure` instead of
/// `ReaderUnavailable` whenever the reader sandbox is down.
pub fn is_transient_connect_error(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    // Unix wording (also reused by reqwest/hyper on Windows in some paths).
    m.contains("connection refused")
        || m.contains("connection reset")
        || m.contains("timed out")
        || m.contains("timeout")
        || m.contains("dns")
        || m.contains("broken pipe")
        // Windows wording (WSAECONNREFUSED 10061, WSAEHOSTUNREACH 10065,
        // WSAENETUNREACH 10051, WSAETIMEDOUT 10060, WSAECONNRESET 10054).
        || m.contains("actively refused")
        || m.contains("host is unreachable")
        || m.contains("no route to host")
        || m.contains("network is unreachable")
        || m.contains("forcibly closed")
        || m.contains("(os error 10061)")
        || m.contains("(os error 10060)")
        || m.contains("(os error 10054)")
        || m.contains("(os error 10051)")
        || m.contains("(os error 10065)")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn retry_once_succeeds_on_second_attempt() {
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let result = retry_once(Duration::from_millis(1), || {
            let c = c.clone();
            async move {
                let attempt = c.fetch_add(1, Ordering::SeqCst);
                if attempt == 0 {
                    Err::<u32, &'static str>("boom")
                } else {
                    Ok(7)
                }
            }
        })
        .await;
        assert_eq!(result, Ok(7));
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn retry_once_returns_second_error_on_double_failure() {
        let result = retry_once(Duration::from_millis(1), || async {
            Err::<u32, &'static str>("nope")
        })
        .await;
        assert_eq!(result, Err("nope"));
    }

    #[tokio::test]
    async fn retry_once_skips_retry_on_first_success() {
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let result: Result<u32, &'static str> = retry_once(Duration::from_millis(1), || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(42)
            }
        })
        .await;
        assert_eq!(result, Ok(42));
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn transient_classifier_matches_connect_errors() {
        // Unix / BSD wording.
        assert!(is_transient_connect_error("Connection refused"));
        assert!(is_transient_connect_error("operation timed out"));
        assert!(is_transient_connect_error("dns error"));
        assert!(is_transient_connect_error("connection reset by peer"));
        assert!(is_transient_connect_error("broken pipe"));
    }

    #[test]
    fn transient_classifier_matches_windows_wording() {
        // Real Windows error strings as surfaced by std::io::Error /
        // reqwest. These wordings differ from BSD; matching them prevents
        // Windows users from seeing the wrong reader-status warning.
        assert!(is_transient_connect_error(
            "No connection could be made because the target machine actively refused it. (os error 10061)"
        ));
        assert!(is_transient_connect_error(
            "An existing connection was forcibly closed by the remote host. (os error 10054)"
        ));
        assert!(is_transient_connect_error(
            "A socket operation was attempted to an unreachable host. (os error 10065)"
        ));
        assert!(is_transient_connect_error(
            "A socket operation was attempted to an unreachable network. (os error 10051)"
        ));
        assert!(is_transient_connect_error(
            "A connection attempt failed... did not properly respond after a period of time (os error 10060)"
        ));
    }

    #[test]
    fn transient_classifier_rejects_semantic_errors() {
        assert!(!is_transient_connect_error("404 Not Found"));
        assert!(!is_transient_connect_error("parse error"));
        assert!(!is_transient_connect_error("invalid json"));
    }
}
