//! Bounded HTTP policy adapted from pinned Codex codex-client/src/retry.rs.
//! e363b08c, Apache-2.0. Separate from V2 stream and ordinary runner retries.
use super::*;

#[derive(Clone, Copy)]
pub struct LegacyCompactionRetryPolicy {
    max_retries: u8,
    base_delay: Duration,
}
impl LegacyCompactionRetryPolicy {
    pub const NONE: Self = Self {
        max_retries: 0,
        base_delay: Duration::ZERO,
    };
    pub const NATIVE: Self = Self {
        max_retries: 3,
        base_delay: Duration::from_millis(250),
    };

    pub fn new(max_retries: u8, base_delay: Duration) -> Result<Self, ModelError> {
        if max_retries > 5 || base_delay > Duration::from_secs(30) {
            return Err(ModelError::InvalidRequest);
        }
        Ok(Self {
            max_retries,
            base_delay,
        })
    }
    pub(super) fn delay(self, error: ModelError, failures: u8) -> Option<Duration> {
        if failures >= self.max_retries
            || !matches!(
                error,
                ModelError::Unavailable
                    | ModelError::TimedOut
                    | ModelError::ConnectionFailed
                    | ModelError::ResponseReadFailed
                    | ModelError::RateLimited
                    | ModelError::HttpRateLimited(_)
                    | ModelError::ServerOverloaded
                    | ModelError::InternalServerError
                    | ModelError::UnexpectedStatus(500..=599)
            )
        {
            return None;
        }
        Some(
            self.base_delay
                .saturating_mul(1 << failures)
                .min(Duration::from_secs(30)),
        )
    }
}
